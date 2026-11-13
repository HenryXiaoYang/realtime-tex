//! The public session API: documents, edits, fast-path compiles, background layouts, events.
//!
//! `Session` is opened, queried and polled here. Its parts:
//! - `config`: what a host configures; `events`: what it is told.
//! - `edit`: applying an edit and routing the touched units (`route` decides live or background).
//! - `engine_loop`: the thread that owns the live engine, with `engine_server` (its
//!   lifecycle) and `engine_results` (results into updates); `live`: what is known about
//!   each unit between layouts.
//! - `passes`: the background thread, with `pass_standby` (pass directories, the standby
//!   engine), `pass_deliver` (a pass into a layout) and `export`.
//! - `state`: what those threads share; `diag`: diagnostics from logs.

use crate::background::{
    run_pass_with, run_pass_with_runner, snapshot_dir, write_snapshot, BibTool, WarmEngine,
};
use crate::document::{Edit, EditOutcome, FileBuf, IdAllocator, ParaId, Revision, Span, SpanKind};
use crate::eligibility::{
    check_engine_unit, classify_source, classify_source_with, everypar_allowed, runs_into_title,
    Policy, Reason, UnitShape,
};
use crate::engine::{FastServer, Response};
use crate::layout::{EngineUnit, Fragment, LayoutStore, SnapshotSpan};
use crate::texlive::TexLive;
use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use rtex_dl::DisplayList;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod config;
mod diag;
mod edit;
mod engine_loop;
mod engine_results;
mod engine_server;
mod events;
mod export;
mod live;
mod pass_deliver;
mod pass_standby;
mod passes;
mod route;
mod state;

pub use config::*;
pub use diag::parse_log;
use diag::*;
use engine_loop::*;
use engine_results::*;
use engine_server::*;
pub use events::*;
use export::*;
use live::*;
use pass_deliver::*;
use pass_standby::*;
use passes::*;
use state::*;

