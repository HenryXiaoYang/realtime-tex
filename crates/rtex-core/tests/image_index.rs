//! IMAGE display-list items must find their file in `images_info` by their own index. The
//! items carry LuaTeX's image index (the image rule's `index`), which counts every image the
//! run makes, also those graphicx never sees (a cached picture is one); `images_info` used to
//! be keyed by `\lastsavedimageresourceindex`, the PDF object number. Skipped without lualatex.

use rtex_core::capture::run_capture_with;
use rtex_core::texlive::TexLive;
use rtex_dl::Item;

// The \directlua lines stand in for what a real document does before its first graphic:
// PDF objects written by other packages (so object numbers run ahead of image indices) and an
// image made outside graphicx, as the picture cache makes one for each cached picture.
const DOC: &str = r#"\documentclass{article}
\usepackage{graphicx}
\begin{document}
\directlua{for i = 1, 20 do pdf.immediateobj("<<>>") end}%
\directlua{node.write(img.node(img.scan(img.new{filename = kpse.find_file("example-image-b.pdf")})))}%
\includegraphics[width=2cm]{example-image-a.pdf}
\includegraphics[width=2cm]{example-image-c.pdf}
\end{document}
"#;

#[test]
fn image_items_are_keyed_by_their_rule_index() {
    let Ok(tl) = TexLive::discover() else {
        eprintln!("SKIP: no lualatex");
        return;
    };
    let root = std::env::temp_dir().join(format!("rtex-image-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("main.tex"), DOC).unwrap();
    let cap =
        run_capture_with(&tl, &project, "main.tex", &root.join("out"), true, "", None).unwrap();
    let dl = cap.page(1).unwrap();
    let mut files = Vec::new();
    let items = dl
        .lines
        .iter()
        .flat_map(|l| l.items.iter())
        .chain(dl.other.iter());
    for it in items {
        if let Item::Image { index, .. } = it {
            if let Some(info) = dl.images.get(&index.to_string()) {
                assert_eq!(info.index, *index);
                files.push(info.file.clone());
            }
        }
    }
    // the image made outside graphicx has no entry; the two graphics resolve to their files
    assert_eq!(
        files,
        ["example-image-a.pdf", "example-image-c.pdf"],
        "images_info keys {:?}",
        dl.images.keys().collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&root);
}
