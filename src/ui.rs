//! Pure rendering: Snapshot → ratatui widgets. No state mutation here.

use std::time::{Duration, Instant};

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

/// Compact elapsed time: 45s, 12m, 1h05m, 2d1h. Widest realistic value is
/// 6 chars ("23h59m"), which callers use as the column width.
fn format_duration(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d{}h", secs / 86400, (secs % 86400) / 3600)
    }
}

/// How long the pane has been in its current state, right-aligned; blank
/// when the snapshot carries no history (raw scans).
fn duration_span(state_since: Option<Instant>, now: Instant) -> Span<'static> {
    let text = state_since
        .map(|since| format_duration(now.saturating_duration_since(since)))
        .unwrap_or_default();
    Span::styled(format!(" {text:>6}"), DIM)
}

/// Flatten the snapshot into styled body lines. `now` is the render instant
/// used to display each pane's time in its current state.
pub fn tree_lines(snapshot: Option<&Snapshot>, now: Instant) -> Vec<Line<'static>> {
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
                            duration_span(pane.state_since, now),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats_scale_with_magnitude() {
        let s = Duration::from_secs;
        assert_eq!(format_duration(s(0)), "0s");
        assert_eq!(format_duration(s(45)), "45s");
        assert_eq!(format_duration(s(60)), "1m");
        assert_eq!(format_duration(s(12 * 60 + 34)), "12m");
        assert_eq!(format_duration(s(3600 + 5 * 60)), "1h05m");
        assert_eq!(format_duration(s(23 * 3600 + 59 * 60)), "23h59m");
        assert_eq!(format_duration(s(2 * 86400 + 3600)), "2d1h");
    }

    #[test]
    fn duration_span_is_blank_without_history() {
        assert_eq!(duration_span(None, Instant::now()).content, "       ");
    }
}
