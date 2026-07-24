//! Pure rendering: Snapshot → ratatui widgets. No state mutation here.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::detect;
use crate::detect::engine::EngineState;
use crate::snapshot::{self, Snapshot};

const HEADER_STYLE: Style = Style::new().add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().fg(Color::DarkGray);

/// Traffic-light indicator for a pane state.
fn state_span(state: EngineState) -> Span<'static> {
    match state {
        EngineState::Blocked => Span::styled(
            "● blocked",
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        EngineState::Working => Span::styled("● working", Style::new().fg(Color::Green)),
        EngineState::Idle | EngineState::Unknown => Span::styled("● idle   ", DIM),
    }
}

/// Flatten the snapshot into styled body lines.
pub fn tree_lines(snapshot: Option<&Snapshot>) -> Vec<Line<'static>> {
    let Some(snapshot) = snapshot else {
        return vec![Line::styled("scanning…", DIM)];
    };
    match snapshot {
        Snapshot::TmuxUnavailable { message } => vec![
            Line::raw(""),
            Line::styled(
                format!("tmux unavailable: {message}"),
                Style::new().fg(Color::Yellow),
            ),
            Line::styled("retrying every 2s…", DIM),
        ],
        Snapshot::Tree(sessions) if sessions.is_empty() => {
            vec![Line::raw(""), Line::styled("no agent panes detected", DIM)]
        }
        Snapshot::Tree(sessions) => {
            let mut lines = Vec::new();
            for session in sessions {
                lines.push(Line::styled(session.name.clone(), HEADER_STYLE));
                for window in &session.windows {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::raw(format!("{}: {}", window.index, window.name)),
                    ]));
                    for pane in &window.panes {
                        let title = pane.info.pane_title.clone();
                        lines.push(Line::from(vec![
                            Span::raw("    "),
                            state_span(pane.detection.state),
                            Span::raw(format!(
                                "  {:10} {:>5}  ",
                                detect::agent_label(pane.agent),
                                pane.info.pane_id
                            )),
                            Span::styled(title, DIM),
                        ]));
                    }
                }
            }
            lines
        }
    }
}

/// Header: state counts and scan freshness.
pub fn header_line(snapshot: Option<&Snapshot>, updated_secs_ago: Option<u64>) -> Line<'static> {
    let mut spans = vec![Span::styled("tmux-agent-watch", HEADER_STYLE)];
    if let Some(Snapshot::Tree(sessions)) = snapshot {
        let counts = snapshot::state_counts(sessions);
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("{} blocked", counts.blocked),
            if counts.blocked > 0 {
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                DIM
            },
        ));
        spans.push(Span::styled(" · ", DIM));
        spans.push(Span::styled(
            format!("{} working", counts.working),
            if counts.working > 0 {
                Style::new().fg(Color::Green)
            } else {
                DIM
            },
        ));
        spans.push(Span::styled(" · ", DIM));
        spans.push(Span::styled(format!("{} idle", counts.idle), DIM));
    }
    if let Some(secs) = updated_secs_ago {
        spans.push(Span::styled(format!("   updated {secs}s ago"), DIM));
    }
    Line::from(spans)
}

pub fn draw(frame: &mut Frame, lines: &[Line<'static>], header: Line<'static>, scroll: u16) {
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(Paragraph::new(header), header_area);
    frame.render_widget(
        Paragraph::new(lines.to_vec()).scroll((scroll, 0)),
        body_area,
    );
    frame.render_widget(
        Paragraph::new(Line::styled("q quit · j/k scroll", DIM)),
        footer_area,
    );
}

/// Visible body height for scroll clamping.
pub fn body_height(area: Rect) -> u16 {
    area.height.saturating_sub(2)
}
