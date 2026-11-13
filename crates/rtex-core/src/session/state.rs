//! State the session's threads share: the request to the live engine, the link to it, the background commands.

use super::*;

/// Outcome of a probe compile (probe mode).
#[derive(Debug, Clone)]
pub(super) enum ProbeVerdict {
    /// The snapshot text reproduced the layout's rows: edits of the unit go live.
    Verified,
    /// Rows differed, the compile failed, or there was nothing to compare (why).
    Mismatch(String),
    /// The compile changed the meaning of a control sequence it mentions (names).
    Leak(String),
}

/// The kernel's `\everypar` after a heading (`\@afterheading`), replayed for a paragraph typed
/// fresh right after a heading span.
pub(super) const AFTER_HEADING_EVERYPAR: &str = "\\if@nobreak \\@nobreakfalse \\clubpenalty \\@M \\if@afterindent \\else {\\setbox \\z@ \\lastbox }\\fi \\else \\clubpenalty \\@clubpenalty \\everypar {}\\fi ";

pub(super) struct FastRequest {
    /// Internal warm-up compile: result is discarded, no event is emitted.
    pub(super) warmup: bool,
    pub(super) par_id: ParaId,
    pub(super) edit_id: u64,
    pub(super) span_hash: u64,
    pub(super) source: String,
    /// Picture cache entries for the source's picture environments, in order (JSON array;
    /// empty when none is cached), and how many pictures the source has.
    pub(super) pics: String,
    pub(super) pics_n: usize,
    pub(super) seq: i64,
    /// Context object for the server; None when the engine link already holds this context
    /// (the engine thread builds it from the layout store when it must send it).
    pub(super) ctx: Option<serde_json::Value>,
    pub(super) versions: Versions,
    pub(super) context_stale: bool,
    pub(super) expected_rows: i64,
    /// Probe mode: the unit must be proven first (its snapshot text, fetched from the layout
    /// current when the engine thread gets to it, compiled and compared with that layout).
    pub(super) probe: bool,
    /// The unit was let through unproven because every picture in it comes from the cache
    /// (its only vocabulary beyond the allow-list): a result that drew a picture after all is
    /// unverified and the unit waits for the pass.
    pub(super) pics_required: bool,
}

/// A compile the server is working on (sent either by the engine thread or directly by the
/// host thread through `EngineLink::writer`).
pub(super) struct InFlight {
    pub(super) req: FastRequest,
    pub(super) t0: Instant,
}

/// Host-visible side of the fast server: lets `apply_edit` submit a compile frame straight to
/// the server's stdin when the server is idle, saving the wake-up of the engine thread. The
/// engine thread owns the server and reads every result, in order.
#[derive(Default)]
pub(super) struct EngineLink {
    pub(super) writer: Option<std::fs::File>,
    pub(super) generation: u64,
    pub(super) inflight: Option<InFlight>,
    pub(super) contexts_sent: HashSet<i64>,
    pub(super) context_rev_sent: u64,
    pub(super) labels_sent: u64,
    pub(super) next_req: i64,
    /// A probe compile is running on the engine thread without the lock held: the direct
    /// dispatch path must not write to the server meanwhile.
    pub(super) probing: bool,
    /// Probe mode statistics: probe compiles run and their total time.
    pub(super) probes: u64,
    pub(super) probe_us: u64,
}

pub(super) struct Shared {
    pub(super) link: Mutex<EngineLink>,
    pub(super) files: Mutex<BTreeMap<String, FileBuf>>,
    pub(super) ids: Mutex<IdAllocator>,
    pub(super) source_revision: AtomicU64,
    pub(super) preamble_revision: AtomicU64,
    /// Per file: last revision at which a background-only span or the preamble changed.
    pub(super) bg_change: Mutex<HashMap<String, Vec<(Revision, i64)>>>, // (revision, first line of the changed span)
    pub(super) layout: Mutex<LayoutStore>,
    pub(super) engine_generation: AtomicU64,
    pub(super) events: Sender<Event>,
    /// Queued requests, latest per unit, served smallest id first (a split's halves in
    /// document order: the second chains its placement onto the first).
    pub(super) pending: Mutex<BTreeMap<ParaId, FastRequest>>,
    pub(super) pending_signal: (Sender<()>, Receiver<()>),
    pub(super) bg_signal: (Sender<BgCmd>, Receiver<BgCmd>),
    pub(super) shutdown: AtomicBool,
    /// A background pass is compiling right now: fast-path timings are not budget evidence.
    pub(super) bg_running: AtomicBool,
    /// Background passes are deferred while paused (benchmarks, host-controlled quiet periods).
    pub(super) bg_paused: AtomicBool,
    pub(super) bg_pending_while_paused: AtomicBool,
    pub(super) cfg: SessionConfig,
    pub(super) tl: TexLive,
    pub(super) policy: Mutex<Policy>,
    /// Files the preamble `\input`s: an edit to one of them is a preamble change.
    pub(super) preamble_inputs: Mutex<std::collections::BTreeSet<String>>,
    /// Files read with `\input` anywhere (see `document::fast_source`).
    pub(super) inputted: Mutex<std::collections::BTreeSet<String>>,
    pub(super) edit_counter: AtomicU64,
    pub(super) convergence: Mutex<Option<Convergence>>,
    /// What the live path records per span between layouts (borrowed contexts, live rows
    /// and placements, probe verdicts, budget marks, leaks...), with their expiry rules.
    pub(super) live: Mutex<LiveUnits>,
    /// Standby background engine (preamble loaded, waiting for the body).
    pub(super) standby: Mutex<Option<WarmEngine>>,
    /// Directory of the last finished background pass (its aux family seeds the next pass;
    /// its PDF and capture files stay until a later pass reuses the directory).
    pub(super) last_pass_dir: Mutex<Option<PathBuf>>,
    /// The picture cache index as of a layout version (the live engine reuses cached pictures).
    pub(super) pic_index: Mutex<Option<(u64, crate::piccache::PicCache)>>,
    /// The source scan for pictures at a revision (the whole project; one scan per edit, not
    /// one per routed span).
    pub(super) pic_scan: Mutex<Option<(Revision, Arc<Vec<crate::piccache::PictureRef>>)>>,
    /// Live-engine failures reported as `fast:<unit>` diagnostics, with the source revision
    /// they were typeset at: the first layout of that revision or a later one withdraws them.
    pub(super) fast_failures: Mutex<BTreeMap<ParaId, Revision>>,
}

pub(super) enum BgCmd {
    Pass,
    Export(u64, PathBuf),
    Quit,
}
