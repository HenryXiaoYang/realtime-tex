//! Warm round trips through a session: one edit at a time per category of unit, timed from `apply_edit` to the decoded `ParagraphUpdate`.

use super::stats::*;
use anyhow::{bail, Result};
use rtex_core::{Edit, Event, Session, SessionConfig};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct CategoryResult {
    pub project: String,
    pub category: String,
    pub par_id: u64,
    pub lines: usize,
    pub glyphs: usize,
    pub individual_total: Stats,
    pub individual_tex: Stats,
    pub individual_traverse: Stats,
    pub individual_pack: Stats,
    pub amortized_total: Stats,
    pub overhead_share_median: f64,
}

pub(super) fn classify(kind: &str, lines: usize, text: &str) -> Option<&'static str> {
    let has_inline_math = text.contains('$')
        && !text.contains("\\[")
        && !text.contains("\\begin{equation")
        && !text.contains("\\begin{align");
    let has_display =
        text.contains("\\[") || text.contains("\\begin{equation") || text.contains("\\begin{align");
    let has_footnote = text.contains("\\footnote");
    // display-math and footnote paragraphs are compared across document sizes, so both
    // fixtures must pick comparably sized ones (3-8 rows)
    match kind {
        "par" if has_display => {
            if (3..=8).contains(&lines) {
                Some("display-math")
            } else {
                None
            }
        }
        "par" if has_footnote => {
            if (3..=8).contains(&lines) {
                Some("footnote")
            } else {
                None
            }
        }
        "par" => match (lines, has_inline_math) {
            (1, false) => Some("short"),
            (4..=5, false) => Some("medium"),
            (l, false) if l >= 10 => Some("long"),
            (1..=3, true) => Some("inline-math"),
            _ => None,
        },
        k if k.starts_with("env:itemize") || k.starts_with("env:enumerate") => Some("list"),
        k if k.starts_with("env:figure") => Some("figure"),
        k if k.starts_with("env:table") => Some("table"),
        k if k.starts_with("heading:") => Some("heading"),
        _ => None,
    }
}

