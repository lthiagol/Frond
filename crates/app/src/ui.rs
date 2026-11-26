//! Drawing: sidebar (tree / tags) + rendered-note viewport + backlinks
//! panel + status bar + help/disambiguation popups.

use std::path::Path;
use std::time::{Instant, SystemTime};

use ratatui::layout::{Position, Rect};
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vault::GitStatus;

/// Sidebar width: ~30% of the terminal, with sane bounds.
pub fn sidebar_width(total: u16) -> u16 {
    (u32::from(total) * 3 / 10).clamp(24, 60) as u16
}

/// What a bar shows, as the thumb needs it: the content's extent (lines or
/// rows), the current offset, and how many rows of it the pane shows.
/// `viewport` is explicit because the track is taller than the text viewport
/// whenever the pane has a vertical margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BarSpan {
    total: usize,
    position: usize,
    viewport: usize,
}

/// Right-edge vertical scrollbar shared by the note, list and help paths.
/// Drawn into a single-column `bar_area`; callers only invoke it when the
/// content overflows, and shrink their text area by that one column.
///
/// The track is painted in `popup_dimmed_fg`; while the pointer is over the
/// bar column the thumb takes the theme's accent (`scrollbar_hover_fg`) so
/// the bar reads as draggable. The pointer test lives here, in the one
/// helper every bar is drawn through, and it only ever changes a colour.
fn draw_scrollbar(
    f: &mut Frame,
    bar_area: Rect,
    span: BarSpan,
    bg: Color,
    ui: UiTheme,
    pointer: Option<Position>,
) {
    // ratatui wants the scroll *range* here, not the line count: its
    // denominator is `content_length - 1 + viewport`, so feeding it the
    // line count shrinks the thumb and stops it short of the track end.
    // The viewport is passed explicitly because the track is taller than
    // the text viewport whenever the pane has a vertical margin; the thumb
    // must stay proportional to the visible fraction of the content.
    let range = span.total.saturating_sub(span.viewport);
    let mut state = ScrollbarState::new(range)
        .position(span.position)
        .viewport_content_length(span.viewport);
    let thumb_fg = if pointer.is_some_and(|pointer| bar_area.contains(pointer)) {
        ui.scrollbar_hover_fg
    } else {
        ui.popup_dimmed_fg
    };
    // Leaf's symbols: a thin track and a solid thumb, no arrow heads, so the
    // track spans the whole bar column (the heads used to eat two rows).
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::new().fg(ui.popup_dimmed_fg).bg(bg))
            .thumb_style(Style::new().fg(thumb_fg).bg(bg))
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█"),
        bar_area,
        &mut state,
    );
}

/// The note pane's line-number gutter width: the digits of the note's last
/// source line. An empty note still needs one column (there is no line 0 to
/// name). `source` is the raw file, so the count includes the frontmatter,
/// exactly like the engine's 1-based `source_line_map`.
fn line_number_gutter_width(source: &str) -> u16 {
    source.lines().count().max(1).to_string().len() as u16
}

