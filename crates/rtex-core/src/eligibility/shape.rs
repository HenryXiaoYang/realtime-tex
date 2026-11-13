//! What kind of unit a source is, and where its content starts.

use super::lex::*;
use super::policy::*;
use super::reason::*;
use super::setup::*;
use super::vocab_envs::*;

/// The source after a heading command and its arguments (`\subsubsection*[Short]{Title}`) that
/// starts `text` (comments stripped; setup statements and vertical material before it skipped),
/// or `None` when it does not start with a command or the title is not closed.
pub(super) fn after_heading(text: &str) -> Option<&str> {
    let text = &text[content_start(text)..];
    let b = text.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut i = skip_ws(0);
    if b.get(i) != Some(&b'\\') {
        return None;
    }
    i += 1;
    while i < b.len() && is_letter(b[i]) {
        i += 1;
    }
    i = skip_ws(i);
    if b.get(i) == Some(&b'*') {
        i = skip_ws(i + 1);
    }
    if b.get(i) == Some(&b'[') {
        i = skip_ws(group_end(b, i)?);
    }
    if b.get(i) != Some(&b'{') {
        return None;
    }
    Some(&text[group_end(b, i)?..])
}

/// Is `src` (comments allowed) nothing but `\label{…}`s and material `has_content` skips?
pub fn labels_only(src: &str) -> bool {
    let text = strip_comments(src);
    let mut rest = text.as_str();
    loop {
        let t = rest.trim_start();
        match t
            .strip_prefix("\\label")
            .map(str::trim_start)
            .filter(|r| r.starts_with('{'))
            .and_then(|r| Some(&r[group_end(r.as_bytes(), 0)?..]))
        {
            Some(r) => rest = r,
            None => return !has_content(t),
        }
    }
}

/// Does the heading that starts `src` (comments allowed) have material after its title other
/// than labels: for a run-in heading, the paragraph or list that sets the title?
pub fn runs_into_title(src: &str) -> bool {
    after_heading(&strip_comments(src)).is_some_and(|rest| !labels_only(rest))
}

/// The unit's shape: a block/float/theorem environment spanning the whole unit (possibly after
/// setup statements and vertical material such as \vspace or \noindent, which the unit box
/// absorbs), a heading, or a paragraph. Also returns where the unit's own heading command may
/// start (a heading macro further in is not the unit's heading).
pub(super) fn unit_shape(text: &str, policy: &Policy) -> (UnitShape, usize) {
    let trimmed = text[setup_prefix(text).0..].trim();
    let mut shape = UnitShape::Par;
    let env_start = vertical_prefix_len(trimmed);
    if let Some((name, _)) = env_name(trimmed, env_start, "\\begin") {
        let end = format!("\\end{{{name}}}");
        if trimmed.ends_with(&end) && policy.is_block_env(&name) {
            shape = UnitShape::Env(name);
        }
    } else if trimmed[env_start..].starts_with('\\') {
        let b = &trimmed.as_bytes()[env_start..];
        let mut j = 1;
        while j < b.len() && is_letter(b[j]) {
            j += 1;
        }
        let name = &trimmed[env_start + 1..env_start + j];
        if HEADINGS.contains(&name) {
            shape = UnitShape::Heading(name.to_string());
        } else if let Some(h) = policy.heading_macros.get(name) {
            shape = UnitShape::Heading(h.clone());
        }
    }
    (shape, env_start + text.len() - trimmed.len())
}

/// Length of the vertical material at the start of `s` (`\vspace`, `\noindent`, `\centering`,
/// page breaks …), which a unit's box absorbs without typesetting anything.
pub fn vertical_prefix_len(trimmed: &str) -> usize {
    let mut k = 0;
    loop {
        let rest = trimmed[k..].trim_start();
        let off = trimmed.len() - rest.len();
        let mut matched = None;
        for v in [
            "\\vspace*",
            "\\vspace",
            "\\noindent",
            "\\newpage",
            "\\clearpage",
            "\\pagebreak",
            "\\smallskip",
            "\\medskip",
            "\\bigskip",
            "\\vfill",
            "\\centering",
        ] {
            if rest.starts_with(v)
                && !rest[v.len()..].starts_with(|c: char| c.is_ascii_alphabetic())
            {
                let mut e = v.len();
                if rest[e..].starts_with('{') {
                    if let Some(c) = rest[e..].find('}') {
                        e += c + 1;
                    }
                }
                matched = Some(off + e);
                break;
            }
        }
        match matched {
            Some(e) => k = e,
            None => break off,
        }
    }
}

/// Offset of the first substantive content of a unit's (comment-stripped) text: after setup
/// statements and vertical material. A block environment starting there is the unit; one
/// starting after other content is an inline box of a paragraph.
pub fn content_start(text: &str) -> usize {
    let (k, _) = setup_prefix(text);
    let rest = &text[k..];
    let ws = rest.len() - rest.trim_start().len();
    k + ws + vertical_prefix_len(rest.trim_start())
}

/// Does `src` (comments allowed) contain substantive content, i.e. anything a paragraph would
/// be open for, beyond setup statements and vertical material?
pub fn has_content(src: &str) -> bool {
    let text = strip_comments(src);
    content_start(&text) < text.trim_end().len()
}
