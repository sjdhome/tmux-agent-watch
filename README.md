# tmux-agent-watch

A read-only TUI that watches your existing tmux server and shows every pane
running an AI coding agent (Claude Code, Codex, Gemini CLI, pi, and ~20
others) as a session → window → pane tree with a traffic-light state
indicator:

- 🔴 **blocked**: the agent is waiting for human input (permission prompt,
  question)
- 🟢 **working**: the agent is actively doing something
- ⚪ **idle**: finished, prompt visible

(In the TUI these render as colored `●` glyphs.)

Next to the state, the TUI shows how long the pane has been in it
(`45s`, `12m`, `1h05m`, `2d1h`). The clock measures from the moment *this
tool* observed the state change, so every pane starts at zero when the
watcher launches; `--once` has no history and therefore no duration column.

State changes appear within ~5 seconds (2s polling plus up to one debounce
cycle). The tool never sends control commands to tmux — it only runs
`list-panes` and `capture-pane` — and never touches your agents' configs.

## Install

```sh
make install                    # /usr/local/bin (may need sudo)
make install PREFIX=~/.local    # ~/.local/bin
make uninstall [PREFIX=...]
```

`DESTDIR` is honored for staged installs. `cargo install --path .` also works
if you prefer ~/.cargo/bin.

Requires macOS (v1; `src/platform/` is the seam for a future Linux port) and
tmux. Run it inside or outside tmux.

## Usage

```sh
tmux-agent-watch                 # TUI
tmux-agent-watch --once          # one scan, plain-text tree
tmux-agent-watch --once --json   # one scan, JSON
tmux-agent-watch --explain %3    # trace every detection rule for one pane
```

TUI keys: `q`/`Esc` quit, `j`/`k` or arrows scroll, `g` top, PgUp/PgDn page.

## How it works

Detection reuses [herdr](https://github.com/ogulcancelik/herdr)'s
evidence-based approach (Apache-2.0, see NOTICE):

1. **Process identification** — from each pane's `#{pane_pid}`, the
   foreground process group of the pane's terminal is resolved via
   `proc_pidinfo`/`proc_listpids`, unwrapping runtime wrappers (`node`,
   `python`, shells) and nested-PTY wrapper shells to find the agent process.
2. **Screen detection** — for agent panes, `capture-pane -p` (the visible
   screen, immune to copy-mode scrolling) plus `#{pane_title}` (OSC title)
   are matched against per-agent rule manifests vendored verbatim from herdr
   (`src/detect/manifests/*.toml`) by a compatible rule engine
   (regions, AND/OR/NOT gates, priority winner selection).
3. **Debounce** — Working→Idle needs two consecutive confirming polls unless
   the screen shows explicit idle chrome; new panes get one cycle of startup
   grace; agent-owned viewer screens (`skip_state_update`) keep the previous
   state. Transitions into/out of Blocked are never delayed.

When a detection looks wrong, `--explain %N` prints the exact screen input
and every rule's evaluation — fix or override the manifest from there.

## Manual end-to-end checklist

1. In tmux, start `claude` in one pane and keep a plain shell elsewhere.
2. Run `tmux-agent-watch` — only the agent branch appears, state idle (dim).
3. Give the agent a long task — green **working** within 5s.
4. Trigger a permission prompt — red **blocked** within 5s.
5. Let it finish — back to idle within ~5s.
6. Kill the agent pane — the branch disappears on the next cycle.
7. `tmux kill-server` — the TUI shows "tmux unavailable" and keeps retrying;
   restart tmux and the tree returns.
8. Any disagreement: `tmux-agent-watch --explain %N`.

## Development

```sh
make help         # list targets
make check        # format check + clippy -D warnings + tests
```

## Refreshing detection manifests

```sh
make refresh-manifests            # from ../herdr, then runs the drift gate
make refresh-manifests HERDR=/path/to/herdr
```

Update the herdr commit hash in NOTICE afterwards.

## Rejected features

- **Jump-to-pane / any tmux control** — would require `select-window`-class
  commands; this tool stays strictly read-only.
- **Hook-based state reporting** (herdr's higher-fidelity channel) — requires
  installing hook scripts into each agent's config; out of scope for v1.
- **Linux** — not yet; contributions welcome via `src/platform/linux.rs`.

## License

Apache-2.0. Detection manifests and detection logic derived from
[herdr](https://github.com/ogulcancelik/herdr) — see NOTICE.
