//! Installing a finished pass as the current layout: units and their contexts, the mapping from units to source spans, and the page lists (a page whose bytes are unchanged keeps its decoded list).

use super::{EngineUnit, LayoutStore, SnapshotSpan};
use crate::capture::{CaptureResult, CapturedParagraph};
use crate::document::{ParaId, Revision};
use rtex_dl::DisplayList;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

impl LayoutStore {
    /// Install a finished capture pass. Maps units to spans by their first source line inside
    /// the snapshot's span line ranges (exactly one unit per span for a fast-eligible mapping).
    pub fn install(
        &mut self,
        cap: &CaptureResult,
        snapshot: Vec<SnapshotSpan>,
        snapshot_revision: Revision,
    ) -> anyhow::Result<Vec<i64>> {
        self.layout_version += 1;
        self.context_revision += 1;
        self.snapshot_revision = snapshot_revision;
        self.units.clear();
        self.by_span.clear();
        let per_span_count = self.units_from_capture(cap, &snapshot);
        self.merge_composite_units(per_span_count);
        let changed = self.install_pages(cap)?;
        self.snapshot_index = snapshot
            .iter()
            .enumerate()
            .map(|(i, sp)| (sp.id, i))
            .collect();
        self.snapshot_spans = snapshot;
        self.capture_dir = Some(cap.out_dir.clone());
        self.pdf = Some(cap.pdf.clone());
        let labels =
            crate::background::read_aux_labels(&cap.out_dir.join(format!("{}.aux", cap.jobname)));
        self.labels_hash = crate::document::hash_str(&format!("{labels:?}"));
        self.labels = Arc::new(labels);
        Ok(changed)
    }

    /// One engine unit per captured unit, each mapped to the snapshot span that holds its first
    /// source line. Returns how many units each span got.
    fn units_from_capture(
        &mut self,
        cap: &CaptureResult,
        snapshot: &[SnapshotSpan],
    ) -> HashMap<ParaId, usize> {
        let paras: HashMap<i64, &CapturedParagraph> =
            cap.json.paragraphs.iter().map(|p| (p.seq, p)).collect();
        let mut per_span_count: HashMap<ParaId, usize> = HashMap::new();
        for u in &cap.json.units {
            let file = u.file.clone().unwrap_or_else(|| "./main.tex".into());
            let file = crate::paths::key(&file);
            let file = file.trim_start_matches("./").to_string();
            let mut span = None;
            for s in snapshot {
                if s.file == file && u.begin_line >= s.first_line && u.begin_line <= s.last_line {
                    span = Some(s.id);
                    break;
                }
            }
            if span.is_none() {
                // a unit that begins on a blank line between two spans: it belongs to the next
                // span when it extends into it (the engine reported the paragraph a line early,
                // seen after \newpage in standby passes), otherwise to the previous one (a macro
                // at the end of its last line read ahead before producing the unit: hyperref's
                // \maketitle wrapper)
                let next = if u.kind == "par" {
                    snapshot
                        .iter()
                        .find(|s| {
                            s.file == file
                                && s.first_line > u.begin_line
                                && u.end_line.unwrap_or(u.begin_line) >= s.first_line
                        })
                        .map(|s| s.id)
                } else {
                    None
                };
                span = next.or_else(|| {
                    snapshot
                        .iter()
                        .find(|s| s.file == file && s.last_line + 1 == u.begin_line)
                        .map(|s| s.id)
                });
            }
            if let Some(id) = span {
                *per_span_count.entry(id).or_default() += 1;
            }
            let mut flags: BTreeMap<String, i64> = BTreeMap::new();
            let mut members = Vec::new();
            for seq in &u.seqs {
                if let Some(p) = paras.get(seq) {
                    for (k, v) in &p.flags {
                        *flags.entry(k.clone()).or_default() += v;
                    }
                    members.push((p.groupcode.clone(), p.nest));
                }
            }
            // the unit's body paragraph: the first member at the unit's own nesting level with a
            // para/begin record (footnote text and \parbox contents are line-broken earlier, at
            // deeper levels)
            let first_para = u
                .seqs
                .iter()
                .filter_map(|s| paras.get(s))
                .find(|p| p.nest == 1 && p.begin.is_some())
                .or_else(|| {
                    u.seqs
                        .iter()
                        .filter_map(|s| paras.get(s))
                        .find(|p| p.begin.is_some())
                })
                .or_else(|| u.seqs.first().and_then(|s| paras.get(s)))
                .map(|p| (*p).clone());
            self.units.push(EngineUnit {
                uid: u.uid,
                captured: u.clone(),
                first_para,
                flags,
                members,
                span,
                uids: vec![u.uid],
            });
        }
        per_span_count
    }

