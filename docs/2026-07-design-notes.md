# Design notes: initial implementation

## Why

The user wanted to monitor the state (working / blocked on human input /
idle) of AI coding agents running in an **existing tmux server**, without
depending on herdr (a terminal-multiplexing agent runtime that solves this
inside its own PTYs). herdr's detection design was studied first and found to
map cleanly onto tmux primitives, so this tool ports that approach instead of
inventing new heuristics. Latency requirement was relaxed (≤5s), so polling
was chosen over tmux control-mode events.

## When

2026-07-24. Vendored manifests and ported logic are from herdr commit
`c0fb777ed7c7950c6a2f397113c1842c2e679306`.

## How

Three layers, mirroring herdr's evidence chain minus the hook channel:

1. **Process identification** (`src/platform/macos.rs`, `src/detect/mod.rs`)
   — PID-based macOS APIs (`proc_pidinfo` → `e_tpgid`, `proc_listpids`,
   `sysctl KERN_PROCARGS2`), plus herdr's argv-unwrapping (node/python/shell
   wrappers, symlink canonicalization, process-group-leader preference).
2. **Screen detection** (`src/detect/engine.rs`, `src/detect/manifests/`) —
   a reimplementation of herdr's manifest engine v3 consuming the 19 vendored
   TOMLs verbatim. Input is `capture-pane -p` (visible screen, trailing blank
   lines trimmed) + `#{pane_title}` as `osc_title`; `osc_progress` is always
   empty because tmux does not track OSC 9;4.
3. **Debounce** (`src/state.rs`) — herdr's stabilization adapted to a 2s
   cadence: 2-poll confirmation for Working→plain-Idle, one-cycle startup
   grace, `skip_state_update` keeps prior state, Blocked never delayed.

Key decisions and their reasons:

- **Vendor manifests + reimplement engine** rather than write simplified
  rules: the manifests encode hard-won knowledge (transcript viewers freeze
  state, model pickers are not "blocked", …). `deny_unknown_fields` plus a
  region-coverage test make `cargo test` the drift gate after
  `scripts/refresh-manifests.sh`.
- **Nested-PTY descent** (`nested_pty_children`, `identify_pane_agent`): not
  in the original plan. Discovered during M2 on the user's machine — panes
  run `koshell`, a wrapper shell that allocates an inner PTY and runs zsh
  (and the agent) on a *different* tty, so a single `e_tpgid` hop only sees
  koshell. Resolution walks children whose controlling tty differs from
  their parent's (budgeted BFS, 8 anchors).
- **Manifest ids are canonical agent labels** ("agy", "copilot"), not file
  stems — lookup is keyed by the parsed `id` field.
- **Raw scan for `--once`, debounced scan for the TUI**: a fresh state store
  would force every pane into startup-grace Idle on a one-shot run.
- **Threads + mpsc, no tokio**: one `list-panes` + one `capture-pane` per
  agent pane every 2s is trivially synchronous; the UI thread polls input at
  100ms so a stalled scan never freezes keys.
- **3 states, not herdr's 4**: herdr's Done-vs-Idle needs "has the user seen
  this pane" signals that a read-only external tool cannot collect honestly.
  Engine `unknown` renders as idle.

## Open issues

- **Interactive TUI not yet human-verified.** All logic is unit-tested (51
  tests) and `--once`/`--explain` were verified against the user's live tmux
  (10 agent panes, correct states including an `osc_title_working` hit), but
  the TUI loop itself needs the README's manual E2E checklist run by a
  human. Resolves when the user runs the checklist.
- **`#{pane_title}` staleness/defaults**: tmux's default pane title is the
  hostname; if some agent manifest's `osc_title` regex ever matches a
  hostname-shaped string it would false-positive. Not observed in practice;
  `--explain` surfaces it if it happens.
- **Linux port**: `src/platform/mod.rs` compile-errors on non-macOS. A
  `linux.rs` reading `/proc/<pid>/stat` (tpgid) + `/proc/<pid>/cmdline` slots
  in without changing callers. Resolves when someone needs it.
- **pi manifest is thin upstream** (single `working_literal` rule), so pi
  panes mostly report idle via fallback. Matches herdr behavior; improves
  automatically when upstream manifests are refreshed.
