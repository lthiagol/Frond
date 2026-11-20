//! The 14 built-in themes: the leaf theme files from the user's dotfiles
//! (`~/.config/leaf/themes/*.toml`, symlinked from
//! `~/.dotfiles/leaf/.config/leaf/themes/`), ported into an in-binary table.
//!
//! Port rules (per file): `base` and `syntax` kept; the `[markdown]` section
//! copied verbatim (field names are the engine's `MarkdownTheme`); the
//! `[ui]` keys mapped onto our `UiTheme` vocabulary:
//! `toc_accent` → sidebar_border_focused + selection_bg + scrollbar_hover_fg,
//! `toc_border` → sidebar_border_unfocused + popup_border_fg,
//! `toc_bg` → sidebar_bg,
//! `status_bg` → status_bg,
//! `status_warning_fg` → changed_marker_fg + status_dirty_fg,
//! `status_shortcut_fg` → dimmed_fg + status_watch_off_fg + popup_dimmed_fg,
//! `status_watch_fg` → status_watch_on_fg,
//! `status_success_fg` → status_success_fg + status_ahead_fg,
//! `status_error_fg` → status_error_fg + status_behind_fg,
//! `status_search_fg` → popup_match_fg, `status_search_bg` → popup_match_bg,
//! `content_bg` → content_bg. Leaf keys with no target in this app are
//! dropped (this table is code, not user input — nothing to warn about);
//! every ported file carried both `toc_bg` and `status_bg`, so nothing
//! needed a fallback.
//!
//! Values below are verbatim from the source TOMLs (hex → `Rgb`); the
//! `theme::tests` spot-checks pin them against the same sources.

use engine::{MarkdownTheme, MarkdownThemePreset};
use ratatui::style::Color;

use crate::theme::{Appearance, UiTheme};

/// One `[markdown]` field setter, so a ported theme can store its palette as
/// `(key, color)` overrides applied over the base preset (the ported files
/// happen to override every field, but `base` stays the inheritance root).
/// The keys live in the [`md`] module, one per `MarkdownTheme` field, generated
/// from the same field list as the config-file overrides
/// (`theme::with_markdown_fields!`), so the two cannot drift.
#[derive(Clone, Copy)]
pub(crate) struct MdKey {
    #[cfg_attr(not(test), allow(dead_code))] // read by the uniqueness test
    pub name: &'static str,
    set: fn(&mut MarkdownTheme, Color),
}

impl MdKey {
    pub fn apply(&self, md: &mut MarkdownTheme, color: Color) {
        (self.set)(md, color);
    }
}

macro_rules! md_keys {
    ($($field:ident),+ $(,)?) => {
        /// One [`MdKey`] constant per `MarkdownTheme` field, named like it.
        #[allow(non_upper_case_globals)]
        pub(crate) mod md {
            use super::*;
            $(pub const $field: MdKey = MdKey {
                name: stringify!($field),
                set: |md, color| md.$field = color,
            };)+
        }
    };
}
crate::theme::with_markdown_fields!(md_keys);

/// One built-in theme: the ported file's `base`/`syntax` keys, its `[ui]`
/// section mapped to the app vocabulary, and its `[markdown]` section as
/// `(key, color)` overrides (alphabetical by name — also the `Shift+T`
/// cycle order).
pub(crate) struct BuiltinTheme {
    pub name: &'static str,
    pub appearance: Appearance,
    pub base: MarkdownThemePreset,
    pub syntax: Option<&'static str>,
    pub ui: UiTheme,
    pub md: &'static [(MdKey, Color)],
}

