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

/// The note sits inside the configured margin on both sides: the text
/// starts `margin` columns in, and the viewport the mouse uses matches.
#[test]
fn note_text_starts_after_the_configured_margin() {
    let file = ui_viewer("ui-margin.md", "first line here\n");
    let mut app = app_for(&file, None);
    app.note_margin = 3;
    app.theme_ui.content_bg = Color::Rgb(9, 9, 9);
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(2, 0)].symbol(), " ", "left margin is blank");
    assert_eq!(
        buffer[(0, 0)].bg,
        Color::Rgb(9, 9, 9),
        "left margin painted"
    );
    assert_eq!(
        buffer[(99, 0)].bg,
        Color::Rgb(9, 9, 9),
        "right margin painted"
    );
    assert_eq!(buffer[(3, 0)].symbol(), "f", "text starts at the margin");
    assert_eq!(app.layout.note_viewport.x, 3);
    assert_eq!(app.layout.note_viewport.width, 94, "100 less 3 per side");
}

/// Scrolling to the bottom of an overflowing note must land on the
/// bottom, even when the scrollbar re-wraps the note one column
/// narrower. Each paragraph below is exactly the full text width, so
/// the narrower wrap adds a line to every paragraph. The scroll must
/// not be clamped against the provisional full-width wrap on the way.
#[test]
fn bottom_scroll_survives_the_scrollbar_rewrap() {
    let paragraph = "abcdefghi abcdefghi abcdefghi abcdefghi abcdefghi \
                     abcdefghi abcdefghi abcdefghi abcdefghi abcdefgh";
    let mut body = String::new();
    for _ in 0..120 {
        body.push_str(paragraph);
        body.push_str("\n\n");
    }
    body.push_str("LAST MARKER LINE\n");
    let file = ui_viewer("ui-bottom-scroll.md", &body);
    let mut app = app_for(&file, None);
    render(&mut app);
    app.viewer.scroll_bottom();
    let screen = render(&mut app);
    assert_eq!(
        app.viewer.scroll,
        app.viewer.max_scroll(),
        "scroll kept at bottom"
    );
    let rows: Vec<&str> = screen.lines().collect();
    let note_rows = &rows[..rows.len() - 1];
    let marker_row = note_rows.iter().rposition(|r| r.contains("LAST MARKER"));
    // Only the note's final paragraph break follows the last text line,
    // so it sits on the row just above the bottom.
    assert_eq!(
        marker_row,
        Some(note_rows.len() - 2),
        "last line above the paragraph break"
    );
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
fn status_bar_shows_only_the_name_when_the_name_is_shared() {
    let dir = name_vault();
    let mut app = status_app(&dir, "a/index.md");
    let screen = render_at(&mut app, 100);
    let row = status_row(&screen);
    let label = format!("{} index.md", app.icons.note());
    assert!(row.contains(&label), "bare name with its icon: {row}");
    assert!(!row.contains("a/index.md"), "no relative path: {row}");
    assert!(
        !row.contains(&dir.path().display().to_string()),
        "vault root not on the bar: {row}"
    );
}

#[test]
fn status_bar_shows_only_the_name_when_sidebar_focused() {
    let dir = name_vault();
    let mut app = status_app(&dir, "solo.md");
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, crate::app::Focus::Sidebar);
    let screen = render_at(&mut app, 100);
    let row = status_row(&screen);
    assert!(
        row.trim_start_matches('"').starts_with("a ") && row.contains(" lines"),
        "focused folder shows its name, no path: {row}"
    );
    assert!(
        !row.contains(&dir.path().display().to_string()),
        "no full path on the bar: {row}"
    );
}

#[test]
fn status_bar_icons_follow_the_icon_set() {
    let dir = name_vault();
    let mut app = status_app(&dir, "solo.md");
    let screen = render_at(&mut app, 100);
    assert!(
        status_row(&screen).contains(&format!("{} solo.md", crate::config::IconSet::Nerd.note())),
        "nerd is the default note icon"
    );
    app.icons = crate::config::IconSet::Unicode;
    let screen = render_at(&mut app, 100);
    assert!(
        status_row(&screen).contains("¶ solo.md"),
        "unicode note icon"
    );
}

