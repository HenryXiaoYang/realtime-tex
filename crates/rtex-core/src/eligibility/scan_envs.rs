//! Scanner: `\\begin` and `\\end`.

use super::lex::*;
use super::reason::*;
use super::scan::*;
use super::shape::*;
use super::vocab_envs::*;

impl<'a> Scan<'a> {
    pub(super) fn begin_environment(&mut self, in_math: bool) {
        let (text, policy) = (self.text, self.policy);
        let begin_pos = self.i - 6;
        let Some((env, after)) = env_name(text, begin_pos, "\\begin") else {
            self.push(Reason::UnbalancedEnvironment);
            return;
        };
        self.i = after;
        if self.block_env_closed_at.is_some() && self.env_depth == 0 {
            // a second block environment after one closed: still fine (both inside the unit)
        }
        // enumitem: `resume`, `resume*` and `series=` continue an earlier list's numbering
        let base = env.trim_end_matches('*');
        if LIST_ENVS.contains(&base) || policy.list_envs.contains(base) {
            if let Some(opts) = optional_arg(&text[after..]) {
                let keys = top_level_split(opts, ',');
                if keys.iter().any(|kv| {
                    let key = kv.split('=').next().unwrap_or("").trim();
                    matches!(key, "resume" | "resume*" | "series")
                }) {
                    self.push(Reason::OutsideState(format!("\\begin{{{env}}}[resume]")));
                }
            }
        }
        let is_amsmath_display = AMSMATH_DISPLAY_ENVS.contains(&env.as_str());
        let is_amsmath_inner = AMSMATH_INNER_ENVS.contains(&env.as_str());
        if OPAQUE_ENVS.contains(&env.as_str()) {
            // verbatim material: skip to \end{env}; the environment is a block unit
            if env == "lstlisting" && !policy.has_package("listings") {
                self.push(Reason::NeedsPackage("listings".into()));
            }
            let end = format!("\\end{{{env}}}");
            match text[self.i..].find(&end) {
                Some(e) => {
                    self.i += e + end.len();
                    if policy.is_block_env(&env) && self.env_depth == 0 {
                        self.block_env_closed_at = Some(self.i);
                    }
                }
                None => self.push(Reason::UnbalancedEnvironment),
            }
            return;
        }
        if policy.is_display_env(&env) {
            if is_amsmath_display && !policy.amsmath {
                self.push(Reason::NeedsPackage("amsmath".into()));
            }
            if in_math {
                self.push(Reason::UnbalancedMath);
            }
            self.stack.push(Frame {
                mode: Mode::Math,
                env: Some(env),
                depth_at_entry: self.depth,
            });
        } else if policy.is_block_env(&env) {
            if in_math {
                self.push(Reason::UnbalancedMath);
            }
            if self.block_env_closed_at.is_some()
                && self.env_depth == 0
                && self.shape == UnitShape::Par
            {
                // text (or another environment) after a block environment: the capture
                // closes the paragraph unit at the first environment's end
                self.push(Reason::TextAfterEnvironment);
            }
            if PICTURE_ENVS.contains(&env.as_str()) {
                // a unit of its own (block), but its drawing commands are not
                // allow-listed: probe mode compiles and compares it, or takes it from
                // the picture cache. The body is the environment's business (one
                // reason, not one per drawing command): skipped to its \end.
                self.push(Reason::DisallowedEnvironment(env.clone()));
                // inside a paragraph's text it is an inline box: the paragraph goes on
                // after it (the capture keeps one unit)
                let inline = self.env_depth == 0
                    && self.shape == UnitShape::Par
                    && !text[content_start(text).min(begin_pos)..begin_pos]
                        .trim()
                        .is_empty();
                match skip_env_body(text, self.i, &env) {
                    Some(e) => {
                        self.i = e;
                        if self.env_depth == 0 && !inline {
                            self.block_env_closed_at = Some(self.i);
                        }
                    }
                    None => self.push(Reason::UnbalancedEnvironment),
                }
                return;
            }
            self.env_depth += 1;
            self.stack.push(Frame {
                mode: Mode::Text,
                env: Some(env),
                depth_at_entry: self.depth,
            });
        } else if INNER_ENVS.contains(&env.as_str())
            || is_amsmath_inner
            || policy.user_inner_envs.contains(&env)
        {
            if is_amsmath_inner && !policy.amsmath {
                self.push(Reason::NeedsPackage("amsmath".into()));
            }
            if env == "tabularx" && !policy.tabularx {
                self.push(Reason::NeedsPackage("tabularx".into()));
            }
            let mode = if is_amsmath_inner || env == "math" {
                Mode::Math
            } else {
                Mode::Text
            };
            if env == "math" && in_math {
                self.push(Reason::UnbalancedMath);
            }
            self.stack.push(Frame {
                mode: if is_amsmath_inner { Mode::Math } else { mode },
                env: Some(env),
                depth_at_entry: self.depth,
            });
        } else {
            self.push(Reason::DisallowedEnvironment(env.clone()));
            self.stack.push(Frame {
                mode: Mode::Text,
                env: Some(env),
                depth_at_entry: self.depth,
            });
        }
    }

    pub(super) fn end_environment(&mut self) {
        let (text, policy) = (self.text, self.policy);
        let Some((env, after)) = env_name(text, self.i - 4, "\\end") else {
            self.push(Reason::UnbalancedEnvironment);
            return;
        };
        self.i = after;
        match self
            .stack
            .iter()
            .rposition(|f| f.env.as_deref() == Some(env.as_str()))
        {
            Some(pos) if pos == self.stack.len() - 1 => {
                let f = self.stack.pop().unwrap();
                if f.depth_at_entry != self.depth {
                    self.push(Reason::UnbalancedBraces);
                    self.depth = f.depth_at_entry;
                }
                if policy.is_block_env(&env) {
                    self.env_depth -= 1;
                    if self.env_depth == 0 {
                        self.block_env_closed_at = Some(self.i);
                    }
                }
            }
            _ => self.push(Reason::UnbalancedEnvironment),
        }
    }
}
