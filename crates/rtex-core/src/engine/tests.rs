use super::tex_log::*;

#[test]
fn startup_errors_quote_the_first_tex_error() {
    let log = std::env::temp_dir().join(format!("rtex-excerpt-{}.log", std::process::id()));
    std::fs::write(
        &log,
        "This is LuaHBTeX\n(./rtex-serve-g1.tex\n! LaTeX Error: File `foo.sty' not found.\n\nType X to quit.\nl.3 \\usepackage{foo}\nmore\n",
    )
    .unwrap();
    let x = tex_log_excerpt(&log).unwrap();
    let _ = std::fs::remove_file(&log);
    assert_eq!(
        x,
        "! LaTeX Error: File `foo.sty' not found. /  / Type X to quit. / l.3 \\usepackage{foo}"
    );
    // a Lua error comes before the TeX errors it causes
    std::fs::write(
        &log,
        {
            // wrapped at 79 characters, as MiKTeX writes it
            let msg = format!(
                "(./{}.tex)D:/rtex-serve.lua:712: cannot open response channel PIPE",
                "y".repeat(40)
            );
            let (head, rest) = msg.split_at(79);
            format!("{head}\n{rest}\nstack traceback:\n\t[C]: in function 'assert'\nl.5 ...\n\n! Undefined control sequence.\nl.7 \\loop\n")
        },
    )
    .unwrap();
    let x = tex_log_excerpt(&log).unwrap();
    let _ = std::fs::remove_file(&log);
    assert!(x.contains("cannot open response channel PIPE"), "{x}");
    // \stop in the preamble: no error, no page
    std::fs::write(&log, "(./rtex-preamble.tex)\n )\n\nwarning  (pdf backend): no pages of output.\n\nPDF statistics: 0 PDF objects out of 1000\n").unwrap();
    let x = tex_log_excerpt(&log).unwrap();
    let _ = std::fs::remove_file(&log);
    assert!(
        x.starts_with("the preamble ended the run before \\begin{document}"),
        "{x}"
    );
}
