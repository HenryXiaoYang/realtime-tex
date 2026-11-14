//! What `rtex verify` takes and reports: its options, a verdict per unit and per page, and the whole report (`--json-out`).

use serde::Serialize;
use std::path::PathBuf;

pub struct VerifyOpts {
    pub project: PathBuf,
    pub main: String,
    pub build: PathBuf,
    pub dpi: u32,
    pub raster: bool,
    pub json_out: Option<PathBuf>,
    pub max_paragraphs: Option<usize>,
    /// Only these unit ids (layer 1).
    pub units: Vec<i64>,
    /// Print fast and capture rows of differing units as text.
    pub dump_rows: bool,
    /// Fail unless at least this many units are eligible (regression gate).
    pub min_eligible: Option<usize>,
    /// Research: ignore the allow-list (unknown macros/environments are eligible) and let the
    /// row comparison judge every unit.
    pub permissive: bool,
    /// Pass layer 1 as long as at most this many units differ (probe-mode fixtures: the units
    /// known to be state-dependent must be caught, not absent).
    pub max_differing: Option<usize>,
    /// Pass layer 1 only with exactly this many differing units: the fixture's state-dependent
    /// units must be caught, neither more nor fewer.
    pub expect_differing: Option<usize>,
    /// Picture cache check: a second capture pass takes every cacheable picture from the
    /// first pass's PDF; units and placements must be identical, and (with `raster`) the two
    /// PDFs must render alike.
    pub pic_cache: bool,
    /// With `pic_cache`: fail unless at least this many pictures were recorded and reused.
    pub min_cached: Option<usize>,
    pub math_owner_attribute: Option<u16>,
}

#[derive(Serialize, Default)]
pub struct ParagraphVerdict {
    pub seq: i64,
    pub kind: String,
    pub lines: usize,
    pub glyphs_total: usize,
    pub glyphs_identical: usize,
    pub status: String,
    pub notes: Vec<String>,
    pub t_total_ms: f64,
}

#[derive(Serialize, Default)]
pub struct PageVerdict {
    pub page: i64,
    pub dl_glyphs: usize,
    pub pdf_glyphs: usize,
    pub matched: usize,
    pub unmatched: usize,
    pub anchored_max_dx_bp: f64,
    pub interior_max_dx_bp: f64,
    pub max_dy_bp: f64,
    pub tj_quantum_bp: f64,
    pub rules_dl: usize,
    pub rules_matched: usize,
    pub colors_dl: usize,
    pub colors_pdf: usize,
    pub exact: bool,
    pub flags: serde_json::Value,
    pub raster_unmatched_fraction: Option<f64>,
    pub raster_glyphs_skipped: Option<usize>,
    pub images_dl: usize,
    pub images_matched: usize,
    pub images_pdf: usize,
}

#[derive(Serialize, Default)]
pub struct Report {
    pub paragraphs_total: usize,
    pub paragraphs_eligible: usize,
    pub ineligible_reasons: std::collections::BTreeMap<String, usize>,
    pub paragraphs: Vec<ParagraphVerdict>,
    pub pages: Vec<PageVerdict>,
    pub pages_degraded: usize,
    pub capture_pdf_equals_clean: bool,
    pub layer1_pass: bool,
    pub layer2_pass: bool,
    pub layer3_pass: Option<bool>,
    pub pic_cache: Option<PicCacheReport>,
}

/// Result of the picture cache check (`--pic-cache`).
#[derive(Debug, Default, Serialize)]
pub struct PicCacheReport {
    /// Picture environments in the sources, and how many of them the cache may hold.
    pub pictures: usize,
    pub cacheable: usize,
    /// Pictures the first pass recorded (drawn as one box) and the second pass took from it.
    pub recorded: usize,
    pub hits: usize,
    /// Cached picture images the second pass's pages contain.
    pub cached_images: usize,
    /// Cached pictures whose skipped body did not end where the source scan said.
    pub mismatched: usize,
    pub units_equal: bool,
    pub placements_equal: bool,
    /// Worst unmatched ink fraction between the two passes' rendered pages (with `raster`).
    pub raster_unmatched_fraction: Option<f64>,
    pub wall_uncached_ms: u64,
    pub wall_cached_ms: u64,
    pub pass: bool,
}

impl Report {
    /// Every layer that ran passed, and the capture left the PDF as a clean build makes it.
    pub fn passed(&self) -> bool {
        self.layer1_pass
            && self.layer2_pass
            && self.layer3_pass.unwrap_or(true)
            && self.capture_pdf_equals_clean
            && self.pic_cache.as_ref().map(|p| p.pass).unwrap_or(true)
    }
}
