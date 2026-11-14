//! The `units` variant of the book: every kind of unit (lists, floats, tables, footnotes, display math, headings).

use super::png::*;
use super::text::*;
use super::{preamble_for, FontSet, Variant};
use std::fmt::Write as _;
use std::path::Path;

/// The `units` variant: a rotation of every unit kind the fast path supports, so each kind
/// appears many times in a 10-page book and can be verified and benchmarked.
pub(super) fn generate_units(
    pages: u32,
    fonts: FontSet,
    seed: u64,
    out: &Path,
    rng: &mut Rng,
) -> anyhow::Result<()> {
    // measured: ≈ 230 words of body text per page once the structural units are counted in
    let target_words = (pages as usize * 230).max(500);
    let mut body = String::new();
    let mut words = 0usize;
    let mut chapter = 0;
    let mut eq = 0;
    let mut fig = 0;
    let mut tab = 0;
    let mut thm = 0;
    let mut para_idx = 0;
    let mut labels: Vec<String> = Vec::new();
    while words < target_words {
        if para_idx % 9 == 0 {
            chapter += 1;
            let _ = writeln!(
                body,
                "\\chapter{{Chapter {}}}\\label{{ch:{}}}\n",
                chapter, chapter
            );
            labels.push(format!("ch:{chapter}"));
            words += 60;
        } else if para_idx % 3 == 0 {
            let _ = writeln!(
                body,
                "\\section{{Section {}}}\\label{{sec:{}}}\n",
                para_idx, para_idx
            );
            labels.push(format!("sec:{para_idx}"));
            words += 25;
        }
        let roll = rng.below(100);
        let sentences = if roll < 12 {
            1
        } else if roll < 24 {
            rng.range(10, 14)
        } else {
            rng.range(2, 7)
        };
        let (mut p, w) = if sentences == 1 {
            short_paragraph(rng)
        } else {
            paragraph(rng, sentences, true)
        };
        words += w;
        // one structural feature per paragraph, in rotation
        match para_idx % 10 {
            0 => {
                // plain
            }
            1 => {
                eq += 1;
                p.push_str(&format!(" The identity\n\\begin{{equation}} \\int_0^1 x^{} \\, dx = \\frac{{1}}{{{}}} \\label{{eq:{}}} \\end{{equation}}\nholds for every exponent, and we use it below.", eq % 7 + 1, eq % 7 + 2, eq));
                labels.push(format!("eq:{eq}"));
            }
            2 => {
                p.push_str(" The displayed sum\n\\[ \\sum_{i=1}^{n} i = \\frac{n(n+1)}{2} \\]\nis classical.");
            }
            3 => {
                p.push_str("\\footnote{A footnote with a short remark about the preceding sentence.} The text goes on after the footnote mark.");
            }
            4 => {
                if !labels.is_empty() {
                    p.push_str(&format!(" As shown in Section~\\ref{{{}}} on page~\\pageref{{{}}}, this holds; see also~\\cite{{knuth1981}}.", labels[rng.below(labels.len() as u64) as usize], labels[0]));
                }
                if eq > 0 {
                    p.push_str(&format!(" Equation~\\eqref{{eq:{}}} is used here.", eq));
                }
            }
            5 => {
                eq += 2;
                p.push_str(&format!(" Two aligned relations\n\\begin{{align}} a_{} &= b + c \\label{{eq:{}}} \\\\ d &= \\sqrt{{e^2 + f^2}} \\label{{eq:{}}} \\end{{align}}\nfinish the paragraph.", eq, eq - 1, eq));
            }
            _ => {}
        }
        let _ = writeln!(body, "{}\n", wrap(&p, 78));
        match para_idx % 10 {
            6 => {
                let _ = writeln!(body, "\\begin{{itemize}}\n\\item First item of a list, with a few words so that it is not trivial.\n\\item Second item, with inline math $x_{} + y$ in it.\n\\item Third.\n\\end{{itemize}}\n", para_idx);
                words += 30;
            }
            7 => {
                fig += 1;
                let _ = writeln!(body, "\\begin{{figure}}[tbp]\n\\centering\n\\includegraphics[width=0.4\\textwidth]{{images/figure.png}}\n\\caption{{Figure {} shows the placeholder image with a caption long enough to wrap onto a second line of the caption block.}}\\label{{fig:{}}}\n\\end{{figure}}\n", fig, fig);
                words += 110;
            }
            8 => {
                tab += 1;
                let _ = writeln!(body, "\\begin{{table}}[tbp]\n\\centering\n\\caption{{Table {} with three columns.}}\\label{{tab:{}}}\n\\begin{{tabular}}{{lrr}}\n\\toprule\nName & Value & Count \\\\\n\\midrule\nalpha & {} & 3 \\\\\nbeta & {} & 12 \\\\\ngamma & {} & 7 \\\\\n\\bottomrule\n\\end{{tabular}}\n\\end{{table}}\n", tab, tab, tab * 11, tab * 7, tab * 5);
                words += 90;
            }
            9 => {
                thm += 1;
                if thm % 2 == 1 {
                    let _ = writeln!(body, "\\begin{{theorem}}\\label{{thm:{}}}\nFor every $n \\geq 1$ the sum of the first $n$ odd numbers is $n^2$, and the statement is long enough to wrap.\n\\end{{theorem}}\n", thm);
                } else {
                    let _ = writeln!(body, "\\begin{{quote}}\nA quotation set with the quote environment, indented on both sides and long enough to need a second line of text.\n\\end{{quote}}\n");
                }
                words += 40;
            }
            _ => {}
        }
        para_idx += 1;
    }
    let mut main = preamble_for(fonts, Variant::Units);
    main.push_str("\\tableofcontents\n\n");
    main.push_str(&body);
    main.push_str("\\bibliographystyle{plain}\n\\bibliography{refs}\n");
    main.push_str("\\end{document}\n");
    std::fs::write(out.join("main.tex"), main)?;
    std::fs::create_dir_all(out.join("images"))?;
    std::fs::write(out.join("images").join("figure.png"), png_placeholder())?;
    std::fs::write(
        out.join("refs.bib"),
        "@article{knuth1981,\n  author = {Donald E. Knuth and Michael F. Plass},\n  title = {Breaking paragraphs into lines},\n  journal = {Software: Practice and Experience},\n  year = {1981},\n  volume = {11},\n  number = {11},\n  pages = {1119--1184}\n}\n",
    )?;
    std::fs::write(
        out.join("fixture.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "pages_requested": pages, "variant": "units", "fonts": format!("{:?}", fonts), "seed": seed,
            "paragraphs": para_idx, "words": words, "equations": eq, "figures": fig, "tables": tab,
        }))?,
    )?;
    Ok(())
}
