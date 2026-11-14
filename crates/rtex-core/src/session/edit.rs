//! Editing a session's documents: applying an edit, routing the touched units to the live engine or the background, and tracking the files a document pulls in.

use super::config::*;
use super::diag::*;
use super::events::*;
use super::pass_standby::*;
use super::state::*;
use super::Session;
use crate::document::{Edit, EditOutcome, FileBuf, ParaId, Revision};
use crate::eligibility::Policy;
use crate::engine::FastServer;
use anyhow::{anyhow, Result};
use rtex_dl::DisplayList;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

impl Session {
    /// The key a host's path maps to: project-relative, without a leading `./`; an absolute
    /// path inside the project root is made relative.
    pub(super) fn rel_key(&self, path: &str) -> String {
        let p = Path::new(path);
        if p.is_absolute() {
            if let Ok(rel) = p.strip_prefix(&self.shared.cfg.project_root) {
                return crate::paths::key(&rel.to_string_lossy());
            }
            if let Ok(canon) = crate::paths::canonical(p) {
                if let Ok(rel) = canon.strip_prefix(&self.shared.cfg.project_root) {
                    return crate::paths::key(&rel.to_string_lossy());
                }
            }
            return path.to_string();
        }
        let path = &crate::paths::key(path);
        let mut t = path.as_str();
        while let Some(r) = t.strip_prefix("./") {
            t = r;
        }
        t.to_string()
    }

    /// Replace a whole buffer. Treated as an edit of the bytes that differ: a host that sends
    /// its buffer when it opens the file must not give every span a new id and restart the
    /// engine (an edit covering the whole file covers the preamble). The text the session
    /// already holds changes nothing (routed to the background, reason `unchanged`).
    pub fn set_document(&self, rel_path: &str, text: &str) -> Result<EditResult> {
        let key = self.rel_key(rel_path);
        let rel_path = key.as_str();
        let diff = self
            .shared
            .files
            .lock()
            .get(rel_path)
            .map(|f| crate::document::diff_edit(&f.text, text));
        if diff.is_none() {
            // the snapshot writes the file at this key
            if !crate::paths::inside_project(rel_path) {
                return Err(anyhow!("{rel_path} is outside the project"));
            }
            let rev = self.shared.source_revision.fetch_add(1, Ordering::SeqCst) + 1;
            let fb = FileBuf::with_rules(
                text,
                &mut self.shared.ids.lock(),
                rev,
                self.shared.policy.lock().segment_rules(),
            );
            self.shared.files.lock().insert(rel_path.to_string(), fb);
            self.schedule_background();
            let edit_id = self.shared.edit_counter.fetch_add(1, Ordering::SeqCst) + 1;
            return Ok(EditResult {
                edit_id,
                source_revision: rev,
                outcome: EditOutcome::default(),
                routed: "background".into(),
                reasons: vec!["new file".into()],
                host_us: [0, 0, 0],
            });
        }
        match diff.flatten() {
            Some(edit) => self.apply_edit(rel_path, edit),
            None => Ok(EditResult {
                edit_id: self.shared.edit_counter.fetch_add(1, Ordering::SeqCst) + 1,
                source_revision: self.shared.source_revision.load(Ordering::SeqCst),
                outcome: EditOutcome::default(),
                routed: "background".into(),
                reasons: vec!["unchanged".into()],
                host_us: [0, 0, 0],
            }),
        }
    }

