//! One-shot CLI modes: `--once` (text), `--once --json`, `--explain <pane>`.
//! The debugging gold when a manifest misbehaves.

use crate::detect::engine::EngineState;
use crate::snapshot::{self, Snapshot};
use crate::{detect, tmux};

pub fn run_once(json: bool) -> i32 {
    match snapshot::scan() {
        Snapshot::TmuxUnavailable { message } => {
            eprintln!("tmux unavailable: {message}");
            1
        }
        Snapshot::Tree(sessions) => {
            if json {
                print_json(&sessions);
            } else {
                print_text(&sessions);
            }
            0
        }
    }
}

fn print_text(sessions: &[snapshot::SessionNode]) {
    if sessions.is_empty() {
        println!("no agent panes detected");
        return;
    }
    let counts = snapshot::state_counts(sessions);
    println!(
        "{} blocked · {} working · {} idle",
        counts.blocked, counts.working, counts.idle
    );
    for session in sessions {
        println!("{}", session.name);
        for window in &session.windows {
            println!("  {}: {}", window.index, window.name);
            for pane in &window.panes {
                println!(
                    "    {} {:10} {:8} rule={}",
                    pane.info.pane_id,
                    detect::agent_label(pane.agent),
                    display_state(pane.detection.state),
                    pane.detection.rule_id.as_deref().unwrap_or("(fallback)"),
                );
            }
        }
    }
}

fn print_json(sessions: &[snapshot::SessionNode]) {
    let value = serde_json::json!({
        "sessions": sessions.iter().map(|session| {
            serde_json::json!({
                "name": session.name,
                "windows": session.windows.iter().map(|window| {
                    serde_json::json!({
                        "index": window.index,
                        "name": window.name,
                        "panes": window.panes.iter().map(|pane| {
                            serde_json::json!({
                                "pane_id": pane.info.pane_id,
                                "agent": detect::agent_label(pane.agent),
                                "state": display_state(pane.detection.state),
                                "rule_id": pane.detection.rule_id,
                                "skip": pane.detection.skip,
                                "visible": pane.detection.visible,
                                "title": pane.info.pane_title,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    println!("{value:#}");
}

/// Engine `unknown` renders as idle everywhere user-facing.
fn display_state(state: EngineState) -> &'static str {
    match state {
        EngineState::Unknown => "idle",
        other => other.label(),
    }
}

pub fn run_explain(pane_id: &str) -> i32 {
    let panes = match tmux::list_panes() {
        Ok(panes) => panes,
        Err(unavailable) => {
            eprintln!("tmux unavailable: {}", unavailable.message);
            return 1;
        }
    };
    let Some(pane) = panes.iter().find(|pane| pane.pane_id == pane_id) else {
        eprintln!("pane {pane_id} not found");
        return 1;
    };
    let Some((agent, process_name)) = detect::identify_pane_agent(pane.pane_pid) else {
        eprintln!(
            "pane {pane_id}: no agent identified (pid {})",
            pane.pane_pid
        );
        return 1;
    };
    println!(
        "pane {pane_id}: agent={} (process {process_name}, pid {})",
        detect::agent_label(agent),
        pane.pane_pid
    );
    println!("osc_title: {:?}", pane.pane_title);

    let Some(raw_screen) = tmux::capture_pane(pane_id) else {
        eprintln!("pane {pane_id}: capture failed");
        return 1;
    };
    let Some((detection, traces, screen)) =
        snapshot::explain_screen(agent, &raw_screen, &pane.pane_title)
    else {
        println!("agent has no manifest; state falls back to idle");
        return 0;
    };

    println!(
        "resolved: state={} rule={} skip={} visible={}",
        display_state(detection.state),
        detection.rule_id.as_deref().unwrap_or("(fallback)"),
        detection.skip,
        detection.visible
    );
    println!("\n--- screen input ({} lines) ---", screen.lines().count());
    for line in screen.lines() {
        println!("| {line}");
    }
    println!("\n--- rules (file order) ---");
    for trace in traces {
        println!(
            "[{}] {} priority={} region={} state={}",
            if trace.matched { "MATCH" } else { "     " },
            trace.rule_id,
            trace.priority,
            trace.region,
            trace.state.label(),
        );
        if trace.matched && trace.region != "whole_recent" {
            for line in trace.region_text.lines().take(6) {
                println!("        > {line}");
            }
        }
    }
    0
}
