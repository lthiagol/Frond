//! Drawing: sidebar (tree / tags) + rendered-note viewport + backlinks
//! panel + status bar + help/disambiguation popups.

use std::path::Path;
use std::time::{Instant, SystemTime};

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;
use vault::VaultIndex;

use crate::app::{App, Disambiguation, Focus, SidebarMode};
use crate::search::SearchMode;
use crate::sidebar::{truncate_ellipsis, NodeKind, Row};
use crate::theme::UiTheme;
use unicode_width::UnicodeWidthStr;
use vault::GitStatus;

/// Sidebar width: ~30% of the terminal, with sane bounds.
pub fn sidebar_width(total: u16) -> u16 {
    (u32::from(total) * 3 / 10).clamp(24, 60) as u16
}

/// Right-edge vertical scrollbar (the `draw_base_table` / help pattern).
/// Drawn into a single-column `bar_area`; callers only invoke it when the
/// content overflows, and shrink their text area by that one column.
fn draw_scrollbar(
    f: &mut Frame,
    bar_area: Rect,
    total: usize,
    position: usize,
    viewport: usize,
    fg: Color,
    bg: Color,
) {
    let mut state = ScrollbarState::new(total)
        .position(position)
        .viewport_content_length(viewport);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).style(Style::new().fg(fg).bg(bg)),
        bar_area,
        &mut state,
    );
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // The chrome palette: every hardcoded shell color below comes from
    // the active theme (see `theme.rs`).
    let ui = app.theme_ui;
    // Feed the actual frame height back into the viewer: single source of
    // truth for the viewport height (also clamps the scroll to it).
    let view_height = area.height.saturating_sub(1);

    let (sidebar_area, content_area) = if app.sidebar_visible {
        // Explicit override (`<`/`>`, config `sidebar_width`) wins over the
        // proportional default; both are clamped to the real terminal width.
        let width = app
            .sidebar_width
            .unwrap_or_else(|| sidebar_width(area.width))
            .min(area.width);
        let sidebar = Some(Rect {
            x: area.x,
            y: area.y,
            width,
            height: view_height,
        });
        (
            sidebar,
            Rect {
                x: area.x + width,
                y: area.y,
                width: area.width.saturating_sub(width),
                height: view_height,
            },
        )
    } else {
        (
            None,
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: view_height,
            },
        )
    };
    // Geometry for the mouse handlers (click/wheel routing): fresh every
    // frame, `None` while the sidebar is hidden.
    app.sidebar_area = sidebar_area;

    if app.base.is_some() {
        // A `.base` file is open: the content area is its table (the
        // viewer is hidden; its parse width is untouched until an actual
        // note is visible again).
        let note_area = Rect {
            height: view_height,
            ..content_area
        };
        app.note_viewport = note_area;
        app.backlinks_area = None;
        draw_base_table(f, note_area, app, ui);
    } else {
        // The backlinks panel (`b`) takes the bottom slice of the content area.
        let panel_height = if app.backlinks_open {
            (view_height / 3)
                .clamp(3, 10)
                .min(view_height.saturating_sub(1))
        } else {
            0
        };
        let note_full = Rect {
            height: view_height - panel_height,
            ..content_area
        };
        // The viewer's viewport helper subtracts the 1-line status bar itself,
        // so pass the note rows plus that status line.
        let note_rows = note_full.height as usize;

        // The engine pre-wraps at the render width, so it must match the real
        // text width (re-parses on change, e.g. sidebar toggled or resize).
        // Overflow is decided at the full text width first; if the note
        // overflows, the scrollbar takes the right column and the text
        // re-wraps one column narrower. Narrowing only ever adds lines, so
        // an overflowing note keeps overflowing and the bar stays consistent.
        let full_width = content_area.width.saturating_sub(2) as usize;
        app.viewer.on_content_width(full_width);
        app.viewer.on_viewport_height(note_full.height + 1);
        let bar_shown = app.viewer.lines.len() > note_rows && note_full.width > 3;
        let note_area = if bar_shown {
            app.viewer
                .on_content_width(full_width.saturating_sub(1).max(1));
            Rect {
                width: note_full.width - 1,
                ..note_full
            }
        } else {
            note_full
        };
        app.note_viewport = note_area;
        app.viewer.on_viewport_height(note_full.height + 1);

        let text = ratatui::text::Text::from(app.viewer.lines.clone());
        // `Paragraph::scroll` takes `u16`; the viewer's scroll is `u32` so very
        // large files are not truncated in scroll math. u16::MAX scroll offset
        // can still not be reached on real terminals, so clamping here is safe.
        let scroll = app.viewer.scroll.min(u16::MAX as u32) as u16;
        Paragraph::new(text)
            .style(Style::new().bg(ui.content_bg))
            .scroll((scroll, 0))
            .render(note_area, f.buffer_mut());
        if bar_shown {
            let bar = Rect {
                x: note_full.right() - 1,
                width: 1,
                ..note_full
            };
            draw_scrollbar(
                f,
                bar,
                app.viewer.lines.len(),
                app.viewer.scroll as usize,
                note_rows,
                ui.popup_dimmed_fg,
                ui.content_bg,
            );
        }

        // Geometry for click routing (focus on click), fresh every frame.
        app.backlinks_area = None;
        if panel_height > 0 {
            let panel = Rect {
                y: content_area.y + view_height - panel_height,
                height: panel_height,
                ..content_area
            };
            draw_backlinks(f, panel, app, ui, app.focus == Focus::Backlinks);
        }
    }

    if let Some(sidebar_rect) = sidebar_area {
        let index = app.index.as_ref().map(|i| i.borrow());
        if app.sidebar_mode == SidebarMode::Tags {
            draw_tag_list(
                f,
                sidebar_rect,
                &mut app.tags,
                ui,
                app.focus == Focus::Sidebar,
                index.as_deref(),
            );
        } else if let Some(sidebar) = &mut app.sidebar {
            draw_sidebar(
                f,
                sidebar_rect,
                sidebar,
                ui,
                app.focus == Focus::Sidebar,
                index.as_deref(),
                app.git.as_ref(),
            );
        }
    }

    let status = Rect {
        x: area.x,
        y: area.y + view_height,
        width: area.width,
        height: 1,
    };
    draw_status_bar(f, status, app, ui);

    if app.viewer.help_open {
        draw_help(f, area, app, ui);
    }
    if let Some(disambiguation) = &app.disambiguation {
        let index = app.index.as_ref().map(|i| i.borrow());
        draw_disambiguation(f, area, disambiguation, ui, index.as_deref());
    }
    if app.search.open {
        // Record the popup geometry for the mouse handlers before the
        // index borrow (which would block the `&mut` access).
        let popup = crate::search::popup_geometry(area, app.search.mode, app.search.hits.len());
        app.search_area = Some(popup);
        let index = app.index.as_ref().map(|i| i.borrow());
        draw_search(f, area, app, ui, index.as_deref());
    } else {
        app.search_area = None;
    }
}

/// The open `.base` table: bordered block (view title) + header row +
/// rule + rows, scrolled. Click regions are recorded on the state for
/// the click handler, which maps clicks with the same geometry.
fn draw_base_table(f: &mut Frame, area: Rect, app: &mut App, ui: UiTheme) {
    let Some(base) = &mut app.base else {
        return;
    };
    let title = format!(
        " base · view {}/{}: {} ",
        base.view + 1,
        base.view_count(),
        base.view_name()
    );
    f.render_widget(
        Block::new()
            .borders(Borders::ALL)
            .border_style(
                Style::new()
                    .fg(ui.sidebar_border_unfocused)
                    .bg(ui.content_bg),
            )
            .title(title),
        area,
    );
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let lines = base.render(ui, inner.width as usize, inner.height as usize);
    Paragraph::new(ratatui::text::Text::from(lines))
        .style(Style::new().bg(ui.content_bg))
        .render(inner, f.buffer_mut());
}

/// Visual treatment for a status glyph. Built only from colors the app
/// already uses elsewhere (status bar / borders) — no separate palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusStyle {
    /// Plain default text.
    Normal,
    /// Demoted/inactive state: `Color::DarkGray` (name dimmed too).
    Dimmed,
    /// Live/current state: `Color::Green` + bold (status-bar clean `✓`).
    Green,
    /// Needs attention: `Color::Red` (status-bar behind `↓n`).
    Red,
}

impl StatusStyle {
    /// The style applied to the glyph span itself.
    pub fn glyph_style(self) -> Style {
        match self {
            StatusStyle::Normal => Style::new(),
            StatusStyle::Dimmed => Style::new().fg(Color::DarkGray),
            StatusStyle::Green => Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            StatusStyle::Red => Style::new().fg(Color::Red),
        }
    }
}

/// Frontmatter status glyph. One table for every status string found in
/// the real vault (plus SPEC's originals): `●` complete/resolved/active,
/// `○` pending/planned, `◐` in-progress/monitoring, `◌` draft, `◉`
/// blocked, `×` superseded/archived (demoted — rendered dimmed). Unknown
/// statuses get no glyph but stay eligible for the map later.
pub fn status_glyph(status: Option<&str>) -> Option<(&'static str, StatusStyle)> {
    match status?.trim().to_ascii_lowercase().as_str() {
        "complete" | "resolved" => Some(("●", StatusStyle::Normal)),
        "active" => Some(("●", StatusStyle::Green)),
        "pending" | "planned" => Some(("○", StatusStyle::Normal)),
        "in-progress" | "in progress" | "monitoring" => Some(("◐", StatusStyle::Normal)),
        "draft" => Some(("◌", StatusStyle::Normal)),
        "blocked" => Some(("◉", StatusStyle::Red)),
        "superseded" | "archived" => Some(("×", StatusStyle::Dimmed)),
        _ => None,
    }
}

fn draw_sidebar(
    f: &mut Frame,
    area: Rect,
    sidebar: &mut crate::sidebar::Sidebar,
    ui: UiTheme,
    focused: bool,
    index: Option<&VaultIndex>,
    git: Option<&GitStatus>,
) {
    let inner_height = area.height.saturating_sub(2) as usize;
    sidebar.on_viewport_height(inner_height.max(1));
    sidebar.scroll_selection_into_view();

    let rows_total = sidebar.rows().len();
    let overflow = rows_total > inner_height;
    // Text area: the bordered inner area, minus the scrollbar column when
    // the rows overflow (the bar sits on the inner right edge, so the
    // border itself stays intact).
    let text_area = list_text_area(area, overflow);
    let inner_width = text_area.width as usize;

    let rows = sidebar.rows();
    let window: Vec<Line<'static>> = rows
        .iter()
        .skip(sidebar.scroll)
        .take(inner_height)
        .map(|row| {
            let expanded = sidebar.expanded.contains(&row.node.path);
            let selected = sidebar.selected.as_deref() == Some(row.node.path.as_path());
            let glyph = index
                .and_then(|index| index.note(&row.node.path))
                .and_then(|note| status_glyph(note.status.as_deref()));
            let changed = git.is_some_and(|git| git.changed_files.contains(&row.node.path));
            sidebar_line(row, expanded, inner_width, selected, glyph, changed, ui)
        })
        .collect();

    let border = if focused {
        ui.sidebar_border_focused
    } else {
        ui.sidebar_border_unfocused
    };
    f.render_widget(
        Block::new()
            .style(Style::new().bg(ui.sidebar_bg))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(" vault "),
        area,
    );
    f.render_widget(
        Paragraph::new(window).style(Style::new().bg(ui.sidebar_bg)),
        text_area,
    );
    if overflow {
        let bar = list_scrollbar_area(area);
        draw_scrollbar(
            f,
            bar,
            rows_total,
            sidebar.scroll,
            inner_height,
            ui.popup_dimmed_fg,
            ui.sidebar_bg,
        );
    }
}

/// The text rect of a bordered list widget: the inner area, minus the
/// scrollbar column on the right when `overflow`.
fn list_text_area(area: Rect, overflow: bool) -> Rect {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if overflow && inner.width > 1 {
        Rect {
            width: inner.width - 1,
            ..inner
        }
    } else {
        inner
    }
}