/// Paints the visible line numbers right-aligned in `area`. `source_line_map`
/// is 1-based; a rendered row repeats its predecessor's source line when it
/// is a wrapped continuation, and only the first row of a source line gets
/// the number. Rows outside `first..last` are not drawn.
fn draw_line_numbers(
    f: &mut Frame,
    area: Rect,
    source_line_map: &[usize],
    first: usize,
    last: usize,
    fg: Color,
    bg: Color,
) {
    let style = Style::new().fg(fg).bg(bg);
    for (row, index) in (first..last).enumerate() {
        let Some(&line) = source_line_map.get(index) else {
            break;
        };
        if index > 0 && source_line_map.get(index - 1) == Some(&line) {
            continue;
        }
        let number = line.to_string();
        // A pane narrower than the note's line count cannot show the whole
        // number. Keep the least significant digits inside the gutter: the
        // full string would start left of it, in the margin.
        let number = if number.len() > area.width as usize {
            &number[number.len() - area.width as usize..]
        } else {
            number.as_str()
        };
        let x = area.right().saturating_sub(number.len() as u16);
        f.buffer_mut()
            .set_stringn(x, area.y + row as u16, number, area.width as usize, style);
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // The chrome palette: every hardcoded shell color below comes from
    // the active theme (see `theme.rs`).
    let ui = app.theme_ui;
    // The pointer's cell decides the bars' hover highlight (read once: the
    // draw borrows parts of `app` mutably below).
    let pointer = app.pointer;
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
        app.layout.note_scrollbar_area = None;
        app.layout.base_scrollbar_area = None;
        app.layout.backlinks_area = None;
        app.layout.backlinks_scrollbar_area = None;
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

        // The engine pre-wraps at the render width, so it must match the real
        // text width (re-parses on change, e.g. sidebar toggled or resize).
        // Overflow is decided at the full text width first; if the note
        // overflows, the scrollbar takes the right column and the text
        // re-wraps one column narrower. Narrowing only ever adds lines, so
        // an overflowing note keeps overflowing and the bar stays consistent.
        // The text sits inside a symmetric margin (config `margin`, blank
        // columns on each side, and `margin_y`, blank rows above and below).
        // The scrollbar, when shown, takes the last column, so the right
        // side is the margin plus that column.
        let margin = app.note_margin;
        let margin_y = app.note_margin_y;
        let gutter_width = if app.line_numbers {
            line_number_gutter_width(&app.viewer.source)
        } else {
            0
        };
        let text_full = Rect {
            x: note_full.x + margin,
            y: note_full.y + margin_y,
            width: note_full.width.saturating_sub(2 * margin),
            height: note_full.height.saturating_sub(margin_y.saturating_mul(2)),
        };
        // The gutter takes its columns from the text area's left edge, so
        // the text is narrower by exactly its width and wraps earlier. The
        // margin stays outside it: it is blank padding, not part of the pane.
        let gutter_width = gutter_width.min(text_full.width);
        let text_width = text_full.width.saturating_sub(gutter_width);
        let text_rows = text_full.height as usize;
        let full_width = text_width as usize;
        // A pane shorter than twice the margin leaves no text rows. ratatui
        // reads a 0 viewport as "unset" and scales the thumb against the
        // track, claiming visible rows the pane does not have: draw no bar.
        let bar_shown = text_rows > 0
            && app.viewer.line_count_at(full_width) > text_rows
            && note_full.width > 3;
        let wrap_width = if bar_shown {
            full_width.saturating_sub(1).max(1)
        } else {
            full_width
        };
        app.viewer.on_content_width(wrap_width);
        let note_area = Rect {
            x: text_full.x + gutter_width,
            width: text_width.saturating_sub(u16::from(bar_shown)),
            ..text_full
        };
        app.layout.note_viewport = note_area;
        // The viewer subtracts the status line itself, so pass the text rows
        // plus that line: the visible rows decide the scroll range.
        app.viewer.on_viewport_height(text_full.height + 1);
        // Re-evaluate the hover from this frame's geometry: a pointer that
        // stayed still must still name what is under it after a scroll or a
        // popup opened/closed since the last pointer event.
        app.sync_link_hover();

        // The text widget paints only its own cells, so the margin cell rows
        // and columns would keep the terminal background: paint the whole note
        // area first.
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
        if app.line_numbers && gutter_width > 0 {
            let gutter = Rect {
                x: text_full.x,
                width: gutter_width,
                ..text_full
            };
            draw_line_numbers(
                f,
                gutter,
                &app.viewer.source_line_map,
                first,
                last,
                app.viewer.code_gutter(),
                ui.content_bg,
            );
        }
        if bar_shown {
            let bar = Rect {
                x: note_full.right() - 1,
                width: 1,
                ..note_full
            };
            app.layout.note_scrollbar_area = Some(bar);
            draw_scrollbar(
                f,
                bar,
                BarSpan {
                    total: app.viewer.lines.len(),
                    position: app.viewer.scroll as usize,
                    viewport: text_rows,
                },
                ui.content_bg,
                ui,
                pointer,
            );
        } else {
            app.layout.note_scrollbar_area = None;
        }

        // Geometry for click routing (focus on click), fresh every frame.
        app.layout.base_scrollbar_area = None;
        app.layout.backlinks_area = None;
        app.layout.backlinks_scrollbar_area = None;
        if panel_height > 0 {
            let panel = Rect {
                y: content_area.y + view_height - panel_height,
                height: panel_height,
                ..content_area
            };
            draw_backlinks(f, panel, app, ui, app.focus == Focus::Backlinks);
        }
    }

    let mut sidebar_bar = None;
    if let Some(sidebar_rect) = sidebar_area {
        // The header the tree and tag views carry: the vault directory's
        // name. It comes from the tree's root (the same path the index
        // holds); the index is the fallback when there is no tree.
        let vault_label = app
            .sidebar
            .as_ref()
            .map(|sidebar| vault_name(&sidebar.root))
            .or_else(|| {
                app.index
                    .as_ref()
                    .map(|index| vault_name(index.borrow().root()))
            });
        let index = app.index.as_ref().map(|i| i.borrow());
        let chrome = ListChrome {
            vault_name: vault_label.as_deref().unwrap_or(""),
            ui,
            focused: app.focus == Focus::Sidebar,
            pointer,
        };
        if app.sidebar_mode == SidebarMode::Tags {
            sidebar_bar = draw_tag_list(f, sidebar_rect, &mut app.tags, chrome, index.as_deref());
        } else if app.sidebar_mode == SidebarMode::Outline {
            let focused = app.focus == Focus::Sidebar;
            // The `.base`-table rule lives in `app::outline_rows`; split
            // the fields so the rows and the cursor can be borrowed at once.
            let App {
                base,
                viewer,
                outline,
                ..
            } = app;
            let entries = crate::app::outline_rows(base, viewer);
            // At the note's bottom the last heading is the active one: the
            // final lines cannot scroll up to meet it (leaf's end rule).
            let max_scroll = viewer.max_scroll();
            let at_end = max_scroll > 0 && viewer.scroll >= max_scroll;
            outline.follow_note(entries, viewer.scroll, at_end);
            sidebar_bar = draw_outline(f, sidebar_rect, outline, entries, ui, focused, pointer);
        } else if let Some(sidebar) = &mut app.sidebar {
            sidebar_bar = draw_sidebar(
                f,
                sidebar_rect,
                sidebar,
                chrome,
                RowFacts {
                    index: index.as_deref(),
                    git: app.git.as_ref(),
                },
            );
        }
    }
    // The panel's bar column is a selection position control: it follows
    // whichever list the sidebar shows (tree, tags or outline).
    app.layout.sidebar_scrollbar_area = sidebar_bar;

    let status = Rect {
        x: area.x,
        y: area.y + view_height,
        width: area.width,
        height: 1,
    };
    draw_status_bar(f, status, app, ui);

    if app.viewer.help_open {
        draw_help(f, area, app, ui);
    } else {
        app.layout.help_scrollbar_area = None;
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
    if let Some(picker) = &app.vault_picker {
        draw_vault_picker_popup(f, area, picker, ui, "Esc close");
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
    let pointer = app.pointer;
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
        // Geometry for the mouse handlers: the bar is a document-position
        // control, like the note pane's.
        app.layout.base_scrollbar_area = Some(bar);
        draw_scrollbar(
            f,
            bar,
            BarSpan {
                total: base.line_count(),
                position: base.scroll,
                viewport: inner.height as usize,
            },
            ui.content_bg,
            ui,
            pointer,
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

/// Paints a list row as the selection bar: `selection_bg` across the panel's
/// whole inner width, the text in the panel's own background colour
/// (inverse video — the rule `links.rs` uses for the focused span), and a
/// space pad carrying the same background out to `text_width`. A popup
/// match span keeps the match highlight; the row's own colours (the status
/// glyph, the changed marker, the dimmed name) keep their shape, not their
/// colour.
fn highlight_selection(line: &mut Line<'_>, text_width: usize, panel_bg: Color, ui: UiTheme) {
    for span in &mut line.spans {
        if span.style.bg == Some(ui.popup_match_bg) {
            continue;
        }
        span.style = span
            .style
            .fg(panel_bg)
            .bg(ui.selection_bg)
            .add_modifier(Modifier::BOLD);
    }
    let used: usize = line
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let pad = text_width.saturating_sub(used);
    if pad > 0 {
        line.spans.push(Span::styled(
            " ".repeat(pad),
            Style::new().fg(panel_bg).bg(ui.selection_bg),
        ));
    }
}

/// The two vault readings a sidebar row is decorated with: the note's
/// frontmatter status (its glyph) and the working tree's changed files (the
/// `*` marker). Each is optional — no index means no glyph, no git status
/// means no marker.
#[derive(Clone, Copy)]
struct RowFacts<'a> {
    index: Option<&'a VaultIndex>,
    git: Option<&'a GitStatus>,
}

/// The vault's display name: the directory name the tree and tag views show
/// in their header block. A root with no final component (the filesystem
/// root) falls back to its full display form, so the header always names
/// what it lists.
fn vault_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// The chrome the tree and tag views draw with: the vault name their header
/// shows, the palette, and the pointer for the bars' hover. Bundled so both
/// renderers take one chrome argument instead of four.
#[derive(Clone, Copy)]
struct ListChrome<'a> {
    /// The vault directory's name, centred in the header block.
    vault_name: &'a str,
    ui: UiTheme,
    focused: bool,
    pointer: Option<Position>,
}

fn draw_sidebar(
    f: &mut Frame,
    area: Rect,
    sidebar: &mut crate::sidebar::Sidebar,
    chrome: ListChrome<'_>,
    facts: RowFacts<'_>,
) -> Option<Rect> {
    let ui = chrome.ui;
    let list_area = list_body_area(area);
    let inner_height = list_area.height.saturating_sub(2) as usize;
    sidebar.on_viewport_height(inner_height.max(1));
    sidebar.scroll_selection_into_view();

    let rows = sidebar.rows();
    let rows_total = rows.len();
    let overflow = rows_total > inner_height;
    // Text area: the bordered inner area, minus the scrollbar column when
    // the rows overflow (the bar sits on the inner right edge, so the
    // border itself stays intact).
    let text_area = list_text_area(list_area, overflow);
    let inner_width = text_area.width as usize;

    let window: Vec<Line<'static>> = rows
        .iter()
        .skip(sidebar.scroll)
        .take(inner_height)
        .map(|row| {
            let expanded = sidebar.expanded.contains(&row.node.path);
            let selected = sidebar.selected.as_deref() == Some(row.node.path.as_path());
            let glyph = facts
                .index
                .and_then(|index| index.note(&row.node.path))
                .and_then(|note| status_glyph(note.status.as_deref()));
            let changed = facts
                .git
                .is_some_and(|git| git.changed_files.contains(&row.node.path));
            sidebar_line(row, expanded, inner_width, selected, glyph, changed, ui)
        })
        .collect();

    render_list_panel(
        f,
        ListPanel {
            area,
            list_area,
            text_area,
            title: " vault ".into(),
            header: Some(chrome.vault_name.to_string()),
            focused: chrome.focused,
            scroll: sidebar.scroll,
            rows_total,
            overflow,
        },
        window,
        ui,
        chrome.pointer,
    )
}

/// Geometry and chrome of one bordered list panel (sidebar tree, tag
/// list, backlinks) for [`render_list_panel`].
struct ListPanel {
    /// The whole bordered panel.
    area: Rect,
    /// The panel rect whose inner rows are the list. It differs from `area`
    /// only when a header sits above the rows (every sidebar mode's label):
    /// the bar's track and its row mapping start below it.
    list_area: Rect,
    text_area: Rect,
    title: String,
    /// The label the header block centres above the rows. The tree and tag
    /// views name the vault; the outline says `TABLE OF CONTENTS`.
    header: Option<String>,
    focused: bool,
    scroll: usize,
    rows_total: usize,
    overflow: bool,
}

/// The shared tail of the three list panels: bordered block on
/// `sidebar_bg`, the visible row window, and the scrollbar when the rows
/// overflow. Keeps the panels' click geometry (`list_row_at`) identical.
/// Returns the bar's column when one was drawn, for the position control.
fn render_list_panel(
    f: &mut Frame,
    panel: ListPanel,
    window: Vec<Line<'static>>,
    ui: UiTheme,
    pointer: Option<Position>,
) -> Option<Rect> {
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
    if let Some(label) = &panel.header {
        // Chrome inside the border: paint it after the rows so the rule's
        // `┤` tee overwrites the border's `│`. The rows cannot reach it:
        // they render into `text_area`, which starts below the header, and
        // ratatui clips a Paragraph to its rect.
        draw_list_header(f, panel.area, label, ui, panel.focused);
    }
    if panel.overflow {
        let bar = list_scrollbar_area(panel.list_area);
        draw_scrollbar(
            f,
            bar,
            BarSpan {
                total: panel.rows_total,
                position: panel.scroll,
                viewport: panel.list_area.height.saturating_sub(2) as usize,
            },
            ui.sidebar_bg,
            ui,
            pointer,
        );
        Some(bar)
    } else {
        None
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
    let mut line = Line::from(spans);
    if selected {
        highlight_selection(&mut line, inner_width, ui.sidebar_bg, ui);
    }
    line
}

/// The header block every sidebar list mode carries above its rows, top to
/// bottom: a blank row, the label, another blank row, and the rule under it
/// — leaf's `render/toc.rs` header chunk with a line of space on each side
/// of the label. The list (its viewport, its bar and its row clicks) starts
/// below these rows.
enum ListHeaderRow {
    Blank,
    Label,
    Rule,
}

const LIST_HEADER: [ListHeaderRow; 4] = [
    ListHeaderRow::Blank,
    ListHeaderRow::Label,
    ListHeaderRow::Blank,
    ListHeaderRow::Rule,
];

/// Rows the header takes above the list. Derived from the row sequence so
/// the renderer, the viewport and the click mapping cannot disagree.
pub(crate) const LIST_HEADER_ROWS: u16 = LIST_HEADER.len() as u16;

/// The panel rect whose inner rows are the list: the panel minus the
/// header. `list_row_at` adds the top border itself, so this skips the
/// header only; the bar drawn from it starts below the rule.
pub(crate) fn list_body_area(area: Rect) -> Rect {
    let header = LIST_HEADER_ROWS.min(area.height.saturating_sub(2));
    Rect {
        y: area.y.saturating_add(header),
        height: area.height.saturating_sub(header),
        ..area
    }
}

/// The outline panel (`o`): the open note's headings, indented relative to
/// its shallowest heading level, the depth-0 rows numbered and the deeper
/// ones bulleted. The empty state covers both no note and a note without
/// headings.
fn draw_outline(
    f: &mut Frame,
    area: Rect,
    outline: &mut crate::outline::Outline,
    entries: &[engine::TocEntry],
    ui: UiTheme,
    focused: bool,
    pointer: Option<Position>,
) -> Option<Rect> {
    let list_area = list_body_area(area);
    let list_height = list_area.height.saturating_sub(2) as usize;
    let rows_total = entries.len();
    outline.on_viewport_height(list_height.max(1), rows_total);

    let overflow = rows_total > list_height;
    let text_area = list_text_area(list_area, overflow);
    let inner_width = text_area.width as usize;

    let rows = crate::outline::rows(entries);
    let window: Vec<Line<'static>> = if rows.is_empty() {
        vec![Line::from(Span::styled(
            "no headings",
            Style::new().fg(ui.dimmed_fg),
        ))]
    } else {
        rows.iter()
            .enumerate()
            .skip(outline.scroll)
            .take(list_height)
            .map(|(i, row)| {
                outline_line(
                    row,
                    i == outline.selected,
                    i == outline.active,
                    inner_width,
                    ui,
                )
            })
            .collect()
    };

    render_list_panel(
        f,
        ListPanel {
            area,
            list_area,
            text_area,
            title: " outline ".into(),
            header: Some("TABLE OF CONTENTS".into()),
            focused,
            scroll: outline.scroll,
            rows_total,
            overflow,
        },
        window,
        ui,
        pointer,
    )
}

/// Paints a list panel's label and its rule inside the panel's top border,
/// row for row from [`LIST_HEADER`]: a blank row, a centred bold label, a
/// blank row, then a rule that tees into the panel's right border with
/// leaf's `┤` corner. The rows come out of the list (see
/// [`list_body_area`]); this is chrome above them.
fn draw_list_header(f: &mut Frame, area: Rect, label: &str, ui: UiTheme, focused: bool) {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 {
        return;
    }
    let border = if focused {
        ui.sidebar_border_focused
    } else {
        ui.sidebar_border_unfocused
    };
    let rule_style = Style::new().fg(border).bg(ui.sidebar_bg);
    for (row, kind) in LIST_HEADER.iter().enumerate() {
        let y = inner.y + row as u16;
        if y >= inner.bottom() {
            break;
        }
        match kind {
            ListHeaderRow::Blank => {}
            ListHeaderRow::Label => {
                let style = Style::new()
                    .fg(ui.dimmed_fg)
                    .bg(ui.sidebar_bg)
                    .add_modifier(Modifier::BOLD);
                let width = UnicodeWidthStr::width(label) as u16;
                // Centre the label in the panel; a panel too narrow for it
                // falls back to the left edge rather than offsetting it.
                let x = if width < inner.width {
                    inner.x + (inner.width - width) / 2
                } else {
                    inner.x
                };
                f.buffer_mut().set_stringn(
                    x,
                    y,
                    label,
                    inner.right().saturating_sub(x) as usize,
                    style,
                );
            }
            ListHeaderRow::Rule => {
                f.buffer_mut().set_stringn(
                    inner.x,
                    y,
                    "─".repeat(inner.width as usize),
                    inner.width as usize,
                    rule_style,
                );
                f.buffer_mut()
                    .set_stringn(area.right().saturating_sub(1), y, "┤", 1, rule_style);
            }
        }
    }
}

/// The numbered prefix of a depth-0 outline row: leaf's TOC index, two
/// digits and a space. One home for the prefix's shape and width, so the
/// renderer and its tests cannot drift.
pub(crate) fn outline_index_prefix(index: usize) -> String {
    format!("{index:02} ")
}

/// The display width of [`outline_index_prefix`]: the column the row's
/// title starts at. The renderer measures the prefix it built; the tests
/// use this to place the title without repeating the `NN ` width.
#[cfg(test)]
pub(crate) fn outline_index_width() -> usize {
    UnicodeWidthStr::width(outline_index_prefix(0).as_str())
}

/// One outline row: the heading's two-digit index at depth 0, or an
/// indented bullet deeper down (leaf's table of contents), then the title,
/// truncated to leave room for that prefix. The selection is the reader's
/// bar; the active heading (the one the note's scroll is at) takes the
/// theme's accent in bold, so the two cursors read apart. The index and the
/// bullet use the accent for the active row and the dimmed role otherwise.
fn outline_line(
    row: &crate::outline::Row<'_>,
    selected: bool,
    active: bool,
    inner_width: usize,
    ui: UiTheme,
) -> Line<'static> {
    let prefix = match row.index {
        Some(index) => outline_index_prefix(index),
        None => format!("{}• ", "  ".repeat(row.depth)),
    };
    let title = truncate_ellipsis(&row.entry.title, inner_width.saturating_sub(prefix.width()));
    let marker = if active {
        ui.sidebar_border_focused
    } else {
        ui.dimmed_fg
    };
    let mut line = Line::from(vec![
        Span::styled(prefix, Style::new().fg(marker)),
        Span::raw(title),
    ]);
    if selected {
        highlight_selection(&mut line, inner_width, ui.sidebar_bg, ui);
    } else if active {
        for span in &mut line.spans {
            span.style = span
                .style
                .fg(ui.sidebar_border_focused)
                .bg(ui.sidebar_bg)
                .add_modifier(Modifier::BOLD);
        }
    }
    line
}

fn draw_tag_list(
    f: &mut Frame,
    area: Rect,
    tags: &mut crate::tags::TagList,
    chrome: ListChrome<'_>,
    index: Option<&VaultIndex>,
) -> Option<Rect> {
    let ui = chrome.ui;
    let list_area = list_body_area(area);
    let inner_height = list_area.height.saturating_sub(2) as usize;
    tags.on_viewport_height(inner_height.max(1));
    tags.scroll_selection_into_view();

    let rows_total = tags.rows().len();
    let overflow = rows_total > inner_height;
    let text_area = list_text_area(list_area, overflow);
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
            list_area,
            text_area,
            title: " tags ".into(),
            header: Some(chrome.vault_name.to_string()),
            focused: chrome.focused,
            scroll: tags.scroll,
            rows_total,
            overflow,
        },
        window,
        ui,
        chrome.pointer,
    )
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
        highlight_selection(&mut line, inner_width, ui.sidebar_bg, ui);
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
    // The bar column positions the panel's selection (the click rect above
    // deliberately stops before it).
    app.layout.backlinks_scrollbar_area = bar_shown.then(|| list_scrollbar_area(area));
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
                    highlight_selection(&mut line, inner_width, ui.sidebar_bg, ui);
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
            list_area: area,
            text_area,
            title: format!(" backlinks ({}) ", app.backlinks.len()),
            header: None,
            focused,
            scroll: app.backlink_scroll,
            rows_total,
            overflow: bar_shown,
        },
        window,
        ui,
        app.pointer,
    );
}

