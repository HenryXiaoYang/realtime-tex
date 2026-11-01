//! Optional host ownership must preserve TeX output, in capture and live compiles.
use rtex_core::background::{run_pass, BibTool};
use rtex_core::capture::run_capture;
use rtex_core::engine::FastServer;
use rtex_core::session::CompileStatus;
use rtex_core::texlive::TexLive;
use rtex_core::{Convergence, Edit, Event, Session, SessionConfig};
use rtex_dl::{DisplayList, Item, MathField, MathRole};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SOURCE: &str = include_str!("../../../fixtures/research/math-owners/main.tex");

fn setup(name: &str, source: &str) -> Option<(TexLive, PathBuf, PathBuf)> {
    let tl = match TexLive::discover() {
        Ok(tl) => tl,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let root = std::env::temp_dir().join(format!("rtex-math-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("main.tex"), source).unwrap();
    Some((tl, root, project))
}

fn package(tl: &TexLive, name: &str) -> bool {
    let mut cmd = std::process::Command::new(
        tl.bin_dir
            .as_ref()
            .map(|p| p.join(rtex_core::paths::exe("kpsewhich")))
            .unwrap_or_else(|| "kpsewhich".into()),
    );
    let found = cmd.arg(name).output().is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("SKIP: {name} is not installed");
    }
    found
}

fn empty<'a>(dl: &'a DisplayList, owner: i64, role: &str) -> &'a MathField {
    let field = dl
        .math_owners
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .find(|f| f.owner == owner && f.role.as_str() == role && f.empty)
        .unwrap_or_else(|| panic!("missing empty {owner}/{role}"));
    assert_eq!(field.bbox[0], field.bbox[2]);
    assert_eq!(field.bbox[1], field.bbox[3]);
    field
}

fn assert_owners(dl: &DisplayList) {
    let m = dl.math_owners.as_ref().expect("owners");
    // Every math glyph (including delimiters and the radical sign) is observed once;
    // ordinary prose uses a different font and is not tagged.
    let math_glyphs: Vec<_> = dl
        .lines
        .iter()
        .flat_map(|l| &l.items)
        .chain(&dl.other)
        .filter_map(|it| match it {
            Item::Glyph {
                font,
                char,
                x,
                y,
                width,
                ..
            } if {
                let f = dl.font(*font).unwrap();
                let file = f.filename.as_deref().unwrap_or("");
                file.contains("/cm")
                    || file.contains("latinmodern-math")
                    || f.name.as_deref().is_some_and(|n| n.starts_with("cm"))
            } =>
            {
                Some((*font, *char, *x, *y, *width))
            }
            _ => None,
        })
        .collect();
    assert_eq!(m.glyphs.len(), math_glyphs.len());
    for (g, (font, ch, x, y, w)) in m.glyphs.iter().zip(math_glyphs) {
        assert_eq!((g.font, g.char, g.bbox[0], g.bbox[2]), (font, ch, x, x + w));
        assert!(g.bbox[1] <= y && y <= g.bbox[3]);
        let expected: &[i64] = match g.owner {
            11 | 51 => &[97, 119886],
            12 | 52 => &[98, 119887],
            20 | 70 => &[120, 119909],
            21 => &[105, 119894],
            22 => &[106, 119895],
            23 => &[107, 119896],
            31 => &[51],
            32 => &[121, 119910],
            41 => &[122, 119911],
            53 => &[99, 119888],
            54 => &[100, 119889],
            100 => &[120, 121, 102, 119909, 119910, 119891],
            30 | 90 => &[112, 8730],
            40 => &[40, 41],
            50 => &[18, 19, 40, 41],
            80 => &[18, 19, 40, 41, 97, 98, 119886, 119887],
            other => panic!("unexpected owner {other}: {g:?}"),
        };
        assert!(
            expected.contains(&g.char)
                || (matches!(g.owner, 30 | 40 | 50 | 80 | 90)
                    && (0xF0000..=0xFFFFD).contains(&g.char)),
            "wrong owner: {g:?}"
        );
    }
    for (owner, count) in [
        (11, 1),
        (12, 1),
        (20, 1),
        (21, 1),
        (22, 1),
        (23, 1),
        (31, 1),
        (32, 1),
        (41, 1),
        (51, 1),
        (52, 1),
        (53, 1),
        (54, 1),
        (70, 1),
        (100, 6),
    ] {
        assert_eq!(
            m.glyphs.iter().filter(|g| g.owner == owner).count(),
            count,
            "owner {owner}"
        );
    }
    for (owner, role) in [
        (11, "num"),
        (12, "den"),
        (21, "sub"),
        (22, "sub"),
        (23, "sup"),
        (32, "radicand"),
        (51, "cell"),
        (52, "cell"),
        (53, "cell"),
        (54, "cell"),
    ] {
        assert!(
            m.fields
                .iter()
                .any(|f| f.owner == owner && f.role.as_str() == role && !f.empty),
            "missing {owner}/{role}: {:?}",
            m.fields
        );
    }
    assert!(
        m.fields.iter().any(|f| f.owner == 31
            && matches!(f.role, MathRole::Math | MathRole::Degree)
            && !f.empty),
        "missing radical degree"
    );
    let num = empty(dl, 60, "num");
    let den = empty(dl, 60, "den");
    assert_eq!(num.bbox[0], den.bbox[0]);
    assert!(num.bbox[1] < den.bbox[1]);
    let sup = empty(dl, 70, "sup");
    let sub = empty(dl, 70, "sub");
    let base = m.glyphs.iter().find(|g| g.owner == 70).unwrap();
    assert!(sup.bbox[1] < sub.bbox[1]);
    assert!(sup.bbox[0] >= base.bbox[2]);
    assert!(sub.bbox[0] >= base.bbox[2]);
    let cells: Vec<_> = m
        .fields
        .iter()
        .filter(|f| f.owner == 80 && f.role == MathRole::Cell && f.empty)
        .collect();
    assert!(cells.len() >= 2, "empty matrix cells");
    assert!(cells.first().unwrap().bbox[0] < cells.last().unwrap().bbox[0]);
    assert!(cells.first().unwrap().bbox[1] < cells.last().unwrap().bbox[1]);
    empty(dl, 90, "radicand");
    let back = DisplayList::from_binary(&dl.to_binary()).unwrap();
    assert_eq!(back.math_owners, dl.math_owners);
    let json = serde_json::to_string(&back).unwrap();
    assert_eq!(
        DisplayList::from_json(&json).unwrap().math_owners,
        dl.math_owners
    );
}

