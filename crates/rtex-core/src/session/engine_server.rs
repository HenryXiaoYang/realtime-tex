//! The live engine's lifecycle, as the engine thread (`engine_loop`) drives it: starting a server for the current preamble, a standby that replaces it when the bibliography changed, and sending it the labels of the last pass.

use super::*;

/// A server for the latest bibliography (`refresh_bibliography`): starting on a helper thread,
/// then ready and waiting for the running server to be idle.
pub(super) struct Standby {
    pub(super) generation: u64,
    pub(super) state: StandbyState,
}

pub(super) enum StandbyState {
    Starting(std::thread::JoinHandle<Result<FastServer>>),
    Ready(Box<FastServer>),
}

/// Hash of the `.bbl` next to `aux` (None: there is none).
pub(super) fn bbl_hash(aux: Option<&Path>) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let bytes = std::fs::read(aux?.with_extension("bbl")).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    Some(h.finish())
}

/// Hash of the `.bbl` a started server read: the copy it made at start, not the pass's file,
/// which a later pass in the same directory may already have rewritten.
pub(super) fn server_bbl_hash(srv: &FastServer) -> Option<u64> {
    bbl_hash(Some(
        &srv.work_dir
            .join(format!("rtex-serve-g{}.aux", srv.generation)),
    ))
}

/// The aux file of the latest layout's pass (a server starts from it and its `.bbl`), with
/// that layout's version, read together.
pub(super) fn layout_aux(s: &Shared) -> (Option<PathBuf>, u64) {
    let layout = s.layout.lock();
    let jobname = Path::new(&s.cfg.main_file)
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("main")
        .to_string();
    (
        layout
            .capture_dir
            .as_ref()
            .map(|d| d.join(format!("{jobname}.aux"))),
        layout.layout_version,
    )
}

/// The link fields that describe the current server, cleared when it goes away.
pub(super) fn reset_link(s: &Shared) {
    let mut l = s.link.lock();
    l.writer = None;
    l.inflight = None;
    l.contexts_sent.clear();
}

impl EngineLoop {
    /// Drop the server (its process is killed) and the link's view of it.
    pub(super) fn drop_server(&mut self) {
        self.server = None;
        reset_link(&self.s);
    }

