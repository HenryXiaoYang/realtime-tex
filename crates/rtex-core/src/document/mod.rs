//! Source buffers, paragraph segmentation with stable ids, and revision bookkeeping.

mod buffer;
mod inputs;
mod lex;
mod segment;
#[cfg(test)]
mod tests;

pub(crate) use inputs::find_inputs;
pub use inputs::{
    body_setup_statements, expand_inputs, input_targets, inputted_files, load_project_files,
    transitive_inputs, FileSet,
};
pub use lex::strip_comment;
pub(crate) use lex::{find_command, is_verbatim_env};
pub use segment::fast_source;

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// Monotonic per-session revision counter (bumped by every edit on any file).
pub type Revision = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ParaId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanKind {
    Preamble,
    Body,
    Env,
    Heading,
    /// After `\end{document}`
    Trailer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub id: ParaId,
    pub range: Range<usize>,
    pub kind: SpanKind,
    pub hash: u64,
    /// Revision of the last edit that touched this span.
    pub last_revision: Revision,
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct FileBuf {
    pub text: String,
    pub line_starts: Vec<usize>,
    pub spans: Vec<Span>,
    /// What the preamble adds to the segmentation rules.
    pub rules: SegmentRules,
}

/// What the preamble tells the segmenter (`Policy::segment_rules`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SegmentRules {
    /// Extra block environments (theorem-like) that end a span like the built-in ones.
    pub block_envs: Vec<String>,
    /// Headings the class sets run-in (`paragraph`): their span runs on through the paragraph
    /// or block environment that sets the title, which the capture keeps in the heading's unit.
    pub run_in: Vec<String>,
}

/// Block environments after whose `\end` a span ends (the capture closes the unit there); the
/// session adds theorem-like environments from the preamble through `SegmentRules::block_envs`.
pub use crate::piccache::PICTURE_ENVS;

pub const BLOCK_ENVS: &[&str] = &[
    "itemize",
    "enumerate",
    "description",
    "quote",
    "quotation",
    "verse",
    "center",
    "flushleft",
    "flushright",
    "abstract",
    "figure",
    "figure*",
    "table",
    "table*",
    "tabbing",
    // amsthm, verbatim material, setspace
    "proof",
    "verbatim",
    "verbatim*",
    "lstlisting",
    "Verbatim",
    "alltt",
    "spacing",
    // manual bibliography: its \section* heading and \bibitem list are one unit
    "thebibliography",
    // IEEEtran's index terms and its description list (nomenclature)
    "IEEEkeywords",
    "IEEEdescription",
    // pictures: a top-level picture is a unit of its own (blank lines inside it never split it)
    "tikzpicture",
    "circuitikz",
    "pgfpicture",
];

const HEADING_CMDS: &[&str] = &[
    "\\chapter",
    "\\section",
    "\\subsection",
    "\\subsubsection",
    "\\part",
    "\\paragraph",
    "\\subparagraph",
    "\\tableofcontents",
];

pub fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The edit that turns `old` into `new`: the bytes between their common prefix and common
/// suffix, on character boundaries. None when they are equal.
pub fn diff_edit(old: &str, new: &str) -> Option<Edit> {
    if old == new {
        return None;
    }
    let (a, b) = (old.as_bytes(), new.as_bytes());
    let mut start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    while !old.is_char_boundary(start) {
        start -= 1;
    }
    let max = a.len().min(b.len()) - start;
    let mut suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max)
        .take_while(|(x, y)| x == y)
        .count();
    // the same bytes follow in both, so a boundary in one is a boundary in the other
    while !old.is_char_boundary(a.len() - suffix) {
        suffix -= 1;
    }
    Some(Edit {
        start_byte: start,
        end_byte: a.len() - suffix,
        text: new[start..b.len() - suffix].to_string(),
    })
}

pub struct IdAllocator(pub u64);

impl IdAllocator {
    pub fn next_id(&mut self) -> ParaId {
        self.0 += 1;
        ParaId(self.0)
    }
}

/// Outcome of applying an edit to a file.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EditOutcome {
    pub touched: Vec<ParaId>,
    pub added: Vec<ParaId>,
    pub removed: Vec<ParaId>,
    pub preamble_changed: bool,
}
