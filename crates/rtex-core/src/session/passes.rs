//! The background thread: debounced full passes with the capture and the picture cache, each
//! delivered as a layout (`pass_deliver`), from a standby engine when one is ready
//! (`pass_standby`); exports (`export`) run on the same thread.

use super::*;

pub(super) fn background_thread(s: Arc<Shared>) {
    prepare_standby(&s);
    while let Ok(cmd) = s.bg_signal.1.recv() {
        match cmd {
            BgCmd::Quit => break,
            BgCmd::Export(job, out) => run_export(&s, job, out),
            BgCmd::Pass => {
                if s.bg_paused.load(Ordering::SeqCst) {
                    s.bg_pending_while_paused.store(true, Ordering::SeqCst);
                    continue;
                }
                // the first pass runs at once; later ones are debounced (drain Pass commands until quiet)
                let debounce = if s.layout.lock().layout_version == 0 {
                    Duration::from_millis(1)
                } else {
                    s.cfg.debounce
                };
                loop {
                    match s.bg_signal.1.recv_timeout(debounce) {
                        Ok(BgCmd::Pass) => continue,
                        Ok(BgCmd::Quit) => return,
                        Ok(BgCmd::Export(job, out)) => {
                            run_export(&s, job, out);
                            continue;
                        }
                        Err(_) => break,
                    }
                }
                if s.shutdown.load(Ordering::SeqCst) {
                    break;
                }
                run_background_pass(&s);
                prepare_standby(&s);
            }
        }
    }
    if let Some(w) = s.standby.lock().take() {
        w.kill();
    }
}

pub(super) fn snapshot(s: &Shared) -> (BTreeMap<String, String>, Vec<SnapshotSpan>, Revision) {
    let files = s.files.lock();
    let rev = s.source_revision.load(Ordering::SeqCst);
    let policy = s.policy.lock();
    let inputted = s.inputted.lock();
    let mut texts = BTreeMap::new();
    let mut spans = Vec::new();
    for (name, fb) in files.iter() {
        texts.insert(name.clone(), fb.text.clone());
        spans.extend(crate::layout::snapshot_spans_of(
            fb,
            name,
            &policy,
            inputted.contains(name),
        ));
    }
    (texts, spans, rev)
}

pub(super) fn run_background_pass(s: &Shared) {
    s.bg_running.store(true, Ordering::SeqCst);
    run_background_pass_inner(s);
    s.bg_running.store(false, Ordering::SeqCst);
}

/// A pass of a multi-pass run that took long enough to be worth showing before the run is
/// stable (TikZ-heavy documents: 45 s per pass, three passes to converge).
pub(super) const PROVISIONAL_LAYOUT_AFTER: Duration = Duration::from_secs(2);

