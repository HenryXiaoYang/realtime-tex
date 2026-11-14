//! Comparing a live result's rows with the rows the pass captured for the same unit (`rtex verify`, probes).

use super::*;

/// Compare a fast-path unit display list with the pass's rows for the same unit, which may be
/// spread over several pages. Every row gets its own (x, y) offset from its placement, so the
/// comparison is of each row's content and geometry, not of the vertical arrangement (which the
/// page builder owns). Returns (identical glyphs, compared glyphs, notes on differences).
pub fn compare_unit_rows(
    fast: &DisplayList,
    pages: &[(i64, &DisplayList)],
    uids: &[i64],
) -> (usize, usize, Vec<String>) {
    let ref_rows = reference_rows(pages, uids);
    let mut notes = Vec::new();
    if ref_rows.len() != fast.lines.len() {
        notes.push(format!(
            "row count: fast {} vs capture {}",
            fast.lines.len(),
            ref_rows.len()
        ));
    }
    if ref_rows.is_empty() || fast.lines.is_empty() {
        return (0, 0, notes);
    }
    let (mut same, mut total) = (0, 0);
    for (k, (fl, (pno, rl))) in fast.lines.iter().zip(ref_rows.iter()).enumerate() {
        let page: &DisplayList = pages
            .iter()
            .find(|(p, _)| p == pno)
            .map(|(_, d)| *d)
            .unwrap();
        let (ox, oy) = (rl.x - fl.x, rl.y - fl.y);
        // a row holding a cached picture (an image standing in for the drawing the fast path
        // typesets) is judged by its box alone: the glyphs of the picture's labels are inside
        // the image
        let is_cached = |l: &Line| {
            l.items
                .iter()
                .any(|i| matches!(i, Item::Unsupported { kind, .. } if kind == "cached_picture"))
        };
        let cached_row = is_cached(rl) || is_cached(fl);
        if cached_row {
            // the box is what is compared: a matching one counts (a unit made of pictures
            // alone has nothing else to prove itself with)
            total += 1;
            if fl.w == rl.w && fl.h == rl.h && fl.d == rl.d {
                same += 1;
            } else {
                notes.push(format!(
                    "row {} box differs (cached picture): fast ({},{},{}) capture ({},{},{})",
                    k + 1,
                    fl.w,
                    fl.h,
                    fl.d,
                    rl.w,
                    rl.h,
                    rl.d
                ));
            }
            continue;
        }
        if fl.w != rl.w || fl.h != rl.h || fl.d != rl.d || (fl.gs - rl.gs).abs() > 1e-12 {
            notes.push(format!(
                "row {} box/glue differs: fast ({},{},{} gs {}) capture ({},{},{} gs {})",
                k + 1,
                fl.w,
                fl.h,
                fl.d,
                fl.gs,
                rl.w,
                rl.h,
                rl.d,
                rl.gs
            ));
        }
        let fg: Vec<&Item> = fl
            .items
            .iter()
            .filter(|i| matches!(i, Item::Glyph { .. }))
            .collect();
        let rg: Vec<&Item> = rl
            .items
            .iter()
            .filter(|i| matches!(i, Item::Glyph { .. }))
            .collect();
        if fg.len() != rg.len() {
            notes.push(format!(
                "row {} glyph count {} vs {}",
                k + 1,
                fg.len(),
                rg.len()
            ));
        }
        for (a, b) in fg.iter().zip(rg.iter()) {
            total += 1;
            if let (
                Item::Glyph {
                    font: fa,
                    char: ca,
                    index: ia,
                    x: xa,
                    y: ya,
                    width: wa,
                    expansion: ea,
                },
                Item::Glyph {
                    font: fb,
                    char: cb,
                    index: ib,
                    x: xb,
                    y: yb,
                    width: wb,
                    expansion: eb,
                },
            ) = (a, b)
            {
                let font_ok = match (fast.font(*fa), page.font(*fb)) {
                    (Some(da), Some(db)) => da.key() == db.key(),
                    _ => fa == fb,
                };
                if font_ok
                    && ca == cb
                    && ia == ib
                    && xa + ox == *xb
                    && ya + oy == *yb
                    && wa == wb
                    && ea == eb
                {
                    same += 1;
                } else if notes.len() < 6 {
                    notes.push(format!(
                        "row {} glyph differs: fast {:?} capture {:?}",
                        k + 1,
                        a,
                        b
                    ));
                }
            }
        }
        for (kind, pred) in [
            (
                "rule",
                (|i: &&Item| matches!(i, Item::Rule { .. })) as fn(&&Item) -> bool,
            ),
            ("image", |i: &&Item| matches!(i, Item::Image { .. })),
        ] {
            let fr = fl.items.iter().filter(pred).count();
            let rr = rl.items.iter().filter(pred).count();
            if fr != rr {
                notes.push(format!("row {} {kind} count {} vs {}", k + 1, fr, rr));
            }
        }
    }
    (same, total, notes)
}

/// The rows the pass captured for a unit, with their pages.
fn reference_rows<'a>(pages: &'a [(i64, &'a DisplayList)], uids: &[i64]) -> Vec<(i64, &'a Line)> {
    // rows of every member unit (a composite unit is several capture units), in member order
    let mut keyed: Vec<((usize, i64), i64, &Line)> = Vec::new();
    for (k, uid) in uids.iter().enumerate() {
        for (pno, page) in pages {
            for l in page.rows_of(*uid) {
                keyed.push(((k, l.row), *pno, l));
            }
        }
    }
    keyed.sort_by_key(|(key, _, _)| *key);
    let ref_rows: Vec<(i64, &Line)> = keyed.into_iter().map(|(_, pno, l)| (pno, l)).collect();
    ref_rows
}
