//! The part of the live engine's TeX log worth showing when it fails.

use std::path::Path;

/// The first TeX or Lua error in a log (its message and the lines up to the `l.<n>` line), else
/// its last lines; at most 400 characters on one line. None when there is no log.
pub(super) fn tex_log_excerpt(log: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log).ok()?;
    // MiKTeX ignores max_print_line and wraps at 79 characters: join wrapped lines first
    let mut joined: Vec<String> = Vec::new();
    let mut wrapped = false;
    for l in text.lines() {
        match joined.last_mut() {
            Some(prev) if wrapped => prev.push_str(l),
            _ => joined.push(l.to_string()),
        }
        wrapped = l.chars().count() == 79;
    }
    let lines: Vec<&str> = joined.iter().map(|l| l.trim_end()).collect();
    // a TeX error starts with "!"; LuaTeX reports a Lua error as "<file>:<line>: message"
    // followed by "stack traceback:", with no "!"
    let first = lines.iter().enumerate().position(|(i, l)| {
        l.starts_with('!')
            || lines
                .get(i + 1)
                .is_some_and(|n| n.starts_with("stack traceback:"))
    });
    let picked: Vec<&str> = match first {
        Some(i) => {
            let end = lines[i..]
                .iter()
                .position(|l| l.starts_with("l."))
                .map(|j| i + j + 1)
                .unwrap_or(lines.len())
                .min(i + 6);
            lines[i..end].to_vec()
        }
        None => {
            let tail: Vec<&str> = lines
                .iter()
                .rev()
                .filter(|l| !l.is_empty())
                .take(3)
                .copied()
                .collect();
            tail.into_iter().rev().collect()
        }
    };
    let mut out = picked.join(" / ");
    if first.is_none() && text.to_ascii_lowercase().contains("no pages of output") {
        // no error, yet the run ended: a \stop or \end{document} before \begin{document}
        out = "the preamble ended the run before \\begin{document} (no page was written)".into();
    }
    if out.chars().count() > 400 {
        out = out.chars().take(400).collect::<String>() + "…";
    }
    (!out.is_empty()).then_some(out)
}
