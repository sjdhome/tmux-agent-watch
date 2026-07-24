# Design notes: state duration column

## Why

The user wanted the TUI to show how long each agent pane has been in its
current state (working / blocked / idle), so it is visible at a glance
whether an agent has been blocked for 10 seconds or an hour.

## When

2026-07-24.

## How

The debounce layer (`src/state.rs`) is the single source of truth for the
*published* state, so the duration clock lives there: `PaneTracker` gained
`published_since: Instant`, and `StateStore::apply` now returns a
`PublishedState { state, since }` pair. The snapshot carries the `Instant`
(not a pre-formatted duration) in `AgentPane::state_since`, so the UI —
which redraws every 100ms — renders a live-ticking value instead of one
that jumps every 2s poll. `ui.rs` formats it compactly (`45s`, `12m`,
`1h05m`, `2d1h`) in a right-aligned 6-char column between the state glyph
and the agent label.

Key decisions:

- **The clock resets on *display-state* change, not engine-state change.**
  Engine `Unknown` renders as idle everywhere, so Idle↔Unknown flips must
  not restart the clock (`display_state` fold in `state.rs`). Held
  (pending-idle) polls and `skip` detections also keep the clock.
- **Working→Idle debounce: the clock restarts at the confirming poll**, not
  retroactively at the first idle observation. The error is bounded by one
  debounce cycle (≤4s) and not worth the extra bookkeeping.
- **`--once` / `--json` are unchanged** (user decision): a one-shot scan
  has no history, so a duration is impossible in principle; no placeholder
  field was added.
- **Testability**: `apply` delegates to a private `apply_at(..., now)` so
  tests drive the clock with explicit instants instead of sleeping.

## Limitations (inherent, not open issues)

- Durations measure from when *this watcher* observed the change. On
  launch, every pane's clock starts at zero (startup grace publishes Idle);
  the tool cannot know how long an agent was already working before it
  started watching.
- A pane that vanishes for a cycle and reappears restarts with startup
  grace, and its clock restarts too — consistent with the state machine's
  existing behavior.

## Open issues

None specific to this feature.
