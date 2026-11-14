//! The document's policy: what its preamble loads and defines.

use super::classify_source;
use super::lex::*;
use super::reason::*;
use super::vocab_envs::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Extra text-mode macros the host vouches for (pure, no global side effects).
    pub trusted_macros: BTreeSet<String>,
    /// Theorem-like environments (from `\newtheorem` in the preamble or the host) treated as units.
    pub theorem_envs: BTreeSet<String>,
    pub amsmath: bool,
    pub graphicx: bool,
    pub booktabs: bool,
    pub tabularx: bool,
    /// `\cite` is replayable from the aux (plain or natbib `\bibcite`); false with biblatex.
    pub cite_ok: bool,
    /// Packages loaded by the preamble (`\usepackage`/`\RequirePackage`, options resolved for
    /// `xcolor[table]` → colortbl).
    pub packages: BTreeSet<String>,
    /// Math-mode macros defined in the preamble whose bodies are allow-listed (`\newcommand{\R}{\mathbb{R}}`).
    pub trusted_math: BTreeSet<String>,
    /// Research mode: unknown macros, environments and missing packages do not make a unit
    /// ineligible (structural reasons still do); the fast result is validated against the layout
    /// instead (`rtex verify --permissive`).
    pub permissive: bool,
    /// The one literal attribute assignment allowed for host math ownership.
    pub math_owner_attribute: Option<u16>,
    /// User macros whose body starts with a heading command (`\newcommand{\unnumberedsection}[1]{\section*{#1}…}`):
    /// macro name → heading name. A span starting with one has that heading's shape.
    pub heading_macros: BTreeMap<String, String>,
    /// Environments defined in the preamble (`\newenvironment`) whose begin/end code is
    /// allow-listed text (font switches, `\noindent`, a label): typeset inside the paragraph
    /// they wrap. Ones whose code opens a block environment are in `theorem_envs` instead.
    pub user_inner_envs: BTreeSet<String>,
    /// Lists the preamble defines with enumitem's `\newlist`, besides `itemize`, `enumerate`
    /// and `description` (their options are checked for `resume`/`series`).
    pub list_envs: BTreeSet<String>,
}

impl Policy {
    /// Derive package facts and theorem environments from the document preamble.
    pub fn from_preamble(preamble: &str, trusted: &[String], unit_envs: &[String]) -> Policy {
        let text = strip_comments(preamble);
        let has_pkg = |name: &str| -> bool {
            let mut idx = 0;
            while let Some(p) = text[idx..].find("\\usepackage") {
                let s = idx + p;
                let rest = &text[s..];
                let end = rest.find('\n').map(|e| s + e).unwrap_or(text.len());
                // the braced list may follow an optional argument
                if let Some(bo) = text[s..end.max(s)].find('{') {
                    if let Some(bc) = text[s + bo..].find('}') {
                        if text[s + bo + 1..s + bo + bc]
                            .split(',')
                            .any(|n| n.trim() == name)
                        {
                            return true;
                        }
                    }
                }
                idx = s + 11;
            }
            text.contains(&format!("\\RequirePackage{{{name}}}"))
        };
        let mut theorem_envs = BTreeSet::new();
        let mut idx = 0;
        while let Some(p) = text[idx..].find("\\newtheorem") {
            let s = idx + p + "\\newtheorem".len();
            let rest = &text[s..];
            let rest = rest.strip_prefix('*').unwrap_or(rest);
            if let Some(b) = rest.strip_prefix('{') {
                if let Some(e) = b.find('}') {
                    theorem_envs.insert(b[..e].trim().to_string());
                }
            }
            idx = s;
        }
        for e in unit_envs {
            theorem_envs.insert(e.clone());
        }
        let mut list_envs = BTreeSet::new();
        let mut idx = 0;
        while let Some(p) = text[idx..].find("\\newlist") {
            let s = idx + p + "\\newlist".len();
            if let Some(b) = text[s..].trim_start().strip_prefix('{') {
                if let Some(e) = b.find('}') {
                    list_envs.insert(b[..e].trim().to_string());
                }
            }
            idx = s;
        }
        let mut packages = BTreeSet::new();
        for (opts, names) in usepackages(&text) {
            for n in names {
                packages.insert(n.clone());
                if n == "xcolor" && opts.iter().any(|o| o == "table") {
                    packages.insert("colortbl".into());
                }
                if n == "hyperref" {
                    packages.insert("url".into());
                }
                if n == "mathtools" {
                    packages.insert("amsmath".into());
                }
                // \mathbb and the AMS symbols also come with amsfonts and unicode-math
                if n == "amsfonts" || n == "unicode-math" {
                    packages.insert("amssymb".into());
                }
            }
        }
        let mut policy = Policy {
            trusted_macros: trusted.iter().cloned().collect(),
            theorem_envs,
            amsmath: has_pkg("amsmath") || has_pkg("mathtools"),
            graphicx: has_pkg("graphicx") || has_pkg("graphics"),
            booktabs: has_pkg("booktabs"),
            tabularx: has_pkg("tabularx"),
            cite_ok: true,
            packages,
            trusted_math: BTreeSet::new(),
            permissive: false,
            math_owner_attribute: None,
            heading_macros: BTreeMap::new(),
            user_inner_envs: BTreeSet::new(),
            list_envs,
        };
        trust_user_macros(&mut policy, &text);
        trust_user_environments(&mut policy, &text);
        policy
    }

