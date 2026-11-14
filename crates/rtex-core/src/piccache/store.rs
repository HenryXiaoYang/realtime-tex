//! `PicCache`: the cache on disk. Its index, the manifest a pass reads, and absorbing what a pass recorded.

use super::{CacheEntry, PictureRef, RecordedPic};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Index {
    serial: u64,
    /// Names the PDFs of this index's lifetime (`p<epoch>-<serial>.pdf`): an index lost and
    /// rebuilt from scratch never reuses a name a reader may still hold.
    #[serde(default)]
    epoch: u64,
    entries: BTreeMap<String, CacheEntry>, // hash (decimal) -> entry
    /// `file:line` keys whose cached body did not end where the source scan said (a pass
    /// reported them mismatched), with the serial until which they are drawn, not cached.
    #[serde(default)]
    bad: BTreeMap<String, u64>,
}

pub struct PicCache {
    dir: PathBuf,
    index: Index,
}

/// Passes an entry survives without being wanted (edits that toggle a picture back and forth).
pub(super) const KEEP_PASSES: u64 = 4;

impl PicCache {
    pub fn open(dir: &Path) -> PicCache {
        // the manifest names the cached PDFs by absolute path: lualatex runs in the snapshot
        // directory, not where the host started
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        let mut index: Index = std::fs::read_to_string(dir.join("index.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if index.epoch == 0 {
            index.epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(1)
                .max(1);
        }
        PicCache { dir, index }
    }

    pub fn entries(&self) -> usize {
        self.index.entries.len()
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(
            &self.dir.join("index.json"),
            &serde_json::to_vec(&self.index)?,
        )
    }

    /// Write the manifest the next capture pass reads: every current picture the cache holds,
    /// keyed by its `file:line`. Returns the number of hits. Removes the file when there are
    /// none (the capture then draws everything).
    pub fn write_manifest(&mut self, pics: &[PictureRef], manifest: &Path) -> Result<usize> {
        self.index.serial += 1;
        let serial = self.index.serial;
        let mut m: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for p in pics.iter().filter(|p| p.cacheable) {
            if let Some(e) = self.index.entries.get_mut(&p.hash.to_string()) {
                e.last_used = serial;
            }
            if let Some(v) = self.entry_json(p) {
                m.insert(p.key.clone(), v);
            }
        }
        if m.is_empty() {
            let _ = std::fs::remove_file(manifest);
        } else {
            write_atomic(manifest, &serde_json::to_vec(&m)?)?;
        }
        let _ = self.save();
        Ok(m.len())
    }

    /// The cache entry for a picture, as the capture and the live engine read it (`pdf` by
    /// absolute path, `bbox` in PDF user space, `state` to compare, `end_line` of the source).
    pub fn entry_json(&self, p: &PictureRef) -> Option<serde_json::Value> {
        if !p.cacheable || self.index.bad.contains_key(&p.key) {
            return None;
        }
        let e = self.index.entries.get(&p.hash.to_string())?;
        if e.env != p.env || !self.dir.join(&e.pdf).exists() {
            return None;
        }
        Some(serde_json::json!({
            "env": e.env,
            "state": e.state,
            "end_line": p.end_line,
            "pdf": self.dir.join(&e.pdf).to_string_lossy(),
            "page": e.page, "bbox": e.bbox, "w": e.w, "h": e.h, "d": e.d,
        }))
    }

    /// After a pass: remember every current picture the pass drew (its page of `pass_pdf`,
    /// extracted into the cache), forget the pictures the pass reported as mismatched
    /// (`stale_keys`: a cached body that did not end where the source scan said), evict what
    /// no picture wanted for a while, drop PDFs nothing references. Returns the number of new
    /// entries.
    pub fn absorb(
        &mut self,
        pics: &[PictureRef],
        recorded: &BTreeMap<String, RecordedPic>,
        stale_keys: &[String],
        pass_pdf: &Path,
    ) -> Result<usize> {
        let serial = self.index.serial;
        // a mismatched key is drawn for a while instead of cached again right away (the
        // mismatch would repeat on every other pass)
        self.index.bad.retain(|_, until| *until >= serial);
        for k in stale_keys {
            if let Some(p) = pics.iter().find(|p| &p.key == k) {
                self.index.entries.remove(&p.hash.to_string());
            }
            self.index.bad.insert(k.clone(), serial + KEEP_PASSES);
        }
        let mut new: Vec<(&PictureRef, &RecordedPic)> = Vec::new();
        for p in pics
            .iter()
            .filter(|p| p.cacheable && !self.index.bad.contains_key(&p.key))
        {
            let Some(r) = recorded.get(&p.key) else {
                continue;
            };
            if r.env != p.env || r.w <= 0 || r.h + r.d <= 0 {
                continue;
            }
            // an entry drawn under another state (font, color, width), or whose PDF is gone,
            // is replaced
            if self
                .index
                .entries
                .get(&p.hash.to_string())
                .map(|e| e.state == r.state && self.dir.join(&e.pdf).exists())
                .unwrap_or(false)
            {
                continue;
            }
            new.push((p, r));
        }
        let added = new.len();
        if !new.is_empty() {
            std::fs::create_dir_all(&self.dir)?;
            let name = format!("p{}-{serial}.pdf", self.index.epoch);
            // only the pages that carry new pictures (a 100-page PDF per edited picture
            // would fill the disk); page numbers are remapped onto the extract
            let pages: Vec<i64> = new
                .iter()
                .map(|(_, r)| r.page)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let remap: BTreeMap<i64, i64> =
                match extract_pages(pass_pdf, &pages, &self.dir.join(&name)) {
                    Ok(()) => pages
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (*p, i as i64 + 1))
                        .collect(),
                    Err(e) => {
                        log::warn!(
                            "picture cache: page extraction failed ({e:#}), copying the pass PDF"
                        );
                        std::fs::copy(pass_pdf, self.dir.join(&name)).with_context(|| {
                            format!("copying {} into the picture cache", pass_pdf.display())
                        })?;
                        pages.iter().map(|p| (*p, *p)).collect()
                    }
                };
            for (p, r) in new {
                // PDF user space: x unchanged (sp of a bp == sp), y measured from the page bottom
                let bbox = [
                    r.x,
                    r.page_height - (r.y + r.d),
                    r.x + r.w,
                    r.page_height - (r.y - r.h),
                ];
                self.index.entries.insert(
                    p.hash.to_string(),
                    CacheEntry {
                        env: p.env.clone(),
                        state: r.state.clone(),
                        pdf: name.clone(),
                        page: remap[&r.page],
                        bbox,
                        w: r.w,
                        h: r.h,
                        d: r.d,
                        last_used: serial,
                    },
                );
            }
        }
        // eviction: entries no current picture wants for KEEP_PASSES passes
        self.index
            .entries
            .retain(|_, e| e.last_used + KEEP_PASSES >= serial);
        // PDFs nothing references any more
        let referenced: BTreeSet<&str> = self
            .index
            .entries
            .values()
            .map(|e| e.pdf.as_str())
            .collect();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if name.starts_with('p')
                    && name.ends_with(".pdf")
                    && !referenced.contains(name.as_str())
                {
                    let _ = std::fs::remove_file(ent.path());
                }
            }
        }
        self.save()?;
        Ok(added)
    }
}

/// Write `data` to `path` through a temporary file and a rename: a reader (the live engine's
/// index lookup, the capture's manifest read) never sees a torn file.
pub(super) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))
}

/// Write the given pages (1-based, ascending) of `src` as `dst`, in that order.
pub(super) fn extract_pages(src: &Path, pages: &[i64], dst: &Path) -> Result<()> {
    let mut doc = lopdf::Document::load(src).context("loading the pass PDF")?;
    let all: Vec<u32> = doc.get_pages().keys().copied().collect();
    let keep: BTreeSet<u32> = pages.iter().map(|p| *p as u32).collect();
    if !keep.iter().all(|p| all.contains(p)) {
        anyhow::bail!("page out of range");
    }
    let delete: Vec<u32> = all.into_iter().filter(|p| !keep.contains(p)).collect();
    doc.delete_pages(&delete);
    doc.prune_objects();
    doc.renumber_objects();
    doc.save(dst).context("writing the page extract")?;
    Ok(())
}
