//! Source-text helpers: comments, groups, environment names, arguments.

/// Strip `%` comments (respecting `\%`). Like TeX, a comment also removes the end of its
/// line, so comment-only lines inside a paragraph do not leave a blank line behind.
pub fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut cut = line.len();
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                // \verb<d>...<d> (and \lstinline): a % inside is literal
                for key in ["\\verb*", "\\verb", "\\lstinline"] {
                    if line[i..].starts_with(key)
                        && i + key.len() < bytes.len()
                        && !(bytes[i + key.len()] as char).is_ascii_alphabetic()
                    {
                        let d = bytes[i + key.len()];
                        let close = if d == b'{' { b'}' } else { d };
                        let start = i + key.len() + 1;
                        if let Some(e) = bytes[start..].iter().position(|&c| c == close) {
                            i = start + e + 1;
                        } else {
                            i = bytes.len();
                        }
                        break;
                    }
                }
                if i >= bytes.len() || bytes[i] != b'\\' {
                    continue;
                }
                i += 2;
                continue;
            }
            if bytes[i] == b'%' {
                cut = i;
                break;
            }
            i += 1;
        }
        out.push_str(&line[..cut]);
        if cut == line.len() {
            out.push('\n');
        }
    }
    out
}

pub(super) fn is_letter(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'@'
}

/// Parse `\begin{name}` / `\end{name}` at `text[i..]` (i at the backslash); returns (name, index after `}`).
pub(super) fn env_name(text: &str, i: usize, what: &str) -> Option<(String, usize)> {
    let rest = &text[i..];
    let rest2 = rest.strip_prefix(what)?;
    let rest3 = rest2.trim_start();
    let body = rest3.strip_prefix('{')?;
    let e = body.find('}')?;
    let name = body[..e].trim().to_string();
    let consumed = rest.len() - body.len() + e + 1;
    Some((name, i + consumed))
}

/// The position after the `\end{env}` that closes an environment whose body starts at `from`
/// (`\begin{env}` already consumed), counting nested environments of the same name. None when
/// the environment is not closed.
pub(super) fn skip_env_body(text: &str, from: usize, env: &str) -> Option<usize> {
    let begin = format!("\\begin{{{env}}}");
    let end = format!("\\end{{{env}}}");
    let mut depth = 1usize;
    let mut i = from;
    loop {
        let e = text[i..].find(&end)?;
        let b = text[i..].find(&begin);
        match b {
            Some(b) if b < e => {
                depth += 1;
                i += b + begin.len();
            }
            _ => {
                depth -= 1;
                i += e + end.len();
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    }
}

/// The index after the `{…}` or `[…]` group that opens at `b[i]` (`\` escapes a character).
pub(super) fn group_end(b: &[u8], i: usize) -> Option<usize> {
    let bracket = b[i] == b'[';
    let mut depth = i32::from(!bracket);
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 1,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if !bracket && depth == 0 {
                    return Some(j + 1);
                }
            }
            b']' if bracket && depth == 0 => return Some(j + 1),
            _ => {}
        }
        j += 1;
    }
    None
}

/// The optional argument `[...]` at the start of `s` (after spaces), without its brackets;
/// brackets inside braces (`label={[\arabic*]}`) do not end it.
pub(super) fn optional_arg(s: &str) -> Option<&str> {
    let t = s.trim_start();
    let off = s.len() - t.len();
    if !t.starts_with('[') {
        return None;
    }
    let b = t.as_bytes();
    let mut depth = 0i32;
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b']' if depth == 0 => return Some(&s[off + 1..off + i]),
            _ => {}
        }
        i += 1;
    }
    None
}

/// `s` split at `sep` outside braces.
pub(super) fn top_level_split(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}
