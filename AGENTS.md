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
  documented behaviour changes the spec line with it, in the same unit. The
  spec is the orchestrator's to write; a builder that finds a rule missing
  reports it instead of editing it.
- `README.md` — the user-facing description and the config keys.
- `TODO.txt` — the orchestrator's state file: the unit in flight, the open
  work, the order.
- `CHANGELOG.md` — the release notes. The `## [Unreleased]` section is the
  orchestrator's: a release's notes describe what shipped, every
  user-visible change included.
- `docs/COLORS.md` — the theme vocabulary (`UiTheme` keys).
- `crates/engine/VENDORED.md` — the vendored engine's boundary: every
  intentional deviation from upstream leaf, and what the app may change.
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
- **A rule written in more than one place drifts.** Give it one home, and
  find the other copies by *grepping for the pattern* — never by line
  number: a brief that enumerated instances by line number missed one twice
  in a row, and the review caught both.
- Match the surrounding code: one concern per module under
  `crates/app/src/`, `ui.rs` for the frame and `ui/tests.rs` for the widget
  tests. Comments explain *why*, not what the line does.
- Rust 2021; workspace-wide dependencies live in the root `Cargo.toml`.
