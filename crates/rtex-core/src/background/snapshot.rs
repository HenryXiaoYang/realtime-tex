//! The source snapshot a pass reads: the edited buffers as real files, everything else linked to the project.

use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

/// Write `files` (relative path → text) under `dir`, and link every other file of `project`
/// (images, .bib, .cls …) into it so relative inputs resolve.
///
/// Nothing is copied where the platform can link: a folder that holds none of `files` is one
/// link to the project's folder, a file next to one of `files` a link to the project's file.
/// A copy of the main file's folder per snapshot (a session keeps three) filled users' disks
/// (plugin issues #13, #15).
pub fn write_snapshot(
    project: &Path,
    files: &BTreeMap<String, String>,
    main: &str,
    dir: &Path,
) -> Result<()> {
    // a snapshot written by rtex 0.0.6 or earlier holds copies: start over once
    let marker = dir.join(".rtex-links");
    if dir.exists() && !marker.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(&marker, "")?;
    // the PDFs built from the document's own sources (main.pdf, a subfile compiled alone):
    // outputs, not inputs. A PDF next to a .tex the document does not read (a standalone
    // figure's source, or another document such as an appendix the host has open) is an input
    // and is linked.
    let mut own = files
        .get(main)
        .map(|t| crate::document::transitive_inputs(t, files))
        .unwrap_or_default();
    own.insert(main.to_string());
    let built: HashSet<PathBuf> = own
        .iter()
        .map(|rel| {
            project
                .join(rel.trim_start_matches("./"))
                .with_extension("pdf")
        })
        .collect();
    // folders that hold one of `files` are real folders of the snapshot, so writing a source
    // never goes through a link into the project
    let mut real = HashSet::new();
    for rel in files.keys() {
        let mut p = Path::new(rel.trim_start_matches("./"));
        while let Some(parent) = p.parent() {
            real.insert(parent.to_path_buf());
            p = parent;
        }
    }
    let dst_canon = crate::paths::canonical(dir).unwrap_or_else(|_| dir.to_path_buf());
    link_tree(project, dir, Path::new(""), &real, &built, &dst_canon)?;
    for (rel, text) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // a link to the project's saved file until now
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, text)?;
    }
    Ok(())
}

/// Whether `p` is a symbolic link (or, on Windows, a junction).
pub(super) fn is_link(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Fill the real snapshot folder `dst` (`rel` in the project) with links to the entries of the
/// project folder `src`.
pub(super) fn link_tree(
    src: &Path,
    dst: &Path,
    rel: &Path,
    real: &HashSet<PathBuf>,
    built: &HashSet<PathBuf>,
    dst_canon: &Path,
) -> Result<()> {
    if is_link(dst) {
        // linked by an earlier snapshot that had none of `files` in it
        remove_link(dst)?;
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') || name_s == "build" || name_s == "target" {
            continue;
        }
        let path = entry.path();
        // never descend into the build tree the snapshot itself lives in (a host may keep it
        // inside the project under any name), and skip FIFOs, sockets and devices
        let path_canon = crate::paths::canonical(&path).unwrap_or_else(|_| path.clone());
        if dst_canon.starts_with(&path_canon) {
            continue;
        }
        let target = dst.join(&name);
        let rel = rel.join(&name);
        if path.is_dir() {
            // a real folder stays one (it held a source, or the folder could not be linked)
            let real_dir = real.contains(&rel) || (!is_link(&target) && target.is_dir());
            if real_dir || (!is_link(&target) && link_dir(&path_canon, &target).is_err()) {
                link_tree(&path, &target, &rel, real, built, dst_canon)?;
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "pdf" && built.contains(&path) {
            continue;
        }
        // the document's own .aux files are written by the pass; another document's
        // (\externaldocument{appendix} reads appendix.aux) is an input
        if ext == "aux" && built.contains(&path.with_extension("pdf")) {
            continue;
        }
        if matches!(
            ext,
            "log"
                | "synctex.gz"
                | "fls"
                | "fdb_latexmk"
                | "out"
                | "toc"
                | "lof"
                | "lot"
                | "bbl"
                | "bcf"
                | "blg"
                | "run.xml"
        ) {
            continue;
        }
        if is_link(&target) {
            continue;
        }
        // a hard link or copy made where symbolic links are not allowed
        if let Ok(t) = std::fs::metadata(&target) {
            if t.modified()? >= std::fs::metadata(&path)?.modified()? {
                continue;
            }
            std::fs::remove_file(&target)?;
        }
        link_file(&path_canon, &target)?;
    }
    Ok(())
}

pub(super) fn remove_link(p: &Path) -> std::io::Result<()> {
    // a link to a folder is a directory entry on Windows
    std::fs::remove_file(p).or_else(|_| std::fs::remove_dir(p))
}

#[cfg(unix)]
pub(super) fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

/// A symbolic link needs Developer Mode or an elevated process; a junction needs neither and
/// reaches other drives.
#[cfg(windows)]
pub(super) fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::os::windows::fs::symlink_dir(src, dst).is_ok() {
        return Ok(());
    }
    let ok = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(dst)
        .arg(src)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?
        .success();
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::other("mklink /J failed"))
    }
}

/// A symbolic link, else (no Developer Mode on Windows, a file system without links) a hard
/// link on the same drive, else a copy of a file up to 8 MiB. A bigger file stays in the
/// project, where `\input@path` and the graphics path still find it (texlive.rs
/// `INPUT_PATH_HOOK`).
pub(super) fn link_file(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(src, dst).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(src, dst).is_ok();
    if linked || std::fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    if std::fs::metadata(src)?.len() <= 8 << 20 {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Create under `out` every directory of `src` (skipping hidden, build and target), so
/// `\include{chapters/x}` can open `chapters/x.aux` in the output directory.
pub fn mirror_dirs(src: &Path, out: &Path, depth: usize) -> Result<()> {
    if depth > 8 {
        return Ok(());
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') || name_s == "build" || name_s == "target" {
            continue;
        }
        if entry.path().is_dir() {
            let sub = out.join(&name);
            std::fs::create_dir_all(&sub)?;
            mirror_dirs(&entry.path(), &sub, depth + 1)?;
        }
    }
    Ok(())
}

pub fn snapshot_dir(build: &Path) -> PathBuf {
    build.join("src")
}

/// Snapshot layout for a standby pass: the project copied as for `write_snapshot`, the preamble
/// (text before `\begin{document}`) in `rtex-preamble.tex`, and `main` replaced by the body
/// preceded by as many empty lines as the preamble had, so every line number and `status.filename`
/// the capture records are those of the original file. Returns the preamble's hash.
pub fn write_body_snapshot(
    project: &Path,
    files: &BTreeMap<String, String>,
    main: &str,
    dir: &Path,
) -> Result<u64> {
    let main_text = files
        .get(main)
        .ok_or_else(|| anyhow::anyhow!("main file {main} not in snapshot"))?;
    let (pre, body) = crate::split_preamble(main_text)
        .ok_or_else(|| anyhow::anyhow!("no \\begin{{document}} in {main}"))?;
    let mut padded = String::with_capacity(main_text.len());
    for _ in 0..pre.matches('\n').count() {
        padded.push('\n');
    }
    padded.push_str(body);
    let mut files2 = files.clone();
    files2.insert(main.to_string(), padded);
    files2.insert("rtex-preamble.tex".to_string(), pre.to_string());
    write_snapshot(project, &files2, main, dir)?;
    Ok(crate::document::hash_str(&crate::document::expand_inputs(
        pre, files,
    )))
}
