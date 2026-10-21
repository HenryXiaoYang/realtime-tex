//! What the live path records per span between layouts, in one record per span with one place
//! that decides when each fact expires. Before, each fact was a map of its own with its own
//! clearing rule, spread over the session; a fact cleared in one place and kept in another is
//! the stale-state bug this module exists to prevent.
//!
//! Three lifetimes:
//! - **until the next layout** (`on_new_layout`): what the live path guessed and the pass now
//!   knows — a borrowed context, the latest live rows and placement, the counters seen;
//! - **for one layout version** (tagged; other versions are ignored and dropped at the next
//!   layout): a probe verdict, an over-budget mark, an internal error;
//! - **until the preamble changes** (`on_preamble_change`): a leaked definition, a quarantine
//!   after the unit hung or crashed the engine (lifted earlier once it is edited after a clean
//!   pass, see `lift_quarantine_if_edited`), over-budget strikes.

use super::*;

/// An over-budget mark that is not tied to a layout version: the unit hung or crashed the
/// engine and stays on the full compile until it is edited after a clean pass (or, after
/// `MAX_CRASH_RETRIES` crashes in a row, until the preamble changes).
pub(super) const QUARANTINED: u64 = u64::MAX;

/// Crashes in a row (with no live compile of the unit in between) after which a unit stays on
/// the full compile until the preamble changes, even when its text changes: a unit that keeps
/// killing the engine would restart it on every keystroke.
pub(super) const MAX_CRASH_RETRIES: u32 = 3;

/// The unit hung or crashed the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Crash {
    /// Hash of the text that did it.
    pub hash: u64,
    /// Crashes in a row: since the live engine last typeset the unit (or the preamble changed).
    pub count: u32,
    /// Source revision of the text that did it.
    pub revision: Revision,
    /// A pass of that revision or a later one compiled without errors: the state the live
    /// engine gets from the layout no longer comes from the broken text.
    pub clean_pass: bool,
}

#[derive(Debug, Default)]
pub(super) struct LiveUnit {
    // --- until the next layout ---
    /// Compiled on a borrowed context: (unit the context came from, span the rows are placed
    /// against, whether they follow it).
    pub derived: Option<(ParaId, ParaId, bool)>,
    /// Row count of the latest live result (anchors a unit placed after it).
    pub rows: Option<i64>,
    /// Where the latest live result was placed: (page, x of its first row, its last baseline);
    /// a unit typed after it on a borrowed context chains onto it.
    pub place: Option<(i64, i64, i64)>,
    /// Counters the latest live compile advanced (a change renumbers what follows: pass).
    pub counters: Option<BTreeMap<String, i64>>,
    // --- for one layout version ---
    /// Probe verdict (probe mode): (layout version, verdict).
    pub probe: Option<(u64, ProbeVerdict)>,
    /// Over the fast budget: (layout version when measured, or QUARANTINED; ms).
    pub over_budget: Option<(u64, u64)>,
    /// A Lua error of the server's own on this unit: (layout version, message). The unit waits
    /// for the pass until the next layout, then gets another chance.
    pub internal_error: Option<(u64, String)>,
    // --- until the preamble changes ---
    /// The compile changed the meaning of a control sequence (a definition leaked).
    pub leak: Option<String>,
    /// Consecutive over-budget compiles (the first ones may be loading fonts).
    pub slow_strikes: u32,
    /// The unit hung or crashed the engine.
    pub crash: Option<Crash>,
}

impl LiveUnit {
    fn is_empty(&self) -> bool {
        self.derived.is_none()
            && self.rows.is_none()
            && self.place.is_none()
            && self.counters.is_none()
            && self.probe.is_none()
            && self.over_budget.is_none()
            && self.internal_error.is_none()
            && self.leak.is_none()
            && self.slow_strikes == 0
            && self.crash.is_none()
    }
}

/// The live records of all spans (one mutex in `Shared`; take it for short, non-nested scopes).
#[derive(Debug, Default)]
pub(super) struct LiveUnits(HashMap<ParaId, LiveUnit>);

impl LiveUnits {
    pub fn get(&self, id: ParaId) -> Option<&LiveUnit> {
        self.0.get(&id)
    }

