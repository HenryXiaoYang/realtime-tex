//! The engine thread: owns the fast server, sends queued requests (the latest per paragraph),
//! collects results (direct or queued), and runs probes and warm-ups. `engine_server` starts and
//! replaces the server; `engine_results` turns a result into a paragraph update or a demotion.

use super::*;

pub(super) fn engine_thread(s: Arc<Shared>) {
    EngineLoop {
        s,
        server: None,
        server_generation: u64::MAX,
        labels_sent: 0,
        server_bbl: None,
        standby: None,
        bbl_checked: u64::MAX,
        swapped_from: None,
        failed_start: None,
    }
    .run();
}

/// The engine thread's own state: the server it owns and what it has sent to that server.
pub(super) struct EngineLoop {
    pub(super) s: Arc<Shared>,
    pub(super) server: Option<FastServer>,
    pub(super) server_generation: u64,
    pub(super) labels_sent: u64,
    /// Hash of the `.bbl` the running server read at `\begin{document}` (None: it had none).
    /// biblatex reads its data once, so a newer one needs a new server (`refresh_bibliography`).
    pub(super) server_bbl: Option<u64>,
    /// A server starting with a newer `.bbl` while the running one keeps serving.
    pub(super) standby: Option<Standby>,
    /// The layout version whose `.bbl` was last compared with the server's.
    pub(super) bbl_checked: u64,
    /// (replaced, current) generations of the last standby swap: requests queued for the
    /// replaced server are still valid (same preamble) and are carried over, as long as the
    /// generation is still the one the swap installed.
    pub(super) swapped_from: Option<(u64, u64)>,
    /// (generation, layout version) of the last start that failed: the same preamble with the
    /// same `.aux` fails the same way, so it is not started again until one of them changes.
    pub(super) failed_start: Option<(u64, u64)>,
}

/// What the probe of a unit decided for this iteration.
pub(super) enum ProbeOutcome {
    /// Proven (now or earlier for this layout): send the compile.
    Send(FastRequest),
    /// Handled (demoted, requeued, or the engine failed): next iteration.
    Done,
}

impl EngineLoop {
    /// One iteration per request or event: 1. collect a result in flight; 2. take the next
    /// queued request; 3. (re)start the server; 3b. send labels; 4. send the context; 4b. probe
    /// (probe mode); 5. send the compile. A step that settles the iteration returns false (or
    /// nothing to go on with) and the loop starts over.
    pub(super) fn run(mut self) {
        loop {
            if self.s.shutdown.load(Ordering::SeqCst) {
                reset_link(&self.s);
                if let Some(mut srv) = self.server.take() {
                    let _ = srv.shutdown();
                }
                return;
            }
            let wanted_gen = self.s.engine_generation.load(Ordering::SeqCst);
            if self.collect_inflight(wanted_gen) {
                continue;
            }
            if self.refresh_bibliography(wanted_gen) {
                continue;
            }
            let Some(req) = self.next_request(wanted_gen) else {
                continue;
            };
            if !self.ensure_server(wanted_gen) || !self.sync_labels(wanted_gen) {
                continue;
            }
            let Some(req) = req else { continue };
            self.serve(req, wanted_gen);
        }
    }

    /// Step 1: A compile is in flight (sent by us or directly by the host): read its result. True
    /// when there was one (the iteration is settled).
    pub(super) fn collect_inflight(&mut self, wanted_gen: u64) -> bool {
        let s = self.s.clone();
        let inflight_gen = s
            .link
            .lock()
            .inflight
            .as_ref()
            .map(|f| f.req.versions.engine_generation);
        let Some(gen) = inflight_gen else {
            return false;
        };
        let srv_ok = self.server.is_some() && self.server_generation == gen && gen == wanted_gen;
        if !srv_ok {
            s.link.lock().inflight = None;
            return true;
        }
        let srv = self.server.as_mut().unwrap();
        let result = srv.recv();
        srv.timeout = compile_timeout(&s.cfg);
        let Some(fl) = s.link.lock().inflight.take() else {
            return true;
        };
        let failure = match result {
            Ok(Response::Result(cr)) => {
                let rt = crate::engine::RoundTrip {
                    total: fl.t0.elapsed(),
                    t_tex: Duration::from_micros(cr.t_tex_us as u64),
                    t_traverse: Duration::from_micros(cr.t_traverse_us as u64),
                    t_pack: Duration::from_micros(cr.t_pack_us as u64),
                };
                handle_result(&s, fl.req, *cr, rt, fl.t0, wanted_gen);
                return true;
            }
            Ok(Response::Fatal {
                reason,
                errors,
                before,
                after,
                ..
            }) => anyhow!("engine fatal: {reason} {errors:?} before=[{before}] after=[{after}]"),
            Ok(other) => anyhow!("unexpected response {other:?}"),
            Err(e) => e,
        };
        engine_failed(&s, &fl.req, failure, wanted_gen, self.server.as_ref());
        self.drop_server();
        true
    }

