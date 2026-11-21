# Changelog

Format follows [Keep a Changelog](https://keepachangelog.com/) loosely; dates are UTC.

## [Unreleased]

### Added
- The outline follows the note: the heading the reader is looking at is
  marked in the accent colour, and the panel scrolls it into view as the
  note scrolls. The reader's own cursor is a separate thing - it is what
  `Enter` jumps to, and a scroll never moves it. Jumping to a heading
  keeps it marked until the reader scrolls the note themselves.
- The outline panel says what it is: while the outline is shown, a
  `TABLE OF CONTENTS` label sits at the top of the panel, above the
  headings, with a rule under it. The headings scroll under the label,
  which takes its rows from the list.
- The `.base` table's scrollbar is a position control: a press on the bar
  jumps the table to that row of its content, and a drag keeps the table
  following the pointer until the button is released. Until now the
  table's bar and the help popup's were display only.
- The help popup's scrollbar is a position control too: a press on the bar
  jumps the popup's body to that row, and a drag keeps the body following
  the pointer until the button is released. The popup's own keys keep
  scrolling the body; the bar is the mouse way into the same scroll.

### Fixed
- A file that becomes unreadable while frond is running is now reported on
  the status bar, the way an unreadable file found at startup already was.
  The reindex that failed to read it recorded the problem, but nothing
  surfaced it, so the note was missing from the index with no word about
  it. The report stays until the file reads again, and the note returns to
  the index with it.
- The status bar's warning for a directory the vault cannot read no longer
  disappears when the vault walk itself fails. A walk that could not look
  is not evidence that every directory reads, so the warning stays until a
  walk can check again; before, a failed walk cleared it and the notes
  inside the unreadable directory were missing with no word about it.

### Changed

## [0.6.5] — 2026-11-20

### Added
- The note pane's scrollbar is a position control: a press on the bar jumps
  the note to that point in the document, and a drag keeps the note
  following the pointer until the button is released. Until now a click on
  the bar did nothing.
- The list panels' scrollbars (the sidebar tree, the tag list, the outline
  and the backlinks panel) position that panel's selection: a press on the
  bar moves the selection to that row of the list, and a drag keeps the
  selection following the pointer. A press on the bar is not a row click
  any more - it never opens the row it lands on.
- A scrollbar's thumb highlights in the theme's accent while the mouse is
  over it, so the bar says it is draggable. The colour is a new theme
  role, `scrollbar_hover_fg`: a custom theme file may set it, and a file
  that does not takes the accent - the one it inherits, or the one the
  file itself sets.
- The link under the mouse is highlighted in the theme's `link_hover`
  colour, so you can see which link you are about to open. The link focus
  does not move, and clicking the link still opens it.
- The global search popup takes a regular expression: a query that starts
  with `re:` is matched as one (case-insensitively, line by line), so a
  query can match a shape and not only a literal string. An invalid
  pattern is reported in the popup and searches nothing.
- The last search stays active: `n` and `N` in the note pane move to the
  next and the previous occurrence of it in the open note, wrapping at both
  ends, so you can walk through the hits of a note you just opened.
