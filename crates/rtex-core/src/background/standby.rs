//! `WarmEngine`: a LuaLaTeX that has loaded the preamble and waits for the body, so a pass skips the start-up.

use super::snapshot::*;
use crate::capture::{capture_command, collect_capture, CaptureResult};
use crate::texlive::TexLive;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct WarmEngine {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    pub preamble_hash: u64,
    pub src_dir: PathBuf,
    out_dir: PathBuf,
    jobname: String,
    instrumented: bool,
    pub spawned: std::time::Instant,
}

/// What a standby engine runs: the project and its sources, where the body snapshot goes and
/// where the pass writes.
#[derive(Clone, Copy)]
pub struct StandbySpec<'a> {
    pub tl: &'a TexLive,
    pub project: &'a Path,
    pub files: &'a BTreeMap<String, String>,
    pub main: &'a str,
    pub src_dir: &'a Path,
    pub out_dir: &'a Path,
    pub instrumented: bool,
    pub unit_envs: &'a str,
}

impl WarmEngine {
    /// Start a standby: writes the body snapshot into `src_dir` and runs lualatex up to the end
    /// of the preamble, where it blocks on stdin (tex/rtex-bg.lua).
    pub fn spawn(spec: StandbySpec) -> Result<WarmEngine> {
        let StandbySpec {
            tl,
            project,
            files,
            main,
            src_dir,
            out_dir,
            instrumented,
            unit_envs,
        } = spec;
        let preamble_hash = write_body_snapshot(project, files, main, src_dir)?;
        let (mut cmd, jobname, out_dir) =
            capture_command(tl, src_dir, main, out_dir, instrumented, unit_envs)?;
        let pkg = if instrumented {
            "\\RequirePackage{rtex-capture}"
        } else {
            ""
        };
        let hook = crate::texlive::INPUT_PATH_HOOK;
        cmd.arg(format!("{hook}{pkg}\\input{{rtex-preamble.tex}}\\directlua{{dofile(kpse.find_file(\"rtex-bg.lua\",\"lua\") or \"rtex-bg.lua\")}}\\input{{{main}}}"));
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().context("spawning standby lualatex")?;
        let stdin = child.stdin.take();
        Ok(WarmEngine {
            child,
            stdin,
            preamble_hash,
            src_dir: src_dir.to_path_buf(),
            out_dir,
            jobname,
            instrumented,
            spawned: std::time::Instant::now(),
        })
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Where this engine writes its pass (PDF, log, capture JSON, aux family).
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// Typeset the body: refresh the snapshot texts (same preamble), release the engine and
    /// collect the pass like a fresh run (killed after `limit`).
    pub fn run(
        mut self,
        project: &Path,
        files: &BTreeMap<String, String>,
        main: &str,
        limit: Option<std::time::Duration>,
    ) -> Result<CaptureResult> {
        let t0 = std::time::Instant::now();
        let h = write_body_snapshot(project, files, main, &self.src_dir)?;
        if h != self.preamble_hash {
            anyhow::bail!("standby preamble differs from the snapshot");
        }
        {
            use std::io::Write;
            let mut stdin = self
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("standby stdin already closed"))?;
            stdin.write_all(b"GO\n").context("releasing standby")?;
            stdin.flush().ok();
            drop(stdin);
        }
        let (status, stdout) = crate::capture::wait_within(&mut self.child, limit)?;
        collect_capture(
            &self.out_dir,
            &self.jobname,
            self.instrumented,
            status,
            &stdout,
            t0.elapsed(),
        )
    }

    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for WarmEngine {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
