# Design notes: Pi ask_user blocked detection

## Why

The user asked `tmux-agent-watch` to classify Pi as **blocked** while the
`ask_user` tool from
`~/Projects/pi-agent-extensions/src/ask-user.ts` is waiting for an answer.
The integration should remain separate from Herdr-derived logic so future
manifest or engine refreshes do not create avoidable merge conflicts.

## When

2026-07-29 12:32:43 +0800.

## Investigation

The extension source was inspected at pi-agent-extensions commit
`fbfe510774e0c92b6d6d288b306c97e6098dd197`.

During `execute`, the tool awaits a non-overlay `ctx.ui.custom()` component.
That component temporarily replaces Pi's editor. Every interactive state
renders one of five `helpText()` variants immediately above a full-width `─`
bottom border. Calling `done()` after submit or cancel removes the component,
so the marker exists only while the tool is waiting.

No externally readable state file, dedicated process, OSC title, or other
state channel is exposed. Reading Pi session JSON would also lack a reliable
pane-to-session mapping and could leave ambiguous in-progress records after a
crash. Modifying the external extension to publish a new signal would widen
the scope to a second repository and require deployment/reload coordination.

## Decision and implementation

A project-native observer was added in `src/pi_ask_user.rs`, outside
`src/detect/`, which contains the Herdr-derived engine and manifests. It
recognizes the current `helpText()` prefixes only when the next line is a
full-width component border. The structural condition avoids treating source
code, documentation, or completed tool transcript text as an active prompt.
Prefix matching retains detection when the help line is truncated in a
moderately narrow pane.

`src/snapshot.rs` applies the observer only to identified Pi panes. A match
returns a visible `Blocked` detection with rule id `pi_ask_user_waiting` and
takes precedence over the Herdr manifest. Non-Pi panes and Pi screens without
the marker continue through the existing manifest engine unchanged. The
existing debounce state machine was not modified.

`--explain` reports whether the native Pi observer matched before listing the
Herdr rule trace. Unit tests cover all current help states, truncation, false
positive guards, Pi-only scoping, and observer precedence.

No changes were made to the external pi-agent-extensions repository or to
vendored Herdr files.

## Open issues

- **The observer is coupled to `ask-user.ts` rendering text and border
  structure.** If `helpText()` or the custom component framing changes, the
  observer can stop matching. Resolve by updating the prefixes and fixtures,
  or preferably by introducing a stable externally observable marker in the
  extension and then updating this project to consume it.
- **Extremely narrow panes can truncate a help line before its distinguishing
  prefix.** Such a pane may not be classified as blocked. Resolve by widening
  the pane or by adding the stable external signal described above.
- **The exit transition has not been separately human-verified.** On
  2026-07-29, the user confirmed that a live Pi `ask_user` prompt made the
  watcher display **blocked**, validating the active-prompt path. The user
  should still confirm that answering or cancelling makes the pane leave
  blocked; a captured successful transition closes this issue.
