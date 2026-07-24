//! Per-pane state machine adapting herdr's detection debounce to a 2s poll
//! cadence:
//!
//! - A newly appeared agent pane gets one cycle of startup grace, published
//!   as Idle, to absorb noisy startup screens.
//! - Working → Idle without visible idle evidence needs 2 consecutive
//!   confirming polls (≈4s); visible evidence flips immediately.
//! - Transitions into Blocked are never delayed, and neither are transitions
//!   out of it: the state the user must act on stays honest.
//! - `skip` detections (agent-owned viewer screens) keep the prior state.

use std::collections::HashMap;
use std::time::Instant;

use crate::detect::Agent;
use crate::detect::engine::{Detection, EngineState};

const PENDING_IDLE_CONFIRMATIONS: u8 = 2;

/// A debounced state together with when it last changed *as displayed*:
/// `since` restarts only when the user-visible bucket (blocked / working /
/// idle) changes, so engine Idle↔Unknown flips do not reset the clock.
#[derive(Debug, Clone, Copy)]
pub struct PublishedState {
    pub state: EngineState,
    pub since: Instant,
}

/// Engine Unknown renders as idle everywhere user-facing; fold it before
/// deciding whether the displayed state actually changed.
fn display_state(state: EngineState) -> EngineState {
    match state {
        EngineState::Unknown => EngineState::Idle,
        other => other,
    }
}

#[derive(Debug)]
struct PaneTracker {
    agent: Agent,
    published: EngineState,
    published_since: Instant,
    pending_idle: u8,
    startup_grace: bool,
    last_seen_cycle: u64,
}

impl PaneTracker {
    fn published_state(&self) -> PublishedState {
        PublishedState {
            state: self.published,
            since: self.published_since,
        }
    }
}

#[derive(Debug, Default)]
pub struct StateStore {
    trackers: HashMap<String, PaneTracker>,
    cycle: u64,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a poll cycle; call `prune` after applying every pane.
    pub fn begin_cycle(&mut self) {
        self.cycle += 1;
    }

    /// Fold one raw detection into the pane's published state.
    pub fn apply(&mut self, pane_id: &str, agent: Agent, detection: &Detection) -> PublishedState {
        self.apply_at(pane_id, agent, detection, Instant::now())
    }

    fn apply_at(
        &mut self,
        pane_id: &str,
        agent: Agent,
        detection: &Detection,
        now: Instant,
    ) -> PublishedState {
        let cycle = self.cycle;
        let tracker = match self.trackers.get_mut(pane_id) {
            Some(tracker) if tracker.agent == agent => tracker,
            _ => {
                // New pane, or a different agent took the pane over.
                self.trackers.insert(
                    pane_id.to_string(),
                    PaneTracker {
                        agent,
                        published: EngineState::Idle,
                        published_since: now,
                        pending_idle: 0,
                        startup_grace: true,
                        last_seen_cycle: cycle,
                    },
                );
                return PublishedState {
                    state: EngineState::Idle,
                    since: now,
                };
            }
        };
        tracker.last_seen_cycle = cycle;

        if tracker.startup_grace {
            tracker.startup_grace = false;
            tracker.published = EngineState::Idle;
            return tracker.published_state();
        }

        if detection.skip {
            tracker.pending_idle = 0;
            return tracker.published_state();
        }

        let next = detection.state;
        let holds_working_to_idle = tracker.published == EngineState::Working
            && matches!(next, EngineState::Idle | EngineState::Unknown)
            && !detection.visible;

        if holds_working_to_idle {
            tracker.pending_idle = tracker.pending_idle.saturating_add(1);
            if tracker.pending_idle < PENDING_IDLE_CONFIRMATIONS {
                return tracker.published_state();
            }
        }
        tracker.pending_idle = 0;
        if display_state(tracker.published) != display_state(next) {
            tracker.published_since = now;
        }
        tracker.published = next;
        tracker.published_state()
    }

