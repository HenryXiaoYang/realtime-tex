//! Scanner: control sequences and their arguments.

use super::lex::*;
use super::reason::*;
use super::scan::*;
use super::vocab_envs::*;
use super::vocab_math::*;
use super::vocab_packages::*;
use super::vocab_text::*;

impl<'a> Scan<'a> {
    /// A control sequence at `i`: math delimiters, environments, opaque arguments, macros.
    /// False at the end of the source.
    pub(super) fn control_sequence(&mut self, in_math: bool) -> bool {
        let (text, b) = (self.text, self.b);
        if self.i + 1 >= b.len() {
            return false;
        }
        let n = b[self.i + 1];
        let name: String;
        if is_letter(n) {
            let mut j = self.i + 1;
            while j < b.len() && is_letter(b[j]) {
                j += 1;
            }
            name = text[self.i + 1..j].to_string();
            // `\tag*`, `\caption*`, `\hspace*`
            if j < b.len()
                && b[j] == b'*'
                && matches!(
                    name.as_str(),
                    "tag"
                        | "caption"
                        | "hspace"
                        | "vspace"
                        | "section"
                        | "subsection"
                        | "subsubsection"
                        | "chapter"
                        | "part"
                        | "paragraph"
                        | "subparagraph"
                        | "alph"
                        | "Alph"
                        | "arabic"
                        | "roman"
                        | "Roman"
                        | "operatorname"
                        | "verb"
                )
            {
                self.i = j + 1;
            } else {
                self.i = j;
            }
        } else if n == b'(' {
            if in_math {
                self.push(Reason::UnbalancedMath);
            }
            self.stack.push(Frame {
                mode: Mode::Math,
                env: None,
                depth_at_entry: self.depth,
            });
            self.i += 2;
            return true;
        } else if n == b')' {
            if !in_math
                || self
                    .stack
                    .last()
                    .map(|f| f.depth_at_entry != self.depth)
                    .unwrap_or(true)
            {
                self.push(Reason::UnbalancedMath);
            }
            if in_math {
                self.stack.pop();
            }
            self.i += 2;
            return true;
        } else if n == b'[' {
            if in_math {
                self.push(Reason::UnbalancedMath);
            }
            self.stack.push(Frame {
                mode: Mode::Math,
                env: None,
                depth_at_entry: self.depth,
            });
            self.i += 2;
            return true;
        } else if n == b']' {
            if !in_math {
                self.push(Reason::UnbalancedMath);
            } else {
                self.stack.pop();
            }
            self.i += 2;
            return true;
        } else {
            name = (n as char).to_string();
            self.i += 2;
        }
        if self.policy.math_owner_attribute.is_some() {
            if name == "attribute" && self.owner_assignment() {
                return true;
            }
            if name == "begingroup" || name == "endgroup" {
                self.character(if name == "begingroup" { b'{' } else { b'}' }, in_math);
                return true;
            }
        }
        if name == "begin" {
            self.begin_environment(in_math);
            return true;
        }
        if name == "end" {
            self.end_environment();
            return true;
        }
        // opaque arguments (URLs, units, verbatim): skip them, check the package
        if let Some((_, nargs)) = OPAQUE_ARGS.iter().find(|(n, _)| *n == name.as_str()) {
            self.opaque_arguments(&name, *nargs);
            return true;
        }
        self.macro_word(name, in_math);
        true
    }