    /// The record of `id`, created empty when there is none.
    pub fn unit(&mut self, id: ParaId) -> &mut LiveUnit {
        self.0.entry(id).or_default()
    }

    /// The probe verdict of `id` for layout `lv`.
    pub fn probe_for(&self, id: ParaId, lv: u64) -> Option<ProbeVerdict> {
        self.get(id)?
            .probe
            .as_ref()
            .filter(|(v, _)| *v == lv)
            .map(|(_, p)| p.clone())
    }

    /// The internal error of `id` recorded for layout `lv`.
    pub fn internal_error_for(&self, id: ParaId, lv: u64) -> Option<String> {
        self.get(id)?
            .internal_error
            .as_ref()
            .filter(|(v, _)| *v == lv)
            .map(|(_, m)| m.clone())
    }

    /// The unit hung or crashed the engine on text `hash` of source revision `revision`:
    /// quarantine it.
    pub fn quarantine(&mut self, id: ParaId, hash: u64, revision: Revision) {
        let u = self.unit(id);
        u.over_budget = Some((QUARANTINED, 0));
        u.crash = Some(Crash {
            hash,
            count: u.crash.map_or(1, |c| c.count + 1),
            revision,
            clean_pass: false,
        });
    }

    /// A pass of source revision `revision` compiled without errors: crashes at that revision
    /// or before no longer taint the state the live engine starts from.
    pub fn on_clean_pass(&mut self, revision: Revision) {
        for u in self.0.values_mut() {
            if let Some(c) = u.crash.as_mut().filter(|c| c.revision <= revision) {
                c.clean_pass = true;
            }
        }
    }

    /// The live engine typeset `id`: its earlier crashes no longer count as crashes in a row.
    pub fn forget_crashes(&mut self, id: ParaId) {
        if let Some(u) = self.0.get_mut(&id) {
            u.crash = None;
        }
    }

    /// Lift the quarantine of `id` when its text (`hash`) is no longer the one that hung or
    /// crashed the engine (e.g. a half-typed command that the user finished) and a pass has
    /// compiled cleanly since, unless the unit has crashed `MAX_CRASH_RETRIES` times in a row.
    /// The crash count stays until the live engine typesets the unit (`forget_crashes`). Without
    /// the clean pass, the live engine would start the unit from state the broken text left
    /// behind and could crash again.
    pub fn lift_quarantine_if_edited(&mut self, id: ParaId, hash: u64) {
        if let Some(u) = self.0.get_mut(&id) {
            let quarantined = u.over_budget.is_some_and(|(v, _)| v == QUARANTINED);
            if quarantined
                && u.crash
                    .is_some_and(|c| c.clean_pass && c.hash != hash && c.count < MAX_CRASH_RETRIES)
            {
                u.over_budget = None;
            }
        }
    }

    /// Is `id` compiled on a borrowed context (since the last layout)?
    pub fn is_derived(&self, id: ParaId) -> bool {
        self.get(id).is_some_and(|u| u.derived.is_some())
    }

    /// A layout was installed (version `lv`): the pass's placements and contexts supersede what
    /// the live path guessed, and facts tagged with an older layout no longer apply.
    pub fn on_new_layout(&mut self, lv: u64) {
        for u in self.0.values_mut() {
            u.derived = None;
            u.rows = None;
            u.place = None;
            u.counters = None;
            if u.probe.as_ref().is_some_and(|(v, _)| *v != lv) {
                u.probe = None;
            }
            if u.over_budget
                .is_some_and(|(v, _)| v != lv && v != QUARANTINED)
            {
                u.over_budget = None;
            }
            if u.internal_error.as_ref().is_some_and(|(v, _)| *v != lv) {
                u.internal_error = None;
            }
        }
        self.prune();
    }

    /// The running server was replaced by one with the same preamble and newer bibliography
    /// data: probe verdicts compared that server's output, the rest still holds.
    pub fn on_server_swap(&mut self) {
        for u in self.0.values_mut() {
            u.probe = None;
        }
    }

