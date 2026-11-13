//! Delivering a finished pass (`passes`) as a layout: installing it, matching units to spans, warming the live engine, and the `LayoutUpdate` event.

use super::*;

pub(super) fn layout_failed(s: &Shared, t0: Instant, rev: Revision, msg: String) {
    s.events
        .send(Event::Diagnostics {
            source: "background".into(),
            items: vec![Diagnostic {
                severity: "error".into(),
                file: None,
                line: None,
                message: msg.clone(),
                context: None,
            }],
        })
        .ok();
    *s.convergence.lock() = Some(Convergence::PassLimitReached {
        passes: 0,
        reasons: vec![msg.clone()],
    });
    s.events
        .send(Event::LayoutUpdate {
            versions: Versions {
                source_revision: rev,
                ..Default::default()
            },
            compile: CompileStatus::Failed,
            convergence: Convergence::PassLimitReached {
                passes: 0,
                reasons: vec![msg],
            },
            passes: 0,
            pages_changed: vec![],
            pages_total: 0,
            placements: vec![],
            eligible_paragraphs: vec![],
            pdf_fallback: None,
            wall_ms: t0.elapsed().as_millis() as u64,
        })
        .ok();
}

/// Withdraw the `fast:<unit>` diagnostics of live-engine failures that a layout of revision
/// `rev` supersedes (it typeset the failing source or a later one): an empty set per source.
pub(super) fn withdraw_fast_failures(s: &Shared, rev: Revision) {
    let mut withdrawn = Vec::new();
    s.fast_failures.lock().retain(|par_id, at| {
        let superseded = *at <= rev;
        if superseded {
            withdrawn.push(*par_id);
        }
        !superseded
    });
    for par_id in withdrawn {
        s.events
            .send(Event::Diagnostics {
                source: format!("fast:{par_id:?}"),
                items: vec![],
            })
            .ok();
    }
}

/// Install a pass's capture as the current layout and tell the host. `provisional`: a pass of a
/// run that is not stable yet (another pass follows); the layout is usable, its convergence is
/// `Converging`.
#[allow(clippy::too_many_arguments)]
pub(super) fn deliver_layout(
    s: &Shared,
    t0: Instant,
    cap: &crate::capture::CaptureResult,
    passes: u32,
    aux_stable: bool,
    spans: Vec<SnapshotSpan>,
    rev: Revision,
    provisional: bool,
) {
    withdraw_fast_failures(s, rev);
    let (diagnostics, errors, compile) = pass_diagnostics(s, cap);
    if compile == CompileStatus::Ok {
        // the state the live engine starts units from no longer comes from text that crashed it
        s.live.lock().on_clean_pass(rev);
    }
    if compile == CompileStatus::Failed {
        send_no_pages(s, t0, rev, passes, diagnostics, compile);
        return;
    }
    let (changed, versions, placements, eligible, eligible_strict, pdf) = {
        // lock order everywhere: files, then layout, then policy (apply_edit holds the first two
        // together)
        let files = s.files.lock();
        let mut layout = s.layout.lock();
        let changed = match layout.install(cap, spans, rev) {
            Ok(c) => {
                publish_layout_pdf(s, cap, &mut layout);
                c
            }
            Err(e) => {
                drop(layout);
                drop(files);
                layout_failed(s, t0, rev, format!("install layout: {e}"));
                return;
            }
        };
        let versions = Versions {
            source_revision: s.source_revision.load(Ordering::SeqCst),
            context_revision: layout.context_revision,
            engine_generation: s.engine_generation.load(Ordering::SeqCst),
            layout_version: layout.layout_version,
        };
        let (mut placements, mut eligible, mut eligible_strict) =
            placements_and_eligibility(s, &layout, &files);
        drop(files);
        placements.sort_by_key(|p| p.par_id);
        eligible.sort();
        eligible_strict.sort();
        (
            changed,
            versions,
            placements,
            eligible,
            eligible_strict,
            layout.pdf.clone(),
        )
    };
    warm_engine(s, rev, &eligible_strict);
    // borrowed contexts, live rows and placements are superseded by the new placements, and
    // facts tagged with an older layout no longer apply
    s.live.lock().on_new_layout(versions.layout_version);
    let convergence = convergence_of(s, rev, passes, aux_stable, provisional, errors);
    *s.convergence.lock() = Some(convergence.clone());
    let pages_changed: Vec<PageUpdate> = {
        let layout = s.layout.lock();
        changed
            .iter()
            .filter_map(|n| {
                layout.pages.get(n).map(|dl| PageUpdate {
                    page: *n,
                    exact: dl.is_exact(),
                    hash: layout.page_hashes[n],
                    dl: dl.clone(),
                })
            })
            .collect()
    };
    let pages_total = cap.json.pages;
    // also when empty: the set replaces the previous pass's, so fixed errors go away
    s.events
        .send(Event::Diagnostics {
            source: "background".into(),
            items: diagnostics,
        })
        .ok();
    // the fallback is named whenever the layout has a degraded page, changed in this
    // layout or not: a host that keeps one PDF per layout needs the current file
    let any_degraded = {
        let layout = s.layout.lock();
        layout.pages.values().any(|dl| !dl.is_exact())
    };
    s.events
        .send(Event::LayoutUpdate {
            versions,
            compile,
            convergence: convergence.clone(),
            passes,
            pages_changed,
            pages_total,
            placements,
            eligible_paragraphs: eligible,
            pdf_fallback: if any_degraded { pdf } else { None },
            wall_ms: t0.elapsed().as_millis() as u64,
        })
        .ok();
    if matches!(convergence, Convergence::Stale { .. }) {
        s.bg_signal.0.send(BgCmd::Pass).ok();
    }
}

