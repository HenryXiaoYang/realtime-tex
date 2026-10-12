//! Diagnostics: debug bundles for engine failures, the live request log, TeX log parsing.

use super::*;

/// With `SessionConfig::debug_dir`: write everything needed to reproduce an engine failure
/// into `<debug_dir>/engine-<time>-g<generation>-par<id>/` and return that directory.
pub(super) fn debug_bundle(
    s: &Shared,
    req: &FastRequest,
    reason: &str,
    gen: u64,
    srv: Option<&FastServer>,
) -> Option<PathBuf> {
    let base = s.cfg.debug_dir.as_ref()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = base.join(format!("engine-{ts}-g{gen}-par{}", req.par_id.0));
    let write = |name: &str, bytes: &[u8]| {
        let _ = std::fs::write(dir.join(name), bytes);
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    write("source.tex", req.source.as_bytes());
    if !req.pics.is_empty() {
        write("pics.json", req.pics.as_bytes());
    }
    let ctx = req.ctx.clone().or_else(|| {
        let layout = s.layout.lock();
        layout
            .units
            .iter()
            .find(|u| u.uid == req.seq)
            .map(|u| u.context_json())
    });
    if let Some(c) = &ctx {
        write(
            "context.json",
            serde_json::to_string_pretty(c)
                .unwrap_or_default()
                .as_bytes(),
        );
    }
    let report = serde_json::json!({
        "reason": reason,
        "par_id": req.par_id.0,
        "edit_id": req.edit_id,
        "context_id": req.seq,
        "probe": req.probe,
        "warmup": req.warmup,
        "pictures_in_source": req.pics_n,
        "versions": req.versions,
        "engine_generation": gen,
        "compile_timeout_ms": s.cfg.compile_timeout.as_millis() as u64,
        "server": srv.map(|x| serde_json::json!({
            "banner": x.banner, "pid": x.pid(), "generation": x.generation,
            "startup_ms": x.startup.as_millis() as u64, "work_dir": x.work_dir,
        })),
        "rtex_version": env!("CARGO_PKG_VERSION"),
    });
    write(
        "report.json",
        serde_json::to_string_pretty(&report)
            .unwrap_or_default()
            .as_bytes(),
    );
    if let Some(x) = srv {
        for f in x.debug_files() {
            if f.is_file() {
                if let Some(name) = f.file_name() {
                    let _ = std::fs::copy(&f, dir.join(name));
                }
            }
        }
    }
    Some(dir)
}

/// With `SessionConfig::debug_dir`: a bundle for a server that failed to start (its driver,
/// preamble copy, TeX log and trace from `build/serve`), named in the returned message.
pub(super) fn debug_bundle_startup(s: &Shared, gen: u64, reason: &str) -> String {
    let Some(base) = s.cfg.debug_dir.as_ref() else {
        return reason.to_string();
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = base.join(format!("engine-{ts}-g{gen}-start"));
    if std::fs::create_dir_all(&dir).is_err() {
        return reason.to_string();
    }
    let serve = s.cfg.build_dir.join("serve");
    for name in [
        format!("rtex-serve-g{gen}.tex"),
        "rtex-preamble.tex".to_string(),
        format!("rtex-serve-g{gen}.log"),
        format!("rtex-serve-g{gen}.trace"),
    ] {
        let f = serve.join(&name);
        if f.is_file() {
            let _ = std::fs::copy(&f, dir.join(&name));
        }
    }
    let report = serde_json::json!({
        "reason": reason, "engine_generation": gen, "stage": "startup",
        "rtex_version": env!("CARGO_PKG_VERSION"),
    });
    let _ = std::fs::write(
        dir.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    );
    format!("{reason} (debug bundle: {})", dir.display())
}

/// With `SessionConfig::debug_dir`: one line per live compile in `<debug_dir>/requests.log`.
pub(super) fn debug_request_line(s: &Shared, line: &str) {
    let Some(base) = s.cfg.debug_dir.as_ref() else {
        return;
    };
    use std::io::Write;
    let _ = std::fs::create_dir_all(base);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(base.join("requests.log"))
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = writeln!(f, "{ts:.3} {line}");
    }
}

/// Parse a LuaLaTeX log (with -file-line-error) into diagnostics.
pub fn parse_log(log: &Path) -> Vec<Diagnostic> {
    let Ok(text) = std::fs::read_to_string(log) else {
        return vec![];
    };
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    // a Windows path starts with its drive (`D:/a/rtex-preamble.tex:5: ...`)
    let re_fle =
        regex::Regex::new(r"^(?P<file>(?:[A-Za-z]:)?[^:\s][^:]*):(?P<line>\d+): (?P<msg>.*)$")
            .unwrap();
    let re_warn =
        regex::Regex::new(r"^(?:LaTeX|Package \w+|Class \w+) Warning: (?P<msg>.*)$").unwrap();
    let re_box =
        regex::Regex::new(r"^(Overfull|Underfull) \\[hv]box .* at lines (\d+)--(\d+)").unwrap();
    // LaTeX, class and package messages continue on lines that start with `(LaTeX)` or
    // `(<package>)`; their text often says what is wrong (versions, file names)
    let re_cont = regex::Regex::new(r"^\([A-Za-z][\w@-]*\)(?:\s+(?P<rest>.*))?$").unwrap();
    let full_message = |first: &str, i: usize| -> (String, usize) {
        let mut msg = first.to_string();
        let mut j = i + 1;
        while let Some(c) = lines.get(j).and_then(|l| re_cont.captures(l)) {
            let rest = c.name("rest").map_or("", |m| m.as_str()).trim_end();
            msg.push('\n');
            msg.push_str(rest);
            j += 1;
        }
        (msg.trim_end().to_string(), j)
    };
    for (i, l) in lines.iter().enumerate() {
        if let Some(c) = re_fle.captures(l) {
            if c["msg"].starts_with("Undefined") || !c["msg"].is_empty() {
                let (message, next) = full_message(&c["msg"], i);
                out.push(Diagnostic {
                    severity: "error".into(),
                    file: Some(c["file"].to_string()),
                    line: c["line"].parse().ok(),
                    message,
                    context: lines.get(next).map(|s| s.to_string()),
                });
            }
        } else if let Some(c) = re_warn.captures(l) {
            out.push(Diagnostic {
                severity: "warning".into(),
                file: None,
                line: None,
                message: full_message(&c["msg"], i).0,
                context: None,
            });
        } else if let Some(c) = re_box.captures(l) {
            out.push(Diagnostic {
                severity: "info".into(),
                file: None,
                line: c[2].parse().ok(),
                message: l.to_string(),
                context: None,
            });
        } else if let Some(first) = l.strip_prefix("! ") {
            let (message, next) = full_message(first, i);
            out.push(Diagnostic {
                severity: "error".into(),
                file: None,
                line: None,
                message,
                context: lines.get(next).map(|s| s.to_string()),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_line_errors_with_and_without_a_drive() {
        let log = std::env::temp_dir().join(format!("rtex-diag-{}.log", std::process::id()));
        std::fs::write(
            &log,
            "./main.tex:12: Undefined control sequence.\nl.12 \\foo\n\
             D:/a/build/rtex-preamble.tex:5: Missing $ inserted.\nl.5 x\n",
        )
        .unwrap();
        let d = parse_log(&log);
        let _ = std::fs::remove_file(&log);
        let got: Vec<(Option<&str>, Option<i64>)> =
            d.iter().map(|d| (d.file.as_deref(), d.line)).collect();
        assert_eq!(
            got,
            vec![
                (Some("./main.tex"), Some(12)),
                (Some("D:/a/build/rtex-preamble.tex"), Some(5))
            ]
        );
    }

    #[test]
    fn latex_errors_keep_their_continuation_lines() {
        let log = std::env::temp_dir().join(format!("rtex-diag-cont-{}.log", std::process::id()));
        std::fs::write(
            &log,
            "(/usr/share/texmf-dist/tex/latex/l3kernel/expl3.sty\n\
             Package: expl3 2026-01-19 L3 programming layer (loader) \n\n\
             /usr/share/texmf-dist/tex/latex/l3kernel/expl3.sty:78: LaTeX Error: Mismatched LaTeX support files detected.\n\
             (LaTeX)        Loading 'expl3.sty' aborted!\n\
             (LaTeX)        \n\
             (LaTeX)        The L3 programming layer in the LaTeX format\n\
             (LaTeX)        is dated 2025-06-30, but in your TeX tree the files require\n\
             (LaTeX)        at least 2026-01-19.\n\n\
             For immediate help type H <return>.\n\
             Package hyperref Warning: Token not allowed in a PDF string (Unicode):\n\
             (hyperref)                removing `math shift' on input line 3.\n\
             (./main.aux)\n",
        )
        .unwrap();
        let d = parse_log(&log);
        let _ = std::fs::remove_file(&log);
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[0].line, Some(78));
        assert_eq!(
            d[0].message,
            "LaTeX Error: Mismatched LaTeX support files detected.\nLoading 'expl3.sty' aborted!\n\n\
             The L3 programming layer in the LaTeX format\nis dated 2025-06-30, but in your TeX tree \
             the files require\nat least 2026-01-19."
        );
        assert_eq!(d[0].context.as_deref(), Some(""));
        assert_eq!(
            d[1].message,
            "Token not allowed in a PDF string (Unicode):\nremoving `math shift' on input line 3."
        );
    }
}