    // Only decimal literals with an explicit '='. Register expressions and the other
    // attribute primitives still go through the ordinary (rejecting) vocabulary check.
    pub(super) fn owner_assignment(&mut self) -> bool {
        let rest = &self.text[self.i..];
        let Some((number, value)) = rest.trim_start().split_once('=') else {
            return false;
        };
        if number.trim().parse::<u16>().ok() != self.policy.math_owner_attribute {
            return false;
        }
        let value_start = rest.len() - value.trim_start().len();
        let bytes = &rest.as_bytes()[value_start..];
        let sign = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
        let digits = bytes[sign..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let end = value_start + sign + digits;
        if digits == 0 || rest[value_start..end].parse::<i32>().is_err() {
            return false;
        }
        self.i += end;
        true
    }

    /// Opaque arguments (URLs, units, verbatim): skipped; the package is checked.
    pub(super) fn opaque_arguments(&mut self, name: &str, nargs: usize) {
        let (text, b, policy) = (self.text, self.b, self.policy);
        let name = name.to_string();
        if let Some(Err(pkg)) = package_macro(&name, policy) {
            self.push(Reason::NeedsPackage(pkg.into()));
        }
        if nargs == 0 {
            // \verb|...| / \lstinline|...| / \lstinline{...}
            if self.i < b.len() {
                let d = b[self.i];
                let close = if d == b'{' { b'}' } else { d };
                match text[self.i + 1..].find(close as char) {
                    Some(e) => self.i = self.i + 1 + e + 1,
                    None => self.push(Reason::Verbatim),
                }
            }
        } else {
            // optional argument, then n balanced brace groups
            while self.i < b.len() && b[self.i] == b' ' {
                self.i += 1;
            }
            if self.i < b.len() && b[self.i] == b'[' {
                if let Some(e) = text[self.i..].find(']') {
                    self.i += e + 1;
                }
            }
            for _ in 0..nargs {
                while self.i < b.len() && b[self.i] == b' ' {
                    self.i += 1;
                }
                if self.i < b.len() && b[self.i] == b'{' {
                    let mut d = 0i32;
                    let mut k = self.i;
                    while k < b.len() {
                        match b[k] {
                            b'\\' => k += 1,
                            b'{' => d += 1,
                            b'}' => {
                                d -= 1;
                                if d == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    self.i = (k + 1).min(b.len());
                } else {
                    break;
                }
            }
        }
    }

    /// A macro: allowed by the vocabulary for its mode and context, or a reason.
    pub(super) fn macro_word(&mut self, name: String, in_math: bool) {
        let (text, policy) = (self.text, self.policy);
        let allowed = if let Some(provided) = package_macro(&name, policy) {
            if let Err(pkg) = provided {
                self.push(Reason::NeedsPackage(pkg.into()));
            }
            if in_math && matches!(name.as_str(), "text" | "mbox") {
                self.text_group_pending = true;
            }
            true
        } else if GROUP_ONLY_SETTERS.contains(&name.as_str())
            || ((name == "renewcommand" || name == "renewcommand*")
                && text[self.i..].trim_start().starts_with("{\\arraystretch}"))
        {
            if self.depth == 0 && self.env_depth == 0 {
                self.push(Reason::SizeDeclarationOutsideGroup(name.clone()));
            }
            // the first argument names the length or macro being set: skip it
            let rest = &text[self.i..];
            let lead = rest.len() - rest.trim_start().len();
            if let Some(after) = rest.trim_start().strip_prefix('{') {
                if let Some(e) = after.find('}') {
                    self.i += lead + 1 + e + 1;
                }
            }
            true
        } else if in_math {
            let ok = MATH_MACROS.contains(&name.as_str())
                || policy.trusted_math.contains(&name)
                || policy.trusted_macros.contains(&name);
            if ok
                && matches!(
                    name.as_str(),
                    "text" | "mbox" | "textrm" | "textit" | "textbf" | "intertext"
                )
            {
                self.text_group_pending = true;
            }
            if (name == "tag"
                || name == "notag"
                || name == "nonumber"
                || name == "intertext"
                || name == "eqref")
                && !policy.amsmath
                && name != "nonumber"
            {
                self.push(Reason::NeedsPackage("amsmath".into()));
            }
            ok
        } else if GROUP_ONLY_DECLARATIONS.contains(&name.as_str()) {
            if self.depth == 0
                && self.env_depth == 0
                && !matches!(self.shape, UnitShape::Heading(_))
            {
                self.push(Reason::SizeDeclarationOutsideGroup(name.clone()));
            }
            true
        } else if HEADINGS.contains(&name.as_str()) || policy.heading_macros.contains_key(&name) {
            // only as the unit's own heading command (possibly after leading vertical
            // material such as \newpage), or a user macro wrapping one
            let h = policy
                .heading_macros
                .get(&name)
                .cloned()
                .unwrap_or_else(|| name.clone());
            matches!(&self.shape, UnitShape::Heading(sh) if *sh == h)
                && self.i <= self.heading_base + name.len() + 2
        } else if name == "item" {
            let in_list = self.stack.iter().any(|f| {
                f.env
                    .as_deref()
                    .map(|e| LIST_ENVS.contains(&e))
                    .unwrap_or(false)
            });
            if !in_list {
                self.push(Reason::MacroOutsideContext(name.clone()));
            }
            true
        } else if FLOAT_MACROS.contains(&name.as_str()) {
            let in_float = self.stack.iter().any(|f| {
                f.env
                    .as_deref()
                    .map(|e| FLOAT_ENVS.contains(&e))
                    .unwrap_or(false)
            });
            if !in_float {
                self.push(Reason::MacroOutsideContext(name.clone()));
            }
            true
        } else if TABULAR_MACROS.contains(&name.as_str())
            || BOOKTABS_MACROS.contains(&name.as_str())
        {
            let in_tab = self.stack.iter().any(|f| {
                f.env
                    .as_deref()
                    .map(|e| e.starts_with("tabular") || e == "array")
                    .unwrap_or(false)
            });
            if !in_tab {
                self.push(Reason::MacroOutsideContext(name.clone()));
            }
            if BOOKTABS_MACROS.contains(&name.as_str()) && !policy.booktabs {
                self.push(Reason::NeedsPackage("booktabs".into()));
            }
            true
        } else if name == "includegraphics" {
            if !policy.graphicx {
                self.push(Reason::NeedsPackage("graphicx".into()));
            }
            true
        } else if name == "cite" {
            if !policy.cite_ok {
                self.push(Reason::DisallowedMacro(name.clone()));
            }
            true
        } else if name == "eqref" {
            if !policy.amsmath {
                self.push(Reason::NeedsPackage("amsmath".into()));
            }
            true
        } else {
            TEXT_MACROS.contains(&name.as_str()) || policy.trusted_macros.contains(&name)
        };
        if !allowed {
            let r = if in_math {
                Reason::DisallowedMathMacro(name)
            } else {
                Reason::DisallowedMacro(name)
            };
            self.push(r);
        }
    }
}