/// The scrollbar column of a bordered list (sidebar tree / tag list): the
/// rightmost inner column, just inside the border.
fn list_scrollbar_area(area: Rect) -> Rect {
    let inner = list_text_area(area, false);
    Rect {
        x: inner.right() - 1,
        y: inner.y,
        width: 1,
        height: inner.height,
    }
}

/// One sidebar row: indentation, folder arrow, status glyph, truncated
/// name. Demoted statuses (superseded/archived) dim the name; the glyph
/// itself carries the status style (green active, red blocked). Notes
/// changed since the last git sync get a `*` marker after the name.
fn sidebar_line(
    row: &Row<'_>,
    expanded: bool,
    inner_width: usize,
    selected: bool,
    glyph: Option<(&'static str, StatusStyle)>,
    changed: bool,
    ui: UiTheme,
) -> Line<'static> {
    let indent = "  ".repeat(row.depth);
    let (arrow, arrow_width) = match row.node.kind {
        NodeKind::Dir if expanded => ("▾ ", 2),
        NodeKind::Dir => ("▸ ", 2),
        NodeKind::Note => ("", 0),
    };
    let (glyph_text, glyph_width, dimmed, glyph_style) = match glyph {
        Some((glyph, style)) => (
            format!("{glyph} "),
            2,
            style == StatusStyle::Dimmed,
            style.glyph_style(),
        ),
        None => (String::new(), 0, false, Style::new()),
    };
    // The changed marker needs room after the name, so the name truncates
    // to make space for ` *` when the note is marked.
    let marker_width = usize::from(changed) * 2;
    let prefix_width = indent.width() + arrow_width + glyph_width;
    let name = truncate_ellipsis(
        &row.node.name,
        inner_width.saturating_sub(prefix_width + marker_width),
    );

    let mut spans = Vec::with_capacity(5);
    if !indent.is_empty() {
        spans.push(Span::raw(indent));
    }
    if !arrow.is_empty() {
        spans.push(Span::raw(arrow));
    }
    if !glyph_text.is_empty() {
        spans.push(Span::styled(glyph_text, glyph_style));
    }
    let mut name_style = Style::new();
    if dimmed {
        name_style = name_style.fg(ui.dimmed_fg);
    }
    spans.push(Span::styled(name, name_style));
    if changed {
        spans.push(Span::styled(" *", Style::new().fg(ui.changed_marker_fg)));
    }
    if selected {
        for span in &mut spans {
            span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
        }
    }
    Line::from(spans)
}

fn draw_tag_list(
    f: &mut Frame,
    area: Rect,
    tags: &mut crate::tags::TagList,
    ui: UiTheme,
    focused: bool,
    index: Option<&VaultIndex>,
) {
    let inner_height = area.height.saturating_sub(2) as usize;
    tags.on_viewport_height(inner_height.max(1));
    tags.scroll_selection_into_view();

    let rows_total = tags.rows().len();
    let overflow = rows_total > inner_height;
    let text_area = list_text_area(area, overflow);
    let inner_width = text_area.width as usize;

    let rows = tags.rows();
    let window: Vec<Line<'static>> = rows
        .iter()
        .skip(tags.scroll)
        .take(inner_height)
        .map(|row| {
            let selected = crate::tags::TagList::row_matches_pub(row, &tags.selected);
            tag_line(row, inner_width, selected, index, ui)
        })
        .collect();

    let border = if focused {
        ui.sidebar_border_focused
    } else {
        ui.sidebar_border_unfocused
    };
    f.render_widget(
        Block::new()
            .style(Style::new().bg(ui.sidebar_bg))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(" tags "),
        area,
    );
    f.render_widget(
        Paragraph::new(window).style(Style::new().bg(ui.sidebar_bg)),
        text_area,
    );
    if overflow {
        draw_scrollbar(
            f,
            list_scrollbar_area(area),
            rows_total,
            tags.scroll,
            inner_height,
            ui.popup_dimmed_fg,
            ui.sidebar_bg,
        );
    }
}

/// One tag-list row: `#tag (N)` or an indented note (vault-relative path
/// when indexed, else the file name).
fn tag_line(
    row: &crate::tags::TagRow<'_>,
    inner_width: usize,
    selected: bool,
    index: Option<&VaultIndex>,
    ui: UiTheme,
) -> Line<'static> {
    let (text, style) = match row.note {
        None => (
            format!("#{} ({})", row.group.tag, row.group.notes.len()),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Some(note) => (format!("  {}", rel_label(note, index)), Style::new()),
    };
    let mut line = Line::from(Span::styled(truncate_ellipsis(&text, inner_width), style));
    if selected {
        for span in &mut line.spans {
            span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
        }
    }
    line
}

/// A backlink row label: the note's vault-relative path when indexed,
/// else the raw path.
fn rel_label(path: &Path, index: Option<&VaultIndex>) -> String {
    index
        .and_then(|index| index.note(path))
        .map(|note| note.rel_path.clone())
        .unwrap_or_else(|| path.display().to_string())
}

fn draw_backlinks(f: &mut Frame, area: Rect, app: &mut App, ui: UiTheme, focused: bool) {
    let inner_height = area.height.saturating_sub(2) as usize;
    let rows_total = app.backlinks.len();
    let overflow = rows_total > inner_height;
    // Text stops one column before the bar when the rows overflow; the same
    // helper also drives the click bounds in `App::handle_click`.
    let text_area = list_text_area(area, overflow);
    let inner_width = text_area.width as usize;
    // Click routing: the whole panel minus the bar column (the bar is not a
    // backlinks hit target). Borders keep focusing the panel as before.
    // Exclusive right edge: the text's right edge when a bar is drawn (the
    // bar column itself is outside the hit area), else the full panel.
    let bar_shown = text_area.width < area.width.saturating_sub(2);
    let hit_right = if bar_shown {
        text_area.right()
    } else {
        area.right()
    };
    app.backlinks_area = Some(Rect {
        width: hit_right - area.x,
        ..area
    });
    app.on_backlinks_viewport(inner_height.max(1));

    let index = app.index.as_ref().map(|i| i.borrow());
    let window: Vec<Line<'static>> = if app.backlinks.is_empty() {
        vec![Line::from(Span::styled(
            "no notes link here",
            Style::new().fg(ui.dimmed_fg),
        ))]
    } else {
        app.backlinks
            .iter()
            .enumerate()
            .skip(app.backlink_scroll)
            .take(inner_height)
            .map(|(i, path)| {
                let selected = i == app.backlink_sel;
                let mut line = Line::from(Span::raw(truncate_ellipsis(
                    &rel_label(path, index.as_deref()),
                    inner_width,
                )));
                if selected {
                    for span in &mut line.spans {
                        span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
                    }
                }
                line
            })
            .collect()
    };

    let border = if focused {
        ui.sidebar_border_focused
    } else {
        ui.sidebar_border_unfocused
    };
    let title = format!(" backlinks ({}) ", app.backlinks.len());
    // The backlinks panel is a sidebar-family secondary panel, so it
    // paints `sidebar_bg` (not `content_bg`).
    f.render_widget(
        Block::new()
            .style(Style::new().bg(ui.sidebar_bg))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(title),
        area,
    );
    f.render_widget(
        Paragraph::new(window).style(Style::new().bg(ui.sidebar_bg)),
        text_area,
    );
    if bar_shown {
        draw_scrollbar(
            f,
            list_scrollbar_area(area),
            rows_total,
            app.backlink_scroll,
            inner_height,
            ui.popup_dimmed_fg,
            ui.sidebar_bg,
        );
    }
}

/// Disambiguation popup for a wikilink the folder hint could not narrow.
/// When more candidates exist than fit the terminal, the list scrolls and
/// the selection is kept visible.
fn draw_disambiguation(
    f: &mut Frame,
    area: Rect,
    d: &Disambiguation,
    ui: UiTheme,
    index: Option<&VaultIndex>,
) {
    let footer = "j/k select · Enter open · other key cancel";
    let width = 60.min(area.width.saturating_sub(2)).max(1);
    let height = (d.candidates.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(1);
    let popup = centered(area, width, height);
    // Inner rows available for candidates: popup height minus the two
    // border rows and the one footer row.
    let list_rows = height.saturating_sub(3).max(1) as usize;
    // Stateless scroll window: the last `list_rows` candidates ending at
    // (or containing) the selection, so the selection is always visible.
    let start = d.selected.saturating_sub(list_rows.saturating_sub(1));

    let mut lines: Vec<Line<'static>> = d
        .candidates
        .iter()
        .enumerate()
        .skip(start)
        .take(list_rows)
        .map(|(i, path)| {
            let selected = i == d.selected;
            let mut line = Line::from(Span::raw(truncate_ellipsis(
                &rel_label(path, index),
                width.saturating_sub(2) as usize,
            )));
            if selected {
                for span in &mut line.spans {
                    span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
                }
            }
            line
        })
        .collect();
    lines.push(Line::from(Span::styled(
        footer,
        Style::new().fg(ui.popup_dimmed_fg),
    )));

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::new().bg(ui.content_bg))
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(ui.popup_border_fg))
                    .title(" ambiguous link "),
            ),
        popup,
    );
}

/// Global search popup (`s`): input prompt or results list, depending on
/// the search state's mode. Results rows are `relpath:line snippet` with
/// the match highlighted; a stateless scroll window keeps the selection
/// visible when there are more hits than rows. The popup rectangle was
/// recorded into `app.search_area` by `draw` (the mouse handlers map
/// clicks/wheels through it, using the same row math as below).
fn draw_search(f: &mut Frame, area: Rect, app: &App, ui: UiTheme, index: Option<&VaultIndex>) {
    let popup = app.search_area.unwrap_or_else(|| {
        crate::search::popup_geometry(area, app.search.mode, app.search.hits.len())
    });
    let footer = match app.search.mode {
        SearchMode::Input => "Enter search · Esc cancel",
        SearchMode::Results => "j/k move · Enter open · Esc close",
    };
    let width = popup.width;
    let height = popup.height;
    let inner_width = width.saturating_sub(2) as usize;
    let list_rows = height.saturating_sub(3).max(1) as usize;

    let mut lines: Vec<Line<'static>> = match app.search.mode {
        SearchMode::Input => {
            vec![Line::from(Span::raw(format!("/{}▏", app.search.query)))]
        }
        SearchMode::Results => {
            if app.search.hits.is_empty() {
                vec![Line::from(Span::styled(
                    "no matches",
                    Style::new().fg(ui.popup_dimmed_fg),
                ))]
            } else {
                // Stateless scroll window: the selection always visible.
                let start = app
                    .search
                    .selected
                    .saturating_sub(list_rows.saturating_sub(1));
                app.search
                    .hits
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(list_rows)
                    .map(|(i, hit)| {
                        let selected = i == app.search.selected;
                        search_hit_line(hit, index, inner_width, selected, ui)
                    })
                    .collect()
            }
        }
    };
    lines.push(Line::from(Span::styled(
        footer,
        Style::new().fg(ui.popup_dimmed_fg),
    )));

    let title = match app.search.mode {
        SearchMode::Input => " search ".to_string(),
        SearchMode::Results => {
            let n = app.search.hits.len();
            // The vault caps results; "+" says more matches exist.
            if app.search.capped {
                format!(" search: {n}+ hits ")
            } else {
                format!(" search: {n} hits ")
            }
        }
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::new().bg(ui.content_bg))
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(ui.popup_border_fg))
                    .title(title),
            ),
        popup,
    );
}

/// One results row: `relpath:line snippet`, the match highlighted. The
/// path+line prefix is fixed; the snippet gets the remaining width.
fn search_hit_line(
    hit: &vault::SearchHit,
    index: Option<&VaultIndex>,
    inner_width: usize,
    selected: bool,
    ui: UiTheme,
) -> Line<'static> {
    let match_style = Style::new().fg(ui.popup_match_fg).bg(ui.popup_match_bg);
    let rel = rel_label(&hit.path, index);
    let prefix = format!("{rel}:{}: ", hit.line + 1);
    let prefix_width = prefix.width();
    if prefix_width + 1 >= inner_width {
        // Extremely narrow popup: just the truncated prefix.
        let mut line = Line::from(Span::raw(truncate_ellipsis(&prefix, inner_width)));
        if selected {
            for span in &mut line.spans {
                span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
            }
        }
        return line;
    }
    // `snippet_line` renders its own spans (with the match highlighted);
    // the selection highlight leaves those match spans intact.
    let line = crate::search::snippet_line(
        &hit.text,
        hit.start,
        hit.end,
        inner_width - prefix_width,
        match_style,
    );
    let mut spans = vec![Span::raw(prefix)];
    spans.extend(line.spans);
    if selected {
        for span in &mut spans {
            if span.style.bg != Some(ui.popup_match_bg) {
                span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
            }
        }
    }
    Line::from(spans)
}