/// The chrome rows inside a popup's border: how many rows sit above the
/// list (a prompt) and below it (a footer).
///
/// A popup's height and its [`PopupList`] window both derive from one
/// `PopupChrome`, so a change to the chrome cannot resize the popup and
/// leave the list window behind (or vice versa).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupChrome {
    pub above: u16,
    pub below: u16,
}

impl PopupChrome {
    /// A prompt row above the list and a footer row below it.
    pub const PROMPT_AND_FOOTER: Self = Self { above: 1, below: 1 };
    /// Just a footer row below the list.
    pub const FOOTER: Self = Self { above: 0, below: 1 };

    /// The popup height for `rows` list rows: the border ring plus the
    /// chrome plus the list itself.
    pub fn height(self, rows: u16) -> u16 {
        rows.saturating_add(2 + self.above + self.below)
    }

    /// The list window of a popup carrying this chrome.
    pub fn list(self, popup: Rect, selected: usize, len: usize) -> PopupList {
        PopupList::new(popup, self, selected, len)
    }
}

/// The list window inside a popup: the popup's border and its chrome rows
/// (the quick jumper's prompt above the list, the footer below it) are
/// never item rows. Both the renderers and the click mappers take the
/// window from here, so the drawn rows and the clickable rows cannot
/// drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupList {
    /// The first screen row of the list.
    pub first_row: u16,
    /// The index of the first visible item (the anchor).
    pub start: usize,
    /// How many item rows the list shows.
    pub rows: usize,
    /// The item count: rows past the last item map to nothing.
    len: usize,
}

