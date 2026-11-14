//! Where passes run and the standby engine for the next one: the two alternating pass directories, and a LuaLaTeX that loads the current preamble while the user types (`passes` runs it).

use super::events::*;
use super::passes::*;
use super::state::*;
use crate::background::WarmEngine;
use crate::document::FileBuf;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

pub(super) fn standby_dir(s: &Shared, n: usize) -> PathBuf {
    s.cfg.build_dir.join(format!("src-body-{n}"))
}

/// Output directory of the pass that runs from `src-body-{n}`. Two alternate: a standby
/// (lualatex up to the end of the preamble) opens its log, and with some preambles its PDF,
/// as soon as it starts, so it must never share a directory with the pass that is running.
pub(super) fn pass_dir(s: &Shared, n: usize) -> PathBuf {
    s.cfg.build_dir.join("bg").join(format!("pass-{n}"))
}

/// The directory of the last finished pass (seeded from disk on the first call: the newer
/// of the two pass directories, or the pre-pass-directory layout `build/bg` itself).
pub(super) fn last_pass_dir(s: &Shared) -> Option<PathBuf> {
    let mut slot = s.last_pass_dir.lock();
    if slot.is_none() {
        let jobname = jobname_of(&s.cfg.main_file);
        let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
        for d in [pass_dir(s, 0), pass_dir(s, 1), s.cfg.build_dir.join("bg")] {
            if let Ok(m) = std::fs::metadata(d.join(format!("{jobname}.aux"))) {
                if let Ok(t) = m.modified() {
                    if best.as_ref().map(|(bt, _)| t > *bt).unwrap_or(true) {
                        best = Some((t, d));
                    }
                }
            }
        }
        *slot = best.map(|(_, d)| d);
    }
    slot.clone()
}

/// The slot (0 or 1) a new standby takes: the one that does not hold the last pass.
/// A standby engine for `slot`: the session's project and sources, the slot's body snapshot
/// and pass directories.
pub(super) fn spawn_standby(
    s: &Shared,
    texts: &BTreeMap<String, String>,
    unit_envs: &str,
    slot: usize,
) -> Result<WarmEngine> {
    WarmEngine::spawn(crate::background::StandbySpec {
        tl: &s.tl,
        project: &s.cfg.project_root,
        files: texts,
        main: &s.cfg.main_file,
        src_dir: &standby_dir(s, slot),
        out_dir: &pass_dir(s, slot),
        instrumented: true,
        unit_envs,
    })
}

pub(super) fn standby_slot(s: &Shared) -> usize {
    match last_pass_dir(s) {
        Some(d) if d.ends_with("pass-0") => 1,
        _ => 0,
    }
}

pub(super) fn jobname_of(main: &str) -> String {
    Path::new(main)
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("main")
        .to_string()
}

/// Start a standby engine for the next background pass (while the user types, it loads the
/// current preamble). Replaces a standby whose preamble is outdated; keeps a matching one.
pub(super) fn prepare_standby(s: &Shared) {
    if !s.cfg.warm_background || s.shutdown.load(Ordering::SeqCst) {
        return;
    }
    let (texts, _, _) = snapshot(s);
    let Some(pre_hash) = standby_preamble_hash(&texts, &s.cfg.main_file) else {
        return;
    };
    let mut slot = s.standby.lock();
    if let Some(w) = slot.as_mut() {
        if w.preamble_hash == pre_hash && w.is_alive() {
            return;
        }
    }
    if let Some(w) = slot.take() {
        w.kill();
    }
    let unit_envs = s.policy.lock().unit_envs_env();
    let n = standby_slot(s);
    match spawn_standby(s, &texts, &unit_envs, n) {
        Ok(w) => *slot = Some(w),
        Err(e) => {
            s.events
                .send(Event::Diagnostics {
                    source: "background".into(),
                    items: vec![Diagnostic {
                        severity: "warning".into(),
                        file: None,
                        line: None,
                        message: format!("standby engine: {e}"),
                        context: None,
                    }],
                })
                .ok();
        }
    }
}

pub(super) fn texts_of(files: &BTreeMap<String, FileBuf>) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(n, fb)| (n.clone(), fb.text.clone()))
        .collect()
}

/// The preamble the fast server loads: the main file's preamble with `\input`ted files inlined
/// from the buffers, plus the document's body setup statements (`crate::server_preamble`).
pub(super) fn effective_preamble(texts: &BTreeMap<String, String>, main: &str) -> String {
    crate::server_preamble(texts, main)
}

/// The preamble as the standby engine hashes it (`write_body_snapshot`): inputs inlined, no
/// body setup (the standby compiles the whole body itself).
pub(super) fn standby_preamble_hash(texts: &BTreeMap<String, String>, main: &str) -> Option<u64> {
    texts
        .get(main)
        .and_then(|t| crate::split_preamble(t))
        .map(|(p, _)| crate::document::hash_str(&crate::document::expand_inputs(p, texts)))
}

pub(super) fn preamble_input_set(
    texts: &BTreeMap<String, String>,
    main: &str,
) -> std::collections::BTreeSet<String> {
    texts
        .get(main)
        .and_then(|t| crate::split_preamble(t))
        .map(|(p, _)| crate::document::transitive_inputs(p, texts))
        .unwrap_or_default()
}