    /// Step 2: The next queued request (the smallest unit id). None: the iteration is settled
    /// (nothing to do and the server is fine: waited for a signal; or the request was
    /// superseded). Some(None): nothing queued but the server needs attention.
    pub(super) fn next_request(&mut self, wanted_gen: u64) -> Option<Option<FastRequest>> {
        let s = &self.s;
        let req = {
            let mut p = s.pending.lock();
            let key = p.keys().next().copied();
            key.and_then(|k| p.remove(&k))
        };
        if req.is_none()
            && self.server.is_some()
            && self.server_generation == wanted_gen
            && self.server.as_mut().unwrap().is_alive()
        {
            let _ = s.pending_signal.1.recv_timeout(Duration::from_millis(200));
            return None;
        }
        let mut req = req;
        if let Some(r) = &mut req {
            if self.swapped_from == Some((r.versions.engine_generation, wanted_gen)) {
                // queued for the server a standby replaced: same preamble, still valid
                r.versions.engine_generation = wanted_gen;
            }
            if r.versions.engine_generation != wanted_gen {
                return None; // superseded by a preamble change
            }
        }
        Some(req)
    }

    /// Steps 4, 4b and 5: context, probe, compile frame, under the link lock (released around the
    /// probe compile and every side effect that takes it again).
    pub(super) fn serve(&mut self, mut req: FastRequest, wanted_gen: u64) {
        let s = self.s.clone();
        let mut link = s.link.lock();
        if !self.ensure_context(&mut link, &mut req, wanted_gen) {
            return;
        }
        if req.probe {
            match self.prove(&mut link, req, wanted_gen) {
                ProbeOutcome::Send(r) => req = r,
                ProbeOutcome::Done => return,
            }
        }
        // 5. the compile frame; the result is collected by step 1
        let req_id = link.next_req;
        link.next_req += 1;
        let t0 = Instant::now();
        let srv = self.server.as_mut().unwrap();
        match srv.send_compile(req_id, req.seq, &req.source, &req.pics) {
            Ok(()) => {
                link.inflight = Some(InFlight { req, t0 });
            }
            Err(e) => {
                drop(link);
                engine_failed(&s, &req, e, wanted_gen, self.server.as_ref());
                self.drop_server();
            }
        }
    }

    /// Step 4: Send the context when the server does not hold it. False: the iteration is settled
    /// (the context is gone, or the server failed).
    pub(super) fn ensure_context(
        &mut self,
        link: &mut parking_lot::MutexGuard<'_, EngineLink>,
        req: &mut FastRequest,
        wanted_gen: u64,
    ) -> bool {
        let s = self.s.clone();
        if link.context_rev_sent != req.versions.context_revision {
            link.contexts_sent.clear();
            link.context_rev_sent = req.versions.context_revision;
        }
        if link.contexts_sent.contains(&req.seq) {
            return true;
        }
        let ctx = match req.ctx.take() {
            Some(c) => Some(c),
            None => {
                // built lazily from the current layout; the unit may have a new id there
                let layout = s.layout.lock();
                layout.unit(req.par_id).map(|eu| {
                    req.seq = eu.uid;
                    eu.context_json()
                })
            }
        };
        let Some(ctx) = ctx else {
            parking_lot::MutexGuard::unlocked(link, || {
                s.events
                    .send(Event::BackgroundScheduled {
                        par_id: Some(req.par_id),
                        reasons: vec!["context no longer available".into()],
                        edit_id: req.edit_id,
                    })
                    .ok();
            });
            return false;
        };
        if link.contexts_sent.contains(&req.seq) {
            // the lazy lookup mapped to a context already installed
            return true;
        }
        let srv = self.server.as_mut().unwrap();
        match srv.set_context(req.seq, &ctx) {
            Ok(()) => {
                link.contexts_sent.insert(req.seq);
                true
            }
            Err(e) => {
                parking_lot::MutexGuard::unlocked(link, || {
                    s.events
                        .send(Event::EngineState {
                            engine_generation: wanted_gen,
                            state: "Restarting".into(),
                            reason: Some(format!("set_context: {e}")),
                        })
                        .ok();
                    self.drop_server();
                });
                false
            }
        }
    }