- The note pane can show a line-number gutter: with the new config key
  `line_numbers` on, a column left of the text shows the note's source
  line numbers (a wrapped line's continuation shows an empty cell).
- The warnings the vault index records while it walks the vault at
  startup (an unreadable file or directory) now reach the status bar.
  They used to be silent, so a note that was skipped was a note you could
  not find and had no way of knowing about. The messages are vault-relative
  and deduplicated, and a file that reads again is reindexed before its
  warning goes, so search, tags and backlinks find it again.

### Fixed

### Changed
- The selected row of every list panel (the sidebar tree, the outline, the
  tag view, the backlinks panel, and the search, quick-switch and
  disambiguation popups) is now one full-width bar: it keeps the theme
  accent as its background, and its text is painted in the panel's own
  background colour, so the selected row is legible in every theme (the
  accent and the text were the same colour, which made the row hard to
  read).
- The documentation gained what the audit showed was missing: a
  "Frontmatter" section in `SPEC.md` (the fence rules, the tag list, and
  the comment handling), a sentence on which popup rows are chrome and
  therefore not clickable, the status bar's message precedence, and
  `README.md`'s feature list now naming the outline (`o`) and the status
  bar's scroll position.

## [0.6.4] — 2026-11-19

### Added

### Fixed
- A dirty vault could report clean: a changed file whose name starts with
  `~` was dropped from the git status, so the guarded write's "the
  repository must be clean" check passed on a dirty tree.
- Toggling a checkbox in a large note froze the app: the `updated:` field
  scan indexed every line of the file, rescanning from the start for each
  one — 96 seconds for a 100,000-line note. It is now one pass that stops
  at the frontmatter.
- Clicking the search popup's footer selected a hit that was not on
  screen (when the results overflowed the popup), and a later `Enter`
  opened that never-seen row.
- A successful checkbox write was reported in red like a refusal, and it
  never expired.
- The quick switcher dropped its footer when there were no matches, so the
  only `Enter`/`Esc` hint disappeared exactly when it was needed.
- A note named `X.MD` could not be linked with `[[X.MD]]` or
  `[[dir/X]]`, although the matching is case-insensitive everywhere else.
- A frontmatter tag containing a comma was split into two tags, and
  `file.hasTag("a, b")` could never match it.
- A directory the vault scan could not read made its notes vanish from
  the index without a warning; the watch path now reports it on the status
  bar (at startup the index still records it silently).
- A note opened with a UTF-8 BOM — or with a repeated `\r` at the fence —
  never had its `updated:` field bumped.
- The watch path reported a directory it could not read through the
  sticky error slot: the message never cleared, and it replaced newer
  errors the user still needed. It has its own slot now: it clears when
  the condition ends, and it never replaces a real error.
- The quick switcher matched an `X.MD` note against its kept `.MD`
  suffix, so the typed highlight landed on the extension.

### Changed
- The popups' list geometry lives in one helper, so the drawn window and
  the clickable window cannot drift apart — the disambiguation popup's
  clicks are affected too: its height reserved a blank row its list
  window rejected.
- The documentation now matches the code: the status bar's segment list
  names the `mouse off` and `metadata off` notices, the keybindings
  section lists the modal popups' own keys and marks the content-pane
  keys, the icon examples say they show the unicode set, README's
  "index.md is shown first" is qualified by the session restore, and
  `docs/COLORS.md` records that every scrollbar draws in
  `popup_dimmed_fg`.

## [0.6.3] — 2026-11-19

### Added
- **Vertical note margin (`margin_y`)**: the note's text now sits inside a
  margin on all four sides — `margin` blank columns on each side and
  `margin_y` blank rows above and below (0–5, default 1). The scroll
  range and the scrollbar's thumb follow the shorter text viewport.
- **Outline (`o`)**: the sidebar can show the open note's headings instead
  of the vault tree. `o` switches it on, `o` again returns to the tree.
  The rows are indented by heading level and follow the open note; `j`/`k`
  move the selection and `Enter` (or a click) scrolls the note to the
  selected heading.
- The `.base` table shows a scrollbar while it overflows, like the other
  panes: the bar sits in the table's last inner column, and the table is
  rendered one column narrower while the bar is drawn.

### Fixed

### Changed

## [0.6.2] — 2026-11-19

### Added
- **Scroll position in the status bar**: the content pane's scroll offset as
  a percentage, right after the size segment (`¶ index.md · 12 lines · 42% ·
  watch on · …`). It reads `0%` at the top, `100%` at the bottom and `100%`
  when the content fits; a `.base` table shows its own offset.
- **Quick switcher (`Ctrl+P`)**: a modal popup that fuzzy-matches the typed
  query against the vault's notes and opens the selected one. The query
  filters live (no Enter needed); the match is a case-insensitive
  subsequence match over the note's path and file name (a file-name match
  wins, so `alpha` surfaces `alpha.md` before `archive/alpha-notes.md`).
  `↑`/`↓` move the selection, `Enter` opens the note through the normal
  navigation stack, `Esc` closes.