/// The pass's diagnostics, how many are errors, and what that makes of the compile.
fn pass_diagnostics(
    s: &Shared,
    cap: &crate::capture::CaptureResult,
) -> (Vec<Diagnostic>, usize, CompileStatus) {
    let mut diagnostics = parse_log(&cap.log);
    for d in &mut diagnostics {
        // the standby reads the preamble from rtex-preamble.tex (same line numbers)
        if d.file
            .as_deref()
            .map(|f| f.ends_with("rtex-preamble.tex"))
            .unwrap_or(false)
        {
            d.file = Some(s.cfg.main_file.clone());
        }
    }
    let errors = diagnostics.iter().filter(|d| d.severity == "error").count();
    let compile = if cap.json.pages == 0 {
        CompileStatus::Failed
    } else if errors > 0 {
        CompileStatus::CompiledWithErrors { count: errors }
    } else {
        CompileStatus::Ok
    };
    (diagnostics, errors, compile)
}

/// A pass that produced no pages: its diagnostics and a layout that says so.
fn send_no_pages(
    s: &Shared,
    t0: Instant,
    rev: Revision,
    passes: u32,
    diagnostics: Vec<Diagnostic>,
    compile: CompileStatus,
) {
    s.events
        .send(Event::Diagnostics {
            source: "background".into(),
            items: diagnostics,
        })
        .ok();
    s.events
        .send(Event::LayoutUpdate {
            versions: Versions {
                source_revision: rev,
                ..Default::default()
            },
            compile,
            convergence: Convergence::PassLimitReached {
                passes,
                reasons: vec!["no pages".into()],
            },
            passes,
            pages_changed: vec![],
            pages_total: 0,
            placements: vec![],
            eligible_paragraphs: vec![],
            pdf_fallback: None,
            wall_ms: t0.elapsed().as_millis() as u64,
        })
        .ok();
}

/// Copies the pass's PDF to `layout-<version>.pdf`, the file hosts draw degraded pages from.
fn publish_layout_pdf(s: &Shared, cap: &crate::capture::CaptureResult, layout: &mut LayoutStore) {
    // the PDF hosts render degraded pages from: a copy per layout, since the next
    // pass (started right after this one, a provisional layout's in particular)
    // rewrites the pass PDF while the host reads it
    let v = layout.layout_version;
    let bg = s.cfg.build_dir.join("bg");
    let stable = bg.join(format!("layout-{v}.pdf"));
    match std::fs::copy(&cap.pdf, &stable) {
        Ok(_) => {
            layout.pdf = Some(stable.clone());
            // the two previous copies stay: a host may still be reading the layout
            // before the one it is replacing
            if v >= 3 {
                let _ = std::fs::remove_file(bg.join(format!("layout-{}.pdf", v - 3)));
            }
            // build/bg/<jobname>.pdf and .log: the latest layout, for hosts that
            // name these files themselves (a link to the copy; nothing writes
            // them in place)
            let main_pdf = bg.join(format!("{}.pdf", cap.jobname));
            let _ = std::fs::remove_file(&main_pdf);
            if std::fs::hard_link(&stable, &main_pdf).is_err() {
                let _ = std::fs::copy(&stable, &main_pdf);
            }
            let _ = std::fs::copy(&cap.log, bg.join(format!("{}.log", cap.jobname)));
        }
        Err(e) => log::warn!("layout PDF copy: {e}"),
    }
}