    /// Joins the paragraph units of one span into a composite unit, then indexes the spans that
    /// have exactly one unit.
    fn merge_composite_units(&mut self, mut per_span_count: HashMap<ParaId, usize>) {
        // Several consecutive paragraph units in one span (an explicit \par, a title line followed
        // by a text line) form one composite unit: the fast path typesets the span as one box, and
        // its rows are the members' rows in order. The first member provides the context.
        let mut idx_by_span: HashMap<ParaId, Vec<usize>> = HashMap::new();
        for (i, eu) in self.units.iter().enumerate() {
            if let Some(id) = eu.span {
                idx_by_span.entry(id).or_default().push(i);
            }
        }
        let mut merged_away: Vec<usize> = Vec::new();
        for (id, idxs) in idx_by_span.iter() {
            if idxs.len() < 2
                || !idxs.iter().all(|i| {
                    self.units[*i].captured.kind == "par" && self.units[*i].captured.nest == 1
                })
            {
                continue;
            }
            let mut idxs = idxs.clone();
            idxs.sort();
            let (first, rest) = idxs.split_first().unwrap();
            let mut placements = Vec::new();
            let mut seqs = Vec::new();
            let mut members = Vec::new();
            let mut flags: BTreeMap<String, i64> = BTreeMap::new();
            let mut end_line = None;
            let mut uids = Vec::new();
            let mut advanced: BTreeMap<String, i64> = BTreeMap::new();
            for i in rest {
                let u = &self.units[*i];
                if let Some(a) = &u.captured.advanced {
                    advanced.extend(a.iter().map(|(k, v)| (k.clone(), *v)));
                }
                placements.extend(u.captured.placements.iter().cloned());
                seqs.extend(u.captured.seqs.iter().cloned());
                members.extend(u.members.iter().cloned());
                for (k, v) in &u.flags {
                    *flags.entry(k.clone()).or_default() += v;
                }
                end_line = u.captured.end_line.or(end_line);
                uids.push(u.uid);
                merged_away.push(*i);
            }
            let f = &mut self.units[*first];
            f.captured.placements.extend(placements);
            for (k, p) in f.captured.placements.iter_mut().enumerate() {
                p.row = k as i64 + 1;
            }
            f.captured.rows = f.captured.placements.len() as i64;
            f.captured.seqs.extend(seqs);
            f.members.extend(members);
            for (k, v) in flags {
                *f.flags.entry(k).or_default() += v;
            }
            if end_line.is_some() {
                f.captured.end_line = end_line;
            }
            if !advanced.is_empty() {
                let mut all = f.captured.advanced.take().unwrap_or_default();
                all.extend(advanced);
                f.captured.advanced = Some(all);
            }
            f.uids.extend(uids);
            per_span_count.insert(*id, 1);
        }
        for i in merged_away {
            self.units[i].span = None;
        }
        for (i, eu) in self.units.iter_mut().enumerate() {
            if let Some(id) = eu.span {
                if per_span_count.get(&id).copied().unwrap_or(0) == 1 {
                    self.by_span.insert(id, i);
                } else {
                    eu.span = None; // ambiguous: several units for one span
                }
            }
        }
    }

    /// Reads the pass's page lists and returns the pages that changed.
    fn install_pages(&mut self, cap: &CaptureResult) -> anyhow::Result<Vec<i64>> {
        // a page whose capture bytes are unchanged keeps its decoded list (after an edit most
        // pages are): read and hash every page, decode only the others
        let mut fresh = Vec::new();
        for n in 1..=cap.json.pages {
            let bytes = cap.page_bytes(n)?;
            let h = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut h);
                h.finish()
            };
            let dl = if self.page_hashes.get(&n) == Some(&h) && self.pages.contains_key(&n) {
                None
            } else {
                Some(DisplayList::from_binary(&bytes)?)
            };
            fresh.push((n, h, dl));
        }
        let mut old_pages = std::mem::take(&mut self.pages);
        let mut changed = Vec::new();
        let mut new_hashes = BTreeMap::new();
        for (n, h, dl) in fresh {
            let dl = match dl {
                Some(dl) => {
                    changed.push(n);
                    dl
                }
                None => old_pages.remove(&n).expect("checked above"),
            };
            new_hashes.insert(n, h);
            self.pages.insert(n, dl);
        }
        for old in self.page_hashes.keys() {
            if !new_hashes.contains_key(old) {
                changed.push(*old);
            }
        }
        self.page_hashes = new_hashes;
        Ok(changed)
    }
}

#[cfg(test)]
mod install_tests {
    use super::*;

    #[test]
    fn unchanged_pages_keep_their_list() {
        let dir = std::env::temp_dir().join(format!("rtex-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = |n: i64, h: i64| {
            let json = format!(r#"{{"kind":"page","page":{n},"page_height":{h},"lines":[]}}"#);
            let dl = DisplayList::from_json(&json).unwrap();
            std::fs::write(dir.join(format!("main.rtex-page{n}.bin")), dl.to_binary()).unwrap();
        };
        let cap = |pages: i64| CaptureResult {
            out_dir: dir.clone(),
            jobname: "main".into(),
            json: crate::capture::CaptureJson {
                pages,
                ..Default::default()
            },
            pdf: dir.join("main.pdf"),
            log: dir.join("main.log"),
            wall: std::time::Duration::ZERO,
            exit_ok: true,
        };
        let mut s = LayoutStore::default();
        page(1, 100);
        page(2, 200);
        assert_eq!(s.install(&cap(2), vec![], 1).unwrap(), vec![1, 2]);
        // page 1 is unchanged and keeps its list (the marker shows it was not parsed again)
        s.pages.get_mut(&1).unwrap().depth = 7;
        page(2, 250);
        assert_eq!(s.install(&cap(2), vec![], 2).unwrap(), vec![2]);
        assert_eq!(s.pages[&1].depth, 7);
        assert_eq!(s.page_height(2), Some(250));
        // a page that went away is reported changed
        assert_eq!(s.install(&cap(1), vec![], 3).unwrap(), vec![2]);
        assert_eq!(s.pages.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