    /// Drop trackers for panes not seen this cycle (pane closed). A pane id
    /// that reappears later starts fresh with startup grace.
    pub fn prune(&mut self) {
        let cycle = self.cycle;
        self.trackers
            .retain(|_, tracker| tracker.last_seen_cycle == cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(state: EngineState, visible: bool) -> Detection {
        Detection {
            state,
            rule_id: Some("test".to_string()),
            skip: false,
            visible,
        }
    }

    fn skip_detection() -> Detection {
        Detection {
            state: EngineState::Unknown,
            rule_id: Some("viewer".to_string()),
            skip: true,
            visible: false,
        }
    }

    fn cycle(store: &mut StateStore, pane: &str, det: &Detection) -> EngineState {
        store.begin_cycle();
        let state = store.apply(pane, Agent::Claude, det).state;
        store.prune();
        state
    }

    fn cycle_at(
        store: &mut StateStore,
        pane: &str,
        det: &Detection,
        now: Instant,
    ) -> PublishedState {
        store.begin_cycle();
        let published = store.apply_at(pane, Agent::Claude, det, now);
        store.prune();
        published
    }

    #[test]
    fn new_pane_gets_one_cycle_startup_grace() {
        let mut store = StateStore::new();
        // First sighting: grace, forced Idle even though screen says working.
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Working, true)),
            EngineState::Idle
        );
        // Second sighting consumes grace, still Idle for this cycle.
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Working, true)),
            EngineState::Idle
        );
        // Third: live.
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Working, true)),
            EngineState::Working
        );
    }

    fn warmed_store(pane: &str, state: EngineState) -> StateStore {
        let mut store = StateStore::new();
        cycle(&mut store, pane, &detection(state, true));
        cycle(&mut store, pane, &detection(state, true));
        cycle(&mut store, pane, &detection(state, true));
        store
    }

    #[test]
    fn working_to_plain_idle_needs_two_confirmations() {
        let mut store = warmed_store("%1", EngineState::Working);
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Idle, false)),
            EngineState::Working,
            "first plain idle poll is held"
        );
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Idle, false)),
            EngineState::Idle,
            "second consecutive idle poll flips"
        );
    }

    #[test]
    fn visible_idle_flips_immediately() {
        let mut store = warmed_store("%1", EngineState::Working);
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Idle, true)),
            EngineState::Idle
        );
    }

    #[test]
    fn working_resumption_resets_pending_idle() {
        let mut store = warmed_store("%1", EngineState::Working);
        cycle(&mut store, "%1", &detection(EngineState::Idle, false));
        cycle(&mut store, "%1", &detection(EngineState::Working, true));
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Idle, false)),
            EngineState::Working,
            "pending idle must restart after working resumed"
        );
    }

    #[test]
    fn blocked_is_immediate_in_both_directions() {
        let mut store = warmed_store("%1", EngineState::Working);
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Blocked, true)),
            EngineState::Blocked
        );
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Idle, false)),
            EngineState::Idle,
            "leaving blocked is not debounced"
        );
    }

    #[test]
    fn skip_keeps_prior_state() {
        let mut store = warmed_store("%1", EngineState::Working);
        assert_eq!(
            cycle(&mut store, "%1", &skip_detection()),
            EngineState::Working
        );
    }

    #[test]
    fn vanished_pane_restarts_with_grace() {
        let mut store = warmed_store("%1", EngineState::Working);
        // Pane absent for one cycle: begin + prune without apply.
        store.begin_cycle();
        store.prune();
        assert_eq!(
            cycle(&mut store, "%1", &detection(EngineState::Working, true)),
            EngineState::Idle,
            "reappearing pane starts with startup grace"
        );
    }

    #[test]
    fn agent_change_restarts_tracking() {
        let mut store = warmed_store("%1", EngineState::Working);
        store.begin_cycle();
        let state = store
            .apply("%1", Agent::Codex, &detection(EngineState::Working, true))
            .state;
        store.prune();
        assert_eq!(state, EngineState::Idle, "new agent gets startup grace");
    }

    fn secs(n: u64) -> std::time::Duration {
        std::time::Duration::from_secs(n)
    }

    #[test]
    fn since_restarts_only_on_display_state_change() {
        let mut store = StateStore::new();
        let t0 = Instant::now();
        // t0: new pane (grace forces Idle), clock starts.
        assert_eq!(
            cycle_at(&mut store, "%1", &detection(EngineState::Working, true), t0).since,
            t0
        );
        // t0+2s: grace consumed, still displayed idle — clock keeps t0.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Working, true),
            t0 + secs(2),
        );
        assert_eq!((p.state, p.since), (EngineState::Idle, t0));
        // t0+4s: flips to working — clock restarts.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Working, true),
            t0 + secs(4),
        );
        assert_eq!((p.state, p.since), (EngineState::Working, t0 + secs(4)));
        // t0+6s: still working — clock unchanged.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Working, true),
            t0 + secs(6),
        );
        assert_eq!(p.since, t0 + secs(4));
    }

    #[test]
    fn idle_unknown_flips_keep_since() {
        let mut store = StateStore::new();
        let t0 = Instant::now();
        cycle_at(&mut store, "%1", &detection(EngineState::Idle, true), t0);
        cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Idle, true),
            t0 + secs(2),
        );
        // Unknown displays as idle: no reset even though the engine state
        // differs.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Unknown, false),
            t0 + secs(4),
        );
        assert_eq!(p.since, t0);
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Idle, false),
            t0 + secs(6),
        );
        assert_eq!(p.since, t0);
    }

    #[test]
    fn held_and_skipped_polls_keep_since() {
        let mut store = StateStore::new();
        let t0 = Instant::now();
        cycle_at(&mut store, "%1", &detection(EngineState::Working, true), t0);
        cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Working, true),
            t0 + secs(2),
        );
        let working = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Working, true),
            t0 + secs(4),
        );
        assert_eq!(working.state, EngineState::Working);
        // Skip detection keeps state and clock.
        let p = cycle_at(&mut store, "%1", &skip_detection(), t0 + secs(6));
        assert_eq!((p.state, p.since), (EngineState::Working, working.since));
        // First plain idle poll is held: still working, clock untouched.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Idle, false),
            t0 + secs(8),
        );
        assert_eq!((p.state, p.since), (EngineState::Working, working.since));
        // Second confirming poll flips; clock restarts at the flip.
        let p = cycle_at(
            &mut store,
            "%1",
            &detection(EngineState::Idle, false),
            t0 + secs(10),
        );
        assert_eq!((p.state, p.since), (EngineState::Idle, t0 + secs(10)));
    }
}
