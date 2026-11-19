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

/// Right-edge vertical scrollbar shared by the note, list and help paths.
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
    // ratatui wants the scroll *range* here, not the line count: its
    // denominator is `content_length - 1 + viewport`, so feeding it the
    // line count shrinks the thumb and stops it short of the track end.
    // Leaving `viewport_content_length` unset makes ratatui use the track
    // height, which is what the classic formula assumes.
    let range = total.saturating_sub(viewport);
    let mut state = ScrollbarState::new(range).position(position);
    // Leaf's symbols: a thin track and a solid thumb, no arrow heads, so the
    // track spans the whole bar column (the heads used to eat two rows).
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::new().fg(fg).bg(bg))
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█"),
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
    app.layout.sidebar_area = sidebar_area;

    if app.base.is_some() {
        // A `.base` file is open: the content area is its table (the
        // viewer is hidden; its parse width is untouched until an actual
        // note is visible again).
        let note_area = Rect {
            height: view_height,
            ..content_area
        };
        app.layout.note_viewport = note_area;
        app.layout.backlinks_area = None;
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
        // The text sits inside a symmetric margin (config `margin`, blank
        // columns on each side). The scrollbar, when shown, takes the last
        // column, so the right side is the margin plus that column.
        let margin = app.note_margin;
        let text_full = Rect {
            x: note_full.x + margin,
            width: note_full.width.saturating_sub(2 * margin),
            ..note_full
        };
        let full_width = text_full.width as usize;
        let bar_shown = app.viewer.line_count_at(full_width) > note_rows && note_full.width > 3;
        let wrap_width = if bar_shown {
            full_width.saturating_sub(1).max(1)
        } else {
            full_width
        };
        app.viewer.on_content_width(wrap_width);
        let note_area = if bar_shown {
            Rect {
                width: text_full.width.saturating_sub(1),
                ..text_full
            }
        } else {
            text_full
        };
        app.layout.note_viewport = note_area;
        app.viewer.on_viewport_height(note_full.height + 1);

        // The text widget paints only its own cells, so the margin columns
        // would keep the terminal background: paint the whole note area first.
        f.buffer_mut()
            .set_style(note_full, Style::new().bg(ui.content_bg));
        // Lines are pre-wrapped (one row each), so only the visible window is
        // cloned and the paragraph needs no scroll offset of its own.
        let first = (app.viewer.scroll as usize).min(app.viewer.lines.len());
        let last = first
            .saturating_add(note_area.height as usize)
            .min(app.viewer.lines.len());
        let text = ratatui::text::Text::from(app.viewer.lines[first..last].to_vec());
        Paragraph::new(text)
            .style(Style::new().bg(ui.content_bg))
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
        app.layout.backlinks_area = None;
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
    if app.quick.open {
        // Record the popup geometry for the mouse handlers before the
        // index borrow (which would block the `&mut` access).
        let popup = crate::quick::popup_geometry(area, app.quick.items.len());
        app.layout.quick_area = Some(popup);
        draw_quick(f, area, app, ui);
    } else {
        app.layout.quick_area = None;
    }
    if app.search.open {
        // Record the popup geometry for the mouse handlers before the
        // index borrow (which would block the `&mut` access).
        let popup = crate::search::popup_geometry(area, app.search.mode, app.search.hits.len());
        app.layout.search_area = Some(popup);
        let index = app.index.as_ref().map(|i| i.borrow());
        draw_search(f, area, app, ui, index.as_deref());
    } else {
        app.layout.search_area = None;
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
    // The bar takes the last inner column while the table overflows. Decide
    // before the render: the extra column only ever narrows the table, and
    // the height (and so the viewport) stays the same either way.
    let overflow = base.line_count() > inner.height as usize;
    let bar_shown = overflow && inner.width > 1;
    let text_width = if bar_shown {
        inner.width - 1
    } else {
        inner.width
    };
    let text_area = Rect {
        width: text_width,
        ..inner
    };
    let lines = base.render(ui, text_width as usize, inner.height as usize);
    Paragraph::new(ratatui::text::Text::from(lines))
        .style(Style::new().bg(ui.content_bg))
        .render(text_area, f.buffer_mut());
    if bar_shown {
        let bar = Rect {
            x: inner.right() - 1,
            y: inner.y,
            width: 1,
            height: inner.height,
        };
        draw_scrollbar(
            f,
            bar,
            base.line_count(),
            base.scroll,
            inner.height as usize,
            ui.popup_dimmed_fg,
            ui.content_bg,
        );
    }
}

/// Visual treatment for a status glyph. Built only from colors the app
/// already uses elsewhere (status bar / borders) — no separate palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusStyle {
    /// Plain default text.
    Normal,
    /// Demoted/inactive state: the theme's dimmed color (name dimmed too).
    Dimmed,
    /// Live/current state: `Color::Green` + bold (status-bar clean `✓`).
    Green,
    /// Needs attention: `Color::Red` (status-bar behind `↓n`).
    Red,
}

