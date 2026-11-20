# Coloring — `UiTheme` keys and their sources

Every chrome color the app styles lives in `UiTheme` (`crates/app/src/theme.rs`);
custom theme files set these names under `[ui]`. Note-content colors are the
engine's `MarkdownTheme` (leaf's own palette, not documented here).

## Appearance: light or dark

Every theme declares whether it is light or dark. The `Shift+T` cycle filters
on it: `Ctrl+T` moves between all themes, light themes only and dark themes
only (session only; `theme_rotation` in config.toml sets the startup group).

- **Presets** (`crates/app/src/theme.rs`, `preset_appearance`): `arctic` is
  light; `forest`, `ocean` and `solarized` are dark.
- **Built-ins** (`crates/app/src/builtin_themes.rs`): each entry sets
  `appearance: Appearance::Light` or `Dark`. Light: `catppuccin-latte`,
  `dracula-alucard`, `rose-pine-dawn`, `tokyonight-day`. The rest are dark.
- **Custom theme files** must set it at the top of the file:

  ```toml
  appearance = "light"   # or "dark"; required
  base = "arctic"
  ```

  A file without it, or with any other value, is not loaded. It is reported
  as a load error that names the file and the key.

The value must match the theme's `content_bg`: a light theme has a light
background. The test `declared_appearance_matches_the_background_of_every_theme`
checks every preset and built-in, so a mislabelled theme fails the suite.

## The keys

| Key | Role in this app | Source role (leaf) |
|---|---|---|
| `sidebar_bg` | Sidebar (tree/tags) + backlinks panel background | `toc_bg` |
| `sidebar_border_focused` | Focused sidebar/tag/backlinks border; the status-bar name when the sidebar is focused | `toc_accent` |
| `sidebar_border_unfocused` | Unfocused sidebar/tag/backlinks border; `.base` table border | `toc_border` |
| `selection_bg` | Selected row bar (full-opacity accent); the row text is the panel background colour | `toc_accent` |
| `changed_marker_fg` | Sidebar ` *` changed-file marker | `status_warning_fg` |
| `dimmed_fg` | Dimmed sidebar names, backlinks empty state, `.base` rule line | `status_shortcut_fg` |
| `content_bg` | Note content, `.base` table, and all popup backgrounds | `content_bg` |
| `status_bg` | Status bar background | `status_bg` |
| `status_watch_on_fg` / `status_watch_off_fg` | `watch on` / `watch off` indicator | `status_watch_fg` / `status_shortcut_fg` |
| `status_success_fg` | Clean `✓` | `status_success_fg` |
| `status_dirty_fg` | Dirty `●` | `status_warning_fg` |
| `status_ahead_fg` / `status_behind_fg` | Ahead `↑n` / behind `↓n` | `status_success_fg` / `status_error_fg` |
| `status_error_fg` | Reload/write/base-warnings in the status bar | `status_error_fg` |
| `popup_border_fg` | Popup borders; `.base` header/group accents | `toc_border` |
| `popup_dimmed_fg` | Popup footers, "no matches", every scrollbar's track (its thumb takes `scrollbar_hover_fg` while the mouse is over the bar) | `status_shortcut_fg` |
| `popup_match_fg` / `popup_match_bg` | Search-match and quick-switcher match highlight; help-pane key labels (`popup_match_fg`) | `status_search_fg` / `status_search_bg` |
| `scrollbar_hover_fg` | A scrollbar's thumb while the mouse is over the bar (the accent of the theme) | `toc_accent` |

Note: `status_ahead_fg`, `status_watch_off_fg`, and `popup_dimmed_fg` have
their own app roles (ahead arrow, `watch off` indicator, popup footers/empty
states) but *default* to the same leaf source values as `status_success_fg`,
`dimmed_fg`'s source key, and `dimmed_fg` respectively — which is why their
roles are easy to conflate with those keys'.

## Source roles per theme family

The 14 built-ins were ported verbatim from the user's leaf theme files; the
mapping above (`toc_bg`/`status_bg` included) is mechanical, so each key
carries the family's intended role:

- **Catppuccin** (latte/frappé/macchiato/mocha — [style guide](https://catppuccin.com/style-guide/)):
  content = **Base**, sidebar/status = **Mantle** (the darker base), borders =
  **Surface 1**, match bg = **Surface 0**, dimmed = **Overlay 0**, selection =
  the flavor's accent (**Lavender**/**Mauve**), status semantics = **Green**
  (success), **Peach** (warning), **Red** (error), **Yellow** (search match).
- **Dracula** (dracula, dracula-alucard — [spec](https://draculatheme.com/spec)):
  content = **Background**, sidebar/status = **Background Darker**, dimmed =
  **Comment**, borders = **Current Line**/Selection-tinted colors, selection =
  **Purple**, status semantics use **syntax** colors (**Green**/**Orange**/
  **Red**/**Yellow**), not dedicated functional colors.
- **Nord** ([nordtheme.com](https://nordtheme.com)): content = **nord0** and
  sidebar/status = **nord1** (both **polar-night**), dimmed/borders =
  **nord3** (polar-night), selection/accent = **nord8** (**frost**), status
  semantics = **aurora** (**nord14** green, **nord13** yellow, **nord11** red).
- **Rosé Pine** (main/dawn/moon): content = **base**, sidebar/status =
  **surface**, dimmed = **muted**, borders = **highlightMed**/**highlightHigh**,
  match bg = **overlay**, selection = **rose**, status semantics = **foam**
  (success), **gold** (warning), **love** (error).
- **Tokyo Night** (day/night/storm): content = **bg**, sidebar/status =
  **bg_dark** (the palette's dedicated darker background), dimmed = the
  muted blue-gray, selection = **blue**.
- **gruvbox**: content = **bg0**, sidebar/status = **bg0_h** (hard dark bg —
  the community convention for chrome), borders/match = **bg1**/**bg2**,
  dimmed = **gray**, selection = **orange** (a bright accent).
- **Presets** (arctic/ocean/forest/solarized): leaf's own hand-tuned `UiTheme`
  constants (`src/theme/presets.rs` in leaf v1.29.2), taken verbatim.

## Deliberate deviations

- **Selection = accent at full opacity.** Catppuccin's style guide selects
  with translucent **Overlay 2**; a TUI has no alpha compositing, so leaf —
  and this app after it — uses the accent color at full opacity instead.
  Every theme family follows this one rule (Nord uses frost **nord8**, not
  a muted polar-night tint; gruvbox uses bright orange, not bg2).
- **The selected row is inverse video.** The bar is the accent at full
  opacity (the rule above), and the row text is the panel background colour
  (`sidebar_bg` for the sidebar family, `content_bg` for the popups). The
  accent is usually the same colour as the text it would sit on (contrast
  ~1.1, the same reason the link focus highlight in `crates/app/src/links.rs`
  is inverse video), so the text is inverted rather than the bar muted.
- **Dimmed/border picks are leaf's choices**, kept verbatim: Catppuccin
  uses **Overlay 0** (not Overlay 1) for dimmed text and **Surface 1** for
  borders; Nord uses **nord3** (not nord5) for dimmed/borders; Dracula
  colors its borders from **Selection/Current-Line** family colors rather
  than a neutral gray.
- **Dracula status colors are syntax colors, not functional ones**: the
  spec assigns Green/Orange/Red/Yellow to code tokens, and leaf reuses them
  for success/warning/error segments — that is faithful to leaf, not an
  oversight.