pub(crate) const BUILTIN_THEMES: &[BuiltinTheme] = &[
    BuiltinTheme {
        name: "catppuccin-frappe",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xba, 0xbb, 0xf1), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x51, 0x57, 0x6d), // toc_border
            selection_bg: Color::Rgb(0xba, 0xbb, 0xf1),           // toc_accent
            sidebar_bg: Color::Rgb(0x29, 0x2c, 0x3c),             // toc_bg
            changed_marker_fg: Color::Rgb(0xef, 0x9f, 0x76),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x73, 0x79, 0x94),              // status_shortcut_fg
            content_bg: Color::Rgb(0x30, 0x34, 0x46),             // content_bg
            status_bg: Color::Rgb(0x29, 0x2c, 0x3c),              // status_bg
            status_watch_on_fg: Color::Rgb(0xa6, 0xd1, 0x89),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x73, 0x79, 0x94),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0xa6, 0xd1, 0x89),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xef, 0x9f, 0x76),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0xa6, 0xd1, 0x89),        // status_success_fg
            status_behind_fg: Color::Rgb(0xe7, 0x82, 0x84),       // status_error_fg
            status_error_fg: Color::Rgb(0xe7, 0x82, 0x84),        // status_error_fg
            popup_border_fg: Color::Rgb(0x51, 0x57, 0x6d),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x73, 0x79, 0x94),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xe5, 0xc8, 0x90),         // status_search_fg
            popup_match_bg: Color::Rgb(0x41, 0x45, 0x59),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xba, 0xbb, 0xf1),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::search_match_bg, Color::Rgb(0xe5, 0xc8, 0x90)),
            (md::code_gutter, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::blockquote_marker, Color::Rgb(0x73, 0x79, 0x94)),
            (md::list_level_1, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::list_level_2, Color::Rgb(0x8c, 0xaa, 0xee)),
            (md::list_level_3, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (md::ordered_list, Color::Rgb(0xef, 0x9f, 0x76)),
            (md::table_border, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::table_separator, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::table_header, Color::Rgb(0xe5, 0xc8, 0x90)),
            (md::table_cell, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (md::heading_1, Color::Rgb(0xca, 0x9e, 0xe6)),
            (md::heading_2, Color::Rgb(0x8c, 0xaa, 0xee)),
            (md::heading_3, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::heading_4, Color::Rgb(0xef, 0x9f, 0x76)),
            (md::heading_other, Color::Rgb(0xba, 0xbb, 0xf1)),
            (md::heading_underline, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::code_frame, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::code_label, Color::Rgb(0x73, 0x79, 0x94)),
            (md::inline_code_fg, Color::Rgb(0xef, 0x9f, 0x76)),
            (md::inline_code_bg, Color::Rgb(0x29, 0x2c, 0x3c)),
            (md::rule, Color::Rgb(0x51, 0x57, 0x6d)),
            (md::link_icon, Color::Rgb(0x99, 0xd1, 0xdb)),
            (md::link_text, Color::Rgb(0x99, 0xd1, 0xdb)),
            (md::link_hover, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::blockquote_text, Color::Rgb(0xa5, 0xad, 0xce)),
            (md::text, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (md::latex_inline_bg, Color::Rgb(0x29, 0x2c, 0x3c)),
            (md::latex_block_fg, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (md::mermaid_keyword, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (md::mermaid_arrow, Color::Rgb(0x99, 0xd1, 0xdb)),
            (md::mermaid_label, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::mermaid_block_fg, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (md::mark_fg, Color::Rgb(0x30, 0x34, 0x46)),
            (md::mark_bg, Color::Rgb(0xe5, 0xc8, 0x90)),
            (md::task_checked, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::task_unchecked, Color::Rgb(0x73, 0x79, 0x94)),
            (md::alert_note, Color::Rgb(0x8c, 0xaa, 0xee)),
            (md::alert_tip, Color::Rgb(0xa6, 0xd1, 0x89)),
            (md::alert_important, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (md::alert_warning, Color::Rgb(0xe5, 0xc8, 0x90)),
            (md::alert_caution, Color::Rgb(0xe7, 0x82, 0x84)),
            (md::footnote_ref, Color::Rgb(0x99, 0xd1, 0xdb)),
            (md::footnote_text, Color::Rgb(0xb5, 0xbf, 0xe2)),
        ],
    },
    BuiltinTheme {
        name: "catppuccin-latte",
        appearance: Appearance::Light,
        base: MarkdownThemePreset::Arctic,
        syntax: Some("InspiredGitHub"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x72, 0x87, 0xfd), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0xbc, 0xc0, 0xcc), // toc_border
            selection_bg: Color::Rgb(0x72, 0x87, 0xfd),           // toc_accent
            sidebar_bg: Color::Rgb(0xe6, 0xe9, 0xef),             // toc_bg
            changed_marker_fg: Color::Rgb(0xfe, 0x64, 0x0b),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x9c, 0xa0, 0xb0),              // status_shortcut_fg
            content_bg: Color::Rgb(0xef, 0xf1, 0xf5),             // content_bg
            status_bg: Color::Rgb(0xe6, 0xe9, 0xef),              // status_bg
            status_watch_on_fg: Color::Rgb(0x40, 0xa0, 0x2b),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x9c, 0xa0, 0xb0),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x40, 0xa0, 0x2b),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xfe, 0x64, 0x0b),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x40, 0xa0, 0x2b),        // status_success_fg
            status_behind_fg: Color::Rgb(0xd2, 0x0f, 0x39),       // status_error_fg
            status_error_fg: Color::Rgb(0xd2, 0x0f, 0x39),        // status_error_fg
            popup_border_fg: Color::Rgb(0xbc, 0xc0, 0xcc),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x9c, 0xa0, 0xb0),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xdf, 0x8e, 0x1d),         // status_search_fg
            popup_match_bg: Color::Rgb(0xe8, 0xe7, 0xe2),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x72, 0x87, 0xfd),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::search_match_bg, Color::Rgb(0xf0, 0xe3, 0xb0)),
            (md::code_gutter, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (md::blockquote_marker, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (md::list_level_1, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::list_level_2, Color::Rgb(0x1e, 0x66, 0xf5)),
            (md::list_level_3, Color::Rgb(0xea, 0x76, 0xcb)),
            (md::ordered_list, Color::Rgb(0xfe, 0x64, 0x0b)),
            (md::table_border, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::table_separator, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::table_header, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (md::table_cell, Color::Rgb(0x4c, 0x4f, 0x69)),
            (md::heading_1, Color::Rgb(0x88, 0x39, 0xef)),
            (md::heading_2, Color::Rgb(0x1e, 0x66, 0xf5)),
            (md::heading_3, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::heading_4, Color::Rgb(0xfe, 0x64, 0x0b)),
            (md::heading_other, Color::Rgb(0x72, 0x87, 0xfd)),
            (md::heading_underline, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::code_frame, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::code_label, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (md::inline_code_fg, Color::Rgb(0xfe, 0x64, 0x0b)),
            (md::inline_code_bg, Color::Rgb(0xdc, 0xe0, 0xe8)),
            (md::rule, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (md::link_icon, Color::Rgb(0x04, 0xa5, 0xe5)),
            (md::link_text, Color::Rgb(0x04, 0xa5, 0xe5)),
            (md::link_hover, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::blockquote_text, Color::Rgb(0x6c, 0x6f, 0x85)),
            (md::text, Color::Rgb(0x4c, 0x4f, 0x69)),
            (md::strong_text, Color::Rgb(0x1d, 0x1e, 0x2b)),
            (md::latex_inline_fg, Color::Rgb(0xea, 0x76, 0xcb)),
            (md::latex_inline_bg, Color::Rgb(0xdc, 0xe0, 0xe8)),
            (md::latex_block_fg, Color::Rgb(0xea, 0x76, 0xcb)),
            (md::mermaid_keyword, Color::Rgb(0xea, 0x76, 0xcb)),
            (md::mermaid_arrow, Color::Rgb(0x04, 0xa5, 0xe5)),
            (md::mermaid_label, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::mermaid_block_fg, Color::Rgb(0x4c, 0x4f, 0x69)),
            (md::mark_fg, Color::Rgb(0xef, 0xf1, 0xf5)),
            (md::mark_bg, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (md::task_checked, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::task_unchecked, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (md::alert_note, Color::Rgb(0x1e, 0x66, 0xf5)),
            (md::alert_tip, Color::Rgb(0x40, 0xa0, 0x2b)),
            (md::alert_important, Color::Rgb(0xea, 0x76, 0xcb)),
            (md::alert_warning, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (md::alert_caution, Color::Rgb(0xd2, 0x0f, 0x39)),
            (md::footnote_ref, Color::Rgb(0x04, 0xa5, 0xe5)),
            (md::footnote_text, Color::Rgb(0x5c, 0x5f, 0x77)),
        ],
    },
    BuiltinTheme {
        name: "catppuccin-macchiato",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xb7, 0xbd, 0xf8), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x49, 0x4d, 0x64), // toc_border
            selection_bg: Color::Rgb(0xb7, 0xbd, 0xf8),           // toc_accent
            sidebar_bg: Color::Rgb(0x1e, 0x20, 0x30),             // toc_bg
            changed_marker_fg: Color::Rgb(0xf5, 0xa9, 0x7f),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x6e, 0x73, 0x8d),              // status_shortcut_fg
            content_bg: Color::Rgb(0x24, 0x27, 0x3a),             // content_bg
            status_bg: Color::Rgb(0x1e, 0x20, 0x30),              // status_bg
            status_watch_on_fg: Color::Rgb(0xa6, 0xda, 0x95),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x6e, 0x73, 0x8d),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0xa6, 0xda, 0x95),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xf5, 0xa9, 0x7f),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0xa6, 0xda, 0x95),        // status_success_fg
            status_behind_fg: Color::Rgb(0xed, 0x87, 0x96),       // status_error_fg
            status_error_fg: Color::Rgb(0xed, 0x87, 0x96),        // status_error_fg
            popup_border_fg: Color::Rgb(0x49, 0x4d, 0x64),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x6e, 0x73, 0x8d),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xee, 0xd4, 0x9f),         // status_search_fg
            popup_match_bg: Color::Rgb(0x36, 0x3a, 0x4f),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xb7, 0xbd, 0xf8),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::search_match_bg, Color::Rgb(0xee, 0xd4, 0x9f)),
            (md::code_gutter, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::blockquote_marker, Color::Rgb(0x6e, 0x73, 0x8d)),
            (md::list_level_1, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::list_level_2, Color::Rgb(0x8a, 0xad, 0xf4)),
            (md::list_level_3, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (md::ordered_list, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (md::table_border, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::table_separator, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::table_header, Color::Rgb(0xee, 0xd4, 0x9f)),
            (md::table_cell, Color::Rgb(0xca, 0xd3, 0xf5)),
            (md::heading_1, Color::Rgb(0xc6, 0xa0, 0xf6)),
            (md::heading_2, Color::Rgb(0x8a, 0xad, 0xf4)),
            (md::heading_3, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::heading_4, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (md::heading_other, Color::Rgb(0xb7, 0xbd, 0xf8)),
            (md::heading_underline, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::code_frame, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::code_label, Color::Rgb(0x6e, 0x73, 0x8d)),
            (md::inline_code_fg, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (md::inline_code_bg, Color::Rgb(0x1e, 0x20, 0x30)),
            (md::rule, Color::Rgb(0x49, 0x4d, 0x64)),
            (md::link_icon, Color::Rgb(0x91, 0xd7, 0xe3)),
            (md::link_text, Color::Rgb(0x91, 0xd7, 0xe3)),
            (md::link_hover, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::blockquote_text, Color::Rgb(0xa5, 0xad, 0xcb)),
            (md::text, Color::Rgb(0xca, 0xd3, 0xf5)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (md::latex_inline_bg, Color::Rgb(0x1e, 0x20, 0x30)),
            (md::latex_block_fg, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (md::mermaid_keyword, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (md::mermaid_arrow, Color::Rgb(0x91, 0xd7, 0xe3)),
            (md::mermaid_label, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::mermaid_block_fg, Color::Rgb(0xca, 0xd3, 0xf5)),
            (md::mark_fg, Color::Rgb(0x24, 0x27, 0x3a)),
            (md::mark_bg, Color::Rgb(0xee, 0xd4, 0x9f)),
            (md::task_checked, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::task_unchecked, Color::Rgb(0x6e, 0x73, 0x8d)),
            (md::alert_note, Color::Rgb(0x8a, 0xad, 0xf4)),
            (md::alert_tip, Color::Rgb(0xa6, 0xda, 0x95)),
            (md::alert_important, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (md::alert_warning, Color::Rgb(0xee, 0xd4, 0x9f)),
            (md::alert_caution, Color::Rgb(0xed, 0x87, 0x96)),
            (md::footnote_ref, Color::Rgb(0x91, 0xd7, 0xe3)),
            (md::footnote_text, Color::Rgb(0xb8, 0xc0, 0xe0)),
        ],
    },
    BuiltinTheme {
        name: "catppuccin-mocha",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-mocha.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xb4, 0xbe, 0xfe), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x45, 0x47, 0x5a), // toc_border
            selection_bg: Color::Rgb(0xb4, 0xbe, 0xfe),           // toc_accent
            sidebar_bg: Color::Rgb(0x18, 0x18, 0x25),             // toc_bg
            changed_marker_fg: Color::Rgb(0xfa, 0xb3, 0x87),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x6c, 0x70, 0x86),              // status_shortcut_fg
            content_bg: Color::Rgb(0x1e, 0x1e, 0x2e),             // content_bg
            status_bg: Color::Rgb(0x18, 0x18, 0x25),              // status_bg
            status_watch_on_fg: Color::Rgb(0xa6, 0xe3, 0xa1),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x6c, 0x70, 0x86),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0xa6, 0xe3, 0xa1),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xfa, 0xb3, 0x87),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0xa6, 0xe3, 0xa1),        // status_success_fg
            status_behind_fg: Color::Rgb(0xf3, 0x8b, 0xa8),       // status_error_fg
            status_error_fg: Color::Rgb(0xf3, 0x8b, 0xa8),        // status_error_fg
            popup_border_fg: Color::Rgb(0x45, 0x47, 0x5a),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x6c, 0x70, 0x86),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xf9, 0xe2, 0xaf),         // status_search_fg
            popup_match_bg: Color::Rgb(0x31, 0x32, 0x44),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xb4, 0xbe, 0xfe),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::search_match_bg, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (md::code_gutter, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::blockquote_marker, Color::Rgb(0x6c, 0x70, 0x86)),
            (md::list_level_1, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::list_level_2, Color::Rgb(0x89, 0xb4, 0xfa)),
            (md::list_level_3, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (md::ordered_list, Color::Rgb(0xfa, 0xb3, 0x87)),
            (md::table_border, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::table_separator, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::table_header, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (md::table_cell, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (md::heading_1, Color::Rgb(0xcb, 0xa6, 0xf7)),
            (md::heading_2, Color::Rgb(0x89, 0xb4, 0xfa)),
            (md::heading_3, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::heading_4, Color::Rgb(0xfa, 0xb3, 0x87)),
            (md::heading_other, Color::Rgb(0xb4, 0xbe, 0xfe)),
            (md::heading_underline, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::code_frame, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::code_label, Color::Rgb(0x6c, 0x70, 0x86)),
            (md::inline_code_fg, Color::Rgb(0xfa, 0xb3, 0x87)),
            (md::inline_code_bg, Color::Rgb(0x18, 0x18, 0x25)),
            (md::rule, Color::Rgb(0x45, 0x47, 0x5a)),
            (md::link_icon, Color::Rgb(0x89, 0xdc, 0xeb)),
            (md::link_text, Color::Rgb(0x89, 0xdc, 0xeb)),
            (md::link_hover, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::blockquote_text, Color::Rgb(0xa6, 0xad, 0xc8)),
            (md::text, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (md::latex_inline_bg, Color::Rgb(0x18, 0x18, 0x25)),
            (md::latex_block_fg, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (md::mermaid_keyword, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (md::mermaid_arrow, Color::Rgb(0x89, 0xdc, 0xeb)),
            (md::mermaid_label, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::mermaid_block_fg, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (md::mark_fg, Color::Rgb(0x1e, 0x1e, 0x2e)),
            (md::mark_bg, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (md::task_checked, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::task_unchecked, Color::Rgb(0x6c, 0x70, 0x86)),
            (md::alert_note, Color::Rgb(0x89, 0xb4, 0xfa)),
            (md::alert_tip, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (md::alert_important, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (md::alert_warning, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (md::alert_caution, Color::Rgb(0xf3, 0x8b, 0xa8)),
            (md::footnote_ref, Color::Rgb(0x89, 0xdc, 0xeb)),
            (md::footnote_text, Color::Rgb(0xba, 0xc2, 0xde)),
        ],
    },
    BuiltinTheme {
        name: "dracula",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-eighties.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xbd, 0x93, 0xf9), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x44, 0x47, 0x5a), // toc_border
            selection_bg: Color::Rgb(0xbd, 0x93, 0xf9),           // toc_accent
            sidebar_bg: Color::Rgb(0x19, 0x1a, 0x21),             // toc_bg
            changed_marker_fg: Color::Rgb(0xff, 0xb8, 0x6c),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x62, 0x72, 0xa4),              // status_shortcut_fg
            content_bg: Color::Rgb(0x28, 0x2a, 0x36),             // content_bg
            status_bg: Color::Rgb(0x19, 0x1a, 0x21),              // status_bg
            status_watch_on_fg: Color::Rgb(0x50, 0xfa, 0x7b),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x62, 0x72, 0xa4),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x50, 0xfa, 0x7b),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xff, 0xb8, 0x6c),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x50, 0xfa, 0x7b),        // status_success_fg
            status_behind_fg: Color::Rgb(0xff, 0x55, 0x55),       // status_error_fg
            status_error_fg: Color::Rgb(0xff, 0x55, 0x55),        // status_error_fg
            popup_border_fg: Color::Rgb(0x44, 0x47, 0x5a),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x62, 0x72, 0xa4),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xf1, 0xfa, 0x8c),         // status_search_fg
            popup_match_bg: Color::Rgb(0x42, 0x44, 0x50),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xbd, 0x93, 0xf9),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::search_match_bg, Color::Rgb(0xf1, 0xfa, 0x8c)),
            (md::code_gutter, Color::Rgb(0x62, 0x72, 0xa4)),
            (md::blockquote_marker, Color::Rgb(0x62, 0x72, 0xa4)),
            (md::list_level_1, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::list_level_2, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::list_level_3, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::ordered_list, Color::Rgb(0xff, 0xb8, 0x6c)),
            (md::table_border, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::table_separator, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::table_header, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::table_cell, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (md::heading_1, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::heading_2, Color::Rgb(0xbd, 0x93, 0xf9)),
            (md::heading_3, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::heading_4, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::heading_other, Color::Rgb(0xff, 0xb8, 0x6c)),
            (md::heading_underline, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::code_frame, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::code_label, Color::Rgb(0x62, 0x72, 0xa4)),
            (md::inline_code_fg, Color::Rgb(0xff, 0xb8, 0x6c)),
            (md::inline_code_bg, Color::Rgb(0x21, 0x22, 0x2c)),
            (md::rule, Color::Rgb(0x44, 0x47, 0x5a)),
            (md::link_icon, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::link_text, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::link_hover, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::blockquote_text, Color::Rgb(0x62, 0x72, 0xa4)),
            (md::text, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::latex_inline_bg, Color::Rgb(0x21, 0x22, 0x2c)),
            (md::latex_block_fg, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::mermaid_keyword, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::mermaid_arrow, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::mermaid_label, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::mermaid_block_fg, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (md::mark_fg, Color::Rgb(0x28, 0x2a, 0x36)),
            (md::mark_bg, Color::Rgb(0xf1, 0xfa, 0x8c)),
            (md::task_checked, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::task_unchecked, Color::Rgb(0x62, 0x72, 0xa4)),
            (md::alert_note, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::alert_tip, Color::Rgb(0x50, 0xfa, 0x7b)),
            (md::alert_important, Color::Rgb(0xff, 0x79, 0xc6)),
            (md::alert_warning, Color::Rgb(0xff, 0xb8, 0x6c)),
            (md::alert_caution, Color::Rgb(0xff, 0x55, 0x55)),
            (md::footnote_ref, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (md::footnote_text, Color::Rgb(0xf8, 0xf8, 0xf2)),
        ],
    },
    BuiltinTheme {
        name: "dracula-alucard",
        appearance: Appearance::Light,
        base: MarkdownThemePreset::Arctic,
        syntax: Some("InspiredGitHub"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x64, 0x4a, 0xc9), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0xcf, 0xcf, 0xde), // toc_border
            selection_bg: Color::Rgb(0x64, 0x4a, 0xc9),           // toc_accent
            sidebar_bg: Color::Rgb(0xe2, 0xde, 0xca),             // toc_bg
            changed_marker_fg: Color::Rgb(0xa3, 0x4d, 0x14),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x6c, 0x66, 0x4b),              // status_shortcut_fg
            content_bg: Color::Rgb(0xff, 0xfb, 0xeb),             // content_bg
            status_bg: Color::Rgb(0xe2, 0xde, 0xca),              // status_bg
            status_watch_on_fg: Color::Rgb(0x14, 0x71, 0x0a),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x6c, 0x66, 0x4b),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x14, 0x71, 0x0a),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xa3, 0x4d, 0x14),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x14, 0x71, 0x0a),        // status_success_fg
            status_behind_fg: Color::Rgb(0xcb, 0x3a, 0x2a),       // status_error_fg
            status_error_fg: Color::Rgb(0xcb, 0x3a, 0x2a),        // status_error_fg
            popup_border_fg: Color::Rgb(0xcf, 0xcf, 0xde),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x6c, 0x66, 0x4b),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0x84, 0x6e, 0x15),         // status_search_fg
            popup_match_bg: Color::Rgb(0xec, 0xea, 0xdf),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x64, 0x4a, 0xc9),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::search_match_bg, Color::Rgb(0xf4, 0xec, 0xb8)),
            (md::code_gutter, Color::Rgb(0x6c, 0x66, 0x4b)),
            (md::blockquote_marker, Color::Rgb(0x6c, 0x66, 0x4b)),
            (md::list_level_1, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::list_level_2, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::list_level_3, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::ordered_list, Color::Rgb(0xa3, 0x4d, 0x14)),
            (md::table_border, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::table_separator, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::table_header, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::table_cell, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (md::heading_1, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::heading_2, Color::Rgb(0x64, 0x4a, 0xc9)),
            (md::heading_3, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::heading_4, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::heading_other, Color::Rgb(0xa3, 0x4d, 0x14)),
            (md::heading_underline, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::code_frame, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::code_label, Color::Rgb(0x6c, 0x66, 0x4b)),
            (md::inline_code_fg, Color::Rgb(0xa3, 0x4d, 0x14)),
            (md::inline_code_bg, Color::Rgb(0xec, 0xe9, 0xdc)),
            (md::rule, Color::Rgb(0xcf, 0xcf, 0xde)),
            (md::link_icon, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::link_text, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::link_hover, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::blockquote_text, Color::Rgb(0x6c, 0x66, 0x4b)),
            (md::text, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (md::strong_text, Color::Rgb(0x00, 0x00, 0x00)),
            (md::latex_inline_fg, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::latex_inline_bg, Color::Rgb(0xec, 0xe9, 0xdc)),
            (md::latex_block_fg, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::mermaid_keyword, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::mermaid_arrow, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::mermaid_label, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::mermaid_block_fg, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (md::mark_fg, Color::Rgb(0xff, 0xfb, 0xeb)),
            (md::mark_bg, Color::Rgb(0x84, 0x6e, 0x15)),
            (md::task_checked, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::task_unchecked, Color::Rgb(0x6c, 0x66, 0x4b)),
            (md::alert_note, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::alert_tip, Color::Rgb(0x14, 0x71, 0x0a)),
            (md::alert_important, Color::Rgb(0xa3, 0x14, 0x4d)),
            (md::alert_warning, Color::Rgb(0xa3, 0x4d, 0x14)),
            (md::alert_caution, Color::Rgb(0xcb, 0x3a, 0x2a)),
            (md::footnote_ref, Color::Rgb(0x03, 0x6a, 0x96)),
            (md::footnote_text, Color::Rgb(0x1f, 0x1f, 0x1f)),
        ],
    },
    BuiltinTheme {
        name: "gruvbox",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xfe, 0x80, 0x19), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x50, 0x49, 0x45), // toc_border
            selection_bg: Color::Rgb(0xfe, 0x80, 0x19),           // toc_accent
            sidebar_bg: Color::Rgb(0x1d, 0x20, 0x21),             // toc_bg
            changed_marker_fg: Color::Rgb(0xfa, 0xbd, 0x2f),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x92, 0x83, 0x74),              // status_shortcut_fg
            content_bg: Color::Rgb(0x28, 0x28, 0x28),             // content_bg
            status_bg: Color::Rgb(0x1d, 0x20, 0x21),              // status_bg
            status_watch_on_fg: Color::Rgb(0xb8, 0xbb, 0x26),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x92, 0x83, 0x74),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0xb8, 0xbb, 0x26),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xfa, 0xbd, 0x2f),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0xb8, 0xbb, 0x26),        // status_success_fg
            status_behind_fg: Color::Rgb(0xfb, 0x49, 0x34),       // status_error_fg
            status_error_fg: Color::Rgb(0xfb, 0x49, 0x34),        // status_error_fg
            popup_border_fg: Color::Rgb(0x50, 0x49, 0x45),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x92, 0x83, 0x74),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xfa, 0xbd, 0x2f),         // status_search_fg
            popup_match_bg: Color::Rgb(0x3c, 0x38, 0x36),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xfe, 0x80, 0x19),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x66, 0x5c, 0x54)),
            (md::search_match_bg, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (md::code_gutter, Color::Rgb(0x66, 0x5c, 0x54)),
            (md::blockquote_marker, Color::Rgb(0x92, 0x83, 0x74)),
            (md::list_level_1, Color::Rgb(0xb8, 0xbb, 0x26)),
            (md::list_level_2, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::list_level_3, Color::Rgb(0xd3, 0x86, 0x9b)),
            (md::ordered_list, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (md::table_border, Color::Rgb(0x66, 0x5c, 0x54)),
            (md::table_separator, Color::Rgb(0x50, 0x49, 0x45)),
            (md::table_header, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (md::table_cell, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (md::heading_1, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (md::heading_2, Color::Rgb(0xb8, 0xbb, 0x26)),
            (md::heading_3, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::heading_4, Color::Rgb(0xd3, 0x86, 0x9b)),
            (md::heading_other, Color::Rgb(0xfe, 0x80, 0x19)),
            (md::heading_underline, Color::Rgb(0x50, 0x49, 0x45)),
            (md::code_frame, Color::Rgb(0x66, 0x5c, 0x54)),
            (md::code_label, Color::Rgb(0x92, 0x83, 0x74)),
            (md::inline_code_fg, Color::Rgb(0xfe, 0x80, 0x19)),
            (md::inline_code_bg, Color::Rgb(0x3c, 0x38, 0x36)),
            (md::rule, Color::Rgb(0x50, 0x49, 0x45)),
            (md::link_icon, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::link_text, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::link_hover, Color::Rgb(0xb8, 0xd4, 0xc0)),
            (md::blockquote_text, Color::Rgb(0xd5, 0xc4, 0xa1)),
            (md::text, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (md::strong_text, Color::Rgb(0xfb, 0xf1, 0xc7)),
            (md::latex_inline_fg, Color::Rgb(0xd3, 0x86, 0x9b)),
            (md::latex_inline_bg, Color::Rgb(0x3c, 0x38, 0x36)),
            (md::latex_block_fg, Color::Rgb(0xd3, 0x86, 0x9b)),
            (md::mermaid_keyword, Color::Rgb(0xfe, 0x80, 0x19)),
            (md::mermaid_arrow, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::mermaid_label, Color::Rgb(0xb8, 0xbb, 0x26)),
            (md::mermaid_block_fg, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (md::mark_fg, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (md::mark_bg, Color::Rgb(0x79, 0x74, 0x0e)),
            (md::task_checked, Color::Rgb(0xb8, 0xbb, 0x26)),
            (md::task_unchecked, Color::Rgb(0x92, 0x83, 0x74)),
            (md::alert_note, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::alert_tip, Color::Rgb(0xb8, 0xbb, 0x26)),
            (md::alert_important, Color::Rgb(0xd3, 0x86, 0x9b)),
            (md::alert_warning, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (md::alert_caution, Color::Rgb(0xfb, 0x49, 0x34)),
            (md::footnote_ref, Color::Rgb(0x83, 0xa5, 0x98)),
            (md::footnote_text, Color::Rgb(0xd5, 0xc4, 0xa1)),
        ],
    },
    BuiltinTheme {
        name: "nord",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x88, 0xc0, 0xd0), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x4c, 0x56, 0x6a), // toc_border
            selection_bg: Color::Rgb(0x88, 0xc0, 0xd0),           // toc_accent
            sidebar_bg: Color::Rgb(0x3b, 0x42, 0x52),             // toc_bg
            changed_marker_fg: Color::Rgb(0xeb, 0xcb, 0x8b),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x4c, 0x56, 0x6a),              // status_shortcut_fg
            content_bg: Color::Rgb(0x2e, 0x34, 0x40),             // content_bg
            status_bg: Color::Rgb(0x3b, 0x42, 0x52),              // status_bg
            status_watch_on_fg: Color::Rgb(0xa3, 0xbe, 0x8c),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x4c, 0x56, 0x6a),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0xa3, 0xbe, 0x8c),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xeb, 0xcb, 0x8b),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0xa3, 0xbe, 0x8c),        // status_success_fg
            status_behind_fg: Color::Rgb(0xbf, 0x61, 0x6a),       // status_error_fg
            status_error_fg: Color::Rgb(0xbf, 0x61, 0x6a),        // status_error_fg
            popup_border_fg: Color::Rgb(0x4c, 0x56, 0x6a),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x4c, 0x56, 0x6a),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xeb, 0xcb, 0x8b),         // status_search_fg
            popup_match_bg: Color::Rgb(0x43, 0x4c, 0x5e),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x88, 0xc0, 0xd0),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::search_match_bg, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (md::code_gutter, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::blockquote_marker, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::list_level_1, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (md::list_level_2, Color::Rgb(0x81, 0xa1, 0xc1)),
            (md::list_level_3, Color::Rgb(0xb4, 0x8e, 0xad)),
            (md::ordered_list, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (md::table_border, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::table_separator, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::table_header, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (md::table_cell, Color::Rgb(0xd8, 0xde, 0xe9)),
            (md::heading_1, Color::Rgb(0x88, 0xc0, 0xd0)),
            (md::heading_2, Color::Rgb(0x81, 0xa1, 0xc1)),
            (md::heading_3, Color::Rgb(0x8f, 0xbc, 0xbb)),
            (md::heading_4, Color::Rgb(0xb4, 0x8e, 0xad)),
            (md::heading_other, Color::Rgb(0xd0, 0x87, 0x70)),
            (md::heading_underline, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::code_frame, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::code_label, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::inline_code_fg, Color::Rgb(0xd0, 0x87, 0x70)),
            (md::inline_code_bg, Color::Rgb(0x3b, 0x42, 0x52)),
            (md::rule, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::link_icon, Color::Rgb(0x88, 0xc0, 0xd0)),
            (md::link_text, Color::Rgb(0x88, 0xc0, 0xd0)),
            (md::link_hover, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (md::blockquote_text, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::text, Color::Rgb(0xd8, 0xde, 0xe9)),
            (md::strong_text, Color::Rgb(0xec, 0xef, 0xf4)),
            (md::latex_inline_fg, Color::Rgb(0xb4, 0x8e, 0xad)),
            (md::latex_inline_bg, Color::Rgb(0x3b, 0x42, 0x52)),
            (md::latex_block_fg, Color::Rgb(0xb4, 0x8e, 0xad)),
            (md::mermaid_keyword, Color::Rgb(0xd0, 0x87, 0x70)),
            (md::mermaid_arrow, Color::Rgb(0x88, 0xc0, 0xd0)),
            (md::mermaid_label, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (md::mermaid_block_fg, Color::Rgb(0xd8, 0xde, 0xe9)),
            (md::mark_fg, Color::Rgb(0x2e, 0x34, 0x40)),
            (md::mark_bg, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (md::task_checked, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (md::task_unchecked, Color::Rgb(0x4c, 0x56, 0x6a)),
            (md::alert_note, Color::Rgb(0x81, 0xa1, 0xc1)),
            (md::alert_tip, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (md::alert_important, Color::Rgb(0xb4, 0x8e, 0xad)),
            (md::alert_warning, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (md::alert_caution, Color::Rgb(0xbf, 0x61, 0x6a)),
            (md::footnote_ref, Color::Rgb(0x88, 0xc0, 0xd0)),
            (md::footnote_text, Color::Rgb(0xd8, 0xde, 0xe9)),
        ],
    },
    BuiltinTheme {
        name: "rose-pine",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xeb, 0xbc, 0xba), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x40, 0x3d, 0x52), // toc_border
            selection_bg: Color::Rgb(0xeb, 0xbc, 0xba),           // toc_accent
            sidebar_bg: Color::Rgb(0x1f, 0x1d, 0x2e),             // toc_bg
            changed_marker_fg: Color::Rgb(0xf6, 0xc1, 0x77),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x6e, 0x6a, 0x86),              // status_shortcut_fg
            content_bg: Color::Rgb(0x19, 0x17, 0x24),             // content_bg
            status_bg: Color::Rgb(0x1f, 0x1d, 0x2e),              // status_bg
            status_watch_on_fg: Color::Rgb(0x9c, 0xcf, 0xd8),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x6e, 0x6a, 0x86),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x9c, 0xcf, 0xd8),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xf6, 0xc1, 0x77),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x9c, 0xcf, 0xd8),        // status_success_fg
            status_behind_fg: Color::Rgb(0xeb, 0x6f, 0x92),       // status_error_fg
            status_error_fg: Color::Rgb(0xeb, 0x6f, 0x92),        // status_error_fg
            popup_border_fg: Color::Rgb(0x40, 0x3d, 0x52),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x6e, 0x6a, 0x86),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xf6, 0xc1, 0x77),         // status_search_fg
            popup_match_bg: Color::Rgb(0x26, 0x23, 0x3a),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xeb, 0xbc, 0xba),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x40, 0x3d, 0x52)),
            (md::search_match_bg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::code_gutter, Color::Rgb(0x52, 0x4f, 0x67)),
            (md::blockquote_marker, Color::Rgb(0x6e, 0x6a, 0x86)),
            (md::list_level_1, Color::Rgb(0xeb, 0xbc, 0xba)),
            (md::list_level_2, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::list_level_3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::ordered_list, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::table_border, Color::Rgb(0x40, 0x3d, 0x52)),
            (md::table_separator, Color::Rgb(0x26, 0x23, 0x3a)),
            (md::table_header, Color::Rgb(0xeb, 0xbc, 0xba)),
            (md::table_cell, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::heading_1, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::heading_2, Color::Rgb(0xeb, 0xbc, 0xba)),
            (md::heading_3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::heading_4, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::heading_other, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::heading_underline, Color::Rgb(0x40, 0x3d, 0x52)),
            (md::code_frame, Color::Rgb(0x40, 0x3d, 0x52)),
            (md::code_label, Color::Rgb(0x90, 0x8c, 0xaa)),
            (md::inline_code_fg, Color::Rgb(0xeb, 0xbc, 0xba)),
            (md::inline_code_bg, Color::Rgb(0x26, 0x23, 0x3a)),
            (md::rule, Color::Rgb(0x40, 0x3d, 0x52)),
            (md::link_icon, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::link_text, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::link_hover, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::blockquote_text, Color::Rgb(0x90, 0x8c, 0xaa)),
            (md::text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::strong_text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::latex_inline_fg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::latex_inline_bg, Color::Rgb(0x26, 0x23, 0x3a)),
            (md::latex_block_fg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::mermaid_keyword, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::mermaid_arrow, Color::Rgb(0x31, 0x74, 0x8f)),
            (md::mermaid_label, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::mermaid_block_fg, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::mark_fg, Color::Rgb(0x19, 0x17, 0x24)),
            (md::mark_bg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::task_checked, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::task_unchecked, Color::Rgb(0x6e, 0x6a, 0x86)),
            (md::alert_note, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::alert_tip, Color::Rgb(0x31, 0x74, 0x8f)),
            (md::alert_important, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::alert_warning, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::alert_caution, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::footnote_ref, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::footnote_text, Color::Rgb(0x90, 0x8c, 0xaa)),
        ],
    },
    BuiltinTheme {
        name: "rose-pine-dawn",
        appearance: Appearance::Light,
        base: MarkdownThemePreset::Arctic,
        syntax: Some("InspiredGitHub"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xd7, 0x82, 0x7e), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0xdf, 0xda, 0xd9), // toc_border
            selection_bg: Color::Rgb(0xd7, 0x82, 0x7e),           // toc_accent
            sidebar_bg: Color::Rgb(0xff, 0xfa, 0xf3),             // toc_bg
            changed_marker_fg: Color::Rgb(0xea, 0x9d, 0x34),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x98, 0x93, 0xa5),              // status_shortcut_fg
            content_bg: Color::Rgb(0xfa, 0xf4, 0xed),             // content_bg
            status_bg: Color::Rgb(0xff, 0xfa, 0xf3),              // status_bg
            status_watch_on_fg: Color::Rgb(0x56, 0x94, 0x9f),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x98, 0x93, 0xa5),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x56, 0x94, 0x9f),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xea, 0x9d, 0x34),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x56, 0x94, 0x9f),        // status_success_fg
            status_behind_fg: Color::Rgb(0xb4, 0x63, 0x7a),       // status_error_fg
            status_error_fg: Color::Rgb(0xb4, 0x63, 0x7a),        // status_error_fg
            popup_border_fg: Color::Rgb(0xdf, 0xda, 0xd9),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x98, 0x93, 0xa5),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xea, 0x9d, 0x34),         // status_search_fg
            popup_match_bg: Color::Rgb(0xf2, 0xe9, 0xe1),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xd7, 0x82, 0x7e),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0xdf, 0xda, 0xd9)),
            (md::search_match_bg, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::code_gutter, Color::Rgb(0xce, 0xca, 0xcd)),
            (md::blockquote_marker, Color::Rgb(0x98, 0x93, 0xa5)),
            (md::list_level_1, Color::Rgb(0xd7, 0x82, 0x7e)),
            (md::list_level_2, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::list_level_3, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::ordered_list, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::table_border, Color::Rgb(0xdf, 0xda, 0xd9)),
            (md::table_separator, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (md::table_header, Color::Rgb(0xd7, 0x82, 0x7e)),
            (md::table_cell, Color::Rgb(0x57, 0x52, 0x79)),
            (md::heading_1, Color::Rgb(0xb4, 0x63, 0x7a)),
            (md::heading_2, Color::Rgb(0xd7, 0x82, 0x7e)),
            (md::heading_3, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::heading_4, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::heading_other, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::heading_underline, Color::Rgb(0xdf, 0xda, 0xd9)),
            (md::code_frame, Color::Rgb(0xdf, 0xda, 0xd9)),
            (md::code_label, Color::Rgb(0x79, 0x75, 0x93)),
            (md::inline_code_fg, Color::Rgb(0xd7, 0x82, 0x7e)),
            (md::inline_code_bg, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (md::rule, Color::Rgb(0xdf, 0xda, 0xd9)),
            (md::link_icon, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::link_text, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::link_hover, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::blockquote_text, Color::Rgb(0x79, 0x75, 0x93)),
            (md::text, Color::Rgb(0x57, 0x52, 0x79)),
            (md::strong_text, Color::Rgb(0x57, 0x52, 0x79)),
            (md::latex_inline_fg, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::latex_inline_bg, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (md::latex_block_fg, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::mermaid_keyword, Color::Rgb(0xb4, 0x63, 0x7a)),
            (md::mermaid_arrow, Color::Rgb(0x28, 0x69, 0x83)),
            (md::mermaid_label, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::mermaid_block_fg, Color::Rgb(0x57, 0x52, 0x79)),
            (md::mark_fg, Color::Rgb(0xfa, 0xf4, 0xed)),
            (md::mark_bg, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::task_checked, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::task_unchecked, Color::Rgb(0x98, 0x93, 0xa5)),
            (md::alert_note, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::alert_tip, Color::Rgb(0x28, 0x69, 0x83)),
            (md::alert_important, Color::Rgb(0x90, 0x7a, 0xa9)),
            (md::alert_warning, Color::Rgb(0xea, 0x9d, 0x34)),
            (md::alert_caution, Color::Rgb(0xb4, 0x63, 0x7a)),
            (md::footnote_ref, Color::Rgb(0x56, 0x94, 0x9f)),
            (md::footnote_text, Color::Rgb(0x79, 0x75, 0x93)),
        ],
    },
    BuiltinTheme {
        name: "rose-pine-moon",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0xea, 0x9a, 0x97), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x44, 0x41, 0x5a), // toc_border
            selection_bg: Color::Rgb(0xea, 0x9a, 0x97),           // toc_accent
            sidebar_bg: Color::Rgb(0x2a, 0x27, 0x3f),             // toc_bg
            changed_marker_fg: Color::Rgb(0xf6, 0xc1, 0x77),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x6e, 0x6a, 0x86),              // status_shortcut_fg
            content_bg: Color::Rgb(0x23, 0x21, 0x36),             // content_bg
            status_bg: Color::Rgb(0x2a, 0x27, 0x3f),              // status_bg
            status_watch_on_fg: Color::Rgb(0x9c, 0xcf, 0xd8),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x6e, 0x6a, 0x86),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x9c, 0xcf, 0xd8),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xf6, 0xc1, 0x77),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x9c, 0xcf, 0xd8),        // status_success_fg
            status_behind_fg: Color::Rgb(0xeb, 0x6f, 0x92),       // status_error_fg
            status_error_fg: Color::Rgb(0xeb, 0x6f, 0x92),        // status_error_fg
            popup_border_fg: Color::Rgb(0x44, 0x41, 0x5a),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x6e, 0x6a, 0x86),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xf6, 0xc1, 0x77),         // status_search_fg
            popup_match_bg: Color::Rgb(0x39, 0x35, 0x52),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0xea, 0x9a, 0x97),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x44, 0x41, 0x5a)),
            (md::search_match_bg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::code_gutter, Color::Rgb(0x56, 0x52, 0x6e)),
            (md::blockquote_marker, Color::Rgb(0x6e, 0x6a, 0x86)),
            (md::list_level_1, Color::Rgb(0xea, 0x9a, 0x97)),
            (md::list_level_2, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::list_level_3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::ordered_list, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::table_border, Color::Rgb(0x44, 0x41, 0x5a)),
            (md::table_separator, Color::Rgb(0x39, 0x35, 0x52)),
            (md::table_header, Color::Rgb(0xea, 0x9a, 0x97)),
            (md::table_cell, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::heading_1, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::heading_2, Color::Rgb(0xea, 0x9a, 0x97)),
            (md::heading_3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::heading_4, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::heading_other, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::heading_underline, Color::Rgb(0x44, 0x41, 0x5a)),
            (md::code_frame, Color::Rgb(0x44, 0x41, 0x5a)),
            (md::code_label, Color::Rgb(0x90, 0x8c, 0xaa)),
            (md::inline_code_fg, Color::Rgb(0xea, 0x9a, 0x97)),
            (md::inline_code_bg, Color::Rgb(0x39, 0x35, 0x52)),
            (md::rule, Color::Rgb(0x44, 0x41, 0x5a)),
            (md::link_icon, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::link_text, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::link_hover, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::blockquote_text, Color::Rgb(0x90, 0x8c, 0xaa)),
            (md::text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::strong_text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::latex_inline_fg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::latex_inline_bg, Color::Rgb(0x39, 0x35, 0x52)),
            (md::latex_block_fg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::mermaid_keyword, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::mermaid_arrow, Color::Rgb(0x3e, 0x8f, 0xb0)),
            (md::mermaid_label, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::mermaid_block_fg, Color::Rgb(0xe0, 0xde, 0xf4)),
            (md::mark_fg, Color::Rgb(0x23, 0x21, 0x36)),
            (md::mark_bg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::task_checked, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::task_unchecked, Color::Rgb(0x6e, 0x6a, 0x86)),
            (md::alert_note, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::alert_tip, Color::Rgb(0x3e, 0x8f, 0xb0)),
            (md::alert_important, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (md::alert_warning, Color::Rgb(0xf6, 0xc1, 0x77)),
            (md::alert_caution, Color::Rgb(0xeb, 0x6f, 0x92)),
            (md::footnote_ref, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (md::footnote_text, Color::Rgb(0x90, 0x8c, 0xaa)),
        ],
    },
    BuiltinTheme {
        name: "tokyonight-day",
        appearance: Appearance::Light,
        base: MarkdownThemePreset::Arctic,
        syntax: Some("InspiredGitHub"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x2e, 0x7d, 0xe9), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0xb7, 0xc1, 0xe3), // toc_border
            selection_bg: Color::Rgb(0x2e, 0x7d, 0xe9),           // toc_accent
            sidebar_bg: Color::Rgb(0xd5, 0xd8, 0xe6),             // toc_bg
            changed_marker_fg: Color::Rgb(0xb1, 0x5c, 0x00),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x89, 0x90, 0xb3),              // status_shortcut_fg
            content_bg: Color::Rgb(0xe1, 0xe2, 0xe7),             // content_bg
            status_bg: Color::Rgb(0xd5, 0xd8, 0xe6),              // status_bg
            status_watch_on_fg: Color::Rgb(0x58, 0x75, 0x39),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x89, 0x90, 0xb3),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x58, 0x75, 0x39),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xb1, 0x5c, 0x00),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x58, 0x75, 0x39),        // status_success_fg
            status_behind_fg: Color::Rgb(0xf5, 0x2a, 0x65),       // status_error_fg
            status_error_fg: Color::Rgb(0xf5, 0x2a, 0x65),        // status_error_fg
            popup_border_fg: Color::Rgb(0xb7, 0xc1, 0xe3),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x89, 0x90, 0xb3),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0x8c, 0x6c, 0x3e),         // status_search_fg
            popup_match_bg: Color::Rgb(0xc4, 0xc8, 0xda),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x2e, 0x7d, 0xe9),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::search_match_bg, Color::Rgb(0xf0, 0xe3, 0xb0)),
            (md::code_gutter, Color::Rgb(0x89, 0x90, 0xb3)),
            (md::blockquote_marker, Color::Rgb(0x89, 0x90, 0xb3)),
            (md::list_level_1, Color::Rgb(0x58, 0x75, 0x39)),
            (md::list_level_2, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (md::list_level_3, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::ordered_list, Color::Rgb(0xb1, 0x5c, 0x00)),
            (md::table_border, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::table_separator, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::table_header, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (md::table_cell, Color::Rgb(0x37, 0x60, 0xbf)),
            (md::heading_1, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::heading_2, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (md::heading_3, Color::Rgb(0x00, 0x71, 0x97)),
            (md::heading_4, Color::Rgb(0x58, 0x75, 0x39)),
            (md::heading_other, Color::Rgb(0xb1, 0x5c, 0x00)),
            (md::heading_underline, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::code_frame, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::code_label, Color::Rgb(0x89, 0x90, 0xb3)),
            (md::inline_code_fg, Color::Rgb(0xb1, 0x5c, 0x00)),
            (md::inline_code_bg, Color::Rgb(0xd5, 0xd8, 0xe6)),
            (md::rule, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (md::link_icon, Color::Rgb(0x00, 0x71, 0x97)),
            (md::link_text, Color::Rgb(0x00, 0x71, 0x97)),
            (md::link_hover, Color::Rgb(0x58, 0x75, 0x39)),
            (md::blockquote_text, Color::Rgb(0x61, 0x72, 0xb0)),
            (md::text, Color::Rgb(0x37, 0x60, 0xbf)),
            (md::strong_text, Color::Rgb(0x24, 0x30, 0x5e)),
            (md::latex_inline_fg, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::latex_inline_bg, Color::Rgb(0xd5, 0xd8, 0xe6)),
            (md::latex_block_fg, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::mermaid_keyword, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::mermaid_arrow, Color::Rgb(0x00, 0x71, 0x97)),
            (md::mermaid_label, Color::Rgb(0x58, 0x75, 0x39)),
            (md::mermaid_block_fg, Color::Rgb(0x37, 0x60, 0xbf)),
            (md::mark_fg, Color::Rgb(0xe1, 0xe2, 0xe7)),
            (md::mark_bg, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (md::task_checked, Color::Rgb(0x58, 0x75, 0x39)),
            (md::task_unchecked, Color::Rgb(0x89, 0x90, 0xb3)),
            (md::alert_note, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (md::alert_tip, Color::Rgb(0x58, 0x75, 0x39)),
            (md::alert_important, Color::Rgb(0x98, 0x54, 0xf1)),
            (md::alert_warning, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (md::alert_caution, Color::Rgb(0xf5, 0x2a, 0x65)),
            (md::footnote_ref, Color::Rgb(0x00, 0x71, 0x97)),
            (md::footnote_text, Color::Rgb(0x61, 0x72, 0xb0)),
        ],
    },
    BuiltinTheme {
        name: "tokyonight-night",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x7a, 0xa2, 0xf7), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x54, 0x5c, 0x7e), // toc_border
            selection_bg: Color::Rgb(0x7a, 0xa2, 0xf7),           // toc_accent
            sidebar_bg: Color::Rgb(0x16, 0x16, 0x1e),             // toc_bg
            changed_marker_fg: Color::Rgb(0xff, 0x9e, 0x64),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x56, 0x5f, 0x89),              // status_shortcut_fg
            content_bg: Color::Rgb(0x1a, 0x1b, 0x26),             // content_bg
            status_bg: Color::Rgb(0x16, 0x16, 0x1e),              // status_bg
            status_watch_on_fg: Color::Rgb(0x9e, 0xce, 0x6a),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x56, 0x5f, 0x89),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x9e, 0xce, 0x6a),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xff, 0x9e, 0x64),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x9e, 0xce, 0x6a),        // status_success_fg
            status_behind_fg: Color::Rgb(0xf7, 0x76, 0x8e),       // status_error_fg
            status_error_fg: Color::Rgb(0xf7, 0x76, 0x8e),        // status_error_fg
            popup_border_fg: Color::Rgb(0x54, 0x5c, 0x7e),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x56, 0x5f, 0x89),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xe0, 0xaf, 0x68),         // status_search_fg
            popup_match_bg: Color::Rgb(0x29, 0x2e, 0x42),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x7a, 0xa2, 0xf7),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::search_match_bg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::code_gutter, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::blockquote_marker, Color::Rgb(0x56, 0x5f, 0x89)),
            (md::list_level_1, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::list_level_2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::list_level_3, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::ordered_list, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::table_border, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::table_separator, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::table_header, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::table_cell, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::heading_1, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::heading_2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::heading_3, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::heading_4, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::heading_other, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::heading_underline, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::code_frame, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::code_label, Color::Rgb(0x56, 0x5f, 0x89)),
            (md::inline_code_fg, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::inline_code_bg, Color::Rgb(0x16, 0x16, 0x1e)),
            (md::rule, Color::Rgb(0x54, 0x5c, 0x7e)),
            (md::link_icon, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::link_text, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::link_hover, Color::Rgb(0x73, 0xda, 0xca)),
            (md::blockquote_text, Color::Rgb(0xa9, 0xb1, 0xd6)),
            (md::text, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::latex_inline_bg, Color::Rgb(0x16, 0x16, 0x1e)),
            (md::latex_block_fg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::mermaid_keyword, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::mermaid_arrow, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::mermaid_label, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::mermaid_block_fg, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::mark_fg, Color::Rgb(0x1a, 0x1b, 0x26)),
            (md::mark_bg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::task_checked, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::task_unchecked, Color::Rgb(0x56, 0x5f, 0x89)),
            (md::alert_note, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::alert_tip, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::alert_important, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::alert_warning, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::alert_caution, Color::Rgb(0xf7, 0x76, 0x8e)),
            (md::footnote_ref, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::footnote_text, Color::Rgb(0xa9, 0xb1, 0xd6)),
        ],
    },
    BuiltinTheme {
        name: "tokyonight-storm",
        appearance: Appearance::Dark,
        base: MarkdownThemePreset::OceanDark,
        syntax: Some("base16-ocean.dark"),
        ui: UiTheme {
            sidebar_border_focused: Color::Rgb(0x7a, 0xa2, 0xf7), // toc_accent
            sidebar_border_unfocused: Color::Rgb(0x41, 0x48, 0x68), // toc_border
            selection_bg: Color::Rgb(0x7a, 0xa2, 0xf7),           // toc_accent
            sidebar_bg: Color::Rgb(0x1f, 0x23, 0x35),             // toc_bg
            changed_marker_fg: Color::Rgb(0xff, 0x9e, 0x64),      // status_warning_fg
            dimmed_fg: Color::Rgb(0x73, 0x7a, 0xa2),              // status_shortcut_fg
            content_bg: Color::Rgb(0x24, 0x28, 0x3b),             // content_bg
            status_bg: Color::Rgb(0x1f, 0x23, 0x35),              // status_bg
            status_watch_on_fg: Color::Rgb(0x9e, 0xce, 0x6a),     // status_watch_fg
            status_watch_off_fg: Color::Rgb(0x73, 0x7a, 0xa2),    // status_shortcut_fg
            status_success_fg: Color::Rgb(0x9e, 0xce, 0x6a),      // status_success_fg
            status_dirty_fg: Color::Rgb(0xff, 0x9e, 0x64),        // status_warning_fg
            status_ahead_fg: Color::Rgb(0x9e, 0xce, 0x6a),        // status_success_fg
            status_behind_fg: Color::Rgb(0xf7, 0x76, 0x8e),       // status_error_fg
            status_error_fg: Color::Rgb(0xf7, 0x76, 0x8e),        // status_error_fg
            popup_border_fg: Color::Rgb(0x41, 0x48, 0x68),        // toc_border
            popup_dimmed_fg: Color::Rgb(0x73, 0x7a, 0xa2),        // status_shortcut_fg
            popup_match_fg: Color::Rgb(0xe0, 0xaf, 0x68),         // status_search_fg
            popup_match_bg: Color::Rgb(0x29, 0x2e, 0x42),         // status_search_bg
            scrollbar_hover_fg: Color::Rgb(0x7a, 0xa2, 0xf7),     // toc_accent
        },
        md: &[
            (md::search_highlight_bg, Color::Rgb(0x41, 0x48, 0x68)),
            (md::search_match_bg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::code_gutter, Color::Rgb(0x41, 0x48, 0x68)),
            (md::blockquote_marker, Color::Rgb(0x73, 0x7a, 0xa2)),
            (md::list_level_1, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::list_level_2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::list_level_3, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::ordered_list, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::table_border, Color::Rgb(0x41, 0x48, 0x68)),
            (md::table_separator, Color::Rgb(0x41, 0x48, 0x68)),
            (md::table_header, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::table_cell, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::heading_1, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::heading_2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::heading_3, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::heading_4, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::heading_other, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::heading_underline, Color::Rgb(0x41, 0x48, 0x68)),
            (md::code_frame, Color::Rgb(0x41, 0x48, 0x68)),
            (md::code_label, Color::Rgb(0x73, 0x7a, 0xa2)),
            (md::inline_code_fg, Color::Rgb(0xff, 0x9e, 0x64)),
            (md::inline_code_bg, Color::Rgb(0x1f, 0x23, 0x35)),
            (md::rule, Color::Rgb(0x41, 0x48, 0x68)),
            (md::link_icon, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::link_text, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::link_hover, Color::Rgb(0x73, 0xda, 0xca)),
            (md::blockquote_text, Color::Rgb(0xa9, 0xb1, 0xd6)),
            (md::text, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::strong_text, Color::Rgb(0xff, 0xff, 0xff)),
            (md::latex_inline_fg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::latex_inline_bg, Color::Rgb(0x1f, 0x23, 0x35)),
            (md::latex_block_fg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::mermaid_keyword, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::mermaid_arrow, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::mermaid_label, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::mermaid_block_fg, Color::Rgb(0xc0, 0xca, 0xf5)),
            (md::mark_fg, Color::Rgb(0x24, 0x28, 0x3b)),
            (md::mark_bg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::task_checked, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::task_unchecked, Color::Rgb(0x73, 0x7a, 0xa2)),
            (md::alert_note, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (md::alert_tip, Color::Rgb(0x9e, 0xce, 0x6a)),
            (md::alert_important, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (md::alert_warning, Color::Rgb(0xe0, 0xaf, 0x68)),
            (md::alert_caution, Color::Rgb(0xf7, 0x76, 0x8e)),
            (md::footnote_ref, Color::Rgb(0x7d, 0xcf, 0xff)),
            (md::footnote_text, Color::Rgb(0xa9, 0xb1, 0xd6)),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_in_cycle_order() {
        let names: Vec<&str> = BUILTIN_THEMES.iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "alphabetical — also the Shift+T order");
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "no duplicate names");
    }

    #[test]
    fn every_theme_resolves_by_name_to_itself() {
        for spec in BUILTIN_THEMES {
            let resolved = crate::theme::resolve_theme(spec.name, &Default::default())
                .unwrap_or_else(|err| panic!("{} did not resolve: {err}", spec.name));
            assert_eq!(resolved.theme.name, spec.name);
            assert_eq!(resolved.theme.appearance, spec.appearance, "{}", spec.name);
            assert_eq!(resolved.theme.ui, spec.ui, "{}", spec.name);
            assert!(
                resolved.warnings.is_empty(),
                "{}: {:?}",
                spec.name,
                resolved.warnings
            );
        }
    }

    #[test]
    fn markdown_overrides_are_applied_over_the_base_preset() {
        for spec in BUILTIN_THEMES {
            let mut expected = engine::markdown_theme_by_preset(spec.base);
            for (key, color) in spec.md {
                key.apply(&mut expected, *color);
            }
            let resolved = crate::theme::resolve_theme(spec.name, &Default::default()).unwrap();
            assert_eq!(resolved.theme.markdown, expected, "{}", spec.name);
        }
    }

    #[test]
    fn no_markdown_key_is_overridden_twice() {
        for spec in BUILTIN_THEMES {
            let mut seen = std::collections::HashSet::new();
            for (key, _) in spec.md {
                assert!(
                    seen.insert(key.name),
                    "{}: a markdown key appears twice",
                    spec.name
                );
            }
        }
    }
}
