//! Source-text helpers for segmentation: comments, verbatim, commands, environments.

pub(super) fn brace_balance(line: &str) -> i32 {
    let b = line.as_bytes();
    let mut bal = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'{' => bal += 1,
            b'}' => bal -= 1,
            _ => {}
        }
        i += 1;
    }
    bal
}

/// Strip an unescaped `%` comment from a source line.
pub fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b'%' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// Does TeX read the body of environment `name` verbatim (`verbatim`, `Verbatim`,
/// `lstlisting`, `minted`, `comment`, `luacode*`, `filecontents` …), so that a `\begin{document}`
/// or a `%` in it is text?
pub(crate) fn is_verbatim_env(name: &str) -> bool {
    let n = name.trim_end_matches('*').to_ascii_lowercase();
    n == "comment"
        || n == "alltt"
        || ["verbatim", "listing", "minted", "luacode", "filecontents"]
            .iter()
            .any(|p| n.contains(p))
}

/// `line` with the argument of every `\verb<c>…<c>` (and `\verb*`) blanked, delimiters
/// included, byte for byte (offsets are kept).
pub(super) fn blank_verb(line: &str) -> String {
    let mut b = line.as_bytes().to_vec();
    let mut i = 0;
    while let Some(k) = line[i..].find("\\verb") {
        let mut j = i + k + "\\verb".len();
        if j < b.len() && b[j].is_ascii_alphabetic() {
            // a longer control word (\verbatim)
            i = j;
            continue;
        }
        if j < b.len() && b[j] == b'*' {
            j += 1;
        }
        let Some(delim) = line[j..].chars().next() else {
            break;
        };
        let body = j + delim.len_utf8();
        let end = line[body..]
            .find(delim)
            .map(|e| body + e + delim.len_utf8())
            .unwrap_or(line.len());
        for x in &mut b[j..end] {
            *x = b' ';
        }
        i = end;
    }
    // only whole characters were replaced by spaces
    String::from_utf8(b).unwrap_or_else(|_| line.to_string())
}

/// Byte offset of the first `marker` (`\begin{document}`, `\end{document}`) that TeX reads as
/// a command: not in a `%` comment, a `\verb` or the body of a verbatim-like environment (a
/// chapter that shows a whole document in `verbatim`, a `\verb|\end{document}|`).
pub(crate) fn find_command(text: &str, marker: &str) -> Option<usize> {
    // the first verbatim-like `\begin{env}` in `vis`: (offset, env, length of the \begin)
    fn verbatim_begin(vis: &str) -> Option<(usize, String, usize)> {
        let mut from = 0;
        while let Some(k) = vis[from..].find("\\begin{") {
            let at = from + k;
            let name_start = at + "\\begin{".len();
            let close = vis[name_start..].find('}')?;
            let env = &vis[name_start..name_start + close];
            if is_verbatim_env(env) {
                return Some((at, env.to_string(), name_start + close + 1 - at));
            }
            from = name_start;
        }
        None
    }
    let mut verb_env: Option<String> = None;
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        let start = off;
        off += line.len();
        // a cursor over the raw line: outside verbatim bodies the line is read as TeX reads it
        // (no comments, no \verb arguments); inside one only its \end{env} counts
        let mut pos = 0;
        loop {
            if let Some(env) = verb_env.take() {
                let end = format!("\\end{{{env}}}");
                match line[pos..].find(&end) {
                    Some(k) => pos += k + end.len(),
                    None => {
                        verb_env = Some(env);
                        break;
                    }
                }
            }
            let visible = blank_verb(&line[pos..]);
            let visible = strip_comment(&visible);
            let m = visible.find(marker);
            match (m, verbatim_begin(visible)) {
                (Some(mk), Some((bk, _, _))) if mk < bk => return Some(start + pos + mk),
                (Some(mk), None) => return Some(start + pos + mk),
                (_, Some((bk, env, blen))) => {
                    pos += bk + blen;
                    verb_env = Some(env);
                }
                (None, None) => break,
            }
        }
    }
    None
}

pub(super) fn find_all_envs(line: &str, prefix: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut idx = 0;
    while let Some(p) = line[idx..].find(prefix) {
        let s = idx + p + prefix.len();
        if let Some(e) = line[s..].find('}') {
            v.push(line[s..s + e].to_string());
            idx = s + e;
        } else {
            break;
        }
    }
    v
}
