//! The limits a benchmark run is checked against, each scaled by the hardware factor.

use super::report::*;
use super::roundtrip::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Gate {
    pub name: String,
    pub value: f64,
    pub limit: f64,
    pub pass: bool,
    pub note: String,
}

/// Checks a report against the limits and records each as a `Gate`.
///
/// Long because it is a list: each block states one limit (its name, value and bound), in the
/// order the report prints them.
pub fn gates(report: &mut BenchReport) {
    let h = report.hardware_factor.max(1.0);
    let mut gates = Vec::new();
    let find = |proj_contains: &str, cat: &str| {
        report.results.iter().find(|r| {
            r.project.contains(proj_contains) && r.category == cat && !r.project.contains("units")
        })
    };
    // size independence across 10/100/300
    for cat in ["short", "medium", "long", "inline-math"] {
        if let (Some(a), Some(b)) = (find("book-10-", cat), find("book-300-", cat)) {
            let ratio = b.individual_total.median / a.individual_total.median;
            gates.push(Gate {
                name: format!("size independence {cat} (300p/10p individual median)"),
                value: ratio,
                limit: 1.25,
                pass: (0.8..=1.25).contains(&ratio),
                note: format!(
                    "{:.3} ms vs {:.3} ms",
                    b.individual_total.median, a.individual_total.median
                ),
            });
        }
        if let (Some(a), Some(b)) = (find("book-10-", cat), find("book-100-", cat)) {
            let ratio = b.individual_total.median / a.individual_total.median;
            gates.push(Gate {
                name: format!("size independence {cat} (100p/10p individual median)"),
                value: ratio,
                limit: 1.25,
                pass: (0.8..=1.25).contains(&ratio),
                note: String::new(),
            });
        }
    }
    // absolute gates on the largest document (hardest case), hardware-qualified
    let big: Vec<&CategoryResult> = report
        .results
        .iter()
        .filter(|r| r.project.contains("book-300-") && !r.project.contains("units"))
        .collect();
    let pick = |cat: &str| -> Option<&CategoryResult> {
        big.iter()
            .copied()
            .find(|r| r.category == cat)
            .or_else(|| report.results.iter().find(|r| r.category == cat))
    };
    if let Some(r) = pick("short") {
        gates.push(Gate {
            name: "short amortized median ≤ 1.0 ms × h".into(),
            value: r.amortized_total.median,
            limit: 1.0 * h,
            pass: r.amortized_total.median <= 1.0 * h,
            note: format!("h = {:.2}", h),
        });
        gates.push(Gate {
            name: "short individual median ≤ 1.5 ms × h".into(),
            value: r.individual_total.median,
            limit: 1.5 * h,
            pass: r.individual_total.median <= 1.5 * h,
            note: format!("P95 {:.3} ms", r.individual_total.p95),
        });
        gates.push(Gate {
            name: "short P95 ≤ 3 × median".into(),
            value: r.individual_total.p95 / r.individual_total.median,
            limit: 3.0,
            pass: r.individual_total.p95 <= 3.0 * r.individual_total.median,
            note: String::new(),
        });
        // IPC + host plumbing (everything outside tex + traverse + pack): two process hops and two
        // thread hops; absolute, hardware-qualified. The paper's "overhead share" is reported, not
        // gated: our engine stage is several times faster than the paper's, so the same fixed
        // wake-up cost is a larger fraction.
        let host = (r.individual_total.median
            - r.individual_tex.median
            - r.individual_traverse.median
            - r.individual_pack.median)
            .max(0.0);
        gates.push(Gate {
            name: "short IPC + host overhead ≤ 0.5 ms × h".into(),
            value: host,
            limit: 0.5 * h,
            pass: host <= 0.5 * h,
            note: format!(
                "overhead share {:.0} % (paper: 18 %)",
                r.overhead_share_median * 100.0
            ),
        });
    }
    if let Some(r) = pick("medium") {
        gates.push(Gate {
            name: "medium individual median ≤ 6.11 ms × h (paper reference)".into(),
            value: r.individual_total.median,
            limit: 6.11 * h,
            pass: r.individual_total.median <= 6.11 * h,
            note: String::new(),
        });
        let tp = r.individual_traverse.median + r.individual_pack.median;
        gates.push(Gate {
            name: "medium traversal + serialization ≤ 0.5 ms".into(),
            value: tp,
            limit: 0.5,
            pass: tp <= 0.5,
            note: String::new(),
        });
        let host = (r.individual_total.median
            - r.individual_tex.median
            - r.individual_traverse.median
            - r.individual_pack.median)
            .max(0.0);
        gates.push(Gate {
            name: "medium IPC + host overhead ≤ 0.5 ms × h".into(),
            value: host,
            limit: 0.5 * h,
            pass: host <= 0.5 * h,
            note: format!(
                "overhead share {:.0} % (paper: 18 %)",
                r.overhead_share_median * 100.0
            ),
        });
    }
    // unit kinds (0.0.2): every real-time unit kind stays under 5 ms per keystroke on the largest
    // units fixture, and is document-size independent (100p/10p)
    let units_big: Vec<&CategoryResult> = report
        .results
        .iter()
        .filter(|r| r.project.contains("units"))
        .collect();
    for cat in [
        "display-math",
        "footnote",
        "list",
        "figure",
        "table",
        "heading",
    ] {
        let largest = units_big
            .iter()
            .copied()
            .filter(|r| r.category == cat)
            .max_by_key(|r| {
                r.project
                    .split('-')
                    .nth(1)
                    .and_then(|n| n.parse::<u32>().ok())
                    .unwrap_or(0)
            });
        if let Some(r) = largest {
            gates.push(Gate {
                name: format!("{cat} individual median ≤ 5.0 ms × h ({})", r.project),
                value: r.individual_total.median,
                limit: 5.0 * h,
                pass: r.individual_total.median <= 5.0 * h,
                note: format!(
                    "P95 {:.3} ms, tex {:.3} ms, {} rows",
                    r.individual_total.p95, r.individual_tex.median, r.lines
                ),
            });
        }
        let small = units_big
            .iter()
            .copied()
            .find(|r| r.category == cat && r.project.contains("book-10-"));
        let big100 = units_big
            .iter()
            .copied()
            .find(|r| r.category == cat && r.project.contains("book-100-"));
        if let (Some(a), Some(b)) = (small, big100) {
            let ratio = b.individual_total.median / a.individual_total.median;
            gates.push(Gate {
                name: format!("size independence {cat} (100p/10p individual median)"),
                value: ratio,
                limit: 1.25,
                pass: (0.75..=1.25).contains(&ratio),
                note: format!(
                    "{:.3} ms vs {:.3} ms",
                    b.individual_total.median, a.individual_total.median
                ),
            });
        }
    }
    report.pass = gates.iter().all(|g| g.pass);
    report.gates = gates;
}