### Fixed
- The scrollbar thumb is proportional to the visible part of the content and
  reaches the end of the track: the note pane, the sidebar tree, the tag list,
  the backlinks panel and the help popup were fed the number of lines where
  the scroll range belongs, so the thumb was too short and stopped short of
  the bottom.
- The mouse-wheel smoke check compares the whole body text instead of a single
  row, so a scroll is no longer missed when the sampled row happens to be a
  blank gap.

### Changed
- The status bar's scroll percentage sits in a fixed four-column field
  (`  0%`, ` 50%`, `100%`), so the segments after it no longer slide
  sideways as the number grows while you scroll.
- The render time is a clock glyph plus `HH:MM` — the glyph follows the
  `icons` setting — instead of the word "rendered" and the seconds.
- The scrollbar draws a thin `│` track and a solid `█` thumb, without the
  arrow heads: the track now spans the full height of the bar column.
- The Makefile finds `cargo` on the `PATH` and falls back to
  `~/.cargo/bin/cargo` (rustup's default), so every target — `make run`
  included — works in a shell whose `PATH` lacks it. Override with
  `make CARGO=... <target>`.
- `SPEC.md` reconciled with the code: the status bar's segments (the status
  badge and the tags are *not* on it), the path label rules (the file name,
  never the full path), the vault watcher (poll-based, not notify-based),
  the wikilink resolution and ambiguity rules, the sidebar width clamps, the
  focus rules of `h`/`l`/`b`, the help popup's layout, the `watch` config
  key and the milestone list. The README's "no vault" wording was corrected
  too (without `--vault` there is no sidebar, but the current directory is
  still indexed).

## [0.6.1] — 2026-11-17

### Added
- **Theme groups**: `Ctrl+T` cycles the `Shift+T` rotation through all themes,
  light themes and dark themes (session only). The status line shows the
  group and how many themes it has. Config `theme_rotation = "all" | "light" |
  "dark"` sets the startup group. `Shift+T` still cycles as before.
- **Theme appearance**: every theme is light or dark. Presets and built-ins
  declare it; custom theme files must set `appearance = "light" | "dark"`
  (see `docs/COLORS.md`). A custom file without it is skipped with a load
  error.
- **Note margin**: config `margin` sets the blank columns on each side of the
  note text (0–10, default 2).
- **Status-bar icons**: config `icons = "nerd" | "unicode"` (default `nerd`).
  The note name on the status bar has an icon: the markdown glyph (U+E73E) for
  `.md` notes in nerd mode and `¶` in unicode mode; `.base` tables show the
  material table glyph (U+F04EB) in nerd mode and `▦` in unicode mode. `nerd`
  needs a Nerd Font in the terminal.
- CI workflow added (`.github/workflows/ci.yml`).

### Changed
- Git status refresh runs on a worker thread (no UI stall on large repos), and
  git is invoked with `--no-optional-locks` so a refresh can never block your
  own concurrent `git commit` on `index.lock`. The checkbox write guard stays
  synchronous.
- Theme palette fields are listed once (`with_markdown_fields!`) and generate
  both the config overrides and the built-in theme keys; a test catches an
  engine field missing from the list.
- Performance: only the visible slice of a note is cloned per frame; a watch
  tick reindexes all changed notes with one derived-state rebuild; syntect
  assets load once per process (theme cycling is instant); global search
  scans all-ASCII notes with one substring search (~5x faster on a 3,000-note
  vault, for ~10% more memory); path-style link resolution is a map lookup; base column
  widths are cached; sidebar scroll bookkeeping walks the tree once.
