//! Robustness of the persistent paragraph server. Skipped (with a notice) when lualatex is
//! not available; run `source build/texlive.env` first.

use rtex_core::capture::run_capture;
use rtex_core::engine::{FastServer, Response};
use rtex_core::fixtures::{generate, FontSet, Variant};
use rtex_core::texlive::TexLive;
use std::time::Duration;

fn setup(name: &str) -> Option<(TexLive, std::path::PathBuf, std::path::PathBuf)> {
    let tl = match TexLive::discover() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let root = std::env::temp_dir().join(format!("rtex-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    generate(2, Variant::Pure, FontSet::Pagella, 7, &project).unwrap();
    Some((tl, root, project))
}

/// A project whose `main.tex` is `main`.
fn setup_document(
    name: &str,
    main: &str,
) -> Option<(TexLive, std::path::PathBuf, std::path::PathBuf)> {
    let tl = match TexLive::discover() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let root = std::env::temp_dir().join(format!("rtex-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("main.tex"), main).unwrap();
    Some((tl, root, project))
}

fn preamble(project: &std::path::Path) -> String {
    let main = std::fs::read_to_string(project.join("main.tex")).unwrap();
    rtex_core::split_preamble(&main).unwrap().0.to_string()
}

/// (unit id, context) of the paragraph units of a capture, in document order.
fn paragraph_units(
    cap: &rtex_core::capture::CaptureResult,
    project: &std::path::Path,
) -> Vec<(i64, serde_json::Value)> {
    let policy = rtex_core::eligibility::Policy::from_preamble(&preamble(project), &[], &[]);
    let (store, _fb) =
        rtex_core::layout::LayoutStore::offline(cap, project, "main.tex", &policy).unwrap();
    store
        .mapped_units()
        .into_iter()
        .filter(|(u, _)| u.kind() == "par" && u.rows() > 0)
        .map(|(u, _)| (u.uid, u.context_json()))
        .collect()
}

#[test]
fn errors_do_not_poison_the_server() {
    let Some((tl, root, project)) = setup("errors") else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let (seq, ctx) = paragraph_units(&cap, &project).remove(0);
    let para_seq = seq;
    let mut s = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        1,
        None,
    )
    .unwrap();
    s.set_context(para_seq, &ctx).unwrap();
    let (ok, _) = s
        .compile(para_seq, "A plain paragraph of text that is fine.")
        .unwrap();
    assert_eq!(ok.status, "ok");
    let lines_ok = ok.dl.unwrap().lines.len();

    // undefined macro → error status with a diagnostic, server keeps going
    let (bad, _) = s
        .compile(para_seq, "Text with \\nosuchmacro here.")
        .unwrap();
    assert_eq!(bad.status, "error");
    assert!(bad.errors.iter().any(|e| e
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Undefined control sequence")));

    // unbalanced brace → TeX inserts the missing brace; still an error, state stays consistent
    let (unb, _) = s.compile(para_seq, "Unbalanced { brace here.").unwrap();
    assert_eq!(unb.status, "error");

    // footnote → insert node: the mark is typeset, the insert is counted (text refreshed by the
    // next layout), no error
    let (fn_, _) = s.compile(para_seq, "Footnote\\footnote{x} here.").unwrap();
    assert_eq!(fn_.status, "ok", "status {}", fn_.status);
    assert_eq!(fn_.dl.as_ref().unwrap().inserts, 1);

    // and the good paragraph still compiles identically
    let (again, _) = s
        .compile(para_seq, "A plain paragraph of text that is fine.")
        .unwrap();
    assert_eq!(again.status, "ok");
    assert_eq!(again.dl.unwrap().lines.len(), lines_ok);
    match s.stats().unwrap() {
        Response::Stats {
            grouplevel, nest, ..
        } => {
            assert_eq!(grouplevel, 0);
            assert_eq!(nest, 0);
        }
        other => panic!("{other:?}"),
    }
    s.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn watchdog_kills_a_runaway_paragraph() {
    let Some((tl, root, project)) = setup("watchdog") else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let (seq, ctx) = paragraph_units(&cap, &project).remove(0);
    let mut s = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        1,
        None,
    )
    .unwrap();
    s.set_context(seq, &ctx).unwrap();
    s.timeout = Duration::from_millis(800);
    let t0 = std::time::Instant::now();
    let r = s.compile(seq, "\\loop\\iftrue\\repeat never ends");
    assert!(r.is_err(), "runaway paragraph must time out");
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(!s.is_alive(), "server must be killed by the watchdog");
    // a fresh generation works again
    let mut s2 = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        2,
        None,
    )
    .unwrap();
    s2.set_context(seq, &ctx).unwrap();
    let (ok, _) = s2.compile(seq, "Back to normal.").unwrap();
    assert_eq!(ok.status, "ok");
    s2.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn paragraph_result_is_independent_of_request_order() {
    let Some((tl, root, project)) = setup("order") else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let paras: Vec<(i64, serde_json::Value)> = paragraph_units(&cap, &project)
        .into_iter()
        .take(3)
        .collect();
    let mut s = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        1,
        None,
    )
    .unwrap();
    for (seq, ctx) in &paras {
        s.set_context(*seq, ctx).unwrap();
    }
    let src =
        "Order independence: {\\itshape italic} and $x_1 + y^2$ with \\textbf{bold} words in it.";
    let (a, _) = s.compile(paras[0].0, src).unwrap();
    let (_b, _) = s
        .compile(paras[1].0, "Something else entirely, \\emph{different}.")
        .unwrap();
    let (c, _) = s.compile(paras[0].0, src).unwrap();
    assert_eq!(
        serde_json::to_string(&a.dl).unwrap(),
        serde_json::to_string(&c.dl).unwrap()
    );
    s.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// A compile that changes the meaning of a control sequence its source mentions reports it.
#[test]
fn leaked_definitions_are_reported() {
    let Some((tl, root, project)) = setup("leaks") else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let (seq, ctx) = paragraph_units(&cap, &project).remove(0);
    let mut s = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        1,
        None,
    )
    .unwrap();
    s.set_context(seq, &ctx).unwrap();
    let (clean, _) = s
        .compile(seq, "Plain text with \\emph{emphasis} only.")
        .unwrap();
    assert_eq!(clean.status, "ok");
    assert!(clean.leaks.is_empty(), "{:?}", clean.leaks);
    // a local definition dies with the unit's box group: no leak
    let (local, _) = s
        .compile(
            seq,
            "Defines \\newcommand{\\lkmacro}{local} and uses \\lkmacro{} here.",
        )
        .unwrap();
    assert_eq!(local.status, "ok", "{:?}", local.errors);
    assert!(local.leaks.is_empty(), "{:?}", local.leaks);
    let (leaky, _) = s
        .compile(
            seq,
            "Defines \\gdef\\lkglobal{leaked} and uses \\lkglobal{} here.",
        )
        .unwrap();
    assert_eq!(leaky.status, "ok", "{:?}", leaky.errors);
    assert_eq!(leaky.leaks, vec!["lkglobal".to_string()]);
    let (relet, _) = s
        .compile(seq, "Text \\global\\let\\emph\\relax more.")
        .unwrap();
    assert_eq!(relet.leaks, vec!["emph".to_string()], "{:?}", relet.leaks);
    s.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// The server must be able to finish writing a result while the host is not reading it yet: a
/// FIFO holds 64 KB on Linux and less on macOS, where a 9 KB display list blocked the server's
/// write and the host's watchdog killed it. Here a result larger than any FIFO capacity is
/// produced and nothing is read for two seconds; the server's trace must show it was sent
/// (the host drains the FIFO on a reader thread), and the result then arrives intact.
#[test]
fn a_large_result_is_drained_while_the_host_is_not_reading() {
    let Some((tl, root, project)) = setup("drain") else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let (seq, ctx) = paragraph_units(&cap, &project).remove(0);
    let serve = root.join("serve");
    let mut s =
        FastServer::spawn_with(&tl, &project, &serve, &preamble(&project), 1, None, true).unwrap();
    s.set_context(seq, &ctx).unwrap();
    // a paragraph long enough for a display list well beyond 64 KB
    let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor ";
    let source = words.repeat(150);
    let req = s.next_req_id();
    s.send_compile(req, seq, &source, "").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let trace = std::fs::read_to_string(serve.join("rtex-serve-g1.trace")).unwrap();
    assert!(
        trace.contains(&format!("finish: result sent req {req}")),
        "the server is still blocked writing its result while the host is not reading:\n{}",
        trace.lines().rev().take(6).collect::<Vec<_>>().join("\n")
    );
    match s.recv().unwrap() {
        Response::Result(cr) => {
            assert_eq!(cr.status, "ok", "{:?}", cr.errors);
            assert!(cr.dl_bytes > 64 * 1024, "result of {} bytes", cr.dl_bytes);
            assert!(cr.dl.map(|d| d.lines.len()).unwrap_or(0) > 20);
        }
        other => panic!("unexpected {other:?}"),
    }
    let _ = s.shutdown();
    let _ = std::fs::remove_dir_all(&root);
}

/// Captures `body` under a fontspec + bm preamble and compiles its `units` units in document
/// order on one fresh server, as `rtex verify` does: each must compile without error and give
/// the rows of the capture.
fn bm_units_reproduce_the_capture(name: &str, body: &str, units: usize) {
    let main = format!(
        "\\documentclass{{article}}\n\\usepackage{{fontspec}}\n\\usepackage{{amsmath}}\n\
         \\usepackage{{bm}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n"
    );
    let Some((tl, root, project)) = setup_document(name, &main) else {
        return;
    };
    let cap = run_capture(&tl, &project, "main.tex", &root.join("cap"), true).unwrap();
    let policy = rtex_core::eligibility::Policy::from_preamble(&preamble(&project), &[], &[]);
    let (store, files) =
        rtex_core::layout::LayoutStore::offline(&cap, &project, "main.tex", &policy).unwrap();
    let mapped: Vec<_> = store
        .mapped_units()
        .into_iter()
        .filter(|(u, _)| u.rows() > 0)
        .collect();
    assert_eq!(mapped.len(), units);
    let mut s = FastServer::spawn(
        &tl,
        &project,
        &root.join("serve"),
        &preamble(&project),
        1,
        None,
    )
    .unwrap();
    for (u, span) in mapped {
        s.set_context(u.uid, &u.context_json()).unwrap();
        let src = files.fast_source(span).unwrap();
        let (r, _) = s
            .compile(u.uid, &src)
            .unwrap_or_else(|e| panic!("unit {}: {e}", u.uid));
        assert_eq!(r.status, "ok", "unit {}: {:?}", u.uid, r.errors);
        if let Err(why) = store.probe_check(span, r.dl.as_ref().unwrap()) {
            panic!("unit {} differs from the capture: {why}", u.uid);
        }
    }
    s.shutdown().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

/// `\bm` typesets its argument in the bold math version, and entering that version re-installs
/// the version's math alphabets globally (`\mathbf` becomes the bold version's). The server's
/// memoized `\glb@settings` (rtex-serve-patches.tex, patch 2) must re-install the normal
/// version's alphabets when math comes back to it, or `\mathbf{M}` is set in the bold
/// version's font.
#[test]
fn math_alphabets_return_with_the_normal_math_version() {
    bm_units_reproduce_the_capture(
        "bm-alphabet",
        "Let $\\bm{0}$ be zero and $\\mathbf{M}$ a matrix.",
        1,
    );
}

/// The same after `\mathbf` in the normal version and a footnote: `\mathbf{M}` after `\bm`
/// used to loop through the kernel's alphabet allocation until TeX gave up, which closed the
/// server's response channel.
#[test]
fn math_alphabets_after_a_footnote_and_bold_math() {
    bm_units_reproduce_the_capture(
        "bm-footnote",
        "where $\\mathbf{c}$ denotes the customer count of each zone.\n\n\
         online.\\footnote{x}\n\n\
         $\\bm{\\beta}_{s}=\\bm{0}$ for a substation $s$ and $\\mathbf{M}=\\bm{0}$.",
        3,
    );
}
