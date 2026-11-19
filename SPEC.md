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
  vault/     vault model (dependency-free):
             - index: walk .md files -> path, frontmatter (status, tags, created,
               updated, archived), wikilinks out; backlink map derived
             - resolver: bare-filename -> path map, ambiguity detection
             - base: the .base YAML subset parser + the table model
             - watcher: poll the vault dir -> dirty paths -> reindex
             - git: status --porcelain, rev-list ahead/behind, changed-files list
  app/       ratatui shell:
             - state: navigation stack, current note + scroll pos, sidebar mode,
               search state, quick-switcher state
             - layout, rendering, keybindings, mouse
```

### Data flow

1. Startup: `vault::index` walks the vault (fast, no daemon), builds link/tag maps. The vault root is `--vault`, else the config's `vault` key, else (with a file argument) the current directory; with neither, the app prompts for the vault directory.
2. Note opened: raw md → wikilink preprocessing (`[[target|alias]]` → `[alias](vault://<n>/<target>)`, the target percent-encoded, an optional `#heading` appended) → `engine::parse` → styled lines.
3. Watch tick (every 250 ms): the open file's mtime+size stamp is polled
   (re-parse + redraw when it changed); once a second the whole vault is
   re-walked and the changed notes are reindexed (the sidebar tree, the
   tags and the backlinks are refreshed). The git status is refreshed too
   (debounced, on a worker thread).
4. Wikilink activated: resolve via vault index → push current note on back stack → open target (scroll pos saved per note in the stack).

### Wikilink resolution

Per vault rule: a target without a `/` matches the bare file name
(`[[stem-analysis]]`, case-insensitive, `.md` optional); a target with a
`/` matches the vault-relative path (`[[archive/index]]`, with the
suffix optional on either side), falling back to a path *suffix* match
(`[[index]]` → `archive/index.md`). A `.base` target resolves against the
vault's base tables only.
An ambiguous basename is resolved *at parse time* by proximity: the
candidate in the nearest ancestor folder of the linking note wins (the
note's own folder first). Two candidates equally near are never guessed
between — only then does the link open the
disambiguation popup; the ambiguous link is painted in the theme's
warning colour (`status_dirty_fg`) — there is no marker glyph.
An unresolved link keeps the link's own colour, struck through and
dimmed; it can still be focused with `x`/`X`, but activating it is an
error (`unresolved link: <target>`), never a guess.

## Layout

```
┌──────────────┬─────────────────────────────────────────┐
│ sidebar      │  rendered note (leaf pipeline)          │
│ tree|tags    │                                         │
│              │                                         │
├──────────────┴─────────────────────────────────────────┤
│ status bar: name · size · watch · rendered · git ↓2 ↑1  │
└─────────────────────────────────────────────────────────┘
```

- Sidebar (~30% of the terminal, clamped 24–60 columns; overridable with
  `<`/`>` or the config `sidebar_width` key, clamped 24–120): mode toggled
  with `t` (tree ⇄ tags). Tree: collapsible folders, truncation
  with the focused name on the status bar. Notes show status glyphs from frontmatter `status:`
  (single mapping table in `crates/app/src/ui.rs`): `●` complete/resolved/active,
  `◐` in-progress/monitoring, `○` pending/planned, `◌` draft, `◉` blocked,
  `×` dimmed for archived/superseded; unknown statuses show no glyph.
  Files whose git status marks them as changed in the working tree get a
  `*` marker.
  `.base` tables are tree rows too (alphabetically among the notes) and
  open the table view.
- Optional backlinks panel toggled with `b` (the panel title is
  `backlinks (N)`; `no notes link here` is its empty state; click/Enter
  opens a listed note). Opening it moves the focus to the panel, closing
  it returns the focus to the content pane.
- Session memory (XDG state dir — `XDG_STATE_HOME/frond`, else
  `~/.local/state/frond`; `FROND_SESSION` overrides it; one file per
  vault, named after a hash of the vault root): the last open note
  with its scroll, plus the sidebar's mode, selection and folder expansion,
  are saved on clean quit and restored on the next start (an explicit note
  argument wins over the restore). `frond --config-show` prints a
  commented default config file; `--help`/`--version` print usage/version.

## Scrollbars

- A scrollable pane shows a one-column scrollbar on its right edge: the note
  pane, the sidebar tree, the tag list, the backlinks panel and the help
  popup. It is drawn only while the content overflows the pane, and the text
  area is narrowed by that one column while it is drawn.
- The thumb is proportional to the visible fraction of the content (±1 row
  of rounding) and its ends track the scroll position: at the top it starts
  at the first track row, at the bottom it ends at the last track row. The
  scrollbar's content length is the scroll *range* — the total number of
  lines less the visible rows — not the number of lines.

## Keybindings (v1)

From leaf, the app inherited scroll keys, the mouse wheel, and the `?` help
popup (leaf's app-layer `/` search, TOC `t` and history `Ctrl+H` were not
vendored; the quick switcher `Ctrl+P` was re-implemented from scratch in
frond — see "Quick switcher" below). The table below is the real key set —
verified against `crates/app/src/app.rs::handle_key` and the help popup in
`crates/app/src/ui.rs`:

| Key | Action |
|---|---|
| `j` / `k`, arrows | Scroll (content); move selection (sidebar, backlinks) |
| PgUp / PgDn | Half page |
| `g` / `G`, Home / End | Top / bottom |
| Mouse wheel | Scroll the content pane; over the sidebar, the backlinks panel or a popup it moves that pane's selection instead |
| Click (sidebar / content / backlinks / popup) | Moves keyboard focus to the clicked pane, then acts on the click; clicking a note opens it and moves focus to the content pane; a click on a popup row selects it (a second click on the selected row opens it) |
| `Tab` | Hide/show sidebar |
| `t` | Sidebar mode: tree ⇄ tags |
| `<` / `>` | Narrow / widen the sidebar (2-column steps, clamped) |
| `h` / `l` (or ← / →) | Focus sidebar / content (`l` expands a folder, `l` on a note row moves the focus to the content; `h` collapses a folder, `h` on a note row collapses to its parent); from the backlinks panel `h` returns to the content |
| `x` / `X` | Focus next/previous link **or checkbox** (document order) |
| `f` | Show / hide the note's frontmatter metadata block |
| `m` | Toggle mouse capture (off = the terminal selects text natively) |
| `Enter` / click | Open focused link; Enter/click on a checkbox toggles it |
| `Backspace` / `Ctrl+O` | Back / forward in the navigation stack (scroll restored) |
| `b` | Toggle backlinks panel (opening moves the focus into it) |
| `s` | Global vault search (Enter opens the hit line) |
| `Ctrl+P` | Quick switcher: fuzzy jump to a note by name (Enter opens it) |
| `u` | Sync status refresh (also auto on watch events) |
| `v` / `1`–`9` | Switch table view while a `.base` file is open |
| `w` | Toggle watch mode (content pane) |
| `Shift+T` | Cycle theme within the current group: presets, built-ins, then custom themes (session-only) |
| `Ctrl+T` | Theme group for `Shift+T`: all → light → dark → all. Changes the group only; the status line shows the group and its size (session-only) |
| `?` | Help popup: a centered single column with grouped sections (bold headers, keys aligned to one column) over a dimmed background, with a fixed title/footer; `j`/`k`/arrows, PgUp/PgDn, `g`/`G` scroll it; any other key closes it. The blank line between the groups is dropped when the body does not fit — then it scrolls behind a scrollbar |
| `q` / `Ctrl+C` | Quit (while the help popup is open, `Ctrl+C` closes the popup instead) |

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
  (TOML: `base` + `appearance` + `syntax` + `[ui]` + `[markdown]`) load
  with preset inheritance — `[markdown]` field names are the engine's own;
  `[ui]` uses
  the app's own vocabulary (`sidebar_*`, `status_*`, `popup_*`) ONLY —
  leaf's `[ui]` key names are not accepted in custom files (they warn as
  unknown). Unknown `[ui]`/`[diff]`/`[markdown]` keys are ignored with a
  listed warning (never silently). `[diff]` has no diff view. An unknown
  `base` preset is a hard load error (the file is skipped and named), and
  so is a missing or invalid `appearance`.
- **Theme dirs:** `~/.config/frond/themes/` (frond-format TOML files only; the
  leaf compat directory is no longer read). `FROND_THEMES` overrides the
  directory (tests and smokes only).
- **Appearance:** every theme is light or dark. Presets and built-ins declare
  it in code; a custom file must set `appearance = "light"` or `"dark"`, and
  a file without a valid value is skipped with a load error. The rotation
  groups (`Shift+T` / `Ctrl+T`) filter on it. Details in `docs/COLORS.md`.
- **Resolution:** `--theme` flag > `FROND_THEME` env > config.toml `theme` key >
  default (`ocean`). A name resolves as a preset, then a built-in, then a
  discovered custom file. A custom file whose name matches a preset or
  built-in is never loaded — the reserved name wins and a warning says so
  (never silently). An unknown theme warns and falls back to the
  default; a custom theme's unknown `syntax` name falls back to the default
  syntect theme with a warning.
- **Config `~/.config/frond/config.toml`:** `theme`, `vault`, `watch`
  (bool, default on: the *open note* re-renders when the file changes —
  the vault-wide scan behind the sidebar and the index always runs),
  `mouse` (bool, default on; off = native text selection),
  `sidebar_width` (columns, clamped 24–120), `metadata` (bool, default
  on; off = the note's frontmatter block does not render),
  `theme_rotation` (`all` | `light` | `dark`, default `all`: the group
  `Shift+T` starts in), `margin` (blank columns on each side of the note,
  0–10, default 2), `icons` (`nerd` | `unicode`, default `nerd`: the note
  and base icons on the status bar; `nerd` needs a Nerd Font in the terminal).
  Read at
  startup; `--vault`/`--theme`/`--watch`/`--mouse` override. Missing file =
  defaults; malformed file = warning + defaults (never a crash); unknown
  keys are ignored *with a warning* (never silently).
  `frond --config-show` prints a commented template of every key.
- **Live:** `Shift+T` cycles presets, built-ins, and discovered custom
  themes with an instant re-render, filtered by the current rotation group
  (session-only, never persists). The
  config file's mtime is polled on the 1 s watcher tick; a changed `theme`
  key live-applies. `vault` path changes apply on the next launch; `watch`
  stays the `w`-key session toggle.

## Status bar

`¶ index.md · 12 lines ·  42% · watch on · ◷ 09:30 · ✓`

- Segments, in order: the name label, the size (`N lines`, or `N rows ·
  view X/Y: <name>` while a `.base` table is open), the scroll position, the
  watch indicator (`watch on` / `watch off`), the last render time, the git
  segment and — last — an error or info message.
- Scroll position: the content pane's scroll **offset** as a percentage,
  immediately after the size segment (`12 lines ·  42%`). It reads `0%` at
  the top, `100%` at the bottom, and `100%` when the content fits in the
  pane; it is omitted when no document is open. It describes the content
  pane — the open note, or the `.base` table when one is open — whichever
  pane has the focus. It is styled like the size segment next to it, not
  dimmed, and it does not change the hints' rule: they stay flush right and
  are hidden only while a message is shown or when they do not fit. The
  segment shares the left run's columns, so on a narrow terminal the hints
  can stop fitting a few columns earlier than before.
- The scroll position occupies a **fixed four-column field**, right-aligned
  (`  0%`, ` 50%`, `100%`). The width of the segments after it therefore
  never changes while scrolling — the bar must not shift under the pointer
  of a scrolling user.
- Last render time: a clock glyph followed by `HH:MM` (`◷ 09:30`). The
  glyph follows the `icons` setting — nerd: `nf-fa-clock_o` (U+F017),
  unicode: `◷` (U+25F7) — like the note and table icons do. Before the
  first render the segment shows the glyph alone.
- Git segment: `✓` clean / `●` dirty / `↓n ↑n` behind/ahead; refreshed on open, watch events, and `u`.
- The frontmatter `status:` badge is **not** on the status bar: the status
  glyph (`●`, `◐`, …) sits next to the note in the sidebar tree, and the
  tags are listed in the sidebar's tag view.
- Path label: content focus shows the open note's **file name** (with its
  icon: `¶` for a note, `▦` for a `.base` table); sidebar focus shows the
  focused row's name (or `#tag` in the tag view). The full path is never
  shown — a name shared by two notes is not disambiguated here (the
  sidebar and the wikilink popup still show folders).
- Clicking the note-name segment copies the vault-relative path via OSC 52
  (`copied: <rel>`); a failed write shows an error instead.
- Info messages (`theme: <name>`, `copied: …`) show in a neutral color for
  3 s, cleared by the poll tick. Errors (red) win over them.
- Keyboard hints (`Tab sidebar · ⇧T theme · ? help`) sit at the right edge
  only when they fit whole, and are hidden while a message is shown.

## Quick switcher (`Ctrl+P`)

A modal popup that fuzzy-matches the typed query against the vault's
notes and opens the selected one. It is the "jump to a note by name"
counterpart to the global search (`s`): the search greps note *content*,
the switcher matches note *paths*.

- `Ctrl+P` opens the popup over every indexed note (an empty query lists
  them all, ranked alphabetically); typing filters the list live — no
  Enter needed, the matches update on every keystroke.
- The match is a case-insensitive **subsequence** match (the query's
  characters appear in the path in order, not necessarily adjacent), the
  way every other fuzzy finder works. Two haystacks are tried per note:
  the whole vault-relative path and the bare file name; a match inside the
  file name wins, so typing `alpha` surfaces `alpha.md` before
  `archive/alpha-notes.md`.
- The scoring rewards contiguous matches, word boundaries (path
  separators, dashes, camelCase humps), a substring match and an early
  start; the matched characters are highlighted in the result row.
- `↑`/`↓` (or `j`/`k`) move the selection, `Enter` opens the selected note
  through the normal navigation stack (so `Backspace` returns to the
  previous note), `Esc` closes the popup. A click selects a row; a second
  click on the selected row opens it; the wheel over the popup moves the
  selection.
- The popup is modal: typed characters go into the query and never reach
  the app (a typed `q` is a query character, not a quit). `Ctrl+C` still
  quits.

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

Opening a `.base` file (as a CLI argument, or from its row in the sidebar
tree) renders its first `type: table`
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

All four are shipped. The work after them (each documented in its own
section above): the guarded checkbox write, the `.base` table views, the
theme groups, the note margin, the status-bar icon sets, the quick
switcher, and the session memory.