- Package renamed `app` → `frond` (`cargo run -p frond`); navigation history is
  capped at 500 entries; one shared vault listing implementation for the
  sidebar and the indexer; smoke fixtures live in a private per-user scratch
  dir.
- The note text is inset by the margin on both sides. It was flush left with 2
  columns only on the right.
- `Ctrl+T` changes only the `Shift+T` group. It no longer moves the theme on
  screen; `Shift+T` does that.
- The status bar shows only the name of the note or the focused row, never the
  full path. A name shared by another note is not disambiguated there (the
  sidebar and the wikilink popup still show folders).
- Notes no longer end with 5 blank rows. Scrolled to the end, the last line sits
  on the bottom row.
- Custom theme files written before this release need an `appearance` line.
  The bundled themes already have one.

### Fixed
- A wikilink with a non-ASCII target (`[[ééé]]`) no longer panics the index
  build (and with it the whole app at startup).
- Link, checkbox and status-glyph states no longer use fixed ANSI colours:
  ambiguous links and status glyphs follow the active theme's roles, unresolved
  links are struck through and dimmed in their own colour, and the focused
  link/checkbox is inverse video (legible on every theme, light ones included;
  a contrast test covers all built-in themes).
- Atomic-write temp files (task toggle, session save) are pid/counter-named so
  concurrent instances cannot clobber each other.
- The margin columns next to the note are painted with the theme background.
  They used to show the terminal background, which is black on many setups.
- Scrolling to the bottom of a note that needs a scrollbar could stop short by
  as many rows as paragraphs re-wrapped at the narrower width. The scroll was
  clamped against the wider wrap before the scrollbar was known.

## [0.6.0] — 2026-11-16

### Added
- **`.base` files in the sidebar tree**: `.base` tables now appear as tree
  rows next to the `.md` notes (sorted alphabetically together with them),
  open the table view on click/Enter/`l` like a note, and `reveal` (link
  follow, search hit, backlink, session restore) selects their row. A
  `.base` file previously had no tree row and no in-app entry point at
  all — the CLI file argument was the only way to open one.
- **`.base` wikilinks resolve**: a target explicitly ending in `.base`
  (`[[inventories.base]]`, bare or path-style, case-insensitive) resolves
  against the vault's base files and opens the table view. The index
  tracks `.base` files path-only through the vault walk (their content is
  never parsed), so created/deleted base files appear and disappear via
  the normal watcher. A bare `[[name]]` still never matches a base,
  matching Obsidian; explicit `.base` targets that no longer exist stay
  `NotFound` (no suffix-match guessing).

### Fixed
- **Frontmatter links render and activate**: the metadata block's values
  are now inline-parsed (engine, recorded as deviation 9a in
  `VENDORED.md`). Frontmatter values like `related:` previously rendered
  as raw `[text](vault://N/target)` syntax and were missing from the
  clickable link list entirely — the links existed only in the vault
  index (backlinks), never in the UI. Now `related:` entries render as
  styled links (marker + label), resolve with the same same-folder
  proximity hint as body links (e.g. `[[project]]` correctly lands on the
  project note among 12 same-named `project.md`s), and open on click like
  every other link.

## [0.5.0] — 2026-11-16

### Added
- **Status line**: `Shift+T` and config theme hot-reloads show a transient
  `theme: <name>` info message (neutral color, 3 s); errors keep red and win.
- **Status bar note name**: shows the file name (`index.md`), or the
  vault-relative path when the name is shared with another note. Clicking
  it copies the vault-relative path to the clipboard (OSC 52).
- **Keyboard hints** on the status bar's right edge when there is room.
- **Mouse capture toggle**: `m` turns mouse capture off/on at runtime —
  off, the terminal keeps the mouse for native text selection/copy (the
  status bar shows `mouse off`); the config `mouse` key (default `true`)
  and the `--mouse[=true|false]` flag set the startup default.
- **Sidebar width override**: `<`/`>` narrow/widen the sidebar in 2-column
  steps (clamped 24–120); the config `sidebar_width` key sets the startup
  width. Content re-parses on every change as with a terminal resize.
