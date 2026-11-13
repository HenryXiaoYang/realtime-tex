//! Index programs (makeindex, xindy) run between passes.

use crate::texlive::TexLive;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Index programs a document may ask for (imakeidx `program=`); anything else runs makeindex.
pub(super) const INDEX_PROGRAMS: &[&str] = &[
    "makeindex",
    "texindy",
    "xindy",
    "truexindy",
    "upmendex",
    "mendex",
];

/// The command for `idx` (a file name in the pass directory): the one the capture recorded
/// from imakeidx (`<job>.rtex-idxcmd`: program, the document's options, the file), else
/// `makeindex -q <idx>`.
pub(super) fn index_command(recorded: &str, idx: &str) -> (String, Vec<String>) {
    for line in recorded.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let (Some(prog), Some(file)) = (words.first(), words.last()) else {
            continue;
        };
        if *file == idx && words.len() >= 2 && INDEX_PROGRAMS.contains(prog) {
            let mut args: Vec<String> = words[1..].iter().map(|w| w.to_string()).collect();
            if *prog == "makeindex" && !args.iter().any(|a| a == "-q") {
                args.insert(0, "-q".into());
            }
            return (prog.to_string(), args);
        }
    }
    ("makeindex".into(), vec!["-q".into(), idx.into()])
}

/// Run the index program on every `.idx` a pass wrote (`\makeindex`, imakeidx's named indexes)
/// whose entries differ from the ones last indexed in `indexed` (name → hash of the `.idx`), or
/// whose `.ind` is missing, with the program and options imakeidx was given (`index_command`).
/// The `.ind` files are part of the aux signature, so a changed index brings another pass. The
/// index is built here rather than by the document (imakeidx's own call needs shell escape and
/// cannot find an `.idx` in the output directory).
pub(super) fn run_makeindex(
    tl: &TexLive,
    snapshot_dir: &Path,
    out_dir: &Path,
    jobname: &str,
    indexed: &mut BTreeMap<String, u64>,
) {
    use std::hash::{Hash, Hasher};
    let Ok(dir) = std::fs::read_dir(out_dir) else {
        return;
    };
    let recorded =
        std::fs::read_to_string(out_dir.join(format!("{jobname}.rtex-idxcmd"))).unwrap_or_default();
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "idx") {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        let hash = h.finish();
        if indexed.get(&name) == Some(&hash) && path.with_extension("ind").exists() {
            continue;
        }
        let (program, args) = index_command(&recorded, &name);
        let mut cmd = Command::new(&program);
        cmd.current_dir(out_dir).args(&args);
        if let Some(d) = &tl.bin_dir {
            crate::paths::prepend_bin_dir(&mut cmd, d);
        }
        // index styles (.ist) the project ships
        cmd.env(
            "INDEXSTYLE",
            format!(
                "{}{}",
                crate::paths::tex(snapshot_dir),
                crate::paths::SEARCH_SEP
            ),
        );
        match cmd.output() {
            Ok(out) if !out.status.success() => {
                log::warn!(
                    "{program} {name} failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                )
            }
            Err(e) => log::warn!("running {program}: {e}"),
            _ => {}
        }
        indexed.insert(name, hash);
    }
}
