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

1. Startup: `vault::index` walks the vault (fast, no daemon), builds link/tag maps. The vault root is `--vault`, else (with no file argument and two or more `[vaults]` entries) the one chosen in the vault picker, else the config's `vault` key, else the only `[vaults]` entry, else (with a file argument) the current directory; with none of these, the app prompts for the vault directory. A `--vault` or `vault` value that is a `[vaults]` name resolves to that entry's path; any other value is a path (write `./work` to mean the directory when a name `work` exists).
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

## Frontmatter

A note's frontmatter is the leading `---` fenced block. The fences follow
the reader's own rules: a CRLF `---` fence counts, a UTF-8 BOM before the
first fence is ignored, and an **unterminated** fence means there is no
frontmatter at all — the whole file is body.

- `status:` maps to the sidebar's status glyph; `tags:` to the tag view.
- A tag list is a list: `tags: [a, b]` and the block form (`tags:` with
  `- a` lines) give the same tags, and a tag may contain a comma
  (`- "mocktag, weird"` is **one** tag).
- A `#` comment after a value is not part of it — in the inline form and
  in a block item alike.

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
- The note text sits inside a margin on all four sides: `margin` blank
  columns on each side and `margin_y` blank rows above and below. The
  visible text rows are the pane's height less twice the vertical margin,
  and the scroll range follows them. The text viewport ends `margin_y`
  rows above the status bar. A click in the padding does nothing. The
  vertical margin applies to the note only — a `.base` table fills its
  pane, as it ignores the horizontal margin too.
- The **link under the mouse is highlighted**: while the pointer is over a
  link in the note pane, that link is painted in the markdown theme's
  `link_hover` colour (the role every theme already sets). The highlight
  is only a colour: it does not move the link focus (`x` / `X`), and
  clicking the link still opens it. Mouse capture off (`m`) disables it,
  like every other mouse action.
- The note pane has an optional **line-number gutter**: with the config
  key `line_numbers` on (default off), a column to the left of the note's
  text shows the source line number of each line. The number appears on
  the first rendered line of a source line; a wrapped continuation line
  shows an empty gutter. The gutter's width follows the digits of the
  note's last line number, the note's text is that much narrower (and
  wraps that much earlier), and the gutter is painted in the markdown
  theme's `code_gutter` colour. The numbers are right-aligned in the
  gutter's columns, and an empty note still draws a one-column gutter
  (there is no line 0 to name). The gutter is a column of its own: the
  text's columns do not move inside the text area, so the links and the
  checkboxes stay where they are.
- Session memory (XDG state dir — `XDG_STATE_HOME/frond`, else
  `~/.local/state/frond`; `FROND_SESSION` overrides it; one file per
  vault, named after a hash of the vault root): the last open note
  with its scroll, plus the sidebar's mode (the tree or the tag view —
  the outline is never persisted, so quitting in the outline restores the
  view it came from), selection and folder expansion,
  are saved on clean quit and restored on the next start (an explicit note
  argument wins over the restore). `frond --config-show` prints a
  commented default config file; `--help`/`--version` print usage/version.
- **Vault picker** (startup): with no `--vault`, no file argument and two
  or more `[vaults]` entries, frond opens full-screen on a picker before
  any vault is indexed. It lists every entry as its name and its path
  (`~` for `$HOME`), in name order; typing filters it with the quick
  switcher's case-insensitive subsequence match over the name and the
  path, ranked best match first (ties in name order) with the best match
  selected; `↑`/`↓` move the selection; `Enter` opens the selected vault;
  `Esc` or `Ctrl+C` quits without a message, exit status 0. The
  preselected entry is the last vault opened (when it is still an entry),
  else the one the `vault` key names, else the first. An entry whose path
  is not a directory stays listed; choosing it shows `not a directory:
  <path>` inside the picker and keeps it open. The last vault opened is
  recorded on a clean quit (`last-vault` in the session dir, the vault's
  canonical root) — the app's own state, never the vault's.