    pub fn apply_edit(&self, rel_path: &str, edit: Edit) -> Result<EditResult> {
        let key = self.rel_key(rel_path);
        let rel_path = key.as_str();
        let t_start = Instant::now();
        let edit_id = self.shared.edit_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let rev = self.shared.source_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let is_setup = |text: &str| {
            crate::eligibility::setup_statements(&crate::eligibility::strip_comments(text))
                .is_some()
        };
        let (mut outcome, setup_span) = {
            let mut files = self.shared.files.lock();
            let fb = files
                .get_mut(rel_path)
                .ok_or_else(|| anyhow!("unknown file {rel_path}"))?;
            let (start, end) = (edit.start_byte, edit.end_byte);
            if start > end || !fb.text.is_char_boundary(start) || !fb.text.is_char_boundary(end) {
                return Err(anyhow!(
                    "bad edit range {start}..{end} in {rel_path}: the file has {} bytes and \
                     offsets must fall on UTF-8 character boundaries",
                    fb.text.len()
                ));
            }
            // a setup span (definitions, lengths after \begin{document}) before or after the
            // edit: the server's preamble changes
            let was_setup = fb.spans.iter().any(|sp| {
                sp.range.start <= edit.end_byte
                    && edit.start_byte <= sp.range.end
                    && is_setup(&fb.text[sp.range.clone()])
            });
            let outcome = fb.apply(&edit, &mut self.shared.ids.lock(), rev);
            let now_setup = outcome
                .touched
                .iter()
                .chain(outcome.added.iter())
                .any(|id| fb.span_text(*id).map(is_setup).unwrap_or(false));
            (outcome, was_setup || now_setup)
        };
        if setup_span || self.shared.preamble_inputs.lock().contains(rel_path) {
            outcome.preamble_changed = true;
        }
        if outcome.preamble_changed {
            self.on_preamble_edit(rel_path, rev);
            return Ok(EditResult {
                edit_id,
                source_revision: rev,
                outcome,
                routed: "preamble".into(),
                reasons: vec!["preamble changed".into()],
                host_us: [t_start.elapsed().as_micros() as u64, 0, 0],
            });
        }
        let t_seg = t_start.elapsed();
        // a new \input/\include line: load the file so its paragraphs get spans (and contexts
        // from the next pass)
        if outcome
            .touched
            .iter()
            .chain(outcome.added.iter())
            .any(|id| {
                let files = self.shared.files.lock();
                files
                    .get(rel_path)
                    .and_then(|fb| fb.span_text(*id))
                    .map(|t| !crate::document::input_targets(t).is_empty())
                    .unwrap_or(false)
            })
        {
            self.load_new_inputs();
        }
        // Every body span the edit touched or created is routed (a split compiles both halves,
        // a merge the merged paragraph, a fresh paragraph borrows a context); the first one in
        // document order decides the reported outcome. Removed spans are announced as empty
        // updates so hosts clear what they drew for them.
        let (requests, primary, any_background) =
            self.route_touched(rel_path, rev, edit_id, &outcome)?;
        let t_elig = t_start.elapsed();
        let Some((primary_id, primary_live, primary_reasons)) = primary else {
            // nothing routable (e.g. everything deleted): background only
            self.note_bg_change(rel_path, rev, &outcome);
            self.schedule_background();
            self.shared
                .events
                .send(Event::BackgroundScheduled {
                    par_id: None,
                    reasons: vec!["paragraph boundaries changed".into()],
                    edit_id,
                })
                .ok();
            return Ok(EditResult {
                edit_id,
                source_revision: rev,
                outcome,
                routed: "background".into(),
                reasons: vec!["paragraph boundaries changed".into()],
                host_us: [t_seg.as_micros() as u64, 0, 0],
            });
        };
        for req in requests {
            self.dispatch_fast(req);
        }
        // a boundary change always needs a pass (pagination of what follows); so does a span
        // the fast path could not take
        if any_background || !outcome.added.is_empty() || !outcome.removed.is_empty() {
            self.note_bg_change(rel_path, rev, &outcome);
            self.schedule_background();
        }
        let host_us = [
            t_seg.as_micros() as u64,
            (t_elig - t_seg).as_micros() as u64,
            (t_start.elapsed() - t_elig).as_micros() as u64,
        ];
        if primary_live {
            Ok(EditResult {
                edit_id,
                source_revision: rev,
                outcome,
                routed: "fast".into(),
                reasons: primary_reasons,
                host_us,
            })
        } else {
            self.shared
                .events
                .send(Event::BackgroundScheduled {
                    par_id: Some(primary_id),
                    reasons: primary_reasons.clone(),
                    edit_id,
                })
                .ok();
            Ok(EditResult {
                edit_id,
                source_revision: rev,
                outcome,
                routed: "background".into(),
                reasons: primary_reasons,
                host_us: [host_us[0], host_us[1], 0],
            })
        }
    }

