//! Reading the current layout: a unit by id, the span a unit maps to, and the fragments (page, rows, baselines) where a unit's live result goes.

use super::*;

impl LayoutStore {
    pub fn unit(&self, id: ParaId) -> Option<&EngineUnit> {
        self.by_span.get(&id).map(|i| &self.units[*i])
    }

    /// The height of page `page` as the installed pass shipped it, when known.
    pub fn page_height(&self, page: i64) -> Option<Sp> {
        self.pages.get(&page).and_then(|d| d.page_height)
    }

    /// Does every row of `frags` sit on its page (a baseline strictly inside the page, after
    /// the top margin and before the bottom edge)? A borrowed or relative placement is a
    /// guess; one that lands above the page top or past its bottom is not trusted (the unit
    /// waits for the pass instead of drawing an overlay in the wrong place). Pages whose height
    /// is unknown are not checked.
    pub fn fragments_on_page(&self, frags: &[Fragment]) -> bool {
        for f in frags {
            let Some(h) = self.page_height(f.page) else {
                continue;
            };
            for &b in &f.baselines {
                if b <= 0 || b >= h {
                    return false;
                }
            }
        }
        true
    }

    /// What the host recorded about span `id` when the installed pass was started.
    pub fn snapshot_span(&self, id: ParaId) -> Option<&SnapshotSpan> {
        self.snapshot_index
            .get(&id)
            .map(|i| &self.snapshot_spans[*i])
    }

    /// The text of span `id` as the installed pass typeset it.
    pub fn snapshot_text(&self, id: ParaId) -> Option<&str> {
        self.snapshot_index
            .get(&id)
            .map(|i| self.snapshot_spans[*i].text.as_str())
    }

    /// Probe check: does `fast`, the fast compile of the span's snapshot text, reproduce the
    /// rows the pass shipped for its unit? `Ok` when every row has the same box, glue set and
    /// glyphs (font, char, position, width, expansion); `Err(why)` names the first difference.
    pub fn probe_check(&self, id: ParaId, fast: &DisplayList) -> Result<(), String> {
        let Some(eu) = self.unit(id) else {
            return Err("no layout unit to compare with".into());
        };
        let mut pnos: Vec<i64> = eu.captured.placements.iter().map(|pl| pl.page).collect();
        pnos.sort();
        pnos.dedup();
        let pages: Vec<(i64, &DisplayList)> = pnos
            .iter()
            .filter_map(|p| self.pages.get(p).map(|d| (*p, d)))
            .collect();
        if pages.len() != pnos.len() {
            return Err("page display lists not available".into());
        }
        let (same, total, notes) = compare_unit_rows(fast, &pages, &eu.uids);
        if let Some(n) = notes.first() {
            return Err(n.clone());
        }
        if same != total {
            return Err(format!("{} of {} glyphs differ", total - same, total));
        }
        if total == 0 {
            // an empty box against no rows proves nothing
            return Err("nothing to compare: the unit has no glyph rows".into());
        }
        Ok(())
    }

    /// Units mapped to spans, in document order: (unit, span id).
    pub fn mapped_units(&self) -> Vec<(&EngineUnit, ParaId)> {
        let mut v: Vec<(&EngineUnit, ParaId)> = self
            .units
            .iter()
            .filter_map(|u| u.span.map(|s| (u, s)))
            .collect();
        v.sort_by_key(|(u, _)| u.uid);
        v
    }

    /// Build a store from a capture of `project/main` without a session (verification tools,
    /// tests): the main file and the files it inputs are segmented like the session would, and
    /// the capture installed.
    pub fn offline(
        cap: &CaptureResult,
        project: &std::path::Path,
        main: &str,
        policy: &crate::eligibility::Policy,
    ) -> anyhow::Result<(LayoutStore, crate::document::FileSet)> {
        let texts = crate::document::load_project_files(project, main)?;
        let mut ids = crate::document::IdAllocator(0);
        let mut set = crate::document::FileSet {
            inputted: crate::document::inputted_files(&texts),
            ..Default::default()
        };
        let mut spans = Vec::new();
        // the main file first so its ids come first (tools pick "the middle paragraph" by id)
        for (name, text) in std::iter::once((main, texts[main].as_str())).chain(
            texts
                .iter()
                .filter(|(n, _)| n.as_str() != main)
                .map(|(n, t)| (n.as_str(), t.as_str())),
        ) {
            let fb =
                crate::document::FileBuf::with_rules(text, &mut ids, 1, policy.segment_rules());
            spans.extend(snapshot_spans_of(
                &fb,
                name,
                policy,
                set.inputted.contains(name),
            ));
            set.files.insert(name.to_string(), fb);
        }
        let mut store = LayoutStore::default();
        store.install(cap, spans, 1)?;
        Ok((store, set))
    }

