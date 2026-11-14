use super::inputs::*;
use super::lex::*;
use super::segment::*;
use super::*;

#[test]
fn input_scanning_and_expansion() {
    let text = "\\input{chapters/one}\n% \\input{nope}\n\\include{chapters/two.tex}\n\\input three\n\\input{../x}\n\\input{\\jobname}\n";
    assert_eq!(
        input_targets(text),
        vec!["chapters/one.tex", "chapters/two.tex", "three.tex"]
    );
    let mut files = std::collections::BTreeMap::new();
    files.insert("a.tex".to_string(), "A\\input{b}".to_string());
    files.insert("b.tex".to_string(), "B".to_string());
    assert_eq!(
        expand_inputs("x \\input{a} y \\input{c}", &files),
        "x AB\n\n y \\input{c}"
    );
    assert_eq!(
        transitive_inputs("\\input{a}", &files)
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["a.tex", "b.tex"]
    );
    let mut ids = IdAllocator(0);
    let fb = FileBuf::new("First.\n\nLast one.\n", &mut ids, 1);
    let last = fb.spans.last().unwrap();
    assert_eq!(fast_source(&fb, last, false), "Last one.");
    assert_eq!(fast_source(&fb, last, true), "Last one.\n{}");
    assert_eq!(fast_source(&fb, &fb.spans[0], true), "First.");
    let fb2 = FileBuf::new("Only.\n\n", &mut ids, 1);
    assert_eq!(fast_source(&fb2, &fb2.spans[0], true), "Only.");
    // a block environment ending the file keeps its `\end` last, with or without the final
    // newline, theorem-like environments included
    let fig = "Text.\n\n\\begin{figure}\nx\n\\end{figure} % done\n";
    for text in [fig, fig.trim_end()] {
        let fb = FileBuf::new(text, &mut ids, 1);
        let env = fb.spans.last().unwrap();
        assert_eq!(env.kind, SpanKind::Env);
        assert!(fast_source(&fb, env, true).ends_with("\\end{figure} % done"));
    }
    let thm = "\\begin{theorem}\nx\n\\end{theorem}\n";
    let fb = FileBuf::with_rules(
        thm,
        &mut ids,
        1,
        SegmentRules {
            block_envs: vec!["theorem".into()],
            ..Default::default()
        },
    );
    assert_eq!(fast_source(&fb, &fb.spans[0], true), thm.trim_end());
    // a paragraph with display math is an Env span too, and keeps its trailing space
    let fb = FileBuf::new(
        "We have\n\\begin{equation}\nx\n\\end{equation}\nwhere x is one.\n",
        &mut ids,
        1,
    );
    assert_eq!(fb.spans[0].kind, SpanKind::Env);
    assert!(fast_source(&fb, &fb.spans[0], true).ends_with("one.\n{}"));
}

#[test]
fn body_setup_statements_come_from_files_the_body_reads() {
    // defs.tex is read in the preamble only (the server preamble inlines it already),
    // chap.tex in the body
    let mut texts = std::collections::BTreeMap::new();
    texts.insert(
        "main.tex".to_string(),
        "\\documentclass{article}\n\\input{defs}\n\\begin{document}\n\\input{chap}\n\\end{document}\n"
            .to_string(),
    );
    texts.insert(
        "defs.tex".to_string(),
        "\\newcommand{\\pre}{P}\n".to_string(),
    );
    texts.insert(
        "chap.tex".to_string(),
        "\\newcommand{\\kw}{K}\n\nText \\kw.\n".to_string(),
    );
    assert_eq!(
        body_setup_statements(&texts, "main.tex"),
        vec!["\\newcommand{\\kw}{K}"]
    );
}
const DOC: &str = "\\documentclass{book}\n\\usepackage{microtype}\n\\begin{document}\n\\chapter{One}\n\nFirst paragraph\nspanning two lines.\n\nSecond paragraph.\n\n\\begin{itemize}\n\\item a\n\n\\item b\n\\end{itemize}\n\nThird.\n\\end{document}\n";