pub struct Session {
    shared: Arc<Shared>,
    events: Receiver<Event>,
    /// Events taken out by a poll but handed back (C ABI returns one event per call).
    requeued: Mutex<std::collections::VecDeque<Event>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Session {
    pub fn open(cfg: SessionConfig) -> Result<Session> {
        let mut tl = TexLive::discover()?;
        let project_root = crate::paths::canonical(&cfg.project_root).context("project root")?;
        tl.project_dir = Some(project_root.clone());
        tl.main_dir = crate::paths::folder(&cfg.main_file);
        tl.math_owner_attribute = cfg.math_owner_attribute;
        let cfg = SessionConfig {
            project_root,
            main_file: crate::paths::key(&cfg.main_file),
            ..cfg
        };
        std::fs::create_dir_all(&cfg.build_dir)?;
        let cfg = SessionConfig {
            build_dir: crate::paths::canonical(&cfg.build_dir)?,
            ..cfg
        };
        // layout PDFs are numbered per session: an earlier session's copies would stay for
        // as long as this one takes to reach their numbers
        if let Ok(rd) = std::fs::read_dir(cfg.build_dir.join("bg")) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if name.starts_with("layout-") && name.ends_with(".pdf") {
                    let _ = std::fs::remove_file(ent.path());
                }
            }
        }
        // the main file and, transitively, every file it \input/\includes
        let texts = crate::document::load_project_files(&cfg.project_root, &cfg.main_file)
            .with_context(|| format!("reading {}", cfg.main_file))?;
        let mut ids = IdAllocator(0);
        let mut files = BTreeMap::new();
        let preamble = effective_preamble(&texts, &cfg.main_file);
        let preamble_inputs = preamble_input_set(&texts, &cfg.main_file);
        let mut policy = Policy::from_preamble(&preamble, &cfg.trusted_macros, &cfg.unit_envs);
        policy.permissive = cfg.eligibility == EligibilityMode::Probe;
        policy.math_owner_attribute = cfg.math_owner_attribute;
        let rules = policy.segment_rules();
        // the main file first so its span ids come first
        files.insert(
            cfg.main_file.clone(),
            FileBuf::with_rules(&texts[&cfg.main_file], &mut ids, 1, rules.clone()),
        );
        for (name, text) in texts.iter().filter(|(n, _)| **n != cfg.main_file) {
            files.insert(
                name.clone(),
                FileBuf::with_rules(text, &mut ids, 1, rules.clone()),
            );
        }
        let (tx, rx) = unbounded();
        let shared = Arc::new(Shared {
            link: Mutex::new(EngineLink {
                next_req: 1,
                ..Default::default()
            }),
            files: Mutex::new(files),
            ids: Mutex::new(ids),
            source_revision: AtomicU64::new(1),
            preamble_revision: AtomicU64::new(1),
            bg_change: Mutex::new(HashMap::new()),
            layout: Mutex::new(LayoutStore::default()),
            engine_generation: AtomicU64::new(0),
            events: tx,
            pending: Mutex::new(BTreeMap::new()),
            pending_signal: unbounded(),
            bg_signal: unbounded(),
            shutdown: AtomicBool::new(false),
            bg_running: AtomicBool::new(false),
            bg_paused: AtomicBool::new(false),
            bg_pending_while_paused: AtomicBool::new(false),
            cfg,
            tl,
            policy: Mutex::new(policy),
            preamble_inputs: Mutex::new(preamble_inputs),
            inputted: Mutex::new(crate::document::inputted_files(&texts)),
            edit_counter: AtomicU64::new(0),
            live: Mutex::new(LiveUnits::default()),
            convergence: Mutex::new(None),
            standby: Mutex::new(None),
            last_pass_dir: Mutex::new(None),
            pic_index: Mutex::new(None),
            pic_scan: Mutex::new(None),
            fast_failures: Mutex::new(BTreeMap::new()),
        });
        let mut threads = Vec::new();
        {
            let s = shared.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("rtex-engine".into())
                    .spawn(move || engine_thread(s))?,
            );
        }
        {
            let s = shared.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("rtex-background".into())
                    .spawn(move || background_thread(s))?,
            );
        }
        shared.bg_signal.0.send(BgCmd::Pass).ok();
        shared.pending_signal.0.send(()).ok(); // eager server start
        Ok(Session {
            shared,
            events: rx,
            requeued: Mutex::new(std::collections::VecDeque::new()),
            threads,
        })
    }

    pub fn versions(&self) -> Versions {
        let layout = self.shared.layout.lock();
        Versions {
            source_revision: self.shared.source_revision.load(Ordering::SeqCst),
            context_revision: layout.context_revision,
            engine_generation: self.shared.engine_generation.load(Ordering::SeqCst),
            layout_version: layout.layout_version,
        }
    }

    /// Diagnostic view of the installed layout's units: (unit id, kind, first line, last line,
    /// mapped span).
    pub fn layout_units(&self) -> Vec<LayoutUnitInfo> {
        let layout = self.shared.layout.lock();
        layout
            .units
            .iter()
            .map(|u| LayoutUnitInfo {
                uid: u.uid,
                kind: u.captured.kind.clone(),
                name: u.captured.name.clone(),
                file: u.captured.file.clone(),
                first_line: u.captured.begin_line,
                last_line: u.captured.end_line,
                rows: u.rows(),
                span: u.span,
            })
            .collect()
    }

    pub fn convergence(&self) -> Option<Convergence> {
        self.shared.convergence.lock().clone()
    }

    pub fn document_text(&self, rel_path: &str) -> Option<String> {
        let key = self.rel_key(rel_path);
        self.shared.files.lock().get(&key).map(|f| f.text.clone())
    }

    /// Probe compiles run so far (probe mode): a unit whose only vocabulary beyond the
    /// allow-list is cached pictures needs none.
    pub fn probes(&self) -> u64 {
        self.shared.link.lock().probes
    }

    pub fn spans(&self, rel_path: &str) -> Vec<crate::document::Span> {
        let key = self.rel_key(rel_path);
        self.shared
            .files
            .lock()
            .get(&key)
            .map(|f| f.spans.clone())
            .unwrap_or_default()
    }

    pub fn request_layout(&self) {
        self.schedule_background();
    }

    /// Defer background passes (they run when resumed). The fast path keeps working.
    pub fn pause_background(&self, paused: bool) {
        self.shared.bg_paused.store(paused, Ordering::SeqCst);
        if !paused
            && self
                .shared
                .bg_pending_while_paused
                .swap(false, Ordering::SeqCst)
        {
            self.shared.bg_signal.0.send(BgCmd::Pass).ok();
        }
    }

    pub(super) fn schedule_background(&self) {
        *self.shared.convergence.lock() = Some(Convergence::Stale {
            pending_since: self.shared.source_revision.load(Ordering::SeqCst),
        });
        self.shared.bg_signal.0.send(BgCmd::Pass).ok();
    }

    pub fn export_pdf(&self, out: impl Into<PathBuf>) -> u64 {
        let job = self.shared.edit_counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared
            .bg_signal
            .0
            .send(BgCmd::Export(job, out.into()))
            .ok();
        job
    }

    /// Put an event back at the front of the queue (used by the C ABI's one-at-a-time poll).
    pub fn requeue(&self, e: Event) {
        self.requeued.lock().push_back(e);
    }

    /// Drain available events, waiting up to `timeout` for the first one.
    pub fn poll(&self, timeout: Duration) -> Vec<Event> {
        let mut v: Vec<Event> = self.requeued.lock().drain(..).collect();
        if v.is_empty() {
            if let Ok(e) = self.events.recv_timeout(timeout) {
                v.push(e);
            }
        }
        while let Ok(e) = self.events.try_recv() {
            v.push(e);
        }
        v
    }

    /// Wait for the first event matching `pred` (others are kept in order in the returned vec).
    pub fn wait_for(
        &self,
        timeout: Duration,
        mut pred: impl FnMut(&Event) -> bool,
    ) -> (Option<Event>, Vec<Event>) {
        let deadline = Instant::now() + timeout;
        let mut others = Vec::new();
        let queued: Vec<Event> = self.requeued.lock().drain(..).collect();
        for e in queued {
            if pred(&e) {
                return (Some(e), others);
            }
            others.push(e);
        }
        loop {
            let now = Instant::now();
            if now >= deadline {
                return (None, others);
            }
            match self.events.recv_timeout(deadline - now) {
                Ok(e) => {
                    if pred(&e) {
                        return (Some(e), others);
                    }
                    others.push(e);
                }
                Err(_) => return (None, others),
            }
        }
    }

    pub fn close(mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.pending_signal.0.send(()).ok();
        self.shared.bg_signal.0.send(BgCmd::Quit).ok();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.shared.pending_signal.0.send(()).ok();
        self.shared.bg_signal.0.send(BgCmd::Quit).ok();
    }
}

fn reason_str(r: &Reason) -> String {
    match r {
        Reason::DisallowedMacro(m) => format!("macro \\{m}"),
        Reason::DisallowedMathMacro(m) => format!("math macro \\{m}"),
        Reason::SizeDeclarationOutsideGroup(m) => format!("\\{m} outside group"),
        Reason::EngineFlag(s) => format!("engine {s}"),
        Reason::OverBudget(ms) => {
            format!("OverBudget: last fast compile took {ms} ms, over the budget")
        }
        other => format!("{other:?}"),
    }
}

/// The first line of a (multi-line) message, such as a Lua error with its traceback.
fn first_line_of(msg: &str) -> &str {
    msg.lines().next().unwrap_or("")
}
