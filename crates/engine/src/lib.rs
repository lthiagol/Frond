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

use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Syntax highlighting assets used as the default input for the parse functions.
///
/// Mirrors what leaf wires up in `main` (default syntax set + a syntect theme).
#[derive(Clone, Debug)]
pub struct SyntaxAssets {
    pub syntax_set: SyntaxSet,
    pub theme: Theme,
}

impl Default for SyntaxAssets {
    fn default() -> Self {
        Self::load_defaults()
    }
}

impl SyntaxAssets {
    /// Loads syntect's built-in syntax set and the `base16-ocean.dark` theme
    /// (leaf's default).
    pub fn load_defaults() -> Self {
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .or_else(|| theme_set.themes.values().next().cloned())
            .expect("syntect default theme set is empty");
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    }

    /// Swaps the highlighting theme for the named syntect theme from the
    /// default theme set. Returns `false` and keeps the current theme when
    /// the name is unknown (callers warn). Only called on theme switches,
    /// never per frame. (An earlier separate `has_syntax_theme` probe was
    /// removed as dead API — this validates internally.)
    pub fn set_syntax_theme(&mut self, name: &str) -> bool {
        match ThemeSet::load_defaults().themes.get(name) {
            Some(theme) => {
                self.theme = theme.clone();
                true
            }
            None => false,
        }
    }
}
