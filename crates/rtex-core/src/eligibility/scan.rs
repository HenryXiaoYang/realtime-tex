//! The scanner: one pass over a unit's source, collecting the reasons it is not eligible.

use super::lex::*;
use super::policy::*;
use super::reason::*;
use super::shape::*;
use super::vocab_text::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Text,
    Math,
}

pub(super) struct Frame {
    pub(super) mode: Mode,
    pub(super) env: Option<String>,
    pub(super) depth_at_entry: i32,
}

/// The scan state of `classify_source_with`: a position in the comment-stripped source, the
/// brace depth, the open groups and environments, and the reasons found so far. One method per
/// construct; a method returning means "go on with the next character".
pub(super) struct Scan<'a> {
    pub(super) text: &'a str,
    pub(super) b: &'a [u8],
    pub(super) policy: &'a Policy,
    pub(super) shape: UnitShape,
    /// Where the unit's own heading command may start (see `unit_shape`).
    pub(super) heading_base: usize,
    pub(super) reasons: Vec<Reason>,
    pub(super) i: usize,
    pub(super) depth: i32,
    pub(super) stack: Vec<Frame>,
    /// Block environments open.
    pub(super) env_depth: i32,
    /// Index after the last `\end{block}` at env_depth 0.
    pub(super) block_env_closed_at: Option<usize>,
    /// The next brace group is text mode (`\text{...}` in math).
    pub(super) text_group_pending: bool,
    pub(super) line_start: usize,
}

impl<'a> Scan<'a> {
    pub(super) fn push(&mut self, r: Reason) {
        if !self.reasons.contains(&r) {
            self.reasons.push(r);
        }
    }

    pub(super) fn in_math(&self) -> bool {
        self.stack
            .last()
            .map(|f| f.mode == Mode::Math)
            .unwrap_or(false)
    }

    /// Control words that stop the fast path wherever they are.
    pub(super) fn hard_stops(&mut self) {
        let text = self.text;
        for (pat, r) in HARD_STOPS {
            if let Some(mut idx) = text.find(pat) {
                // must be a whole control word
                let b = text.as_bytes();
                loop {
                    let end = idx + pat.len();
                    if end >= b.len() || !is_letter(b[end]) {
                        self.push(r.clone());
                        break;
                    }
                    match text[end..].find(pat) {
                        Some(p) => idx = end + p,
                        None => break,
                    }
                }
            }
        }
    }

    /// A leading counter assignment whose argument does not close the unit.
    pub(super) fn leading_counter(&mut self) {
        let text = self.text;
        {
            let t = text.trim_start();
            for m in [
                "setcounter",
                "addtocounter",
                "stepcounter",
                "refstepcounter",
            ] {
                if t.starts_with(&format!("\\{m}{{")) && !t.trim_end().ends_with('}') {
                    self.push(Reason::LeadingCounter(m.into()));
                    break;
                }
            }
        }
    }

    pub(super) fn run(&mut self) {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'\n' {
                self.line_break();
                continue;
            }
            let in_math = self.in_math();
            if c == b'\\' {
                if !self.control_sequence(in_math) {
                    break;
                }
                continue;
            }
            self.character(c, in_math);
            self.i += 1;
        }
    }

    /// Paragraph breaks (blank lines) are only fine inside block environments, and between a
    /// heading (with its labels) and what follows: TeX is still in vertical mode there (a
    /// run-in heading's span goes on to the paragraph that sets its title).
    pub(super) fn line_break(&mut self) {
        let (text, b) = (self.text, self.b);
        let line = &text[self.line_start..self.i];
        if line.trim().is_empty()
            && self.i > 0
            && self.env_depth == 0
            && self.i + 1 < b.len()
            && !text[self.i + 1..].trim().is_empty()
            && self.line_start > 0
            && !(matches!(self.shape, UnitShape::Heading(_))
                && after_heading(&text[..self.line_start]).is_some_and(labels_only))
        {
            self.push(Reason::ParagraphBreak);
        }
        self.line_start = self.i + 1;
        self.i += 1;
    }

    /// A character other than a backslash or a newline: groups, math shifts, text after a
    /// block environment.
    pub(super) fn character(&mut self, c: u8, in_math: bool) {
        let b = self.b;
        match c {
            b'{' => {
                self.depth += 1;
                if self.text_group_pending {
                    self.text_group_pending = false;
                    self.stack.push(Frame {
                        mode: Mode::Text,
                        env: None,
                        depth_at_entry: self.depth,
                    });
                }
            }
            b'}' => {
                self.depth -= 1;
                if self.depth < 0 {
                    self.push(Reason::UnbalancedBraces);
                    self.depth = 0;
                }
                // leaving a \text{...} group
                if let Some(f) = self.stack.last() {
                    if f.env.is_none()
                        && f.mode == Mode::Text
                        && self.stack.len() > 1
                        && f.depth_at_entry == self.depth + 1
                    {
                        self.stack.pop();
                    }
                }
            }
            b'$' => {
                let double = self.i + 1 < b.len() && b[self.i + 1] == b'$';
                if double {
                    self.i += 1;
                }
                if in_math {
                    let f = self.stack.last().unwrap();
                    if f.env.is_some() || f.depth_at_entry != self.depth {
                        self.push(Reason::UnbalancedMath);
                    } else {
                        self.stack.pop();
                    }
                } else {
                    self.stack.push(Frame {
                        mode: Mode::Math,
                        env: None,
                        depth_at_entry: self.depth,
                    });
                }
            }
            _ => {
                if self.block_env_closed_at.is_some()
                    && self.env_depth == 0
                    && self.shape == UnitShape::Par
                    && !c.is_ascii_whitespace()
                {
                    self.push(Reason::TextAfterEnvironment);
                }
            }
        }
    }

    pub(super) fn finish(mut self, permissive: bool) -> (UnitShape, Vec<Reason>) {
        let text = self.text;
        if self.depth != 0 {
            self.push(Reason::UnbalancedBraces);
        }
        if self.stack.len() > 1 {
            let f = self.stack.last().unwrap();
            if f.env.is_some() {
                self.push(Reason::UnbalancedEnvironment);
            } else if f.mode == Mode::Math {
                self.push(Reason::UnbalancedMath);
            }
        }
        let _ = Reason::EnvironmentBoundary;
        let _ = Reason::DisplayMath;
        // the title block is a center environment in the capture (article's \@maketitle)
        if self.shape == UnitShape::Par && text.contains("\\maketitle") {
            self.shape = UnitShape::Env("center".into());
        }
        if permissive {
            self.reasons.retain(|r| !is_vocabulary_reason(r));
        }
        (self.shape, self.reasons)
    }
}
