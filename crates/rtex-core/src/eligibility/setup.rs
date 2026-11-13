//! Setup statements: body-level settings the server replays instead of typesetting.

use super::lex::*;

/// Commands that set up the document rather than typeset: a body span made only of them (and
/// their arguments) is loaded with the preamble by the fast server, so macros defined after
/// `\begin{document}` and global settings (`\renewcommand{\arraystretch}`) reach the fast path.
pub const SETUP_MACROS: &[&str] = &[
    "newcommand",
    "renewcommand",
    "providecommand",
    "newenvironment",
    "renewenvironment",
    "def",
    "edef",
    "gdef",
    "let",
    "newif",
    "newlength",
    "newcounter",
    "newtheorem",
    "theoremstyle",
    "DeclareMathOperator",
    "newcolumntype",
    "setlength",
    "addtolength",
    "linespread",
    "setstretch",
    "selectfont",
    "pagestyle",
    "pagenumbering",
    "graphicspath",
    "lstset",
    "captionsetup",
    "hypersetup",
    "setlist",
    "definecolor",
    "colorlet",
    "numberwithin",
    "allowdisplaybreaks",
    "raggedbottom",
    "flushbottom",
    "sloppy",
    "fussy",
    "bibliographystyle",
    "frenchspacing",
    "nonfrenchspacing",
    "sisetup",
    "fancyhf",
    "fancyhead",
    "fancyfoot",
    "lhead",
    "chead",
    "rhead",
    "lfoot",
    "cfoot",
    "rfoot",
    "setmainfont",
    "setsansfont",
    "setmonofont",
    "setmathfont",
    "newfontfamily",
    "DeclareRobustCommand",
    "NewDocumentCommand",
    "RenewDocumentCommand",
    "ProvideDocumentCommand",
    "NewDocumentEnvironment",
    "RenewDocumentEnvironment",
];

/// `Some(text)` when `text` (comments stripped) consists only of setup statements: each
/// statement a `SETUP_MACROS` command with its arguments (`*`, `[…]`, `{…}`, control-sequence
/// and `#n` parameters, `=`).
pub fn setup_statements(text: &str) -> Option<String> {
    let (end, any) = setup_prefix(text);
    if any && text[end..].trim().is_empty() {
        Some(text.trim().to_string())
    } else {
        None
    }
}

/// The leading run of setup statements of `text`: the offset where the first other content
/// starts (the text's length when it is setup statements only) and whether there was any.
pub fn setup_prefix(text: &str) -> (usize, bool) {
    let b = text.as_bytes();
    let mut i = 0;
    let mut any = false;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let stmt = i; // start of the statement being scanned
        if b[i] != b'\\' {
            return (stmt, any);
        }
        let mut j = i + 1;
        while j < b.len() && is_letter(b[j]) {
            j += 1;
        }
        if j == i + 1 || !SETUP_MACROS.contains(&&text[i + 1..j]) {
            return (stmt, any);
        }
        i = j;
        // arguments
        loop {
            while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                i += 1;
            }
            if i >= b.len() {
                break;
            }
            match b[i] {
                b'*' | b'=' => i += 1,
                b'[' => match text[i..].find(']') {
                    Some(k) => i += k + 1,
                    None => return (stmt, any),
                },
                b'{' => {
                    let mut depth = 0i32;
                    let mut k = i;
                    let mut closed = None;
                    while k < b.len() {
                        match b[k] {
                            b'\\' => k += 1,
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    closed = Some(k);
                                    break;
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    let Some(c) = closed else {
                        return (stmt, any);
                    };
                    i = c + 1;
                }
                b'#' => {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                b'\\' => {
                    // a control-sequence argument (\def\foo, \setlength\parindent); a setup
                    // command starts the next statement instead
                    let mut k = i + 1;
                    while k < b.len() && is_letter(b[k]) {
                        k += 1;
                    }
                    if k == i + 1 {
                        k += 1; // control symbol
                    } else if SETUP_MACROS.contains(&&text[i + 1..k]) {
                        break;
                    }
                    i = k;
                }
                b'\n' => break,
                _ => return (stmt, any),
            }
        }
        any = true;
    }
    (text.len(), any)
}
