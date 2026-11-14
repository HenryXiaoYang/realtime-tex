//! Finding the picture environments of a document, in the order TeX reads them, each with the hash of what can change its rendering.

use super::cacheable::*;
use super::{hash_bytes, PictureRef, PICTURE_ENVS};
use std::collections::BTreeMap;

/// Body-level statements whose effect a later picture may depend on (hashed into every
/// picture after them, the whole statement when it spans lines).
pub(super) const PRELUDE_HEADS: &[&str] = &[
    "def",
    "edef",
    "gdef",
    "xdef",
    "let",
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "tikzset",
    "tikzstyle",
    "pgfplotsset",
    "pgfkeys",
    "definecolor",
    "colorlet",
    "setlength",
    "newlength",
    "pgfmathsetmacro",
    "pgfmathsetlengthmacro",
    "pgfdeclare",
    "ctikzset",
    "usetikzlibrary",
    "usepgfplotslibrary",
    "newcolumntype",
    "linespread",
    "selectfont",
    "pgfdeclarelayer",
    "pgfsetlayers",
];

/// Number of lines, from `lines[0]`, a statement spans: until its braces balance (a `\tikzset{`
/// block over several lines). At least 1, at most 64.
pub(super) fn statement_lines(lines: &[&str]) -> usize {
    let mut depth = 0i32;
    for (n, line) in lines.iter().enumerate().take(64) {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth <= 0 {
            return n + 1;
        }
    }
    lines.len().clamp(1, 64)
}

/// Every picture environment in the project's body text, with its hash (preamble hash, the
/// definitions made before it, its own text) and whether it may be cached.
pub fn scan_pictures(
    texts: &BTreeMap<String, String>,
    main: &str,
    preamble_hash: u64,
) -> Vec<PictureRef> {
    use crate::document::strip_comment;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let body_start = |name: &str| -> usize {
        if name != main {
            return 0;
        }
        // the line after the \begin{document} TeX reads (not one in a comment or verbatim)
        let text = &texts[name];
        crate::document::find_command(text, "\\begin{document}")
            .map(|off| text[..off].matches('\n').count() + 1)
            .unwrap_or(0)
    };
    // `remember picture` anywhere (comments aside; the preamble too: `\tikzset{every
    // picture/.style={remember picture}}`) disables the cache: such pictures place material
    // relative to others
    let has_remember = texts.values().any(|text| {
        text.lines()
            .any(|l| strip_comment(l).contains("remember picture"))
    });
    struct Walker<'a> {
        texts: &'a BTreeMap<String, String>,
        prelude: DefaultHasher,
        out: Vec<PictureRef>,
        has_remember: bool,
        stack: Vec<String>,
        /// How often each file was read: a file `\input` twice yields two pictures per
        /// `file:line` key, which the capture cannot tell apart.
        visits: BTreeMap<String, u32>,
    }
    impl Walker<'_> {
        fn walk(&mut self, name: &str, start: usize) {
            if self.stack.len() > 8 || self.stack.iter().any(|s| s == name) {
                return;
            }
            *self.visits.entry(name.to_string()).or_default() += 1;
            self.stack.push(name.to_string());
            let text = &self.texts[name];
            let lines: Vec<&str> = text.lines().collect();
            let mut i = start;
            while i < lines.len() {
                let line = strip_comment(lines[i]);
                let trimmed = line.trim_start();
                let opened = PICTURE_ENVS
                    .iter()
                    .find(|e| trimmed.contains(&format!("\\begin{{{e}}}")));
                if let Some(env) = opened {
                    let begin_marker = format!("\\begin{{{env}}}");
                    let end_marker = format!("\\end{{{env}}}");
                    let start = i;
                    let mut depth = 0i32;
                    let mut end = None;
                    let mut j = i;
                    while j < lines.len() {
                        let l = strip_comment(lines[j]);
                        depth += l.matches(&begin_marker).count() as i32;
                        depth -= l.matches(&end_marker).count() as i32;
                        if depth <= 0 {
                            end = Some(j);
                            break;
                        }
                        j += 1;
                    }
                    let Some(end) = end else { break };
                    let body: Vec<&str> = lines[start..=end]
                        .iter()
                        .map(|l| strip_comment(l))
                        .collect();
                    let text_all = body.join("\n");
                    // the capture keys a picture by the line its \begin executes on, which is
                    // this line only when nothing precedes the \begin on it (a macro argument
                    // closing here would execute its pictures with this line number)
                    let cacheable = !self.has_remember
                        && trimmed.starts_with(&begin_marker)
                        && text_all.matches(&begin_marker).count() == 1
                        && !line.contains("\\end{")
                        && !uncacheable(&text_all)
                        && skip_stops_at_end(&text_all, env);
                    let mut h = self.prelude.clone();
                    hash_bytes(&mut h, &text_all);
                    self.out.push(PictureRef {
                        key: format!("{}:{}", name.trim_start_matches("./"), start + 1),
                        env: env.to_string(),
                        hash: h.finish(),
                        cacheable,
                        end_line: end + 1,
                    });
                    i = end + 1;
                    continue;
                }
                // files read here come before everything after this line
                for (_, target, _) in crate::document::find_inputs(line) {
                    if self.texts.contains_key(&target) {
                        self.walk(&target, 0);
                    }
                }
                if PRELUDE_HEADS.iter().any(|h| has_cs(line, h)) {
                    let n = statement_lines(&lines[i..]);
                    for l in &lines[i..i + n] {
                        hash_bytes(&mut self.prelude, strip_comment(l).trim());
                    }
                    i += n;
                    continue;
                }
                i += 1;
            }
            self.stack.pop();
        }
    }
    let mut prelude = DefaultHasher::new();
    prelude.write_u64(preamble_hash);
    let mut w = Walker {
        texts,
        prelude,
        out: Vec::new(),
        has_remember,
        stack: Vec::new(),
        visits: BTreeMap::new(),
    };
    if texts.contains_key(main) {
        w.walk(main, body_start(main));
    }
    let twice: Vec<&str> = w
        .visits
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(f, _)| f.trim_start_matches("./"))
        .collect();
    if !twice.is_empty() {
        for p in &mut w.out {
            if p.key
                .rsplit_once(':')
                .is_some_and(|(f, _)| twice.contains(&f))
            {
                p.cacheable = false;
            }
        }
    }
    w.out
}
