//! The 14 built-in themes: the leaf theme files from the user's dotfiles
//! (`~/.config/leaf/themes/*.toml`, symlinked from
//! `~/.dotfiles/leaf/.config/leaf/themes/`), ported into an in-binary table.
//!
//! Port rules (per file): `base` and `syntax` kept; the `[markdown]` section
//! copied verbatim (field names are the engine's `MarkdownTheme`); the
//! `[ui]` keys mapped onto our `UiTheme` vocabulary:
//! `toc_accent` → sidebar_border_focused + selection_bg,
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

/// One `[markdown]` field name, so a ported theme can store its palette as
/// `(key, color)` overrides applied over the base preset (the ported files
/// happen to override every field, but `base` stays the inheritance root).
#[derive(Clone, Copy)]
pub(crate) enum MdKey {
    SearchHighlightBg,
    SearchMatchBg,
    CodeGutter,
    BlockquoteMarker,
    ListLevel1,
    ListLevel2,
    ListLevel3,
    OrderedList,
    TableBorder,
    TableSeparator,
    TableHeader,
    TableCell,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    HeadingOther,
    HeadingUnderline,
    CodeFrame,
    CodeLabel,
    InlineCodeFg,
    InlineCodeBg,
    Rule,
    LinkIcon,
    LinkText,
    LinkHover,
    BlockquoteText,
    Text,
    StrongText,
    LatexInlineFg,
    LatexInlineBg,
    LatexBlockFg,
    MermaidKeyword,
    MermaidArrow,
    MermaidLabel,
    MermaidBlockFg,
    MarkFg,
    MarkBg,
    TaskChecked,
    TaskUnchecked,
    AlertNote,
    AlertTip,
    AlertImportant,
    AlertWarning,
    AlertCaution,
    FootnoteRef,
    FootnoteText,
}

