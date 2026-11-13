//! Which pictures may be cached: a picture whose source mentions anything in these lists depends on more than its text and what precedes it.

/// Control words that make a picture depend on more than its own source and the definitions
/// before it (matched as whole words: `\ref`, not `\reflectbox`).
pub(super) const UNCACHEABLE_WORDS: &[&str] = &[
    // the aux file
    "label",
    "ref",
    "pageref",
    "eqref",
    "footnotemark",
    "footnotetext",
    // links and anchors (not in the page content an image carries)
    "href",
    "hyperref",
    "hyperlink",
    "hypertarget",
    "url",
    "autoref",
    "cref",
    "Cref",
    "vref",
    "nameref",
    // counters, dates
    "the",
    "value",
    "arabic",
    "roman",
    "Roman",
    "alph",
    "Alph",
    "fnsymbol",
    "today",
    "newcounter",
    "stepcounter",
    "refstepcounter",
    "addtocounter",
    "setcounter",
    // other files, verbatim, notes
    "verb",
    "input",
    "pgfimage",
    "includesvg",
    "includepdf",
    "lstinputlisting",
    "include",
    "footnote",
    "index",
    "marginpar",
    "write",
    "immediate",
    // randomness, positions
    "pgfmathrandom",
    "random",
    "pgfmathsetseed",
    "pdfsavepos",
    "savepos",
    // assignments and boxes that escape the picture's group: a skipped body must have no
    // effect on what follows it
    "global",
    "xdef",
    "gdef",
    "savebox",
    "sbox",
    "pgfdeclarelayer",
    "newbox",
    "newsavebox",
    "usebox",
    "setbox",
    // catcode changes and Lua: a skipped body is scanned with the catcodes in force at the
    // picture, so a body that reads part of itself under other catcodes (`%` inside
    // `luacode*`, `\verb|}|`) is not skipped the way the drawing reads it; Lua may also
    // print anything
    "catcode",
    "makeatletter",
    "makeatother",
    "ExplSyntaxOn",
    "obeylines",
    "obeyspaces",
    "scantokens",
    "lstinline",
    "mintinline",
    "Verb",
    "directlua",
    "luaexec",
    "luadirect",
];

/// Control-word prefixes (`\citep`, `\includegraphics*`, `\pgfplotstableread` …).
pub(super) const UNCACHEABLE_PREFIXES: &[&str] =
    &["cite", "includegraphics", "pgfplotstable", "footcite"];

/// Plain text. `trim left`/`trim right` (pgfplots `trim axis left/right`) are kerns pgf puts
/// outside the picture box, which a cached box does not carry; `legend to name` writes a
/// label.
pub(super) const UNCACHEABLE_TEXT: &[&str] = &[
    "remember picture",
    "overlay",
    "trim left",
    "trim right",
    "trim axis",
    "legend to name",
];

pub(super) fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// Does `text` contain the control word `\name` (not as a prefix of a longer word)?
pub(super) fn has_cs(text: &str, name: &str) -> bool {
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(k) = text[from..].find('\\') {
        let start = from + k + 1;
        let end = start + name.len();
        if text[start..].starts_with(name) && (end >= b.len() || !is_letter(b[end])) {
            return true;
        }
        from = start;
    }
    false
}

/// Does `text` print a counter with `\the<counter>` (`\thesection`, `\thepage`, `\theequation`
/// and every user `\the…`)? `\theta` and `\therefore` are symbols, `\the` itself is a word of
/// its own.
pub(super) fn has_the_counter(text: &str) -> bool {
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(k) = text[from..].find("\\the") {
        let start = from + k + 4;
        let mut end = start;
        while end < b.len() && is_letter(b[end]) {
            end += 1;
        }
        let rest = &text[start..end];
        if !rest.is_empty() && !matches!(rest, "ta" | "refore") {
            return true;
        }
        from = start;
    }
    false
}

/// Does `text` contain a control word starting with `\name`?
pub(super) fn has_cs_prefix(text: &str, name: &str) -> bool {
    text.contains(&format!("\\{name}"))
}

/// Does the word `word` (letters around it excluded) appear followed by `[` or `{`, like the
/// pgfplots data sources `\addplot table[x=a] {f.dat}` and `\addplot file {f.dat}`?
pub(super) fn word_before_arg(text: &str, word: &str) -> bool {
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(k) = text[from..].find(word) {
        let start = from + k;
        let end = start + word.len();
        let left_ok = start == 0 || !is_letter(b[start - 1]);
        let mut j = end;
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t' || b[j] == b'\n') {
            j += 1;
        }
        if left_ok
            && j < b.len()
            && (b[j] == b'[' || b[j] == b'{')
            && (end == b.len() || !is_letter(b[end]))
        {
            return true;
        }
        from = end;
    }
    false
}

/// Does `text` open an environment that reads its body under other catcodes (the list is the
/// segmenter's: `document::is_verbatim_env`)?
pub(super) fn has_verbatim_env(text: &str) -> bool {
    let mut from = 0;
    while let Some(k) = text[from..].find("\\begin{") {
        let start = from + k + "\\begin{".len();
        let name = text[start..].split('}').next().unwrap_or("");
        if crate::document::is_verbatim_env(name) {
            return true;
        }
        from = start;
    }
    false
}

/// Would the skip of a cached picture's body (rtex-pic.tex: `\rtex@gobbleto`, which takes
/// everything up to each `\end` as a macro argument) stop at the picture's own `\end{env}`?
/// It cannot when a `}` closes a group the body never opened (an argument error) or when the
/// `\end{env}` sits inside braces (the skip runs past it to the end of the file). `text` is
/// the comment-stripped source from the `\begin{env}` line to the `\end{env}` line.
pub(super) fn skip_stops_at_end(text: &str, env: &str) -> bool {
    let begin = format!("\\begin{{{env}}}");
    let end = format!("\\end{{{env}}}");
    let Some(b) = text.find(&begin) else {
        return false;
    };
    let body = &text[b + begin.len()..];
    let bytes = body.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if depth == 0 && body[i..].starts_with(&end) {
                    return true;
                }
                i += 1;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Does a picture with this (comment-stripped) text depend on more than its source and the
/// definitions before it?
pub fn uncacheable(text: &str) -> bool {
    UNCACHEABLE_WORDS.iter().any(|w| has_cs(text, w))
        || has_verbatim_env(text)
        || has_the_counter(text)
        || UNCACHEABLE_PREFIXES.iter().any(|w| has_cs_prefix(text, w))
        || UNCACHEABLE_TEXT.iter().any(|t| text.contains(t))
        || word_before_arg(text, "table")
        || word_before_arg(text, "file")
}