impl PopupList {
    /// `popup` includes its one-cell border; `chrome` counts the rows
    /// inside that border above and below the list. The window ends at the
    /// selection, so the selection is always visible. With no room for a
    /// list row (a popup clamped to borders + chrome), there are no rows:
    /// every screen row is chrome or border.
    pub fn new(popup: Rect, chrome: PopupChrome, selected: usize, len: usize) -> Self {
        let rows = popup.height.saturating_sub(2 + chrome.above + chrome.below) as usize;
        Self {
            first_row: popup.y + 1 + chrome.above,
            start: selected.saturating_sub(rows.saturating_sub(1)),
            rows,
            len,
        }
    }

    /// The item index at screen row `row`; `None` for the chrome rows (the
    /// borders, the rows above the list, the footer) and for rows past the
    /// last item.
    pub fn index_at(&self, row: u16) -> Option<usize> {
        if row < self.first_row || row >= self.first_row.saturating_add(self.rows as u16) {
            return None;
        }
        let index = self.start + (row - self.first_row) as usize;
        (index < self.len).then_some(index)
    }
}

/// The disambiguation popup's chrome: no prompt row above the candidates,
/// and two rows below them — the footer plus the blank row the popup has
/// always carried under it. Both the height and the list window come from
/// this one value.
const DISAMBIGUATION_CHROME: PopupChrome = PopupChrome { above: 0, below: 2 };