- **Vault picker in the app** (`V`): the same picker opens as a modal
  popup over the main view, with the open vault preselected (when it is an
  entry; else as at startup). `Esc` closes it; `Ctrl+C` quits. The picker
  takes no mouse input. With no `[vaults]` entries, `V` opens nothing and
  the status line says `no vaults configured ([vaults] in config.toml)`.
  Choosing the open vault closes the popup and changes nothing. Choosing
  another one is a clean quit of the open vault (its session and
  `last-vault` saved exactly as on quit) followed by a start of the chosen
  one with no note argument: its index built, its session restored (else
  its `index.md`), and a fresh navigation stack, sidebar, search,
  backlinks, git status and watcher — none of the old vault's state,
  warnings or errors carry over. The run's display state carries over:
  the theme and its `Shift+T` group, mouse capture, watch, the sidebar's
  width and visibility, the metadata block, line numbers and margins. A
  vault that cannot be opened (its index not built, or the note it would
  start on not readable) is not opened: the popup stays open with `cannot
  open vault: <error>`, and the open vault is untouched, its session not
  saved. The
  entries are the ones read at startup. Started on a file argument with
  no vault, `V` works the same, with no session to save.

## Scrollbars

- A scrollable pane shows a one-column scrollbar on its right edge: the note
  pane, the sidebar tree, the tag list, the backlinks panel, the `.base`
  table and the help popup. It is drawn only while the content overflows the
  pane, and the text area is narrowed by that one column while it is drawn.
  A pane too short for twice the vertical margin has no text rows at all,
  and draws no bar.
  The `.base` table's bar sits inside its border, in the last inner column,
  and the table is rendered one column narrower while it is drawn.
- The bar is drawn as a thin `│` track with a solid `█` thumb and **no
  arrow heads**: the track spans the full height of the bar column, so at
  the top of the content the thumb's first row is the column's first row,
  and at the bottom its last row is the column's last row.
- The thumb is proportional to the visible fraction of the content (±1 row
  of rounding) and its ends track the scroll position: at the top it starts
  at the first track row, at the bottom it ends at the last track row. The
  scrollbar's content length is the scroll *range* — the total number of
  lines less the visible rows — not the number of lines.
- The thumb's length is computed against the **text viewport** (the rows of
  text the pane shows), not against the height of the bar column: the two
  are equal only when the pane has no vertical margin. With a vertical
  margin the bar column is taller than the text, and the thumb still
  shows the visible fraction of the content.
- The **note pane's bar is a position control**: a press on the bar column
  jumps the note, and a drag keeps the note following the pointer until
  the button is released. The mapping is linear over the track's rows: the
  track's first row is the document's top and its last row the document's
  bottom, so the pressed row names that fraction of the scroll range. The
  press does not move the keyboard focus, and the release that ends a drag
  is not a click (it must not open a link under the pointer). Mouse capture
  off (`m`) disables it, like every other mouse action.
- The **list panels' bar positions the selection**: the sidebar tree, the
  tag list, the outline and the backlinks panel. A press on the bar column
  moves the selection to the row at that fraction of the list (the track's
  first row is the list's first row, its last row the list's last row), and
  a drag keeps the selection following the pointer until the button is
  released. A press on the bar is **not** a row click: it never opens the
  row it lands on, and it does not move the keyboard focus. The release
  that ends a drag is not a click. Mouse capture off (`m`) disables it.
- The **`.base` table's bar is a position control**: a press on the bar
  column jumps the table to that row of its content, and a drag keeps
  the table following the pointer until the button is released. The
  mapping is linear over the track's rows, like the note pane's bar. The
  table's scroll is a document position, not a selection cursor. The
  press does not move the keyboard focus, and the release that ends a
  drag is not a click (it must not open a row under the pointer). Mouse
  capture off (`m`) disables it.
- The **help popup's bar is a position control**: a press on the bar
  column jumps the popup's body to that row, and a drag keeps the body
  following the pointer until the button is released. The mapping is
  linear over the track's rows. The popup's own keys (`j`/`k`, PgUp/PgDn,
  `g`/`G`) keep scrolling the body; the bar is the mouse way into the
  same scroll. The press does not move the keyboard focus, and the
  release that ends a drag is not a click (it must not close the popup).
  Mouse capture off (`m`) disables it.
