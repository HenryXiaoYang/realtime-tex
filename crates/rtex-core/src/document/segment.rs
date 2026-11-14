//! Segmentation: a file's text into spans (preamble, units, the rest), and the source the live engine typesets for one span.

use super::lex::*;
use super::*;
use std::ops::Range;

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
/// Returns false when the slice ends inside an unclosed environment or with a run-in heading
/// still waiting for its title's paragraph (the caller must then re-segment the whole file,
/// since the environment swallows everything that follows, and the heading what comes next).
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
    let is_run_in = |line: &str| {
        let name = line[1..]
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("");
        rules.run_in.iter().any(|h| h == name)
    };
    let body = &text[body_start..body_end];
    let lines: Vec<(usize, &str)> = {
        let mut v = Vec::new();
        let mut p = 0;
        for l in body.split_inclusive('\n') {
            v.push((p, l));
            p += l.len();
        }
        v
    };
    let flush = |out: &mut Vec<(Range<usize>, SpanKind)>, s: usize, e: usize, k: SpanKind| {
        if e > s && !body[s..e].trim().is_empty() {
            out.push((body_start + s..body_start + e, k));
        }
    };
    let mut cur_start: Option<usize> = None;
    let mut env_stack: Vec<String> = Vec::new();
    let mut env_inline: Vec<bool> = Vec::new(); // parallel to env_stack: an inline picture
    let mut cur_kind = SpanKind::Body;
    // the current span ends before the next non-blank line: after a heading's braces closed or
    // after a block environment closed (the capture closes the unit at those points)
    let mut split_pending = false;
    let mut heading_balance: i32 = 0;
    // the heading's title braces are not closed yet
    let mut heading_open = false;
    // a run-in heading's span: `title_pending` until the paragraph or environment that sets
    // its title begins, then `run_in_body` (a heading line ends that body like a paragraph)
    let mut run_in = false;
    let mut title_pending = false;
    let mut run_in_body = false;
    for (lstart, line) in &lines {
        let trimmed = line.trim();
        let stripped = strip_comment(trimmed);
        let is_blank = stripped.trim().is_empty() && !trimmed.starts_with('%');
        // environment tracking (only at line starts, which covers the common layout)
        let begins: Vec<String> = if stripped.contains("\\begin{") {
            find_all_envs(stripped, "\\begin{")
        } else {
            Vec::new()
        };
        let ends: Vec<String> = if stripped.contains("\\end{") {
            find_all_envs(stripped, "\\end{")
        } else {
            Vec::new()
        };
        let heading = HEADING_CMDS.iter().any(|h| stripped.starts_with(h));
        // a picture environment opened while a paragraph is open (text before it in this span
        // or on its line) is an inline box of that paragraph, as the capture sees it (no unit
        // of its own); the same rule as eligibility/shape.rs (content_start before the \begin)
        let inline_picture = begins.iter().any(|b| PICTURE_ENVS.contains(&b.as_str())) && {
            let k = PICTURE_ENVS
                .iter()
                .filter_map(|e| stripped.find(&format!("\\begin{{{e}}}")))
                .min()
                .unwrap_or(0);
            let paragraph_open = env_stack.is_empty()
                && !split_pending
                && cur_kind == SpanKind::Body
                && cur_start.is_some();
            let mut before = String::new();
            if paragraph_open {
                before.push_str(&body[cur_start.unwrap()..*lstart]);
            }
            before.push_str(&stripped[..k]);
            crate::eligibility::has_content(&before)
        };
        if env_stack.is_empty() {
            if is_blank && trimmed.is_empty() {
                if title_pending {
                    // the run-in title goes into the paragraph after the blank line
                    continue;
                }
                if let Some(s) = cur_start.take() {
                    flush(out, s, *lstart, cur_kind);
                }
                split_pending = false;
                continue;
            }
            let starts_new = cur_start.is_none()
                || (heading && (cur_kind == SpanKind::Body || run_in_body || title_pending))
                || split_pending;
            if starts_new {
                // a heading command starts a new unit even without a blank line, and a unit ends
                // after a heading or a block environment
                if let Some(s) = cur_start {
                    flush(out, s, *lstart, cur_kind);
                }
                cur_start = Some(*lstart);
                cur_kind = if heading {
                    SpanKind::Heading
                } else {
                    SpanKind::Body
                };
                heading_balance = 0;
                heading_open = heading;
                run_in = heading && is_run_in(stripped);
                title_pending = false;
                run_in_body = false;
                split_pending = false;
            } else if title_pending && !crate::eligibility::labels_only(stripped) {
                title_pending = false;
                run_in_body = true;
            }
            if !run_in
                && begins
                    .iter()
                    .any(|b| !(inline_picture && PICTURE_ENVS.contains(&b.as_str())))
            {
                cur_kind = SpanKind::Env;
                heading_open = false;
            }
            if heading_open {
                heading_balance += brace_balance(stripped);
                if heading_balance <= 0 || !begins.is_empty() {
                    heading_open = false;
                    if !run_in {
                        split_pending = begins.is_empty();
                    } else if begins.is_empty()
                        && !crate::eligibility::runs_into_title(
                            &body[cur_start.unwrap()..*lstart + line.len()],
                        )
                    {
                        title_pending = true;
                    } else {
                        run_in_body = true;
                    }
                }
            }
        }
        for b in begins {
            let inline = inline_picture && PICTURE_ENVS.contains(&b.as_str());
            env_stack.push(b);
            env_inline.push(inline);
        }
        for e in ends {
            if let Some(idx) = env_stack.iter().rposition(|x| *x == e) {
                let inline = env_inline[idx];
                env_stack.truncate(idx);
                env_inline.truncate(idx);
                if env_stack.is_empty() && is_block(&e) && !inline {
                    split_pending = true;
                }
            }
        }
    }
    if let Some(s) = cur_start {
        flush(out, s, body.len(), cur_kind);
    }
    env_stack.is_empty() && !title_pending
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