/// Where every mapped unit sits, which units may take the live path, and which of those the
/// allow-list knows in full (safe to compile unprobed).
fn placements_and_eligibility(
    s: &Shared,
    layout: &LayoutStore,
    files: &BTreeMap<String, FileBuf>,
) -> (Vec<ParagraphPlacement>, Vec<ParaId>, Vec<ParaId>) {
    let mut placements = Vec::new();
    let mut eligible = Vec::new();
    // units whose vocabulary the allow-list fully knows (safe to compile unprobed)
    let mut eligible_strict: Vec<ParaId> = Vec::new();
    let policy = s.policy.lock().clone();
    for (id, idx) in &layout.by_span {
        let eu = &layout.units[*idx];
        let c = &eu.captured;
        let rows: Vec<(i64, i64)> = c.placements.iter().map(|p| (p.x, p.y)).collect();
        let kind = match c.kind.as_str() {
            "par" => "par".to_string(),
            k => format!("{k}:{}", c.name.clone().unwrap_or_default()),
        };
        if let Some((frags, _)) = layout.fragments(*id, &rows) {
            placements.push(ParagraphPlacement {
                par_id: *id,
                fragments: frags,
                lines: eu.rows(),
                kind,
            });
        }
        let has_ctx = eu.has_context();
        let span_text = files.values().find_map(|fb| fb.span_text(*id));
        // eligible = capture facts clean AND the span's source passes the allow-list with the
        // shape the capture saw (what apply_edit will decide for a one-character edit)
        let source_ok = layout
            .snapshot_spans
            .iter()
            .find(|sp| sp.id == *id)
            .map(|sp| !sp.background_only)
            .unwrap_or(false)
            && span_text
                .map(|text| {
                    let (shape, _) = classify_source(text, &policy);
                    match (&shape, c.kind.as_str()) {
                        (UnitShape::Par, "par") => true,
                        (UnitShape::Env(n), "env") => Some(n.as_str()) == c.name.as_deref(),
                        (UnitShape::Heading(n), "heading") => Some(n.as_str()) == c.name.as_deref(),
                        _ => false,
                    }
                })
                .unwrap_or(false);
        if source_ok
            && check_engine_unit(
                &c.kind,
                &c.everypar,
                has_ctx,
                eu.rows(),
                &eu.flags,
                c.name
                    .as_deref()
                    .filter(|_| c.runin && !span_text.is_some_and(runs_into_title)),
            )
            .is_empty()
        {
            eligible.push(*id);
            let strict_ok = span_text
                .map(|text| classify_source_with(text, &policy, false).1.is_empty())
                .unwrap_or(false);
            if strict_ok {
                eligible_strict.push(*id);
            }
        }
    }
    (placements, eligible, eligible_strict)
}

/// Queues one warm-up compile for the live engine after a layout.
fn warm_engine(s: &Shared, rev: Revision, eligible_strict: &[ParaId]) {
    // warm the engine: compile the first allow-listed paragraph once so fonts are loaded before
    // the first real keystroke (never an unprobed unit: it could leak state). Skip a unit whose
    // compile already leaked (\maketitle in title-page mode redefines \maketitle): warming it
    // again restarts the engine and schedules a pass, whose layout would warm it again, forever.
    let warm_id = {
        let live = s.live.lock();
        eligible_strict
            .iter()
            .copied()
            .find(|id| live.get(*id).is_none_or(|u| u.leak.is_none()))
    };
    if let Some(id) = warm_id {
        let files = s.files.lock();
        let layout = s.layout.lock();
        if let (Some(eu), Some(text)) = (
            layout.unit(id),
            files.values().find_map(|fb| fb.span_text(id)),
        ) {
            let mut p = s.pending.lock();
            if let std::collections::btree_map::Entry::Vacant(e) = p.entry(id) {
                // exercise the font variants and math a body paragraph commonly needs so their
                // font instances are loaded before the first real keystroke
                let warm = format!("{} \\emph{{warm}} \\textbf{{warm}} \\textit{{warm}} \\textsc{{warm}} {{\\small warm}} $x^2_i + \\alpha \\sum \\frac{{1}}{{2}} \\mathbf{{v}}$", text.trim_end_matches('\n'));
                e.insert(FastRequest {
                    warmup: true,
                    par_id: id,
                    edit_id: 0,
                    span_hash: 0,
                    source: warm,
                    pics: String::new(),
                    pics_n: 0,
                    seq: eu.uid,
                    ctx: None,
                    versions: Versions {
                        source_revision: rev,
                        context_revision: layout.context_revision,
                        engine_generation: s.engine_generation.load(Ordering::SeqCst),
                        layout_version: layout.layout_version,
                    },
                    context_stale: false,
                    expected_rows: 0,
                    probe: false,
                    pics_required: false,
                });
                s.pending_signal.0.send(()).ok();
            }
        }
    }
}

/// Whether the document has settled with this pass, and if not, why.
fn convergence_of(
    s: &Shared,
    rev: Revision,
    passes: u32,
    aux_stable: bool,
    provisional: bool,
    errors: usize,
) -> Convergence {
    let current = s.source_revision.load(Ordering::SeqCst);
    let mut reasons = Vec::new();
    if !aux_stable {
        reasons.push("aux family still changing".into());
    }
    if errors > 0 {
        reasons.push(format!("{errors} compile errors"));
    }

    if current > rev {
        Convergence::Stale {
            pending_since: rev + 1,
        }
    } else if provisional {
        Convergence::Converging {
            pass: passes,
            reasons: vec!["another pass is running".into()],
        }
    } else if aux_stable && errors == 0 {
        Convergence::Converged
    } else if !aux_stable && passes >= s.cfg.max_passes {
        Convergence::PassLimitReached {
            passes,
            reasons: reasons.clone(),
        }
    } else {
        Convergence::Converging {
            pass: passes,
            reasons: reasons.clone(),
        }
    }
}