/// Geometry of the disambiguation popup within the terminal `area`: the
/// popup rectangle (borders included), sized to the candidates and clamped
/// to the terminal. Shared by the renderer and its tests, so the width
/// constant has one home.
fn disambiguation_popup_geometry(area: Rect, candidates: usize) -> Rect {
    let width = 60.min(area.width.saturating_sub(2)).max(1);
    let height = DISAMBIGUATION_CHROME
        .height(candidates as u16)
        .min(area.height.saturating_sub(2))
        .max(1);
    centered(area, width, height)
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
    let popup = disambiguation_popup_geometry(area, d.candidates.len());
    let inner_width = popup.width.saturating_sub(2) as usize;
    // Stateless scroll window over the candidates: no chrome above the
    // list, the footer (and its blank row) below it.
    let list = DISAMBIGUATION_CHROME.list(popup, d.selected, d.candidates.len());

    let mut lines: Vec<Line<'static>> = d
        .candidates
        .iter()
        .enumerate()
        .skip(list.start)
        .take(list.rows)
        .map(|(i, path)| {
            let selected = i == d.selected;
            let mut line = Line::from(Span::raw(truncate_ellipsis(
                &rel_label(path, index),
                inner_width,
            )));
            if selected {
                highlight_selection(&mut line, inner_width, ui.content_bg, ui);
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

/// Quick jumper (`Ctrl+K`): the query prompt on top, the fuzzy
/// matches below (the matched characters highlighted), a footer. A
/// stateless scroll window keeps the selection visible when there are more
/// matches than rows. The popup rectangle was recorded into
/// `app.layout.quick_area` by `draw` (the mouse handlers map clicks/wheels
/// through it and the shared [`PopupList`] geometry).
fn draw_quick(f: &mut Frame, area: Rect, app: &App, ui: UiTheme) {
    let popup = app
        .layout
        .quick_area
        .unwrap_or_else(|| crate::quick::popup_geometry(area, app.quick.items.len()));
    let inner_width = popup.width.saturating_sub(2) as usize;
    // The prompt is the chrome row above the list; the footer is below.
    let list = crate::quick::CHROME.list(popup, app.quick.selected, app.quick.items.len());

    let mut lines: Vec<Line<'static>> =
        vec![Line::from(Span::raw(format!(">{}▏", app.quick.query)))];
    if app.quick.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "no matches",
            Style::new().fg(ui.popup_dimmed_fg),
        )));
    } else {
        // Stateless scroll window: the selection always visible.
        lines.extend(
            app.quick
                .items
                .iter()
                .enumerate()
                .skip(list.start)
                .take(list.rows)
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

/// One quick-jumper row: the note's vault-relative path with the
/// matched characters highlighted.
fn quick_item_line(
    item: &crate::quick::QuickItem,
    inner_width: usize,
    selected: bool,
    ui: UiTheme,
) -> Line<'static> {
    let spans = match_spans(&item.label, &item.positions, Style::new(), ui);
    let truncated = truncate_spans(spans, inner_width);
    let mut line = Line::from(truncated);
    if selected {
        // The bar leaves the matched characters' highlight intact (the
        // same rule the search popup uses).
        highlight_selection(&mut line, inner_width, ui.content_bg, ui);
    }
    line
}

/// `text` as spans, the characters at `positions` in the fuzzy-match
/// highlight and the rest in `base`.
fn match_spans(text: &str, positions: &[usize], base: Style, ui: UiTheme) -> Vec<Span<'static>> {
    let match_style = Style::new().fg(ui.popup_match_fg).bg(ui.popup_match_bg);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run_match = false;
    let mut run = String::new();
    for (i, ch) in text.chars().enumerate() {
        let is_match = positions.contains(&i);
        if is_match != run_match {
            if !run.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    if run_match { match_style } else { base },
                ));
            }
            run_match = is_match;
        }
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(
            run,
            if run_match { match_style } else { base },
        ));
    }
    spans
}

