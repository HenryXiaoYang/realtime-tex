//! What a session tells its host: events, their parts, and the result of an edit.

use super::*;

/// Diagnostic view of one layout unit (`Session::layout_units`, `rtex serve` `units`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LayoutUnitInfo {
    pub uid: i64,
    pub kind: String,
    pub name: Option<String>,
    pub file: Option<String>,
    pub first_line: i64,
    pub last_line: Option<i64>,
    pub rows: i64,
    pub span: Option<ParaId>,
}

#[derive(Debug, Clone, Copy, Serialize, Default, PartialEq, Eq)]
pub struct Versions {
    pub source_revision: Revision,
    pub context_revision: u64,
    pub engine_generation: u64,
    pub layout_version: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state")]
pub enum Convergence {
    Converged,
    Converging { pass: u32, reasons: Vec<String> },
    PassLimitReached { passes: u32, reasons: Vec<String> },
    Stale { pending_since: Revision },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state")]
pub enum CompileStatus {
    Ok,
    CompiledWithErrors { count: usize },
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: String,
    pub file: Option<String>,
    pub line: Option<i64>,
    pub message: String,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Timing {
    pub total_us: u64,
    pub tex_us: u64,
    pub traverse_us: u64,
    pub pack_us: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageUpdate {
    pub page: i64,
    pub exact: bool,
    pub hash: u64,
    pub dl: DisplayList,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphPlacement {
    pub par_id: ParaId,
    pub fragments: Vec<Fragment>,
    /// Number of rows (typeset lines, display rows, float rows) of the unit.
    pub lines: i64,
    /// Unit kind: "par", "env:<name>" or "heading:<name>".
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
// ParagraphUpdate carries its display list inline: an event is moved once, through a channel,
// and boxing the field would change the public type hosts match on for no measurable gain
#[allow(clippy::large_enum_variant)]
pub enum Event {
    ParagraphUpdate {
        par_id: ParaId,
        edit_id: u64,
        versions: Versions,
        status: String,
        reasons: Vec<String>,
        fragments: Vec<Fragment>,
        pagination_stale: bool,
        context_stale: bool,
        dl: DisplayList,
        diagnostics: Vec<Diagnostic>,
        timing: Timing,
    },
    LayoutUpdate {
        versions: Versions,
        compile: CompileStatus,
        convergence: Convergence,
        passes: u32,
        pages_changed: Vec<PageUpdate>,
        pages_total: i64,
        placements: Vec<ParagraphPlacement>,
        eligible_paragraphs: Vec<ParaId>,
        pdf_fallback: Option<PathBuf>,
        wall_ms: u64,
    },
    Diagnostics {
        source: String,
        items: Vec<Diagnostic>,
    },
    EngineState {
        engine_generation: u64,
        state: String,
        reason: Option<String>,
    },
    BackgroundScheduled {
        par_id: Option<ParaId>,
        reasons: Vec<String>,
        edit_id: u64,
    },
    PdfExported {
        job_id: u64,
        path: Option<PathBuf>,
        status: CompileStatus,
        converged: bool,
        passes: u32,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EditResult {
    pub edit_id: u64,
    pub source_revision: Revision,
    pub outcome: EditOutcome,
    pub routed: String,
    pub reasons: Vec<String>,
    /// Host-side stage times in µs: segmentation, eligibility, request dispatch.
    pub host_us: [u64; 3],
}
