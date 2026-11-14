use super::cacheable::*;
use super::scan::*;
use super::store::*;
use super::*;

fn texts_of(main: &str) -> BTreeMap<String, String> {
    let mut t = BTreeMap::new();
    t.insert("main.tex".to_string(), main.to_string());
    t
}

#[test]
fn scan_and_hash() {
    let mut texts = texts_of(
        "\\documentclass{article}\n\\begin{document}\n\\def\\H{4}\nText.\n\\begin{tikzpicture}\n  \\draw (0,0) -- (\\H,1);\n\\end{tikzpicture}\n\n\\begin{tikzpicture}[remember picture]\n\\end{tikzpicture}\n\\begin{tikzpicture}\n\\node {\\ref{x}};\n\\end{tikzpicture}\n\\end{document}\n",
    );
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics.len(), 3);
    assert!(
        !pics.iter().any(|p| p.cacheable),
        "remember picture disables the cache"
    );
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replace("[remember picture]", "");
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics[0].key, "main.tex:5");
    assert_eq!(pics[0].end_line, 7);
    assert!(pics[0].cacheable && pics[1].cacheable && !pics[2].cacheable);
    let h0 = pics[0].hash;
    // a definition before the picture changes its hash; text after it does not
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replace("\\def\\H{4}", "\\def\\H{5}");
    let pics2 = scan_pictures(&texts, "main.tex", 1);
    assert_ne!(pics2[0].hash, h0);
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replace("Text.", "Other text.");
    let pics3 = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics3[0].hash, pics2[0].hash);
    // the preamble is part of every hash
    let pics4 = scan_pictures(&texts, "main.tex", 2);
    assert_ne!(pics4[0].hash, pics3[0].hash);
    // a comment mentioning remember picture does not disable the cache
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replace("Other text.", "Other text. % use remember picture one day");
    assert!(scan_pictures(&texts, "main.tex", 1)[0].cacheable);
    // remember picture set in the preamble (tikzmark, overlays) disables it as well
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replace(
        "\\documentclass{article}\n",
        "\\documentclass{article}\n\\tikzset{every picture/.style={remember picture}}\n",
    );
    assert!(!scan_pictures(&texts, "main.tex", 1)[0].cacheable);
}

#[test]
fn the_body_starts_at_the_begin_document_tex_reads() {
    // a commented \begin{document} in the preamble does not make the preamble's picture
    // (inside a macro definition) a body picture
    let main = "\\documentclass{article}\n% \\begin{document} goes below\n\\newcommand\\pic{%\n\\begin{tikzpicture}\\draw (0,0)--(1,1);\\end{tikzpicture}}\n\\begin{document}\n\\begin{tikzpicture}\n\\draw (0,0) -- (1,1);\n\\end{tikzpicture}\n\\end{document}\n";
    let pics = scan_pictures(&texts_of(main), "main.tex", 0);
    let keys: Vec<&str> = pics.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["main.tex:6"]);
}

#[test]
fn counters_links_and_double_inputs_are_not_cached() {
    // \the<counter> prints a number the picture's text does not show
    assert!(uncacheable("\\node {Section \\thesection};"));
    assert!(uncacheable("\\node {p.~\\thepage};"));
    assert!(!uncacheable("\\draw (0,0) -- (\\theta:1);"));
    assert!(!uncacheable("\\node {$\\therefore x$};"));
    assert!(uncacheable("\\node {\\href{https://x.y}{link}};"));
    assert!(uncacheable("\\node {text\\footnotemark};"));
    assert!(uncacheable("\\pgfimage{fig}"));
    // a file read twice: its pictures share their file:line keys
    let mut texts =
        texts_of("\\begin{document}\n\\input{fig}\n\\def\\H{2}\n\\input{fig}\n\\end{document}\n");
    texts.insert(
        "fig.tex".into(),
        "\\begin{tikzpicture}\n\\draw (0,0) -- (\\H,1);\n\\end{tikzpicture}\n".into(),
    );
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics.len(), 2);
    assert_eq!(pics[0].key, pics[1].key);
    assert!(!pics[0].cacheable && !pics[1].cacheable);
    let t = texts.get_mut("main.tex").unwrap();
    *t = t.replacen("\\input{fig}\n", "", 1);
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics.len(), 1);
    assert!(pics[0].cacheable);
}