/// The spans that fit in `width` columns: whole spans while they fit, then
/// the first that does not is cut at its last character that fits and ends
/// in `…`, so a long path stays visible up to the cut. Shared by the quick
/// jumper and the vault picker rows.
fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut used = 0usize;
    let mut truncated: Vec<Span<'static>> = Vec::new();
    for span in spans {
        let w: usize = UnicodeWidthStr::width(span.content.as_ref());
        if used + w <= width {
            used += w;
            truncated.push(span);
            continue;
        }
        // The `…` takes the last column. When the kept spans fill the width
        // exactly, that column comes off their tail: the row still overflows,
        // so it still ends in `…`.
        let room = width - used;
        if room > 0 {
            let budget = room - 1;
            let mut cut = String::new();
            let mut cut_width = 0usize;
            for ch in span.content.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                if cut_width + cw > budget {
                    break;
                }
                cut.push(ch);
                cut_width += cw;
            }
            cut.push('…');
            truncated.push(Span::styled(cut, span.style));
        } else if let Some(style) = give_back_column(&mut truncated) {
            truncated.push(Span::styled("…", style));
        }
        break;
    }
    truncated
}

/// Takes characters off the tail of `kept` until a column is free, for the
/// `…` of a row whose kept spans fill its width. Returns the style the `…`
/// takes: the span the tail ends in, or the last span trimmed when no text is
/// left. `None` when there is nothing to trim (a zero-width row).
fn give_back_column(kept: &mut Vec<Span<'static>>) -> Option<Style> {
    let mut freed = 0usize;
    let mut last_style = None;
    while freed == 0 {
        let last = kept.last_mut()?;
        last_style = Some(last.style);
        if let Some(ch) = last.content.to_mut().pop() {
            freed += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        if last.content.is_empty() {
            kept.pop();
        }
    }
    kept.last().map(|span| span.style).or(last_style)
}

/// The startup vault picker (SPEC "Vault picker"), full-screen over the
/// content background.
pub fn draw_vault_picker(f: &mut Frame, picker: &crate::vault_picker::PickerState, ui: UiTheme) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(ui.content_bg)), area);
    draw_vault_picker_popup(f, area, picker, ui, "Esc quit");
}

