//! One poll cycle: tmux discovery → process identification → screen
//! detection → tree building.

use crate::detect::engine::{self, Detection, DetectionInput, EngineState};
use crate::detect::{self, Agent};
use crate::tmux::{self, PaneInfo};

/// A pane with a detected agent, carrying the raw (un-debounced) detection.
#[derive(Debug, Clone)]
pub struct AgentPane {
    pub info: PaneInfo,
    pub agent: Agent,
    pub detection: Detection,
}

#[derive(Debug, Clone)]
pub struct WindowNode {
    pub index: u32,
    pub name: String,
    pub panes: Vec<AgentPane>,
}

#[derive(Debug, Clone)]
pub struct SessionNode {
    pub name: String,
    pub windows: Vec<WindowNode>,
}

/// What one scan produced; the UI renders this without further tmux access.
#[derive(Debug, Clone)]
pub enum Snapshot {
    TmuxUnavailable { message: String },
    Tree(Vec<SessionNode>),
}

/// Run one raw scan (no debounce) — used by `--once`, where a fresh state
/// store would force every pane into startup-grace Idle. Only panes with an
/// identified agent are captured and evaluated.
pub fn scan() -> Snapshot {
    scan_inner(None)
}

/// Run one scan with debounced states folded through the store — the TUI
/// path.
pub fn scan_debounced(store: &mut crate::state::StateStore) -> Snapshot {
    scan_inner(Some(store))
}

fn scan_inner(mut store: Option<&mut crate::state::StateStore>) -> Snapshot {
    let panes = match tmux::list_panes() {
        Ok(panes) => panes,
        Err(unavailable) => {
            return Snapshot::TmuxUnavailable {
                message: unavailable.message,
            };
        }
    };

    if let Some(store) = store.as_deref_mut() {
        store.begin_cycle();
    }

    let mut agent_panes = Vec::new();
    for pane in panes {
        let Some((agent, _)) = detect::identify_pane_agent(pane.pane_pid) else {
            continue;
        };
        // Pane may vanish between list and capture; drop it for this cycle.
        let Some(raw_screen) = tmux::capture_pane(&pane.pane_id) else {
            continue;
        };
        let mut detection = detect_screen(agent, &raw_screen, &pane.pane_title);
        if let Some(store) = store.as_deref_mut() {
            detection.state = store.apply(&pane.pane_id, agent, &detection);
        }
        agent_panes.push(AgentPane {
            info: pane,
            agent,
            detection,
        });
    }

    if let Some(store) = store {
        store.prune();
    }

    Snapshot::Tree(build_tree(agent_panes))
}

/// Evaluate the manifest engine for one pane's captured screen.
pub fn detect_screen(agent: Agent, raw_screen: &str, pane_title: &str) -> Detection {
    let screen = detection_screen(raw_screen);
    let input = DetectionInput {
        screen: &screen,
        osc_title: pane_title,
        // tmux does not track OSC 9;4 progress sequences.
        osc_progress: "",
    };
    match detect::manifest_id(agent).and_then(detect::manifests::get) {
        Some(manifest) => engine::evaluate(manifest, input),
        None => engine::KNOWN_AGENT_IDLE_FALLBACK,
    }
}

/// Trace every rule of the pane's manifest for `--explain`.
pub fn explain_screen(
    agent: Agent,
    raw_screen: &str,
    pane_title: &str,
) -> Option<(Detection, Vec<engine::RuleTrace>, String)> {
    let screen = detection_screen(raw_screen);
    let input = DetectionInput {
        screen: &screen,
        osc_title: pane_title,
        osc_progress: "",
    };
    let manifest = detect::manifest_id(agent).and_then(detect::manifests::get)?;
    let (detection, traces) = engine::explain(manifest, input);
    Some((detection, traces, screen))
}

/// The engine input is the visible screen with trailing blank rows trimmed,
/// matching herdr's detection text semantics.
fn detection_screen(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(0);
    lines[..end].join("\n")
}

/// Group agent panes into session → window → pane, preserving tmux order.
/// Branches without agent panes never appear.
fn build_tree(panes: Vec<AgentPane>) -> Vec<SessionNode> {
    let mut sessions: Vec<SessionNode> = Vec::new();
    for pane in panes {
        let session = match sessions
            .iter_mut()
            .find(|session| session.name == pane.info.session)
        {
            Some(session) => session,
            None => {
                sessions.push(SessionNode {
                    name: pane.info.session.clone(),
                    windows: Vec::new(),
                });
                #[allow(clippy::unwrap_used)] // pushed on the line above
                sessions.last_mut().unwrap()
            }
        };
        let window = match session
            .windows
            .iter_mut()
            .find(|window| window.index == pane.info.window_index)
        {
            Some(window) => window,
            None => {
                session.windows.push(WindowNode {
                    index: pane.info.window_index,
                    name: pane.info.window_name.clone(),
                    panes: Vec::new(),
                });
                #[allow(clippy::unwrap_used)] // pushed on the line above
                session.windows.last_mut().unwrap()
            }
        };
        window.panes.push(pane);
    }
    sessions
}

/// Count of panes per engine state, for the header line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    pub working: usize,
    pub blocked: usize,
    pub idle: usize,
}

pub fn state_counts(sessions: &[SessionNode]) -> StateCounts {
    let mut counts = StateCounts::default();
    for session in sessions {
        for window in &session.windows {
            for pane in &window.panes {
                match pane.detection.state {
                    EngineState::Working => counts.working += 1,
                    EngineState::Blocked => counts.blocked += 1,
                    // Unknown renders as idle in the UI.
                    EngineState::Idle | EngineState::Unknown => counts.idle += 1,
                }
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(session: &str, window: u32, id: &str) -> AgentPane {
        AgentPane {
            info: PaneInfo {
                session: session.to_string(),
                window_index: window,
                window_name: format!("w{window}"),
                pane_id: id.to_string(),
                pane_pid: 1,
                pane_title: String::new(),
                current_command: String::new(),
            },
            agent: Agent::Claude,
            detection: engine::KNOWN_AGENT_IDLE_FALLBACK,
        }
    }

    #[test]
    fn detection_screen_trims_only_trailing_blank_lines() {
        assert_eq!(detection_screen("a\n\nb\n\n\n"), "a\n\nb");
        assert_eq!(detection_screen("\n\n"), "");
        assert_eq!(detection_screen("x"), "x");
    }

    #[test]
    fn tree_groups_by_session_then_window_preserving_order() {
        let tree = build_tree(vec![
            pane("s1", 1, "%1"),
            pane("s1", 1, "%2"),
            pane("s2", 0, "%3"),
            pane("s1", 3, "%4"),
        ]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "s1");
        assert_eq!(tree[0].windows.len(), 2);
        assert_eq!(tree[0].windows[0].panes.len(), 2);
        assert_eq!(tree[1].name, "s2");
    }
}
