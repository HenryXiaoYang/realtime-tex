//! Environment, heading and float vocabulary.

/// Picture environments (`piccache::PICTURE_ENVS`): block units when they start the unit,
/// inline boxes after text, in either case with vocabulary the allow-list does not cover.
pub(super) use crate::piccache::PICTURE_ENVS;

/// Environments whose body is opaque (verbatim material): skipped to `\end{name}`.
pub(super) const OPAQUE_ENVS: &[&str] =
    &["verbatim", "verbatim*", "lstlisting", "Verbatim", "alltt"];

/// Block environments that form units (or end a paragraph unit) and may nest.
pub use crate::document::BLOCK_ENVS;

pub const FLOAT_ENVS: &[&str] = &["figure", "figure*", "table", "table*"];

/// Display-math environments (inside paragraph units).
pub const DISPLAY_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "displaymath",
    "eqnarray",
    "eqnarray*",
];

pub const AMSMATH_DISPLAY_ENVS: &[&str] = &[
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "flalign",
    "flalign*",
    "alignat",
    "alignat*",
];

/// Environments allowed inside units (not units themselves).
pub(super) const INNER_ENVS: &[&str] = &[
    "tabular",
    "tabular*",
    "tabularx",
    "minipage",
    "subfigure",
    "subtable",
    "math",
    "em",
    "small",
    "footnotesize",
    "scriptsize",
    "large",
    "Large",
    "bfseries",
    "itshape",
    "sloppypar",
];

pub(super) const AMSMATH_INNER_ENVS: &[&str] = &[
    "split",
    "aligned",
    "gathered",
    "cases",
    "pmatrix",
    "bmatrix",
    "vmatrix",
    "Vmatrix",
    "matrix",
    "smallmatrix",
    "array",
    "subarray",
];

pub(super) const LIST_ENVS: &[&str] = &["itemize", "enumerate", "description", "IEEEdescription"];

pub(super) const TABULAR_MACROS: &[&str] = &["hline", "cline", "multicolumn", "multirow"];

pub(super) const BOOKTABS_MACROS: &[&str] = &[
    "toprule",
    "midrule",
    "bottomrule",
    "cmidrule",
    "addlinespace",
    "specialrule",
];

pub(super) const FLOAT_MACROS: &[&str] = &["caption", "caption*"];

pub(super) const HEADINGS: &[&str] = &[
    "part",
    "chapter",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "subparagraph",
];

/// Headings the standard classes set run-in, recognised by name in the source (a heading the
/// layout has not seen yet). Other classes' run-in headings are recognised from the capture
/// (`check_engine_unit`).
pub(super) const RUN_IN_HEADINGS: &[&str] = &["paragraph", "subparagraph"];
