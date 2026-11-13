use super::engine::*;
use super::lex::*;
use super::policy::*;
use super::reason::*;
use super::setup::*;
use super::shape::*;
use super::*;
use std::collections::BTreeMap;
fn pol() -> Policy {
    Policy {
        amsmath: true,
        graphicx: true,
        booktabs: true,
        tabularx: true,
        cite_ok: true,
        permissive: false,
        math_owner_attribute: None,
        heading_macros: BTreeMap::new(),
        packages: [
            "hyperref", "url", "natbib", "siunitx", "ulem", "listings", "multirow", "colortbl",
            "cancel", "bm", "amssymb",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        ..Default::default()
    }
}
fn ok(s: &str) -> bool {
    check_source(s, &pol()).is_empty()
}
fn reasons(s: &str) -> Vec<Reason> {
    check_source(s, &pol())
}
#[test]
fn math_owner_attribute_is_a_literal_opt_in() {
    let mut p = pol();
    let src = r"$\begingroup\attribute65000=42 \frac{x}{}\endgroup$";
    assert!(!classify_source(src, &p).1.is_empty());
    p.math_owner_attribute = Some(65000);
    for good in [
        src,
        r"$\attribute 65000 = -42 x$",
        r"\begingroup\attribute65000=0 text\endgroup",
        r"$\text{\attribute65000=17 abc}$",
        r"$\attribute65000=-2147483647 x$",
        r"${\attribute65000=+2147483647 x}$",
    ] {
        assert!(
            classify_source(good, &p).1.is_empty(),
            "{good}: {:?}",
            classify_source(good, &p).1
        );
    }
    for bad in [
        r"$\attribute65001=42 x$",
        r"$\attribute65000 x$",
        r"$\attribute65000=\count0 x$",
        r"$\attribute65000=2147483648 x$",
        r"$\attribute65000=- x$",
        r"$\attribute\count0=42 x$",
        r"$\attributedef\foo=65000 x$",
        r"$\global\attribute65000=1 x$",
        r"$\attribute65000=1 \endgroup x$",
    ] {
        assert!(!classify_source(bad, &p).1.is_empty(), "{bad}");
    }
    p.math_owner_attribute = Some(0);
    assert!(classify_source(r"$\attribute0=1 x$", &p).1.is_empty());
    assert!(!classify_source(r"$\attribute65000=1 x$", &p).1.is_empty());
}

#[test]
fn plain_and_fonts_ok() {
    assert!(ok(
        "Plain text, with \\emph{emphasis} and \\textbf{bold}, ``quotes'' and a dash---here."
    ));
    assert!(ok(
        "Inline $x^2 + \\frac{1}{2} \\leq \\alpha$ math and \\(a_i\\) too.\nSecond line."
    ));
    assert!(ok("{\\itshape grouped declaration} and {\\small small}."));
    assert!(ok(
        "A \\textcolor{red}{red} word and \\'e accents, 50\\% and \\&."
    ));
}
#[test]
fn disallowed() {
    assert!(reasons("Footnote\\footnote{x} here.").is_empty());
    assert!(reasons("\\mymacro{x}").contains(&Reason::DisallowedMacro("mymacro".into())));
    assert!(reasons("Text \\large leaking.")
        .contains(&Reason::SizeDeclarationOutsideGroup("large".into())));
    assert!(reasons("one\n\ntwo").contains(&Reason::ParagraphBreak));
    // an explicit \par is fine: a span may hold several consecutive paragraphs (one unit)
    assert!(!reasons("a \\par b").contains(&Reason::ParagraphBreak));
    // comment lines are not blank lines: `%` also removes the end of its line
    for src in [
        "% one\n% two\nHello world.\nMore.",
        "Hello world.\n% one\nMore.",
        "Hello world.\n% one\n% two\nMore.",
        "Hello % note\n% two\nworld.",
    ] {
        assert!(!reasons(src).contains(&Reason::ParagraphBreak), "{src:?}");
    }
    assert!(reasons("one\n% c\n\ntwo").contains(&Reason::ParagraphBreak));
    let r = reasons("{\\Large\\bfseries Title\\par}\nName \\hfill 22 September 2026");
    assert!(r.is_empty(), "{r:?}");
    assert!(!ok("a \\parbox-like? no: \\parskip is disallowed"));
    assert!(reasons("\\begin{tikzpicture}\\end{tikzpicture}")
        .contains(&Reason::DisallowedEnvironment("tikzpicture".into())));
    // opaque arguments and verbatim
    assert!(reasons(
        "see \\url{https://a.b/c_d%e#f} and \\href{https://x.y/_z}{the \\textbf{notes}}"
    )
    .is_empty());
    assert!(reasons("call \\verb|\\foo{bar}| and \\lstinline|add(2, 3)| here").is_empty());
    assert!(reasons("and \\verb+%percent+ signs").is_empty());
    assert_eq!(
        classify_source("\\vspace{1cm}\n\\begin{quote}x\\end{quote}", &pol()).0,
        UnitShape::Env("quote".into())
    );
    assert!(reasons("\\SI{2.5}{\\metre} and \\num{3.2e-1} \\si{\\ohm}").is_empty());
    assert!(reasons("\\begin{verbatim}\n\\foo \\bar{\n%x\n\\end{verbatim}").is_empty());
    assert!(
        reasons("\\begin{lstlisting}[language=C]\nint x; /* } */\n\\end{lstlisting}").is_empty()
    );
    // package-gated macros without the package
    let bare = Policy::default();
    assert!(classify_source("\\url{x}", &bare)
        .1
        .contains(&Reason::NeedsPackage("hyperref".into())));
    assert!(classify_source("\\sout{x}", &bare)
        .1
        .contains(&Reason::NeedsPackage("ulem".into())));
    // natbib citations, boxes, counters, setters
    assert!(
        reasons("as \\citep{a} and \\citet[p.~3]{b} say \\citeauthor{a}").is_empty(),
        "{:?}",
        reasons("as \\citep{a} and \\citet[p.~3]{b} say \\citeauthor{a}")
    );
    assert!(reasons(
        "\\fbox{\\parbox{3cm}{x}} \\colorbox{yellow}{y} \\underline{z} \\rule{1cm}{1pt}"
    )
    .is_empty());
    assert!(reasons("\\begin{enumerate}[label=(\\alph*)]\\item a\\end{enumerate}").is_empty());
    let r = reasons("\\begin{table}\\setlength{\\tabcolsep}{2pt}\\renewcommand{\\arraystretch}{1.2}\\begin{tabular}{l}a\\end{tabular}\\end{table}");
    assert!(r.is_empty(), "{r:?}");
    assert!(reasons("\\setlength{\\parindent}{0pt} text")
        .contains(&Reason::SizeDeclarationOutsideGroup("setlength".into())));
}
#[test]
fn balance() {
    assert!(reasons("Unbalanced { brace").contains(&Reason::UnbalancedBraces));
    assert!(reasons("Unbalanced } brace").contains(&Reason::UnbalancedBraces));
    assert!(reasons("Unbalanced $ math").contains(&Reason::UnbalancedMath));
    assert!(reasons("$a { b $ c }").contains(&Reason::UnbalancedMath));
    assert!(ok("Escaped \\{ \\} \\$ are fine."));
    assert!(reasons("\\begin{itemize}\\item x").contains(&Reason::UnbalancedEnvironment));
}
#[test]
fn math_vocabulary() {
    assert!(reasons("$\\mycmd{x}$").contains(&Reason::DisallowedMathMacro("mycmd".into())));
    assert!(ok("$\\sum_{i=1}^{n} \\mathbf{v}_i \\cdot \\hat{w}$"));
    assert!(ok("$\\text{for all } x$ and $\\mbox{the \\emph{text}} y$"));
}
#[test]
fn display_math_and_refs() {
    let (shape, r) = classify_source("Text before\n\\[ x = y \\]\ntext after with \\eqref{eq:a} and\n\\begin{equation} a = b \\label{eq:a} \\end{equation}\nand \\begin{align} a &= b \\\\ c &= d \\end{align} done.", &pol());
    assert_eq!(shape, UnitShape::Par);
    assert!(r.is_empty(), "{r:?}");
    assert!(ok(
        "See Section~\\ref{sec:x} on page~\\pageref{sec:x} and \\cite{knuth}."
    ));
    let mut p = pol();
    p.amsmath = false;
    assert!(check_source("\\begin{align} a &= b \\end{align}", &p)
        .contains(&Reason::NeedsPackage("amsmath".into())));
    p.cite_ok = false;
    assert!(check_source("See \\cite{k}.", &p).contains(&Reason::DisallowedMacro("cite".into())));
}
#[test]
fn environments() {
    let (shape, r) = classify_source(
        "\\begin{itemize}\n\\item First\n\\item Second with $x$\n\\end{itemize}",
        &pol(),
    );
    assert_eq!(shape, UnitShape::Env("itemize".into()));
    assert!(r.is_empty(), "{r:?}");
    let (shape, r) = classify_source("\\begin{figure}[tbp]\\centering\\includegraphics[width=0.5\\textwidth]{figure.png}\\caption{Figure 1.}\\label{fig:1}\\end{figure}", &pol());
    assert_eq!(shape, UnitShape::Env("figure".into()));
    assert!(r.is_empty(), "{r:?}");
    let (shape, r) = classify_source("\\begin{table}[h]\\centering\\begin{tabular}{lr}\\toprule a & b \\\\ \\midrule 1 & 2 \\\\ \\bottomrule\\end{tabular}\\caption{T}\\end{table}", &pol());
    assert_eq!(shape, UnitShape::Env("table".into()));
    assert!(r.is_empty(), "{r:?}");
    assert!(reasons("\\item outside").contains(&Reason::MacroOutsideContext("item".into())));
    assert!(reasons("\\caption{x}").contains(&Reason::MacroOutsideContext("caption".into())));
    // a paragraph ending in a list is one unit; text after the list is not
    assert!(ok("Intro text:\n\\begin{itemize}\\item a\\end{itemize}"));
    assert!(
        reasons("Intro:\n\\begin{itemize}\\item a\\end{itemize}\nmore text")
            .contains(&Reason::TextAfterEnvironment)
    );
    // blank lines inside a block environment are fine
    assert!(ok("\\begin{quote}\nfirst\n\nsecond\n\\end{quote}"));
    let mut p = pol();
    p.theorem_envs.insert("theorem".into());
    assert_eq!(
        classify_source("\\begin{theorem}\\label{t}Let $x$.\\end{theorem}", &p).0,
        UnitShape::Env("theorem".into())
    );
}
#[test]
fn headings() {
    let (shape, r) = classify_source("\\section{A \\emph{title}}\\label{sec:a}", &pol());
    assert_eq!(shape, UnitShape::Heading("section".into()));
    assert!(r.is_empty(), "{r:?}");
    assert!(reasons("Text \\section{x}").contains(&Reason::DisallowedMacro("section".into())));
}
#[test]
fn counters_and_package_macros() {
    let p = pol();
    assert!(check_source(
        "Text \\stepcounter{foo} and \\setcounter{enumi}{3} \\addtocounter{x}{-1}.",
        &p
    )
    .is_empty());
    // before the first text the command runs before the unit's counters are captured
    assert!(check_source("\\stepcounter{equation}\nText with $x$.", &p)
        .contains(&Reason::LeadingCounter("stepcounter".into())));
    // a lone counter line is a unit of its own (no rows): fine
    assert!(check_source("\\setcounter{section}{3}", &p).is_empty());
    // pol() has hyperref loaded but not caption
    assert!(
        reasons("\\captionof{figure}{A caption}").contains(&Reason::NeedsPackage("caption".into()))
    );
    assert!(classify_source("\\section{\\texorpdfstring{$x$}{x}}", &p)
        .1
        .is_empty());
    let p0 = Policy::from_preamble("\\documentclass{article}\n", &[], &[]);
    assert!(check_source("\\section{\\texorpdfstring{$x$}{x}}", &p0)
        .contains(&Reason::NeedsPackage("hyperref".into())));
    let p2 = Policy::from_preamble("\\usepackage{caption}\\usepackage{hyperref}\n", &[], &[]);
    assert!(check_source("\\captionof{figure}{A caption}", &p2).is_empty());
    assert!(classify_source("\\section{\\texorpdfstring{$x$}{x}}", &p2)
        .1
        .is_empty());
}
#[test]
fn user_environments() {
    let p = Policy::from_preamble(
        "\\newenvironment{solution}{\\par\\noindent\\textbf{Solution.}\\ \\itshape}{\\par}\n\\newenvironment{hint}[1]{\\begin{quote}\\small\\textbf{#1:}\\ }{\\end{quote}}\n\\newenvironment{bad}{\\tikz{x}}{}\n\\renewenvironment{abstract}{\\small}{\\par}\n",
        &[],
        &[],
    );
    assert!(
        p.user_inner_envs.contains("solution") && p.user_inner_envs.contains("abstract"),
        "inner {:?} block {:?}",
        p.user_inner_envs,
        p.theorem_envs
    );
    assert!(p.theorem_envs.contains("hint"));
    assert!(!p.user_inner_envs.contains("bad") && !p.theorem_envs.contains("bad"));
    let (shape, r) = classify_source("\\begin{solution}\nText.\n\\end{solution}", &p);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(shape, UnitShape::Par);
    let (shape, r) = classify_source("\\begin{hint}{Idea}\nText.\n\\end{hint}", &p);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(shape, UnitShape::Env("hint".into()));
    assert!(!check_source("\\begin{bad}x\\end{bad}", &p).is_empty());
}
#[test]
fn pictures_and_setup_prefix() {
    let p = pol();
    // an inline picture does not end the paragraph unit
    let (shape, r) = classify_source(
        "Text \\begin{tikzpicture}\\draw (0,0)--(1,0);\\end{tikzpicture} goes on.",
        &p,
    );
    assert_eq!(shape, UnitShape::Par);
    assert!(!r.contains(&Reason::TextAfterEnvironment), "{r:?}");
    assert!(r.contains(&Reason::DisallowedEnvironment("tikzpicture".into())));
    // a block picture followed by text still is
    let (_, r) = classify_source(
        "\\begin{tikzpicture}\\draw (0,0)--(1,0);\\end{tikzpicture}\nText after.",
        &p,
    );
    assert!(r.contains(&Reason::TextAfterEnvironment), "{r:?}");
    // the body of a picture is the environment's business: one reason for the picture,
    // none for its drawing commands (a cached picture is then the unit's only unknown)
    let (shape, r) = classify_source(
        "\\begin{center}\n\\begin{tikzpicture}\n\\begin{axis}[xlabel=$u$]\n\\addplot[blue] {x};\n\\end{axis}\n\\end{tikzpicture}\n\\end{center}",
        &p,
    );
    assert_eq!(shape, UnitShape::Env("center".into()));
    assert_eq!(r, vec![Reason::DisallowedEnvironment("tikzpicture".into())]);
    let (_, r) = classify_source(
        "A plot: \\begin{tikzpicture}\\foo{x}\\end{tikzpicture} and \\bar.",
        &p,
    );
    assert_eq!(
        r,
        vec![
            Reason::DisallowedEnvironment("tikzpicture".into()),
            Reason::DisallowedMacro("bar".into())
        ]
    );
    // an unclosed picture
    assert!(reasons("\\begin{tikzpicture}\\draw;").contains(&Reason::UnbalancedEnvironment));
    assert_eq!(
        skip_env_body("a\\begin{x}b\\end{x}c\\end{x}d", 0, "x"),
        Some(26)
    );
    assert_eq!(skip_env_body("a\\end{x}", 0, "x"), Some(8));
    assert_eq!(skip_env_body("a\\begin{x}", 0, "x"), None);
    // setup statements before a block environment: the unit's shape is the environment
    let (shape, r) = classify_source("\\def\\R{1.2}\n\\begin{center}\nx\n\\end{center}", &p);
    assert_eq!(shape, UnitShape::Env("center".into()));
    assert!(!r.contains(&Reason::SetupStatement), "{r:?}");
    assert_eq!(setup_prefix("\\def\\R{1.2}\n\\begin{center}"), (12, true));
    assert_eq!(setup_prefix("Text \\def\\R{1}"), (0, false));
    assert_eq!(setup_prefix("\\def\\R{1.2"), (0, false));
    assert!(setup_statements("\\def\\R{1.2}\n\\setlength\\parindent{0pt}").is_some());
    assert!(setup_statements("\\def\\R{1.2}\nText").is_none());
}
#[test]
fn title_block_and_symbols() {
    let p = pol();
    let (shape, r) = classify_source("\\title{A Report}\n\\author{Henry Yang \\and A. Reader\\thanks{Supported.}}\n\\date{\\today}\n\\maketitle", &p);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(shape, UnitShape::Env("center".into()));
    assert!(ok("\\S~3, \\P, \\dag, \\pounds 5, {\\fontsize{10}{12}\\selectfont x}, {\\frenchspacing y.} \\protect\\cite{k}"));
    assert!(reasons("\\frenchspacing x")
        .contains(&Reason::SizeDeclarationOutsideGroup("frenchspacing".into())));
    let p2 = Policy::from_preamble(
        "\\usepackage{xspace,csquotes,mhchem}\n\\newcommand{\\eg}{e.g.\\xspace}\n",
        &[],
        &[],
    );
    assert!(p2.trusted_macros.contains("eg"));
    assert!(check_source("\\enquote{quoted} and \\ce{H2O + CO2} \\eg", &p2).is_empty());
    assert!(!reasons("\\enquote{x}").is_empty());
}
#[test]
fn permissive_keeps_structural_reasons() {
    let mut p = pol();
    p.permissive = true;
    assert!(check_source("Text with \\scalebox{2}{x} and \\foo.", &p).is_empty());
    assert!(check_source("Text {unbalanced", &p).contains(&Reason::UnbalancedBraces));
    assert!(check_source("\\begin{center}x\\end{center} more text", &p)
        .contains(&Reason::TextAfterEnvironment));
    let (_, strict) = classify_source_with("\\foo{x}", &p, false);
    assert!(strict.iter().all(is_vocabulary_reason) && !strict.is_empty());
}
#[test]
fn heading_macros_and_old_font_switches() {
    let p = Policy::from_preamble(
        "\\usepackage{hyperref,graphicx}\n\\newcommand{\\unnumberedsection}[1]{\\section*{#1}\\phantomsection\\addcontentsline{toc}{section}{#1}\\markboth{\\MakeUppercase{#1}}{}}\n",
        &[],
        &[],
    );
    assert_eq!(
        p.heading_macros
            .get("unnumberedsection")
            .map(String::as_str),
        Some("section")
    );
    let (shape, r) = classify_source(
        "\\newpage\n\\unnumberedsection{\\texorpdfstring{0\\quad Premises}{0 Premises}}",
        &p,
    );
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(shape, UnitShape::Heading("section".into()));
    let (shape, r) = classify_source("\\newpage\n\\subsection{After a page break}", &p);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(shape, UnitShape::Heading("subsection".into()));
    // enumitem's resume continues numbering kept outside the counters
    let r = check_source(
        "Then:\n\\begin{enumerate}[resume]\n  \\item Third\n\\end{enumerate}",
        &p,
    );
    assert!(
        r.contains(&Reason::OutsideState("\\begin{enumerate}[resume]".into())),
        "{r:?}"
    );
    // ... matched as a key of a list's options, not as text in other environments' titles
    let r = check_source(
        "\\begin{enumerate}[label={[\\arabic*]}, resume]\n  \\item x\n\\end{enumerate}",
        &p,
    );
    assert!(
        r.iter().any(|x| matches!(x, Reason::OutsideState(_))),
        "{r:?}"
    );
    let r = check_source(
        "\\begin{enumerate}[label=\\roman*]\n  \\item A series of items\n\\end{enumerate}",
        &p,
    );
    assert!(
        !r.iter().any(|x| matches!(x, Reason::OutsideState(_))),
        "{r:?}"
    );
    let r = check_source(
        "\\begin{quote}[Convergence of the series]\nText.\n\\end{quote}",
        &p,
    );
    assert!(
        !r.iter().any(|x| matches!(x, Reason::OutsideState(_))),
        "{r:?}"
    );
    let mut pl = p.clone();
    pl.list_envs.insert("steps".into());
    let r = check_source("\\begin{steps}[resume*]\n  \\item x\n\\end{steps}", &pl);
    assert!(
        r.iter().any(|x| matches!(x, Reason::OutsideState(_))),
        "{r:?}"
    );
    assert_eq!(optional_arg(" [a={[x]}, b] tail"), Some("a={[x]}, b"));
    // run-in headings share the following paragraph's lines: alone they wait, with it
    // they go live
    let (shape, r) = classify_source("\\paragraph{Run-in.} With text.", &p);
    assert_eq!(shape, UnitShape::Heading("paragraph".into()));
    assert!(r.is_empty(), "{r:?}");
    let (_, r) = classify_source_with("\\subparagraph{Deeper.}\n", &p, true);
    assert_eq!(r, vec![Reason::RunInHeading("subparagraph".into())]);
    let r = check_source(
        "Text {\\rm roman} and $\\rm x \\uparrow \\iint$ and \\scalebox{2}{big}",
        &p,
    );
    assert!(r.is_empty(), "{r:?}");
    assert!(check_source("\\rm leaks out", &p)
        .contains(&Reason::SizeDeclarationOutsideGroup("rm".into())));
}
#[test]
fn setup_spans() {
    assert!(setup_statements(
        "\\newcommand{\\kw}[1]{\\textbf{#1}}\n\\renewcommand{\\arraystretch}{1.3}"
    )
    .is_some());
    assert!(setup_statements(
        "\\setlength\\parindent{0pt}\\def\\foo#1{x #1}\\let\\a=\\b\n\\pagestyle{empty}"
    )
    .is_some());
    assert!(setup_statements("\\newcommand{\\kw}{x} Text after it").is_none());
    assert!(setup_statements("Text \\newcommand{\\kw}{x}").is_none());
    assert!(setup_statements("\\setcounter{page}{3}").is_none());
    assert!(setup_statements("\\newcommand{\\kw}{x").is_none());
    let p = pol();
    assert_eq!(
        check_source("\\newcommand{\\kw}[1]{\\textbf{#1}}", &p),
        vec![Reason::SetupStatement]
    );
}
#[test]
fn trusted() {
    let mut p = pol();
    p.trusted_macros.insert("mymacro".into());
    assert!(check_source("\\mymacro{x}", &p).is_empty());
}
#[test]
fn comments_stripped() {
    assert!(ok("Text % \\mymacro{x} in a comment\nmore text"));
    assert!(!ok("Text \\% \\mymacro{x} not a comment"));
}
#[test]
fn preamble_policy() {
    let p = Policy::from_preamble("\\documentclass{book}\n\\usepackage{amsmath,graphicx}\n\\usepackage[backend=biber]{biblatex}\n\\newtheorem{theorem}{Theorem}\n% \\usepackage{booktabs}\n", &[], &[]);
    assert!(p.amsmath && p.graphicx && !p.booktabs && p.cite_ok);
    // user macros: trusted when their bodies are allow-listed, in the mode they classify in
    let p2 = Policy::from_preamble("\\usepackage{amsmath,amssymb}\n\\usepackage[table]{xcolor}\n\\newcommand{\\R}{\\mathbb{R}}\n\\newcommand{\\abs}[1]{\\left\\lvert #1 \\right\\rvert}\n\\newcommand{\\code}[1]{\\texttt{#1}}\n\\newcommand{\\bad}{\\tikz{x}}\n\\DeclareMathOperator*{\\argmax}{arg\\,max}\n\\newcommand{\\absR}[1]{\\abs{#1}\\in\\R}\n", &[], &[]);
    assert!(
        p2.trusted_math.contains("R")
            && p2.trusted_math.contains("abs")
            && p2.trusted_math.contains("argmax")
            && p2.trusted_math.contains("absR"),
        "math {:?} text {:?}",
        p2.trusted_math,
        p2.trusted_macros
    );
    assert!(
        p2.trusted_macros.contains("code")
            && !p2.trusted_macros.contains("bad")
            && !p2.trusted_math.contains("bad")
    );
    assert!(p2.has_package("colortbl"));
    assert!(classify_source("$\\abs{x} \\in \\R$ and \\code{y}", &p2)
        .1
        .is_empty());
    assert!(p.theorem_envs.contains("theorem"));
}
#[test]
fn everypar_patterns() {
    assert!(everypar_allowed(""));
    assert!(everypar_allowed("\\leftprotrusion "));
    assert!(everypar_allowed(
        "{\\setbox \\z@ \\lastbox }\\everypar {}\\@endpefalse "
    ));
    assert!(!everypar_allowed("\\@itemlabel"));
    let mut flags = std::collections::BTreeMap::new();
    flags.insert("ins".to_string(), 1);
    assert!(check_engine_unit("par", "", true, 3, &flags, None).is_empty());
    flags.insert("dir".to_string(), 1);
    assert_eq!(
        check_engine_unit("par", "", true, 3, &flags, None),
        vec![Reason::EngineFlag("dir=1".into())]
    );
}
#[test]
fn ieeetran_lists() {
    let list = "\\begin{IEEEdescription}[\\IEEEusemathlabelsep\\IEEEsetlabelwidth{$\\alpha,\\beta$}]\n\
                \\item[$\\alpha$] First symbol.\n\\item[$\\beta$] Second symbol.\n\\end{IEEEdescription}";
    let keywords = "\\begin{IEEEkeywords}\nReliability, topology.\n\\end{IEEEkeywords}";
    let ieee = Policy::from_preamble("\\documentclass[journal]{IEEEtran}\n", &[], &[]);
    for (src, env) in [(list, "IEEEdescription"), (keywords, "IEEEkeywords")] {
        let (shape, r) = classify_source(src, &ieee);
        assert_eq!(shape, UnitShape::Env(env.into()));
        assert!(r.is_empty(), "{env}: {r:?}");
    }
    // the label setup commands are the class's own
    let article = Policy::from_preamble("\\documentclass{article}\n", &[], &[]);
    assert_eq!(
        classify_source(list, &article).1,
        vec![Reason::NeedsPackage("IEEEtran".into())]
    );
}
#[test]
fn run_in_headings_from_the_capture() {
    let p = pol();
    let flags = BTreeMap::new();
    // a heading the capture saw set run-in, whatever its name (IEEEtran's \subsubsection)
    assert_eq!(
        check_engine_unit("heading", "", true, 2, &flags, Some("subsubsection")),
        vec![Reason::RunInHeading("subsubsection".into())]
    );
    assert!(check_engine_unit("heading", "", true, 2, &flags, None).is_empty());
    // each heading says so once, whether the source, the capture or both know it is run-in
    for (src, run_in, name) in [
        (
            "\\subsubsection{Paths}",
            Some("subsubsection"),
            "subsubsection",
        ),
        ("\\paragraph{Paths.}", Some("paragraph"), "paragraph"),
        ("\\paragraph{Paths.}", None, "paragraph"),
        ("\\paragraph{Paths.}\n\\label{p}\n", None, "paragraph"),
    ] {
        let mut r = classify_source(src, &p).1;
        r.extend(check_engine_unit("heading", "", true, 2, &flags, run_in));
        assert_eq!(
            r,
            vec![Reason::RunInHeading(name.into())],
            "{src} {run_in:?}"
        );
    } // with the paragraph or list that sets its title, it goes live like any heading, a blank
      // line after the heading and its labels included
    for src in [
        "\\paragraph{Paths.} Text.",
        "\\paragraph*[P]{Paths \\{x\\}.}\n\\label{p} % c\n\nText.",
        "\\paragraph{Paths.}\n\\begin{itemize}\n\\item a\n\\end{itemize}",
    ] {
        assert!(runs_into_title(src), "{src}");
        let r = classify_source(src, &p).1;
        assert!(r.is_empty(), "{src} {r:?}");
    }
    assert!(!runs_into_title("\\paragraph{Paths. % }\n}\n\\label{x}"));
    assert!(!runs_into_title("\\paragraph{Unclosed"));
    assert_eq!(
        classify_source("\\paragraph{A}\nText.\n\nMore.", &p).1,
        vec![Reason::ParagraphBreak]
    );
    // the segmentation rules: the standard run-in headings, and IEEEtran's \subsubsection
    let ieee = Policy::from_preamble("\\documentclass[journal]{IEEEtran}\n", &[], &[]);
    assert_eq!(
        ieee.segment_rules().run_in,
        ["paragraph", "subparagraph", "subsubsection"]
    );
    assert_eq!(p.segment_rules().run_in, ["paragraph", "subparagraph"]);
}
