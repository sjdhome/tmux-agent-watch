//! Native observation of the Pi `ask_user` extension's active custom UI.
//!
//! This deliberately lives outside `detect`, whose engine and manifests track
//! Herdr. The extension exposes no external state channel: while its tool is
//! waiting, `ctx.ui.custom()` replaces Pi's editor with a component whose help
//! line sits immediately above a full-width `─` bottom border. Once the user
//! answers or cancels, that component (and therefore this marker) disappears.

/// Prefixes of every help line rendered by the active `ask_user` component.
/// Prefix matching preserves detection when Pi truncates the line in a narrow
/// pane. Keep these synchronized with `helpText()` in the extension.
const HELP_PREFIXES: &[&str] = &[
    "Enter save Other",
    "Enter submit • Tab/Shift+Tab",
    "Enter submit answer • Tab/Shift+Tab",
    "↑↓ move • Space toggle • Enter next/edit Other",
    "↑↓ move • Enter choose/edit Other",
];

const MIN_BORDER_WIDTH: usize = 8;

/// Whether the visible screen contains the active `ask_user` custom UI.
///
/// A help prefix alone is insufficient because source code or old transcript
/// text may contain it. Requiring the component's bottom border on the very
/// next line makes the observation structural and avoids stale completed tool
/// calls, whose custom UI has already been removed.
pub fn is_waiting_for_user(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    lines
        .windows(2)
        .any(|pair| is_help_line(pair[0]) && is_horizontal_border(pair[1]))
}

fn is_help_line(line: &str) -> bool {
    let line = line.trim();
    HELP_PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

fn is_horizontal_border(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() >= MIN_BORDER_WIDTH && line.chars().all(|character| character == '─')
}

#[cfg(test)]
mod tests {
    use super::*;

    const BORDER: &str = "────────────────────────────────────────────────────────────────";

    #[test]
    fn recognizes_every_current_help_state() {
        for help in [
            "Enter save Other • Esc back to options",
            "Enter submit • Tab/Shift+Tab navigate • Esc cancel",
            "Enter submit answer • Tab/Shift+Tab navigate • Esc cancel",
            "↑↓ move • Space toggle • Enter next/edit Other • Tab navigate • Esc cancel",
            "↑↓ move • Enter choose/edit Other • Tab navigate • Esc cancel",
        ] {
            let screen = format!("Questions for you\n\n {help}\n{BORDER}\n");
            assert!(is_waiting_for_user(&screen), "missed help line: {help}");
        }
    }

    #[test]
    fn recognizes_a_truncated_help_line() {
        let screen = format!("Question\n Enter submit • Tab/Shift+Tab\n{BORDER}\n");
        assert!(is_waiting_for_user(&screen));
    }

    #[test]
    fn rejects_help_text_without_adjacent_component_border() {
        let source_excerpt = r#"
return " Enter submit • Tab/Shift+Tab navigate • Esc cancel";
}
"#;
        assert!(!is_waiting_for_user(source_excerpt));
    }

    #[test]
    fn rejects_unrelated_help_above_a_border() {
        let screen = format!("Press Enter to submit\n{BORDER}\n");
        assert!(!is_waiting_for_user(&screen));
    }

    #[test]
    fn rejects_short_rule_like_text() {
        assert!(!is_waiting_for_user("Enter save Other\n────\n"));
    }
}