#[test]
fn document_markers_in_verbatim_comments_and_verb_are_text() {
    // a chapter that shows a document in verbatim has no preamble and no trailer
    let chapter = "\\section{Code}\nInline \\verb|\\end{document}| here.\n\n\\begin{verbatim}\n\\documentclass{article}\n\\begin{document}\nx\n\\end{document}\n\\end{verbatim}\nAfter.\n% \\begin{document} in a comment\n";
    let kinds: Vec<SpanKind> = segment(chapter, &SegmentRules::default())
        .into_iter()
        .map(|(_, k)| k)
        .collect();
    assert!(
        !kinds.contains(&SpanKind::Preamble) && !kinds.contains(&SpanKind::Trailer),
        "{kinds:?}"
    );
    // the real markers still split a main file
    let main = "\\documentclass{article}\n% \\begin{document} (comment)\n\\begin{document}\nText \\verb|\\end{document}|.\n\\end{document}\nafter\n";
    let spans = segment(main, &SegmentRules::default());
    assert_eq!(spans[0].1, SpanKind::Preamble);
    assert_eq!(
        &main[spans[0].0.clone()],
        "\\documentclass{article}\n% \\begin{document} (comment)\n\\begin{document}\n"
    );
    assert_eq!(spans.last().unwrap().1, SpanKind::Trailer);
    assert!(main[spans.last().unwrap().0.clone()].starts_with("\\end{document}"));
    assert_eq!(blank_verb("a \\verb*+%x+ b"), "a \\verb*     b");
    // a marker inside a verbatim body opened on the same line, or after a verbatim
    // environment closed on it
    assert_eq!(
        find_command("\\begin{verbatim}\\begin{document}\n", "\\begin{document}"),
        None
    );
    assert_eq!(
        find_command(
            "x \\begin{lstlisting}[a] \\end{document}\n",
            "\\end{document}"
        ),
        None
    );
    let two =
        "\\begin{verbatim}a\\end{verbatim} \\begin{comment}\n\\begin{document}\n\\end{comment}\n";
    assert_eq!(find_command(two, "\\begin{document}"), None);
    let after = "\\begin{verbatim}%x\\end{verbatim}\\begin{document}\n";
    assert_eq!(
        find_command(after, "\\begin{document}"),
        Some(after.find("\\begin{document}").unwrap())
    );
    assert_eq!(blank_verb("\\verbatim"), "\\verbatim");
}

#[test]
fn segments_kinds() {
    let mut ids = IdAllocator(0);
    let fb = FileBuf::new(DOC, &mut ids, 1);
    let kinds: Vec<SpanKind> = fb.spans.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        vec![
            SpanKind::Preamble,
            SpanKind::Heading,
            SpanKind::Body,
            SpanKind::Body,
            SpanKind::Env,
            SpanKind::Body,
            SpanKind::Trailer
        ]
    );
    assert_eq!(
        fb.span_text(fb.spans[2].id).unwrap(),
        "First paragraph\nspanning two lines.\n"
    );
    assert_eq!(fb.line_range(&fb.spans[2]), (6, 7));
}

#[test]
fn crlf_files_segment_like_lf_files() {
    let mut ids = IdAllocator(0);
    let lf = FileBuf::new(DOC, &mut ids, 1);
    let crlf = FileBuf::new(&DOC.replace('\n', "\r\n"), &mut ids, 1);
    let shape = |fb: &FileBuf| -> Vec<(SpanKind, (i64, i64))> {
        fb.spans
            .iter()
            .map(|s| (s.kind, fb.line_range(s)))
            .collect()
    };
    assert_eq!(shape(&lf), shape(&crlf));
}

#[test]
fn diff_edit_is_minimal_and_on_char_boundaries() {
    assert!(diff_edit("abc", "abc").is_none());
    for (old, new, range, text) in [
        ("abc", "abXc", 2..2, "X"),
        ("abc", "ac", 1..2, ""),
        ("aaa", "aaaa", 3..3, "a"),
        ("aé", "aè", 1..3, "è"),
        ("éa", "èa", 0..2, "è"),
        ("", "x", 0..0, "x"),
        ("x", "", 0..1, ""),
    ] {
        let e = diff_edit(old, new).unwrap();
        assert_eq!((e.start_byte..e.end_byte, e.text.as_str()), (range, text));
        let mut s = old.to_string();
        s.replace_range(e.start_byte..e.end_byte, &e.text);
        assert_eq!(s, new);
    }
}