impl MdKey {
    pub(crate) fn apply(self, md: &mut MarkdownTheme, color: Color) {
        match self {
            MdKey::SearchHighlightBg => md.search_highlight_bg = color,
            MdKey::SearchMatchBg => md.search_match_bg = color,
            MdKey::CodeGutter => md.code_gutter = color,
            MdKey::BlockquoteMarker => md.blockquote_marker = color,
            MdKey::ListLevel1 => md.list_level_1 = color,
            MdKey::ListLevel2 => md.list_level_2 = color,
            MdKey::ListLevel3 => md.list_level_3 = color,
            MdKey::OrderedList => md.ordered_list = color,
            MdKey::TableBorder => md.table_border = color,
            MdKey::TableSeparator => md.table_separator = color,
            MdKey::TableHeader => md.table_header = color,
            MdKey::TableCell => md.table_cell = color,
            MdKey::Heading1 => md.heading_1 = color,
            MdKey::Heading2 => md.heading_2 = color,
            MdKey::Heading3 => md.heading_3 = color,
            MdKey::Heading4 => md.heading_4 = color,
            MdKey::HeadingOther => md.heading_other = color,
            MdKey::HeadingUnderline => md.heading_underline = color,
            MdKey::CodeFrame => md.code_frame = color,
            MdKey::CodeLabel => md.code_label = color,
            MdKey::InlineCodeFg => md.inline_code_fg = color,
            MdKey::InlineCodeBg => md.inline_code_bg = color,
            MdKey::Rule => md.rule = color,
            MdKey::LinkIcon => md.link_icon = color,
            MdKey::LinkText => md.link_text = color,
            MdKey::LinkHover => md.link_hover = color,
            MdKey::BlockquoteText => md.blockquote_text = color,
            MdKey::Text => md.text = color,
            MdKey::StrongText => md.strong_text = color,
            MdKey::LatexInlineFg => md.latex_inline_fg = color,
            MdKey::LatexInlineBg => md.latex_inline_bg = color,
            MdKey::LatexBlockFg => md.latex_block_fg = color,
            MdKey::MermaidKeyword => md.mermaid_keyword = color,
            MdKey::MermaidArrow => md.mermaid_arrow = color,
            MdKey::MermaidLabel => md.mermaid_label = color,
            MdKey::MermaidBlockFg => md.mermaid_block_fg = color,
            MdKey::MarkFg => md.mark_fg = color,
            MdKey::MarkBg => md.mark_bg = color,
            MdKey::TaskChecked => md.task_checked = color,
            MdKey::TaskUnchecked => md.task_unchecked = color,
            MdKey::AlertNote => md.alert_note = color,
            MdKey::AlertTip => md.alert_tip = color,
            MdKey::AlertImportant => md.alert_important = color,
            MdKey::AlertWarning => md.alert_warning = color,
            MdKey::AlertCaution => md.alert_caution = color,
            MdKey::FootnoteRef => md.footnote_ref = color,
            MdKey::FootnoteText => md.footnote_text = color,
        }
    }
}

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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::SearchMatchBg, Color::Rgb(0xe5, 0xc8, 0x90)),
            (MdKey::CodeGutter, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x73, 0x79, 0x94)),
            (MdKey::ListLevel1, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::ListLevel2, Color::Rgb(0x8c, 0xaa, 0xee)),
            (MdKey::ListLevel3, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (MdKey::OrderedList, Color::Rgb(0xef, 0x9f, 0x76)),
            (MdKey::TableBorder, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::TableSeparator, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::TableHeader, Color::Rgb(0xe5, 0xc8, 0x90)),
            (MdKey::TableCell, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (MdKey::Heading1, Color::Rgb(0xca, 0x9e, 0xe6)),
            (MdKey::Heading2, Color::Rgb(0x8c, 0xaa, 0xee)),
            (MdKey::Heading3, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::Heading4, Color::Rgb(0xef, 0x9f, 0x76)),
            (MdKey::HeadingOther, Color::Rgb(0xba, 0xbb, 0xf1)),
            (MdKey::HeadingUnderline, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::CodeFrame, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::CodeLabel, Color::Rgb(0x73, 0x79, 0x94)),
            (MdKey::InlineCodeFg, Color::Rgb(0xef, 0x9f, 0x76)),
            (MdKey::InlineCodeBg, Color::Rgb(0x29, 0x2c, 0x3c)),
            (MdKey::Rule, Color::Rgb(0x51, 0x57, 0x6d)),
            (MdKey::LinkIcon, Color::Rgb(0x99, 0xd1, 0xdb)),
            (MdKey::LinkText, Color::Rgb(0x99, 0xd1, 0xdb)),
            (MdKey::LinkHover, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::BlockquoteText, Color::Rgb(0xa5, 0xad, 0xce)),
            (MdKey::Text, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (MdKey::LatexInlineBg, Color::Rgb(0x29, 0x2c, 0x3c)),
            (MdKey::LatexBlockFg, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (MdKey::MermaidKeyword, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (MdKey::MermaidArrow, Color::Rgb(0x99, 0xd1, 0xdb)),
            (MdKey::MermaidLabel, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xc6, 0xd0, 0xf5)),
            (MdKey::MarkFg, Color::Rgb(0x30, 0x34, 0x46)),
            (MdKey::MarkBg, Color::Rgb(0xe5, 0xc8, 0x90)),
            (MdKey::TaskChecked, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::TaskUnchecked, Color::Rgb(0x73, 0x79, 0x94)),
            (MdKey::AlertNote, Color::Rgb(0x8c, 0xaa, 0xee)),
            (MdKey::AlertTip, Color::Rgb(0xa6, 0xd1, 0x89)),
            (MdKey::AlertImportant, Color::Rgb(0xf4, 0xb8, 0xe4)),
            (MdKey::AlertWarning, Color::Rgb(0xe5, 0xc8, 0x90)),
            (MdKey::AlertCaution, Color::Rgb(0xe7, 0x82, 0x84)),
            (MdKey::FootnoteRef, Color::Rgb(0x99, 0xd1, 0xdb)),
            (MdKey::FootnoteText, Color::Rgb(0xb5, 0xbf, 0xe2)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf0, 0xe3, 0xb0)),
            (MdKey::CodeGutter, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (MdKey::ListLevel1, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::ListLevel2, Color::Rgb(0x1e, 0x66, 0xf5)),
            (MdKey::ListLevel3, Color::Rgb(0xea, 0x76, 0xcb)),
            (MdKey::OrderedList, Color::Rgb(0xfe, 0x64, 0x0b)),
            (MdKey::TableBorder, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::TableSeparator, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::TableHeader, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (MdKey::TableCell, Color::Rgb(0x4c, 0x4f, 0x69)),
            (MdKey::Heading1, Color::Rgb(0x88, 0x39, 0xef)),
            (MdKey::Heading2, Color::Rgb(0x1e, 0x66, 0xf5)),
            (MdKey::Heading3, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::Heading4, Color::Rgb(0xfe, 0x64, 0x0b)),
            (MdKey::HeadingOther, Color::Rgb(0x72, 0x87, 0xfd)),
            (MdKey::HeadingUnderline, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::CodeFrame, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::CodeLabel, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (MdKey::InlineCodeFg, Color::Rgb(0xfe, 0x64, 0x0b)),
            (MdKey::InlineCodeBg, Color::Rgb(0xdc, 0xe0, 0xe8)),
            (MdKey::Rule, Color::Rgb(0xbc, 0xc0, 0xcc)),
            (MdKey::LinkIcon, Color::Rgb(0x04, 0xa5, 0xe5)),
            (MdKey::LinkText, Color::Rgb(0x04, 0xa5, 0xe5)),
            (MdKey::LinkHover, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::BlockquoteText, Color::Rgb(0x6c, 0x6f, 0x85)),
            (MdKey::Text, Color::Rgb(0x4c, 0x4f, 0x69)),
            (MdKey::StrongText, Color::Rgb(0x1d, 0x1e, 0x2b)),
            (MdKey::LatexInlineFg, Color::Rgb(0xea, 0x76, 0xcb)),
            (MdKey::LatexInlineBg, Color::Rgb(0xdc, 0xe0, 0xe8)),
            (MdKey::LatexBlockFg, Color::Rgb(0xea, 0x76, 0xcb)),
            (MdKey::MermaidKeyword, Color::Rgb(0xea, 0x76, 0xcb)),
            (MdKey::MermaidArrow, Color::Rgb(0x04, 0xa5, 0xe5)),
            (MdKey::MermaidLabel, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::MermaidBlockFg, Color::Rgb(0x4c, 0x4f, 0x69)),
            (MdKey::MarkFg, Color::Rgb(0xef, 0xf1, 0xf5)),
            (MdKey::MarkBg, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (MdKey::TaskChecked, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::TaskUnchecked, Color::Rgb(0x9c, 0xa0, 0xb0)),
            (MdKey::AlertNote, Color::Rgb(0x1e, 0x66, 0xf5)),
            (MdKey::AlertTip, Color::Rgb(0x40, 0xa0, 0x2b)),
            (MdKey::AlertImportant, Color::Rgb(0xea, 0x76, 0xcb)),
            (MdKey::AlertWarning, Color::Rgb(0xdf, 0x8e, 0x1d)),
            (MdKey::AlertCaution, Color::Rgb(0xd2, 0x0f, 0x39)),
            (MdKey::FootnoteRef, Color::Rgb(0x04, 0xa5, 0xe5)),
            (MdKey::FootnoteText, Color::Rgb(0x5c, 0x5f, 0x77)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::SearchMatchBg, Color::Rgb(0xee, 0xd4, 0x9f)),
            (MdKey::CodeGutter, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x6e, 0x73, 0x8d)),
            (MdKey::ListLevel1, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::ListLevel2, Color::Rgb(0x8a, 0xad, 0xf4)),
            (MdKey::ListLevel3, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (MdKey::OrderedList, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (MdKey::TableBorder, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::TableSeparator, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::TableHeader, Color::Rgb(0xee, 0xd4, 0x9f)),
            (MdKey::TableCell, Color::Rgb(0xca, 0xd3, 0xf5)),
            (MdKey::Heading1, Color::Rgb(0xc6, 0xa0, 0xf6)),
            (MdKey::Heading2, Color::Rgb(0x8a, 0xad, 0xf4)),
            (MdKey::Heading3, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::Heading4, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (MdKey::HeadingOther, Color::Rgb(0xb7, 0xbd, 0xf8)),
            (MdKey::HeadingUnderline, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::CodeFrame, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::CodeLabel, Color::Rgb(0x6e, 0x73, 0x8d)),
            (MdKey::InlineCodeFg, Color::Rgb(0xf5, 0xa9, 0x7f)),
            (MdKey::InlineCodeBg, Color::Rgb(0x1e, 0x20, 0x30)),
            (MdKey::Rule, Color::Rgb(0x49, 0x4d, 0x64)),
            (MdKey::LinkIcon, Color::Rgb(0x91, 0xd7, 0xe3)),
            (MdKey::LinkText, Color::Rgb(0x91, 0xd7, 0xe3)),
            (MdKey::LinkHover, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::BlockquoteText, Color::Rgb(0xa5, 0xad, 0xcb)),
            (MdKey::Text, Color::Rgb(0xca, 0xd3, 0xf5)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (MdKey::LatexInlineBg, Color::Rgb(0x1e, 0x20, 0x30)),
            (MdKey::LatexBlockFg, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (MdKey::MermaidKeyword, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (MdKey::MermaidArrow, Color::Rgb(0x91, 0xd7, 0xe3)),
            (MdKey::MermaidLabel, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xca, 0xd3, 0xf5)),
            (MdKey::MarkFg, Color::Rgb(0x24, 0x27, 0x3a)),
            (MdKey::MarkBg, Color::Rgb(0xee, 0xd4, 0x9f)),
            (MdKey::TaskChecked, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::TaskUnchecked, Color::Rgb(0x6e, 0x73, 0x8d)),
            (MdKey::AlertNote, Color::Rgb(0x8a, 0xad, 0xf4)),
            (MdKey::AlertTip, Color::Rgb(0xa6, 0xda, 0x95)),
            (MdKey::AlertImportant, Color::Rgb(0xf5, 0xbd, 0xe6)),
            (MdKey::AlertWarning, Color::Rgb(0xee, 0xd4, 0x9f)),
            (MdKey::AlertCaution, Color::Rgb(0xed, 0x87, 0x96)),
            (MdKey::FootnoteRef, Color::Rgb(0x91, 0xd7, 0xe3)),
            (MdKey::FootnoteText, Color::Rgb(0xb8, 0xc0, 0xe0)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (MdKey::CodeGutter, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x6c, 0x70, 0x86)),
            (MdKey::ListLevel1, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::ListLevel2, Color::Rgb(0x89, 0xb4, 0xfa)),
            (MdKey::ListLevel3, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (MdKey::OrderedList, Color::Rgb(0xfa, 0xb3, 0x87)),
            (MdKey::TableBorder, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::TableSeparator, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::TableHeader, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (MdKey::TableCell, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (MdKey::Heading1, Color::Rgb(0xcb, 0xa6, 0xf7)),
            (MdKey::Heading2, Color::Rgb(0x89, 0xb4, 0xfa)),
            (MdKey::Heading3, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::Heading4, Color::Rgb(0xfa, 0xb3, 0x87)),
            (MdKey::HeadingOther, Color::Rgb(0xb4, 0xbe, 0xfe)),
            (MdKey::HeadingUnderline, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::CodeFrame, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::CodeLabel, Color::Rgb(0x6c, 0x70, 0x86)),
            (MdKey::InlineCodeFg, Color::Rgb(0xfa, 0xb3, 0x87)),
            (MdKey::InlineCodeBg, Color::Rgb(0x18, 0x18, 0x25)),
            (MdKey::Rule, Color::Rgb(0x45, 0x47, 0x5a)),
            (MdKey::LinkIcon, Color::Rgb(0x89, 0xdc, 0xeb)),
            (MdKey::LinkText, Color::Rgb(0x89, 0xdc, 0xeb)),
            (MdKey::LinkHover, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::BlockquoteText, Color::Rgb(0xa6, 0xad, 0xc8)),
            (MdKey::Text, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (MdKey::LatexInlineBg, Color::Rgb(0x18, 0x18, 0x25)),
            (MdKey::LatexBlockFg, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (MdKey::MermaidKeyword, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (MdKey::MermaidArrow, Color::Rgb(0x89, 0xdc, 0xeb)),
            (MdKey::MermaidLabel, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xcd, 0xd6, 0xf4)),
            (MdKey::MarkFg, Color::Rgb(0x1e, 0x1e, 0x2e)),
            (MdKey::MarkBg, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (MdKey::TaskChecked, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::TaskUnchecked, Color::Rgb(0x6c, 0x70, 0x86)),
            (MdKey::AlertNote, Color::Rgb(0x89, 0xb4, 0xfa)),
            (MdKey::AlertTip, Color::Rgb(0xa6, 0xe3, 0xa1)),
            (MdKey::AlertImportant, Color::Rgb(0xf5, 0xc2, 0xe7)),
            (MdKey::AlertWarning, Color::Rgb(0xf9, 0xe2, 0xaf)),
            (MdKey::AlertCaution, Color::Rgb(0xf3, 0x8b, 0xa8)),
            (MdKey::FootnoteRef, Color::Rgb(0x89, 0xdc, 0xeb)),
            (MdKey::FootnoteText, Color::Rgb(0xba, 0xc2, 0xde)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf1, 0xfa, 0x8c)),
            (MdKey::CodeGutter, Color::Rgb(0x62, 0x72, 0xa4)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x62, 0x72, 0xa4)),
            (MdKey::ListLevel1, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::ListLevel2, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::ListLevel3, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::OrderedList, Color::Rgb(0xff, 0xb8, 0x6c)),
            (MdKey::TableBorder, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::TableSeparator, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::TableHeader, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::TableCell, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (MdKey::Heading1, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::Heading2, Color::Rgb(0xbd, 0x93, 0xf9)),
            (MdKey::Heading3, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::Heading4, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::HeadingOther, Color::Rgb(0xff, 0xb8, 0x6c)),
            (MdKey::HeadingUnderline, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::CodeFrame, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::CodeLabel, Color::Rgb(0x62, 0x72, 0xa4)),
            (MdKey::InlineCodeFg, Color::Rgb(0xff, 0xb8, 0x6c)),
            (MdKey::InlineCodeBg, Color::Rgb(0x21, 0x22, 0x2c)),
            (MdKey::Rule, Color::Rgb(0x44, 0x47, 0x5a)),
            (MdKey::LinkIcon, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::LinkText, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::LinkHover, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::BlockquoteText, Color::Rgb(0x62, 0x72, 0xa4)),
            (MdKey::Text, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::LatexInlineBg, Color::Rgb(0x21, 0x22, 0x2c)),
            (MdKey::LatexBlockFg, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::MermaidKeyword, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::MermaidArrow, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::MermaidLabel, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xf8, 0xf8, 0xf2)),
            (MdKey::MarkFg, Color::Rgb(0x28, 0x2a, 0x36)),
            (MdKey::MarkBg, Color::Rgb(0xf1, 0xfa, 0x8c)),
            (MdKey::TaskChecked, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::TaskUnchecked, Color::Rgb(0x62, 0x72, 0xa4)),
            (MdKey::AlertNote, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::AlertTip, Color::Rgb(0x50, 0xfa, 0x7b)),
            (MdKey::AlertImportant, Color::Rgb(0xff, 0x79, 0xc6)),
            (MdKey::AlertWarning, Color::Rgb(0xff, 0xb8, 0x6c)),
            (MdKey::AlertCaution, Color::Rgb(0xff, 0x55, 0x55)),
            (MdKey::FootnoteRef, Color::Rgb(0x8b, 0xe9, 0xfd)),
            (MdKey::FootnoteText, Color::Rgb(0xf8, 0xf8, 0xf2)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf4, 0xec, 0xb8)),
            (MdKey::CodeGutter, Color::Rgb(0x6c, 0x66, 0x4b)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x6c, 0x66, 0x4b)),
            (MdKey::ListLevel1, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::ListLevel2, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::ListLevel3, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::OrderedList, Color::Rgb(0xa3, 0x4d, 0x14)),
            (MdKey::TableBorder, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::TableSeparator, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::TableHeader, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::TableCell, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (MdKey::Heading1, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::Heading2, Color::Rgb(0x64, 0x4a, 0xc9)),
            (MdKey::Heading3, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::Heading4, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::HeadingOther, Color::Rgb(0xa3, 0x4d, 0x14)),
            (MdKey::HeadingUnderline, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::CodeFrame, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::CodeLabel, Color::Rgb(0x6c, 0x66, 0x4b)),
            (MdKey::InlineCodeFg, Color::Rgb(0xa3, 0x4d, 0x14)),
            (MdKey::InlineCodeBg, Color::Rgb(0xec, 0xe9, 0xdc)),
            (MdKey::Rule, Color::Rgb(0xcf, 0xcf, 0xde)),
            (MdKey::LinkIcon, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::LinkText, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::LinkHover, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::BlockquoteText, Color::Rgb(0x6c, 0x66, 0x4b)),
            (MdKey::Text, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (MdKey::StrongText, Color::Rgb(0x00, 0x00, 0x00)),
            (MdKey::LatexInlineFg, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::LatexInlineBg, Color::Rgb(0xec, 0xe9, 0xdc)),
            (MdKey::LatexBlockFg, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::MermaidKeyword, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::MermaidArrow, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::MermaidLabel, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::MermaidBlockFg, Color::Rgb(0x1f, 0x1f, 0x1f)),
            (MdKey::MarkFg, Color::Rgb(0xff, 0xfb, 0xeb)),
            (MdKey::MarkBg, Color::Rgb(0x84, 0x6e, 0x15)),
            (MdKey::TaskChecked, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::TaskUnchecked, Color::Rgb(0x6c, 0x66, 0x4b)),
            (MdKey::AlertNote, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::AlertTip, Color::Rgb(0x14, 0x71, 0x0a)),
            (MdKey::AlertImportant, Color::Rgb(0xa3, 0x14, 0x4d)),
            (MdKey::AlertWarning, Color::Rgb(0xa3, 0x4d, 0x14)),
            (MdKey::AlertCaution, Color::Rgb(0xcb, 0x3a, 0x2a)),
            (MdKey::FootnoteRef, Color::Rgb(0x03, 0x6a, 0x96)),
            (MdKey::FootnoteText, Color::Rgb(0x1f, 0x1f, 0x1f)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x66, 0x5c, 0x54)),
            (MdKey::SearchMatchBg, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (MdKey::CodeGutter, Color::Rgb(0x66, 0x5c, 0x54)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x92, 0x83, 0x74)),
            (MdKey::ListLevel1, Color::Rgb(0xb8, 0xbb, 0x26)),
            (MdKey::ListLevel2, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::ListLevel3, Color::Rgb(0xd3, 0x86, 0x9b)),
            (MdKey::OrderedList, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (MdKey::TableBorder, Color::Rgb(0x66, 0x5c, 0x54)),
            (MdKey::TableSeparator, Color::Rgb(0x50, 0x49, 0x45)),
            (MdKey::TableHeader, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (MdKey::TableCell, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (MdKey::Heading1, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (MdKey::Heading2, Color::Rgb(0xb8, 0xbb, 0x26)),
            (MdKey::Heading3, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::Heading4, Color::Rgb(0xd3, 0x86, 0x9b)),
            (MdKey::HeadingOther, Color::Rgb(0xfe, 0x80, 0x19)),
            (MdKey::HeadingUnderline, Color::Rgb(0x50, 0x49, 0x45)),
            (MdKey::CodeFrame, Color::Rgb(0x66, 0x5c, 0x54)),
            (MdKey::CodeLabel, Color::Rgb(0x92, 0x83, 0x74)),
            (MdKey::InlineCodeFg, Color::Rgb(0xfe, 0x80, 0x19)),
            (MdKey::InlineCodeBg, Color::Rgb(0x3c, 0x38, 0x36)),
            (MdKey::Rule, Color::Rgb(0x50, 0x49, 0x45)),
            (MdKey::LinkIcon, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::LinkText, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::LinkHover, Color::Rgb(0xb8, 0xd4, 0xc0)),
            (MdKey::BlockquoteText, Color::Rgb(0xd5, 0xc4, 0xa1)),
            (MdKey::Text, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (MdKey::StrongText, Color::Rgb(0xfb, 0xf1, 0xc7)),
            (MdKey::LatexInlineFg, Color::Rgb(0xd3, 0x86, 0x9b)),
            (MdKey::LatexInlineBg, Color::Rgb(0x3c, 0x38, 0x36)),
            (MdKey::LatexBlockFg, Color::Rgb(0xd3, 0x86, 0x9b)),
            (MdKey::MermaidKeyword, Color::Rgb(0xfe, 0x80, 0x19)),
            (MdKey::MermaidArrow, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::MermaidLabel, Color::Rgb(0xb8, 0xbb, 0x26)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (MdKey::MarkFg, Color::Rgb(0xeb, 0xdb, 0xb2)),
            (MdKey::MarkBg, Color::Rgb(0x79, 0x74, 0x0e)),
            (MdKey::TaskChecked, Color::Rgb(0xb8, 0xbb, 0x26)),
            (MdKey::TaskUnchecked, Color::Rgb(0x92, 0x83, 0x74)),
            (MdKey::AlertNote, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::AlertTip, Color::Rgb(0xb8, 0xbb, 0x26)),
            (MdKey::AlertImportant, Color::Rgb(0xd3, 0x86, 0x9b)),
            (MdKey::AlertWarning, Color::Rgb(0xfa, 0xbd, 0x2f)),
            (MdKey::AlertCaution, Color::Rgb(0xfb, 0x49, 0x34)),
            (MdKey::FootnoteRef, Color::Rgb(0x83, 0xa5, 0x98)),
            (MdKey::FootnoteText, Color::Rgb(0xd5, 0xc4, 0xa1)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::SearchMatchBg, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (MdKey::CodeGutter, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::ListLevel1, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (MdKey::ListLevel2, Color::Rgb(0x81, 0xa1, 0xc1)),
            (MdKey::ListLevel3, Color::Rgb(0xb4, 0x8e, 0xad)),
            (MdKey::OrderedList, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (MdKey::TableBorder, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::TableSeparator, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::TableHeader, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (MdKey::TableCell, Color::Rgb(0xd8, 0xde, 0xe9)),
            (MdKey::Heading1, Color::Rgb(0x88, 0xc0, 0xd0)),
            (MdKey::Heading2, Color::Rgb(0x81, 0xa1, 0xc1)),
            (MdKey::Heading3, Color::Rgb(0x8f, 0xbc, 0xbb)),
            (MdKey::Heading4, Color::Rgb(0xb4, 0x8e, 0xad)),
            (MdKey::HeadingOther, Color::Rgb(0xd0, 0x87, 0x70)),
            (MdKey::HeadingUnderline, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::CodeFrame, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::CodeLabel, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::InlineCodeFg, Color::Rgb(0xd0, 0x87, 0x70)),
            (MdKey::InlineCodeBg, Color::Rgb(0x3b, 0x42, 0x52)),
            (MdKey::Rule, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::LinkIcon, Color::Rgb(0x88, 0xc0, 0xd0)),
            (MdKey::LinkText, Color::Rgb(0x88, 0xc0, 0xd0)),
            (MdKey::LinkHover, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (MdKey::BlockquoteText, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::Text, Color::Rgb(0xd8, 0xde, 0xe9)),
            (MdKey::StrongText, Color::Rgb(0xec, 0xef, 0xf4)),
            (MdKey::LatexInlineFg, Color::Rgb(0xb4, 0x8e, 0xad)),
            (MdKey::LatexInlineBg, Color::Rgb(0x3b, 0x42, 0x52)),
            (MdKey::LatexBlockFg, Color::Rgb(0xb4, 0x8e, 0xad)),
            (MdKey::MermaidKeyword, Color::Rgb(0xd0, 0x87, 0x70)),
            (MdKey::MermaidArrow, Color::Rgb(0x88, 0xc0, 0xd0)),
            (MdKey::MermaidLabel, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xd8, 0xde, 0xe9)),
            (MdKey::MarkFg, Color::Rgb(0x2e, 0x34, 0x40)),
            (MdKey::MarkBg, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (MdKey::TaskChecked, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (MdKey::TaskUnchecked, Color::Rgb(0x4c, 0x56, 0x6a)),
            (MdKey::AlertNote, Color::Rgb(0x81, 0xa1, 0xc1)),
            (MdKey::AlertTip, Color::Rgb(0xa3, 0xbe, 0x8c)),
            (MdKey::AlertImportant, Color::Rgb(0xb4, 0x8e, 0xad)),
            (MdKey::AlertWarning, Color::Rgb(0xeb, 0xcb, 0x8b)),
            (MdKey::AlertCaution, Color::Rgb(0xbf, 0x61, 0x6a)),
            (MdKey::FootnoteRef, Color::Rgb(0x88, 0xc0, 0xd0)),
            (MdKey::FootnoteText, Color::Rgb(0xd8, 0xde, 0xe9)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x40, 0x3d, 0x52)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::CodeGutter, Color::Rgb(0x52, 0x4f, 0x67)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x6e, 0x6a, 0x86)),
            (MdKey::ListLevel1, Color::Rgb(0xeb, 0xbc, 0xba)),
            (MdKey::ListLevel2, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::ListLevel3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::OrderedList, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::TableBorder, Color::Rgb(0x40, 0x3d, 0x52)),
            (MdKey::TableSeparator, Color::Rgb(0x26, 0x23, 0x3a)),
            (MdKey::TableHeader, Color::Rgb(0xeb, 0xbc, 0xba)),
            (MdKey::TableCell, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::Heading1, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::Heading2, Color::Rgb(0xeb, 0xbc, 0xba)),
            (MdKey::Heading3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::Heading4, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::HeadingOther, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::HeadingUnderline, Color::Rgb(0x40, 0x3d, 0x52)),
            (MdKey::CodeFrame, Color::Rgb(0x40, 0x3d, 0x52)),
            (MdKey::CodeLabel, Color::Rgb(0x90, 0x8c, 0xaa)),
            (MdKey::InlineCodeFg, Color::Rgb(0xeb, 0xbc, 0xba)),
            (MdKey::InlineCodeBg, Color::Rgb(0x26, 0x23, 0x3a)),
            (MdKey::Rule, Color::Rgb(0x40, 0x3d, 0x52)),
            (MdKey::LinkIcon, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::LinkText, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::LinkHover, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::BlockquoteText, Color::Rgb(0x90, 0x8c, 0xaa)),
            (MdKey::Text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::StrongText, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::LatexInlineFg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::LatexInlineBg, Color::Rgb(0x26, 0x23, 0x3a)),
            (MdKey::LatexBlockFg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::MermaidKeyword, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::MermaidArrow, Color::Rgb(0x31, 0x74, 0x8f)),
            (MdKey::MermaidLabel, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::MarkFg, Color::Rgb(0x19, 0x17, 0x24)),
            (MdKey::MarkBg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::TaskChecked, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::TaskUnchecked, Color::Rgb(0x6e, 0x6a, 0x86)),
            (MdKey::AlertNote, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::AlertTip, Color::Rgb(0x31, 0x74, 0x8f)),
            (MdKey::AlertImportant, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::AlertWarning, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::AlertCaution, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::FootnoteRef, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::FootnoteText, Color::Rgb(0x90, 0x8c, 0xaa)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0xdf, 0xda, 0xd9)),
            (MdKey::SearchMatchBg, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::CodeGutter, Color::Rgb(0xce, 0xca, 0xcd)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x98, 0x93, 0xa5)),
            (MdKey::ListLevel1, Color::Rgb(0xd7, 0x82, 0x7e)),
            (MdKey::ListLevel2, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::ListLevel3, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::OrderedList, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::TableBorder, Color::Rgb(0xdf, 0xda, 0xd9)),
            (MdKey::TableSeparator, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (MdKey::TableHeader, Color::Rgb(0xd7, 0x82, 0x7e)),
            (MdKey::TableCell, Color::Rgb(0x57, 0x52, 0x79)),
            (MdKey::Heading1, Color::Rgb(0xb4, 0x63, 0x7a)),
            (MdKey::Heading2, Color::Rgb(0xd7, 0x82, 0x7e)),
            (MdKey::Heading3, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::Heading4, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::HeadingOther, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::HeadingUnderline, Color::Rgb(0xdf, 0xda, 0xd9)),
            (MdKey::CodeFrame, Color::Rgb(0xdf, 0xda, 0xd9)),
            (MdKey::CodeLabel, Color::Rgb(0x79, 0x75, 0x93)),
            (MdKey::InlineCodeFg, Color::Rgb(0xd7, 0x82, 0x7e)),
            (MdKey::InlineCodeBg, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (MdKey::Rule, Color::Rgb(0xdf, 0xda, 0xd9)),
            (MdKey::LinkIcon, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::LinkText, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::LinkHover, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::BlockquoteText, Color::Rgb(0x79, 0x75, 0x93)),
            (MdKey::Text, Color::Rgb(0x57, 0x52, 0x79)),
            (MdKey::StrongText, Color::Rgb(0x57, 0x52, 0x79)),
            (MdKey::LatexInlineFg, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::LatexInlineBg, Color::Rgb(0xf2, 0xe9, 0xe1)),
            (MdKey::LatexBlockFg, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::MermaidKeyword, Color::Rgb(0xb4, 0x63, 0x7a)),
            (MdKey::MermaidArrow, Color::Rgb(0x28, 0x69, 0x83)),
            (MdKey::MermaidLabel, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::MermaidBlockFg, Color::Rgb(0x57, 0x52, 0x79)),
            (MdKey::MarkFg, Color::Rgb(0xfa, 0xf4, 0xed)),
            (MdKey::MarkBg, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::TaskChecked, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::TaskUnchecked, Color::Rgb(0x98, 0x93, 0xa5)),
            (MdKey::AlertNote, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::AlertTip, Color::Rgb(0x28, 0x69, 0x83)),
            (MdKey::AlertImportant, Color::Rgb(0x90, 0x7a, 0xa9)),
            (MdKey::AlertWarning, Color::Rgb(0xea, 0x9d, 0x34)),
            (MdKey::AlertCaution, Color::Rgb(0xb4, 0x63, 0x7a)),
            (MdKey::FootnoteRef, Color::Rgb(0x56, 0x94, 0x9f)),
            (MdKey::FootnoteText, Color::Rgb(0x79, 0x75, 0x93)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x44, 0x41, 0x5a)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::CodeGutter, Color::Rgb(0x56, 0x52, 0x6e)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x6e, 0x6a, 0x86)),
            (MdKey::ListLevel1, Color::Rgb(0xea, 0x9a, 0x97)),
            (MdKey::ListLevel2, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::ListLevel3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::OrderedList, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::TableBorder, Color::Rgb(0x44, 0x41, 0x5a)),
            (MdKey::TableSeparator, Color::Rgb(0x39, 0x35, 0x52)),
            (MdKey::TableHeader, Color::Rgb(0xea, 0x9a, 0x97)),
            (MdKey::TableCell, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::Heading1, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::Heading2, Color::Rgb(0xea, 0x9a, 0x97)),
            (MdKey::Heading3, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::Heading4, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::HeadingOther, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::HeadingUnderline, Color::Rgb(0x44, 0x41, 0x5a)),
            (MdKey::CodeFrame, Color::Rgb(0x44, 0x41, 0x5a)),
            (MdKey::CodeLabel, Color::Rgb(0x90, 0x8c, 0xaa)),
            (MdKey::InlineCodeFg, Color::Rgb(0xea, 0x9a, 0x97)),
            (MdKey::InlineCodeBg, Color::Rgb(0x39, 0x35, 0x52)),
            (MdKey::Rule, Color::Rgb(0x44, 0x41, 0x5a)),
            (MdKey::LinkIcon, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::LinkText, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::LinkHover, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::BlockquoteText, Color::Rgb(0x90, 0x8c, 0xaa)),
            (MdKey::Text, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::StrongText, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::LatexInlineFg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::LatexInlineBg, Color::Rgb(0x39, 0x35, 0x52)),
            (MdKey::LatexBlockFg, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::MermaidKeyword, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::MermaidArrow, Color::Rgb(0x3e, 0x8f, 0xb0)),
            (MdKey::MermaidLabel, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xe0, 0xde, 0xf4)),
            (MdKey::MarkFg, Color::Rgb(0x23, 0x21, 0x36)),
            (MdKey::MarkBg, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::TaskChecked, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::TaskUnchecked, Color::Rgb(0x6e, 0x6a, 0x86)),
            (MdKey::AlertNote, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::AlertTip, Color::Rgb(0x3e, 0x8f, 0xb0)),
            (MdKey::AlertImportant, Color::Rgb(0xc4, 0xa7, 0xe7)),
            (MdKey::AlertWarning, Color::Rgb(0xf6, 0xc1, 0x77)),
            (MdKey::AlertCaution, Color::Rgb(0xeb, 0x6f, 0x92)),
            (MdKey::FootnoteRef, Color::Rgb(0x9c, 0xcf, 0xd8)),
            (MdKey::FootnoteText, Color::Rgb(0x90, 0x8c, 0xaa)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::SearchMatchBg, Color::Rgb(0xf0, 0xe3, 0xb0)),
            (MdKey::CodeGutter, Color::Rgb(0x89, 0x90, 0xb3)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x89, 0x90, 0xb3)),
            (MdKey::ListLevel1, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::ListLevel2, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (MdKey::ListLevel3, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::OrderedList, Color::Rgb(0xb1, 0x5c, 0x00)),
            (MdKey::TableBorder, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::TableSeparator, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::TableHeader, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (MdKey::TableCell, Color::Rgb(0x37, 0x60, 0xbf)),
            (MdKey::Heading1, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::Heading2, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (MdKey::Heading3, Color::Rgb(0x00, 0x71, 0x97)),
            (MdKey::Heading4, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::HeadingOther, Color::Rgb(0xb1, 0x5c, 0x00)),
            (MdKey::HeadingUnderline, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::CodeFrame, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::CodeLabel, Color::Rgb(0x89, 0x90, 0xb3)),
            (MdKey::InlineCodeFg, Color::Rgb(0xb1, 0x5c, 0x00)),
            (MdKey::InlineCodeBg, Color::Rgb(0xd5, 0xd8, 0xe6)),
            (MdKey::Rule, Color::Rgb(0xb7, 0xc1, 0xe3)),
            (MdKey::LinkIcon, Color::Rgb(0x00, 0x71, 0x97)),
            (MdKey::LinkText, Color::Rgb(0x00, 0x71, 0x97)),
            (MdKey::LinkHover, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::BlockquoteText, Color::Rgb(0x61, 0x72, 0xb0)),
            (MdKey::Text, Color::Rgb(0x37, 0x60, 0xbf)),
            (MdKey::StrongText, Color::Rgb(0x24, 0x30, 0x5e)),
            (MdKey::LatexInlineFg, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::LatexInlineBg, Color::Rgb(0xd5, 0xd8, 0xe6)),
            (MdKey::LatexBlockFg, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::MermaidKeyword, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::MermaidArrow, Color::Rgb(0x00, 0x71, 0x97)),
            (MdKey::MermaidLabel, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::MermaidBlockFg, Color::Rgb(0x37, 0x60, 0xbf)),
            (MdKey::MarkFg, Color::Rgb(0xe1, 0xe2, 0xe7)),
            (MdKey::MarkBg, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (MdKey::TaskChecked, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::TaskUnchecked, Color::Rgb(0x89, 0x90, 0xb3)),
            (MdKey::AlertNote, Color::Rgb(0x2e, 0x7d, 0xe9)),
            (MdKey::AlertTip, Color::Rgb(0x58, 0x75, 0x39)),
            (MdKey::AlertImportant, Color::Rgb(0x98, 0x54, 0xf1)),
            (MdKey::AlertWarning, Color::Rgb(0x8c, 0x6c, 0x3e)),
            (MdKey::AlertCaution, Color::Rgb(0xf5, 0x2a, 0x65)),
            (MdKey::FootnoteRef, Color::Rgb(0x00, 0x71, 0x97)),
            (MdKey::FootnoteText, Color::Rgb(0x61, 0x72, 0xb0)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::SearchMatchBg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::CodeGutter, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x56, 0x5f, 0x89)),
            (MdKey::ListLevel1, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::ListLevel2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::ListLevel3, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::OrderedList, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::TableBorder, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::TableSeparator, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::TableHeader, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::TableCell, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::Heading1, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::Heading2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::Heading3, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::Heading4, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::HeadingOther, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::HeadingUnderline, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::CodeFrame, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::CodeLabel, Color::Rgb(0x56, 0x5f, 0x89)),
            (MdKey::InlineCodeFg, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::InlineCodeBg, Color::Rgb(0x16, 0x16, 0x1e)),
            (MdKey::Rule, Color::Rgb(0x54, 0x5c, 0x7e)),
            (MdKey::LinkIcon, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::LinkText, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::LinkHover, Color::Rgb(0x73, 0xda, 0xca)),
            (MdKey::BlockquoteText, Color::Rgb(0xa9, 0xb1, 0xd6)),
            (MdKey::Text, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::LatexInlineBg, Color::Rgb(0x16, 0x16, 0x1e)),
            (MdKey::LatexBlockFg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::MermaidKeyword, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::MermaidArrow, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::MermaidLabel, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::MarkFg, Color::Rgb(0x1a, 0x1b, 0x26)),
            (MdKey::MarkBg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::TaskChecked, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::TaskUnchecked, Color::Rgb(0x56, 0x5f, 0x89)),
            (MdKey::AlertNote, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::AlertTip, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::AlertImportant, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::AlertWarning, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::AlertCaution, Color::Rgb(0xf7, 0x76, 0x8e)),
            (MdKey::FootnoteRef, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::FootnoteText, Color::Rgb(0xa9, 0xb1, 0xd6)),
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
        },
        md: &[
            (MdKey::SearchHighlightBg, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::SearchMatchBg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::CodeGutter, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::BlockquoteMarker, Color::Rgb(0x73, 0x7a, 0xa2)),
            (MdKey::ListLevel1, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::ListLevel2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::ListLevel3, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::OrderedList, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::TableBorder, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::TableSeparator, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::TableHeader, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::TableCell, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::Heading1, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::Heading2, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::Heading3, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::Heading4, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::HeadingOther, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::HeadingUnderline, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::CodeFrame, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::CodeLabel, Color::Rgb(0x73, 0x7a, 0xa2)),
            (MdKey::InlineCodeFg, Color::Rgb(0xff, 0x9e, 0x64)),
            (MdKey::InlineCodeBg, Color::Rgb(0x1f, 0x23, 0x35)),
            (MdKey::Rule, Color::Rgb(0x41, 0x48, 0x68)),
            (MdKey::LinkIcon, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::LinkText, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::LinkHover, Color::Rgb(0x73, 0xda, 0xca)),
            (MdKey::BlockquoteText, Color::Rgb(0xa9, 0xb1, 0xd6)),
            (MdKey::Text, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::StrongText, Color::Rgb(0xff, 0xff, 0xff)),
            (MdKey::LatexInlineFg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::LatexInlineBg, Color::Rgb(0x1f, 0x23, 0x35)),
            (MdKey::LatexBlockFg, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::MermaidKeyword, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::MermaidArrow, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::MermaidLabel, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::MermaidBlockFg, Color::Rgb(0xc0, 0xca, 0xf5)),
            (MdKey::MarkFg, Color::Rgb(0x24, 0x28, 0x3b)),
            (MdKey::MarkBg, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::TaskChecked, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::TaskUnchecked, Color::Rgb(0x73, 0x7a, 0xa2)),
            (MdKey::AlertNote, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (MdKey::AlertTip, Color::Rgb(0x9e, 0xce, 0x6a)),
            (MdKey::AlertImportant, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (MdKey::AlertWarning, Color::Rgb(0xe0, 0xaf, 0x68)),
            (MdKey::AlertCaution, Color::Rgb(0xf7, 0x76, 0x8e)),
            (MdKey::FootnoteRef, Color::Rgb(0x7d, 0xcf, 0xff)),
            (MdKey::FootnoteText, Color::Rgb(0xa9, 0xb1, 0xd6)),
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
                    seen.insert(std::mem::discriminant(key)),
                    "{}: a markdown key appears twice",
                    spec.name
                );
            }
        }
    }
}