    /// Page positions for the rows of a unit the layout has no placement for, relative to a
    /// neighbouring unit that has one. `after`: the rows follow the parent's rows, of which the
    /// parent currently shows `parent_rows` (its last fast result; its placement count when
    /// unknown) — exact for consecutive paragraphs with the same baselineskip and no parskip;
    /// otherwise the rows end one baselineskip above the parent's first row. The x is the
    /// parent's left edge (the leftmost of its rows: a display row or a centered line is not
    /// where the text starts). Always approximate.
    pub fn fragments_relative(
        &self,
        parent: ParaId,
        parent_rows: Option<i64>,
        after: bool,
        rows: &[(Sp, Sp)],
    ) -> Option<Vec<Fragment>> {
        let eu = self.unit(parent)?;
        let pl = &eu.captured.placements;
        if pl.is_empty() || rows.is_empty() {
            return None;
        }
        let bs = eu.baselineskip().max(1);
        let (_, fy) = rows[0];
        let left = pl.iter().map(|p| p.x).min().unwrap();
        let (page, ay) = if after {
            let idx = parent_rows.unwrap_or(pl.len() as i64).max(0) as usize;
            if idx < pl.len() {
                (pl[idx].page, pl[idx].y)
            } else {
                let last = pl.last().unwrap();
                (
                    last.page,
                    last.y + (idx as i64 - (pl.len() as i64 - 1)) * bs,
                )
            }
        } else {
            let (_, ly) = rows[rows.len() - 1];
            (pl[0].page, pl[0].y - bs - (ly - fy))
        };
        let ax = left;
        Some(Self::fragments_at(page, ax, ay, rows))
    }

    /// The rows of a unit placed with its first row at (`ax`, `ay`) of `page`, keeping the
    /// box's own geometry: one approximate fragment (none for a unit without rows, such as a
    /// paragraph holding only a `\label`).
    pub fn fragments_at(page: i64, ax: Sp, ay: Sp, rows: &[(Sp, Sp)]) -> Vec<Fragment> {
        let Some(&(fx, fy)) = rows.first() else {
            return Vec::new();
        };
        let xs: Vec<Sp> = rows.iter().map(|(rx, _)| ax + (rx - fx)).collect();
        let baselines: Vec<Sp> = rows.iter().map(|(_, ry)| ay + (ry - fy)).collect();
        vec![Fragment {
            page,
            first_line: 1,
            last_line: rows.len() as i64,
            x: xs[0],
            xs,
            baselines,
            approximate: true,
        }]
    }

    /// Page positions for the rows of a fast result. `rows` are the (x, baseline) of each row in
    /// the fast box's own coordinates. Within a page the rows keep the fast box's geometry,
    /// anchored at the page's first placed row; rows beyond the cached placements continue on the
    /// last page (`approximate`). Returns None when the unit has no placement; the bool says
    /// whether the pagination is stale (row count differs from the layout).
    pub fn fragments(&self, id: ParaId, rows: &[(Sp, Sp)]) -> Option<(Vec<Fragment>, bool)> {
        let eu = self.unit(id)?;
        let placements: &Vec<Placement> = &eu.captured.placements;
        if placements.is_empty() {
            return None;
        }
        let mut stale = rows.len() != placements.len();
        let mut frags = Vec::new();
        let mut i = 0usize; // placement index == fast row index while both exist
        while i < placements.len() && i < rows.len() {
            let page = placements[i].page;
            let anchor = &placements[i];
            let (ax, ay) = rows[i];
            let first = i;
            let mut xs = Vec::new();
            let mut baselines = Vec::new();
            while i < placements.len() && placements[i].page == page && i < rows.len() {
                let (rx, ry) = rows[i];
                xs.push(anchor.x + (rx - ax));
                baselines.push(anchor.y + (ry - ay));
                i += 1;
            }
            let last_page_group = i >= placements.len();
            let mut approximate = false;
            if last_page_group {
                while i < rows.len() {
                    let (rx, ry) = rows[i];
                    xs.push(anchor.x + (rx - ax));
                    baselines.push(anchor.y + (ry - ay));
                    i += 1;
                    approximate = true;
                    stale = true;
                }
            }
            frags.push(Fragment {
                page,
                first_line: first as i64 + 1,
                last_line: i as i64,
                x: xs[0],
                xs,
                baselines,
                approximate,
            });
        }
        Some((frags, stale))
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    fn frag(page: i64, baselines: Vec<Sp>) -> Fragment {
        Fragment {
            page,
            first_line: 1,
            last_line: baselines.len() as i64,
            x: 0,
            xs: baselines.iter().map(|_| 0).collect(),
            baselines,
            approximate: true,
        }
    }

    fn store_with_page(page: i64, height: Sp) -> LayoutStore {
        let mut s = LayoutStore::default();
        let dl = DisplayList {
            page_height: Some(height),
            ..DisplayList::default()
        };
        s.pages.insert(page, dl);
        s
    }

    #[test]
    fn a_placement_off_its_page_is_rejected() {
        let h = 50 * 65536 * 12; // ~12 inch page in sp, order of magnitude only
        let s = store_with_page(1, h);
        // a baseline inside the page is fine
        assert!(s.fragments_on_page(&[frag(1, vec![h / 2])]));
        // a baseline above the page top (the top-of-page overlay bug) is rejected
        assert!(!s.fragments_on_page(&[frag(1, vec![-10])]));
        assert!(!s.fragments_on_page(&[frag(1, vec![0])]));
        // a baseline past the page bottom is rejected
        assert!(!s.fragments_on_page(&[frag(1, vec![h + 10])]));
        // one bad row among good ones rejects the whole placement
        assert!(!s.fragments_on_page(&[frag(1, vec![h / 2, h + 1])]));
        // a page whose height is unknown is not checked (no false drop)
        assert!(s.fragments_on_page(&[frag(9, vec![-100])]));
    }
}
