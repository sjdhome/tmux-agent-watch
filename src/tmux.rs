//! The single tmux boundary of the program.
//!
//! Only read-only tmux verbs (`list-panes`, `capture-pane`) may ever appear in
//! this module. The tool must never send control commands to tmux.

use std::process::{Command, Stdio};

const LIST_PANES_FORMAT: &str = "#{session_name}\t#{window_index}\t#{window_name}\t#{pane_id}\t#{pane_pid}\t#{pane_title}\t#{pane_current_command}";

/// One tmux pane as reported by `list-panes -a`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub session: String,
    pub window_index: u32,
    pub window_name: String,
    /// Stable pane id, e.g. "%3". Used as the tracking key.
    pub pane_id: String,
    pub pane_pid: u32,
    /// Maps to the detection engine's `osc_title` input.
    pub pane_title: String,
    /// Cheap hint only; pid-based identification stays authoritative.
    pub current_command: String,
}

/// tmux could not be queried (server not running, binary missing, ...).
#[derive(Debug, Clone)]
pub struct TmuxUnavailable {
    pub message: String,
}

pub fn list_panes() -> Result<Vec<PaneInfo>, TmuxUnavailable> {
    let output = Command::new("tmux")
        .args(["list-panes", "-a", "-F", LIST_PANES_FORMAT])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| TmuxUnavailable {
            message: format!("failed to run tmux: {err}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        let message = if message.is_empty() {
            format!("tmux exited with {}", output.status)
        } else {
            message.to_string()
        };
        return Err(TmuxUnavailable { message });
    }

    Ok(parse_list_panes(&String::from_utf8_lossy(&output.stdout)))
}

/// Visible screen text of a pane. `None` when the pane vanished between
/// `list-panes` and the capture, or capture failed for any other reason.
pub fn capture_pane(pane_id: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", pane_id])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_list_panes(stdout: &str) -> Vec<PaneInfo> {
    stdout.lines().filter_map(parse_pane_line).collect()
}

/// Parse one `list-panes` line. Session/window names cannot contain tabs, and
/// the title is the second-to-last field, so a fixed 7-way split is safe;
/// malformed lines are skipped rather than aborting the cycle.
fn parse_pane_line(line: &str) -> Option<PaneInfo> {
    let fields: Vec<&str> = line.splitn(7, '\t').collect();
    if fields.len() != 7 {
        return None;
    }
    Some(PaneInfo {
        session: fields[0].to_string(),
        window_index: fields[1].parse().ok()?,
        window_name: fields[2].to_string(),
        pane_id: fields[3].to_string(),
        pane_pid: fields[4].parse().ok()?,
        pane_title: fields[5].to_string(),
        current_command: fields[6].to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_regular_line() {
        let line = "work\t1\tapi\t%3\t4242\tmy-host\tzsh";
        let pane = parse_pane_line(line).expect("line should parse");
        assert_eq!(pane.session, "work");
        assert_eq!(pane.window_index, 1);
        assert_eq!(pane.window_name, "api");
        assert_eq!(pane.pane_id, "%3");
        assert_eq!(pane.pane_pid, 4242);
        assert_eq!(pane.pane_title, "my-host");
        assert_eq!(pane.current_command, "zsh");
    }

    #[test]
    fn session_names_with_spaces_survive() {
        let line = "my project\t0\tmain\t%0\t100\ttitle\tclaude";
        let pane = parse_pane_line(line).expect("line should parse");
        assert_eq!(pane.session, "my project");
        assert_eq!(pane.current_command, "claude");
    }

    #[test]
    fn malformed_lines_are_skipped() {
        assert_eq!(parse_pane_line(""), None);
        assert_eq!(parse_pane_line("only\tfour\tfields\there"), None);
        assert_eq!(parse_pane_line("s\tNaN\tw\t%1\t10\tt\tcmd"), None);
        assert_eq!(parse_pane_line("s\t0\tw\t%1\tNaN\tt\tcmd"), None);
    }

    #[test]
    fn parse_list_panes_keeps_good_lines() {
        let stdout = "a\t0\tw\t%0\t1\tt\tzsh\nbroken line\nb\t2\tw2\t%5\t9\tt2\tclaude\n";
        let panes = parse_list_panes(stdout);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].pane_id, "%0");
        assert_eq!(panes[1].pane_id, "%5");
    }
}
