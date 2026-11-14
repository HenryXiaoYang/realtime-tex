//! Layer 1: every unit the live path may take, typeset by a live engine and compared with the rows the full run captured.

use super::*;

pub(super) fn row_text(l: &Line) -> String {
    let mut s = String::new();
    let mut last_x: Option<(i64, i64)> = None;
    for it in &l.items {
        if let Item::Glyph { char, x, width, .. } = it {
            if let Some((lx, lw)) = last_x {
                if *x - (lx + lw) > 60000 {
                    s.push(' ');
                }
            }
            s.push(char::from_u32(*char as u32).unwrap_or('?'));
            last_x = Some((*x, *width));
        }
    }
    s
}

/// Eligibility and layer 1, unit by unit; fills the report's `paragraphs` and prints the layer's
/// summary.
pub(super) fn check_units(
    tl: &TexLive,
    project: &Path,
    preamble: &str,
    policy: &Policy,
    cap: &rtex_core::capture::CaptureResult,
    opts: &VerifyOpts,
    report: &mut Report,
) -> Result<()> {
    let (store, fb) = LayoutStore::offline(cap, project, &opts.main, policy)?;
    let mut server: Option<FastServer> = None;
    let mut page_cache: std::collections::BTreeMap<i64, DisplayList> =
        std::collections::BTreeMap::new();
    let mut checked = 0usize;
    report.paragraphs_total = cap.json.units.len();
    for (eu, span_id) in store.mapped_units() {
        let c = &eu.captured;
        let src = fb.fast_source(span_id).unwrap_or_default();
        let (shape, mut reasons) = classify_source(&src, policy);
        let has_ctx = eu.has_context();
        reasons.extend(check_engine_unit(
            &c.kind,
            &c.everypar,
            has_ctx,
            eu.rows(),
            &eu.flags,
            c.name
                .as_deref()
                .filter(|_| c.runin && !rtex_core::eligibility::runs_into_title(&src)),
        ));
        let shape_ok = match (&shape, c.kind.as_str()) {
            (UnitShape::Par, "par") => true,
            (UnitShape::Env(n), "env") => Some(n.as_str()) == c.name.as_deref(),
            (UnitShape::Heading(n), "heading") => Some(n.as_str()) == c.name.as_deref(),
            _ => false,
        };
        if !shape_ok {
            reasons.push(rtex_core::eligibility::Reason::KindMismatch);
        }
        if c.placements.is_empty() {
            continue; // never shipped; nothing to compare
        }
        if !opts.units.is_empty() && !opts.units.contains(&eu.uid) {
            continue;
        }
        if !reasons.is_empty() {
            for r in reasons {
                let key = ineligible_key(r, c, &shape, &src);
                *report.ineligible_reasons.entry(key).or_default() += 1;
            }
            continue;
        }
        report.paragraphs_eligible += 1;
        if let Some(max) = opts.max_paragraphs {
            if checked >= max {
                continue;
            }
        }
        checked += 1;
        if server.is_none() {
            let s = start_server(tl, project, preamble, cap, opts)?;
            server = Some(s);
        }
        let srv = server.as_mut().unwrap();
        srv.set_context(eu.uid, &eu.context_json())?;
        let kind_name = match c.kind.as_str() {
            "par" => "par".to_string(),
            k => format!("{k}:{}", c.name.clone().unwrap_or_default()),
        };
        let (res, rt) = match srv.compile(eu.uid, &src) {
            Ok(x) => x,
            Err(e) => {
                report.paragraphs.push(ParagraphVerdict {
                    seq: eu.uid,
                    kind: kind_name,
                    status: format!("engine error: {e}"),
                    ..Default::default()
                });
                server = None;
                continue;
            }
        };
        let mut v = ParagraphVerdict {
            seq: eu.uid,
            kind: kind_name,
            status: res.status.clone(),
            t_total_ms: rt.total.as_secs_f64() * 1e3,
            ..Default::default()
        };
        if let Some(fast) = &res.dl {
            let mut pnos: Vec<i64> = c.placements.iter().map(|pl| pl.page).collect();
            pnos.sort();
            pnos.dedup();
            for pno in &pnos {
                if !page_cache.contains_key(pno) {
                    page_cache.insert(*pno, cap.page(*pno)?);
                }
            }
            let pages: Vec<(i64, &DisplayList)> =
                pnos.iter().map(|pno| (*pno, &page_cache[pno])).collect();
            let (same, total, notes) = compare_rows(fast, &pages, &eu.uids);
            if opts.dump_rows && (same != total || !notes.is_empty()) {
                dump_rows(fast, &pages, eu, c);
            }
            v.lines = fast.lines.len();
            v.glyphs_identical = same;
            v.glyphs_total = total;
            v.notes = notes;
            if res.status == "ok_degraded" {
                v.notes.push(format!(
                    "degraded: {:?}",
                    fast.flags_map().keys().collect::<Vec<_>>()
                ));
            }
            if res.status == "error" {
                v.notes.insert(
                    0,
                    format!(
                        "engine errors: {:?}",
                        res.errors
                            .iter()
                            .map(|e| format!(
                                "{} | {}",
                                e.message.clone().unwrap_or_default(),
                                e.context
                                    .clone()
                                    .unwrap_or_default()
                                    .chars()
                                    .take(160)
                                    .collect::<String>()
                            ))
                            .collect::<Vec<_>>()
                    ),
                );
            }
        } else {
            v.notes.push(format!("errors: {:?}", res.errors));
        }
        report.paragraphs.push(v);
    }
    if let Some(mut s) = server {
        s.shutdown()?;
    }
    summarize_units(opts, report)
}