    /// Step 4b: Probe mode: prove the unit first — its snapshot text (from the layout current now)
    /// compiled and compared with that layout's rows. The compile runs without the link lock
    /// (the host's apply_edit must not wait on it); `probing` keeps the direct dispatch path
    /// off the server meanwhile. A verdict is kept for the layout version.
    pub(super) fn prove(
        &mut self,
        link: &mut parking_lot::MutexGuard<'_, EngineLink>,
        req: FastRequest,
        wanted_gen: u64,
    ) -> ProbeOutcome {
        let s = self.s.clone();
        let (text, lv) = {
            let layout = s.layout.lock();
            (
                layout.snapshot_text(req.par_id).map(|t| t.to_string()),
                layout.layout_version,
            )
        };
        // bound first: a guard in a `match` scrutinee would live through the arms, and the
        // arm that stores the verdict takes the same lock again (a self-deadlock)
        let cached = s.live.lock().probe_for(req.par_id, lv);
        let verdict = match cached {
            Some(v) => v,
            None => {
                let Some(text) = text else {
                    parking_lot::MutexGuard::unlocked(link, || {
                        demote_span(&s, &req, "no snapshot to compare with")
                    });
                    return ProbeOutcome::Done;
                };
                link.probing = true;
                let t_probe = Instant::now();
                let srv = self.server.as_mut().unwrap();
                let res = parking_lot::MutexGuard::unlocked(link, || {
                    srv.compile_with_pics(req.seq, &text, &req.pics)
                });
                link.probing = false;
                link.probes += 1;
                link.probe_us += t_probe.elapsed().as_micros() as u64;
                let requeue = |req: FastRequest| {
                    s.pending.lock().insert(req.par_id, req);
                    s.pending_signal.0.send(()).ok();
                };
                let v = match res {
                    Ok((cr, _))
                        if req.pics_n > 0
                            && cr.pics_seen.is_some_and(|n| n != req.pics_n as i64) =>
                    {
                        // the cache entries were scanned from the current text, the probe
                        // compiles the snapshot's: when they do not pair up the probe is
                        // repeated without them (the pictures are drawn)
                        let mut req = req;
                        req.pics.clear();
                        req.pics_n = 0;
                        parking_lot::MutexGuard::unlocked(link, || requeue(req));
                        return ProbeOutcome::Done;
                    }
                    Ok((cr, _)) if cr.internal.is_some() => ProbeVerdict::Mismatch(format!(
                        "internal error ({})",
                        first_line_of(cr.internal.as_deref().unwrap_or_default())
                    )),
                    Ok((cr, _)) => {
                        if !cr.leaks.is_empty() {
                            ProbeVerdict::Leak(format!(
                                "the unit redefines \\{}",
                                cr.leaks.join(", \\")
                            ))
                        } else if cr.status == "error" {
                            ProbeVerdict::Mismatch(format!(
                                "probe compile failed: {}",
                                cr.errors
                                    .first()
                                    .and_then(|e| e.message.clone())
                                    .unwrap_or_default()
                            ))
                        } else {
                            let layout = s.layout.lock();
                            if layout.layout_version != lv {
                                // the layout moved while the probe ran: judge against the new
                                // one (the request goes back to the queue)
                                drop(layout);
                                parking_lot::MutexGuard::unlocked(link, || requeue(req));
                                return ProbeOutcome::Done;
                            }
                            match &cr.dl {
                                Some(dl) => match layout.probe_check(req.par_id, dl) {
                                    Ok(()) => ProbeVerdict::Verified,
                                    Err(why) => ProbeVerdict::Mismatch(why),
                                },
                                None => ProbeVerdict::Mismatch("probe produced no box".into()),
                            }
                        }
                    }
                    Err(e) => {
                        parking_lot::MutexGuard::unlocked(link, || {
                            engine_failed(&s, &req, e, wanted_gen, self.server.as_ref());
                            self.drop_server();
                        });
                        return ProbeOutcome::Done;
                    }
                };
                s.live.lock().unit(req.par_id).probe = Some((lv, v.clone()));
                v
            }
        };
        match verdict {
            ProbeVerdict::Verified => ProbeOutcome::Send(req),
            ProbeVerdict::Mismatch(why) => {
                parking_lot::MutexGuard::unlocked(link, || demote_span(&s, &req, &why));
                ProbeOutcome::Done
            }
            ProbeVerdict::Leak(why) => {
                parking_lot::MutexGuard::unlocked(link, || {
                    s.live.lock().unit(req.par_id).leak = Some(why.clone());
                    demote_span(&s, &req, &why);
                    // the server's state is no longer the document's: start over
                    s.engine_generation.fetch_add(1, Ordering::SeqCst);
                    self.drop_server();
                });
                ProbeOutcome::Done
            }
        }
    }
}
