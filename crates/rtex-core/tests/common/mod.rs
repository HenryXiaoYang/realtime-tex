//! Shared by the class tests: capture a document, compile each of its units that may go live in
//! the live engine and compare the result with the unit's rows on the pages.
#![allow(dead_code)] // each test crate uses part of it

use rtex_core::capture::run_capture;
use rtex_core::eligibility::{check_engine_unit, classify_source, runs_into_title, Policy, Reason};
use rtex_core::engine::FastServer;
use rtex_core::layout::LayoutStore;
use rtex_core::texlive::TexLive;

/// A unit of the captured document: "kind:name", why it may not go live (the allow-list's
/// checks, as `rtex verify` applies them) and, when it may, how its live compile compares with
/// its rows on the pages.
#[derive(Debug)]
pub struct Unit {
    pub name: String,
    pub reasons: Vec<Reason>,
    pub verdict: Option<Result<(), String>>,
}

/// Captures the project `files` (path, text; `main.tex` among them) and compiles every unit with
/// rows that may go live in the live engine. Units in document order; None without lualatex or
/// without one of the `needs` files on the TeX path.
pub fn units(name: &str, files: &[(&str, &str)], needs: &[&str]) -> Option<Vec<Unit>> {
    let Ok(tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return None;
    };
    for f in needs {
        let found = std::process::Command::new("kpsewhich")
            .arg(f)
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if !found {
            eprintln!("SKIP: no {f}");
            return None;
        }
    }
    let root = std::env::temp_dir().join(format!("rtex-units-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    for (path, text) in files {
        std::fs::write(project.join(path), text).unwrap();
    }
    let main = std::fs::read_to_string(project.join("main.tex")).unwrap();
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
            c.name
                .as_deref()
                .filter(|_| c.runin && !runs_into_title(&src)),
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
pub fn differing(units: &[Unit]) -> Vec<&Unit> {
    units
        .iter()
        .filter(|u| matches!(u.verdict, Some(Err(_))))
        .collect()
}