/// The vault picker's popup: the quick jumper's chrome — filter prompt,
/// the entries (name, then the path dimmed, matches highlighted, a recent
/// path tagged `recent`), and a footer that carries the picker's error
/// when there is one. A typed path lists its one `open` row; with nothing
/// to list, the empty state says how to type one. `esc` names what `Esc`
/// does there (quit at startup, close in the app).
fn draw_vault_picker_popup(
    f: &mut Frame,
    area: Rect,
    picker: &crate::vault_picker::PickerState,
    ui: UiTheme,
    esc: &str,
) {
    let path_mode = picker.path_mode();
    let rows = if path_mode { 1 } else { picker.items.len() };
    let popup = crate::quick::popup_geometry(area, rows);
    let inner_width = popup.width.saturating_sub(2) as usize;
    let list = crate::quick::CHROME.list(popup, picker.selected, picker.items.len());
    let name_width = picker
        .entries
        .iter()
        .map(|entry| UnicodeWidthStr::width(entry.name.as_str()))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::raw(format!(">{}▏", picker.query)))];
    if path_mode {
        let mut line = Line::from(truncate_spans(
            vec![Span::raw(format!("open {}", picker.query))],
            inner_width,
        ));
        highlight_selection(&mut line, inner_width, ui.content_bg, ui);
        lines.push(line);
    } else if picker.items.is_empty() {
        let empty = if picker.entries.is_empty() {
            "type a path to a vault (/, ~ or .)"
        } else {
            "no matches"
        };
        lines.push(Line::from(Span::styled(
            empty,
            Style::new().fg(ui.popup_dimmed_fg),
        )));
    } else {
        lines.extend(
            picker
                .items
                .iter()
                .enumerate()
                .skip(list.start)
                .take(list.rows)
                .map(|(i, item)| {
                    let entry = &picker.entries[item.entry];
                    let pad = name_width - UnicodeWidthStr::width(entry.name.as_str()) + 2;
                    let mut spans =
                        match_spans(&entry.name, &item.name_positions, Style::new(), ui);
                    spans.push(Span::raw(" ".repeat(pad)));
                    spans.extend(match_spans(
                        &entry.display,
                        &item.path_positions,
                        Style::new().fg(ui.popup_dimmed_fg),
                        ui,
                    ));
                    if entry.recent {
                        spans.push(Span::styled(
                            "  recent",
                            Style::new().fg(ui.popup_dimmed_fg),
                        ));
                    }
                    let mut line = Line::from(truncate_spans(spans, inner_width));
                    if i == picker.selected {
                        highlight_selection(&mut line, inner_width, ui.content_bg, ui);
                    }
                    line
                }),
        );
    }
    lines.push(match &picker.error {
        Some(error) => Line::from(Span::styled(
            error.clone(),
            Style::new().fg(ui.status_error_fg),
        )),
        None if path_mode => Line::from(Span::styled(
            format!("Tab complete · Enter open · {esc}"),
            Style::new().fg(ui.popup_dimmed_fg),
        )),
        None => Line::from(Span::styled(
            format!("↑/↓ move · Enter open · {esc}"),
            Style::new().fg(ui.popup_dimmed_fg),
        )),
    });

    let title = format!(" vaults: {} ", rows);
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

