# frond — v1 Spec

Read-only Obsidian-style TUI for Markdown vaults. Humans read; agents write.
Rendering engine: leaf's markdown module, vendored as a library.

## Goals / Non-goals

**v1 goals**

- Read the vault comfortably: sidebar, wikilink navigation, tags, search, backlinks
- Live reload when agents edit notes (vault-wide watch)
- Git sync status in the status bar (read-only)

**Non-goals (v1)**

- ~~Any editing~~ — the one write feature (guarded checkbox toggling)
  was approved and built post-v1 (see "Guarded writes" below); all other
  editing (free text, renames) remains out of scope.
- Graph view, canvas, embeds `![[...]]`; `.base` card views, arbitrary
  formulas, and editing (the table-rendering subset approved in post-v1
  loop 6 is documented below)
- Multi-vault support

## Architecture

Rust workspace, three crates:

```
crates/
  engine/    leaf's src/markdown/ vendored, minimal changes:
             expose ParseResult (styled lines, TOC, link spans, source-line maps),
             add wikilink preprocessing hook
  vault/     vault model:
             - index: walk .md files -> path, frontmatter (status, tags, created,
               updated, archived), wikilinks out; backlink map derived
             - resolver: bare-filename -> path map, ambiguity detection
             - watcher: notify on vault dir -> dirty paths -> reindex + event bus
             - git: status --porcelain, rev-list ahead/behind, changed-files list
  app/       ratatui shell:
             - state: navigation stack, current note + scroll pos, sidebar mode,
               search state
             - layout, rendering, keybindings, mouse
```

### Data flow

1. Startup: `vault::index` walks the vault (≈480 notes — fast, no daemon), builds link/tag maps.
2. Note opened: raw md → wikilink preprocessing (`[[target|alias]]` → `[alias](vault://target)`) → `engine::parse` → styled lines.
3. Watcher event (file changed/deleted): reindex that file, if it's the visible note re-parse and redraw; refresh git status.
4. Wikilink activated: resolve via vault index → push current note on back stack → open target (scroll pos saved per note in the stack).

### Wikilink resolution

Per vault rule: bare filename first (`[[stem-analysis]]`), path fallback (`[[archive/index]]`).
Ambiguous basenames render with a warning marker and resolve on click via a small popup if needed.
Unresolved links render dimmed/strikethrough, non-clickable.

## Layout

```
┌──────────────┬─────────────────────────────────────────┐
│ sidebar      │  rendered note (leaf pipeline)          │
│ tree|tags    │                                         │
│              │                                         │
├──────────────┴─────────────────────────────────────────┤
│ status bar: path · status badge · tags · git ↓2 ↑1 · q │
└────────────────────────────────────────────────────────┘
```

- Sidebar (~30%, overridable with `<`/`>` or the config `sidebar_width`
  key, clamped 24–120 columns): mode toggled with `t` (tree ⇄ tags). Tree: collapsible folders, truncation
  with full path on focus. Notes show status glyphs from frontmatter `status:`
  (single mapping table in `crates/app/src/ui.rs`): `●` complete/resolved/active,
  `◐` in-progress/monitoring, `○` pending/planned, `◌` draft, `◉` blocked,
  `×` dimmed for archived/superseded; unknown statuses show no glyph.
  Files changed since last sync get a `*` marker.
- Optional backlinks panel toggled with `b` (lists "N notes link here", click/Enter opens).
- Session memory (XDG state dir, one file per vault): the last open note
  with its scroll, plus the sidebar's mode, selection and folder expansion,
  are saved on clean quit and restored on the next start (an explicit note
  argument wins over the restore). `frond --config-show` prints a
  commented default config file; `--help`/`--version` print usage/version.

## Keybindings (v1)

