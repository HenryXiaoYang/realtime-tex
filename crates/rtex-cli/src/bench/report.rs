//! The benchmark report and its Markdown and CSV forms.

use super::gates::*;
use super::roundtrip::*;
use super::*;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Default)]
pub struct BenchReport {
    pub machine: String,
    pub hardware_factor: f64,
    pub linebreak_ms: BTreeMap<String, (f64, f64)>,
    pub results: Vec<CategoryResult>,
    pub gates: Vec<Gate>,
    pub pass: bool,
}

pub fn write_markdown(report: &BenchReport, path: &Path) -> Result<()> {
    let mut md = String::new();
    md.push_str(&format!("# Round-trip benchmark\n\nMachine: {}  \nHardware factor h = {:.2} (in-engine line breaking vs the paper's Table 1)\n\n", report.machine, report.hardware_factor));
    md.push_str("## In-engine line breaking (upstream `texlode/luatex-benchmark`, vendored in bench/upstream)\n\n| Category | Paper | Here (amortized median) |\n|---|---|---|\n");
    for (cat, paper) in PAPER_LINEBREAK_MS {
        if let Some((med, _)) = report.linebreak_ms.get(cat) {
            md.push_str(&format!("| {cat} | {paper:.2} ms | {med:.3} ms |\n"));
        }
    }
    md.push_str("\n## Warm-session round trips (apply_edit → ParagraphUpdate in hand)\n\n| Project | Category | Lines | Glyphs | Individual median | P95 | TeX | Traverse | Pack | Amortized median | Overhead share |\n|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &report.results {
        md.push_str(&format!("| {} | {} | {} | {} | {:.3} ms | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} ms | {:.0} % |\n", r.project, r.category, r.lines, r.glyphs, r.individual_total.median, r.individual_total.p95, r.individual_tex.median, r.individual_traverse.median, r.individual_pack.median, r.amortized_total.median, r.overhead_share_median * 100.0));
    }
    md.push_str(&format!("\nPaper references: short round trip {:.2} ms, medium {:.2} ms.\n\n## Gates\n\n| Gate | Value | Limit | Pass | Note |\n|---|---|---|---|---|\n", PAPER_ROUNDTRIP_MS[0].1, PAPER_ROUNDTRIP_MS[1].1));
    for g in &report.gates {
        md.push_str(&format!(
            "| {} | {:.3} | {:.3} | {} | {} |\n",
            g.name,
            g.value,
            g.limit,
            if g.pass { "yes" } else { "**no**" },
            g.note
        ));
    }
    md.push_str(&format!(
        "\nOverall: {}\n",
        if report.pass {
            "all gates pass"
        } else {
            "some gates fail"
        }
    ));
    std::fs::write(path, md)?;
    Ok(())
}

pub fn write_csv(report: &BenchReport, path: &Path) -> Result<()> {
    let mut s = String::from("project,category,lines,glyphs,individual_median_ms,individual_p5_ms,individual_p95_ms,tex_median_ms,traverse_median_ms,pack_median_ms,amortized_median_ms,amortized_p95_ms,overhead_share\n");
    for r in &report.results {
        s.push_str(&format!(
            "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}\n",
            r.project,
            r.category,
            r.lines,
            r.glyphs,
            r.individual_total.median,
            r.individual_total.p5,
            r.individual_total.p95,
            r.individual_tex.median,
            r.individual_traverse.median,
            r.individual_pack.median,
            r.amortized_total.median,
            r.amortized_total.p95,
            r.overhead_share_median
        ));
    }
    std::fs::write(path, s)?;
    Ok(())
}

pub fn machine_string() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
        })
        .unwrap_or_else(|| "unknown cpu".into());
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    format!("{cpu} ({n} threads)")
}

pub(super) fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // days since epoch → civil date (no external crate)
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}",
        (secs % 86400) / 3600,
        (secs % 3600) / 60
    )
}

pub(super) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