- **Session memory**: the last open note with its scroll, plus the
  sidebar's mode, selection and folder expansion, are saved on clean quit
  to `~/.local/state/frond/session-<hash>.toml` (one file per vault) and
  restored on the next start; an explicit note argument wins over the
  restore. `FROND_SESSION` redirects the directory (tests/smokes).
- **`--config-show`**: prints a fully commented default `config.toml`
  (pinned to the real key vocabulary by a drift test), so a starter config
  is `frond --config-show > ~/.config/frond/config.toml`. Also added
  `--help` (full usage) and `--version`.
- **Restart smoke (M5)**: end-to-end coverage for the session memory's
  actual promise — run frond twice against the same vault through a pty:
  quit with a note open (scrolled, folders expanded) and the next launch
  comes back exactly there; an explicit note argument wins over the
  restore; a corrupted session file falls back silently to the index.
- **Backlink rows are clickable** (SPEC compliance): a click on a listed
  row in the backlinks panel selects it and opens the note (focus moves
  to the content pane, like every other open path); clicking the panel's
  border rows or empty space keeps the previous focus-only behavior.
- **Opened notes reveal in the sidebar**: following a wikilink, opening a
  search hit or backlink, or restoring the session expands the note's
  ancestor folders and selects its row in the tree (no focus change; the
  sidebar staying hidden is fine — the state is ready for the next `Tab`).
  A `.base` table expands its parent folder only (no tree row). The
  revealed expansion persists in the session file like any other.
- **`f` toggles the metadata block**: the note's frontmatter, which
  renders as a key/value table at the top, can be hidden to read the
  note itself (`f`, session toggle; the config `metadata` key sets the
  startup default, status bar shows `metadata off` while hidden). Purely
  visual — tags/status/backlinks come from the vault index either way.

### Changed
- **Opening a note moves focus to the content pane**: `Enter` on a note (tree
  or tag mode) and a click on a note row now hand the keyboard focus to the
  content, so the next `j`/`k`/PgDn/wheel scrolls the note just opened instead
  of the sidebar the selection came from (`l` on a note already did). The same
  applies to Enter on a backlink and on a search hit. Folder/tag toggles keep
  the sidebar focused; a failed open (error in the status bar) also keeps the
  pane that initiated the open. A wheel with the cursor over the sidebar
  column still scrolls the sidebar, regardless of focus.
- **←/→ join `h`/`l`**: the arrows now do exactly what the vim keys do —
  focus sidebar/content, expand/collapse a folder — completing the
  arrow set (↑/↓ scrolled and selected already, Home/End jumped).
- **Help popup is always a single column**: the multi-column packing
  (up to 3 side-by-side groups on wide terminals) is gone; the five groups
  render as one packed column with the scrollbar handling tall bodies, and
  the footer advertises `scroll …` only while the body actually overflows.
  Simpler, predictable layout at every terminal size.

### Fixed
- **Session save/restore honors `.base` tables**: quitting with a table
  open now remembers the base file and its row offset (previously the
  previous note and its stale scroll were saved); restore reopens the
  table at that offset. Restore also honors vault-relative entries only —
  absolute paths and `..` escapes in the (user-editable) session file are
  skipped, matching the documented "skipped silently" contract.
- **Mouse-off is real**: with capture disabled, mouse events are ignored
  by the event loop too, not just expected to stop arriving.
- **`$XDG_STATE_HOME`** is honored for the session directory before the
  `$HOME/.local/state` fallback; `~` in the `vault` config/CLI path now
  expands to `$HOME` (`~user` stays literal).
- **Help footer no longer clipped** at the popup's standard width
  (abbreviated to `scroll j/k · g/G ends · close esc/any key`).
- **`--config-show` template** corrected: the default theme is `ocean`;
  `~` in the example now actually expands.
