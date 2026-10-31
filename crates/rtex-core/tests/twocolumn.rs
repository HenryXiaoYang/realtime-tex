//! Two-column documents: LaTeX builds the page's two-column box in its output routine, which
//! runs while a paragraph unit is still open when a column break falls inside the paragraph.
//! That box must not count as a row of the unit, or the lines inside it lose their placements
//! (HenryXiaoYang/realtime-tex#2). Skipped without lualatex.

use rtex_core::capture::run_capture_with;
use rtex_core::texlive::TexLive;

/// 160 paragraphs of one or two sentences: column breaks land inside paragraphs, while the
/// paragraph's unit is open.
fn document() -> String {
    let body: Vec<String> = (0..160)
        .map(|i| {
            format!(
                "Paragraph {i} has a few ordinary words so that it fills about three lines of a narrow column. "
            )
            .repeat(1 + i % 2)
        })
        .collect();
    format!(
        "\\documentclass[twocolumn]{{article}}\n\\begin{{document}}\n{}\n\\end{{document}}\n",
        body.join("\n\n")
    )
}

#[test]
fn every_unit_of_a_two_column_document_has_its_lines_as_rows() {
    let Ok(tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return;
    };
    let root = std::env::temp_dir().join(format!("rtex-twocolumn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("main.tex"), document()).unwrap();
    let cap =
        run_capture_with(&tl, &project, "main.tex", &root.join("out"), true, "", None).unwrap();
    assert!(cap.json.pages >= 4, "{} pages", cap.json.pages);
    let lines: std::collections::HashMap<i64, i64> = cap
        .json
        .paragraphs
        .iter()
        .map(|p| (p.seq, p.lines.unwrap_or(0)))
        .collect();
    let mut wrong = Vec::new();
    for u in cap.json.units.iter().filter(|u| u.kind == "par") {
        let expected: i64 = u
            .seqs
            .iter()
            .map(|s| lines.get(s).copied().unwrap_or(0))
            .sum();
        if u.placements.len() as i64 != expected {
            wrong.push((u.begin_line, expected, u.placements.len()));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        wrong.is_empty(),
        "units whose rows are not their lines (line, lines, rows): {wrong:?}"
    );
}
