//! One background pass: snapshot the buffers, run the instrumented full compile (with
//! biber/bibtex when the document asks for it), and return the capture.

mod auxfiles;
mod index;
mod snapshot;
mod standby;
#[cfg(test)]
mod tests;

pub use auxfiles::{copy_aux_family, read_aux_labels};
pub use snapshot::{mirror_dirs, snapshot_dir, write_body_snapshot, write_snapshot};
pub use standby::{StandbySpec, WarmEngine};

use crate::capture::CaptureResult;
use crate::texlive::TexLive;
use anyhow::{Context, Result};
use auxfiles::*;
use index::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BibTool {
    Auto,
    Biber,
    BibTeX,
    None,
}

#[derive(Debug, Clone)]
pub struct PassOutcome {
    pub capture: CaptureResult,
    pub passes: u32,
    pub bib_ran: bool,
    pub aux_stable: bool,
}

impl Clone for CaptureResult {
    fn clone(&self) -> Self {
        CaptureResult {
            out_dir: self.out_dir.clone(),
            jobname: self.jobname.clone(),
            json: self.json.clone(),
            pdf: self.pdf.clone(),
            log: self.log.clone(),
            wall: self.wall,
            exit_ok: self.exit_ok,
        }
    }
}

/// Run instrumented passes in `out_dir` until the aux family is stable or `max_passes` is hit.
pub fn run_pass(
    tl: &TexLive,
    snapshot_dir: &Path,
    main: &str,
    out_dir: &Path,
    max_passes: u32,
    bib: BibTool,
    instrumented: bool,
) -> Result<PassOutcome> {
    run_pass_with(
        tl,
        snapshot_dir,
        main,
        out_dir,
        max_passes,
        bib,
        instrumented,
        "",
        None,
    )
}

/// `run_pass` with extra unit environments for the capture (see `run_capture_with`).
#[allow(clippy::too_many_arguments)]
pub fn run_pass_with(
    tl: &TexLive,
    snapshot_dir: &Path,
    main: &str,
    out_dir: &Path,
    max_passes: u32,
    bib: BibTool,
    instrumented: bool,
    unit_envs: &str,
    limit: Option<std::time::Duration>,
) -> Result<PassOutcome> {
    let mut runner = |_pass: u32| {
        crate::capture::run_capture_with(
            tl,
            snapshot_dir,
            main,
            out_dir,
            instrumented,
            unit_envs,
            limit,
        )
    };
    run_pass_with_runner(
        PassPlan {
            tl,
            snapshot_dir,
            main,
            aux_dir: out_dir,
            max_passes,
            bib,
        },
        &mut runner,
        &mut |_, _| {},
    )
}

/// Where and how a sequence of passes runs.
#[derive(Clone, Copy)]
pub struct PassPlan<'a> {
    pub tl: &'a TexLive,
    pub snapshot_dir: &'a Path,
    pub main: &'a str,
    /// The aux family the first pass starts from (each pass may run in its own directory).
    pub aux_dir: &'a Path,
    pub max_passes: u32,
    pub bib: BibTool,
}

/// `run_pass_with` with the single pass supplied by `runner` (a fresh lualatex, or a standby
/// `WarmEngine` that already holds the preamble). Each pass may run in its own directory
/// (`CaptureResult::out_dir`); `aux_dir` holds the aux family the first pass starts from.
pub fn run_pass_with_runner(
    plan: PassPlan,
    runner: &mut dyn FnMut(u32) -> Result<CaptureResult>,
    on_pass: &mut dyn FnMut(&CaptureResult, u32),
) -> Result<PassOutcome> {
    let PassPlan {
        tl,
        snapshot_dir,
        main,
        aux_dir,
        max_passes,
        bib,
    } = plan;
    std::fs::create_dir_all(aux_dir)?;
    let main_dir = match crate::paths::folder(main) {
        Some(m) => snapshot_dir.join(m),
        None => snapshot_dir.to_path_buf(),
    };
    let jobname = Path::new(main)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    let mut sig_before = aux_signature(aux_dir, &jobname);
    // what the bibliography program last ran on (`bib_input`)
    let mut bib_ran_on = None;
    let mut indexed = BTreeMap::new();
    let mut last: Option<CaptureResult> = None;
    let mut passes = 0;
    let mut stable = false;
    while passes < max_passes {
        passes += 1;
        let cap = runner(passes)?;
        let out_dir = cap.out_dir.clone();
        let out_dir = out_dir.as_path();
        let bcf = out_dir.join(format!("{jobname}.bcf"));
        let aux = out_dir.join(format!("{jobname}.aux"));
        let wants_bib = match bib {
            BibTool::None => false,
            BibTool::Biber => bcf.exists(),
            BibTool::BibTeX => std::fs::read_to_string(&aux)
                .map(|s| s.contains("\\citation") || s.contains("\\bibdata"))
                .unwrap_or(false),
            BibTool::Auto => {
                bcf.exists()
                    || std::fs::read_to_string(&aux)
                        .map(|s| s.contains("\\bibdata"))
                        .unwrap_or(false)
            }
        };
        let bib_now = wants_bib.then(|| bib_input(out_dir, &jobname));
        // again when a pass cites more (an entry whose note cites another entry)
        if wants_bib && bib_ran_on != bib_now {
            let tool = if bcf.exists() { "biber" } else { "bibtex" };
            let mut cmd = Command::new(tool);
            // in the main file's folder, as `latexmk -cd` runs it: a document-relative database
            // (`\bibliography{../../refs}`) is found from there; the outputs go to the pass
            // directory
            cmd.current_dir(&main_dir);
            if tool == "biber" {
                cmd.arg(format!("--output-directory={}", crate::paths::tex(out_dir)))
                    .arg(&jobname);
            } else {
                // writing the .bbl outside the working directory
                cmd.arg(crate::paths::tex(&out_dir.join(&jobname)))
                    .env("openout_any", "a")
                    .env("MIKTEX_ALLOWUNSAFEOUTPUTFILES", "1");
            }
            if let Some(d) = &tl.bin_dir {
                crate::paths::prepend_bin_dir(&mut cmd, d);
            }
            // .bib and .bst files at the top of the snapshot; biblatex's BibTeX backend
            // writes its control database (<jobname>-blx.bib) next to the aux file
            let search = format!(
                "{}{sep}{}{sep}",
                crate::paths::tex(snapshot_dir),
                crate::paths::tex(out_dir),
                sep = crate::paths::SEARCH_SEP
            );
            cmd.env("BIBINPUTS", &search).env("BSTINPUTS", &search);
            let out = cmd.output().with_context(|| format!("running {tool}"))?;
            if !out.status.success() {
                log::warn!("{tool} failed: {}", String::from_utf8_lossy(&out.stderr));
            }
            bib_ran_on = bib_now;
        }
        run_makeindex(tl, snapshot_dir, out_dir, &jobname, &mut indexed);
        let sig_after = aux_signature(out_dir, &jobname);
        // a bibliography run that changed nothing (same .bbl) needs no extra pass: the .bbl is
        // part of the signature
        let rerun = log_requests_rerun(&cap.log, bib_ran_on.is_some()) || sig_after != sig_before;
        sig_before = sig_after;
        if rerun && passes < max_passes {
            // another pass follows: the caller may show this one meanwhile
            on_pass(&cap, passes);
        }
        last = Some(cap);
        if !rerun {
            stable = true;
            break;
        }
    }
    Ok(PassOutcome {
        capture: last.unwrap(),
        passes,
        bib_ran: bib_ran_on.is_some(),
        aux_stable: stable,
    })
}