- **Space-form flag mistakes error clearly**: `--mouse false` /
  `--watch false` fail with the `--mouse=<true|false>` hint instead of
  silently turning the value into the file argument.
- **Sidebar `<` cannot underflow** on degenerate drawn widths.
- Workspace version bumped to 0.5.0 so `--version` matches the CHANGELOG.

## [0.4.0] — 2026-11-11

### Added
- **Sidebar and status bar are themed**: two new `[ui]` keys,
  `sidebar_bg` (sidebar/tag view/backlinks panel) and `status_bg`
  (status bar), mapped from leaf's `toc_bg`/`status_bg` for the presets
  and all 14 built-ins — the sidebar and status bar no longer render on
  the terminal's default background. Popups (help/search/disambiguation)
  now paint `content_bg` instead of the terminal default. Per-key roles
  and per-family sources documented in `docs/COLORS.md`.

### Changed
- **Themes are frond-native**: leaf theme-file support is removed. The 14 leaf
  themes from the user's dotfiles (catppuccin-latte/frappé/macchiato/mocha,
  dracula, dracula-alucard, gruvbox, nord, rose-pine, rose-pine-dawn,
  rose-pine-moon, tokyonight-day/night/storm) are now built into the binary
  (resolved by name; `[ui]` keys translated to the app vocabulary at port
  time), and the `~/.config/leaf/themes/` compat directory is no longer
  read — theme files live in `~/.config/frond/themes/` in the frond format only
  (`base` + `syntax` + `[ui]` + `[markdown]`, app key vocabulary). A
  leaf-vocabulary file in the frond dir now loads with its leaf `[ui]` keys
  reported as unknown-key warnings instead of silently styling the chrome
  through aliases; unknown keys still warn, never pass silently.

- **`?` help pane redesigned** to a single centered column: a bold
  "keybinds" title and "esc close" chip at the top, grouped sections
  (Navigation, Links & tasks, Content, Sidebar, Theme & app) with
  `popup_match_fg` key labels, keys aligned to one column, a blank line
  between groups, the background dimmed behind the popup, and a fixed
  herdr-style footer (`scroll j/k/↑↓/pgup/pgdn · g/G top/bottom · close esc/any key`). The body scrolls with
  `j`/`k`/arrows, PgUp/PgDn, `g`/`G` (scrollbar on the right border); any
  other key closes it. All existing key labels are unchanged.

### Fixed
- **Mouse clicks move keyboard focus**: clicking a sidebar row moves focus to
  the sidebar (so `j`/`k`/`Enter` act there), clicking note content moves it
  to the content pane, and clicking the backlinks panel moves it there.
  `b` now always focuses the backlinks panel when it opens.

## [0.3.0] — 2026-11-11

### Added
- **`.base` table rendering**: opening a Bases file renders its table views (filters `file.inFolder`/`file.hasTag`/equality/`>= today()-Nd`, `displayName` columns, `file.asLink`/`file.name` cells as clickable links, sort + groupBy); `v`/`1-9` switches views; live refresh via the vault watch; `!=` supported as an approved extension; unknown expressions fail open with a warning; parser depth-capped and panic-proof.
- **Mid-session theme discovery**: a theme file added while the TUI runs joins the rotation at the next `Shift+T` wrap; an unresolvable config theme retries per tick until its file appears.
- **End-to-end smoke coverage** for the disambiguation popup, heading-anchor scrolling, and backlinks selection (smoke_m3: 29 → 46 checks).
- Config warnings persist on hot-reload paths (previously cleared on apply).