#[test]
fn prelude_order_and_multiline_statements() {
    // a multi-line \tikzset: a change on its second line reaches the pictures after it
    let mut texts = texts_of(
        "\\begin{document}\n\\tikzset{%\n  every node/.style={draw,red},\n}\n\\begin{tikzpicture}\n\\node {a};\n\\end{tikzpicture}\n\\input{styles}\n\\begin{tikzpicture}[mystyle]\n\\node {b};\n\\end{tikzpicture}\n\\end{document}\n",
    );
    texts.insert(
        "styles.tex".to_string(),
        "\\centering\\tikzset{mystyle/.style={thick}}\n".to_string(),
    );
    let p1 = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(p1.len(), 2);
    let changed = texts["main.tex"].replace("draw,red", "draw,blue");
    texts.insert("main.tex".to_string(), changed);
    let p2 = scan_pictures(&texts, "main.tex", 1);
    assert_ne!(p1[0].hash, p2[0].hash, "second line of the \\tikzset block");
    // a definition in an \input file (not at the start of its line) reaches the pictures
    // after the \input, not the ones before it
    texts.insert(
        "styles.tex".to_string(),
        "\\centering\\tikzset{mystyle/.style={dashed}}\n".to_string(),
    );
    let p3 = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(p2[0].hash, p3[0].hash);
    assert_ne!(p2[1].hash, p3[1].hash);
}

#[test]
fn cacheability_rules() {
    assert!(!uncacheable(
        "\\node {$\\alpha$}; \\draw (0,0) -- (\\theta:1);"
    ));
    assert!(!uncacheable(
        "\\reflectbox{x} \\romannumeral 4 \\indexspace"
    ));
    assert!(uncacheable("\\node {\\ref{x}};"));
    assert!(uncacheable("\\node {\\alph{page}};"));
    assert!(uncacheable("\\citep{k}"));
    assert!(uncacheable("\\addplot table[x=t,y=v] {results.dat};"));
    assert!(uncacheable("\\addplot table [col sep=comma] {f.csv};"));
    assert!(uncacheable("\\addplot file {f.dat};"));
    assert!(!uncacheable("\\node {a table of files};"));
    assert!(uncacheable("\\pgfmathsetmacro\\w{3} \\xdef\\figwidth{\\w}"));
    assert!(uncacheable("\\global\\advance\\c by 1"));
    assert!(uncacheable("\\includegraphics*[width=1cm]{f}"));
    // bodies read under other catcodes, Lua
    assert!(uncacheable(
        "\\begin{luacode*}\n tex.print(\"{hsb}{\")\n\\end{luacode*}"
    ));
    assert!(uncacheable("\\begin{Verbatim}x\\end{Verbatim}"));
    assert!(uncacheable("\\node {\\directlua{tex.print(1)}};"));
    assert!(uncacheable("{\\catcode`\\|=13 x}"));
    assert!(!uncacheable("\\node[draw] {a comment-free node};"));
}

#[test]
fn the_body_skip_must_stop_at_the_pictures_end() {
    let ok = "\\begin{tikzpicture}\n\\draw (0,0) -- (1,1);\n\\end{tikzpicture}";
    assert!(skip_stops_at_end(ok, "tikzpicture"));
    // a `{` left open by a stripped `%` (luacode* read under normal catcodes)
    let open = "\\begin{tikzpicture}\n\\fill[c] {hsb}{\n\\end{tikzpicture}";
    assert!(!skip_stops_at_end(open, "tikzpicture"));
    let extra = "\\begin{tikzpicture}\n}\\draw;\n\\end{tikzpicture}";
    assert!(!skip_stops_at_end(extra, "tikzpicture"));
    let nested = "\\begin{tikzpicture}\n\\node {\\end{tikzpicture}};";
    assert!(!skip_stops_at_end(nested, "tikzpicture"));
    // a picture after text on its line, or with text before its \begin, is not keyed by
    // the line its \begin is on
    let texts = texts_of(
        "\\begin{document}\n\\foo{\n\\begin{tikzpicture} a\n\\end{tikzpicture}}\\begin{tikzpicture}\nb\n\\end{tikzpicture}\n\\end{document}\n",
    );
    // the scan sees one picture (two \begins before the last \end): not cacheable
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics.len(), 1);
    assert!(!pics[0].cacheable);
    let texts = texts_of(
        "\\begin{document}\n\\foo{x}\\begin{tikzpicture}\nb\n\\end{tikzpicture}\n\\end{document}\n",
    );
    let pics = scan_pictures(&texts, "main.tex", 1);
    assert_eq!(pics.len(), 1);
    assert!(!pics[0].cacheable, "text before the \\begin on its line");
}

