//! Vendored markdown rendering engine (from leaf, MIT licensed).
//!
//! See `markdown/VENDORED.md` (crate root: `VENDORED.md`) for the full list of
//! deviations from upstream leaf.

pub mod markdown;
pub mod theme;

#[cfg(test)]
pub(crate) use markdown::line_plain_text;

pub use markdown::{
    parse_markdown, parse_markdown_with_width, parse_markdown_with_width_opts, CodeBlockInfo,
    LinkId, LinkSpan, ParseResult, TocEntry,
};
pub use theme::{
    default_markdown_theme, markdown_theme_by_preset, MarkdownTheme, MarkdownThemePreset,
};

#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

/// The bundled syntax set, deserialised once per process (multi-megabyte,
/// tens of milliseconds) and shared by every [`SyntaxAssets`].
fn default_syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The bundled theme set, built once per process.
fn default_theme_set() -> &'static ThemeSet {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    SET.get_or_init(ThemeSet::load_defaults)
}

/// Syntax highlighting assets used as the default input for the parse functions.
///
/// Mirrors what leaf wires up in `main` (default syntax set + a syntect theme).
/// The syntax set is process-wide and shared, so cloning or reloading the
/// assets (theme cycling) never re-deserialises it.
#[derive(Clone, Debug)]
pub struct SyntaxAssets {
    pub syntax_set: &'static SyntaxSet,
    pub theme: Theme,
}

impl Default for SyntaxAssets {
    fn default() -> Self {
        Self::load_defaults()
    }
}

impl SyntaxAssets {
    /// The built-in syntax set and the `base16-ocean.dark` theme (leaf's
    /// default). Heavy assets are loaded once and shared.
    pub fn load_defaults() -> Self {
        let theme_set = default_theme_set();
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .or_else(|| theme_set.themes.values().next().cloned())
            .expect("syntect default theme set is empty");
        Self {
            syntax_set: default_syntax_set(),
            theme,
        }
    }

    /// Swaps the highlighting theme for the named syntect theme from the
    /// default theme set. Returns `false` and keeps the current theme when
    /// the name is unknown (callers warn). Only called on theme switches,
    /// never per frame. (An earlier separate `has_syntax_theme` probe was
    /// removed as dead API — this validates internally.)
    pub fn set_syntax_theme(&mut self, name: &str) -> bool {
        match default_theme_set().themes.get(name) {
            Some(theme) => {
                self.theme = theme.clone();
                true
            }
            None => false,
        }
    }
}
