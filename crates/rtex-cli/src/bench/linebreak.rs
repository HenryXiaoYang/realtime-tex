//! The paper's in-engine line-breaking benchmark, run unmodified: its result gives the hardware factor.

use anyhow::{bail, Context, Result};
use rtex_core::texlive::TexLive;
use std::collections::BTreeMap;
use std::path::Path;

/// Run the paper's own `systematic-benchmark.tex` (vendored verbatim in
/// bench/upstream/luatex-benchmark) and return per-category (median, p95) in ms, keyed by our
/// category names. `quick` is ignored: the upstream script fixes its own sample plan (30 × 100).
pub fn linebreak(
    tl: &TexLive,
    repo: &Path,
    out_dir: &Path,
    _quick: bool,
) -> Result<BTreeMap<String, (f64, f64)>> {
    std::fs::create_dir_all(out_dir)?;
    let out_dir = rtex_core::paths::canonical(out_dir)?;
    let dir = repo.join("bench/upstream/luatex-benchmark");
    let mut cmd = tl.lualatex_cmd(&dir);
    cmd.arg("-interaction=nonstopmode")
        .arg(format!(
            "--output-directory={}",
            rtex_core::paths::tex(&out_dir)
        ))
        .arg("systematic-benchmark.tex");
    let out = cmd
        .output()
        .context("running upstream systematic-benchmark.tex")?;
    let log = std::fs::read_to_string(out_dir.join("systematic-benchmark.log")).unwrap_or_default();
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), log);
    let re = regex::Regex::new(r"(?m)^(Short|Medium|Long|Inline math|Display math)\s+median=\s*([0-9.]+) ms\s+P5=\s*([0-9.]+)\s+P95=\s*([0-9.]+)").unwrap();
    let mut m = BTreeMap::new();
    for c in re.captures_iter(&text) {
        let key = match &c[1] {
            "Short" => "short",
            "Medium" => "medium",
            "Long" => "long",
            "Inline math" => "inline-math",
            _ => "display-math",
        };
        m.insert(
            key.to_string(),
            (c[2].parse().unwrap_or(0.0), c[4].parse().unwrap_or(0.0)),
        );
    }
    if m.len() < 5 {
        bail!(
            "could not parse the upstream benchmark summary; lualatex exit {:?}; tail:\n{}",
            out.status,
            text.chars()
                .rev()
                .take(600)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        );
    }
    Ok(m)
}