#[test]
fn status_bar_name_segment_records_rect_only_in_content_focus() {
    let dir = name_vault();
    let mut app = status_app(&dir, "solo.md");
    render_at(&mut app, 100);
    let area = app.layout.status_name_area.expect("name segment recorded");
    assert_eq!(area.y, 23, "status row");
    assert_eq!(area.x, 0);
    let label = format!("{} solo.md", app.icons.note());
    assert_eq!(area.width as usize, UnicodeWidthStr::width(label.as_str()));
    app.handle_key(key(KeyCode::Char('h')));
    render_at(&mut app, 100);
    assert!(
        app.layout.status_name_area.is_none(),
        "sidebar focus: no name"
    );
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
    assert_eq!(app.viewer.render_width, 96, "full width minus margins");
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
    // Content re-wrapped to the narrower area: 100 - 30 - 2 margins = 66.
    assert_eq!(app.viewer.render_width, 66);

    // Focus the sidebar: the status bar shows the selected entry's path
    // (the first row is the `sub` folder — dirs sort first).
    app.handle_key(key(KeyCode::Char('h')));
    let screen = render(&mut app);
    assert!(
        status_row(&screen)
            .trim_start_matches('"')
            .starts_with("sub ")
            && status_row(&screen).contains(" lines"),
        "status bar shows the focused folder's name"
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
    assert_eq!(app.viewer.render_width, 96);
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
    let hit = app.layout.backlinks_area.expect("panel drawn");
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
    app.wait_git();
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
        changed_files: [Path::new("/x/a.md").to_path_buf()].into(),
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
        changed_files: Default::default(),
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
        changed_files: [vault_dir.path().join("alpha.md")].into(),
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

/// The help popup builds its own scrollbar state: its thumb must end on the
/// last track row at the bottom too, and span the visible fraction of the
/// body.
#[test]
fn help_scrollbar_thumb_ends_track_the_scroll_position() {
    let file = ui_viewer("ui-bar-help-ends.md", "x\n");
    let mut app = app_for(&file, None);
    app.viewer.help_open = true;
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();

    // The popup is the only scrollbar at 100x30 (the note is one line): find
    // its begin arrow, then read the bar's column below it.
    let buffer = terminal.backend().buffer();
    let (x, top) = (0..buffer.area.width)
        .find_map(|x| {
            (0..buffer.area.height).find_map(|y| (buffer[(x, y)].symbol() == "▲").then_some((x, y)))
        })
        .expect("help scrollbar drawn");
    let track = scrollbar_track_rows(buffer, x, top..buffer.area.height);
    let thumb = scrollbar_thumb_rows(buffer, x, top..buffer.area.height);
    assert_eq!(
        thumb.first(),
        track.first(),
        "at the top the thumb starts at the first track row"
    );
    // The bar area is the body rows (two arrow heads included).
    let body_height = track.len() + 2;
    let total = help_grid(buffer.area.height).rows;
    let expected = proportional_thumb_len(total, body_height, track.len());
    assert!(
        (thumb.len() as f64 - expected).abs() <= 1.0,
        "thumb length {} is within 1 row of the proportional {expected:.1}",
        thumb.len()
    );

    // `G` clamps the scroll to the body's end at draw time.
    app.viewer.help_scroll = usize::MAX;
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let track = scrollbar_track_rows(buffer, x, top..buffer.area.height);
    let thumb = scrollbar_thumb_rows(buffer, x, top..buffer.area.height);
    assert_eq!(
        thumb.last(),
        track.last(),
        "at the bottom the thumb ends at the last track row"
    );
}

// --- scrollbars (markdown viewer, sidebar tree, tag list) -------------

/// Buffer symbols of a whole row, left to right.
fn row_symbols(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}

/// Rows of column `x` drawn as the scrollbar's track or thumb, arrow heads
/// excluded: the extent a proportional thumb has to span. The default
/// symbols are `║` (track), `█` (thumb), `▲`/`▼` (heads).
fn scrollbar_track_rows(
    buffer: &ratatui::buffer::Buffer,
    x: u16,
    ys: std::ops::Range<u16>,
) -> Vec<u16> {
    ys.into_iter()
        .filter(|&y| matches!(buffer[(x, y)].symbol(), "║" | "█"))
        .collect()
}

/// Rows of column `x` drawn as the scrollbar thumb.
fn scrollbar_thumb_rows(
    buffer: &ratatui::buffer::Buffer,
    x: u16,
    ys: std::ops::Range<u16>,
) -> Vec<u16> {
    ys.into_iter()
        .filter(|&y| buffer[(x, y)].symbol() == "█")
        .collect()
}

/// Length a proportional thumb should have on a `track_len` track: the
/// visible fraction of the content.
fn proportional_thumb_len(total: usize, visible: usize, track_len: usize) -> f64 {
    visible as f64 / total as f64 * track_len as f64
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
    assert_eq!(
        app.layout.note_viewport.width, 96,
        "text width less the margins"
    );
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
    assert_eq!(
        app.layout.note_viewport.x + app.layout.note_viewport.width,
        99 - app.note_margin
    );
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

/// The thumb's ends track the scroll position: the first track row at the
/// top, the *last* track row at the bottom (the bug this pins stopped it
/// two rows short), and its length is the visible fraction of the note.
#[test]
fn note_scrollbar_thumb_ends_track_the_scroll_position() {
    let file = ui_viewer("ui-bar-ends.md", &long_note(200));
    let mut app = app_for(&file, None);
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let track = scrollbar_track_rows(buffer, 99, 0..23);
    let thumb = scrollbar_thumb_rows(buffer, 99, 0..23);
    assert_eq!(
        thumb.first(),
        track.first(),
        "at the top the thumb starts at the first track row"
    );
    // The bar spans 23 rows here (2 arrow heads), the note 200 lines.
    let expected = proportional_thumb_len(200, 23, track.len());
    assert!(
        (thumb.len() as f64 - expected).abs() <= 1.0,
        "thumb length {} is within 1 row of the proportional {expected:.1}",
        thumb.len()
    );

    app.viewer.scroll_bottom();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let track = scrollbar_track_rows(buffer, 99, 0..23);
    let thumb = scrollbar_thumb_rows(buffer, 99, 0..23);
    assert_eq!(
        thumb.last(),
        track.last(),
        "at the bottom the thumb ends at the last track row"
    );
}

#[test]
fn note_scrollbar_click_mapping_matches_the_narrowed_viewport() {
    let file = ui_viewer("ui-bar-click.md", &long_note(200));
    let mut app = app_for(&file, None);
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    // The viewport recorded for clicks is exactly the drawn text area.
    let vp = app.layout.note_viewport;
    assert_eq!(vp.width, 95, "100 columns less the margins and the bar");
    assert_eq!(vp.x, app.note_margin);
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

/// The list panels share the helper, so the sidebar tree stands in for the
/// tag list and backlinks: the thumb reaches the last track row at the
/// bottom of a long list, and spans the visible fraction of it.
#[test]
fn sidebar_scrollbar_thumb_ends_track_the_scroll_position() {
    let vault_dir = TempDir::new("ui-bar-side-ends");
    for i in 0..40 {
        std::fs::write(vault_dir.path().join(format!("n{i:02}.md")), "x\n").unwrap();
    }
    let file = ui_viewer("ui-bar-side-ends.md", "x\n");
    let mut app = app_for(&file, Some(vault_dir.path()));
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();

    let bar_x = sidebar_width(100) - 2;
    let sidebar_area = app.layout.sidebar_area.expect("sidebar visible");
    let inner = sidebar_area.y + 1..sidebar_area.bottom() - 1;
    let rows_total = app.sidebar.as_ref().expect("sidebar").row_count();
    let inner_height = sidebar_area.height.saturating_sub(2) as usize;

    let buffer = terminal.backend().buffer();
    let track = scrollbar_track_rows(buffer, bar_x, inner.clone());
    let thumb = scrollbar_thumb_rows(buffer, bar_x, inner.clone());
    assert_eq!(
        thumb.first(),
        track.first(),
        "at the top the thumb starts at the first track row"
    );
    let expected = proportional_thumb_len(rows_total, inner_height, track.len());
    assert!(
        (thumb.len() as f64 - expected).abs() <= 1.0,
        "thumb length {} is within 1 row of the proportional {expected:.1}",
        thumb.len()
    );

    // Select the last row: the list scrolls to its end.
    app.sidebar.as_mut().expect("sidebar").move_down(rows_total);
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    let track = scrollbar_track_rows(buffer, bar_x, inner.clone());
    let thumb = scrollbar_thumb_rows(buffer, bar_x, inner);
    assert_eq!(
        thumb.last(),
        track.last(),
        "at the bottom the thumb ends at the last track row"
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
    let area = app.layout.sidebar_area.expect("sidebar visible");
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
    let area = app.layout.sidebar_area.expect("sidebar visible");
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
    let area = app.layout.sidebar_area.expect("sidebar visible");
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
        let lines = help_body(app_ui_theme(), &grid);
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
/// and never in the compact layout.
#[test]
fn help_has_one_blank_line_between_groups() {
    assert_eq!(help_rows(true), help_rows(false) + HELP_SECTIONS.len() - 1);
    let blanks_in = |spaced: bool| {
        let grid = HelpGrid {
            width: HELP_WIDTH,
            rows: help_rows(spaced),
            spaced,
        };
        help_body(app_ui_theme(), &grid)
            .iter()
            .filter(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .count()
    };
    assert_eq!(
        blanks_in(true),
        HELP_SECTIONS.len() - 1,
        "one blank row between groups"
    );
    assert_eq!(blanks_in(false), 0, "none in the compact layout");
}

/// Layout is always a single column: the spaced layout (blank rows
/// between groups) when it fits the frame height, the compact one
/// (which scrolls) otherwise.
#[test]
fn help_grid_is_single_column_spaced_when_it_fits() {
    let tall = help_grid(40);
    assert!(tall.spaced, "spaced when the body fits");
    assert_eq!(
        help_body(app_ui_theme(), &tall).len(),
        tall.rows,
        "every group placed once"
    );
    assert!(tall.width <= 140, "sane width on a wide terminal");

    // Short frame: compact layout, the body scrolls.
    let short = help_grid(30);
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
            let inside = x >= popup.x && x < popup.right() && y >= popup.y && y < popup.bottom();
            let dim = buffer[(x, y)].modifier.contains(Modifier::DIM);
            if inside {
                assert!(!dim, "popup cell ({x},{y}) must not be dimmed");
            } else {
                assert!(dim, "background cell ({x},{y}) must be dimmed");
            }
        }
    }
}

// --- the quick note switcher (Ctrl+P) --------------------------------------

#[test]
fn quick_switcher_renders_the_prompt_the_matches_and_the_footer() {
    let vault_dir = TempDir::new("ui-quick");
    std::fs::write(vault_dir.path().join("alpha.md"), "alpha\n").unwrap();
    std::fs::create_dir_all(vault_dir.path().join("sub")).unwrap();
    std::fs::write(vault_dir.path().join("sub").join("beta.md"), "beta\n").unwrap();
    let file = ui_viewer("ui-quick.md", "x\n");
    let mut app = app_with_index(&file, &vault_dir);
    app.open_quick();
    assert!(app.quick.open);
    app.quick.push_char('b');
    let screen = render(&mut app);
    assert!(
        screen.contains(">b▏"),
        "the prompt shows the query: {screen}"
    );
    assert!(screen.contains("beta.md"), "the matching note is listed");
    assert!(screen.contains("jump:"), "the popup title shows the count");
    assert!(
        screen.contains("↑/↓ move · Enter open · Esc close"),
        "the footer documents the keys: {screen}"
    );
}

#[test]
fn quick_switcher_reports_no_matches() {
    let vault_dir = TempDir::new("ui-quick-none");
    std::fs::write(vault_dir.path().join("alpha.md"), "alpha\n").unwrap();
    let file = ui_viewer("ui-quick-none.md", "x\n");
    let mut app = app_with_index(&file, &vault_dir);
    app.open_quick();
    app.quick.push_char('z');
    app.quick.push_char('z');
    app.quick.push_char('z');
    let screen = render(&mut app);
    assert!(screen.contains("no matches"), "{screen}");
}

#[test]
fn quick_switcher_highlights_the_matched_characters() {
    let vault_dir = TempDir::new("ui-quick-hl");
    std::fs::write(vault_dir.path().join("alpha.md"), "alpha\n").unwrap();
    let file = ui_viewer("ui-quick-hl.md", "x\n");
    let mut app = app_with_index(&file, &vault_dir);
    app.open_quick();
    app.quick.push_char('a');
    app.quick.push_char('l');
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| draw(f, &mut app)).unwrap();
    let buffer = terminal.backend().buffer();
    // The popup is 70 wide, centered: x = 15. The result row is the
    // second inner row (the prompt is the first).
    let popup = crate::quick::popup_geometry(Rect::new(0, 0, 100, 24), app.quick.items.len());
    let row = popup.y + 2;
    // The label is `alpha.md`; the match covers `al` (columns 16..18).
    let matched_bg = buffer[(popup.x + 1, row)].bg;
    let unmatched_bg = buffer[(popup.x + 5, row)].bg;
    assert_ne!(
        matched_bg, unmatched_bg,
        "the matched characters are highlighted differently"
    );
}
