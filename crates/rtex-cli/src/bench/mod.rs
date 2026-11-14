//! `rtex bench`: the paper's in-engine line-breaking replica (hardware factor) and host-side
//! round-trip benchmarks over paragraph categories and document sizes, with the gates of
//! docs/benchmarks.md.

mod gates;
mod linebreak;
mod report;
mod roundtrip;
mod stats;

pub use gates::gates;
pub use linebreak::linebreak;
pub use report::{machine_string, write_csv, write_markdown, BenchReport};
pub use roundtrip::roundtrip;

use anyhow::Result;
use report::*;
use rtex_core::texlive::TexLive;
use std::path::PathBuf;

pub const PAPER_LINEBREAK_MS: [(&str, f64); 5] = [
    ("short", 0.17),
    ("medium", 0.70),
    ("long", 1.99),
    ("inline-math", 0.20),
    ("display-math", 0.11),
];

pub const PAPER_ROUNDTRIP_MS: [(&str, f64); 2] = [("short", 0.79), ("medium", 6.11)];

pub fn run_all(
    projects: Vec<PathBuf>,
    categories: Vec<String>,
    samples: usize,
    inner: usize,
    build: PathBuf,
    out: PathBuf,
    quick: bool,
) -> Result<BenchReport> {
    let tl = TexLive::discover()?;
    let repo = tl.rtex_texdir.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&build)?;
    std::fs::create_dir_all(&out)?;
    println!(
        "== in-engine line breaking (upstream texlode/luatex-benchmark systematic-benchmark.tex)"
    );
    let lb = linebreak(&tl, &repo, &build.join("linebreak"), quick)?;
    for (cat, paper) in PAPER_LINEBREAK_MS {
        if let Some((med, _)) = lb.get(cat) {
            println!("  {cat:12} here {med:.3} ms   paper {paper:.2} ms");
        }
    }
    let h_short = lb.get("short").map(|v| v.0 / 0.17).unwrap_or(1.0);
    let h_medium = lb.get("medium").map(|v| v.0 / 0.70).unwrap_or(1.0);
    let h = h_short.max(h_medium);
    println!("  hardware factor h = max({h_short:.2}, {h_medium:.2}) = {h:.2}");
    let mut report = BenchReport {
        machine: machine_string(),
        hardware_factor: h,
        linebreak_ms: lb,
        ..Default::default()
    };
    for p in &projects {
        println!("== round trips: {}", p.display());
        let mut r = roundtrip(p, &categories, samples, inner, &build)?;
        report.results.append(&mut r);
    }
    gates(&mut report);
    for g in &report.gates {
        println!(
            "  gate {:60} {:8.3} ≤ {:7.3}  {}",
            g.name,
            g.value,
            g.limit,
            if g.pass { "pass" } else { "FAIL" }
        );
    }
    let stamp = chrono_like_now();
    write_markdown(&report, &out.join(format!("roundtrip-{stamp}.md")))?;
    write_csv(&report, &out.join(format!("roundtrip-{stamp}.csv")))?;
    std::fs::write(
        out.join(format!("roundtrip-{stamp}.json")),
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("results written to {}", out.display());
    Ok(report)
}
