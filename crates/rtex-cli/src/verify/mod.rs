//! `rtex verify`: run the fidelity layers over a whole project.
//!   layer 1: every fast-eligible paragraph through the persistent server vs the shipout extractor
//!   layer 2: every page's display list vs the clean PDF's content stream (independent parser)
//!   layer 3: every page rasterized from the display list vs PyMuPDF's raster of the PDF
//!
//! `run` builds the project twice (with and without the capture) and hands the result to
//! `units` (layer 1) and `pages` (layers 2 and 3); `report` has the types, `pic_cache` the
//! optional picture-cache check.

mod pages;
mod pic_cache;
mod report;
mod units;

pub use report::*;

use anyhow::Result;
use pages::*;
use pic_cache::*;
use rtex_core::background::{run_pass_with, BibTool};
use rtex_core::eligibility::Policy;
use rtex_core::texlive::TexLive;
use units::*;

pub fn run(opts: VerifyOpts) -> Result<Report> {
    let mut tl = TexLive::discover()?;
    tl.math_owner_attribute = opts.math_owner_attribute;
    let project = rtex_core::paths::canonical(&opts.project)?;
    std::fs::create_dir_all(&opts.build)?;
    println!("== verify {} ({})", project.display(), opts.main);
    // bibliography support for mixed fixtures: run biber when a .bcf shows up after the first pass
    let preamble = rtex_core::project_preamble(&project, &opts.main)?;
    let mut policy = Policy::from_preamble(&preamble, &[], &[]);
    policy.permissive = opts.permissive;
    policy.math_owner_attribute = opts.math_owner_attribute;
    let unit_envs = policy.unit_envs_env();
    // Both builds run to aux convergence (labels, TOC, bibliography) exactly like the
    // background compiler does: a single first pass would leave every \ref as "??" while
    // the fast server resolves the labels from the aux that pass writes.
    const MAX_PASSES: u32 = 5;
    let cap_pass = run_pass_with(
        &tl,
        &project,
        &opts.main,
        &opts.build.join("capture"),
        MAX_PASSES,
        BibTool::Auto,
        true,
        &unit_envs,
        None,
    )?;
    let clean_pass = run_pass_with(
        &tl,
        &project,
        &opts.main,
        &opts.build.join("clean"),
        MAX_PASSES,
        BibTool::Auto,
        false,
        "",
        None,
    )?;
    if !cap_pass.aux_stable || !clean_pass.aux_stable {
        println!(
            "warning: aux family not stable after {MAX_PASSES} passes (capture {}, clean {})",
            cap_pass.aux_stable, clean_pass.aux_stable
        );
    }
    let (cap, clean) = (cap_pass.capture, clean_pass.capture);
    println!(
        "capture: {} paragraphs, {} pages ({:.1}s, {} passes); clean build {:.1}s ({} passes)",
        cap.json.paragraphs.len(),
        cap.json.pages,
        cap.wall.as_secs_f64(),
        cap_pass.passes,
        clean.wall.as_secs_f64(),
        clean_pass.passes
    );
    let mut report = Report {
        paragraphs_total: cap.json.paragraphs.len(),
        ..Default::default()
    };
    // T3: the capture package must not change the output
    let t3 = rtex_verify::pdfcompare::compare(&cap.pdf, &clean.pdf)?;
    report.capture_pdf_equals_clean = t3.equal;
    println!(
        "T3 (instrumented PDF == clean PDF): {}{}",
        t3.equal,
        if t3.equal {
            String::new()
        } else {
            format!(" {:?}", t3.differences.iter().take(3).collect::<Vec<_>>())
        }
    );
    check_units(&tl, &project, &preamble, &policy, &cap, &opts, &mut report)?;
    check_pages(&cap, &clean, &opts, &mut report)?;
    if opts.pic_cache {
        report.pic_cache = Some(pic_cache_check(&tl, &project, &opts, &unit_envs, &cap)?);
    }
    if let Some(p) = &opts.json_out {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(report)
}
