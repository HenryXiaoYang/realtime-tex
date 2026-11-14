//! What the live engine answers: a compile result with its display list, an error, and the round trip's timing.

use rtex_dl::DisplayList;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompileResult {
    pub req: i64,
    #[serde(default)]
    pub ctx: Option<i64>,
    pub status: String,
    #[serde(default)]
    pub errors: Vec<EngineError>,
    #[serde(default)]
    pub dl: Option<DisplayList>,
    /// The display list as received (binary encoding revision 1), kept so hosts need no re-encoding.
    #[serde(skip)]
    pub dl_binary: Option<Vec<u8>>,
    #[serde(default)]
    pub dl_bytes: i64,
    #[serde(default)]
    pub t_tex_us: i64,
    #[serde(default)]
    pub t_traverse_us: i64,
    #[serde(default)]
    pub t_pack_us: i64,
    #[serde(default)]
    pub stages_us: serde_json::Value,
    /// Image resources used by the display list (index → file), from the server's graphicx hook.
    #[serde(default)]
    pub images: std::collections::BTreeMap<String, rtex_dl::ImageInfo>,
    /// Counters the compile advanced: name → value at the end (see rtex-serve.lua finish()).
    #[serde(default)]
    pub counters: std::collections::BTreeMap<String, i64>,
    /// Control sequences the source mentions whose meaning the compile changed (a definition
    /// or \let that leaked out of the unit's box).
    #[serde(default)]
    pub leaks: Vec<String>,
    /// Picture environments the compile began, and how many of them came from the picture
    /// cache (present when the request carried cache entries).
    #[serde(default)]
    pub pics_seen: Option<i64>,
    #[serde(default)]
    pub pics_used: Option<i64>,
    /// A Lua error in the server while handling the request (with its traceback): the result
    /// is an error the unit's text did not cause, sent instead of nothing so the host does not
    /// wait out the watchdog.
    #[serde(default)]
    pub internal: Option<String>,
    /// Host-side stage times (µs): send, wait, read, parse.
    #[serde(skip)]
    pub host_us: [u64; 4],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op")]
pub enum Response {
    #[serde(rename = "ready")]
    Ready {
        banner: String,
        #[serde(default)]
        font_nextid: i64,
        #[serde(default)]
        fingerprint: String,
    },
    #[serde(rename = "ok")]
    Ok { id: i64 },
    #[serde(rename = "result")]
    Result(Box<CompileResult>),
    #[serde(rename = "fatal")]
    Fatal {
        #[serde(default)]
        req: Option<i64>,
        reason: String,
        #[serde(default)]
        errors: Vec<EngineError>,
        #[serde(default)]
        before: String,
        #[serde(default)]
        after: String,
    },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "stats")]
    Stats {
        requests: i64,
        font_nextid: i64,
        node_mem: String,
        grouplevel: i64,
        nest: i64,
        luastate: f64,
    },
    #[serde(rename = "profile")]
    Profile {
        #[serde(default)]
        us: serde_json::Value,
        #[serde(default)]
        input_ptr: i64,
    },
    #[serde(rename = "bye")]
    Bye,
    #[serde(rename = "error")]
    Error { message: String },
}

/// Timing of one round trip as seen from the host.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct RoundTrip {
    pub total: Duration,
    pub t_tex: Duration,
    pub t_traverse: Duration,
    pub t_pack: Duration,
}
