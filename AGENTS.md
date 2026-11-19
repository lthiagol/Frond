# frond — notes for agents

frond is a read-only Obsidian-style terminal markdown viewer (Rust; ratatui
0.30 + crossterm 0.29). Workspace crates: `engine` (vendored markdown
renderer), `vault` (index, wikilinks, git status), `app` (the TUI).

## The loop you run in

This project runs the orchestration flow. Load the global skill
`orchestration-flow` (by its ID) for the loop mechanics — the hub-and-spoke
topology, the review gate, the round cap, and who commits. A builder never
commits; the orchestrator commits after a clean review.

## Read before you touch code

- `SPEC.md` — the product spec. It is normative: a change that alters
  documented behaviour changes the spec line with it, in the same unit.
- `README.md` — the user-facing description and the config keys.
- `TODO.txt` — the orchestrator's state file: the unit in flight, the open
  work, the order.
- `docs/COLORS.md` — the theme vocabulary (`UiTheme` keys).
- `TODO.md` — local planning file (gitignored): past investigations and the
  measurements behind them.

## Project rules

- The app is read-only. The one write path is the checkbox toggle
  (`crates/app/src/tasks.rs`), and it is guarded.
- The gate is `make check` = `fmt` + `clippy` + `test` + `smoke`. The smoke
  suites drive the TUI through a pty (`uv run --with pyte`); they are slow
  and they are part of the gate.
- `crates/engine` is vendored from leaf and keeps its module-wide
  `#![allow(dead_code)]`. Keep that allow; do not tidy the vendored engine.
- Match the surrounding code: one concern per module under
  `crates/app/src/`, `ui.rs` for the frame and `ui/tests.rs` for the widget
  tests. Comments explain *why*, not what the line does.
- Rust 2021; workspace-wide dependencies live in the root `Cargo.toml`.
