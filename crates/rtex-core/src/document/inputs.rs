//! `\\input`, `\\include` and `\\subfile`: which files a text pulls in, loading them, and inlining them.

use super::segment::*;
use super::*;
use std::ops::Range;

/// `\input{…}`,`\include{…}` and `\subfile{…}` commands in `text` (outside comments), as
/// (byte range of the command, project-relative target with a `.tex` extension). Absolute
/// paths, `..`, and names built from macros are skipped.
pub(crate) fn find_inputs(text: &str) -> Vec<(Range<usize>, String, &'static str)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                i += text[i..].find('\n').map(|k| k + 1).unwrap_or(b.len() - i);
            }
            b'\\' => {
                let start = i;
                i += 1;
                let name_end = i + text[i..]
                    .bytes()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .count();
                if name_end == i {
                    // escaped character (\%, \\ …): skip it so the next byte is not a comment start
                    i = (i + 1).min(b.len());
                    continue;
                }
                let name = &text[i..name_end];
                i = name_end;
                if !matches!(name, "input" | "include" | "subfile") {
                    continue;
                }
                let mut j = name_end;
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                let (arg, end) = if j < b.len() && b[j] == b'{' {
                    let Some(close) = text[j..].find('}') else {
                        break;
                    };
                    (&text[j + 1..j + close], j + close + 1)
                } else if name == "input" && j < b.len() {
                    // plain TeX form: \input file
                    let k = j + text[j..]
                        .bytes()
                        .take_while(|c| {
                            !c.is_ascii_whitespace() && *c != b'\\' && *c != b'{' && *c != b'}'
                        })
                        .count();
                    (&text[j..k], k)
                } else {
                    continue;
                };
                i = end.max(i);
                if let Some(t) = normalize_input_target(arg) {
                    let kind = match name {
                        "input" => "input",
                        "include" => "include",
                        _ => "subfile",
                    };
                    out.push((start..end, t, kind));
                }
            }
            _ => i += 1,
        }
    }
    out
}

pub(super) fn normalize_input_target(arg: &str) -> Option<String> {
    let t = arg.trim().trim_matches('"');
    let t = t.strip_prefix("./").unwrap_or(t);
    if t.is_empty()
        || t.starts_with('/')
        || t.contains("..")
        || t.contains('\\')
        || t.contains('#')
        || t.contains(',')
    {
        return None;
    }
    let last = t.rsplit('/').next().unwrap_or(t);
    Some(if last.contains('.') {
        t.to_string()
    } else {
        format!("{t}.tex")
    })
}

/// Files `text` pulls in with `\input`/`\include`/`\subfile`, normalised to project-relative
/// paths with a `.tex` extension, in order of appearance.
pub fn input_targets(text: &str) -> Vec<String> {
    find_inputs(text).into_iter().map(|(_, t, _)| t).collect()
}

/// Files read with `\input` (not `\include`) by any of `texts`: LaTeX reads the rest of the
/// `\input` line after the file, so a paragraph that ends at such a file's end keeps one
/// interword space before `\parfillskip` (see `fast_source`).
pub fn inputted_files(
    texts: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeSet<String> {
    texts
        .values()
        .flat_map(|t| find_inputs(t))
        .filter(|(_, _, kind)| *kind == "input" || *kind == "subfile")
        .map(|(_, t, _)| t)
        .collect()
}

/// `main` plus, transitively, every file it inputs that exists under `root` (missing files are
/// left to the compiler to report), keyed by project-relative path.
pub fn load_project_files(
    root: &std::path::Path,
    main: &str,
) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    use anyhow::Context;
    let mut files = std::collections::BTreeMap::new();
    let main_text =
        std::fs::read_to_string(root.join(main)).with_context(|| format!("reading {main}"))?;
    let mut queue = input_targets(&main_text);
    files.insert(main.to_string(), main_text);
    while let Some(rel) = queue.pop() {
        if files.contains_key(&rel) || files.len() > 256 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        queue.extend(input_targets(&text));
        files.insert(rel, text);
    }
    Ok(files)
}

/// Files reachable from `text` through `\input`/`\include` chains, restricted to `files`.
pub fn transitive_inputs(
    text: &str,
    files: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = input_targets(text);
    while let Some(rel) = queue.pop() {
        if !files.contains_key(&rel) || !seen.insert(rel.clone()) {
            continue;
        }
        queue.extend(input_targets(&files[&rel]));
    }
    seen
}

/// `text` with every `\input{f}`/`\include{f}` whose target is in `files` replaced by that
/// file's text (recursively, bounded depth): the preamble the fast server loads, so it sees the
/// host's buffers of preamble files rather than what is on disk.
pub fn expand_inputs(text: &str, files: &std::collections::BTreeMap<String, String>) -> String {
    fn go(text: &str, files: &std::collections::BTreeMap<String, String>, depth: u32) -> String {
        if depth > 8 {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (range, target, _) in find_inputs(text) {
            if let Some(t) = files.get(&target) {
                out.push_str(&text[last..range.start]);
                out.push_str(&go(t, files, depth + 1));
                out.push('\n');
                last = range.end;
            }
        }
        out.push_str(&text[last..]);
        out
    }
    go(text, files, 0)
}

/// Setup statements found in the document body (see `eligibility::setup_statements`), in
/// document order: the main file's body, then the files the body reads. A file read only in
/// the preamble is not scanned: `crate::server_preamble` inlines it there already. Chunks are
/// separated by blank lines, like the segmenter's paragraph spans.
pub fn body_setup_statements(
    texts: &std::collections::BTreeMap<String, String>,
    main: &str,
) -> Vec<String> {
    // segmented like the session does it, so a definition inside an environment (a
    // \newcommand in a tikzpicture, between blank lines) is part of that environment's span
    // and never a setup statement of its own
    let mut out = Vec::new();
    let mut ids = IdAllocator(0);
    let mut scan = |text: &str| {
        let fb = FileBuf::new(text, &mut ids, 1);
        for sp in &fb.spans {
            if sp.kind != SpanKind::Body {
                continue;
            }
            let stripped = crate::eligibility::strip_comments(&fb.text[sp.range.clone()]);
            if let Some(st) = crate::eligibility::setup_statements(&stripped) {
                out.push(st);
            }
        }
    };
    if let Some(t) = texts.get(main) {
        scan(t);
        // a file read in both the preamble and the body is read again by the body
        let body = crate::split_preamble(t).map_or(t.as_str(), |(_, body)| body);
        for name in transitive_inputs(body, texts) {
            if name != main {
                scan(&texts[&name]);
            }
        }
    }
    out
}

/// The file buffers of a project (offline tools): spans are found across files by id.
#[derive(Default)]
pub struct FileSet {
    pub files: std::collections::BTreeMap<String, FileBuf>,
    /// Files read with `\input` (see `fast_source`).
    pub inputted: std::collections::BTreeSet<String>,
}

impl FileSet {
    pub fn span_text(&self, id: ParaId) -> Option<&str> {
        self.files.values().find_map(|fb| fb.span_text(id))
    }
    /// What the fast path compiles for span `id` (see `fast_source`).
    pub fn fast_source(&self, id: ParaId) -> Option<String> {
        self.files.iter().find_map(|(name, fb)| {
            fb.span(id)
                .map(|sp| fast_source(fb, sp, self.inputted.contains(name)))
        })
    }
    pub fn get(&self, rel: &str) -> Option<&FileBuf> {
        self.files.get(rel)
    }
}
