//! What a host configures when it opens a session.

use crate::background::BibTool;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub project_root: PathBuf,
    pub main_file: String,
    pub build_dir: PathBuf,
    pub debounce: Duration,
    pub max_passes: u32,
    pub trusted_macros: Vec<String>,
    pub fast_on_stale_context: bool,
    pub bib_tool: BibTool,
    pub compile_timeout: Duration,
    /// A background or export lualatex run that takes longer than this is killed and the
    /// layout fails (default 5 min; a document can loop forever).
    pub pass_timeout: Duration,
    /// Units whose fast compiles take longer than this three times in a row are routed to the
    /// background path until the next layout (default 50 ms: a unit that slow, a large plot
    /// being drawn, would hold up the units typed after it; a quicker one, even an `align`
    /// of 15 ms, is still far quicker live than through a pass of seconds).
    pub fast_budget: Duration,
    /// Extra block environments (theorem-like) treated as units, besides those found by scanning
    /// the preamble for `\newtheorem`.
    pub unit_envs: Vec<String>,
    /// Keep a standby lualatex with the preamble loaded for background passes (default true):
    /// a pass then only typesets the body, 3–4× faster for short documents.
    pub warm_background: bool,
    /// How a unit qualifies for the fast path (default `Probe`).
    pub eligibility: EligibilityMode,
    /// Background passes reuse unchanged pictures (tikzpicture, circuitikz) from an earlier
    /// pass's PDF instead of drawing them again (default true; `docs/how-it-works.md`).
    pub picture_cache: bool,
    /// Observe opaque host ids carried by this LuaTeX attribute (off by default).
    pub math_owner_attribute: Option<u16>,
    /// Debugging: a directory the session writes diagnostics into (default: `$RTEX_DEBUG_DIR`
    /// when set, else none). Every engine failure (watchdog, state mismatch, crash) leaves a
    /// bundle there with the request's source, context and picture entries, the server's
    /// driver, preamble, TeX log and stage trace; `requests.log` gets one line per live
    /// compile. C ABI `"debug_dir"`, CLI `rtex serve --debug-dir`.
    pub debug_dir: Option<PathBuf>,
}

/// How the session decides that a unit may be typeset live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EligibilityMode {
    /// Only units whose every command is on the allow-list (`eligibility/`).
    AllowList,
    /// Any structurally sound unit: one whose vocabulary is not allow-listed is probed first —
    /// its text as the last pass typeset it is compiled once and compared row by row with the
    /// pass; a match verifies it for this layout, anything else keeps it on the background
    /// path (`BackgroundScheduled` with an `unverified:` reason). Allow-listed units skip the
    /// probe. A unit the layout does not know yet (typed fresh, split off) has nothing to
    /// compare with: it is compiled on a borrowed context only when its vocabulary is
    /// allow-listed, or when what is not are picture environments the cache holds (their
    /// bodies are not run); otherwise it waits for the pass.
    #[default]
    Probe,
}

impl EligibilityMode {
    pub fn parse(s: &str) -> Option<EligibilityMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "probe" => Some(EligibilityMode::Probe),
            "allowlist" | "allow-list" | "allow_list" => Some(EligibilityMode::AllowList),
            _ => None,
        }
    }
}

impl SessionConfig {
    pub fn new(project_root: impl Into<PathBuf>, main_file: impl Into<String>) -> Self {
        let project_root = project_root.into();
        SessionConfig {
            build_dir: project_root.join("build").join("rtex"),
            project_root,
            main_file: main_file.into(),
            debounce: Duration::from_millis(300),
            max_passes: 5,
            trusted_macros: Vec::new(),
            fast_on_stale_context: true,
            bib_tool: BibTool::Auto,
            compile_timeout: Duration::from_secs(5),
            pass_timeout: Duration::from_secs(300),
            fast_budget: Duration::from_millis(50),
            unit_envs: Vec::new(),
            warm_background: true,
            eligibility: EligibilityMode::Probe,
            picture_cache: true,
            math_owner_attribute: None,
            debug_dir: std::env::var_os("RTEX_DEBUG_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}
