//! Conservative fast-path eligibility: an **allow-list** over a unit's source.
//!
//! A unit (paragraph, block environment or heading) may take the per-keystroke path only when
//! it can be typeset in isolation with no effect on, and no dependence on, state outside the
//! unit beyond what the captured context replays (parameters, fonts, counters, labels). Anything
//! not explicitly allowed — including every macro defined in the preamble — sends the unit to the
//! background path. See docs/how-it-works.md and docs/live-editing.md.

mod engine;
mod lex;
mod policy;
mod reason;
mod scan;
mod scan_envs;
mod scan_macros;
mod setup;
mod shape;
#[cfg(test)]
mod tests;
mod vocab_envs;
mod vocab_math;
mod vocab_packages;
mod vocab_text;

pub use crate::document::BLOCK_ENVS;
pub use engine::{check_engine, check_engine_unit, everypar_allowed};
pub use lex::strip_comments;
pub use policy::Policy;
pub use reason::{is_vocabulary_reason, Reason, UnitShape};
pub use setup::{setup_prefix, setup_statements, SETUP_MACROS};
pub use shape::{content_start, has_content, labels_only, runs_into_title, vertical_prefix_len};
pub use vocab_envs::{AMSMATH_DISPLAY_ENVS, DISPLAY_ENVS, FLOAT_ENVS};

use scan::*;
use shape::*;
use vocab_envs::*;

pub fn classify_source(src: &str, policy: &Policy) -> (UnitShape, Vec<Reason>) {
    classify_source_with(src, policy, policy.permissive)
}

/// `classify_source` with the permissive flag given explicitly (`false`: every reason,
/// vocabulary included).
pub fn classify_source_with(
    src: &str,
    policy: &Policy,
    permissive: bool,
) -> (UnitShape, Vec<Reason>) {
    let text = strip_comments(src);
    if text.trim().is_empty() {
        return (UnitShape::Par, vec![Reason::Blank]);
    }
    if setup_statements(&text).is_some() {
        return (UnitShape::Par, vec![Reason::SetupStatement]);
    }
    let (shape, heading_base) = unit_shape(&text, policy);
    // a run-in heading whose span holds the paragraph or list that sets its title is typeset
    // like any heading; alone, it would typeset nothing
    let run_in = match &shape {
        UnitShape::Heading(h)
            if RUN_IN_HEADINGS.contains(&h.as_str()) && !runs_into_title(&text) =>
        {
            Some(h.clone())
        }
        _ => None,
    };
    let mut scan = Scan {
        text: &text,
        b: text.as_bytes(),
        policy,
        shape,
        heading_base,
        reasons: Vec::new(),
        i: 0,
        depth: 0,
        stack: vec![Frame {
            mode: Mode::Text,
            env: None,
            depth_at_entry: 0,
        }],
        env_depth: 0,
        block_env_closed_at: None,
        text_group_pending: false,
        line_start: 0,
    };
    scan.hard_stops();
    scan.leading_counter();
    scan.run();
    if let Some(h) = run_in {
        scan.push(Reason::RunInHeading(h));
    }
    scan.finish(permissive)
}

/// Lexical check of a unit's source. Returns the (possibly empty) list of reasons.
pub fn check_source(src: &str, policy: &Policy) -> Vec<Reason> {
    classify_source(src, policy).1
}
