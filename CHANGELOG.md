# Changelog

Format follows [Keep a Changelog](https://keepachangelog.com/) loosely; dates are UTC.

## [Unreleased]

### Added
- **Status line**: `Shift+T` and config theme hot-reloads show a transient
  `theme: <name>` info message (neutral color, 3 s); errors keep red and win.
- **Status bar note name**: shows the file name (`index.md`), or the
  vault-relative path when the name is shared with another note. Clicking
  it copies the vault-relative path to the clipboard (OSC 52).
- **Keyboard hints** on the status bar's right edge when there is room.

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

