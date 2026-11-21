//! IEEEtran (the two-column journal class) under LuaLaTeX: units the class typesets differently
//! from the standard classes compile live exactly as in the full run. Skipped without lualatex
//! or IEEEtran.cls.

mod common;

use common::{differing, Unit};
use rtex_core::eligibility::Reason;

const PAR: &str = "Ordinary words fill this paragraph so that it runs over a few lines of the \
                   narrow column of the journal layout here.";

/// Captures `body` in an IEEEtran journal document and compiles its units (`common::units`).
fn units(name: &str, body: &str) -> Option<Vec<Unit>> {
    let main = format!(
        "\\documentclass[journal]{{IEEEtran}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n"
    );
    common::units(
        &format!("ieeetran-{name}"),
        &[("main.tex", &main)],
        &["IEEEtran.cls"],
    )
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
/// start of the following paragraph (or list), whose lines the heading's unit then has. The
/// heading's span runs on through that paragraph or list, with or without a label and a blank
/// line between, so it goes live like any heading.
#[test]
fn run_in_headings_go_live_with_the_paragraph_that_sets_their_title() {
    let list = "\\begin{IEEEdescription}[\\IEEEusemathlabelsep\\IEEEsetlabelwidth{$V_1$}]\n\
                \\item[$A$] The first symbol, with a description over a line or two of the column.\n\
                \\item[$B$] The second symbol.\n\\end{IEEEdescription}\n";
    let body = format!(
        "\\section{{Method}}\n{PAR}\n\n\\subsubsection{{Device paths}}\n{PAR}\n\n{PAR}\n\n\
         \\subsubsection{{Labelled paths}}\n\\label{{sec:lp}}\n\n{PAR}\n\n\
         \\subsubsection*{{Unnumbered paths}} {PAR}\n\n\
         \\subsubsection*{{Nomenclature}}\n{list}\n{PAR}\n"
    );
    let Some(units) = units("runin", &body) else {
        return;
    };
    let headings: Vec<(&str, &[Reason])> = units
        .iter()
        .filter(|u| u.name.starts_with("heading:"))
        .map(|u| (u.name.as_str(), u.reasons.as_slice()))
        .collect();
    assert_eq!(
        headings,
        [
            ("heading:section", &[][..]),
            ("heading:subsubsection", &[]),
            ("heading:subsubsection", &[]),
            ("heading:subsubsection", &[]),
            ("heading:subsubsection", &[]),
        ],
        "{units:?}"
    );
    assert!(
        units.iter().all(|u| u.verdict.is_some()),
        "all live: {units:?}"
    );
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
        [
            "heading:subsubsection",
            "par:",
            "par:",
            "heading:subsubsection",
            "heading:section",
            "par:"
        ],
        "{units:?}"
    );
    let differing = differing(&units);
    assert!(
        differing.is_empty(),
        "differ from the full run: {differing:?}"
    );
}
