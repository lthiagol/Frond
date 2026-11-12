# frond

A read-only-first Obsidian-style terminal UI for your markdown vault — tree
sidebar, wikilink navigation, tags, backlinks, global search, git sync status,
and `.base` table views.

Markdown rendering is powered by [leaf](https://github.com/rivolink/leaf), the
main engine of this project (vendored into `crates/engine`).

## Features

- Rendered markdown (leaf's engine, vendored): tables, lists, callouts, syntax highlighting, LaTeX, Mermaid
- Collapsible folder-tree sidebar with status glyphs and changed-file markers, plus a tag view
- Wikilink navigation: resolution per Obsidian rules, heading anchors, disambiguation popup, back/forward history with scroll restore
- Backlinks panel; global vault search with jump-to-line
- Git sync status (clean/dirty/ahead/behind) and per-note changed markers — read-only
- `.base` table views (filters, sort, groupBy) with live refresh from the vault watcher
- Guarded checkbox toggling — the single write feature (git-clean + unchanged-file guards, atomic write)
- Themes (built-in presets + custom TOML themes), live config hot-reload, full mouse support

## Requirements

- Rust stable (via [rustup](https://rustup.rs))
- A terminal with 256-color or truecolor support
- [`uv`](https://docs.astral.sh/uv/) — only for the optional smoke test suites

## Install

Clone the repository and install the `frond` binary with Cargo:

```sh
git clone git@github.com:lthiagol/Frond.git
cd Frond
cargo install --path crates/app
```

This puts `frond` in `~/.cargo/bin` (make sure it is on your `PATH`).

To build without installing:

```sh
make build      # debug build:   target/debug/frond
make release    # release build: target/release/frond
```

## Usage

```sh
frond                          # no vault given: prompts for the vault directory
frond --vault ~/notes          # open a vault (index.md is shown first)
frond --vault ~/notes project.md   # open a specific note inside the vault
frond some-note.md             # single-file viewer, no vault
```

If you start `frond` without `--vault` (and no `vault` key in the config
file), it asks for the vault directory. Relative paths and `~/` are accepted.

Options (all optional; flags override the config file):

| Flag | Meaning |
|---|---|
| `--vault <dir>` | Vault directory (wikilinks, tags, backlinks, search, sidebar) |
| `--theme <name>` | Theme name (see Themes below) |
| `--watch[=true\|false]` | Start with watch mode on or off (reload on file changes) |

Running `frond --help` is not supported: an unknown option prints the usage
line and exits with status 2.

Press `?` inside the app for the full key list. The most used keys:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Scroll / move selection |
| `Tab` | Show or hide the sidebar |
| `h` / `l` | Focus sidebar / content |
| `Enter` | Open the focused link |
| `Backspace` | Go back |
| `s` | Search the vault |
| `b` | Toggle backlinks |
| `Shift+T` | Cycle themes |
| `q` | Quit |

Clicking a rendered checkbox (or pressing `Enter` on it) cycles its state.
This is the only thing that writes to your vault, and it is guarded: it
refuses unless the note's git repository is clean and in sync and the file
is unchanged since it was read.

## Try it with the test vault

The repository ships a small lorem-ipsum vault in `testvault/`:

```sh
make run        # launches frond and prompts for the vault path
# enter: testvault
```

Or directly:

```sh
cargo run -p app --bin frond -- --vault testvault
```

## Configuration

Config file: `~/.config/frond/config.toml` (override the location with the
`FROND_CONFIG` environment variable).

```toml
theme = "ocean"                 # theme name
vault = "/home/you/notes"       # default vault when --vault is not given (absolute path; no ~ expansion)
watch = true                    # reload on file changes
```

Missing file means defaults. A malformed file prints a warning and falls back
to defaults.

## Themes

- 4 presets (`arctic`, `forest`, `ocean`, `solarized`) and 14 built-ins
  (catppuccin, dracula, gruvbox, nord, rose-pine, tokyonight variants).
- Custom themes: TOML files in `~/.config/frond/themes/` (override with the
  `FROND_THEMES` environment variable). Format: `base` + `syntax` + `[ui]` +
  `[markdown]`. Unknown keys are reported as warnings, never silently ignored.
- Resolution order: `--theme` > `FROND_THEME` env var > config `theme` > default (`ocean`).
- Colour roles for every key are documented in [`docs/COLORS.md`](docs/COLORS.md).

## Development

```sh
make check      # fmt + clippy + tests + smoke suites
make test       # unit and integration tests only
make smoke      # pty smoke suites (runs with `uv run --with pyte`)
make clean      # remove build artifacts
```

`SPEC.md` is the design reference; `CHANGELOG.md` lists releases.

## Acknowledgments

The markdown rendering engine is vendored from
[leaf](https://github.com/rivolink/leaf) (MIT); deviations are documented in
`crates/engine/VENDORED.md`.

## License

MIT — see [LICENSE](LICENSE).
