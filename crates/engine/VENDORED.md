# VENDORED.md — deviations from upstream leaf

Source: https://github.com/rivolink/leaf (MIT), v1.29.2, `src/markdown/` +
markdown-relevant parts of `src/theme/`. A reference clone of that tag is
needed to diff against upstream — the old `/tmp/leaf` copy does not survive
a restart, so re-clone the tag when a merge is due.

Goal: keep the vendored code as close to upstream as practical so future leaf
improvements can be merged. Every intentional change is listed here.

## Structure

1. **Relocation.** `src/markdown/` → `crates/engine/src/markdown/` (unchanged
   file set: 18 modules). `lib.rs` declares `pub mod markdown;`.
2. **Theme trimmed.** leaf's `src/theme/` (mod/presets/resolution/serde, ~4
   presets with UI + diff + config machinery) collapsed into
   `crates/engine/src/theme.rs`: the `MarkdownTheme` struct (verbatim) plus the
   four built-in markdown palettes (Arctic = BASE_LIGHT_MARKDOWN, Forest,
   OceanDark = BASE_DARK_MARKDOWN, SolarizedDark). Arctic and OceanDark are
   verbatim from `presets.rs` standalone consts (`presets.rs:163/213`); Forest
   and SolarizedDark are extracted verbatim from the inline `markdown:` blocks
   of the `AppTheme` consts `FOREST_THEME` / `SOLARIZED_DARK_THEME`
   (`presets.rs:309/407`), not standalone constants upstream. Dropped:
   `UiTheme`, `DiffTheme`, `CustomTheme`, `ThemeSelection`, global
   `CURRENT_THEME` state, serde config, color-string resolution. Palettes are
   exposed via `markdown_theme_by_preset()` / `default_markdown_theme()`
   (default = OceanDark, leaf's default preset).

## Code changes inside vendored files

3. **Removed `read_file_state` / `hash_file_contents` / `hash_str`**
   (`markdown/mod.rs`). They referenced leaf's `crate::app::FileState` and are
   unused by the markdown module itself (leaf's app shell used them).
3a. **List-item source-line snapshot** (post-v1 loop 4, `lists.rs` +
   `mod.rs`): `ItemState` gained `src_line`, snapshotted from the event
   cursor at `Start(Item)` and restored before an item's spans are
   flushed (`End(Item)`, and the `Start(List)` flush of the enclosing
   item). Upstream computes the source map from the *flush-time* event
   position, which attributes a list item's rendered lines to whatever
   line the cursor reached when the item ended — for nested lists the
   outer item's lines map to the inner item's line (verified against the
   upstream logic: the flush happens after the nested list's items, so
   `current_src_line` has already advanced). The snapshot makes
   `source_line_map` point at the item's own first raw line, which the
   viewer needs for scroll-to-line (search hits on list items) and for
   mapping rendered checkbox glyphs back to their raw source lines.
   No rendering changes — the map values only move closer to the truth.
4. **Threading `&MarkdownTheme` through table rendering** (`tables.rs`,
   `mod.rs`): `TableBuf::render` and `handle_table_event` now take the theme as
   a parameter instead of calling leaf's global `crate::theme::app_theme()`
   inside `render` (the global state was removed per item 2).
5. **Visibility bumps** so the public API works: `ParseResult` (+ all fields),
   `parse_markdown`, `parse_markdown_with_width`, `CodeBlockInfo` (+ fields),
   `CodeRow` (+ fields), `LinkId`, `LinkSpan`, `TocEntry` changed from
   `pub(crate)`/`pub(super)` to `pub`. `width`/`toc` modules remain
   crate-internal, but `TocEntry` is re-exported from `markdown/mod.rs` and
   `lib.rs` so dependents can name the type (it appears in the public
   `ParseResult.toc` field).
6. **`#![allow(dead_code)]` module-wide in `markdown/mod.rs`.** leaf's app
   consumes some functions the engine doesn't call yet (search highlight:
   `highlight_line`, `build_searchable_lines`, …; `set_link_marker`; bullet
   glyphs `BULLET_*`; `is_ordered_marker`; `toc_levels`). Kept upstream-intact
   for upcoming milestones instead of deleting them; the re-exports that only
   the ported tests use are gated `#[cfg(test)]`.
7. **Removed root re-exports** that existed for leaf's `main.rs` glob imports
   and are unused by the engine.

## New engine-level API (not in leaf)

8. `crates/engine/src/lib.rs`: `SyntaxAssets::load_defaults()` (syntect default
   syntax set + `base16-ocean.dark` theme) and re-exports of the parse API and
   theme helpers. The parse function signatures are leaf's own
   (`parse_markdown_with_width(src, &SyntaxSet, &Theme, width, &MarkdownTheme,
   file_mode, code_line_numbers)`), including the `theme_colors` parameter
   name used by both `parse_markdown` and `parse_markdown_with_width` (leaf's
   original name). Post-v1 loop 3 (themes) added one more `SyntaxAssets`
   helper: `set_syntax_theme(name)` (swap the highlighting theme by name
   from syntect's default theme set; `false` when unknown, assets
   untouched — the validation is internal, so no separate
   `has_syntax_theme` probe exists; an earlier draft of one was removed
   as dead API) — leaf's equivalent lives in its app layer
   (`theme::current_syntect_theme`).
9. **`parse_markdown_with_width_opts`** (metadata toggle): same signature as
   `parse_markdown_with_width` plus a trailing `show_metadata: bool` — when
   false, the frontmatter key/value block is not rendered (the body renders
   as if the frontmatter were absent; `source_line_map` keeps the
   frontmatter offset, so scroll-to-source stays accurate). leaf's
   `parse_markdown_with_width` delegates with `true`, so the upstream API
   surface is unchanged and all existing call sites keep compiling.
9a. **Metadata values are inline-parsed** (post-v1 loop 7, `tables.rs` +
   `mod.rs`): the metadata block is built by
   `TableBuf::from_key_value_pairs_inline`, which runs each *value* through
   the same pulldown inline pipeline as body-table cells (keys stay plain
   text). This matters for the app's wikilink preprocessing: frontmatter
   values like `related:` arrive as `"[alias](vault://N/target)"` lists, and
   without inline parsing they rendered as raw `[text](url)` syntax with the
   links unclickable (the old call discarded the link-span sink via
   `&mut Vec::new()`). Now the block's links register into the shared
   `link_urls`/`link_spans` vectors (the block renders first, so its links
   take the lowest ids) and the app's URL-exact-match mapping makes them
   clickable like body links.

## Theme loading (the un-trim, app-side)

12. leaf's theme **TOML loading** (serde color parsing, preset inheritance,
    theme-file resolution) was ported in post-v1 loop 3 — but into the app
    crate (`crates/app/src/theme.rs`), not back into the engine. The engine's
    `theme.rs` stays markdown-only (item 2 stands): the app-level `Theme` =
    engine `MarkdownTheme` + an app-owned `UiTheme` chrome palette. Ported
    from leaf `src/theme/serde.rs`: the `ThemeColor` deserializer (hex /
    `rgb()` / named / `[r,g,b]`) and the overrides-apply pattern (the
    `theme_overrides!` macro, extended with `#[serde(flatten)]` unknown-key
    capture so ignored keys are *reported*, never silent). Ported from
    leaf `src/theme/resolution.rs`: `parse_theme_color` (as `parse_color`),
    preset-name aliases (`arctic`, `ocean`/`ocean-dark`/`dark`, `forest`,
    `solarized`/`solarized-dark`), and base-preset inheritance
    (`custom_theme_selection` semantics; default base = OceanDark). Not
    ported: leaf's `[diff]` theme (no diff view here — its keys are parsed
    only to be listed in the warning), the global `CURRENT_THEME` state and
    file-path-based theme selection (theme dirs are scanned instead:
    `~/.config/frond/themes/`; the leaf compat directory was dropped when the
    14 leaf themes were ported into the app's built-in table).
    Leaf-parity deviations in theme-file handling (deliberate, same end
    state): leaf trims `syntax` and errors on `syntax = ""` at load, while
    the app passes it through and defers to an apply-time warning +
    default syntect theme; and the app trims `base` where leaf does not
    (the app is more lenient there).

## Tests

9. leaf's `src/tests/` markdown suites ported to `crates/engine/src/tests/`
   (10 files, 192 tests, all passing): `markdown_blocks`, `markdown_embedded`,
   `markdown_footnotes`, `markdown_html_tag`, `markdown_links`,
   `markdown_list_blocks`, `markdown_lists`, `markdown_rendering`,
   `markdown_tables`, `wrapping`, plus the needed common helpers. Not ported:
   suites bound to leaf's `App`/render pipeline (link_mapping, toc, app_*…).
   Adjustments: `App::fence_wrap` inlined as a local helper; tests that read
   `crate::theme::app_theme().markdown` now use `test_md_theme()`;
   `theme_by_preset(ThemePreset::OceanDark).markdown` →
   `default_markdown_theme()`.

## Dependencies / workspace

10. **Workspace `ratatui` bumped 0.29 → 0.30** (and `crossterm` 0.28 → 0.29):
    leaf's markdown uses `ratatui::buffer::CellWidth` / `str::cell_width()`
    (0.30 API); engine and app must share one ratatui version for `Line<'static>`
    values to flow between them. leaf's exact versions: ratatui 0.30,
    crossterm 0.29.
11. Engine dependencies mirror leaf's markdown module: `pulldown-cmark 0.12`,
    `syntect 5.2` (same feature set), `unicode-width 0.2`,
    `unicode-segmentation 1`, `mmdflux 2.5`, `unicodeit 0.2`.

## Not yet ported (upcoming work)

- Wikilink preprocessing hook (`[[target|alias]]` → `[alias](vault://target)`) —
  implemented in M3 **app-side** (`crates/app/src/links.rs`), not in the engine:
  the app rewrites wikilinks to plain markdown links before calling
  `parse_markdown_with_width`, so the engine stays byte-identical to upstream.
- `crate::app::FileState` equivalents and file-mode plumbing that leaf's shell
  owns (re-parse triggers etc.).

## Related approved deviation (not a vendored-code change)

13. **`prop != value` inequality** in the `.base` filter subset
    (`crates/vault/src/base.rs` — workspace code, not vendored leaf) is an
    approved extension beyond Obsidian's documented Bases surface (approved
    by the orchestrator, needed by the real GPU-matrix base). Recorded here
    alongside SPEC.md because this file is the project's deviations ledger.
