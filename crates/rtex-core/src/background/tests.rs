use super::auxfiles::*;
use super::index::*;
use super::snapshot::*;
use super::*;

#[test]
fn snapshot_copies_nested_folders() {
    let dir = std::env::temp_dir().join(format!("rtex-snap-nested-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("a/b")).unwrap();
    std::fs::write(project.join("main.tex"), "x").unwrap();
    std::fs::write(project.join("a/b/file.txt"), "nested").unwrap();
    std::fs::write(project.join("a/top.bib"), "bib").unwrap();
    let mut files = BTreeMap::new();
    files.insert("main.tex".to_string(), "edited".to_string());
    write_snapshot(&project, &files, "main.tex", &dir.join("snap")).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("snap/a/b/file.txt")).unwrap(),
        "nested"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("snap/a/top.bib")).unwrap(),
        "bib"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("snap/main.tex")).unwrap(),
        "edited"
    );
    // a second snapshot (files already present) is fine too
    write_snapshot(&project, &files, "main.tex", &dir.join("snap")).unwrap();
}

#[test]
fn snapshot_links_instead_of_copying() {
    let dir = std::env::temp_dir().join(format!("rtex-snap-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("fig")).unwrap();
    std::fs::create_dir_all(project.join("chap")).unwrap();
    std::fs::write(project.join("main.tex"), "saved").unwrap();
    std::fs::write(project.join("chap/one.tex"), "saved").unwrap();
    std::fs::write(project.join("chap/data.csv"), "1,2").unwrap();
    std::fs::write(project.join("fig/a.png"), "v1").unwrap();
    // a snapshot from rtex 0.0.6 (copies, no marker) is replaced
    std::fs::create_dir_all(dir.join("src/fig")).unwrap();
    std::fs::write(dir.join("src/fig/a.png"), "old copy").unwrap();
    let files = BTreeMap::from([
        ("main.tex".to_string(), "edited".to_string()),
        ("chap/one.tex".to_string(), "edited one".to_string()),
    ]);
    write_snapshot(&project, &files, "main.tex", &dir.join("src")).unwrap();
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    assert_eq!(read("src/main.tex"), "edited");
    assert_eq!(read("src/chap/one.tex"), "edited one");
    assert_eq!(read("src/chap/data.csv"), "1,2");
    assert_eq!(read("src/fig/a.png"), "v1");
    // the sources were not written through into the project
    assert_eq!(read("project/main.tex"), "saved");
    assert_eq!(read("project/chap/one.tex"), "saved");
    // a folder with a source is real, one without is a link (where links are allowed)
    assert!(!is_link(&dir.join("src/chap")));
    #[cfg(unix)]
    assert!(is_link(&dir.join("src/fig")));
    // a figure saved again is what the next pass reads
    std::fs::write(project.join("fig/a.png"), "v2").unwrap();
    write_snapshot(&project, &files, "main.tex", &dir.join("src")).unwrap();
    assert_eq!(read("src/fig/a.png"), "v2");
    // a folder that gets a source becomes real
    let mut more = files.clone();
    more.insert("fig/caption.tex".to_string(), "edited caption".to_string());
    write_snapshot(&project, &more, "main.tex", &dir.join("src")).unwrap();
    assert!(!is_link(&dir.join("src/fig")));
    assert_eq!(read("src/fig/caption.tex"), "edited caption");
    assert!(!project.join("fig/caption.tex").exists());
    assert_eq!(read("src/fig/a.png"), "v2");
}

#[test]
fn snapshot_copies_other_documents_aux_only() {
    let dir = std::env::temp_dir().join(format!("rtex-snap-aux-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("main.aux"), "stale").unwrap();
    std::fs::write(project.join("chap.aux"), "stale").unwrap();
    // read by \externaldocument{appendix}
    std::fs::write(project.join("appendix.aux"), "labels").unwrap();
    std::fs::write(project.join("appendix.pdf"), "built alone").unwrap();
    let mut files = BTreeMap::new();
    files.insert("main.tex".to_string(), "\\include{chap}".to_string());
    files.insert("chap.tex".to_string(), "x".to_string());
    // the host sends every open file, also another document the main file does not read
    files.insert("appendix.tex".to_string(), "x".to_string());
    write_snapshot(&project, &files, "main.tex", &dir.join("snap")).unwrap();
    assert!(!dir.join("snap/main.aux").exists());
    assert!(!dir.join("snap/chap.aux").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("snap/appendix.aux")).unwrap(),
        "labels"
    );
}

#[test]
fn index_commands_follow_imakeidx() {
    let rec = "makeindex -s mystyle.ist main.idx\ntexindy -L english -C utf8 names.idx\n";
    assert_eq!(
        index_command(rec, "main.idx"),
        (
            "makeindex".to_string(),
            vec!["-q", "-s", "mystyle.ist", "main.idx"]
                .into_iter()
                .map(String::from)
                .collect()
        )
    );
    assert_eq!(index_command(rec, "names.idx").0, "texindy");
    assert_eq!(
        index_command(rec, "names.idx").1.last().unwrap(),
        "names.idx"
    );
    // no record (makeidx, or an index imakeidx never printed): plain makeindex
    assert_eq!(
        index_command(rec, "other.idx"),
        (
            "makeindex".to_string(),
            vec!["-q".to_string(), "other.idx".to_string()]
        )
    );
    // a program outside the list is not run
    assert_eq!(index_command("rm -rf x.idx", "x.idx").0, "makeindex");
}

#[test]
fn signature_ignores_the_pass_directory() {
    let dir = std::env::temp_dir().join(format!("rtex-sig-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (a, b) = (dir.join("pass-0"), dir.join("pass-1"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d).unwrap();
        let p = crate::paths::canonical(d).unwrap().join("main.ind");
        let hex: String = p
            .to_string_lossy()
            .bytes()
            .map(|x| format!("{x:02X}"))
            .collect();
        std::fs::write(
            d.join("main.aux"),
            format!(
                "\\BKM@entry{{srcfile={{{hex}}}}}\n\\@input{{{}}}\n",
                p.display()
            ),
        )
        .unwrap();
        std::fs::write(d.join("main.ind"), "\\begin{theindex}\\end{theindex}").unwrap();
    }
    assert_eq!(aux_signature(&a, "main"), aux_signature(&b, "main"));
    std::fs::write(
        b.join("main.ind"),
        "\\begin{theindex}\\item x\\end{theindex}",
    )
    .unwrap();
    assert_ne!(aux_signature(&a, "main"), aux_signature(&b, "main"));
}

#[test]
fn snapshot_keeps_input_pdfs_and_drops_built_ones() {
    let dir = std::env::temp_dir().join(format!("rtex-snap-pdf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("images")).unwrap();
    std::fs::write(project.join("main.tex"), "x").unwrap();
    std::fs::write(project.join("main.pdf"), "built").unwrap();
    std::fs::write(project.join("figure.pdf"), "input").unwrap();
    std::fs::write(project.join("images/plot.pdf"), "input").unwrap();
    // a standalone figure: its source sits next to it but the document only reads the PDF
    std::fs::write(project.join("images/fig.tex"), "standalone").unwrap();
    std::fs::write(project.join("images/fig.pdf"), "input").unwrap();
    let files = BTreeMap::from([("main.tex".to_string(), "x".to_string())]);
    write_snapshot(&project, &files, "main.tex", &dir.join("snap")).unwrap();
    assert!(dir.join("snap/images/fig.pdf").is_file());
    assert!(!dir.join("snap/main.pdf").exists());
    assert!(dir.join("snap/figure.pdf").is_file());
    assert!(dir.join("snap/images/plot.pdf").is_file());
}

#[test]
fn body_snapshot_keeps_line_numbers() {
    let dir = std::env::temp_dir().join(format!("rtex-body-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let main = "\\documentclass{article}\n\\usepackage{xcolor}\n% two\n\\begin{document}\nFirst line of the body.\n\n\\end{document}\n";
    std::fs::write(project.join("main.tex"), main).unwrap();
    let mut files = BTreeMap::new();
    files.insert("main.tex".to_string(), main.to_string());
    let h = write_body_snapshot(&project, &files, "main.tex", &dir.join("snap")).unwrap();
    let pre = std::fs::read_to_string(dir.join("snap/rtex-preamble.tex")).unwrap();
    let body = std::fs::read_to_string(dir.join("snap/main.tex")).unwrap();
    assert_eq!(
        pre,
        "\\documentclass{article}\n\\usepackage{xcolor}\n% two\n"
    );
    assert_eq!(h, crate::document::hash_str(&pre));
    // \begin{document} is on line 4 in both the original and the padded body file
    assert_eq!(
        main.lines()
            .position(|l| l.starts_with("\\begin{document}")),
        Some(3)
    );
    assert_eq!(
        body.lines()
            .position(|l| l.starts_with("\\begin{document}")),
        Some(3)
    );
    assert_eq!(body.lines().count(), main.lines().count());
    assert_eq!(body.lines().nth(4), Some("First line of the body."));
}