- The **scrollbar's thumb highlights under the mouse**: while the pointer
  is over a bar column, that bar's thumb is painted in `scrollbar_hover_fg`
  (a new theme role; every theme's value is its accent). The track is
  unchanged, and the thumb does not move. Mouse capture off (`m`) disables
  the hover, like every other mouse action.

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
| PgUp / PgDn | Half page (content pane) |
| `g` / `G`, Home / End | Top / bottom (content pane) |
| Mouse wheel | Scroll the content pane; over the tree, the tag view, the backlinks panel, the search popup or the quick switcher it moves that pane's selection instead; over the outline it scrolls the panel's view |
| Press / drag on the note pane's scrollbar | Jump the note to that position; a drag follows the pointer until the button is released |
| Press / drag on a list panel's scrollbar | Move that panel's selection to that position; a drag follows the pointer until the button is released |
| Press / drag on the `.base` table's scrollbar | Jump the table to that row of its content; a drag follows the pointer until the button is released |
| Press / drag on the help popup's scrollbar | Jump the popup's body to that row; a drag follows the pointer until the button is released |
| Hover over a link (note pane) | Highlights the link in the theme's `link_hover` colour; the link focus does not move |
| `n` / `N` (note pane) | Move to the next / previous occurrence of the last search in the open note |
| Click (sidebar / content / backlinks / popup) | Moves keyboard focus to the clicked pane, then acts on the click; clicking a note opens it and moves focus to the content pane; a click on a row of the search or quick-switcher popup selects it (a second click on the selected row opens it), while the disambiguation popup takes no clicks at all; a click on a popup's chrome — its borders, the quick switcher's prompt, its footer — does nothing |
| `Tab` | Hide/show sidebar |
| `t` | Sidebar mode: tree ⇄ tags |
| `o` | Sidebar mode: the open note's headings (the outline); `o` again returns to the tree |
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
| `V` | Vault picker: switch to another `[vaults]` entry (see "Vault picker") |
| `u` | Sync status refresh (also auto on watch events) |
| `v` / `1`–`9` | Switch table view while a `.base` file is open |
| `w` | Toggle watch mode (content pane) |
| `Shift+T` | Cycle theme within the current group: presets, built-ins, then custom themes (session-only) |
| `Ctrl+T` | Theme group for `Shift+T`: all → light → dark → all. Changes the group only; the status line shows the group and its size (session-only) |
| `?` | Help popup: a centered single column with grouped sections (bold headers, keys aligned to one column) over a dimmed background, with a fixed title/footer; `j`/`k`/arrows, PgUp/PgDn, `g`/`G` scroll it; any other key closes it. The blank line between the groups is dropped when the body does not fit — then it scrolls behind a scrollbar |
| `q` / `Ctrl+C` | Quit (while the help popup is open, `Ctrl+C` closes the popup instead) |

The table is the key set of the **main view**. The modal popups have their
own keys, and they take them instead of the app:

- **Search (`s`)** — a query line over the vault's hits. In the query
  line, `Enter` runs the search, `Backspace` edits the query and `Esc`
  closes. In the results list, `j`/`k` (or the arrows) and PgUp/PgDn move
  the selection, `Enter` opens the selected hit's line, and any typed
  character returns to the query and refines it. `Esc` closes; `Ctrl+C`
  quits.
  The query is a **regular expression** when it starts with `re:`; the
  prefix is stripped and the rest is matched as a regular expression,
  case-insensitively, line by line. A query without the prefix is the
  case-insensitive substring search. An invalid regular expression is
  reported in the popup (the footer says what is wrong); nothing is
  searched and the popup stays in the query line with the query intact.
  The matches are highlighted in the results exactly as the substring
  matches are. An empty pattern (a bare `re:`) searches nothing: the
  popup shows its results list with no matches.
  The last executed search stays as the **active search**. In the note
  pane `n` moves to the next occurrence of the active search in the open
  note and `N` to the previous one, scrolling the occurrence into view;
  the search wraps around at both ends. The occurrences are the ones the
  search itself finds — in the note's source text, not in the rendered
  text — so a query finds the same places in the note as in the vault.
  With no active search, or with a note that has no occurrence, `n` and
  `N` do nothing.
- **Quick switcher (`Ctrl+P`)** — a query line over the vault's notes:
  `Enter` opens the selected note, `Backspace` edits the query, `j`/`k`
  (or the arrows) and PgUp/PgDn move the selection, any typed character
  goes into the query, and `Esc` closes (`Ctrl+C` quits). See "Quick
  switcher" below for the matching rules.
- **Disambiguation popup** (an ambiguous wikilink) — `j`/`k` or the
  arrows choose a candidate, `Enter` opens it, and any other key cancels
  the popup (`Ctrl+C` quits).

## Themes & configuration

- **Theme = `MarkdownTheme` + `UiTheme`** (chrome: sidebar, status bar,
  popups). `UiTheme`'s `[ui]` key list (app vocabulary, also what custom
  files accept): `sidebar_bg`, `sidebar_border_focused`,
  `sidebar_border_unfocused`, `selection_bg`, `changed_marker_fg`,
  `dimmed_fg`, `content_bg`, `status_bg`, `status_watch_on_fg`,
  `status_watch_off_fg`, `status_success_fg`, `status_dirty_fg`,
  `status_ahead_fg`, `status_behind_fg`, `status_error_fg`,
  `popup_border_fg`, `popup_dimmed_fg`, `popup_match_fg`,
  `popup_match_bg`, `scrollbar_hover_fg` — the sidebar (and backlinks
  panel) paint `sidebar_bg`, the status bar paints `status_bg`, popups
  paint `content_bg`, and a scrollbar's thumb paints `scrollbar_hover_fg`
  while the mouse is over it; see `docs/COLORS.md` for roles and sources.
  Four paired
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
- **The selected row** of every list panel (the sidebar tree, the outline,
  the tag view, the backlinks panel, and the search, quick-switch and
  disambiguation popups) is one full-width bar: its background is
  `selection_bg` across the panel's whole inner width (up to the scrollbar
  column when one is drawn), and its text is painted in the panel's own
  background colour (`sidebar_bg` for the sidebar family, `content_bg` for
  the popups). This is the app's inverse-video rule, the same one the link
  focus highlight uses (`crates/app/src/links.rs`): the selection background
  and the row's text colour are never the same colour, so the selected row
  stays legible in every theme. The selected row's own colours are dropped:
  the status glyph, the changed marker and the dimmed name keep their shape,
  not their colour. A search or quick-switch match highlight inside the
  selected row is kept as it is.
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
- **Config `~/.config/frond/config.toml`:** `theme`, `vault` (a path,
  or a `[vaults]` name), `vaults` (a table of `name = "path"`, `~`
  expanded: the vault picker's entries), `watch`
  (bool, default on: the *open note* re-renders when the file changes —
  the vault-wide scan behind the sidebar and the index always runs),
  `mouse` (bool, default on; off = native text selection),
  `sidebar_width` (columns, clamped 24–120), `metadata` (bool, default
  on; off = the note's frontmatter block does not render),
  `theme_rotation` (`all` | `light` | `dark`, default `all`: the group
  `Shift+T` starts in), `margin` (blank columns on each side of the note,
  0–10, default 2), `margin_y` (blank rows above and below the note, 0–5,
  default 1), `icons` (`nerd` | `unicode`, default `nerd`: the note
  and base icons on the status bar; `nerd` needs a Nerd Font in the terminal),
  `line_numbers` (bool, default off: the note pane shows a gutter with the
  note's source line numbers).
  Read at
  startup; `--vault`/`--theme`/`--watch`/`--mouse` override. Missing file =
  defaults; malformed file = warning + defaults (never a crash); unknown
  keys are ignored *with a warning* (never silently).
  `frond --config-show` prints a commented template of every key.
- **Live:** `Shift+T` cycles presets, built-ins, and discovered custom
  themes with an instant re-render, filtered by the current rotation group
  (session-only, never persists). The
  config file's mtime is polled on the 1 s watcher tick; a changed `theme`
  key live-applies. `vault` and `vaults` changes apply on the next launch; `watch`
  stays the `w`-key session toggle.
- **Unreadable directories:** a directory the vault scan cannot read is
  reported on the status bar — its notes are missing from the index, so
  the reader is told — and the report stays for as long as the condition
  lasts, clearing itself when the directory is readable again. It is a
  background fact, so it never replaces a newer error the user still
  needs (a refused write, a failed open), and it does not outlive the
  condition.
  The **startup** walk reports the same way: the warnings the index
  records while it builds (an unreadable file or directory) are surfaced
  in the same slot, with the paths relative to the vault root. A note
  that was skipped is a note the reader cannot find, so it is never
  silent — the same condition reported twice (once at startup, once by a
  later walk) is one message, not two.
  A file that becomes unreadable **later** is reported the same way: the
  reindex that fails to read it records the problem, and the report
  appears in the same slot, with the path relative to the vault root. It
  stays for as long as the file stays unreadable, and the note returns to
  the index with the report's end.
  The **open note** is re-read on every watch tick, so a file that turns
  unreadable **without changing** - a permission change moves neither its
  timestamp nor its size, so nothing else would notice - is reported too.
  The report clears when the file reads again. A `.base` table is not
  covered: the table's own file is watched by stamp alone, so a
  permission break on it goes unreported - and a `.base` frond was
  launched on records no unreadable-path warning at all, even though it
  is the one file probed by read.

## Status bar

`¶ index.md · 12 lines ·  42% · watch on · ◷ 09:30 · ✓`

(The example shows the **unicode** icon set; under the default
`icons = "nerd"` the note, table and clock glyphs are the Nerd Font ones.)

- Segments, in order: the name label, the size (`N lines`, or `N rows ·
  view X/Y: <name>` while a `.base` table is open), the scroll position, the
  watch indicator (`watch on` / `watch off`), the last render time, the
  `mouse off` notice (only while the mouse capture is off), the
  `metadata off` notice (only while the frontmatter block is hidden), the
  git segment and — last — an error or info message.
- Scroll position: the content pane's scroll **offset** as a percentage,
  immediately after the size segment (`12 lines ·  42%`). It reads `0%` at
  the top, `100%` at the bottom, and `100%` when the content fits in the
  pane; it is omitted when no document is open. It describes the content
  pane — the open note, or the `.base` table when one is open —
  **regardless of which pane holds the focus**. It is styled like the size
  segment next to it, not
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
  icon: `¶` for a note, `▦` for a `.base` table under the `unicode` icon
  set — the glyphs follow the `icons` setting, so the nerd set shows the
  Nerd Font's markdown and table glyphs instead); sidebar focus shows the
  focused row's name (or `#tag` in the tag view). The full path is never
  shown — a name shared by two notes is not disambiguated here (the
  sidebar and the wikilink popup still show folders).
- Clicking the note-name segment copies the vault-relative path via OSC 52
  (`copied: <rel>`); a failed write shows an error instead.
- Info messages (`theme: <name>`, `copied: …`) show in a neutral color for
  3 s, cleared by the poll tick. The message slot's order is: the real
  error (red), then the `.base` parse warnings, then the walk warning
  (red), and only then the info message — each of the first three hides
  the info message.
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
  start; the matched characters are highlighted in the result row. A row
  wider than the popup is cut at the last character that fits and ends
  in `…` (the vault picker's rows cut the same way).
- `↑`/`↓` (or `j`/`k`) move the selection, `PgUp`/`PgDn` move it by a
  page, `Enter` opens the selected note
  through the normal navigation stack (so `Backspace` returns to the
  previous note), `Esc` closes the popup. A click selects a row; a second
  click on the selected row opens it; the wheel over the popup moves the
  selection.
- The popup is modal: typed characters go into the query and never reach
  the app (a typed `q` is a query character, not a quit). `Backspace`
  edits the query. `Ctrl+C` still
  quits.

## Outline (`o`)

The sidebar can show the **open note's headings** instead of the vault
tree: `o` switches it on, `o` again returns to the tree. The tree and tag
views (`t`) are untouched, and `o` returns to whichever of the two was
last shown.

- The list is the open note's heading table (`viewer.toc`), in document
  order, each row indented by its heading level (the shallowest level in
  the note is flush left, each deeper level one step in).
- While a `.base` table is open in the content pane the outline is empty:
  a table is not a note, and the note hidden behind it would list headings
  that are stale and cannot be jumped to.
- `o` switches the mode like `t` does — including while the sidebar is
  hidden, where the mode changes and shows when the sidebar is revealed
  again.
- The outline follows the open note: it is the note currently in the
  content pane, and it is rebuilt when that note changes.
- `j`/`k` (and the arrows) move the selection; the panel scrolls the
  selection into view, like the tree.
- `Enter` scrolls the note to the selected heading and moves the focus to
  the content pane. A click on a row does the same.
- `l` moves the focus to the content pane; `h` does nothing (a heading
  list has no hierarchy to collapse).
- The panel says what it is: while the outline is shown, a
  `TABLE OF CONTENTS` label sits at the top of the panel, above the
  headings - a blank line, the label, another blank line, and the rule
  under that, so the label has a line of space on each side. The label
  takes its rows from the list, and the headings scroll under it. The
  label is **centred** in the panel. The panel's border title keeps
  naming the mode (` outline `), as the tree's and the tags' do.
- The tree and tag views carry the **vault's name** in the same header
  block - the vault directory's name, centred, over the same rule - so
  every mode says what it lists. The outline keeps its own
  `TABLE OF CONTENTS` label, because it lists the open note's headings
  and not the vault.
- The outline **follows the note**: the heading the reader is looking at
  is the **active** one, marked in the accent colour and in bold, and
  the panel scrolls it into view as the note scrolls. The active
  heading is the last one at or above the top of the note's viewport;
  at the end of the note it is the last heading, and above the first
  heading it is the first one. Where the active heading is also the
  selected one - right after a jump pins it - the selection's bar is the
  mark: the reader chose that row, and the bar says so.
- The active heading and the panel's **selection** are different things.
  The selection is the cursor the reader moves (`j`/`k`, a click) and
  the thing `Enter` jumps to; the active heading is the one the note's
  scroll moves. A note scroll **takes the selection with it** - the
  selection becomes the active heading - so the bar marks the heading
  the reader is looking at. The reader's own cursor stays where they
  put it until the next scroll. Jumping to a heading **pins** it, as
  the active one and as the selection, until the reader scrolls the note
  themselves; then the outline follows the scroll again and the
  selection goes with it.
- The wheel over the outline scrolls the panel's **view**, not its
  cursor: the reading position stays where the note put it, and the
  panel stops following the note until the reader scrolls the note
  themselves - leaf's rule. The keys and a click still move the
  cursor. The tree, the tag view and the backlinks panel keep the
  other rule: their wheel moves their selection, as their scrollbars
  do.
- The headings are **numbered**, the way leaf numbers them: the
  top-level headings (the shallowest level in the note, the ones that
  are flush left) carry a two-digit index - `01`, `02` - counted in
  document order, and the deeper headings carry a bullet. The title is
  truncated to leave room for the index or the bullet, and the row
  still fills the panel's width.
- A new note starts the outline at its top: both cursors reset to the
  first heading, and the pin is dropped. The selection's index means
  nothing across notes.
- With no note open, or a note with no headings, the panel shows the
  empty state `no headings`.
- While the sidebar is focused, the status bar shows the selected
  heading's title as the sidebar's label (the tree shows the focused
  row's name, the tag view `#tag`). With no heading selected the label
  reads `(no headings)`.

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
The frontmatter block is recognized with the reader's own fence rules: a
CRLF `---` fence counts, and the field is only bumped inside the block,
never in the body.
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
switcher, the session memory, and — in 0.6.2 — the scrollbar geometry and
symbols, and the scroll position in the status bar. In 0.6.5: the
selection bar (every list panel), the interactive scrollbars (the note
pane and the list panels) with the thumb's hover highlight, the link
hover, the regular-expression search with its match-next, the
line-number gutter, and the vault index's startup warnings.
