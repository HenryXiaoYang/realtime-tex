//! TFM virtual fonts in the live display list. Skipped without lualatex or newtxmath.

use rtex_core::{Convergence, Edit, Event, Session, SessionConfig};
use std::collections::BTreeSet;
use std::time::Duration;

fn kpsewhich(file: &str) -> bool {
    std::process::Command::new("kpsewhich")
        .arg(file)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

#[test]
fn newtxmath_glyphs_name_the_real_fonts() {
    // newtxmath's math fonts (ntxmi, ntxsy, ...) are TFM fonts with a .vf. LuaTeX reports them
    // as real fonts and reads the .vf only when it writes the PDF, so the live display list
    // named the virtual font, which no map file gives an outline file for
    if rtex_core::texlive::TexLive::discover().is_err() {
        eprintln!("SKIP: no lualatex");
        return;
    }
    if !kpsewhich("newtxmath.sty") {
        eprintln!("SKIP: no newtxmath.sty");
        return;
    }
    let root = std::env::temp_dir().join(format!("rtex-vf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let main = "\\documentclass{article}\n\\usepackage{newtxmath}\n\\begin{document}\n\
                Some text with $a+b$ in it.\n\nA second paragraph.\n\\end{document}\n";
    std::fs::write(project.join("main.tex"), main).unwrap();
    let mut cfg = SessionConfig::new(&project, "main.tex");
    cfg.build_dir = root.join("build");
    cfg.debounce = Duration::from_millis(50);
    cfg.fast_budget = Duration::from_millis(5000); // timing is not what this test measures
    let s = Session::open(cfg).unwrap();
    let (ev, others) = s.wait_for(Duration::from_secs(180), |e| {
        matches!(
            e,
            Event::LayoutUpdate {
                convergence: Convergence::Converged | Convergence::PassLimitReached { .. },
                ..
            }
        )
    });
    for o in others {
        s.requeue(o);
    }
    ev.expect("no converged layout");

    let at = main.find("a+b").unwrap() + "a+b".len();
    s.apply_edit(
        "main.tex",
        Edit {
            start_byte: at,
            end_byte: at,
            text: "-c".into(),
        },
    )
    .unwrap();
    let (ev, _) = s.wait_for(Duration::from_secs(30), |e| {
        matches!(
            e,
            Event::ParagraphUpdate { .. } | Event::BackgroundScheduled { .. }
        )
    });
    let dl = match ev {
        Some(Event::ParagraphUpdate { status, dl, .. }) => {
            assert_eq!(status, "ok");
            dl
        }
        other => panic!("the paragraph is not live: {other:?}"),
    };
    // TFM fonts (no file name) the glyphs are drawn in: the math fonts' base fonts
    let tfm: BTreeSet<String> = dl
        .lines
        .iter()
        .flat_map(|l| l.items.iter())
        .filter_map(|i| match i {
            rtex_dl::Item::Glyph { font, .. } => dl.fonts.get(&font.to_string()),
            _ => None,
        })
        .filter(|f| f.filename.is_none())
        .filter_map(|f| f.name.clone())
        .collect();
    assert!(!tfm.is_empty(), "no math glyphs in the live display list");
    let virtual_fonts: Vec<&String> = tfm
        .iter()
        .filter(|name| kpsewhich(&format!("{name}.vf")))
        .collect();
    assert!(
        virtual_fonts.is_empty(),
        "glyphs name virtual fonts {virtual_fonts:?} (all TFM fonts: {tfm:?})"
    );
}