From leaf, the app inherited scroll keys, the mouse wheel, and the `?` help
popup (leaf's app-layer `/` search, TOC `t`, quick switcher `Ctrl+P`, and
history `Ctrl+H` were not vendored). The table below is the real key set —
verified against `crates/app/src/app.rs::handle_key` and the help popup in
`crates/app/src/ui.rs`:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Scroll (content); move selection (sidebar, backlinks) |
| PgUp / PgDn | Half page |
| `g` / `G`, Home / End | Top / bottom |
| Mouse wheel | Scroll the pane under the cursor / the focused pane |
| Click (sidebar / content / backlinks) | Moves keyboard focus to the clicked pane, then acts on the click; clicking a note opens it and moves focus to the content pane |
| `Tab` | Hide/show sidebar |
| `t` | Sidebar mode: tree ⇄ tags |
| `<` / `>` | Narrow / widen the sidebar (2-column steps, clamped) |
| `h` / `l` | Focus sidebar / content (`l` expands a folder, `h` collapses) |
| `x` / `X` | Focus next/previous link **or checkbox** (document order) |
| `m` | Toggle mouse capture (off = the terminal selects text natively) |
| `Enter` / click | Open focused link; Enter/click on a checkbox toggles it |
| `Backspace` / `Ctrl+O` | Back / forward in the navigation stack (scroll restored) |
| `b` | Toggle backlinks panel |
| `s` | Global vault search (Enter opens the hit line) |
| `u` | Sync status refresh (also auto on watch events) |
| `v` / `1`–`9` | Switch table view while a `.base` file is open |
| `w` | Toggle watch mode (content pane) |
| `Shift+T` | Cycle theme: presets, built-ins, then custom themes (session-only) |
| `?` | Help popup: a centered single column with grouped sections (bold headers, one blank line between groups, keys aligned to one column) over a dimmed background, with a fixed title/footer; `j`/`k`/arrows, PgUp/PgDn, `g`/`G` scroll it; any other key closes it |
| `q` / `Ctrl+C` | Quit |

## Themes & configuration

- **Theme = `MarkdownTheme` + `UiTheme`** (chrome: sidebar, status bar,
  popups). `UiTheme`'s `[ui]` key list (app vocabulary, also what custom
  files accept): `sidebar_bg`, `sidebar_border_focused`,
  `sidebar_border_unfocused`, `selection_bg`, `changed_marker_fg`,
  `dimmed_fg`, `content_bg`, `status_bg`, `status_watch_on_fg`,
  `status_watch_off_fg`, `status_success_fg`, `status_dirty_fg`,
  `status_ahead_fg`, `status_behind_fg`, `status_error_fg`,
  `popup_border_fg`, `popup_dimmed_fg`, `popup_match_fg`,
  `popup_match_bg` — the sidebar (and backlinks panel) paint
  `sidebar_bg`, the status bar paints `status_bg`, popups paint
  `content_bg`; see `docs/COLORS.md` for roles and sources. Four paired
  presets (arctic/forest/ocean/solarized) reuse leaf's
  preset values; **14 built-in themes** (catppuccin-latte/frappé/macchiato/
  mocha, dracula, dracula-alucard, gruvbox, nord, rose-pine,
  rose-pine-dawn, rose-pine-moon, tokyonight-day/night/storm) are compiled
  into the binary (ported from the user's leaf theme files, `[ui]` keys
  translated to the app vocabulary at port time). Custom theme files
  (TOML: `base` + `syntax` + `[ui]` + `[markdown]`) load with preset
  inheritance — `[markdown]` field names are the engine's own; `[ui]` uses
  the app's own vocabulary (`sidebar_*`, `status_*`, `popup_*`) ONLY —
  leaf's `[ui]` key names are not accepted in custom files (they warn as
  unknown). Unknown `[ui]`/`[diff]`/`[markdown]` keys are ignored with a
  listed warning (never silently). `[diff]` has no diff view.
- **Theme dirs:** `~/.config/frond/themes/` (frond-format TOML files only; the
  leaf compat directory is no longer read).
- **Resolution:** `--theme` flag > `FROND_THEME` env > config.toml `theme` key >
  default (`ocean`). A name resolves as a preset, then a built-in, then a
  discovered custom file. A custom file whose name matches a preset or
  built-in is never loaded — the reserved name wins and a warning says so
  (never silently). An unknown theme warns and falls back to the
  default; a custom theme's unknown `syntax` name falls back to the default
  syntect theme with a warning.
- **Config `~/.config/frond/config.toml`:** `theme`, `vault`, `watch`
  (bool), `mouse` (bool, default on; off = native text selection),
  `sidebar_width` (columns, clamped 24–120). Read at startup;
  `--vault`/`--theme`/`--watch`/`--mouse` override. Missing file =
  defaults; malformed file = warning + defaults (never a crash).
  `frond --config-show` prints a commented template of every key.
- **Live:** `Shift+T` cycles presets, built-ins, and discovered custom
  themes with an instant re-render (session-only, never persists). The
  config file's mtime is polled on the 1 s watcher tick; a changed `theme`
  key live-applies. `vault` path changes apply on the next launch; `watch`
  stays the `w`-key session toggle.

## Status bar

`~/notes/projects/…/note.md · ● complete · #aws #infrastructure · ↓2 ↑1 · 09:30`