    /// The segmentation rules from the preamble: theorem-like environments end a span like a
    /// list, and a heading the class sets run-in keeps the paragraph or list that sets its
    /// title in its span (other classes' run-in headings are only known from the capture).
    pub fn segment_rules(&self) -> crate::document::SegmentRules {
        let mut run_in: Vec<String> = RUN_IN_HEADINGS.iter().map(|h| h.to_string()).collect();
        if self.has_package("IEEEtran") {
            run_in.push("subsubsection".into());
        }
        crate::document::SegmentRules {
            block_envs: self.theorem_envs.iter().cloned().collect(),
            run_in,
        }
    }

    pub fn has_package(&self, name: &str) -> bool {
        self.packages.contains(name)
    }

    pub fn is_block_env(&self, name: &str) -> bool {
        BLOCK_ENVS.contains(&name) || FLOAT_ENVS.contains(&name) || self.theorem_envs.contains(name)
    }
    pub fn is_display_env(&self, name: &str) -> bool {
        DISPLAY_ENVS.contains(&name) || AMSMATH_DISPLAY_ENVS.contains(&name)
    }
    /// Comma-separated extra unit environments for the capture run (`$RTEX_UNIT_ENVS`).
    pub fn unit_envs_env(&self) -> String {
        self.theorem_envs
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Adds the preamble's own macros to the policy: heading wrappers, and macros whose bodies
/// the allow-list accepts.
fn trust_user_macros(policy: &mut Policy, text: &str) {
    // Macros defined in the preamble whose bodies are themselves allow-listed are trusted:
    // in text mode, in math mode, or both, depending on how the body classifies. Two rounds
    // let a macro use one defined before it.
    let defs = user_macro_definitions(text);
    // heading wrappers: the body starts with a heading command (the rest is bookkeeping:
    // \phantomsection, \addcontentsline, \markboth); the capture sees the heading
    for (name, _, body) in &defs {
        let b = body.trim_start();
        for h in HEADINGS {
            if let Some(rest) = b.strip_prefix(&format!("\\{h}")) {
                if rest.starts_with('*') || rest.starts_with('{') || rest.starts_with('[') {
                    policy.heading_macros.insert(name.clone(), (*h).to_string());
                }
            }
        }
    }
    for _round in 0..2 {
        for (name, nargs, body) in &defs {
            let mut b = body.clone();
            for k in 1..=*nargs {
                b = b.replace(&format!("#{k}"), "x");
            }
            if b.contains('#')
                || b.contains("\\def")
                || b.contains("\\let")
                || b.contains("\\global")
                || b.contains("\\gdef")
                || b.contains("\\xdef")
                || b.contains("\\newcommand")
                || b.contains("\\renewcommand")
            {
                continue;
            }
            if classify_source(&format!("x {b} x"), policy).1.is_empty() {
                policy.trusted_macros.insert(name.clone());
            }
            if classify_source(&format!("$x {b} x$"), policy).1.is_empty() {
                policy.trusted_math.insert(name.clone());
            }
        }
    }
}

/// Adds the preamble's own environments to the policy, as block units or as inline material.
fn trust_user_environments(policy: &mut Policy, text: &str) {
    // Environments defined in the preamble: allowed when their begin and end code is
    // allow-listed. One that opens a block environment (quote, itemize, center, a theorem)
    // is a block unit itself (the capture tags it like a theorem); otherwise it is typeset
    // inside the paragraph that uses it.
    for (name, nargs, begin, end) in user_environment_definitions(text) {
        let mut b = format!("{begin} x {end}");
        for k in 1..=nargs {
            b = b.replace(&format!("#{k}"), "x");
        }
        if b.contains('#')
            || b.contains("\\def")
            || b.contains("\\let")
            || b.contains("\\global")
            || b.contains("\\gdef")
            || b.contains("\\xdef")
            || b.contains("\\newcommand")
            || b.contains("\\renewcommand")
            || name.is_empty()
        {
            continue;
        }
        let (shape, mut reasons) = classify_source(&b, policy);
        // declarations in the begin code are local to the environment's group
        reasons.retain(|r| !matches!(r, Reason::SizeDeclarationOutsideGroup(_)));
        if !reasons.is_empty() {
            continue;
        }
        match shape {
            UnitShape::Env(_) => {
                policy.theorem_envs.insert(name);
            }
            UnitShape::Par => {
                policy.user_inner_envs.insert(name);
            }
            UnitShape::Heading(_) => {}
        }
    }
}

/// `\usepackage[opts]{a,b}` / `\RequirePackage` occurrences as (options, names), and
/// `\documentclass`: commands a class provides are allowed like a package's.
pub(super) fn usepackages(text: &str) -> Vec<(Vec<String>, Vec<String>)> {
    let mut out = Vec::new();
    for key in ["\\usepackage", "\\RequirePackage", "\\documentclass"] {
        let mut idx = 0;
        while let Some(p) = text[idx..].find(key) {
            let s = idx + p + key.len();
            let rest = text[s..].trim_start();
            let mut opts = Vec::new();
            let mut r = rest;
            if let Some(after) = r.strip_prefix('[') {
                if let Some(e) = after.find(']') {
                    opts = after[..e]
                        .split(',')
                        .map(|o| o.split('=').next().unwrap_or("").trim().to_string())
                        .collect();
                    r = after[e + 1..].trim_start();
                }
            }
            if let Some(after) = r.strip_prefix('{') {
                if let Some(e) = after.find('}') {
                    let names = after[..e]
                        .split(',')
                        .map(|n| n.trim().to_string())
                        .filter(|n| !n.is_empty())
                        .collect();
                    out.push((opts, names));
                }
            }
            idx = s;
        }
    }
    out
}

/// Simple macro definitions in the preamble: `\newcommand{\name}[n]{body}` (also `*`,
/// `\renewcommand`, `\providecommand`, an optional default argument), `\DeclareMathOperator`
/// and parameterless `\def\name{body}`. Returns (name, argument count, body).
/// `\newenvironment{name}[n][default]{begin}{end}` / `\renewenvironment` in `text`:
/// (name, argument count, begin code, end code).
pub(super) fn user_environment_definitions(text: &str) -> Vec<(String, usize, String, String)> {
    fn balanced(text: &str, open_at: usize) -> Option<usize> {
        let b = text.as_bytes();
        if b.get(open_at) != Some(&b'{') {
            return None;
        }
        let mut depth = 0i32;
        let mut i = open_at;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }
    let mut out = Vec::new();
    let b = text.as_bytes();
    for key in ["\\newenvironment", "\\renewenvironment"] {
        let mut idx = 0;
        while let Some(p) = text[idx..].find(key) {
            let mut i = idx + p + key.len();
            idx = i;
            if i < b.len() && b[i] == b'*' {
                i += 1;
            }
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            let Some(e) = balanced(text, i) else { continue };
            let name = text[i + 1..e].trim().to_string();
            i = e + 1;
            let mut nargs = 0usize;
            let mut has_default = false;
            for round in 0..2 {
                while i < b.len() && (b[i] == b' ' || b[i] == b'\n') {
                    i += 1;
                }
                if i < b.len() && b[i] == b'[' {
                    if let Some(k) = text[i..].find(']') {
                        if round == 0 {
                            nargs = text[i + 1..i + k].trim().parse().unwrap_or(0);
                        } else {
                            has_default = true;
                        }
                        i += k + 1;
                    }
                }
            }
            while i < b.len() && (b[i] == b' ' || b[i] == b'\n') {
                i += 1;
            }
            let Some(e1) = balanced(text, i) else {
                continue;
            };
            let begin = text[i + 1..e1].to_string();
            i = e1 + 1;
            while i < b.len() && (b[i] == b' ' || b[i] == b'\n') {
                i += 1;
            }
            let Some(e2) = balanced(text, i) else {
                continue;
            };
            let end = text[i + 1..e2].to_string();
            out.push((name, nargs.max(if has_default { 1 } else { 0 }), begin, end));
        }
    }
    out
}

pub(super) fn user_macro_definitions(text: &str) -> Vec<(String, usize, String)> {
    fn balanced(text: &str, open_at: usize) -> Option<usize> {
        let b = text.as_bytes();
        if b.get(open_at) != Some(&b'{') {
            return None;
        }
        let mut depth = 0i32;
        let mut i = open_at;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 1,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }
    let mut out = Vec::new();
    let b = text.as_bytes();
    for key in [
        "\\newcommand",
        "\\renewcommand",
        "\\providecommand",
        "\\DeclareMathOperator",
        "\\def",
    ] {
        let mut idx = 0;
        while let Some(p) = text[idx..].find(key) {
            let mut i = idx + p + key.len();
            idx = i;
            if i < b.len() && b[i] == b'*' {
                i += 1;
            }
            // control sequence, braced or bare
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            let braced = i < b.len() && b[i] == b'{';
            if braced {
                i += 1;
            }
            if i >= b.len() || b[i] != b'\\' {
                continue;
            }
            let mut j = i + 1;
            while j < b.len() && (b[j] as char).is_ascii_alphabetic() {
                j += 1;
            }
            if j == i + 1 {
                continue;
            }
            let name = text[i + 1..j].to_string();
            i = j;
            if braced {
                if i < b.len() && b[i] == b'}' {
                    i += 1;
                } else {
                    continue;
                }
            }
            if key == "\\DeclareMathOperator" {
                out.push((name, 0, "\\operatorname{x}".to_string()));
                continue;
            }
            if key == "\\def" {
                // only the parameterless form
                if i < b.len() && b[i] == b'{' {
                    if let Some(e) = balanced(text, i) {
                        out.push((name, 0, text[i + 1..e].to_string()));
                    }
                }
                continue;
            }
            let mut nargs = 0usize;
            let mut has_default = false;
            for _ in 0..2 {
                if i < b.len() && b[i] == b'[' {
                    if let Some(e) = text[i..].find(']') {
                        let inner = text[i + 1..i + e].trim();
                        if nargs == 0 && !has_default {
                            nargs = inner.parse().unwrap_or(0);
                        } else {
                            has_default = true;
                        }
                        if nargs == 0 && !has_default {
                            has_default = true;
                        }
                        i += e + 1;
                    }
                }
            }
            if i < b.len() && b[i] == b'{' {
                if let Some(e) = balanced(text, i) {
                    out.push((
                        name,
                        nargs.max(if has_default { 1 } else { 0 }),
                        text[i + 1..e].to_string(),
                    ));
                }
            }
        }
    }
    out
}
