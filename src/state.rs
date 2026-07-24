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

use crate::detect::Agent;
use crate::detect::engine::{Detection, EngineState};

const PENDING_IDLE_CONFIRMATIONS: u8 = 2;

#[derive(Debug)]
struct PaneTracker {
    agent: Agent,
    published: EngineState,
    pending_idle: u8,
    startup_grace: bool,
    last_seen_cycle: u64,
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
    pub fn apply(&mut self, pane_id: &str, agent: Agent, detection: &Detection) -> EngineState {
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
                        pending_idle: 0,
                        startup_grace: true,
                        last_seen_cycle: cycle,
                    },
                );
                return EngineState::Idle;
            }
        };
        tracker.last_seen_cycle = cycle;

        if tracker.startup_grace {
            tracker.startup_grace = false;
            tracker.published = EngineState::Idle;
            return tracker.published;
        }

        if detection.skip {
            tracker.pending_idle = 0;
            return tracker.published;
        }

        let next = detection.state;
        let holds_working_to_idle = tracker.published == EngineState::Working
            && matches!(next, EngineState::Idle | EngineState::Unknown)
            && !detection.visible;

        if holds_working_to_idle {
            tracker.pending_idle = tracker.pending_idle.saturating_add(1);
            if tracker.pending_idle < PENDING_IDLE_CONFIRMATIONS {
                return tracker.published;
            }
        }
        tracker.pending_idle = 0;
        tracker.published = next;
        tracker.published
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
        let state = store.apply(pane, Agent::Claude, det);
        store.prune();
        state
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
        let state = store.apply("%1", Agent::Codex, &detection(EngineState::Working, true));
        store.prune();
        assert_eq!(state, EngineState::Idle, "new agent gets startup grace");
    }
}