pub fn roundtrip(
    project: &Path,
    categories: &[String],
    samples: usize,
    inner: usize,
    build: &Path,
) -> Result<Vec<CategoryResult>> {
    let name = project
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let mut cfg = SessionConfig::new(project, "main.tex");
    cfg.build_dir = build.join(&name);
    // the benchmark measures; the gates judge the 5 ms budget, so the session must not re-route
    cfg.fast_budget = Duration::from_millis(500);
    let session = Session::open(cfg)?;
    let (first, _) = session.wait_for(Duration::from_secs(600), |e| {
        matches!(e, Event::LayoutUpdate { .. })
    });
    let Some(Event::LayoutUpdate {
        eligible_paragraphs,
        placements,
        ..
    }) = first
    else {
        bail!("no layout")
    };
    session.pause_background(true); // no background passes competing for the CPU during timing
                                    // wait for the warm-up compile to finish so the engine is hot
    std::thread::sleep(Duration::from_millis(1500));
    let doc = session.document_text("main.tex").unwrap();
    let spans = session.spans("main.tex");
    let lines_of: BTreeMap<u64, (usize, String)> = placements
        .iter()
        .map(|p| (p.par_id.0, (p.lines as usize, p.kind.clone())))
        .collect();
    let mut picked: BTreeMap<&str, (u64, usize, String)> = BTreeMap::new();
    // prefer paragraphs in the middle of the document
    let mut elig: Vec<_> = eligible_paragraphs
        .iter()
        .filter_map(|id| spans.iter().find(|s| s.id == *id))
        .collect();
    let mid = elig.len() / 2;
    elig.rotate_left(mid);
    for sp in elig {
        let text = &doc[sp.range.clone()];
        let (lines, kind) = lines_of.get(&sp.id.0).cloned().unwrap_or((0, "par".into()));
        if let Some(cat) = classify(&kind, lines, text) {
            if categories.iter().any(|c| c == cat) && !picked.contains_key(cat) {
                picked.insert(cat, (sp.id.0, lines, text.to_string()));
            }
        }
    }
    let mut results = Vec::new();
    for cat in categories {
        let Some((pid, lines, text)) = picked.get(cat.as_str()).cloned() else {
            eprintln!("  {name}: no eligible paragraph for category {cat}");
            continue;
        };
        // positions are read fresh: earlier categories may have shifted later spans
        let spans_now = session.spans("main.tex");
        let sp = spans_now.iter().find(|s| s.id.0 == pid).unwrap().clone();
        let pos = sp.range.start + text.find(' ').unwrap_or(0);
        let mut total = Vec::new();
        let mut tex = Vec::new();
        let mut trav = Vec::new();
        let mut pack = Vec::new();
        let mut glyphs = 0;
        let mut toggled = false;
        let do_edit =
            |session: &Session, toggled: &mut bool| toggle_edit(session, toggled, pos, pid, cat);
        // warm this paragraph's fonts
        for _ in 0..3 {
            let _ = do_edit(&session, &mut toggled)?;
        }
        for _ in 0..samples {
            let (t, a, b, c, g) = do_edit(&session, &mut toggled)?;
            total.push(t);
            tex.push(a);
            trav.push(b);
            pack.push(c);
            glyphs = g;
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut amort = Vec::new();
        for _ in 0..(samples / 10).max(5) {
            let t0 = Instant::now();
            for _ in 0..inner {
                do_edit(&session, &mut toggled)?;
            }
            amort.push(t0.elapsed().as_secs_f64() * 1e3 / inner as f64);
        }
        // leave the paragraph as it was (the toggle may have ended in the inserted state)
        if toggled {
            do_edit(&session, &mut toggled)?;
        }
        let st = stats(&total);
        let stx = stats(&tex);
        let overhead = if st.median > 0.0 {
            (st.median - stx.median).max(0.0) / st.median
        } else {
            0.0
        };
        println!("  {name} {cat:12} par {pid} lines {lines} glyphs {glyphs}: individual median {:.3} ms (P95 {:.3}; tex {:.3} traverse {:.3} pack {:.3}); amortized median {:.3} ms",
            st.median, st.p95, stx.median, stats(&trav).median, stats(&pack).median, stats(&amort).median);
        results.push(CategoryResult {
            project: name.clone(),
            category: cat.clone(),
            par_id: pid,
            lines,
            glyphs,
            individual_total: st,
            individual_tex: stx,
            individual_traverse: stats(&trav),
            individual_pack: stats(&pack),
            amortized_total: stats(&amort),
            overhead_share_median: overhead,
        });
    }
    session.close();
    Ok(results)
}

/// One timed edit: inserts two characters at `pos`, or removes them again, and waits for the
/// unit's update. Returns the round trip and the engine's stage times in ms, and the glyph count.
fn toggle_edit(
    session: &Session,
    toggled: &mut bool,
    pos: usize,
    pid: u64,
    cat: &str,
) -> Result<(f64, f64, f64, f64, usize)> {
    let edit = if *toggled {
        Edit {
            start_byte: pos,
            end_byte: pos + 2,
            text: String::new(),
        }
    } else {
        Edit {
            start_byte: pos,
            end_byte: pos,
            text: " x".into(),
        }
    };
    *toggled = !*toggled;
    let t0 = Instant::now();
    let r = session.apply_edit("main.tex", edit)?;
    if r.routed != "fast" {
        bail!("edit not routed fast: {:?}", r.reasons);
    }
    let (ev, others) = session.wait_for(Duration::from_secs(30), |e| {
        matches!(e, Event::ParagraphUpdate { .. })
    });
    let Some(Event::ParagraphUpdate {
        timing,
        dl,
        status,
        diagnostics,
        ..
    }) = ev
    else {
        let seen: Vec<String> = others
            .iter()
            .map(|e| match e {
                Event::EngineState { state, reason, .. } => {
                    format!("EngineState {state} {reason:?}")
                }
                Event::BackgroundScheduled { reasons, .. } => {
                    format!("BackgroundScheduled {reasons:?}")
                }
                Event::Diagnostics { items, .. } => format!(
                    "Diagnostics {:?}",
                    items.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
                ),
                other => serde_json::to_value(other)
                    .map(|v| v["event"].to_string())
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect();
        bail!(
            "no paragraph update for par {pid} ({cat}); events seen: {seen:?}; edit result: {:?}",
            r.reasons
        )
    };
    if status != "ok" && status != "ok_degraded" {
        bail!(
            "status {status} for {cat} par {pid} (edit {:?} at {pos}): {:?}",
            if *toggled { "insert" } else { "delete" },
            diagnostics
                .iter()
                .map(|d| format!(
                    "{} | {}",
                    d.message,
                    d.context
                        .clone()
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect::<String>()
                ))
                .collect::<Vec<_>>()
        );
    }
    Ok((
        t0.elapsed().as_secs_f64() * 1e3,
        timing.tex_us as f64 / 1e3,
        timing.traverse_us as f64 / 1e3,
        timing.pack_us as f64 / 1e3,
        dl.glyph_count(),
    ))
}
