//! IEEEtran (the two-column journal class) under LuaLaTeX: units the class typesets differently
//! from the standard classes compile live exactly as in the full run. Skipped without lualatex
//! or IEEEtran.cls.

use rtex_core::capture::run_capture;
use rtex_core::eligibility::{check_engine_unit, classify_source, Policy, Reason};
use rtex_core::engine::FastServer;
use rtex_core::layout::LayoutStore;
use rtex_core::texlive::TexLive;

const PAR: &str = "Ordinary words fill this paragraph so that it runs over a few lines of the \
                   narrow column of the journal layout here.";

/// A unit of the captured document: "kind:name", why it may not go live (the allow-list's
/// checks, as `rtex verify` applies them) and, when it may, how its live compile compares with
/// its rows on the pages.
#[derive(Debug)]
struct Unit {
    name: String,
    reasons: Vec<Reason>,
    verdict: Option<Result<(), String>>,
}

/// Captures `body` in an IEEEtran journal document and compiles every unit with rows that may go
/// live in the live engine. Units in document order.
fn units(name: &str, body: &str) -> Option<Vec<Unit>> {
    let Ok(tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return None;
    };
    let have_class = std::process::Command::new("kpsewhich")
        .arg("IEEEtran.cls")
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if !have_class {
        eprintln!("SKIP: no IEEEtran.cls");
        return None;
    }
    let root = std::env::temp_dir().join(format!("rtex-ieeetran-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let main = format!(
        "\\documentclass[journal]{{IEEEtran}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n"
    );
    std::fs::write(project.join("main.tex"), &main).unwrap();
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let preamble = rtex_core::split_preamble(&main).unwrap().0.to_string();
    let policy = Policy::from_preamble(&preamble, &[], &[]);
    let (store, files) = LayoutStore::offline(&cap, &project, "main.tex", &policy).unwrap();
    let mut srv =
        FastServer::spawn(&tl, &project, &root.join("serve"), &preamble, 1, None).unwrap();
    let mut out = Vec::new();
    for (eu, id) in store.mapped_units() {
        if eu.rows() == 0 {
            continue;
        }
        let c = &eu.captured;
        let src = files.fast_source(id).unwrap();
        let mut reasons = classify_source(&src, &policy).1;
        reasons.extend(check_engine_unit(
            &c.kind,
            &c.everypar,
            eu.has_context(),
            eu.rows(),
            &eu.flags,
            c.name.as_deref().filter(|_| c.runin),
        ));
        let verdict = reasons.is_empty().then(|| {
            srv.set_context(eu.uid, &eu.context_json()).unwrap();
            let (res, _) = srv.compile(eu.uid, &src).unwrap();
            match &res.dl {
                Some(dl) => store.probe_check(id, dl),
                None => Err(format!("no result: {}", res.status)),
            }
        });
        out.push(Unit {
            name: format!("{}:{}", eu.kind(), eu.name().unwrap_or_default()),
            reasons,
            verdict,
        });
    }
    let _ = srv.shutdown();
    let _ = std::fs::remove_dir_all(&root);
    Some(out)
}

/// Units that went live and differ from the full run.
fn differing(units: &[Unit]) -> Vec<&Unit> {
    units
        .iter()
        .filter(|u| matches!(u.verdict, Some(Err(_))))
        .collect()
}

/// IEEEtran redefines `\@floatboxreset` inside `\table` and `\table*` (`\footnotesize`) and
/// inside `\figure` and `\figure*` (space at the top of the float): a float compiled live must
/// get the class's float box, not the kernel's `\normalsize` one.
#[test]
fn floats_get_the_class_float_box() {
    let tabular = "\\centering\n\\caption{A table}\n\\begin{tabular}{cc}\nalpha & beta \\\\\n\
                   gamma & delta \\\\\n\\end{tabular}\n";
    let figure = "\\centering\n\\rule{2cm}{1cm}\n\\caption{A figure with a caption}\n";
    let body = format!(
        "{PAR}\n\n\\begin{{table}}[!t]\n{tabular}\\end{{table}}\n\n{PAR}\n\n\
         \\begin{{figure}}[!t]\n{figure}\\end{{figure}}\n\n{PAR}\n\n\
         \\begin{{table*}}[!t]\n{tabular}\\end{{table*}}\n\n{PAR}\n\n\
         \\begin{{figure*}}[!t]\n{figure}\\end{{figure*}}\n\n{PAR}\n"
    );
    let Some(units) = units("floats", &body) else {
        return;
    };
    // every float may go live
    let floats: Vec<(&str, &[Reason])> = units
        .iter()
        .filter(|u| u.name.starts_with("env:"))
        .map(|u| (u.name.as_str(), u.reasons.as_slice()))
        .collect();
    assert_eq!(
        floats,
        [
            ("env:table", &[][..]),
            ("env:figure", &[]),
            ("env:table*", &[]),
            ("env:figure*", &[])
        ],
        "{units:?}"
    );
    let differing = differing(&units);
    assert!(
        differing.is_empty(),
        "differ from the full run: {differing:?}"
    );
}

/// IEEEtran's `\subsubsection` is a run-in heading, like `\paragraph`: its title is set at the
/// start of the following paragraph, whose lines the heading's unit then has. The capture sees
/// it run in, so the heading waits for the full compile and says why, once.
#[test]
fn run_in_headings_wait_for_the_full_compile() {
    let body = format!(
        "\\section{{Method}}\n{PAR}\n\n\\subsubsection{{Device paths}}\n{PAR}\n\n{PAR}\n\n\
         \\subsubsection*{{Unnumbered paths}}\n{PAR}\n"
    );
    let Some(units) = units("runin", &body) else {
        return;
    };
    let section = units.iter().find(|u| u.name == "heading:section");
    assert!(section.is_some_and(|u| u.reasons.is_empty()), "{units:?}");
    let run_in: Vec<&Vec<Reason>> = units
        .iter()
        .filter(|u| u.name == "heading:subsubsection")
        .map(|u| &u.reasons)
        .collect();
    assert_eq!(run_in.len(), 2, "{units:?}");
    for reasons in run_in {
        let n = reasons
            .iter()
            .filter(|r| **r == Reason::RunInHeading("subsubsection".into()))
            .count();
        assert_eq!(n, 1, "{units:?}");
    }
    let differing = differing(&units);
    assert!(
        differing.is_empty(),
        "differ from the full run: {differing:?}"
    );
}

/// The paragraph after a run-in heading's own paragraph starts under the heading's `\everypar`,
/// which by then only restores `\clubpenalty` and empties itself: it goes live like any other,
/// and so does a heading that follows it.
#[test]
fn the_paragraph_after_a_run_in_heading_goes_live() {
    let body = format!(
        "\\subsubsection{{Device paths}}\n{PAR}\n\n{PAR}\n\n{PAR}\n\n\
         \\subsubsection{{Zones}}\n{PAR}\n\n\\section{{Assessment}}\n{PAR}\n"
    );
    let Some(units) = units("after-runin", &body) else {
        return;
    };
    let live: Vec<&str> = units
        .iter()
        .filter(|u| matches!(u.verdict, Some(Ok(()))))
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(
        live,
        ["par:", "par:", "heading:section", "par:"],
        "{units:?}"
    );
    let differing = differing(&units);
    assert!(
        differing.is_empty(),
        "differ from the full run: {differing:?}"
    );
}