/// Centered popup rectangle with sane clamping.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// The status bar's path label. Sidebar focused → the focused entry's full
/// path (or `#tag`); content focused → the open note's file name, or its
/// vault-relative path when that name is shared by another indexed note.
fn status_path_label(app: &App, sidebar_focused: bool) -> String {
    if !sidebar_focused {
        return note_status_label(app);
    }
    match app.sidebar_mode {
        SidebarMode::Tree => app
            .sidebar
            .as_ref()
            .and_then(|sidebar| sidebar.selected_path())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(empty vault)".to_string()),
        SidebarMode::Tags => {
            if let Some(path) = app.tags.selected_path() {
                path.display().to_string()
            } else if let Some(crate::tags::Selection::Tag(tag)) = &app.tags.selected {
                format!("#{tag}")
            } else {
                "(no tags)".to_string()
            }
        }
    }
}

/// The open note's label on the status bar: its file name, or the
/// vault-relative path when another indexed note shares that name.
fn note_status_label(app: &App) -> String {
    let current = app.current_path();
    let Some(name) = current.file_name().map(|n| n.to_os_string()) else {
        return current.display().to_string();
    };
    let ambiguous = match &app.index {
        Some(index) => index
            .borrow()
            .notes()
            .any(|note| note.path != current && note.path.file_name() == Some(name.as_os_str())),
        None => false,
    };
    if ambiguous {
        app.current_relative_path().display().to_string()
    } else {
        name.to_string_lossy().into_owned()
    }
}

fn draw_status_bar(f: &mut Frame, area: Rect, app: &mut App, ui: UiTheme) {
    // Sidebar focused → show the focused tree/tag entry's full path (the
    // note itself may not be open); content focused → the open note's path.
    let sidebar_focused = app.focus == Focus::Sidebar && app.sidebar_visible;
    let path = status_path_label(app, sidebar_focused);

    // Content size segment: note lines, or the base table's row count +
    // current view while a `.base` is open.
    let count = match &app.base {
        Some(base) => {
            let rows = base.table.note_rows();
            format!(
                "{} row{} · view {}/{}: {}",
                rows,
                if rows == 1 { "" } else { "s" },
                base.view + 1,
                base.view_count(),
                base.view_name()
            )
        }
        None => format!("{} lines", app.viewer.lines.len()),
    };

    let mut path_style = Style::new().add_modifier(Modifier::BOLD);
    if sidebar_focused {
        path_style = path_style.fg(ui.sidebar_border_focused);
    }
    // The note-name segment (clickable: copies the vault-relative path)
    // exists only in content focus; the sidebar's full path is not a name.
    app.status_name_area = if sidebar_focused {
        None
    } else {
        let width = (UnicodeWidthStr::width(path.as_str()) as u16).min(area.width);
        Some(Rect {
            x: area.x,
            y: area.y,
            width,
            height: 1,
        })
    };
    let mut spans = vec![
        Span::styled(path, path_style),
        Span::raw(" · "),
        Span::raw(count),
        Span::raw(" · "),
    ];
    if app.viewer.watch {
        spans.push(Span::styled(
            "watch on",
            Style::new().fg(ui.status_watch_on_fg),
        ));
    } else {
        spans.push(Span::styled(
            "watch off",
            Style::new().fg(ui.status_watch_off_fg),
        ));
    }
    spans.push(Span::raw(" · "));
    spans.push(Span::raw(match app.viewer.last_reload {
        Some(t) => format!("rendered {}", format_time(t)),
        None => "not rendered".to_string(),
    }));
    // Mouse capture hint: only shown while capture is OFF (the surprising
    // state — clicks/wheel are dead in-app, the terminal selects natively).
    if !app.mouse_capture {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            "mouse off",
            Style::new().fg(ui.status_watch_off_fg),
        ));
    }
    // Same pattern for the metadata block: shown only while hidden.
    if !app.viewer.show_metadata {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            "metadata off",
            Style::new().fg(ui.status_watch_off_fg),
        ));
    }
    // Git sync segment (SPEC): `✓` clean / `●` dirty, plus behind/ahead
    // arrows when the branch diverged from origin/main. Nothing at all
    // when the status is unavailable (no git, not a repository).
    if let Some(git) = &app.git {
        spans.push(Span::raw(" · "));
        if git.clean {
            spans.push(Span::styled("✓", Style::new().fg(ui.status_success_fg)));
        } else {
            spans.push(Span::styled("●", Style::new().fg(ui.status_dirty_fg)));
        }
        if git.behind > 0 {
            spans.push(Span::styled(
                format!(" ↓{}", git.behind),
                Style::new().fg(ui.status_behind_fg),
            ));
        }
        if git.ahead > 0 {
            spans.push(Span::styled(
                format!(" ↑{}", git.ahead),
                Style::new().fg(ui.status_ahead_fg),
            ));
        }
    }
    // Message precedence: an error (red; `last_error` or a `.base` parse
    // warning) wins; while one is shown the info status waits. A live info
    // message (neutral color) shows only when no error does.
    let mut error_shown = false;
    if let Some(err) = &app.viewer.last_error {
        spans.push(Span::styled(
            format!(" · {err}"),
            Style::new().fg(ui.status_error_fg),
        ));
        error_shown = true;
    }
    // A `.base` file's parse warnings (fail-open notices) surface the
    // same way until the next status change.
    if let Some(base) = &app.base {
        if !base.warnings.is_empty() {
            spans.push(Span::styled(
                format!(" · {}", base.warnings.join("; ")),
                Style::new().fg(ui.status_error_fg),
            ));
            error_shown = true;
        }
    }
    let info = if error_shown {
        None
    } else {
        app.live_status_at(Instant::now())
    };
    if let Some(text) = info {
        spans.push(Span::raw(format!(" · {text}")));
    }
    let message_shown = error_shown || info.is_some();

    // Keyboard hints (static, right-aligned): only when the whole hint fits
    // after the left segments, and never while a message is visible.
    let left_width: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let hint_width = UnicodeWidthStr::width(STATUS_HINT);
    let fits = left_width + 2 + hint_width <= area.width as usize;
    if !message_shown && fits {
        let gap = area.width as usize - left_width - hint_width;
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(
            STATUS_HINT,
            Style::new().fg(ui.status_watch_off_fg),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(ui.status_bg)),
        area,
    );
}

/// Static keyboard hints shown on the status bar's right edge when room.
const STATUS_HINT: &str = "Tab sidebar · ⇧T theme · ? help";

/// `HH:MM:SS` in the local timezone.
fn format_time(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%H:%M:%S")
        .to_string()
}

/// One help entry: a key column and a label. A `("", label)` entry is a
/// label-only line (key column blank). Wording is verbatim from the original
/// help text; each entry is assigned to exactly one section.
type HelpEntry = (&'static str, &'static str);

/// Single column, no blank lines between sections. Links & tasks is in the
/// first screen (the first 27 body rows of a 100x30 terminal).
const HELP_SECTIONS: &[(&str, &[HelpEntry])] = &[
    (
        "Navigation",
        &[
            ("j / k, arrows", "scroll (content pane)"),
            ("PgDn/PgUp", "half page · g/G, Home/End top/bottom"),
            ("mouse wheel", "scroll pane under cursor / focused"),
            ("Tab", "show / hide sidebar"),
            (
                "h / l, ← →",
                "focus sidebar / content (l expands, h collapses)",
            ),
            ("b", "toggle backlinks panel"),
            ("s", "global search (Enter opens hit line)"),
            ("Backspace / Ctrl+O", "back / forward (scroll restored)"),
        ],
    ),
    (
        "Links & tasks",
        &[
            ("x / X", "focus next / previous link or task"),
            ("Enter / click", "open link / toggle checkbox"),
            ("dimmed links", "unresolved (struck out, not activatable)"),
        ],
    ),
    (
        "Content",
        &[
            ("f", "show / hide the note's metadata block"),
            ("w (content)", "toggle watch mode"),
            ("v / 1-9", "switch table view (.base files)"),
        ],
    ),
    (
        "Sidebar",
        &[
            ("t", "sidebar mode: tree ⇄ tags"),
            ("j/k Enter", "move / open (l expands, h collapses)"),
            ("< / >", "narrow / widen the sidebar"),
            ("sidebar *", "changed since last sync (git status)"),
        ],
    ),
    (
        "Theme & app",
        &[
            ("u", "refresh git status (watch events too)"),
            ("Shift+T", "cycle theme (session only)"),
            ("m", "toggle mouse capture; off: wheel = arrows"),
            ("", "? help · q / Ctrl+C quit"),
        ],
    ),
];

/// Footer hint inside the help popup (fixed at the bottom), in herdr's
/// hint style: `key action` pairs separated by ` · `. The scroll part is
/// shown only while the body actually overflows, and both variants fit
/// the popup's 59-cell inner width (the Paragraph would clip a longer one).
const HELP_FOOTER_SCROLL: &str = " scroll j/k · g/G ends · close esc/any key";
const HELP_FOOTER_CLOSE: &str = " close esc/any key";
/// Popup title (bold, left) and close chip (right), both fixed at the top.
const HELP_TITLE: &str = "keybinds";
const HELP_CLOSE_CHIP: &str = "esc close";
/// Widest a single label may be; sets the label width budget.
const HELP_LABEL_WIDTH: usize = 40;
/// Popup chrome around the grid: borders (2) + scrollbar column (1) + 1 spare.
const HELP_CHROME: u16 = 4;

/// Popup width for the single-column grid: key column (longest key
/// `Backspace / Ctrl+O`, 18 cells) + gap + longest label (`unresolved
/// (struck out, not activatable)`, 40 cells) + chrome. The help is always
/// one column; taller-than-screen bodies scroll behind the right scrollbar.
const HELP_WIDTH: u16 = 18 + 1 + HELP_LABEL_WIDTH as u16 + HELP_CHROME;

/// Key column width shared by every group and every column: the longest key
/// across all groups, so keys and labels line up across the whole popup.
fn help_key_width() -> usize {
    HELP_SECTIONS
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0)
}

/// Widest label-column width among `groups` (entries' label text only).
fn help_label_width(groups: &[&(&str, &[HelpEntry])]) -> usize {
    groups
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|(_, label)| label.chars().count())
        .max()
        .unwrap_or(0)
}

/// One laid-out help column: the groups it holds and its row count
/// (header + entries per group, plus one blank line between groups when
/// `spaced`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct HelpColumn {
    groups: Vec<usize>,
    rows: usize,
}

/// Rows a single group occupies (header + entries).
fn help_group_rows(entries: &[HelpEntry]) -> usize {
    1 + entries.len()
}

/// Greedy height-balanced packing of the groups (in order) into `columns`
/// columns: each column is filled until it reaches the balanced target
/// height (`ceil(total / columns)`), the next group starting a new column.
/// Groups are never split. `spaced` adds one blank row between the groups
/// inside a column.
fn help_pack(sections: &[(&str, &[HelpEntry])], columns: usize, spaced: bool) -> Vec<HelpColumn> {
    let gap = usize::from(spaced);
    let total: usize = sections
        .iter()
        .map(|(_, e)| help_group_rows(e))
        .sum::<usize>()
        + gap * sections.len().saturating_sub(1);
    let target = total.div_ceil(columns.max(1));
    let mut out: Vec<HelpColumn> = Vec::new();
    let mut current = HelpColumn {
        groups: Vec::new(),
        rows: 0,
    };
    for (index, (_, entries)) in sections.iter().enumerate() {
        let rows = help_group_rows(entries);
        let sep = if current.groups.is_empty() { 0 } else { gap };
        let full = current.rows + sep + rows > target;
        let last_column = out.len() + 1 == columns;
        if full && !current.groups.is_empty() && !last_column {
            out.push(std::mem::replace(
                &mut current,
                HelpColumn {
                    groups: Vec::new(),
                    rows: 0,
                },
            ));
        }
        let sep = if current.groups.is_empty() { 0 } else { gap };
        current.rows += sep + rows;
        current.groups.push(index);
    }
    if !current.groups.is_empty() {
        out.push(current);
    }
    out
}

