# frond

A read-only-first Obsidian-style terminal UI for your markdown vault — tree
sidebar, wikilink navigation, tags, backlinks, global search, git sync status,
and `.base` table views.

Markdown rendering is powered by [leaf](https://github.com/rivolink/leaf), the
main engine of this project (vendored into `crates/engine`).

## Features

- Rendered markdown (leaf's engine, vendored): tables, lists, callouts, syntax highlighting, LaTeX, Mermaid
- Collapsible folder-tree sidebar (resizable with `<`/`>` or the config) with status glyphs and changed-file markers, plus a tag view
- The selected row of every panel is one full-width bar, legible in every theme
- Click-to-open everywhere — sidebar rows, backlink rows, in-note links — with opened notes revealing in the tree and focus following the reader
- Wikilink navigation: resolution per Obsidian rules, heading anchors, disambiguation popup, back/forward history with scroll restore
- Backlinks panel; global vault search with jump-to-line, and a regular-expression mode (`re:` prefix)
- Quick jumper (`Ctrl+K`): fuzzy jump to a note by name
- The last search stays active: `n`/`N` walk its occurrences in the open note
- Outline (`o`): the sidebar shows the open note's headings, indented by level; `Enter` jumps to one
- Scrollbars that are position controls: a press jumps, a drag follows, and the thumb highlights under the mouse
- The link under the mouse is highlighted, so you see which one you are about to open
- An optional line-number gutter in the note pane (the `line_numbers` config key)
- Status bar: the note's size, its scroll position as a percentage, the watch state and the last render time
- Git sync status (clean/dirty/ahead/behind) and per-note changed markers — read-only
- `.base` table views (filters, sort, groupBy) with live refresh from the vault watcher
- Guarded checkbox toggling — the single write feature (git-clean + unchanged-file guards, atomic write)
- Session memory: the last open note with its scroll and the sidebar's mode/selection/folder expansion, saved per vault on quit and restored on the next start
- Themes (built-in presets + custom TOML themes), live config hot-reload, full mouse support (toggleable capture for native text selection)

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
frond                          # no vault given: the vault picker
                               # (on your [vaults], else type a path)
frond --vault ~/notes          # open a vault (index.md is shown first, unless a saved session restores another note)
frond --vault ~/notes project.md   # open a specific note inside the vault
frond --vault work             # open the [vaults] entry named work
frond some-note.md             # open a file without a vault: no sidebar
```

With two or more `[vaults]` in the config, `frond` with no `--vault` and no
file opens on a vault picker: type to filter (by name or path), `↑`/`↓` to
select, `Enter` to open, `Esc` to quit. The last vault you opened (else the
`vault` key) is preselected. With exactly one `[vaults]` entry and no
`vault` key, that vault opens directly.

In the picker, a query that starts with `/`, `~` or `.` is a path, not a
filter: the list shows `open <path>`, `Tab` completes the directory name
(hidden ones only after a `.`), and `Enter` opens it. Vaults opened that
way are remembered (the five most recent, `recent-vaults` in the state
directory) and listed after the `[vaults]` entries, tagged `recent`.

`v` opens the same picker inside the app to switch vaults: the open vault
is saved as on quit and the chosen one opens as a fresh start (its own
session restored), keeping your theme, mouse, watch, sidebar, metadata,
line-number and margin settings. `Esc` closes the picker.

If you start `frond` without `--vault` (and no `vault` key or `[vaults]` in
the config file), the picker opens on your recent paths: type a path to a
vault. A relative path starts with `./` or `../` (a bare name filters the
list instead), and `~/` is accepted.
Without `--vault` there is no sidebar, but the current directory is still
indexed (the opened file's own folder is *not* searched), so wikilinks,
backlinks and the search keep working.

Options (all optional; flags override the config file):

| Flag | Meaning |
|---|---|
| `--vault <dir\|name>` | Vault directory (wikilinks, tags, backlinks, search, sidebar), or a `[vaults]` name |
| `--theme <name>` | Theme name (see Themes below) |
| `--watch[=true\|false]` | Start with watch mode on or off (reload on file changes) |
| `--mouse[=true\|false]` | Mouse capture on/off (off = native text selection; `M` toggles at runtime) |
| `--config-show` | Print a commented default `config.toml` and exit |
| `--help` / `--version` | Full usage / version |

`frond --config-show > ~/.config/frond/config.toml` is the easiest way to
bootstrap a configuration file.

Press `?` inside the app for the full key list. The most used keys:

| Key | Action |
|---|---|
| `j` / `k`, ↑ / ↓ | Scroll / move selection |
| `f` | Move the focus to the next pane: sidebar → note → backlinks |
| `h` / `l`, ← / → | Collapse / expand a folder or tag (sidebar tree and tags only) |
| `Space` | Fold / unfold the selected folder or tag |
| `Tab` | Show or hide the sidebar |
| `<` / `>` | Narrow / widen the sidebar |
| `t` | Sidebar mode: tree ⇄ tags |
| `o` | Sidebar mode: the open note's headings (the outline) |
| `m` | Show / hide the note's metadata block |
| `Enter` | Open the focused link |
| `Backspace` | Go back |
| `s` | Search the vault |
| `Ctrl+K` | Quick jumper: jump to a note by name |
| `b` | Toggle backlinks |
| `M` | Toggle mouse capture |
| `Shift+T` | Cycle themes (within the group set by `Ctrl+T`) |
| `Ctrl+T` | Theme group for `Shift+T`: all → light → dark (session only) |
| `v` | Switch vaults: a `[vaults]` entry, a recent path or a typed one |
| `]` / `[`, `1`–`9` | Next / previous / nth table view (`.base` files) |
| `q` | Quit |

Clicking a rendered checkbox (or pressing `Enter` on it) cycles its state.
This is the only thing that writes to your vault, and it is guarded: it
refuses unless the note's git repository is clean and in sync and the file
is unchanged since it was read.

## Try it with the test vault

The repository ships a small lorem-ipsum vault in `testvault/`:

```sh
make run        # launches frond on the vault picker
# type: ./testvault
```

Or directly:

```sh
cargo run -p frond --bin frond -- --vault testvault
```

## Configuration

Config file: `~/.config/frond/config.toml` (override the location with the
`FROND_CONFIG` environment variable). Every key is optional; `frond
--config-show` prints a fully commented template pinned to the real key
vocabulary.

```toml
theme = "ocean"                 # theme name
vault = "~/notes"               # default vault when --vault is not given (~ is expanded);
                                # a [vaults] name works too (write ./work for a directory named like one)
watch = true                    # reload on file changes
mouse = true                    # mouse capture (off = native text selection)
sidebar_width = 40              # sidebar columns, clamped 24-120
metadata = true                 # render the note's frontmatter block (m toggles)
theme_rotation = "all"          # Shift+T group at startup: all, light or dark
margin = 2                      # blank columns on each side of the note (0-10)
margin_y = 1                    # blank rows above and below the note (0-5)
icons = "nerd"                  # status-bar icons: nerd (needs a Nerd Font) or unicode
line_numbers = false            # show a gutter with the note's source line numbers

[vaults]                        # named vaults: the vault picker's entries (keep this table last)
notes = "~/notes"
work = "~/work/notes"
```

Missing file means defaults. A malformed file prints a warning and falls back
to defaults.

frond also remembers where you were: the last open note with its scroll and
the sidebar's state (mode, selection, folder expansion) are saved per vault on
quit (`~/.local/state/frond/`, `FROND_SESSION` overrides the directory) and
restored on the next start. The last vault opened is recorded there too
(`last-vault`), for the vault picker's preselection, and so are the
vaults opened from a typed path (`recent-vaults`).

## Themes

- 4 presets (`arctic`, `forest`, `ocean`, `solarized`) and 14 built-ins
  (catppuccin, dracula, gruvbox, nord, rose-pine, tokyonight variants).
- Custom themes: TOML files in `~/.config/frond/themes/` (override with the
  `FROND_THEMES` environment variable). Format: `appearance` (required:
  `"light"` or `"dark"`) + `base` + `syntax` + `[ui]` + `[markdown]`. Unknown
  keys are reported as warnings, never silently ignored.
- `Shift+T` cycles the themes in the current group; `Ctrl+T` changes the group
  between all, light and dark. It does not change the theme on screen.
- Resolution order: `--theme` > `FROND_THEME` env var > config `theme` > default (`ocean`).
- Colour roles for every key are documented in [`docs/COLORS.md`](docs/COLORS.md).

## Development

```sh
make check      # fmt + clippy + tests + smoke suites
make test       # unit and integration tests only
make smoke      # pty smoke suites (runs with `uv run --with pyte`)
make clean      # remove build artifacts
```

The targets use the `cargo` on your `PATH`, falling back to
`~/.cargo/bin/cargo` (rustup's default) when `PATH` does not have it.
Override it with `make CARGO=/path/to/cargo <target>` or an exported
`CARGO`.

`SPEC.md` is the design reference; `CHANGELOG.md` lists releases.

## Acknowledgments

The markdown rendering engine is vendored from
[leaf](https://github.com/rivolink/leaf) (MIT); deviations are documented in
`crates/engine/VENDORED.md`.

## License

MIT — see [LICENSE](LICENSE).