#[test]
fn edit_inside_paragraph_keeps_ids() {
    let mut ids = IdAllocator(0);
    let mut fb = FileBuf::new(DOC, &mut ids, 1);
    let before: Vec<ParaId> = fb.spans.iter().map(|s| s.id).collect();
    let pos = fb.text.find("spanning").unwrap();
    let out = fb.apply(
        &Edit {
            start_byte: pos,
            end_byte: pos,
            text: "now ".into(),
        },
        &mut ids,
        2,
    );
    let after: Vec<ParaId> = fb.spans.iter().map(|s| s.id).collect();
    assert_eq!(before, after);
    assert_eq!(out.touched, vec![before[2]]);
    assert!(out.added.is_empty() && out.removed.is_empty() && !out.preamble_changed);
    assert_eq!(fb.spans[2].last_revision, 2);
    assert_eq!(fb.spans[3].last_revision, 1);
    assert!(fb.span_text(before[2]).unwrap().contains("now spanning"));
}

#[test]
fn split_and_merge() {
    let mut ids = IdAllocator(0);
    let mut fb = FileBuf::new(DOC, &mut ids, 1);
    let n = fb.spans.len();
    let pos = fb.text.find("spanning").unwrap();
    let before: Vec<ParaId> = fb.spans.iter().map(|s| s.id).collect();
    let out = fb.apply(
        &Edit {
            start_byte: pos,
            end_byte: pos,
            text: "\n\n".into(),
        },
        &mut ids,
        2,
    );
    assert_eq!(fb.spans.len(), n + 1);
    // the first half keeps the paragraph's id, the second half is new
    assert_eq!(out.touched, vec![before[2]]);
    assert!(out.removed.is_empty());
    assert_eq!(out.added.len(), 1);
    assert_eq!(fb.spans[2].id, before[2]);
    assert_eq!(fb.spans[3].id, out.added[0]);
    // merge back by deleting the inserted blank line: the first id survives, the second goes
    let out2 = fb.apply(
        &Edit {
            start_byte: pos,
            end_byte: pos + 2,
            text: String::new(),
        },
        &mut ids,
        3,
    );
    assert_eq!(fb.spans.len(), n);
    assert_eq!(out2.touched, vec![before[2]]);
    assert_eq!(out2.removed, vec![out.added[0]]);
    assert!(out2.added.is_empty());
}

