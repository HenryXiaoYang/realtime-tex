//! Segmentation: a file's text into spans (preamble, units, the rest), and the source the live engine typesets for one span.

use super::lex::*;
use super::{FileBuf, SegmentRules, Span, SpanKind, BLOCK_ENVS, HEADING_CMDS};
use crate::piccache::PICTURE_ENVS;
use std::ops::Range;

fn heading_name(source: &str) -> Option<&str> {
    let command = source.strip_prefix('\\')?;
    let name = command.split(|c: char| !c.is_ascii_alphabetic()).next()?;
    HEADING_CMDS
        .contains(&&source[..name.len() + 1])
        .then_some(name)
}

enum EnvCommand<'a> {
    Begin(usize, &'a str),
    End(&'a str),
}

/// Scan a logical line until the next outer heading or the physical line ending.
fn scan_line<'a>(
    line: &'a str,
    env_stack: &[(&str, bool)],
    mut depth: usize,
) -> (usize, usize, Vec<EnvCommand<'a>>) {
    let mut commands = Vec::new();
    if !line.bytes().any(|b| matches!(b, b'\\' | b'{' | b'}')) {
        return (line.len(), depth, commands);
    }
    let trimmed_start = line.len() - line.trim_start().len();
    let mut envs: Vec<&str> = env_stack.iter().map(|&(name, _)| name).collect();
    let mut cursor = 0;
    while cursor < line.len() {
        let rest = &line[cursor..];
        if let Some(env) = envs.last().filter(|env| is_verbatim_env(env)) {
            let end = format!("\\end{{{env}}}");
            let Some(pos) = rest.find(&end) else { break };
            let name_start = cursor + pos + "\\end{".len();
            commands.push(EnvCommand::End(&line[name_start..name_start + env.len()]));
            cursor += pos + end.len();
            envs.pop();
            continue;
        }
        let ch = rest.chars().next().unwrap();
        match ch {
            '%' => break,
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '\\' => {
                let word_len = rest[1..]
                    .bytes()
                    .take_while(u8::is_ascii_alphabetic)
                    .count();
                if word_len == 0 {
                    // Consume a control symbol, including an escaped backslash, brace or percent.
                    cursor += rest[1..].chars().next().map_or(1, |c| 1 + c.len_utf8());
                    continue;
                }
                let word_end = 1 + word_len;
                if cursor > trimmed_start
                    && depth == 0
                    && envs.is_empty()
                    && HEADING_CMDS.contains(&&rest[..word_end])
                {
                    return (cursor, depth, commands);
                }
                let word = &rest[1..word_end];
                let tail = &rest[word_end..];
                if matches!(word, "begin" | "end") {
                    if let Some((name, after)) =
                        tail.strip_prefix('{').and_then(|arg| arg.split_once('}'))
                    {
                        if word == "begin" {
                            commands.push(EnvCommand::Begin(cursor, name));
                            envs.push(name);
                        } else {
                            commands.push(EnvCommand::End(name));
                            if let Some(i) = envs.iter().rposition(|e| *e == name) {
                                envs.truncate(i);
                            }
                        }
                        cursor = line.len() - after.len();
                        continue;
                    }
                }
                if word == "verb" {
                    let arg = tail.strip_prefix('*').unwrap_or(tail);
                    if let Some(delimiter) = arg.chars().next() {
                        let inner = &arg[delimiter.len_utf8()..];
                        if let Some(end) = inner.find(delimiter) {
                            cursor = line.len() - inner.len() + end + delimiter.len_utf8();
                            continue;
                        }
                    }
                    break;
                }
                cursor += word_end;
                continue;
            }
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    (line.len(), depth, commands)
}

pub(super) fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

/// Boundaries of paragraph-ish units in `text`, as byte ranges with kinds (ids not assigned).
pub(super) fn segment(text: &str, rules: &SegmentRules) -> Vec<(Range<usize>, SpanKind)> {
    let mut out: Vec<(Range<usize>, SpanKind)> = Vec::new();
    let begin_doc = find_command(text, "\\begin{document}");
    let end_doc = find_command(text, "\\end{document}");
    let body_start = match begin_doc {
        Some(i) => {
            let e = i + "\\begin{document}".len();
            // preamble includes the \begin{document} line
            let e = text[e..]
                .find('\n')
                .map(|k| e + k + 1)
                .unwrap_or(text.len());
            out.push((0..e, SpanKind::Preamble));
            e
        }
        None => 0,
    };
    let body_end = end_doc.filter(|e| *e >= body_start).unwrap_or(text.len());
    let _ = segment_body(text, body_start, body_end, &mut out, rules);
    if body_end < text.len() {
        out.push((body_end..text.len(), SpanKind::Trailer));
    }
    out
}

/// Segment `text[body_start..body_end]` (body material only) appending absolute ranges to `out`.
/// Returns false when the slice ends inside an open environment or group, or with a run-in
/// heading still waiting for its title's paragraph. The caller must then re-segment the whole file.
///
/// Long on purpose: it is one pass over the lines with its state in locals (the open span, the
/// environment stack, a pending split, a heading whose braces or run-in title are still open),
/// and every line reads and updates several of them. Splitting it would pass that state around.
pub(super) fn segment_body(
    text: &str,
    body_start: usize,
    body_end: usize,
    out: &mut Vec<(Range<usize>, SpanKind)>,
    rules: &SegmentRules,
) -> bool {
    let is_block =
        |name: &str| BLOCK_ENVS.contains(&name) || rules.block_envs.iter().any(|e| e == name);
    let body = &text[body_start..body_end];
    let flush = |out: &mut Vec<(Range<usize>, SpanKind)>, s: usize, e: usize, k: SpanKind| {
        if e > s && !body[s..e].trim().is_empty() {
            out.push((body_start + s..body_start + e, k));
        }
    };
    let mut cur_start: Option<usize> = None;
    let mut env_stack = Vec::new(); // (environment name, inline picture)
    let mut cur_kind = SpanKind::Body;
    // the current span ends before the next non-blank line: after a heading's braces closed or
    // after a block environment closed (the capture closes the unit at those points)
    let mut split_pending = false;
    let mut group_depth = 0;
    // the heading's title braces are not closed yet
    let mut heading_open = false;
    // A run-in heading's span continues until the paragraph or environment that sets its title.
    let mut run_in = false;
    let mut title_pending = false;
    let mut lstart = 0;
    let mut lines = body.split_inclusive('\n');
    let mut remaining = "";
    loop {
        if remaining.is_empty() {
            let Some(line) = lines.next() else { break };
            remaining = line;
        }
        if env_stack.is_empty() && remaining.trim().is_empty() {
            // a blank line ends any group the scan still counts open: an argument cannot hold a
            // \par, so the count went wrong (`%` read as a comment inside \url{...})
            group_depth = 0;
        }
        let outer = group_depth == 0 && env_stack.is_empty();
        let (cut, depth, commands) = scan_line(remaining, &env_stack, group_depth);
        group_depth = depth;
        let (line, rest) = remaining.split_at(cut);
        remaining = rest;
        let start = lstart;
        lstart += cut;
        let trimmed = line.trim();
        let stripped = strip_comment(trimmed);
        let begins = || {
            commands.iter().filter_map(|command| match *command {
                EnvCommand::Begin(pos, name) => Some((pos, name)),
                EnvCommand::End(_) => None,
            })
        };
        let has_begin = begins().next().is_some();
        let heading = heading_name(stripped);
        // a picture environment opened while a paragraph is open (text before it in this span
        // or on its line) is an inline box of that paragraph, as the capture sees it (no unit
        // of its own); the same rule as eligibility/shape.rs (content_start before the \begin)
        let inline_picture = begins()
            .find(|&(_, name)| PICTURE_ENVS.contains(&name))
            .is_some_and(|(pos, _)| {
                let paragraph_open = env_stack.is_empty()
                    && !split_pending
                    && cur_kind == SpanKind::Body
                    && cur_start.is_some();
                let before = if paragraph_open {
                    cur_start.unwrap()
                } else {
                    start
                };
                crate::eligibility::has_content(&body[before..start + pos])
            });
        if outer || heading_open {
            if trimmed.is_empty() {
                if title_pending || heading_open {
                    // An open heading or a pending run-in title continues after the blank.
                    continue;
                }
                if let Some(s) = cur_start.take() {
                    flush(out, s, start, cur_kind);
                }
                split_pending = false;
                continue;
            }
            let starts_new =
                cur_start.is_none() || (heading.is_some() && !heading_open) || split_pending;
            if starts_new {
                // a heading command starts a new unit even without a blank line, and a unit ends
                // after a heading or a block environment
                if let Some(s) = cur_start {
                    flush(out, s, start, cur_kind);
                }
                cur_start = Some(start);
                cur_kind = if heading.is_some() {
                    SpanKind::Heading
                } else {
                    SpanKind::Body
                };
                heading_open = heading.is_some();
                run_in = heading.is_some_and(|name| rules.run_in.iter().any(|h| h == name));
                title_pending = false;
                split_pending = false;
            } else if title_pending && !crate::eligibility::labels_only(stripped) {
                title_pending = false;
            }
            if !run_in
                && begins().any(|(_, name)| !(inline_picture && PICTURE_ENVS.contains(&name)))
            {
                cur_kind = SpanKind::Env;
                heading_open = false;
            }
            if heading_open && (group_depth == 0 || has_begin) {
                heading_open = false;
                if !run_in {
                    split_pending = !has_begin;
                } else {
                    title_pending = !has_begin
                        && !crate::eligibility::runs_into_title(&body[cur_start.unwrap()..lstart]);
                }
            }
        }
        for command in commands {
            match command {
                EnvCommand::Begin(_, name) => {
                    env_stack.push((name, inline_picture && PICTURE_ENVS.contains(&name)));
                }
                EnvCommand::End(name) => {
                    if let Some(idx) = env_stack.iter().rposition(|&(env, _)| env == name) {
                        let (_, inline) = env_stack[idx];
                        env_stack.truncate(idx);
                        if env_stack.is_empty() && is_block(name) && !inline {
                            split_pending = true;
                        }
                    }
                }
            }
        }
    }
    if let Some(s) = cur_start {
        flush(out, s, body.len(), cur_kind);
    }
    env_stack.is_empty() && group_depth == 0 && !title_pending
}

/// The source the fast path typesets for a span: its text without the trailing newline, plus
/// `\n{}` when the span ends a file read with `\input` without a final blank line. At that
/// point TeX produces a second end-of-line space (the parent's `\input` line), and `\par`
/// removes only the last glue, so the paragraph keeps one trailing space; `{}` on a new line
/// reproduces that space token exactly (same space factor), and nothing else. A span that ends
/// with a block environment (a float, a list, a theorem) gets no `{}`: TeX is in vertical mode
/// there and drops the space, and the unit must end with the environment's `\end` to be
/// classified as that environment (`eligibility::unit_shape`).
pub fn fast_source(fb: &FileBuf, span: &Span, file_is_inputted: bool) -> String {
    let text = fb.text[span.range.clone()].trim_end_matches('\n');
    let mut src = text.to_string();
    if file_is_inputted && span.kind != SpanKind::Trailer && !text.trim().is_empty() {
        let rest = &fb.text[span.range.end..];
        let own = &fb.text[span.range.clone()];
        let own_tail = &own[own.trim_end().len()..];
        let ends_file = rest.trim().is_empty()
            && own_tail.matches('\n').count() + rest.matches('\n').count() <= 1;
        if ends_file && !ends_with_block_env(fb, text) {
            src.push_str("\n{}");
        }
    }
    src
}

/// The last non-comment line of `text` ends with `\end{…}` of a block environment.
pub(super) fn ends_with_block_env(fb: &FileBuf, text: &str) -> bool {
    text.lines()
        .map(|l| strip_comment(l).trim())
        .rfind(|l| !l.is_empty())
        .and_then(|l| l.strip_suffix('}')?.rsplit_once("\\end{"))
        .is_some_and(|(_, name)| {
            BLOCK_ENVS.contains(&name) || fb.rules.block_envs.iter().any(|e| e == name)
        })
}
