//! Deterministic filler text: a small random generator, a word list, and sentences and paragraphs made from them.

use std::fmt::Write as _;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    pub fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }
}

pub(super) const WORDS: &[&str] = &[
    "paragraph",
    "line",
    "breaking",
    "engine",
    "glyph",
    "position",
    "measure",
    "document",
    "page",
    "layout",
    "font",
    "kerning",
    "margin",
    "protrusion",
    "expansion",
    "editor",
    "keystroke",
    "latency",
    "cache",
    "background",
    "convergence",
    "footnote",
    "float",
    "counter",
    "reference",
    "typesetting",
    "algorithm",
    "demerits",
    "penalty",
    "glue",
    "stretch",
    "shrink",
    "baseline",
    "height",
    "depth",
    "width",
    "box",
    "list",
    "node",
    "attribute",
    "callback",
    "process",
    "persistent",
    "session",
    "revision",
    "snapshot",
    "render",
    "display",
    "export",
    "quality",
    "typography",
    "author",
    "book",
    "chapter",
    "section",
    "sentence",
    "word",
    "letter",
    "space",
    "the",
    "a",
    "an",
    "of",
    "in",
    "on",
    "with",
    "and",
    "or",
    "but",
    "for",
    "to",
    "from",
    "by",
    "that",
    "which",
    "when",
    "while",
    "because",
    "although",
    "every",
    "each",
    "some",
    "many",
    "is",
    "are",
    "was",
    "were",
    "becomes",
    "remains",
    "depends",
    "produces",
    "requires",
    "allows",
    "quickly",
    "slowly",
    "exactly",
    "nearly",
    "always",
    "never",
    "often",
    "rarely",
    "again",
    "fidelity",
    "identical",
    "independent",
    "local",
    "global",
    "incremental",
    "immediate",
];

pub(super) const MATH: &[&str] = &[
    r"$f(x) = x^2 + 1$",
    r"$\sum_{i=1}^{n} a_i$",
    r"$\alpha + \beta = \gamma$",
    r"$O(1)$",
    r"$O(n)$",
    r"$\frac{1}{2}$",
    r"$x \in \mathbb{R}$",
    r"$\sqrt{2}$",
    r"$\pi \approx 3.14159$",
    r"$e^{i\pi} + 1 = 0$",
    r"$\lim_{n \to \infty} x_n$",
    r"$a \leq b$",
    r"$\partial f / \partial x$",
    r"$\mathbf{v} \cdot \mathbf{w}$",
];

pub(super) fn sentence(rng: &mut Rng, pure_math_ok: bool) -> String {
    let n = rng.range(7, 18) as usize;
    let mut s = String::new();
    for i in 0..n {
        let mut w = WORDS[rng.below(WORDS.len() as u64) as usize].to_string();
        if i == 0 {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                w = f.to_uppercase().collect::<String>() + c.as_str();
            }
        }
        if i > 0 {
            s.push(' ');
        }
        if pure_math_ok && rng.chance(25) {
            s.push_str(MATH[rng.below(MATH.len() as u64) as usize]);
        } else if rng.chance(30) {
            let _ = write!(s, "\\emph{{{}}}", w);
        } else if rng.chance(12) {
            let _ = write!(s, "\\textbf{{{}}}", w);
        } else if rng.chance(8) {
            let _ = write!(s, "\\textsc{{{}}}", w);
        } else {
            s.push_str(&w);
        }
        if i + 1 < n && rng.chance(80) {
            s.push(',');
        }
    }
    s.push('.');
    s
}

/// Greedy word wrap that never splits inside `$...$` or `{...}` groups.
pub fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut token = String::new();
    let mut depth_brace = 0i32;
    let mut in_math = false;
    let flush = |token: &mut String, out: &mut String, col: &mut usize| {
        if token.is_empty() {
            return;
        }
        if *col > 0 && *col + 1 + token.len() > width {
            out.push('\n');
            *col = 0;
        } else if *col > 0 {
            out.push(' ');
            *col += 1;
        }
        out.push_str(token);
        *col += token.len();
        token.clear();
    };
    for ch in text.chars() {
        match ch {
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            '$' => in_math = !in_math,
            _ => {}
        }
        if ch == ' ' && depth_brace == 0 && !in_math {
            flush(&mut token, &mut out, &mut col);
        } else {
            token.push(ch);
        }
    }
    flush(&mut token, &mut out, &mut col);
    out
}

/// A one-line paragraph (4-7 plain words), the paper's "short" category.
pub fn short_paragraph(rng: &mut Rng) -> (String, usize) {
    let n = rng.range(4, 7) as usize;
    let mut s = String::new();
    for i in 0..n {
        let mut w = WORDS[rng.below(WORDS.len() as u64) as usize].to_string();
        if i == 0 {
            let mut c = w.chars();
            if let Some(f) = c.next() {
                w = f.to_uppercase().collect::<String>() + c.as_str();
            }
        }
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&w);
    }
    s.push('.');
    (s, n)
}

/// A body paragraph. Returns (text, word_estimate).
pub fn paragraph(rng: &mut Rng, sentences: u64, math: bool) -> (String, usize) {
    let mut p = String::new();
    let mut words = 0;
    for i in 0..sentences {
        if i > 0 {
            p.push(' ');
        }
        let s = sentence(rng, math);
        words += s.split_whitespace().count();
        p.push_str(&s);
    }
    (p, words)
}