#[test]
fn windowed_resegmentation_matches_full_pass() {
    let mut body = String::new();
    for i in 0..3000 {
        if i % 7 == 0 {
            body.push_str(&format!("\\section{{S{i}}}\n\n"));
        }
        if i % 11 == 0 {
            body.push_str("\\begin{itemize}\n\\item a\n\n\\item b\n\\end{itemize}\n\n");
        }
        body.push_str(&format!(
            "Paragraph {i} with some words\nand a second line.\n\n"
        ));
    }
    let doc = format!("\\documentclass{{book}}\n\\begin{{document}}\n{body}\\end{{document}}\n");
    let mut ids = IdAllocator(0);
    let mut fb = FileBuf::new(&doc, &mut ids, 1);
    let n = fb.spans.len();
    // many kinds of edits: insert, delete across a boundary, split, merge, inside env
    let probes = [" x", "\n\n", "", "\\emph{y}"];
    let mut t_total = std::time::Duration::ZERO;
    let mut times: Vec<std::time::Duration> = Vec::new();
    // probe positions inside paragraph text (never inside \begin/\end lines, which would turn
    // the rest of the document into one unclosed environment and dominate the timing)
    let anchors: Vec<usize> = fb
        .text
        .match_indices("with some words")
        .map(|(i, _)| i + 5)
        .collect();
    for k in 0..200 {
        let pos = anchors[(k * 7919 + 13) % anchors.len()] + (k % 3);
        let text = probes[k % probes.len()].to_string();
        let del = if k % 5 == 0 { 3 } else { 0 };
        let t0 = std::time::Instant::now();
        fb.apply(
            &Edit {
                start_byte: pos,
                end_byte: pos + del,
                text,
            },
            &mut ids,
            2 + k as u64,
        );
        let dt = t0.elapsed();
        t_total += dt;
        times.push(dt);
        assert_eq!(
            fb.line_starts,
            compute_line_starts(&fb.text),
            "line starts after edit {k}"
        );
        let full = segment(&fb.text, &fb.rules);
        let got: Vec<(Range<usize>, SpanKind)> =
            fb.spans.iter().map(|s| (s.range.clone(), s.kind)).collect();
        if got != full {
            let i = got
                .iter()
                .zip(full.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(full.len()));
            let lo = i.saturating_sub(2);
            panic!("edit {k} at {pos} (del {del}, text {:?}): first mismatch at span {i}\n got: {:?}\nfull: {:?}\ntext around: {:?}",
                probes[k % probes.len()], &got[lo..(i + 3).min(got.len())], &full[lo..(i + 3).min(full.len())],
                &fb.text[pos.saturating_sub(60)..(pos + 60).min(fb.text.len())]);
        }
    }
    times.sort();
    eprintln!("200 edits on a {}-byte / {}-span document: median {:?}, mean {:?} per edit (the mean includes edits after a probe broke an \\end{{itemize}}, which makes the rest of the document one span)", fb.text.len(), n, times[100], t_total / 200);
    // the timing bound is a release-build property (debug builds are ~10× slower and also run
    // the debug_assert that recomputes every line start); correctness is checked above either way
    if !cfg!(debug_assertions) {
        assert!(
            times[100] < std::time::Duration::from_micros(250),
            "median per-edit cost {:?}",
            times[100]
        );
    }
}

#[test]
fn heading_and_environment_splits() {
    let doc = "\\documentclass{book}\n\\begin{document}\n\\section{Title\nover two lines}\nFirst paragraph after the heading.\nSecond line.\n\nIntro:\n\\begin{itemize}\n\\item a\n\\end{itemize}\nText after the list.\n\nMath \\begin{equation} x \\end{equation} stays\ntogether.\n\\end{document}\n";
    let mut ids = IdAllocator(0);
    let fb = FileBuf::new(doc, &mut ids, 1);
    let texts: Vec<(&str, SpanKind)> = fb
        .spans
        .iter()
        .map(|s| (fb.text[s.range.clone()].trim_end(), s.kind))
        .collect();
    assert_eq!(
        texts[1],
        ("\\section{Title\nover two lines}", SpanKind::Heading)
    );
    assert_eq!(
        texts[2],
        (
            "First paragraph after the heading.\nSecond line.",
            SpanKind::Body
        )
    );
    assert_eq!(
        texts[3],
        (
            "Intro:\n\\begin{itemize}\n\\item a\n\\end{itemize}",
            SpanKind::Env
        )
    );
    assert_eq!(texts[4], ("Text after the list.", SpanKind::Body));
    assert_eq!(
        texts[5],
        (
            "Math \\begin{equation} x \\end{equation} stays\ntogether.",
            SpanKind::Env
        )
    );
    assert_eq!(fb.spans.len(), 7);
}

