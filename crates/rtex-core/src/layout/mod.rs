//! Versioned layout store: engine units (contexts + row placements) from the latest background
//! pass, their mapping to host spans, page display lists, labels, and fragment construction.
//!
//! The types are here; `install` puts a pass in, `query` reads it, `compare` checks a live
//! result against it.

mod compare;
mod install;
mod query;

pub use compare::compare_unit_rows;

use crate::capture::{CaptureResult, CapturedParagraph, CapturedUnit, Placement};
use crate::document::{ParaId, Revision};
use rtex_dl::{DisplayList, Item, Line, Sp};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct Fragment {
    pub page: i64,
    /// 1-based inclusive range of the unit's rows on this page.
    pub first_line: i64,
    pub last_line: i64,
    /// x of the first row of the fragment (page coordinates).
    pub x: Sp,
    /// Per-row x positions (page coordinates), parallel to `baselines`.
    pub xs: Vec<Sp>,
    pub baselines: Vec<Sp>,
    /// Rows beyond the cached placements were extrapolated from the fast box's own geometry.
    pub approximate: bool,
}

/// What the host recorded about a span when a pass was started.
#[derive(Debug, Clone)]
pub struct SnapshotSpan {
    pub id: ParaId,
    pub file: String,
    pub first_line: i64,
    pub last_line: i64,
    pub last_revision: Revision,
    pub background_only: bool,
    /// What the fast path compiles for the span as the pass typeset it (`document::fast_source`
    /// of the snapshot: trailing newline removed, `\input` end-of-file space reproduced); the
    /// probe compile replays exactly this.
    pub text: String,
}

/// A capture unit with the derived data the session needs.
#[derive(Debug, Clone)]
pub struct EngineUnit {
    pub uid: i64,
    pub captured: CapturedUnit,
    /// The unit's first paragraph (source of the line-breaking parameters for "par" units).
    pub first_para: Option<CapturedParagraph>,
    /// Node flags merged over the unit's paragraphs.
    pub flags: BTreeMap<String, i64>,
    /// (groupcode, nest) of every member paragraph.
    pub members: Vec<(String, i64)>,
    pub span: Option<ParaId>,
    /// Capture unit ids this unit is made of: one, or several consecutive paragraph units that
    /// share a source span (`{\Large Title\par}` followed by a line of text), typeset as one box.
    pub uids: Vec<i64>,
}

impl EngineUnit {
    pub fn kind(&self) -> &str {
        &self.captured.kind
    }
    /// Does the unit have a replayable context? Environment and heading units always (their
    /// state is captured when they open); a paragraph unit when one of its paragraphs has a
    /// `para/begin` record, or when it was opened by `para/begin` itself (a paragraph that
    /// opens with display math has no line-broken text before the display, but the unit's
    /// own record holds the state at that point).
    pub fn has_context(&self) -> bool {
        self.captured.kind != "par"
            || self
                .first_para
                .as_ref()
                .map(|p| p.begin.is_some())
                .unwrap_or(false)
            || self
                .captured
                .nfss
                .family
                .as_deref()
                .map(|f| !f.is_empty())
                .unwrap_or(false)
    }

    /// Counters the unit advanced in the last pass (name → value at its end).
    pub fn advanced(&self) -> BTreeMap<String, i64> {
        self.captured.advanced.clone().unwrap_or_default()
    }
    pub fn name(&self) -> Option<&str> {
        self.captured.name.as_deref()
    }
    pub fn rows(&self) -> i64 {
        self.captured.placements.len() as i64
    }
    pub fn baselineskip(&self) -> Sp {
        let g = self
            .first_para
            .as_ref()
            .map(|p| &p.glues)
            .unwrap_or(&self.captured.glues);
        g.get("baselineskip")
            .and_then(|v| v.first())
            .map(|v| *v as i64)
            .unwrap_or(0)
    }
    /// The context object the fast server replays before typesetting this unit.
    pub fn context_json(&self) -> serde_json::Value {
        let c = &self.captured;
        let (ints, dims, glues, parshape) = match &self.first_para {
            Some(p) if c.kind == "par" => (&p.ints, &p.dims, &p.glues, &p.parshape),
            _ => (&c.ints, &c.dims, &c.glues, &c.parshape),
        };
        let (nfss, color) = match self.first_para.as_ref().and_then(|p| p.begin.as_ref()) {
            Some(b) if c.kind == "par" => (b.nfss.clone(), b.color.clone()),
            _ => (c.nfss.clone(), c.color.clone()),
        };
        serde_json::json!({
            "kind": c.kind, "name": c.name,
            "ints": ints, "dims": dims, "glues": glues, "parshape": parshape,
            "everypar": c.everypar, "nobreak": c.nobreak, "afterindent": c.afterindent, "noskipsec": c.noskipsec,
            "counters": c.abs_counters, "thefmt": c.abs_thefmt, "macros": c.abs_macros,
            "begin": { "nfss": nfss, "color": color },
        })
    }
}

/// Snapshot spans of a file buffer (what the session records when a pass starts).
pub fn snapshot_spans_of(
    fb: &crate::document::FileBuf,
    file: &str,
    policy: &crate::eligibility::Policy,
    file_is_inputted: bool,
) -> Vec<SnapshotSpan> {
    use crate::document::SpanKind;
    fb.spans
        .iter()
        .map(|sp| {
            let (a, b) = fb.line_range(sp);
            let text = &fb.text[sp.range.clone()];
            let background_only = matches!(sp.kind, SpanKind::Preamble | SpanKind::Trailer)
                || !crate::eligibility::classify_source(text, policy)
                    .1
                    .is_empty();
            SnapshotSpan {
                id: sp.id,
                file: file.to_string(),
                first_line: a,
                last_line: b,
                last_revision: sp.last_revision,
                background_only,
                text: crate::document::fast_source(fb, sp, file_is_inputted),
            }
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct LayoutStore {
    pub layout_version: u64,
    pub context_revision: u64,
    /// `source_revision` of the snapshot the latest layout was compiled from.
    pub snapshot_revision: Revision,
    pub units: Vec<EngineUnit>,
    pub by_span: HashMap<ParaId, usize>,
    pub pages: BTreeMap<i64, DisplayList>,
    pub page_hashes: BTreeMap<i64, u64>,
    pub snapshot_spans: Vec<SnapshotSpan>,
    /// Span id → index into `snapshot_spans`.
    snapshot_index: HashMap<ParaId, usize>,
    pub capture_dir: Option<std::path::PathBuf>,
    pub pdf: Option<std::path::PathBuf>,
    /// `\newlabel`/`\bibcite` definitions of the pass (name, value) for the server's `\ref`/`\cite`.
    pub labels: Arc<Vec<(String, String)>>,
    pub labels_hash: u64,
}
