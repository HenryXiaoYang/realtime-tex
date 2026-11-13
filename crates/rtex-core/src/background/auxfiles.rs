//! The aux family between passes: its signature (has anything a later pass reads changed?), what the bibliography program reads, rerun requests in the log, copying it to the next pass, and the labels in it.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// `bytes` with the pass directory's path taken out, plain and hex-encoded (bookmark writes
/// `srcfile={<hex of the .ind path>}` into the `.aux`): passes alternate between directories,
/// and the same document must give the same signature in either.
pub(super) fn without_dir(bytes: &[u8], dir_forms: &[Vec<u8>]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for form in dir_forms {
        if form.is_empty() {
            continue;
        }
        let mut next = Vec::with_capacity(out.len());
        let mut i = 0;
        while i < out.len() {
            if out[i..].starts_with(form) {
                next.extend_from_slice(b"<dir>");
                i += form.len();
            } else {
                next.push(out[i]);
                i += 1;
            }
        }
        out = next;
    }
    out
}

pub(super) fn dir_forms(out_dir: &Path) -> Vec<Vec<u8>> {
    let mut dirs = vec![out_dir.to_path_buf()];
    if let Ok(c) = crate::paths::canonical(out_dir) {
        dirs.push(c);
    }
    // as given and as TeX writes it back (forward slashes on Windows)
    let mut paths: Vec<String> = Vec::new();
    for d in &dirs {
        paths.push(d.to_string_lossy().into_owned());
        paths.push(crate::paths::tex(d));
    }
    let mut forms = Vec::new();
    for p in paths {
        let hex: String = p.bytes().map(|b| format!("{b:02X}")).collect();
        forms.push(hex.to_ascii_lowercase().into_bytes());
        forms.push(hex.into_bytes());
        forms.push(p.into_bytes());
    }
    // longest first, so a path is replaced before a shorter form inside it; equal forms
    // (an already canonical directory) adjacent so `dedup` drops them
    forms.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    forms.dedup();
    forms
}

pub(super) fn aux_signature(out_dir: &Path, jobname: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    let forms = dir_forms(out_dir);
    // every index (imakeidx's `name=` indexes too), as makeindex left it
    let mut inds: Vec<PathBuf> = std::fs::read_dir(out_dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "ind"))
                .collect()
        })
        .unwrap_or_default();
    inds.sort();
    for p in &inds {
        if let Ok(b) = std::fs::read(p) {
            p.file_name().hash(&mut h);
            without_dir(&b, &forms).hash(&mut h);
        }
    }
    for ext in ["aux", "toc", "lof", "lot", "out", "bcf", "bbl", "idx"] {
        if let Ok(b) = std::fs::read(out_dir.join(format!("{jobname}.{ext}"))) {
            let b = without_dir(&b, &forms);
            ext.hash(&mut h);
            b.hash(&mut h);
            if ext == "aux" {
                // partial aux files of \include'd chapters (\@input{chapters/x.aux})
                let text = String::from_utf8_lossy(&b);
                for part in text.split("\\@input{").skip(1) {
                    let Some(name) = part.split('}').next() else {
                        continue;
                    };
                    if let Ok(pb) = std::fs::read(out_dir.join(name)) {
                        name.hash(&mut h);
                        without_dir(&pb, &forms).hash(&mut h);
                    }
                }
            }
        }
    }
    h.finish()
}

/// What the bibliography program reads from a pass: Biber's control file, BibTeX's lines of
/// the aux file. The program runs again only when this changes.
pub(super) fn bib_input(out_dir: &Path, jobname: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let forms = dir_forms(out_dir);
    if let Ok(b) = std::fs::read(out_dir.join(format!("{jobname}.bcf"))) {
        without_dir(&b, &forms).hash(&mut h);
    }
    if let Ok(aux) = std::fs::read_to_string(out_dir.join(format!("{jobname}.aux"))) {
        for line in aux.lines().filter(|l| {
            ["\\citation", "\\bibdata", "\\bibstyle"]
                .iter()
                .any(|c| l.starts_with(c))
        }) {
            line.hash(&mut h);
        }
    }
    h.finish()
}

/// Whether the log of a pass asks for another one. `bib_ran`: the bibliography program has
/// run on what this pass gave it. biblatex's request for it ("Please (re)run Biber on the
/// file: … and rerun LaTeX afterwards.") then stands for as long as the program cannot meet
/// it (a citation the database lacks, a program that fails), and no further pass changes that.
pub(super) fn log_requests_rerun(log: &Path, bib_ran: bool) -> bool {
    let Ok(mut s) = std::fs::read_to_string(log) else {
        return false;
    };
    const BIB_REQUEST: &str = "Please (re)run Bib";
    const BIB_REQUEST_END: &str = "rerun LaTeX afterwards";
    while let Some(start) = s.find(BIB_REQUEST).filter(|_| bib_ran) {
        let end = s[start..]
            .find(BIB_REQUEST_END)
            .map_or(BIB_REQUEST.len(), |e| e + BIB_REQUEST_END.len());
        s.replace_range(start..start + end, "");
    }
    s.contains("Rerun to get") || s.contains("rerun LaTeX") || s.contains(BIB_REQUEST)
}

