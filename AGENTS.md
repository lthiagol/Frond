# frond — notes for agents

frond is a read-only Obsidian-style terminal markdown viewer (Rust; ratatui
0.30 + crossterm 0.29). Workspace crates: `engine` (vendored markdown
renderer), `vault` (index, frontmatter, wikilinks, `.base` tables, the
watcher, git status, and the one guarded write), `app` (the TUI).

## The loop you run in

This project runs the orchestration flow. Load the global skill
`orchestration-flow` (by its ID) for the loop mechanics — the hub-and-spoke
topology, the review gate, the round cap, and who commits. A builder never
commits; the orchestrator commits after a clean review.

The project's name in the flow's own words is `frond` — the same as the
prefix in `.orchestration.json`, and the title on the toast that announces a
closed unit.

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

- The app never writes into the vault. The one write path is the checkbox
  toggle: `vault::tasks::write_toggle` in `crates/vault/src/tasks.rs`,
  called from `crates/app/src/app.rs`. It is guarded (the note's mtime and
  size, its repository's cleanliness, a re-scan of the marker's state) and
  the write is atomic. `crates/app/src/tasks.rs` only renders the
  checkboxes — it writes nothing. The app does write its own session
  state, under `~/.local/state/frond/`, outside the vault.
- The gate is `make check` = `fmt` + `clippy` + `test` + `smoke`. The smoke
  suites drive the TUI through a pty (`uv run --with pyte`); they are slow
  and they are part of the gate. CI (`.github/workflows/ci.yml`) runs the
  same gates, with `fmt` as `--check`.
- `crates/engine` is vendored from leaf and keeps its module-wide
  `#![allow(dead_code)]` (in `crates/engine/src/markdown/mod.rs`). Keep that
  allow; do not tidy the vendored engine.
- **A rule written in more than one place drifts.** Give it one home, and
  find the other copies by *grepping for the pattern* — never by line
  number: a brief that enumerated instances by line number missed one twice
  in a row, and the review caught both.
- Match the surrounding code: one concern per module under
  `crates/app/src/`. A module with a large suite keeps it in
  `<module>/tests.rs` — `ui.rs` for the frame and `ui/tests.rs` for its
  widgets, `app.rs` for the app state and `app/tests.rs` for its tests.
  Comments explain *why*, not what the line does.
- Rust 2021; a dependency used by more than one crate is declared once in
  the root `Cargo.toml` (`[workspace.dependencies]`) and taken with
  `x.workspace = true`.
