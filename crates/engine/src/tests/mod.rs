//! Ported subset of leaf's `src/tests/` common helpers: only what the vendored
//! markdown tests need (no App/render-pipeline helpers).

mod markdown_blocks;
mod markdown_embedded;
mod markdown_footnotes;
mod markdown_html_tag;
mod markdown_links;
mod markdown_list_blocks;
mod markdown_lists;
mod markdown_rendering;
mod markdown_tables;
mod wrapping;

use ratatui::backend::TestBackend;
use ratatui::{text::Line, widgets::Paragraph, Terminal};
use std::sync::{Arc, LazyLock};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: LazyLock<Arc<SyntaxSet>> =
    LazyLock::new(|| Arc::new(SyntaxSet::load_defaults_newlines()));
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

pub(super) fn test_syntax_set() -> Arc<SyntaxSet> {
    SYNTAX_SET.clone()
}

pub(super) fn test_assets() -> (Arc<SyntaxSet>, Theme) {
    (
        test_syntax_set(),
        THEME_SET.themes["base16-ocean.dark"].clone(),
    )
}

pub(super) fn test_md_theme() -> crate::theme::MarkdownTheme {
    crate::theme::default_markdown_theme()
}

pub(super) fn render_buffer(lines: &[Line<'static>]) -> ratatui::buffer::Buffer {
    let width = lines
        .iter()
        .map(|line| line.width())
        .max()
        .unwrap_or(1)
        .max(1) as u16;
    let height = lines.len().max(1) as u16;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            f.render_widget(Paragraph::new(lines.to_vec()), f.area());
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

pub(super) fn find_symbol(buffer: &ratatui::buffer::Buffer, symbol: &str) -> Option<(u16, u16)> {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if buffer
                .cell((x, y))
                .is_some_and(|cell| cell.symbol() == symbol)
            {
                return Some((x, y));
            }
        }
    }
    None
}

pub(super) fn rendered_non_empty_lines(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(crate::markdown::line_plain_text)
        .filter(|line| !line.is_empty())
        .collect()
}