/// Copy the aux family a pass leaves for the next one (`.aux .toc .lof .lot .out .bbl .bcf
/// .ind .gls .nav .snm`, and the partial `.aux` files of `\include`d chapters in
/// subdirectories) from one pass directory into another. Pass outputs (PDF, log, capture
/// JSON) and the picture cache are not copied, nor are the files TeX only writes, which a
/// standby engine may already hold open from its preamble (`.idx` from `\makeindex`, `.glo`
/// from `\makeglossaries`): a copy would land in a file being written.
pub fn copy_aux_family(from: &Path, to: &Path) -> Result<()> {
    fn go(root: &Path, dir: &Path, to: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if name != "pic-cache" && !name.starts_with("pass-") {
                    go(root, &path, to)?;
                }
                continue;
            }
            let keep = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    matches!(
                        e,
                        "aux"
                            | "toc"
                            | "lof"
                            | "lot"
                            | "out"
                            | "bbl"
                            | "bcf"
                            | "blg"
                            | "ind"
                            | "gls"
                            | "nav"
                            | "snm"
                            | "vrb"
                            | "xml"
                    )
                })
                .unwrap_or(false);
            if keep {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                let dst = to.join(rel);
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&path, dst)?;
            }
        }
        Ok(())
    }
    if from == to || !from.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    go(from, from, to)
}

/// `\newlabel{name}{value}` and `\bibcite{key}{value}` definitions from an `.aux` file (and the
/// files it `\@input`s), as (control-sequence name, macro body) pairs the fast server can define
/// with `token.set_macro`: `r@name` → value, `b@key` → value.
pub fn read_aux_labels(aux: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    read_aux_into(aux, &mut out, &mut seen, 0);
    out
}

pub(super) fn read_aux_into(
    aux: &Path,
    out: &mut Vec<(String, String)>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if depth > 8 || !seen.insert(aux.to_path_buf()) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(aux) else {
        return;
    };
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        let (prefix, kind) = if rest.starts_with("\\newlabel") {
            ("\\newlabel", "r@")
        } else if rest.starts_with("\\bibcite") {
            ("\\bibcite", "b@")
        } else if rest.starts_with("\\@input") {
            ("\\@input", "")
        } else {
            i += 1;
            continue;
        };
        let mut j = i + prefix.len();
        let Some((name, after)) = brace_group(&text, j) else {
            i += 1;
            continue;
        };
        j = after;
        if kind.is_empty() {
            let child = aux.parent().unwrap_or(Path::new(".")).join(name.trim());
            read_aux_into(&child, out, seen, depth + 1);
            i = j;
            continue;
        }
        let Some((value, after)) = brace_group(&text, j) else {
            i += 1;
            continue;
        };
        out.push((format!("{kind}{name}"), value));
        i = after;
    }
}

/// The contents of the brace group starting at (or after whitespace from) `from`; returns the
/// inner text and the index after the closing brace.
pub(super) fn brace_group(text: &str, from: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    let mut i = from;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\n' || b[i] == b'\r' || b[i] == b'\t') {
        i += 1;
    }
    if i >= b.len() || b[i] != b'{' {
        return None;
    }
    let start = i + 1;
    let mut depth = 0i32;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod aux_tests {
    use super::*;
    #[test]
    fn labels_and_bibcites() {
        let dir = std::env::temp_dir().join(format!("rtex-aux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.aux"), "\\relax\n\\newlabel{eq:a}{{1.2}{5}}\n\\bibcite{knuth}{1}\n\\@input{ch.aux}\n\\newlabel{fig:x}{{1}{2}{Caption}{figure.1}{}}\n").unwrap();
        std::fs::write(dir.join("ch.aux"), "\\newlabel{sec:b}{{3}{7}}\n").unwrap();
        let v = read_aux_labels(&dir.join("main.aux"));
        assert_eq!(
            v,
            vec![
                ("r@eq:a".to_string(), "{1.2}{5}".to_string()),
                ("b@knuth".to_string(), "1".to_string()),
                ("r@sec:b".to_string(), "{3}{7}".to_string()),
                (
                    "r@fig:x".to_string(),
                    "{1}{2}{Caption}{figure.1}{}".to_string()
                ),
            ]
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Standby background engine: a lualatex that has loaded the preamble and waits for the body.
// Engine start, format and preamble (packages, fonts) are ~75 % of a pass over a short
// document (docs/benchmarks.md), so a pass that only typesets the body is 3–4× faster.
// ---------------------------------------------------------------------------------------------