    /// Hand a fast request to the server: written straight to its stdin when the server is idle
    /// and already holds the context (no engine-thread wake-up on the critical path), queued for
    /// the engine thread otherwise. Queued requests coalesce per paragraph.
    pub(super) fn dispatch_fast(&self, req: FastRequest) {
        let mut link = self.shared.link.lock();
        let direct = !req.probe
            && !link.probing
            && link.writer.is_some()
            && link.inflight.is_none()
            && link.generation == req.versions.engine_generation
            && link.context_rev_sent == req.versions.context_revision
            && link.contexts_sent.contains(&req.seq);
        if direct {
            let req_id = link.next_req;
            link.next_req += 1;
            let frame = FastServer::encode_compile(req_id, req.seq, &req.source, &req.pics);
            let t0 = Instant::now();
            use std::io::Write;
            if link.writer.as_mut().unwrap().write_all(&frame).is_ok() {
                link.inflight = Some(InFlight { req, t0 });
                drop(link);
                // wake the engine thread so it is spinning on the FIFO when the result lands
                self.shared.pending_signal.0.send(()).ok();
                return;
            }
            link.writer = None; // broken pipe: the engine thread restarts the server
        }
        drop(link);
        self.shared.pending.lock().insert(req.par_id, req);
        self.shared.pending_signal.0.send(()).ok();
    }

    pub(super) fn note_bg_change(&self, rel_path: &str, rev: Revision, outcome: &EditOutcome) {
        let files = self.shared.files.lock();
        let line = files
            .get(rel_path)
            .and_then(|fb| {
                outcome
                    .touched
                    .iter()
                    .chain(outcome.added.iter())
                    .filter_map(|id| fb.span(*id))
                    .map(|s| fb.line_range(s).0)
                    .min()
            })
            .unwrap_or(0);
        self.shared
            .bg_change
            .lock()
            .entry(rel_path.to_string())
            .or_default()
            .push((rev, line));
    }

    /// Load from disk every file the tracked files now `\input` that is not tracked yet.
    /// Returns the number of files added.
    pub(super) fn load_new_inputs(&self) -> usize {
        let mut files = self.shared.files.lock();
        let rules = self.shared.policy.lock().segment_rules();
        let mut added = 0;
        loop {
            let wanted: Vec<String> = files
                .values()
                .flat_map(|fb| crate::document::input_targets(&fb.text))
                .filter(|t| !files.contains_key(t))
                .collect();
            if wanted.is_empty() || files.len() > 256 {
                break;
            }
            let mut any = false;
            for rel in wanted {
                if files.contains_key(&rel) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(self.shared.cfg.project_root.join(&rel))
                else {
                    // missing on disk: remember it as empty so we do not retry on every edit;
                    // the compiler reports it
                    files.insert(
                        rel,
                        FileBuf::with_rules("", &mut self.shared.ids.lock(), 1, rules.clone()),
                    );
                    continue;
                };
                let rev = self.shared.source_revision.load(Ordering::SeqCst);
                files.insert(
                    rel,
                    FileBuf::with_rules(&text, &mut self.shared.ids.lock(), rev, rules.clone()),
                );
                added += 1;
                any = true;
            }
            if !any {
                break;
            }
        }
        if added > 0 {
            *self.shared.inputted.lock() = crate::document::inputted_files(&texts_of(&files));
        }
        added
    }

