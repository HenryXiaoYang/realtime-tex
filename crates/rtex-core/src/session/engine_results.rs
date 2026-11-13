//! What the engine thread (`engine_loop`) does with a result of the live engine: a paragraph update, a demotion to the background, or a restart after a failure or timeout.

use super::*;

/// A fast request the probe (or a leak) took away from the live path after `apply_edit`
/// reported it as fast: undo the live bookkeeping (no overlay claims a result at this
/// revision; the change counts as a background change for the spans after it), tell the host,
/// and schedule the pass that will typeset it.
pub(super) fn demote_span(s: &Shared, req: &FastRequest, why: &str) {
    {
        let files = s.files.lock();
        if let Some((name, line)) = files.iter().find_map(|(name, fb)| {
            fb.span(req.par_id)
                .map(|sp| (name.clone(), fb.line_range(sp).0))
        }) {
            s.bg_change
                .lock()
                .entry(name)
                .or_default()
                .push((req.versions.source_revision, line));
        }
    }
    if !req.warmup {
        s.events
            .send(Event::BackgroundScheduled {
                par_id: Some(req.par_id),
                reasons: vec![format!("unverified: {why}")],
                edit_id: req.edit_id,
            })
            .ok();
    }
    s.bg_signal.0.send(BgCmd::Pass).ok();
}

pub(super) fn engine_failed(
    s: &Shared,
    req: &FastRequest,
    e: anyhow::Error,
    wanted_gen: u64,
    srv: Option<&FastServer>,
) {
    let mut message = e.to_string();
    if let Some(dir) = debug_bundle(s, req, &message, wanted_gen, srv) {
        message = format!("{message} (debug bundle: {})", dir.display());
    }
    s.events
        .send(Event::EngineState {
            engine_generation: wanted_gen,
            state: "Restarting".into(),
            reason: Some(message.clone()),
        })
        .ok();
    s.events
        .send(Event::Diagnostics {
            source: format!("fast:{:?}", req.par_id),
            items: vec![Diagnostic {
                severity: "error".into(),
                file: None,
                line: None,
                message,
                context: None,
            }],
        })
        .ok();
    // nothing on the live path reports this unit again (it stays on the background path): the
    // next layout of this revision or a later one withdraws the diagnostic
    s.fast_failures
        .lock()
        .insert(req.par_id, req.versions.source_revision);
    s.engine_generation.fetch_add(1, Ordering::SeqCst);
    // the paragraph goes to the background path, and stays there until it is edited after a
    // clean pass (or, after a few crashes, until the preamble changes): retrying the text that
    // hung or crashed the engine would kill the server on every keystroke
    s.bg_signal.0.send(BgCmd::Pass).ok();
    if !req.warmup {
        s.live
            .lock()
            .quarantine(req.par_id, req.span_hash, req.versions.source_revision);
        s.events
            .send(Event::BackgroundScheduled {
                par_id: Some(req.par_id),
                reasons: vec!["engine restarted".into()],
                edit_id: req.edit_id,
            })
            .ok();
    }
}

/// The per-compile watchdog. With macro tracing on (`$RTEX_TRACE_MACROS` with a debug
/// directory) every macro expansion is written to the TeX log, which makes a heavy compile (a
/// pgfplots axis drawn) many times slower: the watchdog is lengthened rather than letting the
/// debug setting itself kill the engine.
pub(super) fn compile_timeout(cfg: &SessionConfig) -> Duration {
    if cfg.debug_dir.is_some() && std::env::var_os("RTEX_TRACE_MACROS").is_some_and(|v| v != "0") {
        cfg.compile_timeout * 10
    } else {
        cfg.compile_timeout
    }
}