fn capture_identity(name: &str, source: &str) {
    let Some((mut tl, root, project)) = setup(name, source) else {
        return;
    };
    let off = run_capture(&tl, &project, "main.tex", &root.join("off"), true).unwrap();
    assert!(off.exit_ok);
    assert!(off.page(1).unwrap().math_owners.is_none());
    tl.math_owner_attribute = Some(65000);
    let on = run_capture(&tl, &project, "main.tex", &root.join("on"), true).unwrap();
    assert!(on.exit_ok);
    let mut dl = on.page(1).unwrap();
    assert_owners(&dl);
    dl.math_owners = None;
    assert_eq!(dl, off.page(1).unwrap(), "ownership changed drawing");
    let clean = run_pass(
        &tl,
        &project,
        "main.tex",
        &root.join("clean"),
        5,
        BibTool::Auto,
        false,
    )
    .unwrap();
    assert!(clean.aux_stable && clean.capture.exit_ok);
    assert_eq!(
        std::fs::read(&on.pdf).unwrap(),
        std::fs::read(&clean.capture.pdf).unwrap(),
        "capture must not change the tagged source's PDF"
    );
    // Also compare with observation disabled, so a marker-induced spacing change cannot hide
    // behind an observer enabled in both the captured and the clean run.
    assert_eq!(
        std::fs::read(&on.pdf).unwrap(),
        std::fs::read(&off.pdf).unwrap()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn owners_and_empty_fields_preserve_output() {
    capture_identity("cm", SOURCE);
}

#[test]
fn unicode_math_owners_and_empty_fields_preserve_output() {
    let Ok(tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return;
    };
    if !package(&tl, "unicode-math.sty") {
        return;
    }
    capture_identity(
        "unicode",
        &SOURCE.replace(
            "\\usepackage{amsmath}",
            "\\usepackage{amsmath}\n\\usepackage{unicode-math}",
        ),
    );
    // unicode-math's empty radical degree has no surviving output anchor. It is deliberately
    // not fabricated from a nearby box; see docs/embedding.md.
}

#[test]
fn cjk_text_owners() {
    let Ok(mut tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return;
    };
    if !package(&tl, "ctex.sty") || !package(&tl, "FandolSong-Regular.otf") {
        return;
    }
    let source = "\\documentclass{article}\n\\usepackage[fontset=fandol]{ctex}\n\\usepackage{amsmath}\n\\begin{document}\n$\\begingroup\\attribute65000=110 \\text{中文}\\endgroup$\n\\end{document}";
    let Some((_, root, project)) = setup("cjk", source) else {
        return;
    };
    tl.math_owner_attribute = Some(65000);
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    assert!(cap.exit_ok);
    let dl = cap.page(1).unwrap();
    let glyphs = &dl.math_owners.as_ref().unwrap().glyphs;
    assert_eq!(glyphs.len(), 2);
    assert_eq!(
        glyphs.iter().map(|g| (g.owner, g.char)).collect::<Vec<_>>(),
        vec![(110, '中' as i64), (110, '文' as i64)]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_edit_and_export_keep_owners() {
    let Some((tl, root, project)) = setup("live", SOURCE) else {
        return;
    };
    for warm in [false, true] {
        let mut cfg = SessionConfig::new(&project, "main.tex");
        cfg.build_dir = root.join(format!("session-{warm}"));
        cfg.math_owner_attribute = Some(65000);
        cfg.eligibility = rtex_core::session::EligibilityMode::AllowList;
        cfg.warm_background = warm;
        cfg.fast_budget = Duration::from_secs(5);
        let s = Session::open(cfg).unwrap();
        let (ev, _) = s.wait_for(Duration::from_secs(120), |e| {
            matches!(
                e,
                Event::LayoutUpdate {
                    convergence: Convergence::Converged,
                    ..
                }
            )
        });
        let Some(Event::LayoutUpdate {
            compile,
            pages_changed,
            eligible_paragraphs,
            ..
        }) = ev
        else {
            panic!("no layout")
        };
        assert_eq!(compile, CompileStatus::Ok);
        assert_eq!(eligible_paragraphs.len(), 10);
        assert_owners(&pages_changed[0].dl);
        s.pause_background(true);
        let pos = SOURCE.find("=11 a").unwrap() + 4;
        let edit = s
            .apply_edit(
                "main.tex",
                Edit {
                    start_byte: pos,
                    end_byte: pos + 1,
                    text: "z".into(),
                },
            )
            .unwrap();
        assert_eq!(edit.routed, "fast", "{:?}", edit.reasons);
        let (ev, _) = s.wait_for(Duration::from_secs(60), |e| {
            matches!(e,
            Event::ParagraphUpdate { edit_id, .. } if *edit_id == edit.edit_id)
        });
        let Some(Event::ParagraphUpdate { status, dl, .. }) = ev else {
            panic!("no live result")
        };
        assert_eq!(status, "ok");
        let glyphs = &dl.math_owners.as_ref().unwrap().glyphs;
        assert_eq!(
            glyphs.iter().map(|g| (g.owner, g.char)).collect::<Vec<_>>(),
            vec![(11, 122), (12, 98)]
        );
        let out = root.join(format!("export-{warm}.pdf"));
        let job = s.export_pdf(&out);
        let (ev, _) = s.wait_for(Duration::from_secs(120), |e| {
            matches!(e,
            Event::PdfExported { job_id, .. } if *job_id == job)
        });
        let Some(Event::PdfExported {
            status, converged, ..
        }) = ev
        else {
            panic!("no export")
        };
        assert_eq!(status, CompileStatus::Ok);
        assert!(converged);
        let clean_src = root.join(format!("clean-src-{warm}"));
        std::fs::create_dir_all(&clean_src).unwrap();
        std::fs::write(
            clean_src.join("main.tex"),
            s.document_text("main.tex").unwrap(),
        )
        .unwrap();
        let clean = run_pass(
            &tl,
            &clean_src,
            "main.tex",
            &root.join(format!("clean-{warm}")),
            5,
            BibTool::Auto,
            false,
        )
        .unwrap();
        assert!(clean.aux_stable && clean.capture.exit_ok);
        assert_eq!(
            std::fs::read(&out).unwrap(),
            std::fs::read(&clean.capture.pdf).unwrap()
        );
        s.close();
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unset_matches_baseline_display_lists() {
    // Optional comparison against an unmodified checkout's support files, on this machine's
    // fonts and engine. Avoid a golden file tied to one TeX Live/font installation.
    let Some(baseline) = std::env::var_os("RTEX_BASELINE_TEXDIR") else {
        eprintln!("SKIP: set RTEX_BASELINE_TEXDIR to main's tex/ for byte identity against main");
        return;
    };
    let Some((mut tl, root, project)) = setup(
        "baseline",
        include_str!("../../../fixtures/corpus/math/main.tex"),
    ) else {
        return;
    };
    let current = run_capture(&tl, &project, "main.tex", &root.join("current"), true).unwrap();
    assert!(current.exit_ok);
    let preamble = rtex_core::project_preamble(&project, "main.tex").unwrap();
    let ctx = current
        .json
        .paragraphs
        .iter()
        .find(|u| u.is_top_level())
        .unwrap()
        .context_json();
    let source = "An existing math fixture: $\\frac{x}{y} + x_i^f + \\sqrt[3]{y}$.";
    let mut server = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve-current"),
        &preamble,
        1,
        None,
    )
    .unwrap();
    server.set_context(1, &ctx).unwrap();
    let (now, _) = server.compile(1, source).unwrap();
    server.shutdown().unwrap();
    tl.rtex_texdir = Path::new(&baseline).canonicalize().unwrap();
    let old = run_capture(&tl, &project, "main.tex", &root.join("baseline"), true).unwrap();
    assert!(old.exit_ok);
    assert_eq!(
        std::fs::read(current.out_dir.join("main.rtex-page1.json")).unwrap(),
        std::fs::read(old.out_dir.join("main.rtex-page1.json")).unwrap()
    );
    assert_eq!(
        current.page(1).unwrap().to_binary(),
        old.page(1).unwrap().to_binary()
    );
    let mut server =
        FastServer::spawn(&tl, &project, &root.join("serve-old"), &preamble, 1, None).unwrap();
    server.set_context(1, &ctx).unwrap();
    let (old, _) = server.compile(1, source).unwrap();
    server.shutdown().unwrap();
    assert_eq!(now.status, "ok");
    assert_eq!(old.status, "ok");
    assert_eq!(
        now.dl_binary, old.dl_binary,
        "raw live binary changed with option unset"
    );
    assert_eq!(
        serde_json::to_vec(&now.dl).unwrap(),
        serde_json::to_vec(&old.dl).unwrap()
    );
    let _ = std::fs::remove_dir_all(root);
}
