//! Macros a loaded package adds to the vocabulary.

use super::policy::*;

/// Macros provided by a package: allowed when the package is loaded, `NeedsPackage` otherwise.
/// The document class counts as a package of its name.
pub(super) const PKG_MACROS: &[(&str, &[&str])] = &[
    // list label setup, local to the list
    ("IEEEtran", &["IEEEsetlabelwidth", "IEEEusemathlabelsep"]),
    (
        "hyperref",
        &[
            "href",
            "url",
            "autoref",
            "nameref",
            "hyperref",
            "phantomsection",
            "texorpdfstring",
        ],
    ),
    ("caption", &["captionof", "captionof*"]),
    (
        "graphicx",
        &[
            "scalebox",
            "rotatebox",
            "resizebox",
            "resizebox*",
            "reflectbox",
        ],
    ),
    ("xspace", &["xspace"]),
    ("nicefrac", &["nicefrac"]),
    ("units", &["nicefrac"]),
    ("mhchem", &["ce"]),
    ("csquotes", &["enquote", "textquote", "foreignquote"]),
    ("pifont", &["ding"]),
    ("subfig", &["subfloat"]),
    (
        "biblatex",
        &[
            "parencite",
            "textcite",
            "autocite",
            "footcite",
            "citeauthor",
            "citetitle",
            "citeyear",
            "citedate",
            "Parencite",
            "Textcite",
            "Autocite",
            "Citeauthor",
            "smartcite",
            "supercite",
            "fullcite",
            "footfullcite",
            "nocite",
        ],
    ),
    (
        "natbib",
        &[
            "citep",
            "citet",
            "citealp",
            "citealt",
            "citeauthor",
            "citeyear",
            "citeyearpar",
            "citenum",
            "Citep",
            "Citet",
            "citetext",
        ],
    ),
    (
        "siunitx",
        &[
            "SI", "si", "num", "SIrange", "numrange", "qty", "unit", "ang", "qtyrange", "numlist",
            "qtylist",
        ],
    ),
    (
        "ulem",
        &[
            "sout",
            "uline",
            "uwave",
            "xout",
            "dashuline",
            "dotuline",
            "uuline",
        ],
    ),
    ("soul", &["hl", "so", "ul", "st", "caps"]),
    ("listings", &["lstinline"]),
    ("multirow", &["multirow"]),
    (
        "colortbl",
        &["rowcolor", "cellcolor", "rowcolors", "arrayrulecolor"],
    ),
    ("cancel", &["cancel", "bcancel", "xcancel", "cancelto"]),
    ("bm", &["bm"]),
    (
        "amssymb",
        &[
            "mathbb",
            "checkmark",
            "square",
            "blacksquare",
            "lesssim",
            "gtrsim",
            "nexists",
            "varnothing",
            "therefore",
            "because",
        ],
    ),
];

/// Is `name` a package macro? `Some(Ok(()))` when one of the packages providing it is loaded,
/// `Some(Err(first provider))` when none is, `None` for macros no package list names.
pub(super) fn package_macro(name: &str, policy: &Policy) -> Option<Result<(), &'static str>> {
    let mut first: Option<&'static str> = None;
    for (pkg, ms) in PKG_MACROS {
        if ms.contains(&name) {
            if policy.has_package(pkg) {
                return Some(Ok(()));
            }
            first.get_or_insert(pkg);
        }
    }
    first.map(Err)
}
