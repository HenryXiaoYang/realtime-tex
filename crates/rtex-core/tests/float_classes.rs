//! Floats whose class sets them up in its own float environments (Elsevier's cas-sc, issue #49):
//! a float compiled live gets what the environment's begin code does around `\@float` and what
//! packages add to `\@xfloat` (setspace's single spacing), exactly as in the full run. Skipped
//! without lualatex or setspace.sty.

mod common;

use common::differing;

const PAR: &str = "Ordinary words fill this paragraph so that it runs over a few lines of the \
                   page, double spaced as a manuscript for review is.";

/// Like cas-common.sty: `\@makecaption` set before `\@float`, the font after it, both local to
/// the environment.
const CLS: &str = "\\NeedsTeXFormat{LaTeX2e}\n\\ProvidesClass{floatcls}\n\\LoadClass{article}\n\
    \\RequirePackage{setspace}\n\\AtBeginDocument{\\doublespacing}\n\
    \\long\\def\\fcls@figcaption#1#2{\\vskip6pt\\parbox{\\linewidth}{\\raggedright\\sffamily\\small\
    \\textbf{#1:}~#2\\par}\\vskip6pt}\n\
    \\renewenvironment{figure}[1][]{\\let\\@makecaption\\fcls@figcaption\\@float{figure}\\raggedleft\
    \\sffamily\\small}{\\end@float}\n";

#[test]
fn floats_get_the_class_float_environment() {
    let main = format!(
        "\\documentclass{{floatcls}}\n\\begin{{document}}\n{PAR}\n\n\
         \\begin{{figure}}[pos=!htbp]\n\\centering\n\\rule{{3cm}}{{1cm}}\n\
         \\caption{{A caption under the figure, with $x^2$ in it.}}\n\\end{{figure}}\n\n{PAR}\n\n\
         \\begin{{table}}[!t]\n\\centering\n\\caption{{A table caption long enough to run over \
         two lines in the single spacing setspace gives floats.}}\n\
         \\begin{{tabular}}{{cc}}\nalpha & beta \\\\\ngamma & delta \\\\\n\\end{{tabular}}\n\
         \\end{{table}}\n\n{PAR}\n\\end{{document}}\n"
    );
    let Some(units) = common::units(
        "float-classes",
        &[("main.tex", &main), ("floatcls.cls", CLS)],
        &["setspace.sty"],
    ) else {
        return;
    };
    let floats: Vec<&str> = units
        .iter()
        .filter(|u| u.name.starts_with("env:") && u.verdict.is_some())
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(floats, ["env:figure", "env:table"], "{units:?}");
    let differing = differing(&units);
    assert!(
        differing.is_empty(),
        "differ from the full run: {differing:?}"
    );
}