    /// The preamble changed: the engine restarts, so verdicts, budget marks, quarantines and
    /// leaks about the old engine no longer apply.
    pub fn on_preamble_change(&mut self) {
        for u in self.0.values_mut() {
            u.probe = None;
            u.over_budget = None;
            u.leak = None;
            u.crash = None;
        }
        self.prune();
    }

    fn prune(&mut self) {
        self.0.retain(|_, u| !u.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_rules() {
        let mut l = LiveUnits::default();
        let a = ParaId(1);
        let b = ParaId(2);
        {
            let u = l.unit(a);
            u.derived = Some((b, b, true));
            u.rows = Some(3);
            u.place = Some((1, 2, 3));
            u.probe = Some((5, ProbeVerdict::Verified));
            u.over_budget = Some((5, 9));
            u.internal_error = Some((5, "e".into()));
            u.leak = Some("\\x".into());
            u.slow_strikes = 2;
        }
        l.unit(b).over_budget = Some((QUARANTINED, 0));
        assert!(l.is_derived(a));
        assert!(l.probe_for(a, 5).is_some() && l.probe_for(a, 4).is_none());
        assert_eq!(l.internal_error_for(a, 5).as_deref(), Some("e"));
        // the same layout version installed again keeps the version-tagged facts
        l.on_new_layout(5);
        assert!(!l.is_derived(a) && l.get(a).unwrap().rows.is_none());
        assert!(l.probe_for(a, 5).is_some());
        assert!(l.get(a).unwrap().over_budget.is_some());
        // a newer layout drops them; quarantine, leak and strikes stay
        l.on_new_layout(6);
        let u = l.get(a).unwrap();
        assert!(u.probe.is_none() && u.over_budget.is_none() && u.internal_error.is_none());
        assert!(u.leak.is_some() && u.slow_strikes == 2);
        assert_eq!(l.get(b).unwrap().over_budget, Some((QUARANTINED, 0)));
        // a preamble change clears quarantines and leaks; strikes stay; empty records go
        l.on_preamble_change();
        assert!(l.get(b).is_none(), "empty record pruned");
        assert!(l.get(a).unwrap().leak.is_none() && l.get(a).unwrap().slow_strikes == 2);
    }

    #[test]
    fn quarantine_lifts_when_edited_after_a_clean_pass() {
        let mut l = LiveUnits::default();
        let a = ParaId(1);
        let quarantined = |l: &LiveUnits| {
            l.get(a)
                .is_some_and(|u| u.over_budget == Some((QUARANTINED, 0)))
        };
        l.quarantine(a, 10, 5);
        // the text that crashed the engine stays quarantined, also across layouts
        l.lift_quarantine_if_edited(a, 10);
        l.on_new_layout(7);
        assert!(quarantined(&l));
        // edited, but no clean pass since the crash: the layout still carries the broken text
        l.lift_quarantine_if_edited(a, 11);
        assert!(quarantined(&l));
        // a clean pass of an older revision does not count
        l.on_clean_pass(4);
        l.lift_quarantine_if_edited(a, 11);
        assert!(quarantined(&l));
        // a clean pass of that revision or later, then an edit: back to the live engine
        l.on_clean_pass(6);
        l.lift_quarantine_if_edited(a, 10);
        assert!(quarantined(&l), "still the text that crashed it");
        l.lift_quarantine_if_edited(a, 11);
        assert!(!quarantined(&l));
        // a live compile in between resets the count: these crashes are not in a row
        l.forget_crashes(a);
        for n in 1..MAX_CRASH_RETRIES {
            l.quarantine(a, 20 + u64::from(n), 10 + u64::from(n));
        }
        l.on_clean_pass(100);
        l.lift_quarantine_if_edited(a, 99);
        assert!(
            !quarantined(&l),
            "fewer than MAX_CRASH_RETRIES since the live compile"
        );
        // after MAX_CRASH_RETRIES crashes in a row, only a preamble change lifts it
        l.quarantine(a, 30, 40);
        assert_eq!(l.get(a).unwrap().crash.unwrap().count, MAX_CRASH_RETRIES);
        l.on_clean_pass(100);
        l.lift_quarantine_if_edited(a, 99);
        assert!(quarantined(&l));
        l.on_preamble_change();
        assert!(l.get(a).is_none());
    }
}