### Fixed
- **Leaf `[ui]` theme keys now style the chrome** : custom theme files written in leaf's full vocabulary (e.g. the catppuccin/gruvbox/nord files served from the `~/.config/leaf/themes/` compat directory) previously ignored every `[ui]` key as "unknown" — `Shift+T` to such a theme updated the note content but the sidebar/status chrome kept the base preset's colors, and a long unknown-keys warning fired. Leaf keys with a counterpart in this app now apply through two paths: keys unique to leaf's vocabulary (`toc_accent`, `toc_border`, `status_warning_fg`, `status_shortcut_fg`, `status_watch_fg`, `status_search_fg/bg`) through aliases, and the two keys that are also app-native names (`status_success_fg`, `status_error_fg`) through the native path — which also colors the git ↑/↓ arrows unless those are set explicitly (app-native keys still win per field when both are set); leaf keys with no counterpart here (TOC panel internals, brand/scrollbar/percent segments, and the `*_bg` status segments other than `status_search_bg`) remain unknown-key warnings, never silent.


## [0.2.0] — 2026-11-11

### Added
- **Status-glyph coverage** for every status used in the vault: `●` complete/resolved/active, `◐` in-progress/monitoring, `○` pending/planned, `◌` draft, `◉` blocked, `×` dimmed archived/superseded; unknown statuses stay glyph-less.
- **Theme support**: 4 paired built-in presets (arctic, forest, ocean, solarized — leaf's color values); custom themes in leaf's TOML format (`base` inheritance, `[markdown]`/`[ui]` sections) with unknown-key warnings, never silent; theme dirs `~/.config/frond/themes/` with `~/.config/leaf/themes/` compat fallback; `Shift+T` live cycling (session-only); theme key hot-reloads from config; resolution `--theme` > `FROND_THEME` > config > default.
- **Configuration file** `~/.config/frond/config.toml` (`theme`, `vault`, `watch`), CLI flags override, malformed file warns and falls back to defaults.
- **Guarded checkbox toggling** ( the app's first write feature): click or `x`/`X`+`Enter` cycles `- [ ]` → `- [~]` → `- [x]`; writes are byte-exact (marker splice + `updated:` bump per vault rules, CRLF/comment-preserving, field never added), atomic (temp+rename, fsync, permissions preserved), and fail closed unless the note's own git repo is clean and in sync and the file is unchanged since parse.
- **Search-row mouse support**: click selects a result, Enter/second-click opens, wheel scrolls the result list.
- `Makefile` (`build`, `release`, `test`, `smoke`, `fmt`, `clippy`, `check`, `run`, `clean`, `clean-fixtures`, `help`).
- Local git repository (no remote yet); `.gitignore` (build artifacts, no-git-era snapshots).

### Fixed
- Upstream leaf bug in `source_line_map`: nested list items mapped to the wrong source line (affected search scroll-to-line on lists) — fixed in the vendored engine, documented as `VENDORED.md` item 3a.

### Removed
- Temporary `render` bin; `cargo run -p app` now opens the TUI directly (`default-run`).

## [0.1.0] — 2026-11-10

Initial release: read-only Obsidian-style TUI for Markdown vaults.

- **Engine**: leaf's markdown renderer vendored as a library (18 modules, 192 ported tests) — tables, lists, frontmatter, callouts, syntax highlighting, LaTeX, Mermaid.
- **Reading**: scrollable TUI with watch mode (auto-reload preserving scroll), status bar, help popup.
- **Sidebar**: collapsible folder tree with status glyphs and changed-file markers; tag view; mouse clicks and wheel; sidebar/content focus switching.
- **Navigation**: wikilink rendering and activation (bare-filename resolution per vault rules, Obsidian same-folder precedence for ambiguous links, heading anchors, disambiguation popup for unresolvable ties), back/forward stack with scroll restore, backlinks panel (`b`), quick navigation.
- **Vault index**: frontmatter parsing, wikilink extraction (body + frontmatter, embeds tagged), resolver with ambiguity detection (never guesses), backlinks and tag maps, incremental single-file reindex, vault-wide watch (~2.7 ms/s on 478 notes).
- **Search**: global vault search with match highlighting and jump-to-line.
- **Git sync status** (read-only): clean/dirty/ahead/behind in the status bar, refreshed on open/watch/`u`.
- Frontmatter `status:` glyphs, unresolved-link styling, `- [~]` in-progress rendering.