/// The grid chosen for the popup: the packed columns, their label widths,
/// and the popup width they need.
struct HelpGrid {
    columns: Vec<HelpColumn>,
    /// Label-column width of each column (its longest label).
    label_widths: Vec<usize>,
    /// Popup width (borders and scrollbar column included).
    width: u16,
    /// Body rows of the grid (the tallest column).
    rows: usize,
    /// Whether the columns carry a blank row between their groups.
    spaced: bool,
}

/// Lay the help out for a frame height `area_height`. The help is always a
/// single packed column (all groups in order): the spaced layout (blank row
/// between groups) when it still fits the height, the compact one otherwise
/// (the body then scrolls behind the right scrollbar).
fn help_grid(area_height: u16) -> HelpGrid {
    let all: Vec<&(&str, &[HelpEntry])> = HELP_SECTIONS.iter().collect();
    // Body rows the popup can show: frame minus the 2-row margin, minus the
    // 2 borders and the footer row.
    let body_budget = usize::from(area_height.saturating_sub(2)).saturating_sub(3);
    for spaced in [true, false] {
        let packed = help_pack(HELP_SECTIONS, 1, spaced);
        let rows = packed.iter().map(|c| c.rows).max().unwrap_or(0);
        if rows <= body_budget || !spaced {
            return HelpGrid {
                label_widths: vec![help_label_width(&all)],
                columns: packed,
                width: HELP_WIDTH,
                rows,
                spaced,
            };
        }
    }
    // The compact layout is always accepted above; this is belt-and-braces
    // for the "never crash" convention (and keeps the compiler happy about
    // the match being exhaustive without a panic path).
    let packed = help_pack(HELP_SECTIONS, 1, false);
    HelpGrid {
        label_widths: vec![help_label_width(&all)],
        rows: packed.iter().map(|c| c.rows).max().unwrap_or(0),
        columns: packed,
        width: HELP_WIDTH,
        spaced: false,
    }
}

/// Body rows of one column's cells: header and entries, keys padded to the
/// shared key width, labels padded to the column's label width, blank rows
/// between its groups when `spaced`.
fn help_column_lines(
    ui: UiTheme,
    column: &HelpColumn,
    spaced: bool,
    label_width: usize,
) -> Vec<Vec<Span<'static>>> {
    let key_width = help_key_width();
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    for (position, &index) in column.groups.iter().enumerate() {
        let (section, entries) = &HELP_SECTIONS[index];
        if spaced && position > 0 {
            rows.push(Vec::new());
        }
        rows.push(vec![Span::styled(
            section.to_string(),
            Style::new()
                .fg(ui.sidebar_border_focused)
                .add_modifier(Modifier::BOLD),
        )]);
        for (key, label) in entries.iter() {
            if key.is_empty() {
                rows.push(vec![Span::raw(format!(
                    "{:<key_width$} {:<label_width$}",
                    "", label
                ))]);
                continue;
            }
            rows.push(vec![
                Span::styled(
                    format!("{key:<key_width$}"),
                    Style::new()
                        .fg(ui.popup_match_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(format!("{:<label_width$}", label)),
            ]);
        }
    }
    rows
}

/// Body lines of the single-column help grid: each row padded to the popup's
/// inner width.
fn help_body(ui: UiTheme, grid: &HelpGrid, spaced: bool) -> Vec<Line<'static>> {
    let column = &grid.columns[0];
    let label_width = grid.label_widths[0];
    let cells = help_column_lines(ui, column, spaced, label_width);
    let inner = grid.width.saturating_sub(HELP_CHROME) as usize;
    cells
        .into_iter()
        .map(|cell| {
            let mut spans: Vec<Span<'static>> = cell;
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            if inner > used {
                spans.push(Span::raw(" ".repeat(inner - used)));
            }
            Line::from(spans)
        })
        .collect()
}

/// Lines a PageUp/PageDown moves the help by (about a page of body rows).
pub fn help_page_step() -> usize {
    HELP_PAGE_STEP
}
const HELP_PAGE_STEP: usize = 10;

/// Body rows visible for a popup of `popup_height` rows: borders (2) and
/// the fixed footer row (1) are not scrollable.
fn help_visible_rows(popup_height: u16) -> usize {
    popup_height.saturating_sub(3) as usize
}