#[test]
fn run_in_heading_keeps_the_paragraph_that_sets_its_title() {
    let body = "\\paragraph{Same line.} Text.\n\n\\paragraph{Apart.}\n\\label{a} % note\n\nTitle text\nhere.\n\n\
                Next.\n\n\\paragraph*{List.}\n\\begin{itemize}\n\\item a\n\\end{itemize}\nAfter.\n\n\
                \\paragraph{Alone.}\n\\section{S}\nText.\n\\paragraph{Ends}\nhere\n\\subparagraph{X}y\n";
    let doc = format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\\end{{document}}\n");
    let rules = SegmentRules {
        run_in: vec!["paragraph".into(), "subparagraph".into()],
        ..Default::default()
    };
    let mut ids = IdAllocator(0);
    let fb = FileBuf::with_rules(&doc, &mut ids, 1, rules.clone());
    let texts: Vec<(&str, SpanKind)> = fb
        .spans
        .iter()
        .map(|s| (fb.text[s.range.clone()].trim_end(), s.kind))
        .collect();
    assert_eq!(
        &texts[1..texts.len() - 1],
        [
            ("\\paragraph{Same line.} Text.", SpanKind::Heading),
            (
                "\\paragraph{Apart.}\n\\label{a} % note\n\nTitle text\nhere.",
                SpanKind::Heading
            ),
            ("Next.", SpanKind::Body),
            (
                "\\paragraph*{List.}\n\\begin{itemize}\n\\item a\n\\end{itemize}",
                SpanKind::Heading
            ),
            ("After.", SpanKind::Body),
            ("\\paragraph{Alone.}", SpanKind::Heading),
            ("\\section{S}", SpanKind::Heading),
            ("Text.", SpanKind::Body),
            ("\\paragraph{Ends}\nhere", SpanKind::Heading),
            ("\\subparagraph{X}y", SpanKind::Heading),
        ]
    );
    // a display heading's paragraph stays a span of its own
    let plain = FileBuf::new(&doc, &mut ids, 1);
    assert!(plain
        .spans
        .iter()
        .any(|s| plain.text[s.range.clone()].trim_end() == "Title text\nhere."));
    // the edit window ends with a heading still waiting for its title: the whole file is
    // segmented again, so the heading takes the paragraph after it
    let mut fb = FileBuf::with_rules(&doc, &mut ids, 1, rules);
    let at = fb.text.find("Title text").unwrap();
    let end = fb.text.find("Next.").unwrap();
    fb.apply(
        &Edit {
            start_byte: at,
            end_byte: end,
            text: String::new(),
        },
        &mut ids,
        2,
    );
    let apart = fb
        .spans
        .iter()
        .find(|s| fb.text[s.range.clone()].starts_with("\\paragraph{Apart.}"))
        .unwrap();
    assert!(fb.text[apart.range.clone()]
        .trim_end()
        .ends_with("\n\nNext."));
    assert_eq!(
        fb.spans
            .iter()
            .map(|s| (s.range.clone(), s.kind))
            .collect::<Vec<_>>(),
        segment(&fb.text, &fb.rules)
    );
}

#[test]
fn inline_picture_stays_in_its_paragraph() {
    let mut ids = IdAllocator(0);
    let text = "\\documentclass{article}\n\\begin{document}\nAn inline \\begin{tikzpicture}\\draw (0,0)--(1,0);\\end{tikzpicture} picture\nand more text.\n\n\\begin{tikzpicture}\n\\draw (0,0)--(1,0);\n\\end{tikzpicture}\nText after a block picture.\n\\end{document}\n";
    let fb = FileBuf::new(text, &mut ids, 1);
    let texts: Vec<(&str, SpanKind)> = fb
        .spans
        .iter()
        .map(|s| (fb.text[s.range.clone()].trim_end(), s.kind))
        .collect();
    assert_eq!(
        texts[1],
        (
            "An inline \\begin{tikzpicture}\\draw (0,0)--(1,0);\\end{tikzpicture} picture\nand more text.",
            SpanKind::Body
        )
    );
    assert_eq!(
        texts[2],
        (
            "\\begin{tikzpicture}\n\\draw (0,0)--(1,0);\n\\end{tikzpicture}",
            SpanKind::Env
        )
    );
    assert_eq!(texts[3], ("Text after a block picture.", SpanKind::Body));
}

#[test]
fn preamble_edit_flagged() {
    let mut ids = IdAllocator(0);
    let mut fb = FileBuf::new(DOC, &mut ids, 1);
    let pos = fb.text.find("microtype").unwrap();
    let out = fb.apply(
        &Edit {
            start_byte: pos,
            end_byte: pos + 9,
            text: "xcolor".into(),
        },
        &mut ids,
        2,
    );
    assert!(out.preamble_changed);
}
