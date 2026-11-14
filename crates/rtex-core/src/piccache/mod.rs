//! Picture cache for background passes. A TikZ-heavy document spends most of a layout pass
//! drawing pictures that did not change. Each pass records where every picture environment
//! landed (page, position, dimensions; `rtex-capture.lua`), and the next pass replaces every
//! picture whose source and surroundings are unchanged with that region of the earlier pass's
//! PDF (an image of exactly the picture's size), so the pass only typesets what changed.
//!
//! What identifies a picture: its environment's text, plus everything that can change its
//! rendering without changing its text: the preamble, and the definitions and settings made in
//! the document body before it (`\def`, `\newcommand`, `\tikzset`, `\definecolor` …), in the
//! order TeX reads them (`\input` chains followed). The font, color and width in force at the
//! picture are recorded by the capture and compared at use (`RecordedPic::state`). Pictures
//! that depend on more than that (labels, references, counters, external files, data files,
//! `remember picture`/`overlay`, group-escaping assignments) are never cached.

mod cacheable;
mod scan;
mod store;
#[cfg(test)]
mod tests;

pub use cacheable::uncacheable;
pub use scan::scan_pictures;
pub use store::PicCache;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Picture environments: units of their own when they start a line, inline boxes after text;
/// the cache handles each (every one has its own `\end{…}` delimiter). The capture's list
/// (`rtex_capture.PICTURE_ENVS`) names the same environments.
pub const PICTURE_ENVS: &[&str] = &["tikzpicture", "circuitikz", "pgfpicture"];

/// A picture environment found in the document sources.
#[derive(Debug, Clone)]
pub struct PictureRef {
    /// `file:line` of its `\begin{…}` (what the capture reports).
    pub key: String,
    pub env: String,
    pub hash: u64,
    pub cacheable: bool,
    /// 1-based line of its `\end{…}` (the capture checks it after skipping the body).
    pub end_line: usize,
}

/// Where a pass drew a picture (`pics` in the capture JSON).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecordedPic {
    #[serde(default)]
    pub env: String,
    pub page: i64,
    /// Page coordinates (sp from the page's top-left): left edge and baseline.
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub d: i64,
    pub page_height: i64,
    /// Font, color and width in force where the picture began (the capture compares it before
    /// using the cached picture).
    #[serde(default)]
    pub state: String,
}

/// A cached picture: a region of a kept pass PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub env: String,
    #[serde(default)]
    pub state: String,
    pub pdf: String,
    pub page: i64,
    /// PDF-space bounding box (llx, lly, urx, ury) in sp of a bp (`img.new{bbox}`).
    pub bbox: [i64; 4],
    pub w: i64,
    pub h: i64,
    pub d: i64,
    /// Pass serial the entry was last wanted in (for eviction).
    pub last_used: u64,
}

fn hash_bytes(h: &mut std::collections::hash_map::DefaultHasher, s: &str) {
    use std::hash::Hasher;
    h.write(s.as_bytes());
    h.write_u8(0);
}

/// Hash of the preamble (with its `\input`s expanded) for the picture hashes: every picture
/// depends on it.
pub fn preamble_hash(texts: &BTreeMap<String, String>, main: &str) -> u64 {
    texts
        .get(main)
        .and_then(|t| crate::split_preamble(t))
        .map(|(p, _)| crate::document::hash_str(&crate::document::expand_inputs(p, texts)))
        .unwrap_or(0)
}