#[test]
fn cache_roundtrip() {
    let dir = std::env::temp_dir().join(format!("rtex-piccache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pdf = dir.join("pass.pdf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&pdf, b"%PDF-1.5 fake").unwrap();
    let pics = vec![PictureRef {
        key: "main.tex:5".into(),
        env: "tikzpicture".into(),
        hash: 42,
        cacheable: true,
        end_line: 9,
    }];
    let mut cache = PicCache::open(&dir.join("cache"));
    let manifest = dir.join("pic-manifest.json");
    assert_eq!(cache.write_manifest(&pics, &manifest).unwrap(), 0);
    assert!(!manifest.exists());
    let mut rec = BTreeMap::new();
    rec.insert(
        "main.tex:5".to_string(),
        RecordedPic {
            env: "tikzpicture".into(),
            page: 2,
            x: 100,
            y: 1000,
            w: 300,
            h: 200,
            d: 50,
            page_height: 5000,
            state: "font/0 g 0 G".into(),
        },
    );
    // not a real PDF: the whole file is copied and page numbers stay
    assert_eq!(cache.absorb(&pics, &rec, &[], &pdf).unwrap(), 1);
    assert_eq!(cache.entries(), 1);
    assert_eq!(cache.write_manifest(&pics, &manifest).unwrap(), 1);
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    let e = &m["main.tex:5"];
    assert_eq!(e["page"], 2);
    assert_eq!(e["end_line"], 9);
    assert_eq!(
        e["bbox"],
        serde_json::json!([100, 5000 - 1050, 400, 5000 - 800])
    );
    assert_eq!(e["d"], 50);
    assert_eq!(e["state"], "font/0 g 0 G");
    // the same picture drawn under another state (font, color, width) replaces the entry
    let mut rec2 = rec.clone();
    rec2.get_mut("main.tex:5").unwrap().state = "other font/0 g 0 G".into();
    rec2.get_mut("main.tex:5").unwrap().h = 400;
    assert_eq!(cache.absorb(&pics, &rec2, &[], &pdf).unwrap(), 1);
    assert_eq!(cache.entries(), 1);
    cache.write_manifest(&pics, &manifest).unwrap();
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    assert_eq!(m["main.tex:5"]["state"], "other font/0 g 0 G");
    assert_eq!(m["main.tex:5"]["h"], 400);
    // a pass that reports the picture as mismatched forgets it
    assert_eq!(
        cache
            .absorb(&pics, &BTreeMap::new(), &["main.tex:5".into()], &pdf)
            .unwrap(),
        0
    );
    assert_eq!(cache.entries(), 0);
    // and draws it (no entry) for KEEP_PASSES passes before caching it again
    assert_eq!(cache.absorb(&pics, &rec, &[], &pdf).unwrap(), 0);
    for _ in 0..KEEP_PASSES {
        assert_eq!(cache.write_manifest(&pics, &manifest).unwrap(), 0);
        assert_eq!(cache.absorb(&pics, &rec, &[], &pdf).unwrap(), 0);
    }
    cache.write_manifest(&pics, &manifest).unwrap();
    assert_eq!(cache.absorb(&pics, &rec, &[], &pdf).unwrap(), 1);
    assert_eq!(cache.write_manifest(&pics, &manifest).unwrap(), 1);
    // a changed picture: no hit; after KEEP_PASSES unwanted passes the entry and its PDF go
    let changed = vec![PictureRef {
        hash: 43,
        ..pics[0].clone()
    }];
    for _ in 0..(KEEP_PASSES + 1) {
        assert_eq!(cache.write_manifest(&changed, &manifest).unwrap(), 0);
        cache.absorb(&changed, &BTreeMap::new(), &[], &pdf).unwrap();
    }
    assert_eq!(cache.entries(), 0);
    assert!(std::fs::read_dir(dir.join("cache"))
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().ends_with(".pdf")));
    let _ = std::fs::remove_dir_all(&dir);
}