- Git segment: `✓` clean / `●` dirty / `↓n ↑n` behind/ahead; refreshed on open, watch events, and `u`.
- Frontmatter `status:` is the badge source of truth (per vault rules).
- Path label: content focus shows the note's file name (`index.md`); when
  another indexed note has the same name, the vault-relative path. Sidebar
  focus shows the full path (or `#tag`).
- Clicking the note-name segment copies the vault-relative path via OSC 52
  (`copied: <rel>`); a failed write shows an error instead.
- Info messages (`theme: <name>`, `copied: …`) show in a neutral color for
  3 s, cleared by the poll tick. Errors (red) win over them.
- Keyboard hints (`Tab sidebar · ⇧T theme · ? help`) sit at the right edge
  only when they fit whole, and are hidden while a message is shown.

## Guarded writes (post-v1 loop 4)

The app's one write feature: clicking a rendered checkbox glyph (or
`Enter` on a focused one) cycles the marker `- [ ]` → `- [~]` → `- [x]` →
`- [ ]`. The write is guarded on every attempt; any failed guard is a
status-bar error, never a dialog and never a partial write:

1. on-disk mtime+size must match what was parsed (no racing external edit);
2. the repository that directly contains the note must be clean and the
   branch neither ahead nor behind (syncing is the user's job; an
   unreadable git status refuses too). The guard follows the note's own
   repo — `repo_toplevel` is resolved from the note's directory, so a
   note inside a nested repository is guarded by that nested repo, not
   the vault root's. A note not inside any repository skips the git
   guard but keeps the mtime guard;
3. the write itself re-scans the exact source line and refuses on a
   marker-state mismatch.

The write is byte-preserving outside two edits: the marker state byte on
that one source line (located by re-scanning the line, immune to
wikilink-preprocessing offset shifts) and the frontmatter `updated:`
value (bumped to today, only when the field already exists — an absent
field is never added; the key, post-colon whitespace, any trailing
comment, and the line terminator including a CRLF's `\r` are preserved).
Replacement is atomic: the original's permissions are read together with
the content (before the temp file exists — unreadable metadata refuses
the write), the temp file is fsynced before the rename, and the existing
watch/reload + reindex pipeline picks the change up on the next poll.

## `.base` table views (post-v1 loop 6)

Opening a `.base` file (CLI argument) renders its first `type: table`
view instead of the markdown viewer: a bordered block, a header row
built from the view's `order` (with `properties:` displayName mappings),
and one row per matching note. `v` cycles views, `1`–`9` jump; `j/k`,
PgUp/PgDn, `g/G`, and the wheel scroll the table. `sort` and `groupBy`
apply (group headers render as bold `▸ label` rows); rows show the
shared status glyph in the `status` column; `file.name` and
`file.asLink(title)` cells are clickable wikilinks through the normal
navigation stack.

Supported filter subset (`crates/vault/src/base.rs`, hand-rolled YAML
parser — the crate stays dependency-free): `and`/`or` combinators
(nestable) of `file.inFolder(path)`, `file.hasTag(tag)`, property
equality `prop == value` (quoted strings and bare booleans), inequality
`prop != value` — an **approved extension beyond Obsidian's documented
Bases surface** (approved by the orchestrator; needed by the real
GPU-matrix base, where failing open without it would wrongly include
confirmed root-causes; recorded in `crates/engine/VENDORED.md`), and one date comparison `prop >= today() - "Nd"` over `YYYY-MM-DD` values.
**Fail-open contract:** unknown filter expressions match everything
(with a status-bar warning), unsupported formulas render an em-dash,
unknown view types are dropped, and a wholly unparsable file shows one
default all-notes view — the visible set can only ever be wider than
the author intended, never silently narrower. The same contract covers
malformed quoting in `==`/`!=` values (an unterminated or stray quote
fails open with a warning instead of parsing to a literal that would
silently match nothing), over-deep nesting (> 32 levels fails open),
and tabs in indentation (rejected; tabs inside values are tolerated).

`.base` files are NOT index entries (the index walks `.md` only): the
open base file is re-read when its mtime+size stamp changes, and the
table re-derives from the live index on every vault reindex, so note
edits move rows between views through the normal watch pipeline.

## Milestones

1. **M1 — Engine**: vendor leaf markdown crate; render a vault index note read-only with watch.
2. **M2 — Shell**: sidebar tree, status bar, navigation stack + back.
3. **M3 — Vault**: index (frontmatter/tags/links), wikilink resolution + navigation, backlinks, tag view.
4. **M4 — Polish**: global search, git sync status + changed-file markers, unresolved-link handling.