pub(super) fn run_background_pass_inner(s: &Shared) {
    let t0 = Instant::now();
    let (texts, spans, rev) = snapshot(s);
    let snap_dir = snapshot_dir(&s.cfg.build_dir);
    // a pass that cannot run is still a layout result: hosts see compile = Failed instead of a
    // pass that never ends
    let failed = |msg: String| layout_failed(s, t0, rev, msg);
    if let Err(e) = write_snapshot(&s.cfg.project_root, &texts, &s.cfg.main_file, &snap_dir) {
        failed(format!("snapshot: {e}"));
        return;
    }
    let out_dir = s.cfg.build_dir.join("bg");
    let unit_envs = s.policy.lock().unit_envs_env();
    // picture cache: pictures whose source and surroundings are unchanged come from an earlier
    // pass's PDF; the manifest is refreshed before every pass of the run (a pass's own
    // drawings serve the next one)
    let _ = std::fs::create_dir_all(&out_dir);
    let pics = cacheable_pictures(s, &texts);
    let pic_cache =
        std::sync::Mutex::new(crate::piccache::PicCache::open(&out_dir.join("pic-cache")));
    // written into the directory of the pass about to run (the capture reads it from there)
    let refresh_manifest = |dir: &Path| {
        let manifest = dir.join("pic-manifest.json");
        if pics.is_empty() {
            let _ = std::fs::remove_file(&manifest);
            return;
        }
        if let Err(e) = pic_cache.lock().unwrap().write_manifest(&pics, &manifest) {
            log::warn!("picture cache manifest: {e:#}");
        }
    };
    let absorb = |cap: &crate::capture::CaptureResult| {
        if pics.is_empty() {
            return;
        }
        if let Err(e) = pic_cache.lock().unwrap().absorb(
            &pics,
            &cap.json.recorded_pics(),
            &cap.json.pic_mismatch,
            &cap.pdf,
        ) {
            log::warn!("picture cache: {e:#}");
        }
    };
    // the aux family every pass of this run starts from
    let aux_dir = last_pass_dir(s).unwrap_or_else(|| pass_dir(s, 0));
    // before a pass runs in its directory: the previous pass's aux family and the manifest
    let prepare_dir = |dir: &Path| {
        if let Some(from) = last_pass_dir(s) {
            if let Err(e) = crate::background::copy_aux_family(&from, dir) {
                log::warn!("aux family: {e:#}");
            }
        }
        refresh_manifest(dir);
    };
    let finished = |cap: &crate::capture::CaptureResult| {
        *s.last_pass_dir.lock() = Some(cap.out_dir.clone());
    };
    let spans_for_provisional = spans.clone();
    let mut pass_started = Instant::now();
    let mut on_pass = |cap: &crate::capture::CaptureResult, pass: u32| {
        // the host sees the provisional layout before the cache absorbs the pass's pictures
        // (a PDF round trip)
        if pass_started.elapsed() >= PROVISIONAL_LAYOUT_AFTER {
            deliver_layout(
                s,
                t0,
                cap,
                pass,
                false,
                spans_for_provisional.clone(),
                rev,
                true,
            );
        }
        absorb(cap);
        pass_started = Instant::now();
    };
    let result = if s.cfg.warm_background {
        // Every pass of the loop runs in a standby engine: the one prepared while the user
        // typed, then the one started when the previous pass was released (its preamble loads
        // while the body is typeset). Two snapshot directories alternate so a loading standby
        // never rewrites the files a running one reads.
        let pre_hash = texts
            .get(&s.cfg.main_file)
            .and_then(|t| crate::split_preamble(t))
            .map(|(p, _)| crate::document::hash_str(p));
        let mut runner = |_pass: u32| -> Result<crate::capture::CaptureResult> {
            standby_pass(s, pre_hash, &texts, &unit_envs, &prepare_dir, &finished)
        };
        run_pass_with_runner(
            crate::background::PassPlan {
                tl: &s.tl,
                snapshot_dir: &standby_dir(s, 0),
                main: &s.cfg.main_file,
                aux_dir: &aux_dir,
                max_passes: s.cfg.max_passes,
                bib: s.cfg.bib_tool,
            },
            &mut runner,
            &mut on_pass,
        )
    } else {
        // plain passes: a fresh lualatex per pass in pass-0 (sequential, so one directory)
        let dir = pass_dir(s, 0);
        let mut runner = |_pass: u32| -> Result<crate::capture::CaptureResult> {
            plain_pass(s, &dir, &snap_dir, &unit_envs, &prepare_dir, &finished)
        };
        run_pass_with_runner(
            crate::background::PassPlan {
                tl: &s.tl,
                snapshot_dir: &snap_dir,
                main: &s.cfg.main_file,
                aux_dir: &aux_dir,
                max_passes: s.cfg.max_passes,
                bib: s.cfg.bib_tool,
            },
            &mut runner,
            &mut on_pass,
        )
    };
    let outcome = match result {
        Ok(o) => o,
        Err(e) => {
            failed(format!("background pass: {e:#}"));
            return;
        }
    };
    absorb(&outcome.capture);
    let outcome_cap = outcome.capture;
    deliver_layout(
        s,
        t0,
        &outcome_cap,
        outcome.passes,
        outcome.aux_stable,
        spans,
        rev,
        false,
    );
}

/// One pass in a standby engine: the one prepared while the user typed if its preamble is
/// current, else a new one. Starts the next standby in the other slot before running.
fn standby_pass(
    s: &Shared,
    pre_hash: Option<u64>,
    texts: &BTreeMap<String, String>,
    unit_envs: &str,
    prepare_dir: &dyn Fn(&Path),
    finished: &dyn Fn(&crate::capture::CaptureResult),
) -> Result<crate::capture::CaptureResult> {
    let ready = {
        let mut slot = s.standby.lock();
        match slot.take() {
            Some(mut w) => {
                if Some(w.preamble_hash) == pre_hash && w.is_alive() {
                    Some(w)
                } else {
                    w.kill();
                    None
                }
            }
            None => None,
        }
    };
    let w = match ready {
        Some(w) => w,
        None => {
            let slot = standby_slot(s);
            spawn_standby(s, texts, unit_envs, slot)?
        }
    };
    // this pass starts from the last one's aux family, then the next standby starts
    // in the other slot (whose previous results are consumed by now)
    prepare_dir(w.out_dir());
    let other = if w.src_dir.ends_with("src-body-0") {
        1
    } else {
        0
    };
    if let Ok(next) = spawn_standby(s, texts, unit_envs, other) {
        *s.standby.lock() = Some(next);
    }
    let cap = w.run(
        &s.cfg.project_root,
        texts,
        &s.cfg.main_file,
        Some(s.cfg.pass_timeout),
    )?;
    finished(&cap);
    Ok(cap)
}

/// One pass in a fresh LuaLaTeX, in `dir`.
fn plain_pass(
    s: &Shared,
    dir: &Path,
    snap_dir: &Path,
    unit_envs: &str,
    prepare_dir: &dyn Fn(&Path),
    finished: &dyn Fn(&crate::capture::CaptureResult),
) -> Result<crate::capture::CaptureResult> {
    prepare_dir(dir);
    let cap = crate::capture::run_capture_with(
        &s.tl,
        snap_dir,
        &s.cfg.main_file,
        dir,
        true,
        unit_envs,
        Some(s.cfg.pass_timeout),
    )?;
    finished(&cap);
    Ok(cap)
}

/// The document's picture environments, for the picture cache; none when it is off.
fn cacheable_pictures(
    s: &Shared,
    texts: &BTreeMap<String, String>,
) -> Vec<crate::piccache::PictureRef> {
    if s.cfg.picture_cache {
        let pre_hash = crate::piccache::preamble_hash(texts, &s.cfg.main_file);
        crate::piccache::scan_pictures(texts, &s.cfg.main_file, pre_hash)
    } else {
        Vec::new()
    }
}