pub(super) fn handle_result(
    s: &Shared,
    req: FastRequest,
    cr: crate::engine::CompileResult,
    rt: crate::engine::RoundTrip,
    t0: Instant,
    wanted_gen: u64,
) {
    if let Some(msg) = cr.internal.as_deref() {
        // the server hit a Lua error of its own on this unit and answered with it (it used to
        // stay silent until the watchdog killed the engine): the unit waits for the pass until
        // the next layout; the engine is fine
        if req.warmup {
            // nothing to demote, and no pass to schedule: a pass would install a layout, which
            // warms the engine again, which would fail again (a loop of passes)
            log::warn!("warm-up compile: internal error in the server: {msg}");
            return;
        }
        let lv = s.layout.lock().layout_version;
        let first = s
            .live
            .lock()
            .unit(req.par_id)
            .internal_error
            .replace((lv, msg.to_string()))
            .is_none_or(|(prev, _)| prev != lv);
        log::warn!("{:?}: internal error in the server: {msg}", req.par_id);
        // one bundle per unit and layout, not one per keystroke
        if first {
            if let Some(dir) =
                debug_bundle(s, &req, &format!("internal error: {msg}"), wanted_gen, None)
            {
                log::warn!("debug bundle: {}", dir.display());
            }
        }
        demote_span(s, &req, &format!("internal error ({})", first_line_of(msg)));
        return;
    }
    if !cr.leaks.is_empty() {
        // before any early return: the compile changed the meaning of a control sequence it
        // mentions, so the server is no longer the document's state whatever else happened.
        // Demote the span until the preamble changes and restart the engine.
        let why = format!("the unit redefines \\{}", cr.leaks.join(", \\"));
        s.live.lock().unit(req.par_id).leak = Some(why.clone());
        demote_span(s, &req, &why);
        s.engine_generation.fetch_add(1, Ordering::SeqCst);
        s.pending_signal.0.send(()).ok();
        return;
    }
    if s.cfg.debug_dir.is_some() {
        debug_request_line(
            s,
            &format!(
                "par {} ctx {} status {} rows {} tex_us {} total_us {} pics {}/{} errors {}{}",
                req.par_id.0,
                req.seq,
                cr.status,
                cr.dl.as_ref().map(|d| d.lines.len()).unwrap_or(0),
                rt.t_tex.as_micros(),
                t0.elapsed().as_micros(),
                cr.pics_used.unwrap_or(0),
                cr.pics_seen.unwrap_or(0),
                cr.errors.len(),
                if req.warmup { " (warm-up)" } else { "" }
            ),
        );
    }
    if req.warmup {
        return;
    }
    // a unit let through unprobed (a borrowed context, or a probe skipped) because every
    // picture in it comes from the cache: when the engine drew one after all (a state
    // mismatch, a picture the scan did not see), the result is unverified and the unit waits
    // for the pass
    if req.pics_required && cr.status != "error" && cr.pics_used != Some(req.pics_n as i64) {
        demote_span(
            s,
            &req,
            "a picture drawn outside the cache on a borrowed context",
        );
        return;
    }
    // the engine saw a different number of picture environments than the source scan (a
    // picture made by a macro, a nested one): its cache entries may have gone to the wrong
    // pictures, so the result is dropped and the compile repeated without them
    if req.pics_n > 0 && cr.pics_seen.is_some_and(|n| n != req.pics_n as i64) {
        log::warn!(
            "{:?}: {} pictures scanned, engine saw {:?}; compiling without the cache",
            req.par_id,
            req.pics_n,
            cr.pics_seen
        );
        let mut again = req;
        again.pics.clear();
        again.pics_n = 0;
        let mut pending = s.pending.lock();
        pending.entry(again.par_id).or_insert(again);
        drop(pending);
        s.pending_signal.0.send(()).ok();
        return;
    }
    // discard rule: the span changed meanwhile, or the generation moved on
    let current_hash = s
        .files
        .lock()
        .values()
        .find_map(|fb| fb.span(req.par_id).map(|sp| sp.hash));
    if current_hash != Some(req.span_hash)
        || s.engine_generation.load(Ordering::SeqCst) != wanted_gen
    {
        return;
    }
    let diagnostics: Vec<Diagnostic> = cr
        .errors
        .iter()
        .map(|e| Diagnostic {
            severity: "error".into(),
            file: None,
            line: e.line.map(|l| l - 1), // line 1 is the replay head
            message: e.message.clone().unwrap_or_default(),
            context: e.context.clone(),
        })
        .collect();
    let timing = Timing {
        total_us: t0.elapsed().as_micros() as u64,
        tex_us: rt.t_tex.as_micros() as u64,
        traverse_us: rt.t_traverse.as_micros() as u64,
        pack_us: rt.t_pack.as_micros() as u64,
    };
    if cr.status == "error" || cr.dl.is_none() {
        s.events
            .send(Event::ParagraphUpdate {
                par_id: req.par_id,
                edit_id: req.edit_id,
                versions: req.versions,
                status: "error".into(),
                reasons: vec![],
                fragments: vec![],
                pagination_stale: false,
                context_stale: req.context_stale,
                dl: cr.dl.unwrap_or_default(),
                diagnostics,
                timing,
            })
            .ok();
        return;
    }
    let mut dl = cr.dl.unwrap();
    if !cr.images.is_empty() {
        dl.images = cr.images.clone();
    }
    let rows: Vec<(i64, i64)> = dl.lines.iter().map(|l| (l.x, l.y)).collect();
    let (fragments, mut stale) = {
        let layout = s.layout.lock();
        match layout.fragments(req.par_id, &rows) {
            Some(x) => x,
            None => {
                // a borrowed context: placed relative to the anchor's rows (a layout unit's
                // placements, or the live placement of a unit on a borrowed context itself)
                let derived = s.live.lock().get(req.par_id).and_then(|u| u.derived);
                let frags = derived
                    .and_then(|(parent, anchor, after)| {
                        let placed = s.live.lock().get(anchor).and_then(|u| u.place);
                        match placed.filter(|_| layout.unit(anchor).is_none()) {
                            Some((page, x, last)) => {
                                let bs = layout.unit(parent)?.baselineskip().max(1);
                                Some(LayoutStore::fragments_at(page, x, last + bs, &rows))
                            }
                            None => {
                                // a layout unit, or a chained unit not delivered (yet)
                                let a = if layout.unit(anchor).is_some() {
                                    anchor
                                } else {
                                    parent
                                };
                                let live = s.live.lock().get(a).and_then(|u| u.rows);
                                layout.fragments_relative(a, live, after, &rows)
                            }
                        }
                    })
                    .unwrap_or_default();
                (frags, true)
            }
        }
    };
    // a placement that lands off its page is a bad guess (a borrowed anchor on another page,
    // a unit grown past the page bottom, rows extrapolated above the top): drop it so the unit
    // waits for the pass instead of drawing its rows, pictures and all, at the page edge or
    // past it (two graphs, a plot pinned to the page top)
    if !fragments.is_empty() && !s.layout.lock().fragments_on_page(&fragments) {
        demote_span(s, &req, "the placement would fall off its page");
        return;
    }
    // a box the host cannot place (no placement for the unit, no neighbour to place it
    // against) is not a live result: the unit waits for the pass
    if fragments.is_empty() && !dl.lines.is_empty() {
        demote_span(s, &req, "no placement for the unit's rows");
        return;
    }
    if s.cfg.debug_dir.is_some() {
        let f = fragments.first();
        debug_request_line(
            s,
            &format!(
                "par {} placed: rows {} fragments {} page {:?} x {:?} y {:?}..{:?} approximate {:?}",
                req.par_id.0,
                dl.lines.len(),
                fragments.len(),
                f.map(|f| f.page),
                f.and_then(|f| f.xs.first().copied()),
                f.and_then(|f| f.baselines.first().copied()),
                fragments.last().and_then(|f| f.baselines.last().copied()),
                f.map(|f| f.approximate),
            ),
        );
    }
    {
        let mut live = s.live.lock();
        let u = live.unit(req.par_id);
        u.rows = Some(dl.lines.len() as i64);
        if let Some(f) = fragments.last() {
            if let (Some(x), Some(last)) = (f.xs.first(), f.baselines.last()) {
                u.place = Some((f.page, *x, *last));
            }
        }
    }
    if req.expected_rows != dl.lines.len() as i64 {
        stale = true;
    }
    let mut reasons: Vec<String> = dl.flags_map().keys().cloned().collect();
    if dl.inserts > 0 {
        // footnote/margin text is placed by the page builder: refreshed by the next layout
        reasons.push("inserts".into());
    }
    // counters: a unit that now advances other counters, or to other values, than the layout
    // saw (an added equation, item, footnote or \stepcounter) renumbers what follows: pass
    let counters_changed = {
        let expected = s
            .layout
            .lock()
            .unit(req.par_id)
            .map(|u| u.advanced())
            .unwrap_or_default();
        let prev = s
            .live
            .lock()
            .unit(req.par_id)
            .counters
            .replace(cr.counters.clone())
            .unwrap_or(expected);
        prev != cr.counters
    };
    if stale || counters_changed || !reasons.is_empty() {
        s.bg_signal.0.send(BgCmd::Pass).ok();
    }
    // the engine typeset the unit: its earlier crashes no longer count as crashes in a row
    s.live.lock().forget_crashes(req.par_id);
    let total_us = timing.total_us;
    s.events
        .send(Event::ParagraphUpdate {
            par_id: req.par_id,
            edit_id: req.edit_id,
            versions: req.versions,
            status: if reasons.is_empty() {
                "ok".into()
            } else {
                "ok_degraded".into()
            },
            reasons,
            fragments,
            pagination_stale: stale,
            context_stale: req.context_stale,
            dl,
            diagnostics,
            timing,
        })
        .ok();
    // fast budget: a unit whose compiles are too slow leaves the fast path until the next
    // layout. The first slow compiles of a unit are forgiven (font loading, cold caches); three
    // in a row mark the unit.
    const STRIKES: u32 = 3;
    if total_us > s.cfg.fast_budget.as_micros() as u64 {
        if s.bg_running.load(Ordering::SeqCst) {
            // a layout pass is using the CPU: a slow round trip now says nothing about the unit
            return;
        }
        // one lock scope: the strikes and the mark are the same record
        let marked = {
            let mut live = s.live.lock();
            let u = live.unit(req.par_id);
            u.slow_strikes += 1;
            if u.slow_strikes >= STRIKES {
                u.slow_strikes = 0;
                u.over_budget = Some((req.versions.layout_version, total_us / 1000));
                true
            } else {
                false
            }
        };
        if marked {
            s.events
                .send(Event::BackgroundScheduled {
                    par_id: Some(req.par_id),
                    reasons: vec![reason_str(&Reason::OverBudget(total_us / 1000))],
                    edit_id: req.edit_id,
                })
                .ok();
        }
    } else {
        s.live.lock().unit(req.par_id).slow_strikes = 0;
    }
}
