//! Checks of a captured unit against what the live engine can replay.

use super::reason::*;
use super::vocab_envs::*;

/// `\everypar` contents the server may replay verbatim at a unit's start: the kernel's own
/// patterns (after a heading, after an environment, first paragraph of a box) and microtype's
/// `\leftprotrusion`. Everything else makes the unit background-only.
pub fn everypar_allowed(ep: &str) -> bool {
    let e = ep
        .trim_start_matches("\\leftprotrusion ")
        .trim_end_matches("\\leftprotrusion ");
    if e.is_empty() {
        return true;
    }
    const PATTERNS: &[&str] = &[
        // \@afterheading (with and without the \if@afterindent branch)
        "\\if@nobreak \\@nobreakfalse \\clubpenalty \\@M \\if@afterindent \\else {\\setbox \\z@ \\lastbox }\\fi \\else \\clubpenalty \\@clubpenalty \\everypar {}\\fi ",
        "\\if@nobreak \\@nobreakfalse \\clubpenalty \\@M \\setbox \\z@ \\lastbox \\else \\clubpenalty \\@clubpenalty \\everypar {}\\fi ",
        // \@doendpe (first paragraph after an environment)
        "{\\setbox \\z@ \\lastbox }\\everypar {}\\@endpefalse ",
        // \@setminipage (first paragraph in a float or minipage)
        "\\@minipagefalse \\everypar {}",
    ];
    PATTERNS.contains(&e)
}

/// Engine-side check of a unit from the capture: `everypar` pattern, node flags, members.
/// `run_in`: the heading's name when the capture saw it set run-in (`CapturedUnit::runin`).
pub fn check_engine_unit(
    kind: &str,
    everypar: &str,
    has_context: bool,
    rows: i64,
    flags: &std::collections::BTreeMap<String, i64>,
    run_in: Option<&str>,
) -> Vec<Reason> {
    let mut reasons = Vec::new();
    // `classify_source` reports the run-in headings it knows by name from the source
    if let Some(h) = run_in.filter(|h| !RUN_IN_HEADINGS.contains(h)) {
        reasons.push(Reason::RunInHeading(h.to_string()));
    }
    if !everypar_allowed(everypar) {
        reasons.push(Reason::NonDefaultEverypar);
    }
    if !has_context {
        reasons.push(Reason::NoContext);
    }
    if rows == 0 {
        reasons.push(Reason::NoPlacement);
    }
    let _ = kind;
    for (k, v) in flags {
        if *v == 0 {
            continue;
        }
        // inserts (footnotes), marks (running heads) and adjust material are placed by the page
        // builder and refreshed by the next layout; direction changes are not supported.
        if k == "dir" {
            reasons.push(Reason::EngineFlag(format!("{k}={v}")));
        }
    }
    reasons
}

/// Legacy per-paragraph engine check (kept for the verification tools).
pub fn check_engine(
    groupcode: &str,
    nest: i64,
    everypar: &str,
    has_begin: bool,
    flags: &std::collections::BTreeMap<String, i64>,
) -> Vec<Reason> {
    let mut reasons = Vec::new();
    if !(groupcode.is_empty() && nest == 1) {
        reasons.push(Reason::NotTopLevel);
    }
    if !everypar_allowed(everypar) {
        reasons.push(Reason::NonDefaultEverypar);
    }
    if !has_begin {
        reasons.push(Reason::NoContext);
    }
    for (k, v) in flags {
        if *v != 0 && k == "dir" {
            reasons.push(Reason::EngineFlag(format!("{k}={v}")));
        }
    }
    reasons
}