impl StatusStyle {
    /// The style applied to the glyph span itself.
    pub fn glyph_style(self, ui: UiTheme) -> Style {
        match self {
            StatusStyle::Normal => Style::new(),
            StatusStyle::Dimmed => Style::new().fg(ui.dimmed_fg),
            StatusStyle::Green => Style::new()
                .fg(ui.status_success_fg)
                .add_modifier(Modifier::BOLD),
            StatusStyle::Red => Style::new().fg(ui.status_error_fg),
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

/// Paints the selected-row highlight over every span of a list row.
fn highlight_selection(spans: &mut [Span<'_>], ui: UiTheme) {
    for span in spans {
        span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
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

    let rows = sidebar.rows();
    let rows_total = rows.len();
    let overflow = rows_total > inner_height;
    // Text area: the bordered inner area, minus the scrollbar column when
    // the rows overflow (the bar sits on the inner right edge, so the
    // border itself stays intact).
    let text_area = list_text_area(area, overflow);
    let inner_width = text_area.width as usize;

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

    render_list_panel(
        f,
        ListPanel {
            area,
            text_area,
            title: " vault ".into(),
            focused,
            scroll: sidebar.scroll,
            rows_total,
            overflow,
        },
        window,
        ui,
    );
}

/// Geometry and chrome of one bordered list panel (sidebar tree, tag
/// list, backlinks) for [`render_list_panel`].
struct ListPanel {
    area: Rect,
    text_area: Rect,
    title: String,
    focused: bool,
    scroll: usize,
    rows_total: usize,
    overflow: bool,
}

/// The shared tail of the three list panels: bordered block on
/// `sidebar_bg`, the visible row window, and the scrollbar when the rows
/// overflow. Keeps the panels' click geometry (`list_row_at`) identical.
fn render_list_panel(f: &mut Frame, panel: ListPanel, window: Vec<Line<'static>>, ui: UiTheme) {
    let border = if panel.focused {
        ui.sidebar_border_focused
    } else {
        ui.sidebar_border_unfocused
    };
    f.render_widget(
        Block::new()
            .style(Style::new().bg(ui.sidebar_bg))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(panel.title),
        panel.area,
    );
    f.render_widget(
        Paragraph::new(window).style(Style::new().bg(ui.sidebar_bg)),
        panel.text_area,
    );
    if panel.overflow {
        draw_scrollbar(
            f,
            list_scrollbar_area(panel.area),
            panel.rows_total,
            panel.scroll,
            panel.area.height.saturating_sub(2) as usize,
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
        // Notes and `.base` tables carry no arrow.
        NodeKind::Note | NodeKind::Base => ("", 0),
    };
    let (glyph_text, glyph_width, dimmed, glyph_style) = match glyph {
        Some((glyph, style)) => (
            format!("{glyph} "),
            2,
            style == StatusStyle::Dimmed,
            style.glyph_style(ui),
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
        highlight_selection(&mut spans, ui);
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
            let selected = crate::tags::TagList::row_matches(row, &tags.selected);
            tag_line(row, inner_width, selected, index, ui)
        })
        .collect();

    render_list_panel(
        f,
        ListPanel {
            area,
            text_area,
            title: " tags ".into(),
            focused,
            scroll: tags.scroll,
            rows_total,
            overflow,
        },
        window,
        ui,
    );
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
        highlight_selection(&mut line.spans, ui);
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
    app.layout.backlinks_area = Some(Rect {
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
                    highlight_selection(&mut line.spans, ui);
                }
                line
            })
            .collect()
    };

    // The backlinks panel is a sidebar-family secondary panel, so it
    // paints `sidebar_bg` (not `content_bg`).
    render_list_panel(
        f,
        ListPanel {
            area,
            text_area,
            title: format!(" backlinks ({}) ", app.backlinks.len()),
            focused,
            scroll: app.backlink_scroll,
            rows_total,
            overflow: bar_shown,
        },
        window,
        ui,
    );
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
                highlight_selection(&mut line.spans, ui);
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

/// Quick note switcher (`Ctrl+P`): the query prompt on top, the fuzzy
/// matches below (the matched characters highlighted), a footer. A
/// stateless scroll window keeps the selection visible when there are more
/// matches than rows. The popup rectangle was recorded into
/// `app.layout.quick_area` by `draw` (the mouse handlers map clicks/wheels
/// through it, using the same row math as below).
fn draw_quick(f: &mut Frame, area: Rect, app: &App, ui: UiTheme) {
    let popup = app
        .layout
        .quick_area
        .unwrap_or_else(|| crate::quick::popup_geometry(area, app.quick.items.len()));
    let inner_width = popup.width.saturating_sub(2) as usize;
    // borders (2) + prompt (1) + footer (1).
    let list_rows = popup.height.saturating_sub(4).max(1) as usize;

    let mut lines: Vec<Line<'static>> =
        vec![Line::from(Span::raw(format!(">{}▏", app.quick.query)))];
    if app.quick.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches",
            Style::new().fg(ui.popup_dimmed_fg),
        )));
    } else {
        // Stateless scroll window: the selection always visible.
        let start = app
            .quick
            .selected
            .saturating_sub(list_rows.saturating_sub(1));
        lines.extend(
            app.quick
                .items
                .iter()
                .enumerate()
                .skip(start)
                .take(list_rows)
                .map(|(i, item)| {
                    let selected = i == app.quick.selected;
                    quick_item_line(item, inner_width, selected, ui)
                }),
        );
    }
    lines.push(Line::from(Span::styled(
        "↑/↓ move · Enter open · Esc close",
        Style::new().fg(ui.popup_dimmed_fg),
    )));

    let title = format!(" jump: {} ", app.quick.items.len());
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

/// One quick-switcher row: the note's vault-relative path with the
/// matched characters highlighted.
fn quick_item_line(
    item: &crate::quick::QuickItem,
    inner_width: usize,
    selected: bool,
    ui: UiTheme,
) -> Line<'static> {
    let match_style = Style::new().fg(ui.popup_match_fg).bg(ui.popup_match_bg);
    let chars: Vec<char> = item.label.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run_match = false;
    let mut run = String::new();
    for (i, ch) in chars.iter().enumerate() {
        let is_match = item.positions.contains(&i);
        if is_match != run_match {
            if !run.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    if run_match { match_style } else { Style::new() },
                ));
            }
            run_match = is_match;
        }
        run.push(*ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(
            run,
            if run_match { match_style } else { Style::new() },
        ));
    }
    // Truncate to the popup width (keeping whole characters).
    let mut used = 0usize;
    let mut truncated: Vec<Span<'static>> = Vec::new();
    for span in spans {
        let w: usize = UnicodeWidthStr::width(span.content.as_ref());
        if used + w > inner_width {
            break;
        }
        used += w;
        truncated.push(span);
    }
    let mut line = Line::from(truncated);
    if selected {
        // The selection tint replaces the background of the *unmatched*
        // spans only: the matched characters keep the match highlight (the
        // same rule the search popup uses).
        for span in &mut line.spans {
            if span.style.bg != Some(ui.popup_match_bg) {
                span.style = span.style.bg(ui.selection_bg).add_modifier(Modifier::BOLD);
            }
        }
    }
    line
}

