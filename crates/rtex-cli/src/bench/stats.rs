//! Summary statistics of a set of timings.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Stats {
    pub n: usize,
    pub median: f64,
    pub p5: f64,
    pub p95: f64,
    pub min: f64,
    pub max: f64,
}

pub fn stats(v: &[f64]) -> Stats {
    if v.is_empty() {
        return Stats::default();
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    let at = |f: f64| s[(((n as f64) * f).floor() as usize + 1).min(n) - 1];
    Stats {
        n,
        median: at(0.5),
        p5: at(0.05),
        p95: at(0.95),
        min: s[0],
        max: s[n - 1],
    }
}