/// How a reason a unit is not eligible is counted in the report: by macro or environment name,
/// with the unit's kind and text where that explains it.
fn ineligible_key(
    r: rtex_core::eligibility::Reason,
    c: &rtex_core::capture::CapturedUnit,
    shape: &UnitShape,
    src: &str,
) -> String {
    let key = match r {
        rtex_core::eligibility::Reason::DisallowedMacro(m) => format!("macro \\{m}"),
        rtex_core::eligibility::Reason::DisallowedMathMacro(m) => {
            format!("math macro \\{m}")
        }
        rtex_core::eligibility::Reason::NonDefaultEverypar => format!(
            "NonDefaultEverypar [{}]",
            c.everypar.chars().take(70).collect::<String>()
        ),
        rtex_core::eligibility::Reason::KindMismatch => format!(
            "KindMismatch (source {:?}, capture {}:{}, text {:?})",
            shape,
            c.kind,
            c.name.clone().unwrap_or_default(),
            src.chars().take(50).collect::<String>()
        ),
        rtex_core::eligibility::Reason::NoContext => format!(
            "NoContext ({}:{} at line {}, text {:?})",
            c.kind,
            c.name.clone().unwrap_or_default(),
            c.begin_line,
            src.chars().take(40).collect::<String>()
        ),
        other => format!("{other:?}"),
    };
    key
}

/// A live engine on the project's preamble, with the labels of the full run.
fn start_server(
    tl: &TexLive,
    project: &Path,
    preamble: &str,
    cap: &rtex_core::capture::CaptureResult,
    opts: &VerifyOpts,
) -> Result<FastServer> {
    let s = FastServer::spawn(
        tl,
        project,
        &opts.build.join("serve"),
        preamble,
        1,
        Some(&cap.out_dir.join(format!("{}.aux", cap.jobname))),
    )?;
    println!("server ready in {:.2}s", s.startup.as_secs_f64());
    let labels =
        rtex_core::background::read_aux_labels(&cap.out_dir.join(format!("{}.aux", cap.jobname)));
    let mut s = s;
    if !labels.is_empty() {
        s.set_labels(&labels)?;
    }
    Ok(s)
}