    /// An edit changed the preamble: rebuild the policy from it, re-segment the files whose rules
    /// changed, restart the live engine and schedule a pass.
    fn on_preamble_edit(&self, rel_path: &str, rev: Revision) {
        self.shared.preamble_revision.store(rev, Ordering::SeqCst);
        self.load_new_inputs();
        {
            let files = self.shared.files.lock();
            let texts = texts_of(&files);
            let preamble = effective_preamble(&texts, &self.shared.cfg.main_file);
            *self.shared.preamble_inputs.lock() =
                preamble_input_set(&texts, &self.shared.cfg.main_file);
            *self.shared.inputted.lock() = crate::document::inputted_files(&texts);
            let mut policy = Policy::from_preamble(
                &preamble,
                &self.shared.cfg.trusted_macros,
                &self.shared.cfg.unit_envs,
            );
            policy.permissive = self.shared.cfg.eligibility == EligibilityMode::Probe;
            policy.math_owner_attribute = self.shared.cfg.math_owner_attribute;
            let rules = policy.segment_rules();
            *self.shared.policy.lock() = policy;
            drop(files);
            let mut files = self.shared.files.lock();
            for fb in files.values_mut() {
                if fb.rules != rules {
                    fb.rules = rules.clone();
                    fb.resegment(&mut self.shared.ids.lock(), rev);
                }
            }
        }
        self.shared
            .bg_change
            .lock()
            .entry(rel_path.to_string())
            .or_default()
            .push((rev, 0));
        // restart the engine with the new preamble, drop overlays and the standby, schedule a pass
        if let Some(w) = self.shared.standby.lock().take() {
            w.kill();
        }
        self.shared.live.lock().on_preamble_change();
        self.shared.engine_generation.fetch_add(1, Ordering::SeqCst);
        self.shared.pending.lock().clear();
        self.shared.pending_signal.0.send(()).ok();
        self.schedule_background();
    }

    /// Routes every body span the edit touched or created, in document order, and announces the
    /// removed ones. Returns the live requests, the first span with whether it went live and why
    /// not, and whether any span needs the background.
    #[allow(clippy::type_complexity)]
    fn route_touched(
        &self,
        rel_path: &str,
        rev: Revision,
        edit_id: u64,
        outcome: &EditOutcome,
    ) -> Result<(Vec<FastRequest>, Option<(ParaId, bool, Vec<String>)>, bool)> {
        let mut requests: Vec<FastRequest> = Vec::new();
        let mut primary: Option<(ParaId, bool, Vec<String>)> = None;
        let mut any_background = false;
        {
            let files = self.shared.files.lock();
            let fb = files
                .get(rel_path)
                .ok_or_else(|| anyhow!("unknown file {rel_path}"))?;
            let layout = self.shared.layout.lock();
            let policy = self.shared.policy.lock();
            let mut cands: Vec<ParaId> = outcome
                .touched
                .iter()
                .chain(outcome.added.iter())
                .copied()
                .collect();
            cands.sort_by_key(|id| fb.span(*id).map(|sp| sp.range.start).unwrap_or(usize::MAX));
            cands.dedup();
            for id in cands {
                let Some(span) = fb.span(id) else { continue };
                let (req, reasons) =
                    self.route_span(rel_path, &files, fb, span, rev, edit_id, &layout, &policy);
                let live = req.is_some();
                any_background |= !live;
                if !live && self.shared.cfg.debug_dir.is_some() {
                    debug_request_line(
                        &self.shared,
                        &format!("par {} refused: {}", id.0, reasons.join("; ")),
                    );
                }
                if primary.is_none() {
                    primary = Some((id, live, reasons));
                }
                if let Some(r) = req {
                    requests.push(r);
                }
            }
            if !outcome.removed.is_empty() {
                let versions = Versions {
                    source_revision: rev,
                    context_revision: layout.context_revision,
                    engine_generation: self.shared.engine_generation.load(Ordering::SeqCst),
                    layout_version: layout.layout_version,
                };
                for id in &outcome.removed {
                    self.shared
                        .events
                        .send(Event::ParagraphUpdate {
                            par_id: *id,
                            edit_id,
                            versions,
                            status: "removed".into(),
                            reasons: vec![],
                            fragments: vec![],
                            pagination_stale: true,
                            context_stale: false,
                            dl: DisplayList::default(),
                            diagnostics: vec![],
                            timing: Timing::default(),
                        })
                        .ok();
                }
            }
        }
        Ok((requests, primary, any_background))
    }
}