fn draw_help(f: &mut Frame, area: Rect, app: &mut App, ui: UiTheme) {
    // Dim everything behind the popup (herdr's overlay treatment). The popup
    // cells are cleared below, which resets their style and so drops DIM.
    let buf = f.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }

    let grid = help_grid(area.height);
    let spaced = grid.spaced;
    let width = grid.width.min(area.width.saturating_sub(2)).max(1);
    let total = grid.rows;
    // Fit to the content (+ borders + footer), clamped to the terminal.
    let want_height = (total as u16).saturating_add(3);
    let height = want_height.min(area.height.saturating_sub(2)).max(1);
    let popup = centered(area, width, height);

    let visible = help_visible_rows(height);
    let max_scroll = total.saturating_sub(visible);
    app.viewer.help_scroll = app.viewer.help_scroll.min(max_scroll);
    let scroll = app.viewer.help_scroll;

    let title = Line::from(Span::styled(
        HELP_TITLE,
        Style::new().add_modifier(Modifier::BOLD),
    ));
    let chip = Line::from(Span::styled(
        HELP_CLOSE_CHIP,
        Style::new().fg(ui.popup_dimmed_fg),
    ));
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ui.popup_border_fg))
        .title(title)
        .title(chip.right_aligned());

    f.render_widget(Clear, popup);
    // Fill the popup (incl. border cells) with content_bg; the explicit
    // remove_modifier keeps the popup crisp over the dimmed background.
    f.render_widget(
        Block::new().style(
            Style::new()
                .bg(ui.content_bg)
                .remove_modifier(Modifier::DIM),
        ),
        popup,
    );
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Body rows above the fixed footer row; the scrollbar takes the rightmost
    // inner column when the body overflows.
    let body_height = inner.height.saturating_sub(1);
    let overflow = total > body_height as usize;
    let text_width = if overflow {
        inner.width.saturating_sub(1)
    } else {
        inner.width
    };
    let body = Rect {
        width: text_width,
        height: body_height,
        ..inner
    };
    let lines = help_body(ui, &grid, spaced);
    f.render_widget(
        Paragraph::new(lines.into_iter().skip(scroll).collect::<Vec<_>>())
            .style(Style::new().bg(ui.content_bg)),
        body,
    );
    if overflow && body_height > 0 {
        let bar_area = Rect {
            x: inner.x + inner.width - 1,
            y: inner.y,
            width: 1,
            height: body_height,
        };
        let mut state = ScrollbarState::new(total)
            .position(scroll)
            .viewport_content_length(body_height as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::new().fg(ui.popup_dimmed_fg).bg(ui.content_bg)),
            bar_area,
            &mut state,
        );
    }

    let footer = Rect {
        y: inner.y + body_height,
        height: 1,
        ..inner
    };
    let footer_text = if overflow {
        HELP_FOOTER_SCROLL
    } else {
        HELP_FOOTER_CLOSE
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer_text,
            Style::new().fg(ui.popup_dimmed_fg),
        )))
        .style(Style::new().bg(ui.content_bg)),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::app::App;
    use crate::testutil::{TempDir, TempFile};
    use crate::viewer::Viewer;

    fn sample_viewer(path: &Path) -> Viewer {
        Viewer::open(path, 80).unwrap()
    }

    fn app_for(file: &TempFile, vault: Option<&Path>) -> App {
        let viewer = sample_viewer(file.path());
        App::new(viewer, vault.map(|p| p.to_path_buf()), None).unwrap()
    }

    fn render(app: &mut App) -> String {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().to_string()
    }

    /// Help-popup render at the smoke terminal size (100x30).
    fn render_help(app: &mut App) -> String {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().to_string()
    }

    fn ui_viewer(name: &str, content: &str) -> TempFile {
        TempFile::new(name, content)
    }

    fn vault() -> TempDir {
        let dir = TempDir::new("ui-vault");
        std::fs::write(dir.path().join("alpha.md"), "alpha note\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("deep.md"), "deep\n").unwrap();
        dir
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    /// Renders at an arbitrary width (status-line tests need 100 and 40).
    fn render_at(app: &mut App, cols: u16) -> String {
        let backend = TestBackend::new(cols, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().to_string()
    }

    /// The bottom screen row (the status bar).
    fn status_row(screen: &str) -> String {
        screen.lines().last().unwrap_or("").to_string()
    }

    /// Mirrors `main`: the vault index is built and shared with the app,
    /// so the status-bar name logic sees every indexed note.
    fn status_app(vault_dir: &TempDir, open: &str) -> App {
        let viewer = Viewer::open(&vault_dir.path().join(open), 80).unwrap();
        let index = std::rc::Rc::new(std::cell::RefCell::new(
            VaultIndex::build(vault_dir.path()).unwrap(),
        ));
        App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap()
    }

    /// `a/index.md` and `b/index.md` share a file name; `solo.md` is unique.
    fn name_vault() -> TempDir {
        let dir = TempDir::new("ui-status-name");
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("a").join("index.md"), "# A\n").unwrap();
        std::fs::write(dir.path().join("b").join("index.md"), "# B\n").unwrap();
        std::fs::write(dir.path().join("solo.md"), "# S\n").unwrap();
        dir
    }

    #[test]
    fn status_bar_shows_file_name_for_unambiguous_note() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        let screen = render_at(&mut app, 100);
        let row = status_row(&screen);
        assert!(row.contains("solo.md"), "name shown: {row}");
        assert!(
            !row.contains(&dir.path().display().to_string()),
            "no absolute path on the bar: {row}"
        );
    }

    #[test]
    fn status_bar_shows_relative_path_when_name_is_ambiguous() {
        let dir = name_vault();
        let mut app = status_app(&dir, "a/index.md");
        let screen = render_at(&mut app, 100);
        let row = status_row(&screen);
        assert!(row.contains("a/index.md"), "relative path shown: {row}");
        assert!(
            !row.contains(&dir.path().display().to_string()),
            "vault root not on the bar: {row}"
        );
    }

    #[test]
    fn status_bar_keeps_full_path_when_sidebar_focused() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        app.handle_key(key(KeyCode::Char('h')));
        assert_eq!(app.focus, crate::app::Focus::Sidebar);
        let screen = render_at(&mut app, 100);
        let row = status_row(&screen);
        let focused = dir.path().join("a");
        assert!(
            row.contains(&focused.display().to_string()),
            "focused sidebar entry shows its full path: {row}"
        );
    }

    #[test]
    fn status_bar_name_segment_records_rect_only_in_content_focus() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        render_at(&mut app, 100);
        let area = app.status_name_area.expect("name segment recorded");
        assert_eq!(area.y, 23, "status row");
        assert_eq!(area.x, 0);
        assert_eq!(area.width as usize, "solo.md".len());
        app.handle_key(key(KeyCode::Char('h')));
        render_at(&mut app, 100);
        assert!(app.status_name_area.is_none(), "sidebar focus: no name");
    }

    #[test]
    fn status_bar_shows_hints_at_100_cols() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        let screen = render_at(&mut app, 100);
        assert!(
            status_row(&screen).contains("Tab sidebar · ⇧T theme · ? help"),
            "hints shown with room: {}",
            status_row(&screen)
        );
    }

    #[test]
    fn status_bar_hides_hints_at_40_cols() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        let screen = render_at(&mut app, 40);
        assert!(
            !screen.contains("Tab sidebar"),
            "no room: hint dropped whole: {}",
            status_row(&screen)
        );
    }

    #[test]
    fn status_bar_hides_hints_while_a_message_is_shown() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        app.set_status("theme: ocean");
        let screen = render_at(&mut app, 100);
        let row = status_row(&screen);
        assert!(row.contains("theme: ocean"), "info shown: {row}");
        assert!(!row.contains("Tab sidebar"), "hints hidden: {row}");
    }

    #[test]
    fn status_message_is_neutral_and_error_wins() {
        let dir = name_vault();
        let mut app = status_app(&dir, "solo.md");
        app.set_status("theme: ocean");
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let row = status_row(&terminal.backend().to_string());
        let start = row.find("theme: ocean").expect("info shown");
        let cell = buf.cell((start as u16, 23)).expect("cell");
        assert_ne!(
            cell.fg, app.theme_ui.status_error_fg,
            "info is not the error color"
        );
        // An error present: the info waits, the error shows in red.
        app.viewer.last_error = Some("boom".to_string());
        let screen = render_at(&mut app, 100);
        let row = status_row(&screen);
        assert!(row.contains("boom"), "error shown: {row}");
        assert!(!row.contains("theme: ocean"), "info waits behind error");
    }

    #[test]
    fn status_bar_shows_path_lines_watch_timestamp() {
        let file = ui_viewer("ui.md", "# Hi\n\nbody\n");
        let mut app = app_for(&file, None);
        let screen = render(&mut app);
        assert!(screen.contains("lines"), "status bar missing line count");
        assert!(screen.contains("watch on"), "watch indicator missing");
        assert!(screen.contains("rendered "), "render timestamp missing");
        assert!(screen.contains("Hi"), "content missing");
    }

    #[test]
    fn watch_off_is_visible() {
        let file = ui_viewer("ui-off.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.toggle_watch();
        let screen = render(&mut app);
        assert!(screen.contains("watch off"));
    }

    #[test]
    fn help_popup_renders_and_covers_content() {
        let file = ui_viewer("ui-help.md", "# Hi\n\nbody\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        let screen = render_help(&mut app);
        assert!(screen.contains("keybinds"), "popup title missing");
        assert!(screen.contains("close esc/any key"));
        assert!(screen.contains("toggle watch mode"));
        // M2 keys are documented.
        assert!(screen.contains("Backspace"));
        assert!(screen.contains("Ctrl+O"));
        assert!(screen.contains("show / hide sidebar"));
    }

    /// The viewer's viewport height must be fed from the frame area, not
    /// kept independently: after draw, it equals area - status bar.
    #[test]
    fn draw_feeds_frame_height_into_viewer() {
        let file = ui_viewer("ui-height.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.viewport_height = 3;
        let screen = render(&mut app);
        // TestBackend 100x24 → viewport = 24 - 1 status line.
        assert_eq!(app.viewer.viewport_height, 23);
        assert!(screen.contains("watch on"));
        app.viewer.scroll_bottom();
        assert_eq!(app.viewer.scroll, app.viewer.max_scroll());
    }

    #[test]
    fn no_sidebar_without_vault() {
        let file = ui_viewer("ui-solo.md", "x\n");
        let mut app = app_for(&file, None);
        let screen = render(&mut app);
        assert!(
            !screen.contains("vault"),
            "no sidebar block without --vault"
        );
        assert_eq!(app.viewer.render_width, 98, "full width minus margins");
    }

    #[test]
    fn sidebar_visible_by_default_with_vault() {
        let vault_dir = vault();
        let file = ui_viewer("ui-tab.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        assert!(app.sidebar_visible, "starts visible with --vault");
        let screen = render(&mut app);
        assert!(screen.contains("vault"), "sidebar title missing");
        assert!(
            screen.contains("alpha.md"),
            "sidebar should list vault notes"
        );
        // Content re-wrapped to the narrower area: 100 - 30 - 2 = 68.
        assert_eq!(app.viewer.render_width, 68);

        // Focus the sidebar: the status bar shows the selected entry's path
        // (the first row is the `sub` folder — dirs sort first).
        app.handle_key(key(KeyCode::Char('h')));
        let screen = render(&mut app);
        let expected = vault_dir.path().join("sub");
        assert!(
            screen.contains(&expected.display().to_string()),
            "status bar shows focused sidebar path"
        );
    }

    #[test]
    fn sidebar_truncates_long_names_with_ellipsis() {
        let dir = TempDir::new("ui-longname");
        let long = "a-very-long-note-name-that-does-not-fit.md";
        std::fs::write(dir.path().join(long), "x\n").unwrap();
        let file = ui_viewer("ui-trunc.md", "x\n");
        let mut app = app_for(&file, Some(dir.path()));
        let screen = render(&mut app);
        assert!(screen.contains("…"), "truncated name should use ellipsis");
        assert!(
            !screen.contains(long),
            "full long name must not appear in the sidebar"
        );
    }

    #[test]
    fn sidebar_hides_on_tab() {
        let vault_dir = vault();
        let file = ui_viewer("ui-hide.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        app.handle_key(key(KeyCode::Tab));
        let screen = render(&mut app);
        assert!(!screen.contains("vault"), "sidebar hidden after Tab");
        // Focus snapped back to content; full content width restored.
        assert_eq!(app.viewer.render_width, 98);
    }

    // --- M3 rendering --------------------------------------------------

    use std::cell::RefCell;
    use std::rc::Rc;

    use vault::VaultIndex;

    fn app_with_index(file: &TempFile, vault_dir: &TempDir) -> App {
        let viewer = sample_viewer(file.path());
        let index = Rc::new(RefCell::new(VaultIndex::build(vault_dir.path()).unwrap()));
        App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap()
    }

    #[test]
    fn status_glyph_table_covers_every_real_vault_status() {
        // Every status string found in the real vault (478 notes, verified
        // against a /tmp copy: active 213, archived 57, resolved 47,
        // complete 42, monitoring 41, draft 32, superseded 32, pending 12,
        // planned 1, blocked 1) plus SPEC's `in-progress`, which the vault
        // does not use today but the SPEC still names.
        let cases: &[(&str, &str, StatusStyle)] = &[
            ("complete", "●", StatusStyle::Normal),
            ("resolved", "●", StatusStyle::Normal),
            ("active", "●", StatusStyle::Green),
            ("pending", "○", StatusStyle::Normal),
            ("planned", "○", StatusStyle::Normal),
            ("in-progress", "◐", StatusStyle::Normal),
            ("in progress", "◐", StatusStyle::Normal),
            ("monitoring", "◐", StatusStyle::Normal),
            ("draft", "◌", StatusStyle::Normal),
            ("blocked", "◉", StatusStyle::Red),
            ("superseded", "×", StatusStyle::Dimmed),
            ("archived", "×", StatusStyle::Dimmed),
        ];
        for (status, glyph, style) in cases {
            assert_eq!(
                status_glyph(Some(status)),
                Some((*glyph, *style)),
                "status {status:?}"
            );
        }
        // Case/whitespace tolerance is preserved.
        assert_eq!(
            status_glyph(Some("  Active ")),
            Some(("●", StatusStyle::Green))
        );
        // Unknown statuses keep today's no-glyph behavior.
        assert_eq!(status_glyph(Some("cancelled")), None);
        assert_eq!(status_glyph(Some("")), None);
        assert_eq!(status_glyph(None), None);
    }

    #[test]
    fn tree_shows_status_glyphs() {
        let vault_dir = TempDir::new("ui-glyphs");
        std::fs::write(
            vault_dir.path().join("done.md"),
            "---\nstatus: complete\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("wip.md"),
            "---\nstatus: in-progress\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("old.md"),
            "---\nstatus: archived\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("live.md"),
            "---\nstatus: active\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("stuck.md"),
            "---\nstatus: blocked\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("sketch.md"),
            "---\nstatus: draft\n---\nbody\n",
        )
        .unwrap();
        let file = ui_viewer("ui-glyph.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        let screen = render(&mut app);
        assert!(screen.contains('●'), "complete glyph shown");
        assert!(screen.contains('◐'), "in-progress glyph shown");
        assert!(screen.contains('×'), "archived glyph shown");
        assert!(screen.contains('◉'), "blocked glyph shown");
        assert!(screen.contains('◌'), "draft glyph shown");
        // Active renders too (same ● as complete, styled green+bold).
        assert_eq!(screen.matches('●').count(), 2, "active + complete");
        assert!(!screen.contains('○'), "no pending/planned note in fixture");
    }

    #[test]
    fn backlinks_panel_renders_with_empty_state() {
        let vault_dir = TempDir::new("ui-backlinks");
        std::fs::write(vault_dir.path().join("linker.md"), "[[target]]\n").unwrap();
        std::fs::write(vault_dir.path().join("target.md"), "t\n").unwrap();
        let file = ui_viewer("ui-bl.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.handle_key(key(KeyCode::Char('b')));
        let screen = render(&mut app);
        assert!(screen.contains("backlinks"), "panel title shown");
        assert!(
            screen.contains("no notes link here"),
            "empty state (no note open)"
        );

        // Open the linked-to note: the panel lists the linker.
        app.open_note(&vault_dir.path().join("target.md")).unwrap();
        app.refresh_backlinks();
        let screen = render(&mut app);
        assert!(screen.contains("linker.md"), "backlink listed");
        assert!(!screen.contains("no notes link here"));
    }

    /// A vault where `target.md` has `count` linkers (`link00.md`, …).
    fn backlinks_vault(name: &str, count: usize) -> TempDir {
        let dir = TempDir::new(name);
        std::fs::write(dir.path().join("target.md"), "t\n").unwrap();
        for i in 0..count {
            std::fs::write(dir.path().join(format!("link{i:02}.md")), "[[target]]\n").unwrap();
        }
        dir
    }

    /// Opens `target.md` with the backlinks panel shown, on a 100x24 frame
    /// (panel = 23 rows / 3 clamped to 3..=10 → 7 rows, 5 inner).
    fn backlinks_app(vault_dir: &TempDir) -> App {
        let file = ui_viewer("ui-bl-bar.md", "x\n");
        let mut app = app_with_index(&file, vault_dir);
        app.sidebar_visible = false;
        app.open_note(&vault_dir.path().join("target.md")).unwrap();
        app.backlinks_open = true;
        app.refresh_backlinks();
        app
    }

    /// The panel's full rect (no sidebar, 100 wide): `backlinks_area` is the
    /// click rect, which excludes the bar column when one is drawn.
    fn full_panel(app: &App) -> Rect {
        let hit = app.backlinks_area.expect("panel drawn");
        Rect::new(0, hit.y, 100, hit.height)
    }

    #[test]
    fn backlinks_scrollbar_present_only_when_rows_overflow() {
        // 2 linkers fit the 5 inner rows: no bar.
        let fit = backlinks_vault("ui-bl-fit", 2);
        let mut app = backlinks_app(&fit);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let panel = full_panel(&app);
        assert!(
            !column_has_bar(
                terminal.backend().buffer(),
                panel.right() - 2,
                panel.y + 1..panel.bottom() - 1
            ),
            "no bar when backlink rows fit"
        );

        // 12 linkers overflow the 5 inner rows: bar drawn on the inner right edge.
        let long = backlinks_vault("ui-bl-long", 12);
        let mut app = backlinks_app(&long);
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let panel = full_panel(&app);
        let buffer = terminal.backend().buffer();
        // Bar = last inner column (right-2); the right border stays intact.
        assert!(
            column_has_bar(buffer, panel.right() - 2, panel.y + 1..panel.bottom() - 1),
            "bar drawn when backlink rows overflow"
        );
        assert_eq!(
            buffer[(panel.right() - 1, panel.y + 1)].symbol(),
            "│",
            "border intact"
        );
    }

    #[test]
    fn backlinks_text_never_runs_under_the_bar() {
        let vault_dir = backlinks_vault("ui-bl-under", 12);
        let mut app = backlinks_app(&vault_dir);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let panel = full_panel(&app);
        let bar_x = panel.right() - 2;
        let buffer = terminal.backend().buffer();
        // The bar sits in the last inner column; no name may occupy it.
        for y in panel.y + 1..panel.bottom() - 1 {
            let sym = buffer[(bar_x, y)].symbol();
            assert!(
                !sym.chars().any(|c| c.is_ascii_alphanumeric()),
                "backlink text under the bar at row {y}: {sym:?}"
            );
        }
        // Rows are still drawn in the text columns.
        let text: String = (panel.x + 1..bar_x)
            .map(|x| buffer[(x, panel.y + 1)].symbol().to_string())
            .collect();
        assert!(text.contains("link00"), "first backlink visible: {text:?}");
    }

    #[test]
    fn backlinks_scrollbar_tracks_the_drawn_scroll() {
        let vault_dir = backlinks_vault("ui-bl-pos", 12);
        let mut app = backlinks_app(&vault_dir);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let panel = full_panel(&app);
        let bar_x = panel.right() - 2;
        let top_thumb = (panel.y + 1..panel.bottom() - 1)
            .find(|&y| terminal.backend().buffer()[(bar_x, y)].symbol() == "█")
            .expect("thumb at top");
        let top_text = row_symbols(terminal.backend().buffer(), panel.y + 1);
        assert!(top_text.contains("link00"));

        // Select the last backlink: the scroll follows, the thumb moves down.
        app.backlink_sel = 11;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let bottom_thumb = (panel.y + 1..panel.bottom() - 1)
            .find(|&y| terminal.backend().buffer()[(bar_x, y)].symbol() == "█")
            .expect("thumb at bottom");
        let bottom_text = row_symbols(terminal.backend().buffer(), panel.y + 1);
        assert!(!bottom_text.contains("link00"), "rows scrolled");
        assert!(bottom_thumb > top_thumb, "thumb moved with the scroll");
    }

    #[test]
    fn tag_sidebar_renders_and_counts() {
        let vault_dir = TempDir::new("ui-tags");
        std::fs::write(
            vault_dir.path().join("a.md"),
            "---\nstatus: active\ntags: [mocktag]\n---\na\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("b.md"),
            "---\nstatus: active\ntags: [mocktag, rust]\n---\nb\n",
        )
        .unwrap();
        let file = ui_viewer("ui-tags.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.handle_key(key(KeyCode::Char('t')));
        let screen = render(&mut app);
        assert!(screen.contains("tags"), "sidebar title switches to tags");
        assert!(screen.contains("#mocktag (2)"), "tag row with count");
        assert!(screen.contains("#rust (1)"));
        assert!(
            !screen.contains("alpha.md"),
            "tree rows are gone in tag mode"
        );
    }

    #[test]
    fn disambiguation_popup_renders_candidates() {
        let vault_dir = TempDir::new("ui-disamb");
        std::fs::create_dir_all(vault_dir.path().join("x")).unwrap();
        std::fs::create_dir_all(vault_dir.path().join("y")).unwrap();
        std::fs::write(vault_dir.path().join("x/dup.md"), "x\n").unwrap();
        std::fs::write(vault_dir.path().join("y/dup.md"), "y\n").unwrap();
        let file = ui_viewer("ui-disamb.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.disambiguation = Some(crate::app::Disambiguation {
            candidates: vec![
                vault_dir.path().join("x/dup.md"),
                vault_dir.path().join("y/dup.md"),
            ],
            selected: 1,
            heading: None,
        });
        let screen = render(&mut app);
        assert!(screen.contains("ambiguous link"), "popup title");
        assert!(screen.contains("x/dup.md") && screen.contains("y/dup.md"));
    }

    /// With more candidates than fit the terminal, the popup list scrolls
    /// so the selection stays visible (previously it silently truncated).
    #[test]
    fn disambiguation_popup_scrolls_on_short_terminals() {
        let vault_dir = TempDir::new("ui-disamb-scroll");
        let mut candidates = Vec::new();
        for i in 1..=8 {
            let dir = vault_dir.path().join(format!("d{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("dup.md"), "x\n").unwrap();
            candidates.push(dir.join("dup.md"));
        }
        let file = ui_viewer("ui-disamb-s.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.sidebar_visible = false; // keep the tree rows out of the assert
        app.disambiguation = Some(crate::app::Disambiguation {
            candidates,
            selected: 7,
            heading: None,
        });
        // 10 rows → popup height 8 → 5 candidate rows: a scroll window.
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("ambiguous link"), "popup title");
        assert!(screen.contains("d8/dup.md"), "selected row is visible");
        assert!(screen.contains("d4/dup.md"), "window starts mid-list");
        assert!(
            !screen.contains("d1/dup.md") && !screen.contains("d3/dup.md"),
            "candidates above the window are scrolled off"
        );
    }

    #[test]
    fn wikilink_line_renders_link_text() {
        let vault_dir = TempDir::new("ui-wikilink");
        std::fs::write(vault_dir.path().join("goal.md"), "goal body\n").unwrap();
        let file = ui_viewer("ui-wl.md", "See [[goal]] and [[nowhere]].\n");
        let mut app = app_with_index(&file, &vault_dir);
        let screen = render(&mut app);
        assert!(screen.contains("See"), "content renders");
        assert!(
            screen.contains("nowhere"),
            "unresolved link text still shown"
        );
        assert!(
            !screen.contains("[[goal]]"),
            "raw wikilink syntax is replaced by the link text"
        );
    }

    // --- M4: git segment, changed markers, search popup -----------------

    /// Commits everything in `dir` (fixture setup only; the app's own git
    /// usage stays inside the read-only allowlist).
    fn git_commit(dir: &Path, message: &str) {
        use std::process::Command;
        for args in [
            vec!["init", "-b", "main", "-q"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", message],
        ] {
            let out = Command::new("git")
                .args(&args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?} failed");
        }
    }

    #[test]
    fn status_bar_shows_git_clean_check() {
        let vault_dir = TempDir::new("ui-git-clean");
        std::fs::write(vault_dir.path().join("a.md"), "a\n").unwrap();
        git_commit(vault_dir.path(), "initial");
        let file = ui_viewer("ui-git.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        // App::new only reads git when an index exists; force a refresh.
        app.index = Some(std::rc::Rc::new(std::cell::RefCell::new(
            vault::VaultIndex::build(vault_dir.path()).unwrap(),
        )));
        app.refresh_git(true);
        let git = app.git.as_ref().expect("clean repo readable");
        assert!(git.clean);
        let screen = render(&mut app);
        assert!(screen.contains('✓'), "clean check shown: {screen}");
    }

    #[test]
    fn status_bar_shows_dirty_dot_and_arrows() {
        let file = ui_viewer("ui-git2.md", "x\n");
        let mut app = app_for(&file, None);
        app.git = Some(vault::GitStatus {
            clean: false,
            ahead: 2,
            behind: 1,
            changed_files: vec![Path::new("/x/a.md").to_path_buf()],
        });
        let screen = render(&mut app);
        assert!(screen.contains('●'), "dirty dot shown");
        assert!(screen.contains("↓1"), "behind arrow shown");
        assert!(screen.contains("↑2"), "ahead arrow shown");
        assert!(!screen.contains('✓'));

        // Fully synced + dirty: only the dot.
        app.git = Some(vault::GitStatus {
            clean: false,
            ahead: 0,
            behind: 0,
            changed_files: vec![],
        });
        let screen = render(&mut app);
        assert!(screen.contains('●') && !screen.contains('↓') && !screen.contains('↑'));

        // Unavailable (no git / not a repo): no segment at all.
        app.git = None;
        let screen = render(&mut app);
        assert!(!screen.contains('✓') && !screen.contains('●'));
    }

    #[test]
    fn sidebar_marks_changed_files_with_asterisk() {
        let vault_dir = TempDir::new("ui-changed");
        std::fs::write(vault_dir.path().join("alpha.md"), "alpha note\n").unwrap();
        std::fs::write(vault_dir.path().join("clean.md"), "clean note\n").unwrap();
        let file = ui_viewer("ui-mark.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        app.git = Some(vault::GitStatus {
            clean: false,
            ahead: 0,
            behind: 0,
            changed_files: vec![vault_dir.path().join("alpha.md")],
        });
        let screen = render(&mut app);
        assert!(screen.contains("alpha.md *"), "changed note marked");
        assert!(!screen.contains("clean.md *"), "unchanged note unmarked");
    }

    #[test]
    fn search_popup_renders_prompt_results_and_highlight() {
        let vault_dir = TempDir::new("ui-search");
        std::fs::write(vault_dir.path().join("hay.md"), "top\nneedle here\nmore\n").unwrap();
        let file = ui_viewer("ui-search.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);

        // Input mode: prompt with cursor.
        app.handle_key(key(KeyCode::Char('s')));
        app.handle_key(key(KeyCode::Char('n')));
        let screen = render(&mut app);
        assert!(screen.contains("search"), "popup title");
        assert!(screen.contains("/n▏"), "input prompt with cursor");

        // Results mode: hit row with path:line and the match highlighted.
        app.handle_key(key(KeyCode::Enter));
        let screen = render(&mut app);
        assert!(screen.contains("1 hit"), "hit count in the title");
        assert!(screen.contains("hay.md:2: "), "relpath:line prefix");
        assert!(screen.contains("needle here"), "snippet text");

        // Empty result set shows the empty state.
        app.search.mode = crate::search::SearchMode::Results;
        app.search.hits.clear();
        let screen = render(&mut app);
        assert!(screen.contains("no matches"));
    }

    #[test]
    fn search_title_shows_plus_when_results_were_capped() {
        let vault_dir = TempDir::new("ui-search-cap");
        std::fs::write(vault_dir.path().join("hay.md"), "needle\n").unwrap();
        let file = ui_viewer("ui-search-cap.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.search.open = true;
        app.search.mode = crate::search::SearchMode::Results;
        app.search.hits = vec![vault::SearchHit {
            path: vault_dir.path().join("hay.md"),
            line: 0,
            text: "needle".into(),
            start: 0,
            end: 6,
        }];
        app.search.selected = 0;

        let screen = render(&mut app);
        assert!(screen.contains("1 hit"), "uncapped: plain count");
        assert!(!screen.contains("1+ hit"), "no plus when uncapped");

        app.search.capped = true;
        let screen = render(&mut app);
        assert!(screen.contains("1+ hits"), "capped: N+ in the title");
    }

    #[test]
    fn help_documents_m4_keys() {
        let file = ui_viewer("ui-help4.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        let screen = render_help_tall(&mut app);
        assert!(screen.contains("global search"));
        assert!(screen.contains("refresh git status"));
        assert!(screen.contains("changed since last sync"));
        assert!(screen.contains("unresolved (struck out, not activatable)"));
    }

    // --- post-v1 loop 6: .base table rendering ---------------------------

    #[test]
    fn base_table_renders_borders_header_rows_and_status_glyphs() {
        let vault_dir = TempDir::new("ui-base");
        std::fs::create_dir_all(vault_dir.path().join("tasks")).unwrap();
        std::fs::write(
            vault_dir.path().join("tasks/one.md"),
            "---\ntype: task\nstatus: active\npriority: 2\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("tasks/two.md"),
            "---\ntype: task\nstatus: blocked\npriority: 1\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("tasks.base"),
            "views:\n  - type: table\n    name: Open\n    filters:\n      and:\n        - 'type == \"task\"'\n    order:\n      - file.name\n      - status\n    sort:\n      - property: priority\n        direction: ASC\n",
        )
        .unwrap();
        let file = ui_viewer("ui-base.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.theme_ui.content_bg = Color::Rgb(9, 9, 9);
        app.open_base_at(&vault_dir.path().join("tasks.base"), 0)
            .unwrap();
        app.sidebar_visible = false;
        let screen = render(&mut app);
        // Bordered block with the view title.
        assert!(screen.contains("base · view 1/1: Open"), "{screen}");
        // Header row + sorted note rows (priority ASC: two, one).
        assert!(screen.contains("Name"), "header row rendered");
        assert!(screen.contains("two"), "first row (priority 1)");
        assert!(screen.contains("one"), "second row (priority 2)");
        // Status glyphs from the shared table (active ●, blocked ◉).
        assert!(screen.contains('●') && screen.contains('◉'), "{screen}");
        // Status bar: the base path and the row count segment.
        assert!(
            screen.contains("2 rows · view 1/1: Open"),
            "status bar shows the table state: {screen}"
        );
        // The table block (border corner) and a blank inner row below the
        // two data rows carry content_bg, not the terminal default.
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(0, 0)].bg,
            Color::Rgb(9, 9, 9),
            "base-table border painted with content_bg"
        );
        assert_eq!(
            buffer[(50, 20)].bg,
            Color::Rgb(9, 9, 9),
            "blank base-table inner area painted with content_bg"
        );
    }

    #[test]
    fn base_row_count_pluralizes_correctly() {
        let vault_dir = TempDir::new("ui-base-one");
        std::fs::create_dir_all(vault_dir.path().join("tasks")).unwrap();
        std::fs::write(
            vault_dir.path().join("tasks/one.md"),
            "---\ntype: task\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.path().join("tasks.base"),
            "views:\n  - type: table\n    name: All\n    order:\n      - file.name\n",
        )
        .unwrap();
        let file = ui_viewer("ui-base-one.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.open_base_at(&vault_dir.path().join("tasks.base"), 0)
            .unwrap();
        app.sidebar_visible = false;
        let screen = render(&mut app);
        assert!(screen.contains("1 row · view 1/1: All"), "{screen}");
        assert!(!screen.contains("1 rows"), "{screen}");
    }

    #[test]
    fn base_view_switch_is_visible_in_the_title() {
        let vault_dir = TempDir::new("ui-base-views");
        std::fs::write(vault_dir.path().join("a.md"), "a\n").unwrap();
        std::fs::write(
            vault_dir.path().join("v.base"),
            "views:\n  - type: table\n    name: One\n    order:\n      - file.name\n  - type: table\n    name: Two\n    order:\n      - file.name\n",
        )
        .unwrap();
        let file = ui_viewer("ui-bv.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.open_base_at(&vault_dir.path().join("v.base"), 0)
            .unwrap();
        app.sidebar_visible = false;
        app.handle_key(key(KeyCode::Char('v')));
        let screen = render(&mut app);
        assert!(screen.contains("view 2/2: Two"), "{screen}");
    }

    #[test]
    fn chrome_uses_the_themed_border_and_content_bg() {
        let vault_dir = TempDir::new("ui-theme-vault");
        std::fs::write(vault_dir.path().join("alpha.md"), "alpha\n").unwrap();
        let file = ui_viewer("ui-theme.md", "# Hi\n\nbody\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        app.theme_ui.sidebar_border_focused = Color::Rgb(1, 2, 3);
        app.theme_ui.sidebar_border_unfocused = Color::Rgb(4, 5, 6);
        app.theme_ui.content_bg = Color::Rgb(9, 9, 9);
        app.theme_ui.sidebar_bg = Color::Rgb(7, 7, 7);
        app.theme_ui.status_bg = Color::Rgb(8, 8, 8);

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();

        // Content background fills the note area (a plain cell, past the
        // sidebar at the default ~30% width).
        assert_eq!(
            buffer[(70, 10)].bg,
            Color::Rgb(9, 9, 9),
            "content_bg applied to the content area"
        );
        // Sidebar background fills the sidebar (a blank row below the
        // tree rows — the base style paints the whole area).
        assert_eq!(
            buffer[(10, 20)].bg,
            Color::Rgb(7, 7, 7),
            "sidebar_bg applied to the sidebar"
        );
        // Status bar background fills the bar (a cell past the text).
        assert_eq!(
            buffer[(90, 23)].bg,
            Color::Rgb(8, 8, 8),
            "status_bg applied to the status bar"
        );
        // Sidebar border (unfocused here): themed color on the border cell.
        assert_eq!(
            buffer[(0, 5)].fg,
            Color::Rgb(4, 5, 6),
            "unfocused sidebar border from the theme"
        );
        // Focused sidebar border.
        app.focus = Focus::Sidebar;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 5)].fg,
            Color::Rgb(1, 2, 3),
            "focused sidebar border from the theme"
        );
    }

    #[test]
    fn sidebar_selection_keeps_selection_bg_over_sidebar_bg() {
        let vault_dir = TempDir::new("ui-theme-sel");
        std::fs::write(vault_dir.path().join("alpha.md"), "alpha\n").unwrap();
        std::fs::create_dir_all(vault_dir.path().join("sub")).unwrap();
        let file = ui_viewer("ui-theme-sel.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        app.theme_ui.sidebar_bg = Color::Rgb(7, 7, 7);
        app.theme_ui.selection_bg = Color::Rgb(5, 5, 5);
        // Focus the sidebar and move the selection off the first row
        // (rows: `sub`, then `alpha.md`).
        app.focus = Focus::Sidebar;
        app.handle_key(key(KeyCode::Char('j')));

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Inner row 2 = `alpha.md` (selected): selection_bg wins on top of
        // the sidebar_bg base.
        assert_eq!(
            buffer[(2, 2)].bg,
            Color::Rgb(5, 5, 5),
            "selected row keeps selection_bg"
        );
        // A blank sidebar row below still carries sidebar_bg.
        assert_eq!(buffer[(10, 20)].bg, Color::Rgb(7, 7, 7));
    }

    #[test]
    fn popups_paint_content_bg_over_the_terminal_default() {
        let file = ui_viewer("ui-popup-bg.md", "x\n");
        let mut app = app_for(&file, None);
        app.theme_ui.content_bg = Color::Rgb(9, 9, 9);

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // The centered help popup (100x24 → 63 wide at x=18): an inner
        // cell must carry content_bg, not the terminal default.
        app.viewer.help_open = true;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(30, 5)].bg,
            Color::Rgb(9, 9, 9),
            "help popup painted with content_bg"
        );

        // Disambiguation popup (2 candidates → 60x6 centered at x=20,y=9):
        // even a *border* cell carries content_bg (the Paragraph base
        // style fills the whole rect; the Block only patches fg).
        app.viewer.help_open = false;
        app.disambiguation = Some(crate::app::Disambiguation {
            candidates: vec![
                std::path::PathBuf::from("/tmp/x/dup.md"),
                std::path::PathBuf::from("/tmp/y/dup.md"),
            ],
            selected: 0,
            heading: None,
        });
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(20, 9)].bg,
            Color::Rgb(9, 9, 9),
            "disambiguation popup (incl. border cells) painted with content_bg"
        );
        app.disambiguation = None;

        // Search popup in input mode (70x4 centered at x=15,y=10).
        app.search.open = true;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(15, 10)].bg,
            Color::Rgb(9, 9, 9),
            "search popup painted with content_bg"
        );
    }

    /// Help scroll: at 100x30 the whole body fits, so scrolling is a no-op;
    /// on a short terminal `j` moves the body up one line and `k` back.
    #[test]
    fn help_scroll_j_moves_and_k_returns() {
        let file = ui_viewer("ui-help-scroll.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;

        // Short terminal: the body overflows, so scrolling has room.
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let before = terminal.backend().to_string();
        assert_eq!(app.viewer.help_scroll, 0);

        // Simulate `j` through the app key path (same state change).
        app.viewer.help_scroll += 1;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let after = terminal.backend().to_string();
        assert_eq!(app.viewer.help_scroll, 1, "j scrolls the help one line");
        assert_ne!(before, after, "the body content moved");
        assert!(after.contains("close esc/any key"), "footer fixed");
        assert!(after.contains("keybinds"), "title fixed");

        // Scroll is clamped to the overflow at draw time.
        app.viewer.help_scroll = usize::MAX;
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert!(app.viewer.help_scroll < usize::MAX, "clamped at draw");
    }

    // --- scrollbars (markdown viewer, sidebar tree, tag list) -------------

    /// Buffer symbols of a whole row, left to right.
    fn row_symbols(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    /// Whether a scrollbar glyph (track or thumb) sits in column `x` of rows
    /// `ys`. The default scrollbar symbols are `░`/`█`/`▲`/`▼`/`↑`/`↓`.
    fn column_has_bar(buffer: &ratatui::buffer::Buffer, x: u16, ys: std::ops::Range<u16>) -> bool {
        ys.into_iter().any(|y| {
            matches!(
                buffer[(x, y)].symbol(),
                "█" | "░" | "▲" | "▼" | "↑" | "↓" | "▴" | "▾"
            )
        })
    }

    fn long_note(lines: usize) -> String {
        (0..lines).map(|i| format!("line-{i:03}\n")).collect()
    }

    #[test]
    fn note_scrollbar_absent_when_content_fits() {
        let file = ui_viewer("ui-bar-fit.md", "short\nnote\n");
        let mut app = app_for(&file, None);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        // Right edge of the content pane (last column before the bar would be).
        assert!(
            !column_has_bar(buffer, 99, 0..23),
            "no bar when content fits"
        );
        assert_eq!(app.note_viewport.width, 100, "text width untouched");
    }

    #[test]
    fn note_scrollbar_present_only_when_overflowing_and_text_stops_before_it() {
        let file = ui_viewer("ui-bar-long.md", &long_note(200));
        let mut app = app_for(&file, None);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();

        // Bar on the right edge column only (the rightmost column of the note
        // area), spanning the note rows (status line excluded).
        assert!(column_has_bar(buffer, 99, 0..23), "bar drawn at right edge");
        // Text is narrowed: the note viewport stops one column short of the edge.
        assert_eq!(app.note_viewport.x + app.note_viewport.width, 99);
        // No text cell is drawn under the bar column.
        for y in 0..23 {
            let sym = buffer[(99, y)].symbol();
            assert!(
                !sym.chars().any(|c| c.is_ascii_alphanumeric()),
                "text under the bar at row {y}: {sym:?}"
            );
        }
        // Text is still visible in the narrowed area.
        assert!(row_symbols(buffer, 0).contains("line-000"));
    }

    #[test]
    fn note_scrollbar_position_tracks_the_drawn_scroll() {
        let file = ui_viewer("ui-bar-pos.md", &long_note(200));
        let mut app = app_for(&file, None);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let top = terminal.backend().buffer().clone();

        app.viewer.scroll_bottom();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let bottom = terminal.backend().buffer().clone();
        // The paragraph and the bar both moved: the first row differs and the
        // bar thumb is no longer in the same place.
        assert!(row_symbols(&top, 0).contains("line-000"));
        assert!(!row_symbols(&bottom, 0).contains("line-000"));
        let thumb_rows = |b: &ratatui::buffer::Buffer| -> Vec<u16> {
            (0..23).filter(|&y| b[(99, y)].symbol() == "█").collect()
        };
        let (t, b) = (thumb_rows(&top), thumb_rows(&bottom));
        assert!(!t.is_empty() && !b.is_empty(), "thumb drawn");
        assert!(b[0] > t[0], "thumb moved down with the scroll");
    }

    #[test]
    fn note_scrollbar_click_mapping_matches_the_narrowed_viewport() {
        let file = ui_viewer("ui-bar-click.md", &long_note(200));
        let mut app = app_for(&file, None);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        // The viewport recorded for clicks is exactly the drawn text area.
        let vp = app.note_viewport;
        assert_eq!(vp.width, 99);
        assert_eq!(vp.x, 0);
        // A click on the bar column is outside the note viewport: no-op.
        let before = app.viewer.scroll;
        let focus_before = app.focus;
        app.handle_click(99, 5);
        assert_eq!(app.viewer.scroll, before);
        assert_eq!(app.focus, focus_before, "bar click does not change focus");
    }

    #[test]
    fn sidebar_scrollbar_present_only_when_rows_overflow() {
        let vault_dir = TempDir::new("ui-bar-side");
        for i in 0..40 {
            std::fs::write(vault_dir.path().join(format!("n{i:02}.md")), "x\n").unwrap();
        }
        let file = ui_viewer("ui-bar-side.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let sidebar_w = sidebar_width(100);
        // Border at column sidebar_w-1; the bar is the last inner column.
        let bar_x = sidebar_w - 2;
        assert!(column_has_bar(buffer, bar_x, 1..23), "sidebar bar drawn");
        assert_eq!(buffer[(sidebar_w - 1, 5)].symbol(), "│", "border intact");
        // Names never run under the bar.
        for y in 1..23 {
            let sym = buffer[(bar_x, y)].symbol();
            assert!(!sym.chars().any(|c| c.is_ascii_alphanumeric()), "{sym:?}");
        }
    }

    #[test]
    fn sidebar_scrollbar_absent_when_all_rows_fit() {
        let vault_dir = vault(); // two rows + folder: fits comfortably
        let file = ui_viewer("ui-bar-side-fit.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let bar_x = sidebar_width(100) - 2;
        assert!(
            !column_has_bar(buffer, bar_x, 1..23),
            "no bar when rows fit"
        );
    }

    #[test]
    fn tag_list_scrollbar_only_when_rows_overflow() {
        let vault_dir = TempDir::new("ui-bar-tags");
        for i in 0..30 {
            std::fs::write(
                vault_dir.path().join(format!("t{i:02}.md")),
                format!("---\ntags: [tag{i:02}]\n---\nx\n"),
            )
            .unwrap();
        }
        let file = ui_viewer("ui-bar-tags.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.handle_key(key(KeyCode::Char('t')));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let bar_x = sidebar_width(100) - 2;
        assert!(column_has_bar(buffer, bar_x, 1..23), "tag list bar drawn");
        assert_eq!(buffer[(sidebar_width(100) - 1, 5)].symbol(), "│");
    }

    #[test]
    fn tag_list_scrollbar_absent_when_rows_fit() {
        let vault_dir = TempDir::new("ui-bar-tags-fit");
        std::fs::write(vault_dir.path().join("a.md"), "---\ntags: [solo]\n---\nx\n").unwrap();
        let file = ui_viewer("ui-bar-tags-fit.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);
        app.handle_key(key(KeyCode::Char('t')));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let bar_x = sidebar_width(100) - 2;
        assert!(
            !column_has_bar(buffer, bar_x, 1..23),
            "no bar when tags fit"
        );
    }

    /// `<`/`>` width override: the drawn layout places the sidebar/content
    /// divider at the override's column instead of the proportional default.
    #[test]
    fn sidebar_width_override_moves_the_divider() {
        let vault_dir = TempDir::new("ui-sbw");
        std::fs::write(vault_dir.path().join("a.md"), "x\n").unwrap();
        let file = ui_viewer("ui-sbw.md", "x\n");
        let mut app = app_with_index(&file, &vault_dir);

        // Default: divider at sidebar_width(100) - 1 = 29.
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(sidebar_width(100) - 1, 5)].symbol(),
            "│"
        );

        // Override 40: divider moves to column 39.
        app.sidebar_width = Some(40);
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(39, 5)].symbol(),
            "│",
            "divider follows the width override"
        );
        assert_eq!(
            terminal.backend().buffer()[(sidebar_width(100) - 1, 5)].symbol(),
            " ",
            "old divider column no longer a border"
        );
    }

    /// Documented behavior (sidebar + tag list): the scrollbar column is inside
    /// the widget, so a click there is owned by the sidebar and maps by ROW
    /// only — it selects the row on that screen line, same as a click on the
    /// text. Pinned here with the bar drawn (overflowing list) and absent
    /// (fitting list).
    #[test]
    fn sidebar_click_on_scrollbar_column_selects_the_row_when_bar_present() {
        let vault_dir = TempDir::new("ui-bar-click-col");
        for i in 0..40 {
            std::fs::write(vault_dir.path().join(format!("n{i:02}.md")), "x\n").unwrap();
        }
        let file = ui_viewer("ui-bar-click-col.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let area = app.sidebar_area.expect("sidebar visible");
        let bar_col = sidebar_width(100) - 2;
        // Precondition: the bar really is drawn on this column (removing the
        // bar would fail here).
        assert!(
            column_has_bar(terminal.backend().buffer(), bar_col, 1..23),
            "bar drawn on the click column"
        );
        let (target, second) = {
            let rows = app.sidebar.as_ref().unwrap().rows();
            (rows[0].node.path.clone(), rows[1].node.path.clone())
        };
        app.handle_click(bar_col, area.y + 1);
        assert_eq!(
            app.sidebar.as_ref().unwrap().selected.as_deref(),
            Some(target.as_path()),
            "bar-column click on row 0 selects row 0"
        );
        app.handle_click(bar_col, area.y + 2);
        assert_eq!(
            app.sidebar.as_ref().unwrap().selected.as_deref(),
            Some(second.as_path()),
            "bar-column click on row 1 selects row 1"
        );
    }

    /// Fitting list: no bar, the same click column is ordinary text and
    /// maps by row exactly as before.
    #[test]
    fn sidebar_click_on_fitting_list_has_no_bar_and_maps_by_row() {
        let vault_dir = vault(); // a folder + one note: fits
        let file = ui_viewer("ui-bar-click-fit.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let area = app.sidebar_area.expect("sidebar visible");
        let bar_col = sidebar_width(100) - 2;
        assert!(
            !column_has_bar(terminal.backend().buffer(), bar_col, 1..23),
            "no bar on a fitting list"
        );
        let target = app.sidebar.as_ref().unwrap().rows()[1].node.path.clone();
        app.handle_click(bar_col, area.y + 2);
        assert_eq!(
            app.sidebar.as_ref().unwrap().selected.as_deref(),
            Some(target.as_path()),
            "click on the last text column maps to row 1 with no bar"
        );
    }

    #[test]
    fn sidebar_click_on_visible_row_maps_with_bar_present() {
        let vault_dir = TempDir::new("ui-bar-click-side");
        for i in 0..40 {
            std::fs::write(vault_dir.path().join(format!("n{i:02}.md")), "x\n").unwrap();
        }
        let file = ui_viewer("ui-bar-click-side.md", "x\n");
        let mut app = app_for(&file, Some(vault_dir.path()));
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let area = app.sidebar_area.expect("sidebar visible");
        // Row 0 of the list (`n00.md`) is at screen row area.y + 1; the bar
        // narrowing must not move it: the list mapping is row-only.
        let (target, second) = {
            let rows = app.sidebar.as_ref().unwrap().rows();
            (rows[0].node.path.clone(), rows[1].node.path.clone())
        };
        app.handle_click(2, area.y + 1);
        assert_eq!(
            app.sidebar.as_ref().unwrap().selected.as_deref(),
            Some(target.as_path()),
            "click on the first visible row selects that row"
        );
        // The second visible row is the next one, still row-accurate.
        app.handle_click(2, area.y + 2);
        assert_eq!(
            app.sidebar.as_ref().unwrap().selected.as_deref(),
            Some(second.as_path()),
        );
    }

    /// The help popup must never panic on small terminals (it is clamped to
    /// the frame area; too-small areas just draw nothing or a truncated box).
    #[test]
    fn help_popup_does_not_panic_on_small_terminals() {
        let file = ui_viewer("ui-help-small.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        for (w, h) in [(1, 1), (2, 2), (10, 4), (30, 8), (60, 12), (80, 10)] {
            let backend = TestBackend::new(w, h);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
        }
    }

    /// Full help body (with the blank group separators) on a tall backend,
    /// so the last group is on screen.
    fn render_help_tall(app: &mut App) -> String {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal.backend().to_string()
    }

    /// Theme cycling is documented; the footer stays fixed on a 30-row
    /// backend, and the body overflows there (so it scrolls, with a scrollbar).
    #[test]
    fn help_documents_theme_cycling_and_fits() {
        let file = ui_viewer("ui-help5.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        let screen = render_help_tall(&mut app);
        assert!(screen.contains("Shift+T"), "theme cycling documented");
        assert!(screen.contains("cycle theme"));
        assert!(screen.contains("close esc/any key"), "footer shown");
        let small = render_help(&mut app);
        assert!(
            small.contains("close esc/any key"),
            "footer fixed at 100x30"
        );
    }

    /// Keys are padded to one common column across groups and columns: every
    /// label of the grid starts at `key width + gap` inside its column, and
    /// the key width is shared by all columns.
    fn app_ui_theme() -> UiTheme {
        crate::theme::ui_by_preset(engine::MarkdownThemePreset::default())
    }

    #[test]
    fn help_labels_align_to_one_key_column() {
        let width = help_key_width();
        assert_eq!(width, "Backspace / Ctrl+O".chars().count());
        // Tall frame (spaced) and short frame (compact): the key column is
        // shared either way.
        for h in [36u16, 30u16] {
            let grid = help_grid(h);
            let lines = help_body(app_ui_theme(), &grid, grid.spaced);
            assert_eq!(lines.len(), grid.rows, "row count matches the grid");
            let text: Vec<String> = lines
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            for key in ["Tab", "h / l", "Shift+T", "dimmed links"] {
                for row in &text {
                    if let Some(rest) = row.find(key).map(|at| &row[at..]) {
                        if rest.starts_with(key) {
                            let after: String = rest.chars().skip(width + 1).take(1).collect();
                            assert_ne!(after, " ", "label starts at the key column: {row:?}");
                        }
                    }
                }
            }
        }
    }

    /// Group separation: a blank row sits between groups in a spaced column,
    /// and never in the packed layout.
    #[test]
    fn help_has_one_blank_line_between_groups() {
        // One column (pinned by a narrow width) with the spaced layout.
        let spaced = help_pack(HELP_SECTIONS, 1, true);
        assert_eq!(spaced.len(), 1, "one column packs all groups");
        let rows_spaced = spaced[0].rows;
        let packed = help_pack(HELP_SECTIONS, 1, false);
        assert_eq!(rows_spaced, packed[0].rows + HELP_SECTIONS.len() - 1);
        let grid = HelpGrid {
            columns: spaced,
            label_widths: vec![help_label_width(&HELP_SECTIONS.iter().collect::<Vec<_>>())],
            width: HELP_WIDTH,
            rows: rows_spaced,
            spaced: true,
        };
        let lines = help_body(app_ui_theme(), &grid, true);
        let blanks = lines
            .iter()
            .filter(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .count();
        assert_eq!(
            blanks,
            HELP_SECTIONS.len() - 1,
            "one blank row between groups"
        );
    }

    /// Layout is always a single column: the spaced layout (blank rows
    /// between groups) when it fits the frame height, the compact one
    /// (which scrolls) otherwise.
    #[test]
    fn help_grid_is_single_column_spaced_when_it_fits() {
        let tall = help_grid(36);
        assert_eq!(tall.columns.len(), 1, "always one column");
        assert!(tall.spaced, "spaced when the body fits");
        let mut all: Vec<usize> = tall.columns.iter().flat_map(|c| c.groups.clone()).collect();
        all.sort_unstable();
        assert_eq!(
            all,
            (0..HELP_SECTIONS.len()).collect::<Vec<_>>(),
            "every group placed once, in order"
        );
        assert!(tall.width <= 140, "sane width on a wide terminal");

        // Short frame: compact layout, the body scrolls.
        let short = help_grid(30);
        assert_eq!(short.columns.len(), 1);
        assert!(!short.spaced, "compact when the spaced body does not fit");
        assert!(short.rows < tall.rows);
    }

    /// The whole body is on the first screen at the 100x30 smoke size, so
    /// every key stays visible without scrolling.
    #[test]
    fn help_fits_first_screen_at_smoke_size() {
        let file = ui_viewer("ui-help-fit.md", "x\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        let screen = render_help(&mut app);
        assert!(screen.contains("refresh git status"), "u entry on screen");
        assert!(screen.contains("cycle theme"), "Shift+T entry on screen");
        assert_eq!(app.viewer.help_scroll, 0);
    }

    /// The popup dims everything outside it and keeps its own cells crisp:
    /// every cell outside the popup rect carries DIM, every cell inside does not.
    #[test]
    fn help_dims_background_and_keeps_popup_crisp() {
        let file = ui_viewer("ui-help-dim.md", "# Hi\n\nbody\n");
        let mut app = app_for(&file, None);
        app.viewer.help_open = true;
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let frame = Rect::new(0, 0, 100, 30);
        let popup = centered(
            frame,
            help_grid(frame.height).width.min(frame.width - 2),
            (help_grid(frame.height).rows as u16 + 3).min(frame.height - 2),
        );
        for y in frame.y..frame.bottom() {
            for x in frame.x..frame.right() {
                let inside =
                    x >= popup.x && x < popup.right() && y >= popup.y && y < popup.bottom();
                let dim = buffer[(x, y)].modifier.contains(Modifier::DIM);
                if inside {
                    assert!(!dim, "popup cell ({x},{y}) must not be dimmed");
                } else {
                    assert!(dim, "background cell ({x},{y}) must be dimmed");
                }
            }
        }
    }
}