/// `--dump-rows`: the live rows of a unit and the rows the full run captured for it.
fn dump_rows(
    fast: &DisplayList,
    pages: &[(i64, &DisplayList)],
    eu: &rtex_core::layout::EngineUnit,
    c: &rtex_core::capture::CapturedUnit,
) {
    println!("--- unit {} ({}) fast rows:", eu.uid, c.kind);
    for (k, l) in fast.lines.iter().enumerate() {
        println!(
            "  f{:2} x={} y={} w={} h={} gs={:.4}: {}",
            k + 1,
            l.x,
            l.y,
            l.w,
            l.h,
            l.gs,
            row_text(l)
        );
    }
    println!("--- capture rows:");
    for uid in &eu.uids {
        for (_, pg) in pages {
            for l in pg.rows_of(*uid) {
                println!(
                    "  c{:2} x={} y={} w={} h={} gs={:.4}: {}",
                    l.row,
                    l.x,
                    l.y,
                    l.w,
                    l.h,
                    l.gs,
                    row_text(l)
                );
            }
        }
    }
}

/// Layer 1's verdict from the units checked, and its summary on stdout.
fn summarize_units(opts: &VerifyOpts, report: &mut Report) -> Result<()> {
    let l1_total: usize = report.paragraphs.iter().map(|p| p.glyphs_total).sum();
    let l1_same: usize = report.paragraphs.iter().map(|p| p.glyphs_identical).sum();
    let l1_bad: Vec<&ParagraphVerdict> = report
        .paragraphs
        .iter()
        .filter(|p| {
            p.glyphs_identical != p.glyphs_total
                || p.notes.iter().any(|n| !n.starts_with("degraded"))
                || !(p.status == "ok" || p.status == "ok_degraded")
        })
        .collect();
    if let Some(min) = opts.min_eligible {
        if report.paragraphs_eligible < min {
            anyhow::bail!(
                "eligibility gate failed: {} eligible of {} units, {min} required; reasons: {:?}",
                report.paragraphs_eligible,
                report.paragraphs_total,
                report.ineligible_reasons
            );
        }
    }
    report.layer1_pass = match (opts.expect_differing, opts.max_differing) {
        (Some(n), _) => l1_bad.len() == n && !report.paragraphs.is_empty(),
        (None, Some(max)) => l1_bad.len() <= max && !report.paragraphs.is_empty(),
        (None, None) => l1_bad.is_empty() && !report.paragraphs.is_empty(),
    };
    println!("layer 1 (fast vs extractor): {} eligible of {} units; {} compiled; {}/{} glyphs identical; {} units with differences",
        report.paragraphs_eligible, report.paragraphs_total, report.paragraphs.len(), l1_same, l1_total, l1_bad.len());
    {
        // per kind: count, identical, median round trip
        let mut by_kind: std::collections::BTreeMap<String, Vec<&ParagraphVerdict>> =
            std::collections::BTreeMap::new();
        for p in &report.paragraphs {
            by_kind.entry(p.kind.clone()).or_default().push(p);
        }
        for (k, v) in &by_kind {
            let mut t: Vec<f64> = v.iter().map(|p| p.t_total_ms).collect();
            t.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let bad = v
                .iter()
                .filter(|p| l1_bad.iter().any(|b| b.seq == p.seq))
                .count();
            println!("  {k:16} {:4} units, {:3} with differences, round trip median {:.3} ms, max {:.3} ms", v.len(), bad, t[t.len() / 2], t[t.len() - 1]);
        }
    }
    for p in l1_bad.iter().take(8) {
        println!(
            "  unit {} ({}): status {} {:?}",
            p.seq,
            p.kind,
            p.status,
            p.notes.iter().take(3).collect::<Vec<_>>()
        );
    }
    if !report.ineligible_reasons.is_empty() {
        println!("  ineligible reasons: {:?}", report.ineligible_reasons);
    }
    Ok(())
}