    /// Step 1b: biblatex reads its `.bbl` once, at `\begin{document}`: a server started before
    /// the pass that wrote the bibliography (a session opened without an earlier build), or
    /// before the bibliography changed, prints citations as unresolved keys, and the probe
    /// keeps every citing unit on the pass. When the latest layout's `.bbl` differs from the
    /// server's, a server is started with it on a helper thread while the running one keeps
    /// serving, and replaces it once ready and idle. True: the iteration is settled (swapped).
    pub(super) fn refresh_bibliography(&mut self, wanted_gen: u64) -> bool {
        let s = self.s.clone();
        if let Some(mut sb) = self.standby.take() {
            if let StandbyState::Starting(handle) = sb.state {
                if !handle.is_finished() {
                    sb.state = StandbyState::Starting(handle);
                    self.standby = Some(sb);
                    return false;
                }
                match handle.join() {
                    Ok(Ok(srv)) => sb.state = StandbyState::Ready(Box::new(srv)),
                    Ok(Err(e)) => {
                        log::warn!("standby server for the new bibliography: {e:#}");
                        return false;
                    }
                    Err(_) => return false,
                }
            }
            let StandbyState::Ready(mut srv) = sb.state else {
                unreachable!()
            };
            // only onto the server it was started to replace (a preamble change meanwhile
            // restarts the engine anyway)
            if self.server_generation != wanted_gen || sb.generation != wanted_gen + 1 {
                srv.kill();
                return false;
            }
            // and only while nothing is in flight or queued (try again next iteration); the
            // check and the swap of the link happen under one lock, so a host thread cannot
            // write a compile to the old server in between (it sees the new generation and
            // queues the request, which `next_request` carries over)
            srv.timeout = compile_timeout(&s.cfg).max(Duration::from_secs(30)); // first compile loads fonts
            {
                let mut l = s.link.lock();
                let idle = l.inflight.is_none() && s.pending.lock().is_empty();
                if !idle
                    || s.engine_generation
                        .compare_exchange(
                            wanted_gen,
                            sb.generation,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .is_err()
                {
                    drop(l);
                    sb.state = StandbyState::Ready(srv);
                    self.standby = Some(sb);
                    return false;
                }
                l.writer = srv.stdin_clone().ok();
                l.generation = sb.generation;
                l.contexts_sent.clear();
                l.inflight = None;
            }
            if let Some(mut old) = self.server.replace(*srv) {
                let _ = old.shutdown();
            }
            self.swapped_from = Some((self.server_generation, sb.generation));
            self.server_generation = sb.generation;
            self.server_bbl = self.server.as_ref().and_then(server_bbl_hash);
            self.labels_sent = 0;
            // verdicts reached against the old server's citations no longer apply
            s.live.lock().on_server_swap();
            s.events
                .send(Event::EngineState {
                    engine_generation: sb.generation,
                    state: "Ready".into(),
                    reason: Some("restarted with the bibliography of the latest layout".into()),
                })
                .ok();
            return true;
        }
        let (aux, lv) = layout_aux(&s);
        if lv == self.bbl_checked
            || self.server_generation != wanted_gen
            || !self.server.as_mut().is_some_and(|srv| srv.is_alive())
        {
            return false;
        }
        self.bbl_checked = lv;
        let bbl = bbl_hash(aux.as_deref());
        if bbl.is_none() || bbl == self.server_bbl {
            return false;
        }
        let preamble = effective_preamble(&texts_of(&s.files.lock()), &s.cfg.main_file);
        let generation = wanted_gen + 1;
        let s2 = s.clone();
        let handle = std::thread::spawn(move || {
            FastServer::spawn_with(
                &s2.tl,
                &s2.cfg.project_root,
                &s2.cfg.build_dir.join("serve-next"),
                &preamble,
                generation,
                aux.as_deref(),
                s2.cfg.debug_dir.is_some(),
            )
        });
        self.standby = Some(Standby {
            generation,
            state: StandbyState::Starting(handle),
        });
        false
    }

    /// Step 3: (Re)start the server when the generation changed or it died. A preamble edit bumps
    /// the generation per keystroke; wait until it has been quiet for the debounce time so a
    /// burst of preamble keystrokes costs one restart, not one per keystroke. False: the
    /// iteration is settled (the generation moved on, or the start failed).
    pub(super) fn ensure_server(&mut self, wanted_gen: u64) -> bool {
        let s = self.s.clone();
        let healthy = self.server_generation == wanted_gen
            && self.server.as_mut().is_some_and(|srv| srv.is_alive());
        if healthy {
            return true;
        }
        if self.server.is_some() && self.server_generation != wanted_gen {
            let mut g = wanted_gen;
            loop {
                let _ = s.pending_signal.1.recv_timeout(s.cfg.debounce);
                let now = s.engine_generation.load(Ordering::SeqCst);
                if now == g || s.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                g = now;
            }
            if g != wanted_gen {
                return false; // re-evaluate with the settled generation
            }
        }
        reset_link(&s);
        if let Some(mut old) = self.server.take() {
            old.kill();
        }
        // the layout the server starts from: its .bbl is compared with later layouts'
        let (aux, aux_layout) = layout_aux(&s);
        if self.failed_start == Some((wanted_gen, aux_layout)) {
            // wait for a preamble edit (a new generation) or a new pass instead of retrying
            let _ = s.pending_signal.1.recv_timeout(Duration::from_millis(200));
            return false;
        }
        let preamble = effective_preamble(&texts_of(&s.files.lock()), &s.cfg.main_file);
        s.events
            .send(Event::EngineState {
                engine_generation: wanted_gen,
                state: "Starting".into(),
                reason: None,
            })
            .ok();
        match FastServer::spawn_with(
            &s.tl,
            &s.cfg.project_root,
            &s.cfg.build_dir.join("serve"),
            &preamble,
            wanted_gen,
            aux.as_deref(),
            s.cfg.debug_dir.is_some(),
        ) {
            Ok(mut srv) => {
                srv.timeout = compile_timeout(&s.cfg).max(Duration::from_secs(30)); // first compile loads fonts
                {
                    let mut l = s.link.lock();
                    l.writer = srv.stdin_clone().ok();
                    l.generation = wanted_gen;
                    l.contexts_sent.clear();
                    l.inflight = None;
                }
                self.server_bbl = server_bbl_hash(&srv);
                self.server = Some(srv);
                self.server_generation = wanted_gen;
                self.bbl_checked = aux_layout;
                self.labels_sent = 0;
                s.events
                    .send(Event::EngineState {
                        engine_generation: wanted_gen,
                        state: "Ready".into(),
                        reason: None,
                    })
                    .ok();
                true
            }
            Err(e) => {
                let reason = debug_bundle_startup(&s, wanted_gen, &format!("{e:#}"));
                s.events
                    .send(Event::EngineState {
                        engine_generation: wanted_gen,
                        state: "Failed".into(),
                        reason: Some(reason),
                    })
                    .ok();
                self.failed_start = Some((wanted_gen, aux_layout));
                false
            }
        }
    }

    /// Step 3b: Labels (\newlabel/\bibcite of the last pass) when they changed; done while idle
    /// when possible, and before a compile otherwise. False: the server failed (dropped).
    pub(super) fn sync_labels(&mut self, wanted_gen: u64) -> bool {
        let s = self.s.clone();
        let (labels, labels_hash) = {
            let layout = s.layout.lock();
            (layout.labels.clone(), layout.labels_hash)
        };
        if labels_hash == self.labels_sent || labels.is_empty() {
            return true;
        }
        let mut link = s.link.lock();
        if link.inflight.is_some() {
            return true;
        }
        let srv = self.server.as_mut().unwrap();
        match srv.set_labels(&labels) {
            Ok(()) => {
                self.labels_sent = labels_hash;
                link.labels_sent = labels_hash;
                true
            }
            Err(e) => {
                drop(link);
                s.events
                    .send(Event::EngineState {
                        engine_generation: wanted_gen,
                        state: "Restarting".into(),
                        reason: Some(format!("labels: {e}")),
                    })
                    .ok();
                self.drop_server();
                false
            }
        }
    }
}