/// Global search popup (`s`): input prompt or results list, depending on
/// the search state's mode. Results rows are `relpath:line snippet` with
/// the match highlighted; a stateless scroll window keeps the selection
/// visible when there are more hits than rows. The popup rectangle was
/// recorded into `app.layout.search_area` by `draw` (the mouse handlers map
/// clicks/wheels through it, using the same row math as below).
fn draw_search(f: &mut Frame, area: Rect, app: &App, ui: UiTheme, index: Option<&VaultIndex>) {
    let popup = app.layout.search_area.unwrap_or_else(|| {
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
            highlight_selection(&mut line.spans, ui);
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

/// The status bar's name label. Content focused → the open note's name;
/// sidebar focused → the focused row's name (`#tag` for a tag). Notes and
/// bases carry an icon; folders and tags show their bare name.
fn status_path_label(app: &App, sidebar_focused: bool) -> String {
    if !sidebar_focused {
        return note_status_label(app);
    }
    match app.sidebar_mode {
        SidebarMode::Tree => app
            .sidebar
            .as_ref()
            .and_then(|sidebar| sidebar.selected_path())
            .map(|path| file_label(app, path))
            .unwrap_or_else(|| "(empty vault)".to_string()),
        SidebarMode::Tags => {
            if let Some(path) = app.tags.selected_path() {
                file_label(app, path)
            } else if let Some(crate::tags::Selection::Tag(tag)) = &app.tags.selected {
                format!("#{tag}")
            } else {
                "(no tags)".to_string()
            }
        }
    }
}

/// The open note's label on the status bar: its icon and file name.
fn note_status_label(app: &App) -> String {
    file_label(app, &app.current_path())
}

/// `path`'s file name, prefixed by its icon when it is a note (`.md`) or a
/// base (`.base`). Any other path (a folder) keeps the bare name.
fn file_label(app: &App, path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("md") => format!("{} {name}", app.icons.note()),
        Some("base") => format!("{} {name}", app.icons.base()),
        _ => name,
    }
}

/// The content pane's scroll offset and scroll range. `None` when no
/// document is open: the viewer always holds a path in a running app, so an
/// empty path is the sentinel (only tests reach it).
fn content_scroll(app: &App) -> Option<(u32, u32)> {
    if let Some(base) = &app.base {
        return Some((base.scroll as u32, base.max_scroll() as u32));
    }
    if app.viewer.path.as_os_str().is_empty() {
        return None;
    }
    Some((app.viewer.scroll, app.viewer.max_scroll()))
}

/// The scroll offset as a percentage of the scroll range: `0` at the top,
/// `100` at the bottom, and `100` when the content fits (range `0`).
fn scroll_percent(scroll: u32, max_scroll: u32) -> u16 {
    if max_scroll == 0 {
        return 100;
    }
    (u64::from(scroll.min(max_scroll)) * 100 / u64::from(max_scroll)).min(100) as u16
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
    app.layout.status_name_area = if sidebar_focused {
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
    // Scroll position: the content pane's offset, whichever pane has the
    // focus. An empty viewer path is the no-document sentinel (the app
    // refuses to start without a file), and then the segment is omitted.
    let scroll = content_scroll(app);
    let mut spans = vec![
        Span::styled(path, path_style),
        Span::raw(" · "),
        Span::raw(count),
    ];
    if let Some((offset, max_scroll)) = scroll {
        spans.push(Span::raw(" · "));
        // Fixed four columns, right-aligned: the segments after it must not
        // move while the percentage grows (SPEC, "Status bar").
        spans.push(Span::raw(format!(
            "{:>3}%",
            scroll_percent(offset, max_scroll)
        )));
    }
    spans.push(Span::raw(" · "));
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
        Some(t) => format!("{} {}", app.icons.clock(), format_clock(t)),
        None => app.icons.clock().to_string(),
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

/// `HH:MM` in the local timezone. Seconds are dropped on purpose: the
/// status bar would otherwise churn a column every tick.
fn format_clock(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%H:%M")
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
            ("s / Ctrl+P", "global search · quick note switcher"),
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
            ("Ctrl+T", "theme group: all → light → dark"),
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

/// Widest label text across every help group.
fn help_label_width() -> usize {
    HELP_SECTIONS
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .map(|(_, label)| label.chars().count())
        .max()
        .unwrap_or(0)
}

/// Body rows of the single help column: per group a header plus its
/// entries, with one blank row between groups when `spaced`.
fn help_rows(spaced: bool) -> usize {
    let rows: usize = HELP_SECTIONS
        .iter()
        .map(|(_, entries)| 1 + entries.len())
        .sum();
    rows + usize::from(spaced) * HELP_SECTIONS.len().saturating_sub(1)
}

/// The layout chosen for the popup.
struct HelpGrid {
    /// Popup width (borders and scrollbar column included).
    width: u16,
    /// Body rows of the column.
    rows: usize,
    /// Whether the column carries a blank row between its groups.
    spaced: bool,
}

/// Lay the help out for a frame height `area_height`. The help is always a
/// single column (all groups in order): the spaced layout (blank row
/// between groups) when it still fits the height, the compact one otherwise
/// (the body then scrolls behind the right scrollbar).
fn help_grid(area_height: u16) -> HelpGrid {
    // Body rows the popup can show: frame minus the 2-row margin, minus the
    // 2 borders and the footer row.
    let body_budget = usize::from(area_height.saturating_sub(2)).saturating_sub(3);
    let spaced = help_rows(true) <= body_budget;
    HelpGrid {
        width: HELP_WIDTH,
        rows: help_rows(spaced),
        spaced,
    }
}

/// Body lines of the help column, each padded to the popup's inner width:
/// header and entries, keys padded to the shared key width, labels padded to
/// the widest label, blank rows between groups when `grid.spaced`.
fn help_body(ui: UiTheme, grid: &HelpGrid) -> Vec<Line<'static>> {
    let key_width = help_key_width();
    let label_width = help_label_width();
    let inner = grid.width.saturating_sub(HELP_CHROME) as usize;
    let mut cells: Vec<Vec<Span<'static>>> = Vec::new();
    for (position, (section, entries)) in HELP_SECTIONS.iter().enumerate() {
        if grid.spaced && position > 0 {
            cells.push(Vec::new());
        }
        cells.push(vec![Span::styled(
            section.to_string(),
            Style::new()
                .fg(ui.sidebar_border_focused)
                .add_modifier(Modifier::BOLD),
        )]);
        for (key, label) in entries.iter() {
            if key.is_empty() {
                cells.push(vec![Span::raw(format!(
                    "{:<key_width$} {:<label_width$}",
                    "", label
                ))]);
                continue;
            }
            cells.push(vec![
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
    cells
        .into_iter()
        .map(|mut spans| {
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            if inner > used {
                spans.push(Span::raw(" ".repeat(inner - used)));
            }
            Line::from(spans)
        })
        .collect()
}

/// Lines a PageUp/PageDown moves the help by (about a page of body rows).
pub const HELP_PAGE_STEP: usize = 10;

/// Body rows visible for a popup of `popup_height` rows: borders (2) and
/// the fixed footer row (1) are not scrollable.
fn help_visible_rows(popup_height: u16) -> usize {
    popup_height.saturating_sub(3) as usize
}

/// The help popup's geometry for a frame `area`.
struct HelpPopup {
    /// The popup's rectangle (borders and scrollbar column included).
    rect: Rect,
    /// Body rows of the chosen layout.
    total: usize,
    /// Body rows the popup shows at once.
    visible: usize,
}

/// The popup's geometry: the layout's width, its height fitted to the body
/// (+ borders and footer) and clamped to the terminal, centered in `area`.
/// The single source of the clamps for `draw_help` and its tests.
fn help_popup(area: Rect) -> HelpPopup {
    let grid = help_grid(area.height);
    let width = grid.width.min(area.width.saturating_sub(2)).max(1);
    let total = grid.rows;
    let want_height = (total as u16).saturating_add(3);
    let height = want_height.min(area.height.saturating_sub(2)).max(1);
    HelpPopup {
        rect: centered(area, width, height),
        total,
        visible: help_visible_rows(height),
    }
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
    let HelpPopup {
        rect: popup,
        total,
        visible,
    } = help_popup(area);
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
    let lines = help_body(ui, &grid);
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
        draw_scrollbar(
            f,
            bar_area,
            total,
            scroll,
            body_height as usize,
            ui.popup_dimmed_fg,
            ui.content_bg,
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
mod tests;