/// Global search popup (`s`): input prompt or results list, depending on
/// the search state's mode. Results rows are `relpath:line snippet` with
/// the match highlighted; a stateless scroll window keeps the selection
/// visible when there are more hits than rows. The popup rectangle was
/// recorded into `app.layout.search_area` by `draw` (the mouse handlers map
/// clicks/wheels through it and the shared [`PopupList`] geometry).
fn draw_search(f: &mut Frame, area: Rect, app: &App, ui: UiTheme, index: Option<&VaultIndex>) {
    let popup = app.layout.search_area.unwrap_or_else(|| {
        crate::search::popup_geometry(area, app.search.mode, app.search.hits.len())
    });
    let inner_width = popup.width.saturating_sub(2) as usize;
    let footer = match app.search.error.as_deref() {
        Some(error) => error.to_string(),
        None => match app.search.mode {
            SearchMode::Input => "Enter search · Esc cancel".to_string(),
            SearchMode::Results => "j/k move · Enter open · Esc close".to_string(),
        },
    };
    // A long regex diagnostic is one line: clamp it to the popup's inner
    // width instead of letting the border cut it mid-word.
    let footer = truncate_ellipsis(&footer, inner_width);

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
                // Results mode has no prompt row above the list.
                let list = crate::search::RESULTS_CHROME.list(
                    popup,
                    app.search.selected,
                    app.search.hits.len(),
                );
                app.search
                    .hits
                    .iter()
                    .enumerate()
                    .skip(list.start)
                    .take(list.rows)
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
            highlight_selection(&mut line, inner_width, ui.content_bg, ui);
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
    let mut line = Line::from(spans);
    if selected {
        // The bar leaves the match snippet's own highlight intact.
        highlight_selection(&mut line, inner_width, ui.content_bg, ui);
    }
    line
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
        SidebarMode::Outline => {
            // The selected heading's title; a `.base` table in the content
            // pane has no outline (see `app::outline_rows`).
            crate::app::outline_rows(&app.base, &app.viewer)
                .get(app.outline.selected)
                .map(|entry| entry.title.clone())
                .unwrap_or_else(|| "(no headings)".to_string())
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
    // Message precedence: the real error (red; `last_error` or a `.base`
    // parse warning) wins, then the walk warning, then the info status.
    // While any of them is shown the info status waits; the walk warning
    // has its own slot, so it never overwrote the real error.
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
    // The vault's unreadable-path state (a directory the walk cannot read,
    // a file the index could not read — at startup or in a later reindex):
    // error-ish, but it outlives a transient info message and clears with
    // the condition.
    if let Some(warning) = &app.walk_warning {
        spans.push(Span::styled(
            format!(" · {warning}"),
            Style::new().fg(ui.status_error_fg),
        ));
        error_shown = true;
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
            ("j / k, ↑ ↓", "scroll · move the selection (lists)"),
            ("PgDn/PgUp", "half page · g/G, Home/End top/bottom"),
            ("mouse wheel", "scroll pane under cursor / focused"),
            ("Tab", "show / hide sidebar"),
            ("f", "focus next pane: sidebar → note → backlinks"),
            (
                "h / l, ← →, Space",
                "collapse / expand / fold (tree and tags)",
            ),
            ("b", "toggle backlinks panel"),
            ("s / Ctrl+K", "global search · quick jumper"),
            ("n / N", "next/previous search match (note pane)"),
            ("Backspace / Ctrl+O", "back / forward (scroll restored)"),
        ],
    ),
    (
        "Links & tasks",
        &[
            ("x / X", "focus next / previous link or task"),
            ("Enter / click", "open link or row / toggle checkbox"),
            ("dimmed links", "unresolved (struck out, not activatable)"),
        ],
    ),
    (
        "Content",
        &[
            ("m", "show / hide the note's metadata block"),
            ("w (content)", "toggle watch mode"),
            ("] / [, 1-9", "switch table view: next / previous / nth"),
        ],
    ),
    (
        "Sidebar",
        &[
            ("t / o", "sidebar mode: tree ⇄ tags / outline"),
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
            ("M", "toggle mouse capture; off: wheel = arrows"),
            ("v", "switch vault (an entry, a recent or typed path)"),
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
    /// The layout the popup renders, so `draw_help` need not rebuild it.
    grid: HelpGrid,
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
        grid,
        total,
        visible: help_visible_rows(height),
    }
}

fn draw_help(f: &mut Frame, area: Rect, app: &mut App, ui: UiTheme) {
    // The bar is only a position control while it is actually drawn; a
    // popup whose body fits leaves the field `None`.
    app.layout.help_scrollbar_area = None;
    // Dim everything behind the popup (herdr's overlay treatment). The popup
    // cells are cleared below, which resets their style and so drops DIM.
    let buf = f.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }

    let HelpPopup {
        rect: popup,
        grid,
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
        // Geometry for the mouse handlers: the bar is a body-position
        // control, mapped over the popup's own scroll range.
        app.layout.help_scrollbar_area = Some(bar_area);
        app.layout.help_scroll_max = max_scroll;
        draw_scrollbar(
            f,
            bar_area,
            BarSpan {
                total,
                position: scroll,
                viewport: body_height as usize,
            },
            ui.content_bg,
            ui,
            app.pointer,
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
