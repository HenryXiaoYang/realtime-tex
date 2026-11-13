//! Why a unit is not eligible, and the shape a unit has.

/// Why a unit is not fast-eligible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Blank,
    UnbalancedBraces,
    UnbalancedMath,
    UnbalancedEnvironment,
    DisplayMath,
    EnvironmentBoundary,
    /// Text continues after a block environment inside the same span.
    TextAfterEnvironment,
    DisallowedEnvironment(String),
    ParagraphBreak,
    Verbatim,
    DisallowedMacro(String),
    DisallowedMathMacro(String),
    /// A macro that is only meaningful inside a particular environment (`\item`, `\caption`, `\hline`).
    MacroOutsideContext(String),
    /// A package the construct needs is not loaded (amsmath, graphicx, booktabs).
    NeedsPackage(String),
    /// A counter command before the unit's first text runs in vertical mode, before the point
    /// where the unit's counters are captured; replaying it would count twice.
    LeadingCounter(String),
    /// The span is document setup (`\newcommand`, `\setlength`, `\renewcommand{\arraystretch}`
    /// … after `\begin{document}`): it draws nothing, and the server loads it with the preamble.
    SetupStatement,
    SizeDeclarationOutsideGroup(String),
    EngineFlag(String),
    NoContext,
    NoPlacement,
    NotTopLevel,
    NonDefaultEverypar,
    /// The unit's source shape (paragraph / environment / heading) does not match the layout's unit.
    KindMismatch,
    /// The last fast compile of this unit exceeded the session's budget (ms).
    OverBudget(u64),
    /// A run-in heading (`\paragraph`, `\subparagraph`): LaTeX typesets its title at the
    /// start of the following paragraph, whose lines the unit then shares.
    RunInHeading(String),
    /// The unit reads state that its context does not carry (enumitem's `resume` and
    /// `series`: the item count of an earlier list, kept in a macro, not a counter).
    OutsideState(String),
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::DisallowedMacro(m) => {
                write!(f, "macro \\{m} is not on the fast-path allow-list")
            }
            Reason::DisallowedMathMacro(m) => {
                write!(f, "math macro \\{m} is not on the fast-path allow-list")
            }
            Reason::DisallowedEnvironment(e) => {
                write!(f, "environment {e} is not on the fast-path allow-list")
            }
            Reason::MacroOutsideContext(m) => {
                write!(f, "\\{m} outside the environment it belongs to")
            }
            Reason::NeedsPackage(p) => {
                write!(f, "needs package {p}, which the preamble does not load")
            }
            Reason::SizeDeclarationOutsideGroup(m) => write!(
                f,
                "\\{m} outside a brace group would leak past the paragraph"
            ),
            Reason::EngineFlag(s) => write!(f, "engine saw {s} in this unit"),
            Reason::SetupStatement => write!(
                f,
                "document setup (definitions, lengths): loaded with the preamble, draws nothing"
            ),
            Reason::LeadingCounter(m) => write!(
                f,
                "\\{m} before the unit's text runs before its counters are captured; put it inside the paragraph or environment, or on its own line"
            ),
            Reason::OverBudget(ms) => write!(f, "last fast compile took {ms} ms, over the budget"),
            Reason::OutsideState(what) => write!(
                f,
                "{what} reads state from earlier in the document that the fast path does not replay"
            ),
            Reason::RunInHeading(h) => write!(
                f,
                "\\{h} is a run-in heading: its title is set in the following paragraph"
            ),
            other => write!(f, "{other:?}"),
        }
    }
}

/// The shape of a unit's source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitShape {
    Par,
    Env(String),
    Heading(String),
}

/// Lexical check of a unit's source. Returns its shape and the (possibly empty) list of reasons.
/// Reasons about the unit's vocabulary (what the allow-list knows), as opposed to its structure.
/// In probe mode these do not decide eligibility: the fast result is compared with the layout.
pub fn is_vocabulary_reason(r: &Reason) -> bool {
    matches!(
        r,
        Reason::DisallowedMacro(_)
            | Reason::DisallowedMathMacro(_)
            | Reason::DisallowedEnvironment(_)
            | Reason::NeedsPackage(_)
            | Reason::MacroOutsideContext(_)
            | Reason::SizeDeclarationOutsideGroup(_)
            | Reason::LeadingCounter(_)
            | Reason::Verbatim
    )
}
