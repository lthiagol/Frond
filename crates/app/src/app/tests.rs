use super::*;
use std::cell::RefCell;
use std::rc::Rc;

use vault::VaultIndex;

use crate::testutil::{TempDir, TempFile};
use crate::theme::Appearance;

/// A tiny vault: root/a.md, root/sub/b.md, root/sub/c.md.
fn vault() -> TempDir {
    let dir = TempDir::new("app-vault");
    std::fs::write(dir.path().join("a.md"), "A\n\n".repeat(50)).unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("b.md"), "B\n\n".repeat(50)).unwrap();
    std::fs::write(dir.path().join("sub").join("c.md"), "C\n\n".repeat(50)).unwrap();
    dir
}

fn shared_index(dir: &TempDir) -> SharedIndex {
    Rc::new(RefCell::new(VaultIndex::build(dir.path()).unwrap()))
}

fn app(vault_dir: &TempDir) -> (TempFile, App) {
    let file = TempFile::new("app-main.md", &"MAIN\n\n".repeat(50));
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let index = shared_index(vault_dir);
    let app = App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap();
    (file, app)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

#[test]
fn tab_toggles_and_snaps_focus() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    assert!(app.sidebar_visible, "visible by default with --vault");
    assert_eq!(app.handle_key(key(KeyCode::Tab)), Flow::Continue);
    assert!(!app.sidebar_visible);
    assert_eq!(app.focus, Focus::Content);
    app.handle_key(key(KeyCode::Tab));
    assert!(app.sidebar_visible);
    app.focus = Focus::Sidebar;
    app.handle_key(key(KeyCode::Tab));
    assert!(!app.sidebar_visible);
    assert_eq!(
        app.focus,
        Focus::Content,
        "focus snaps out of hidden sidebar"
    );
}

#[test]
fn m_toggles_mouse_capture_intent() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    assert!(app.mouse_capture, "capture on by default");
    app.handle_key(key(KeyCode::Char('m')));
    assert!(!app.mouse_capture, "m turns capture off");
    app.handle_key(key(KeyCode::Char('m')));
    assert!(app.mouse_capture, "m turns capture back on");
}

/// `f`: the frontmatter metadata block hides and comes back; the
/// toggle survives into later reparses (scroll/width changes).
#[test]
fn f_toggles_the_metadata_block() {
    let file = TempFile::new(
        "meta-toggle.md",
        "---\ntitle: Demo\nstatus: active\n---\n# Head\n\nbody\n",
    );
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();

    fn first_line_text(app: &App) -> String {
        app.viewer.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    fn has_text(app: &App, needle: &str) -> bool {
        app.viewer
            .lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
    }

    assert!(app.viewer.show_metadata, "shown by default");
    assert!(has_text(&app, "Demo"), "the metadata block renders");
    assert!(has_text(&app, "Head"), "the body renders too");

    app.handle_key(key(KeyCode::Char('f')));
    assert!(!app.viewer.show_metadata);
    assert!(!has_text(&app, "Demo"), "the metadata block is gone");
    assert_eq!(
        first_line_text(&app).trim_start(),
        "Head",
        "the body starts at the top with the block hidden"
    );

    // Back on, and the toggle holds across an unrelated reparse.
    app.handle_key(key(KeyCode::Char('f')));
    assert!(app.viewer.show_metadata);
    assert!(has_text(&app, "Demo"));
    app.handle_key(key(KeyCode::Char('f')));
    app.viewer.on_content_width(60);
    assert!(
        !has_text(&app, "Demo"),
        "the hidden state survives a width reparse"
    );
}

#[test]
fn sidebar_width_keys_step_clamp_and_survive_hide() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    // The layout has not run yet: no drawn area, so the step starts
    // from the fallback default (60) — clamped down from there.
    app.handle_key(key(KeyCode::Char('<')));
    assert_eq!(app.sidebar_width, Some(58));

    // Draw once so the drawn area feeds the proportional default (30
    // at 100 cols), then step from the real current width.
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.sidebar_width = None;
    app.handle_key(key(KeyCode::Char('>')));
    assert_eq!(app.sidebar_width, Some(32), "steps up from the default");
    app.handle_key(key(KeyCode::Char('<')));
    app.handle_key(key(KeyCode::Char('<')));
    assert_eq!(app.sidebar_width, Some(28));

    // A config-provided width steps the same way.
    app.sidebar_width = Some(40);
    app.handle_key(key(KeyCode::Char('>')));
    assert_eq!(app.sidebar_width, Some(42), "steps from the config value");

    // Clamped at both ends (and no u16 underflow from a degenerate
    // drawn width).
    app.layout.sidebar_area = Some(Rect::new(0, 0, 1, 20));
    app.sidebar_width = None;
    app.handle_key(key(KeyCode::Char('<')));
    assert_eq!(app.sidebar_width, Some(SIDEBAR_MIN_WIDTH), "no underflow");
    app.sidebar_width = Some(SIDEBAR_MIN_WIDTH);
    app.handle_key(key(KeyCode::Char('<')));
    assert_eq!(app.sidebar_width, Some(SIDEBAR_MIN_WIDTH));
    for _ in 0..50 {
        app.handle_key(key(KeyCode::Char('>')));
    }
    assert_eq!(app.sidebar_width, Some(SIDEBAR_MAX_WIDTH));

    // Survives a sidebar hide/show cycle.
    app.handle_key(key(KeyCode::Tab));
    assert!(!app.sidebar_visible);
    assert_eq!(app.sidebar_width, Some(SIDEBAR_MAX_WIDTH));
}

#[test]
fn session_snapshot_and_restore_roundtrip() {
    let vault_dir = vault();
    let (_file, mut shaped) = app(&vault_dir);

    // Shape a session: open a nested note, scroll it, expand the tree,
    // select its row.
    shaped
        .open_note(&vault_dir.path().join("sub/b.md"))
        .unwrap();
    shaped.viewer.scroll = 5;
    if let Some(sidebar) = &mut shaped.sidebar {
        sidebar.expanded.insert(vault_dir.path().join("sub"));
        sidebar.selected = Some(vault_dir.path().join("sub/b.md"));
    }
    let state = shaped.session_state(vault_dir.path());
    assert_eq!(state.note.as_deref(), Some("sub/b.md"));
    assert_eq!(state.scroll, 5);
    assert_eq!(state.expanded, vec!["sub".to_string()]);
    assert_eq!(state.selected.as_deref(), Some("sub/b.md"));
    assert_eq!(state.mode.as_deref(), Some("tree"));

    // A fresh app restores it: note, scroll, expansion and selection.
    let (file2, mut fresh) = app(&vault_dir);
    assert_eq!(fresh.viewer.path, file2.path(), "precondition: index open");
    fresh.restore_session(&state, vault_dir.path());
    assert_eq!(fresh.viewer.path, vault_dir.path().join("sub/b.md"));
    assert_eq!(fresh.viewer.scroll, 5);
    assert!(fresh
        .sidebar
        .as_ref()
        .unwrap()
        .expanded
        .contains(&vault_dir.path().join("sub")));
    assert_eq!(
        fresh.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("sub/b.md"))
    );
    assert_eq!(fresh.sidebar_mode, SidebarMode::Tree);

    // Tags mode restores too.
    let mut tagged = state;
    tagged.mode = Some("tags".to_string());
    fresh.restore_session(&tagged, vault_dir.path());
    assert_eq!(fresh.sidebar_mode, SidebarMode::Tags);
}

#[test]
fn session_restore_survives_missing_note() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    let state = crate::session::SessionState {
        note: Some("gone.md".to_string()),
        scroll: 3,
        expanded: vec![],
        selected: Some("gone.md".to_string()),
        mode: Some("tree".to_string()),
    };
    app.restore_session(&state, vault_dir.path());
    // Open failed silently: the startup note stays, scroll untouched.
    assert_eq!(app.viewer.path, _file.path());
    assert_eq!(app.viewer.scroll, 0);
    assert!(app.viewer.last_error.is_none(), "no startup noise");
}

/// Opening a note from anywhere but the tree (a link follow, search
/// hit, backlink, session restore) reveals it in the sidebar: the
/// ancestor folders expand and the row is selected.
#[test]
fn open_note_reveals_the_note_in_the_sidebar() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    // The tree starts fully collapsed; the sidebar is visible.
    assert!(!app
        .sidebar
        .as_ref()
        .unwrap()
        .expanded
        .contains(&vault_dir.path().join("sub")));

    app.open_note(&vault_dir.path().join("sub/b.md")).unwrap();
    let sidebar = app.sidebar.as_ref().unwrap();
    assert!(
        sidebar.expanded.contains(&vault_dir.path().join("sub")),
        "the note's folder expanded"
    );
    assert_eq!(
        sidebar.selected,
        Some(vault_dir.path().join("sub/b.md")),
        "the opened note is selected"
    );
    assert_eq!(app.focus, Focus::Content, "no focus change");

    // The reveal persists in the session snapshot (expansion set).
    let state = app.session_state(vault_dir.path());
    assert!(state.expanded.contains(&"sub".to_string()));
    assert_eq!(state.selected.as_deref(), Some("sub/b.md"));
}

/// The session file lives outside the vault and is user-editable, so
/// restore must not honor absolute paths or `..` escapes; a `.base`
/// note reopens as the table with its row offset.
#[test]
fn session_restore_rejects_escapes_and_restores_base_tables() {
    let vault_dir = vault();
    let (_file, mut base_app) = app(&vault_dir);
    std::fs::write(
        vault_dir.path().join("tbl.base"),
        "views:\n  - type: table\n    name: Solo\n    order:\n      - file.name\n",
    )
    .unwrap();
    base_app
        .open_note(&vault_dir.path().join("tbl.base"))
        .unwrap();
    base_app.base.as_mut().unwrap().scroll = 2;

    // Snapshot while the base is open: the base file (not the previous
    // note) and its row offset are what gets remembered.
    let state = base_app.session_state(vault_dir.path());
    assert_eq!(
        state.note.as_deref(),
        Some("tbl.base"),
        "an open base table is the remembered note"
    );
    assert_eq!(state.scroll, 2);

    // Escape attempts are skipped, not opened.
    let hostile = crate::session::SessionState {
        note: Some("../../etc/passwd".to_string()),
        scroll: 0,
        expanded: vec!["../..".to_string()],
        selected: Some("/etc".to_string()),
        mode: Some("tree".to_string()),
    };
    base_app.restore_session(&hostile, vault_dir.path());
    assert!(
        base_app.current_path().ends_with("tbl.base"),
        "hostile note skipped: the base table stayed open"
    );
    assert!(
        !base_app
            .sidebar
            .as_ref()
            .unwrap()
            .expanded
            .iter()
            .any(|p| !p.starts_with(vault_dir.path())),
        "no expansion entry escapes the vault"
    );

    // The base snapshot restores: table open again, row offset back.
    let (_f2, mut fresh) = app(&vault_dir);
    fresh.restore_session(&state, vault_dir.path());
    let base = fresh.base.as_ref().expect("base table reopened");
    assert_eq!(base.path, vault_dir.path().join("tbl.base"));
    assert_eq!(base.scroll, 2, "row offset restored");
    assert_eq!(fresh.current_path(), vault_dir.path().join("tbl.base"));
}

#[test]
fn no_vault_tab_is_a_noop() {
    let file = TempFile::new("solo.md", "x\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    app.handle_key(key(KeyCode::Tab));
    assert!(!app.sidebar_visible);
    assert!(app.sidebar.is_none());
}

#[test]
fn h_l_switch_focus() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    assert_eq!(app.focus, Focus::Content);
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, Focus::Sidebar);
    // Selection starts on the `sub` folder, where `l` expands instead of
    // switching focus — move onto a note first.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.focus, Focus::Content);

    // The arrows map 1:1 onto h/l (focus and sidebar fold semantics).
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.focus, Focus::Sidebar);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.focus, Focus::Content);

    // Left on a top-level note: only the focus moved back (the first
    // press); a second Left is a no-op — the note's parent IS the
    // vault root, so there is nothing to collapse into.
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.focus, Focus::Sidebar);
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("a.md"))
    );
    app.handle_key(key(KeyCode::Left));
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("a.md")),
        "top-level note: nothing to collapse"
    );
}

#[test]
fn sidebar_navigation_and_open_pushes_history() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('h'))); // focus sidebar
    assert_eq!(app.focus, Focus::Sidebar);

    // Selection starts on the `sub` folder: move to a.md, Enter opens it
    // and pushes history.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
    assert!(app.nav.can_go_back());

    // Backspace returns to the original file; Ctrl+O forward again.
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path());
    app.handle_key(ctrl(KeyCode::Char('o')));
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

#[test]
fn backspace_restores_scroll_position() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.viewer.viewport_height = 10;
    app.viewer.scroll_down(20);
    let target = vault_dir.path().join("a.md");
    app.open_note(&target).unwrap();
    assert_eq!(app.viewer.scroll, 0);
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path());
    assert_eq!(app.viewer.scroll, 20, "scroll position restored");
    // Forward again via Ctrl+O.
    app.handle_key(ctrl(KeyCode::Char('o')));
    assert_eq!(app.viewer.path, target);
    assert_eq!(app.viewer.scroll, 0);
}

#[test]
fn open_same_note_does_not_pollute_history() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.viewer.scroll_down(7);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    assert!(app.nav.can_go_back());
    // Re-opening the same note is a no-op: one Backspace returns to the
    // start file, and a second one has nowhere to go.
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path());
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path(), "history exhausted");
}

/// Regression test for nav-stack corruption: `Backspace` toward a
/// history target that was deleted since must surface the error and
/// leave both stacks intact (peek-then-commit, no phantom entries).
#[test]
fn back_to_deleted_file_keeps_history_intact() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    let a = vault_dir.path().join("a.md");
    app.open_note(&a).unwrap();
    assert!(app.nav.can_go_back());

    // The BACK target (the note we came from) vanishes before Backspace
    // is pressed.
    std::fs::remove_file(file.path()).unwrap();
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.viewer.last_error.is_some(), "back failure surfaced");
    assert_eq!(app.viewer.path, a, "current note unchanged on failure");
    assert!(
        app.nav.can_go_back(),
        "back stack entry must survive a failed open"
    );
    assert!(!app.nav.can_go_forward(), "no phantom forward entry");

    // Once the file is back, the same Backspace succeeds and history
    // round-trips as normal.
    file.rewrite(&"MAIN\n\n".repeat(50));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path());
    app.handle_key(ctrl(KeyCode::Char('o')));
    assert_eq!(app.viewer.path, a, "forward still works after recovery");
}

/// Mirror case for `Ctrl+O`: a deleted forward target must not corrupt
/// the stacks either.
#[test]
fn forward_to_deleted_file_keeps_history_intact() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    let a = vault_dir.path().join("a.md");
    app.open_note(&a).unwrap();
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.nav.can_go_forward());

    std::fs::remove_file(&a).unwrap();
    app.handle_key(ctrl(KeyCode::Char('o')));
    assert!(app.viewer.last_error.is_some(), "forward failure surfaced");
    assert_eq!(app.viewer.path, file.path(), "current note unchanged");
    assert!(
        app.nav.can_go_forward(),
        "forward stack entry must survive a failed open"
    );
    assert!(!app.nav.can_go_back(), "no phantom back entry");

    std::fs::write(&a, "A\n\n".repeat(50)).unwrap();
    app.handle_key(ctrl(KeyCode::Char('o')));
    assert_eq!(app.viewer.path, a, "forward succeeds after recovery");
}

#[test]
fn open_missing_note_surfaces_error_keeps_current() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    let before = app.viewer.lines.len();
    // Direct open: propagates the error, no history pollution.
    app.open_note(&vault_dir.path().join("nope.md"))
        .unwrap_err();
    assert_eq!(app.viewer.path, file.path());
    assert_eq!(app.viewer.lines.len(), before);
    assert!(!app.nav.can_go_back());
    // Via the sidebar (Enter): the error is surfaced in the status bar.
    app.sidebar.as_mut().unwrap().selected = Some(vault_dir.path().join("nope.md"));
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Enter));
    assert!(app.viewer.last_error.is_some(), "error surfaced");
    assert_eq!(app.viewer.path, file.path());
    assert_eq!(
        app.focus,
        Focus::Sidebar,
        "failed open keeps the sidebar focused"
    );
}

#[test]
fn help_swallows_next_key() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('?')));
    assert!(app.viewer.help_open);
    // `q` closes the help instead of quitting.
    assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Flow::Continue);
    assert!(!app.viewer.help_open);
}

#[test]
fn help_j_scrolls_and_other_keys_close_it() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('?')));
    assert!(app.viewer.help_open);
    assert_eq!(app.viewer.help_scroll, 0);
    // j scrolls the help (help stays open); the note does not scroll.
    app.handle_key(key(KeyCode::Char('j')));
    assert!(app.viewer.help_open, "j does not close help");
    assert_eq!(app.viewer.help_scroll, 1);
    assert_eq!(app.viewer.scroll, 0, "j does not scroll the note");
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.viewer.help_scroll, 0, "k scrolls back");
    // A non-scroll key still closes help and resets the scroll.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(!app.viewer.help_open, "q closes help instead of quitting");
    assert_eq!(app.viewer.help_scroll, 0);
}

#[test]
fn scroll_keys_are_focus_gated() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('h')));
    let sel = app.sidebar.as_ref().unwrap().selected.clone();
    // j in sidebar mode moves the selection, not the scroll.
    app.handle_key(key(KeyCode::Char('j')));
    assert_ne!(app.sidebar.as_ref().unwrap().selected, sel);
    assert_eq!(app.viewer.scroll, 0);
}

#[test]
fn sidebar_rescans_after_watch_reload() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    let before = app.sidebar.as_ref().unwrap().rows().len();
    std::fs::write(vault_dir.path().join("fresh.md"), "new\n").unwrap();
    file.rewrite("changed\n");
    app.poll();
    let after = app.sidebar.as_ref().unwrap().rows().len();
    assert_eq!(after, before + 1, "sidebar picked up the new file");
    // A poll with no reload must not rescan (selection preserved anyway).
    app.poll();
}

// --- M3: wikilink activation -------------------------------------

/// Vault with cross-links:
/// - start.md links [[a]] (unique), [[dup]] (ambiguous, hint → the
///   same-folder dup.md), [[missing]] (unresolved), [[#Section]]
///   (self-heading). Long enough to have scrollable room.
fn link_vault() -> TempDir {
    let dir = TempDir::new("app-links");
    std::fs::write(
        dir.path().join("start.md"),
                    format!(
            "# Start\n\nSee [[a]] and [[dup]] and [[missing]] and [[#Section]].\n\n## Section\n\n{}",
            "section body\n\n".repeat(30)
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a.md"),
        format!("# A\n\n{}", "a body\n\n".repeat(30)),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub").join("dup.md"),
        format!("# Dup\n\n{}", "dup body\n\n".repeat(30)),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("dup.md"),
        format!("# Dup root\n\n{}", "root dup body\n\n".repeat(30)),
    )
    .unwrap();
    dir
}

fn link_app(vault_dir: &TempDir) -> (TempFile, App) {
    let file = TempFile::new("app-link-main.md", &"MAIN\n\n".repeat(50));
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let index = shared_index(vault_dir);
    let app = App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap();
    (file, app)
}

#[test]
fn resolved_wikilink_activates_with_history() {
    let vault_dir = link_vault();
    let (_file, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    let idx = app
        .viewer
        .links
        .iter()
        .position(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "a"))
        .expect("a link rendered");
    app.viewer.set_focus_link(Some(idx));
    app.activate_focused_link();
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
    assert!(app.nav.can_go_back());
    // Backspace returns to the note we came from (start.md).
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, vault_dir.path().join("start.md"));
}

#[test]
fn ambiguous_wikilink_gets_same_folder_hint() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    // Open start.md so the hint uses its folder (the vault root):
    // [[dup]] is ambiguous between dup.md and sub/dup.md; the
    // same-folder candidate (dup.md, root) must win.
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    let dup_link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "dup"))
        .expect("dup link rendered");
    assert_eq!(
        dup_link.wikilink.as_ref().unwrap().state,
        LinkState::Resolved(vault_dir.path().join("dup.md")),
        "same-folder hint resolves the ambiguity"
    );
}

#[test]
fn unresolved_wikilink_surfaces_error() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    let idx = app
        .viewer
        .links
        .iter()
        .position(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "missing"))
        .expect("missing link rendered");
    app.viewer.set_focus_link(Some(idx));
    app.activate_focused_link();
    assert!(app.viewer.last_error.is_some(), "unresolved link surfaced");
    assert_eq!(app.viewer.path, vault_dir.path().join("start.md"));
}

#[test]
fn self_heading_link_opens_scrolled_to_section() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    let idx = app
        .viewer
        .links
        .iter()
        .position(|l| {
            l.wikilink
                .as_ref()
                .is_some_and(|w| w.target.is_empty() && w.heading.is_some())
        })
        .expect("self link rendered");
    app.viewer.set_focus_link(Some(idx));
    app.activate_focused_link();
    // Same note: no history push; scrolled to the `## Section` heading.
    assert_eq!(app.viewer.path, vault_dir.path().join("start.md"));
    assert!(app.viewer.scroll > 0, "scrolled to the section heading");
}

#[test]
fn hintless_ambiguity_opens_disambiguation_popup() {
    // A source at the vault root linking [[dup]] where both candidates
    // live in subfolders: the same-folder/nearest-ancestor hint finds
    // nothing, so activation must open the popup instead of guessing.
    let vault_dir = TempDir::new("app-popup");
    let _ = std::fs::create_dir_all(vault_dir.path().join("x"));
    let _ = std::fs::create_dir_all(vault_dir.path().join("y"));
    std::fs::write(vault_dir.path().join("x/dup.md"), "x\n").unwrap();
    std::fs::write(vault_dir.path().join("y/dup.md"), "y\n").unwrap();
    std::fs::write(vault_dir.path().join("far.md"), "See [[dup]]\n").unwrap();

    let file = TempFile::new("app-popup-main.md", "MAIN\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let index = shared_index(&vault_dir);
    let mut app = App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap();
    app.open_note(&vault_dir.path().join("far.md")).unwrap();
    let idx = app
        .viewer
        .links
        .iter()
        .position(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "dup"))
        .expect("dup link rendered");
    app.viewer.set_focus_link(Some(idx));
    app.activate_focused_link();
    let popup = app.disambiguation.as_ref().expect("popup open");
    assert_eq!(popup.candidates.len(), 2, "far.md gets no folder hint");

    // Enter opens the highlighted candidate.
    app.handle_key(key(KeyCode::Enter));
    assert!(app.disambiguation.is_none(), "popup closed after open");
    assert!(
        app.viewer.path.ends_with("dup.md"),
        "a candidate note opened: {:?}",
        app.viewer.path
    );
}

#[test]
fn popup_keys_navigate_and_cancel() {
    let vault_dir = link_vault();
    let (file, mut app) = link_app(&vault_dir);
    app.disambiguation = Some(Disambiguation {
        candidates: vec![
            vault_dir.path().join("dup.md"),
            vault_dir.path().join("sub/dup.md"),
        ],
        selected: 0,
        heading: None,
    });
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.disambiguation.as_ref().unwrap().selected, 1);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.disambiguation.as_ref().unwrap().selected, 0);
    // Any other key cancels without opening.
    app.handle_key(key(KeyCode::Char('z')));
    assert!(app.disambiguation.is_none());
    assert_eq!(app.viewer.path, file.path(), "current note unchanged");
}

/// Ctrl+C must quit even while the disambiguation popup is open
/// (previously it landed in the catch-all cancel arm).
#[test]
fn ctrl_c_quits_while_popup_open() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.disambiguation = Some(Disambiguation {
        candidates: vec![vault_dir.path().join("dup.md")],
        selected: 0,
        heading: None,
    });
    assert_eq!(app.handle_key(ctrl(KeyCode::Char('c'))), Flow::Quit);
}

/// Clicks are gated on the popup: a click on a link behind it must be
/// ignored, not activate that link and silently overwrite the popup
/// (losing the selection).
#[test]
fn clicks_are_ignored_while_popup_open() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    app.layout.note_viewport = Rect::new(0, 0, 40, 20);
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "dup"))
        .expect("dup link rendered")
        .clone();
    let region = link.regions[0];
    app.disambiguation = Some(Disambiguation {
        candidates: vec![
            vault_dir.path().join("dup.md"),
            vault_dir.path().join("sub/dup.md"),
        ],
        selected: 1,
        heading: None,
    });
    app.handle_click(region.start as u16, region.line as u16);
    let popup = app.disambiguation.as_ref().expect("popup not overwritten");
    assert_eq!(popup.selected, 1, "popup selection preserved");
    assert_eq!(popup.candidates.len(), 2);
    assert_eq!(
        app.viewer.path,
        vault_dir.path().join("start.md"),
        "no navigation happened behind the popup"
    );
}

#[test]
fn click_on_link_region_activates_it() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    // Fake the geometry ui::draw would set: viewport at (0,0), 40 cols.
    app.layout.note_viewport = Rect::new(0, 0, 40, 20);
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "a"))
        .expect("a link rendered")
        .clone();
    let region = link.regions[0];
    app.handle_click(region.start as u16, region.line as u16);
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

/// The link hit-test under the geometry the frame actually records: a
/// vertical margin insets the text area by rows, and a click on the link's
/// line inside that inset still activates it. The tests above fake the
/// viewport and so never see the margin.
#[test]
fn click_on_link_region_activates_it_under_the_real_margin_geometry() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    app.note_margin_y = 2;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let vp = app.layout.note_viewport;
    assert_eq!(vp.y, 2, "the draw recorded the vertical margin inset");
    assert_eq!(app.viewer.scroll, 0, "the note opens at the top");
    // Regions are taken after the draw: the frame re-wraps the note to the
    // real text width, which is what the click arithmetic must agree with.
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "a"))
        .expect("a link rendered")
        .clone();
    let region = link.regions[0];
    // The cells the click lands on are the link's own cells, painted (the
    // engine's `#` link marker plus the target) — not a blank margin row.
    let row = vp.y + region.line as u16;
    let painted: String = (vp.x + region.start as u16..vp.x + region.end as u16)
        .map(|x| term.backend().buffer()[(x, row)].symbol().to_string())
        .collect();
    assert_eq!(painted, "#a", "the link is painted where the click lands");
    app.handle_click(vp.x + region.start as u16, row);
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

/// The scrollbar column sits just past the note text rect. A link region
/// reaching that column must not be activated by a click on it (the
/// bounds check, not the regions, is what keeps bar clicks out), while
/// the last text column still activates it.
#[test]
fn click_on_scrollbar_column_never_reaches_link_hit_test() {
    let vault_dir = link_vault();
    let (_f, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    // Text rect is 0..40; the bar column is x == 40.
    app.layout.note_viewport = Rect::new(0, 0, 40, 20);
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "a"))
        .expect("a link rendered")
        .clone();
    // Stretch the link's region out to the bar column on its line.
    let line = link.regions[0].line;
    app.viewer.links.iter_mut().for_each(|l| {
        if l.wikilink.as_ref().is_some_and(|w| w.target == "a") {
            l.regions = vec![crate::links::LinkRegion {
                line,
                start: 0,
                end: 41,
            }];
        }
    });
    let before = app.viewer.path.clone();

    // The bar column (x == 40): no-op, no navigation.
    app.handle_click(40, line as u16);
    assert_eq!(
        app.viewer.path, before,
        "bar-column click must not activate"
    );

    // The last text column (x == 39) still activates the region.
    app.handle_click(39, line as u16);
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

// --- mouse: sidebar clicks and position-routed wheel ----------------

#[test]
fn sidebar_click_expands_collapses_and_opens_notes() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    // Collapsed rows: 0 = `sub` folder, 1 = a.md; inner rows start at
    // screen row 1 (below the top border).
    let sub_row = 1;

    // Folder click: selects + expands, opens nothing, focus unchanged.
    app.handle_click(3, sub_row);
    assert!(app
        .sidebar
        .as_ref()
        .unwrap()
        .expanded
        .contains(&vault_dir.path().join("sub")));
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("sub"))
    );
    assert_eq!(app.viewer.path, file.path(), "folder click opens nothing");
    assert_eq!(
        app.focus,
        Focus::Sidebar,
        "sidebar click focuses the sidebar"
    );

    // Expanded rows: [sub, b.md, c.md, a.md] — click row 2 (b.md).
    app.handle_click(3, 2);
    assert_eq!(app.viewer.path, vault_dir.path().join("sub/b.md"));
    assert!(app.nav.can_go_back(), "opened through the normal open path");
    assert_eq!(app.viewer.scroll, 0, "open resets the scroll");
    assert_eq!(
        app.focus,
        Focus::Content,
        "note click moves focus to the content pane"
    );

    // Clicking the folder again collapses it.
    app.handle_click(3, sub_row);
    assert!(!app
        .sidebar
        .as_ref()
        .unwrap()
        .expanded
        .contains(&vault_dir.path().join("sub")));
}

#[test]
fn click_sidebar_note_opens_and_moves_focus_to_content() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.layout.note_viewport = Rect::new(30, 0, 70, 20);
    // Start on the sidebar so the click must move the focus.
    app.focus = Focus::Sidebar;

    // Row 2 (inner row 1) is `a.md` in the collapsed list [sub, a.md].
    app.handle_click(3, 2);
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
    assert_eq!(
        app.focus,
        Focus::Content,
        "opening a note moves focus to the content pane"
    );

    // Focus now routes j/k to the content scroll, not the sidebar
    // selection the click came from.
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.viewer.scroll, 1, "j scrolled the content");
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("a.md")),
        "sidebar selection untouched"
    );
}

#[test]
fn enter_on_sidebar_note_moves_focus_to_content() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('h'))); // focus the sidebar
    assert_eq!(app.focus, Focus::Sidebar);

    // Enter on the selected folder: expands in place, sidebar keeps
    // the focus (j/k must keep moving the tree selection).
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.focus,
        Focus::Sidebar,
        "folder Enter keeps the sidebar focused"
    );

    // Enter on a note: opens it and moves focus to the content pane,
    // so the next j/k/PgDn/wheel scrolls the note just opened.
    app.handle_key(key(KeyCode::Char('j'))); // onto sub/b.md
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.viewer.path, vault_dir.path().join("sub/b.md"));
    assert_eq!(
        app.focus,
        Focus::Content,
        "note Enter moves focus to the content pane"
    );
}

#[test]
fn click_content_line_moves_focus_to_content_then_j_scrolls() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.layout.note_viewport = Rect::new(30, 0, 70, 20);
    // Start with focus on the sidebar so the click must change it.
    app.focus = Focus::Sidebar;

    // Plain text line in the content pane (no link/checkbox at col 40).
    app.handle_click(40, 5);
    assert_eq!(app.focus, Focus::Content, "content click focuses content");

    let scroll_before = app.viewer.scroll;
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.viewer.scroll, scroll_before + 1, "j scrolls content");
}

#[test]
fn click_base_table_moves_focus_to_content() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.layout.note_viewport = Rect::new(30, 0, 70, 20);
    std::fs::write(
        vault_dir.path().join("t.base"),
        "views:\n  - type: table\n    name: all\n",
    )
    .unwrap();
    app.open_base(&vault_dir.path().join("t.base")).unwrap();
    app.focus = Focus::Sidebar;
    app.handle_click(40, 5);
    assert_eq!(
        app.focus,
        Focus::Content,
        "base-table click focuses content"
    );
}

#[test]
fn click_backlinks_panel_moves_focus_to_backlinks() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.layout.note_viewport = Rect::new(30, 0, 70, 15);
    app.backlinks_open = true;
    app.layout.backlinks_area = Some(Rect::new(30, 15, 70, 5));
    assert_eq!(app.focus, Focus::Content);

    app.handle_click(40, 17);
    assert_eq!(app.focus, Focus::Backlinks, "backlinks click focuses panel");
}

#[test]
fn click_backlinks_panel_moves_focus_after_real_draw() {
    // Render-driven (no hand-set geometry): `ui::draw` must record the
    // panel rect, otherwise a click in the panel never focuses it.
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.backlinks_open = true;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let panel = app
        .layout
        .backlinks_area
        .expect("draw records the backlinks panel");
    assert_eq!(app.focus, Focus::Content);

    app.handle_click(panel.x + 1, panel.y + 1);
    assert_eq!(
        app.focus,
        Focus::Backlinks,
        "click inside the drawn panel focuses it"
    );

    // Closing the panel clears the rect so the old area cannot route clicks.
    app.focus = Focus::Content;
    app.backlinks_open = false;
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    assert!(app.layout.backlinks_area.is_none());
}

/// Render-driven: a 12-linker backlinks panel overflows its 5 inner rows,
/// so the scrollbar takes the panel's last inner column.
fn overflowing_backlinks_app(vault_dir: &TempDir) -> App {
    for i in 0..12 {
        std::fs::write(vault_dir.path().join(format!("bl{i:02}.md")), "[[a]]\n").unwrap();
    }
    let (_f, mut app) = app(vault_dir);
    app.sidebar_visible = false;
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.backlinks_open = true;
    app.refresh_backlinks();
    app
}

#[test]
fn click_on_backlinks_scrollbar_column_is_bounded_out() {
    let vault_dir = vault();
    let mut app = overflowing_backlinks_app(&vault_dir);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    // The bar is the panel's last inner column (one left of the border);
    // the hit rect stops right before it.
    let bar_col = hit.right();
    assert_eq!(bar_col, 98, "bar column is inside the panel, border at 99");
    let buffer = term.backend().buffer();
    assert!(
        (hit.y + 1..hit.bottom() - 1).any(|y| matches!(buffer[(bar_col, y)].symbol(), "│" | "█")),
        "precondition: the bar is drawn on the click column"
    );

    app.focus = Focus::Content;
    let sel_before = app.backlink_sel;
    app.handle_click(bar_col, hit.y + 2);
    assert_eq!(
        app.focus,
        Focus::Content,
        "bar click does not focus the panel"
    );
    assert_eq!(
        app.backlink_sel, sel_before,
        "bar click does not change selection"
    );
}

/// SPEC: "click/Enter opens" a backlink. A click on a listed row
/// selects it and opens it (focus moves to the content pane); a click
/// on the panel's border rows keeps the focus-only behavior.
#[test]
fn click_on_visible_backlink_row_opens_it_with_bar_present() {
    let vault_dir = vault();
    let mut app = overflowing_backlinks_app(&vault_dir);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    // First visible row (inner row 1 = bl00): click opens it.
    app.handle_click(hit.x + 1, hit.y + 1);
    assert_eq!(app.viewer.path, vault_dir.path().join("bl00.md"));
    assert_eq!(
        app.focus,
        Focus::Content,
        "opening a backlink moves focus to the content pane"
    );

    // Fresh panel: the last visible row (viewport 5 → bl04) opens too.
    let mut app = overflowing_backlinks_app(&vault_dir);
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    app.handle_click(hit.x + 1, hit.bottom() - 2);
    assert_eq!(app.viewer.path, vault_dir.path().join("bl04.md"));

    // The panel's own border rows keep the focus-only behavior.
    let mut app = overflowing_backlinks_app(&vault_dir);
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    app.focus = Focus::Content;
    app.handle_click(hit.x + 1, hit.y);
    assert_eq!(app.focus, Focus::Backlinks, "title border still routes");
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"), "no open");
}

/// Clicking below the last listed row (empty space in the panel)
/// focuses the panel without opening anything.
#[test]
fn click_on_backlinks_empty_space_is_focus_only() {
    let vault_dir = vault();
    // Only two backlinks: the 5-row panel has empty space below them.
    for name in ["bl-a.md", "bl-b.md"] {
        std::fs::write(vault_dir.path().join(name), "[[a]]\n").unwrap();
    }
    let (_f, mut app) = app(&vault_dir);
    app.sidebar_visible = false;
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.backlinks_open = true;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    // Inner rows 3..5 are below the two listed rows.
    app.handle_click(hit.x + 1, hit.y + 4);
    assert_eq!(app.focus, Focus::Backlinks, "empty space focuses the panel");
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"), "no open");
    assert_eq!(app.backlink_sel, 0, "selection untouched");
}

/// The row mapping goes through the panel's scroll offset: with
/// `backlink_scroll = 2`, the first inner row is the third entry.
#[test]
fn click_on_backlinks_row_maps_through_scroll() {
    let vault_dir = vault();
    let mut app = overflowing_backlinks_app(&vault_dir);
    // Selection inside the scrolled window (the draw keeps the
    // selection visible, so the scroll survives only with sel >= 2).
    app.backlink_sel = 2;
    app.backlink_scroll = 2;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let hit = app.layout.backlinks_area.expect("panel drawn");
    app.handle_click(hit.x + 1, hit.y + 1);
    assert_eq!(
        app.viewer.path,
        vault_dir.path().join("bl02.md"),
        "scroll offset shifts which row the click opens"
    );
}

#[test]
fn click_while_popup_open_does_not_move_focus() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.focus = Focus::Content;
    app.viewer.help_open = true;
    app.handle_click(3, 2);
    assert_eq!(app.focus, Focus::Content, "modal popup ignores the click");
}

#[test]
fn sidebar_click_maps_screen_row_through_scroll() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    // Collapsed rows [sub, a.md]; scroll 1 hides `sub`, so the first
    // inner screen row is a.md.
    let sidebar = app.sidebar.as_mut().unwrap();
    sidebar.viewport = 5;
    sidebar.scroll = 1;
    app.handle_click(3, 1);
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

#[test]
fn sidebar_border_and_past_end_clicks_are_noops() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    // Top border row and a row past the short list: neither selects
    // nor opens anything.
    app.handle_click(3, 0);
    app.handle_click(3, 5);
    assert!(app.sidebar.as_ref().unwrap().expanded.is_empty());
    assert_eq!(app.viewer.path, file.path());
    assert!(app.sidebar.as_ref().unwrap().selected.is_some());
}

#[test]
fn sidebar_click_does_not_fall_through_to_content_links() {
    let vault_dir = link_vault();
    let (_file, mut app) = link_app(&vault_dir);
    app.open_note(&vault_dir.path().join("start.md")).unwrap();
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.layout.note_viewport = Rect::new(30, 0, 70, 20);
    // Scroll 1 puts the `[[a]]` link (rendered line 2) at screen row
    // 1 — the same row as the sidebar's first inner row (`sub`).
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "a"))
        .expect("a link rendered")
        .clone();
    assert_eq!(link.regions[0].line, 2);
    app.viewer.set_scroll(1);
    app.handle_click(5, 1);
    assert_eq!(
        app.viewer.path,
        vault_dir.path().join("start.md"),
        "the content link was not activated"
    );
    assert!(
        app.sidebar
            .as_ref()
            .unwrap()
            .expanded
            .contains(&vault_dir.path().join("sub")),
        "the sidebar handled the click instead"
    );
}

#[test]
fn sidebar_clicks_are_gated_by_popups() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    let sub = vault_dir.path().join("sub");

    // Help popup: modal, clicks ignored entirely.
    app.viewer.help_open = true;
    app.handle_click(3, 1);
    assert!(app.viewer.help_open);
    assert!(!app.sidebar.as_ref().unwrap().expanded.contains(&sub));
    assert_eq!(app.viewer.path, file.path());
    app.viewer.help_open = false;

    // Search popup: same gating.
    app.search.open = true;
    app.handle_click(3, 1);
    assert!(app.search.open);
    assert!(!app.sidebar.as_ref().unwrap().expanded.contains(&sub));
    app.search.open = false;

    // Disambiguation popup: click ignored, popup intact.
    app.disambiguation = Some(Disambiguation {
        candidates: vec![vault_dir.path().join("a.md")],
        selected: 0,
        heading: None,
    });
    app.handle_click(3, 1);
    assert_eq!(app.disambiguation.as_ref().unwrap().selected, 0);
    assert!(!app.sidebar.as_ref().unwrap().expanded.contains(&sub));
}

#[test]
fn sidebar_click_works_in_tags_mode() {
    let vault_dir = vault();
    std::fs::write(
        vault_dir.path().join("a.md"),
        "---\nstatus: active\ntags: [mocktag]\n---\nA\n",
    )
    .unwrap();
    let (file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('t'))); // sidebar → tags mode
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    // Rows: 0 = the mocktag tag (collapsed), then its note when expanded.
    app.handle_click(3, 1); // tag row: expand
    assert!(app.tags.expanded.contains("mocktag"));
    assert_eq!(app.viewer.path, file.path(), "tag click opens nothing");
    app.handle_click(3, 2); // note row: open
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
    assert_eq!(app.focus, Focus::Content, "note click focuses content");
}

#[test]
fn wheel_over_sidebar_scrolls_sidebar_even_when_content_has_focus() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    assert_eq!(app.focus, Focus::Content);
    // Wheel down over the sidebar column: the sidebar list scrolls
    // (selection moves, clamped at the last row), content untouched.
    app.handle_wheel_at(5, 10, true);
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("a.md")),
        "selection moved down 3 (clamped)"
    );
    assert_eq!(app.viewer.scroll, 0, "content did not scroll");
    app.handle_wheel_at(5, 10, false);
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("sub")),
        "wheel up moves back up"
    );
}

#[test]
fn wheel_outside_sidebar_keeps_focus_routing() {
    let vault_dir = vault();
    let (_file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.handle_wheel_at(50, 10, true);
    assert_eq!(app.viewer.scroll, 3, "content (focused) scrolled");
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("sub")),
        "sidebar selection untouched"
    );
    app.handle_wheel_at(50, 10, false);
    assert_eq!(app.viewer.scroll, 0);
}

#[test]
fn wheel_is_gated_by_the_search_popup() {
    let vault_dir = vault();
    let (file, mut app) = app(&vault_dir);
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));
    app.search.open = true;
    app.handle_wheel_at(5, 10, true);
    app.handle_wheel_at(50, 10, true);
    assert_eq!(app.viewer.scroll, 0);
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        Some(vault_dir.path().join("sub"))
    );
    assert_eq!(app.viewer.path, file.path());
}

// --- M3: backlinks panel -----------------------------------------

#[test]
fn backlinks_panel_lists_and_opens() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    // a.md does not exist yet as a backlink source; open a.md, then
    // make b.md link to it.
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    std::fs::write(
        vault_dir.path().join("sub").join("b.md"),
        "[[a]]\n\n".repeat(3),
    )
    .unwrap();
    app.poll();
    app.handle_key(key(KeyCode::Char('b')));
    assert!(app.backlinks_open);
    assert_eq!(app.focus, Focus::Backlinks);
    assert_eq!(
        app.backlinks,
        vec![vault_dir.path().join("sub").join("b.md")]
    );
    // Enter opens the backlink and moves focus to the content pane
    // (consistent with the sidebar open paths).
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.viewer.path, vault_dir.path().join("sub").join("b.md"));
    assert!(app.nav.can_go_back());
    assert_eq!(
        app.focus,
        Focus::Content,
        "backlink Enter moves focus to the content pane"
    );

    // b closes again; focus snaps out of the panel.
    app.handle_key(key(KeyCode::Char('b')));
    assert!(!app.backlinks_open);
    assert_eq!(app.focus, Focus::Content);
}

#[test]
fn backlinks_open_from_sidebar_focus_takes_focus() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    // Two backlinks to a.md so `j` has somewhere to go.
    std::fs::write(vault_dir.path().join("sub").join("b.md"), "[[a]]\n").unwrap();
    std::fs::write(vault_dir.path().join("sub").join("c.md"), "[[a]]\n").unwrap();
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.poll();
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 20));

    // Click the sidebar row for `sub` (row 1, collapsed) → focus Sidebar.
    app.handle_click(3, 1);
    assert_eq!(app.focus, Focus::Sidebar);

    // `b` opens the panel and must take focus even from the sidebar.
    app.handle_key(key(KeyCode::Char('b')));
    assert!(app.backlinks_open);
    assert_eq!(app.focus, Focus::Backlinks);
    assert_eq!(app.backlinks.len(), 2);
    assert_eq!(app.backlink_sel, 0);

    // `j` moves the backlink selection, not the sidebar selection.
    let sidebar_sel = app.sidebar.as_ref().unwrap().selected.clone();
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.backlink_sel, 1, "j moves backlink selection");
    assert_eq!(
        app.sidebar.as_ref().unwrap().selected,
        sidebar_sel,
        "j leaves the sidebar selection alone"
    );
}

#[test]
fn backlinks_empty_state_and_selection_clamp() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(key(KeyCode::Char('b')));
    assert!(app.backlinks.is_empty(), "nothing links to a.md yet");
    app.backlink_move(5); // must not panic
    assert_eq!(app.backlink_sel, 0);
    app.handle_key(key(KeyCode::Enter)); // no selection: no-op
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
}

// --- M3: tag view ------------------------------------------------

#[test]
fn tag_view_toggles_and_opens_note() {
    let vault_dir = vault();
    std::fs::write(
        vault_dir.path().join("a.md"),
        "---\nstatus: active\ntags: [mocktag]\n---\nA\n",
    )
    .unwrap();
    let (_f, mut app) = app(&vault_dir);
    app.poll(); // watcher picks up the retagged a.md, tags refresh
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tags);
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tree);

    // In tag mode: focus the sidebar, expand the tag, move to its
    // note, open it.
    app.handle_key(key(KeyCode::Char('t')));
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Enter)); // expand mocktag
    app.handle_key(key(KeyCode::Char('j'))); // onto a.md
    app.handle_key(key(KeyCode::Enter)); // open
    assert_eq!(app.viewer.path, vault_dir.path().join("a.md"));
    assert!(app.nav.can_go_back());
    assert_eq!(
        app.focus,
        Focus::Content,
        "tag-mode Enter moves focus to the content pane"
    );
}

#[test]
fn tag_view_without_vault_is_noop() {
    let file = TempFile::new("solo-tags.md", "x\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tree);
}

// --- unit 11: the outline ----------------------------------------

/// A vault whose `a.md` has two headings (`# One`, `## Two`) separated by
/// enough body to make the second heading a real scroll target; `b.md` has
/// one heading and `c.md` none.
fn outline_vault() -> TempDir {
    let dir = TempDir::new("app-outline");
    let mut a = String::from("# One\n\n");
    for i in 0..40 {
        a.push_str(&format!("line {i}\n\n"));
    }
    a.push_str("## Two\n\n");
    for i in 0..40 {
        a.push_str(&format!("tail {i}\n\n"));
    }
    std::fs::write(dir.path().join("a.md"), a).unwrap();
    std::fs::write(dir.path().join("b.md"), "# Bee\n\nbody\n").unwrap();
    std::fs::write(dir.path().join("c.md"), "plain body, no headings\n").unwrap();
    dir
}

#[test]
fn o_toggles_the_outline_and_returns_to_the_last_view() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();

    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Outline);
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tree, "back to the tree");

    // Over the tag view: `o` returns there, not to the tree.
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tags);
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Outline);
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tags);

    // `t` still toggles the tree ⇄ tags pair; from the outline it leaves
    // to the other member of the pair.
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('t')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tree);
}

#[test]
fn outline_without_vault_is_noop() {
    let file = TempFile::new("solo-outline.md", "x\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Tree);
}

#[test]
fn outline_enter_jumps_to_the_heading_and_focuses_content() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    // A small viewport keeps the second heading inside the scroll range.
    app.viewer.on_viewport_height(10);

    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, Focus::Sidebar, "h focuses the sidebar");
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.outline.selected, 1, "j moved to the second heading");
    let line = app.viewer.toc[1].line as u32;
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.viewer.scroll, line, "the note scrolled to the heading");
    assert_eq!(
        app.focus,
        Focus::Content,
        "Enter hands over the content pane"
    );
}

#[test]
fn outline_click_jumps_to_the_clicked_heading() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.viewer.on_viewport_height(10);
    app.handle_key(key(KeyCode::Char('o')));
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 10));

    app.handle_click(5, 2); // the second inner row: `## Two`
    assert_eq!(app.outline.selected, 1);
    assert_eq!(app.viewer.scroll, app.viewer.toc[1].line as u32);
    assert_eq!(app.focus, Focus::Content, "a row click jumps like Enter");
}

#[test]
fn outline_l_moves_to_content_and_h_does_nothing() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(key(KeyCode::Char('o')));
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, Focus::Sidebar);
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, Focus::Sidebar, "h has no hierarchy to collapse");
    assert_eq!(app.sidebar_mode, SidebarMode::Outline);
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.focus, Focus::Content, "l moves to the content pane");
}

#[test]
fn outline_follows_the_open_note_and_clamps_the_selection() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(key(KeyCode::Char('o')));
    app.outline.select(1, app.viewer.toc.len());
    assert_eq!(app.outline.selected, 1);

    // b.md has one heading: the cursor must stay valid.
    app.open_note(&vault_dir.path().join("b.md")).unwrap();
    assert_eq!(app.viewer.toc.len(), 1);
    assert_eq!(app.outline.selected, 0, "selection clamped to the new list");
    assert_eq!(app.sidebar_mode, SidebarMode::Outline, "the view stays on");
}

#[test]
fn outline_with_no_headings_has_an_empty_selection() {
    let vault_dir = outline_vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("c.md")).unwrap();
    assert!(app.viewer.toc.is_empty(), "precondition: no headings");
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.sidebar_mode, SidebarMode::Outline);
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.focus, Focus::Sidebar);
    app.handle_key(key(KeyCode::Char('j'))); // must not panic
    app.handle_key(key(KeyCode::Enter)); // nothing to jump to
    assert_eq!(app.focus, Focus::Sidebar, "Enter stays on an empty list");
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.focus, Focus::Content, "l still hands over the content");
}

#[test]
fn outline_has_no_rows_while_a_base_table_is_open() {
    let vault_dir = outline_vault();
    std::fs::write(
        vault_dir.path().join("tbl.base"),
        "views:\n  - type: table\n    name: Solo\n    order:\n      - file.name\n",
    )
    .unwrap();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.outline_len(), 2, "the note's headings are the rows");

    app.open_note(&vault_dir.path().join("tbl.base")).unwrap();
    assert!(app.base.is_some());
    assert_eq!(app.outline_len(), 0, "a .base table has no outline rows");

    // Click routing takes its row count from the same helper: with no rows,
    // a click where a heading row used to be maps to nothing, so the
    // selection stays on the first heading. The jump's own guard would keep
    // the scroll put either way, so the selection is what pins the mapping.
    assert_eq!(
        app.outline.selected, 0,
        "precondition: the selection cursor is at 0"
    );
    app.viewer.scroll = 5;
    app.layout.sidebar_area = Some(Rect::new(0, 0, 30, 10));
    app.handle_click(5, 2);
    assert_eq!(app.outline.selected, 0, "the click mapped to no row");
    assert_eq!(app.viewer.scroll, 5, "no jump into the hidden note");

    // Back to a note: its rows return.
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    assert_eq!(app.outline_len(), 2);
}

#[test]
fn session_saved_in_the_outline_restores_the_view_behind_it() {
    let vault_dir = outline_vault();
    let (_f, mut shaped) = app(&vault_dir);
    shaped.open_note(&vault_dir.path().join("a.md")).unwrap();

    // Saved over the tree: the file names the tree, not the overlay. The
    // save side is what pins this half — `Tree` is a fresh app's default and
    // `restore_session` has no "tree" arm, so a restore assert could not
    // fail. The tags half below round-trips for real.
    shaped.handle_key(key(KeyCode::Char('o')));
    assert_eq!(shaped.sidebar_mode, SidebarMode::Outline);
    let over_tree = shaped.session_state(vault_dir.path());
    assert_eq!(
        over_tree.mode.as_deref(),
        Some("tree"),
        "the outline is never persisted"
    );

    // Saved over the tags: that view round-trips, and `o` still returns
    // to it from the restored outline.
    shaped.handle_key(key(KeyCode::Char('o')));
    shaped.handle_key(key(KeyCode::Char('t')));
    assert_eq!(shaped.sidebar_mode, SidebarMode::Tags);
    shaped.handle_key(key(KeyCode::Char('o')));
    assert_eq!(shaped.sidebar_mode, SidebarMode::Outline);
    let over_tags = shaped.session_state(vault_dir.path());
    assert_eq!(over_tags.mode.as_deref(), Some("tags"));
    let (_f3, mut fresh) = app(&vault_dir);
    fresh.restore_session(&over_tags, vault_dir.path());
    assert_eq!(
        fresh.sidebar_mode,
        SidebarMode::Tags,
        "restored to the tags"
    );
    fresh.handle_key(key(KeyCode::Char('o')));
    assert_eq!(fresh.sidebar_mode, SidebarMode::Outline);
    fresh.handle_key(key(KeyCode::Char('o')));
    assert_eq!(
        fresh.sidebar_mode,
        SidebarMode::Tags,
        "the tags are behind it"
    );
}

// --- M3: status glyphs -------------------------------------------

#[test]
fn vault_watcher_reindexes_and_reparses() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    // Change a.md's status via the index path the watcher would use.
    std::fs::write(
        vault_dir.path().join("a.md"),
        "---\nstatus: archived\n---\nA\n\n".repeat(3),
    )
    .unwrap();
    app.viewer.last_reload = None; // force a poll delta
    app.poll();
    // The status clock is HH:MM, so two reloads inside one minute look
    // identical through the pty: only this field can prove the render
    // time follows a watch reload.
    let reloaded_at = app.viewer.last_reload;
    assert!(
        reloaded_at.is_some(),
        "the watch reload (not the test) refreshed the render time"
    );
    let index = app.index.as_ref().unwrap().borrow();
    assert_eq!(
        index
            .note(&vault_dir.path().join("a.md"))
            .unwrap()
            .status
            .as_deref(),
        Some("archived"),
        "watch path reindexed the changed file"
    );
}

/// A directory that becomes unreadable after startup: the watch walk
/// reports it, and the app puts the warning in its own status-bar slot (the
/// startup index keeps the same warning to itself; mid-session the bar is
/// the channel the user can see). It is state: it stays while the
/// condition lasts and clears on the walk that finds the directory readable
/// again — no unrelated action needed.
#[test]
fn a_directory_locked_after_startup_surfaces_a_warning_that_clears() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    let sub = vault_dir.path().join("sub");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    app.poll();
    let message = app
        .walk_warning
        .clone()
        .unwrap_or_else(|| panic!("the unreadable directory is silent"));
    assert!(
        message.contains("sub"),
        "names the directory, vault-relative: {message:?}"
    );
    assert!(
        message.contains("unreadable"),
        "says what happened: {message:?}"
    );
    assert!(
        app.viewer.last_error.is_none(),
        "the walk warning has its own slot, not the error one: {:?}",
        app.viewer.last_error
    );

    // Still unreadable: the next scan keeps the same state (no rewrite
    // into a new event).
    app.last_vault_scan = None; // test clock: force the 1 s scan
    app.poll();
    assert_eq!(app.walk_warning.as_deref(), Some(message.as_str()));

    // Readable again: the warning clears itself on the next walk.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    app.last_vault_scan = None; // test clock: force the 1 s scan
    app.poll();
    assert!(
        app.walk_warning.is_none(),
        "a readable vault clears the warning without an unrelated action: {:?}",
        app.walk_warning
    );
}

/// A real error the user still needs survives the background walk warning:
/// the warning lives in its own slot, so the scan tick cannot replace the
/// error with a stale "unreadable" notice.
#[test]
fn a_walk_warning_does_not_replace_a_real_error() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.viewer.last_error = Some("not writing: changed on disk".to_string());
    let sub = vault_dir.path().join("sub");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    app.poll();
    assert!(
        app.walk_warning
            .as_deref()
            .is_some_and(|w| w.contains("unreadable")),
        "the condition is still recorded: {:?}",
        app.walk_warning
    );
    assert_eq!(
        app.viewer.last_error.as_deref(),
        Some("not writing: changed on disk"),
        "the background warning left the newer error alone"
    );
    // Restore so the fixture's TempDir can clean up.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// --- M4: global search --------------------------------------------

fn search_vault() -> TempDir {
    let dir = TempDir::new("app-search");
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub").join("hay.md"),
        "top\nneedle here\nfiller\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("other.md"), "no match\n").unwrap();
    dir
}

#[test]
fn s_opens_prompt_typing_runs_and_results_navigate() {
    let vault_dir = search_vault();
    let (file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('s')));
    assert!(app.search.open);
    assert_eq!(app.search.mode, crate::search::SearchMode::Input);

    // Modal: typed characters build the query (a typed `q` must NOT
    // quit the app).
    for c in "needle".chars() {
        assert_eq!(app.handle_key(key(KeyCode::Char(c))), Flow::Continue);
    }
    assert_eq!(app.search.query, "needle");
    assert_eq!(app.viewer.path, file.path(), "still open");

    // Enter executes the search → results mode, one hit.
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search.mode, crate::search::SearchMode::Results);
    assert_eq!(app.search.hits.len(), 1);
    assert_eq!(app.search.hits[0].path, vault_dir.path().join("sub/hay.md"));
    assert_eq!(app.search.hits[0].line, 1, "0-based raw line");
    assert_eq!(app.search.hits[0].text, "needle here");

    // Esc closes the popup entirely.
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search.open);
}

#[test]
fn search_enter_opens_note_scrolled_to_the_hit() {
    let vault_dir = TempDir::new("app-search-open");
    let body: String = (0..40)
        .map(|i| format!("filler {i}\n\n"))
        .chain(["the needle line\n\n".to_string()])
        .collect();
    std::fs::write(vault_dir.path().join("far.md"), &body).unwrap();

    let (file, mut app) = app(&vault_dir);
    app.viewer.viewport_height = 10;
    app.handle_key(key(KeyCode::Char('s')));
    for c in "needle".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Enter)); // open the selected hit

    assert_eq!(app.viewer.path, vault_dir.path().join("far.md"));
    assert!(app.nav.can_go_back(), "opened through the nav stack");
    assert!(
        app.viewer.scroll > 0,
        "the hit's line is scrolled into view (scroll = {})",
        app.viewer.scroll
    );
    // Backspace returns to the previous note (scroll restored there).
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.viewer.path, file.path());
    // The popup closed after a successful open.
    assert!(!app.search.open);
}

#[test]
fn search_results_typing_refines_and_open_failure_keeps_popup() {
    let vault_dir = search_vault();
    let (_file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('s')));
    app.handle_key(key(KeyCode::Char('n')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search.mode, crate::search::SearchMode::Results);
    // A typed character in results mode returns to input, appended.
    app.handle_key(key(KeyCode::Char('o')));
    assert_eq!(app.search.mode, crate::search::SearchMode::Input);
    assert_eq!(app.search.query, "no");

    // Enter on an empty result set stays open, shows nothing selected.
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Char('z')));
    app.handle_key(key(KeyCode::Char('z')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search.mode, crate::search::SearchMode::Results);
    assert!(app.search.hits.is_empty());
    app.handle_key(key(KeyCode::Enter)); // no selection: harmless no-op
    assert!(app.search.open);
}
#[test]
fn search_without_vault_is_a_noop() {
    let file = TempFile::new("solo-search.md", "x\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    app.handle_key(key(KeyCode::Char('s')));
    assert!(!app.search.open, "no index: no search");
}

#[test]
fn search_cancel_untouched_state_and_backspace_edits() {
    let vault_dir = search_vault();
    let (file, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('s')));
    app.handle_key(key(KeyCode::Char('a')));
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.search.query, "a");
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search.open);
    // Nothing happened to the app state.
    assert_eq!(app.viewer.path, file.path());
    assert_eq!(app.focus, Focus::Content);
}

// --- Unit 29: `n` / `N` walk the active search in the open note ------

/// Runs `query` through the search popup and closes it, leaving it as the
/// active search.
fn run_search(app: &mut App, query: &str) {
    app.handle_key(key(KeyCode::Char('s')));
    for c in query.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Esc));
}

/// The 0-based source lines of the open note the active search finds.
fn active_lines(app: &App) -> Vec<usize> {
    app.active_search
        .as_ref()
        .map(|active| active.occurrences(&app.viewer.source))
        .unwrap_or_default()
}

/// Where a jump to the rendered line containing `marker` lands: the line
/// with two context rows above it, clamped to the scroll range. Computed
/// from the rendered text, independently of the source-line map.
fn landing(app: &App, marker: &str) -> u32 {
    let rendered = app
        .viewer
        .lines
        .iter()
        .position(|line| line.spans.iter().any(|s| s.content.contains(marker)))
        .unwrap_or_else(|| panic!("no rendered line contains {marker:?}"));
    (rendered.saturating_sub(2) as u32).min(app.viewer.max_scroll())
}

/// A note long enough to scroll, whose paragraphs 20 and 45 carry the
/// needle.
fn needle_note() -> String {
    (0..60)
        .map(|i| match i {
            20 | 45 => format!("needle {i}\n\n"),
            _ => format!("para {i}\n\n"),
        })
        .collect()
}

#[test]
fn n_and_big_n_walk_the_active_search_and_wrap() {
    let vault_dir = TempDir::new("app-search-walk");
    let body = needle_note();
    std::fs::write(vault_dir.path().join("walk.md"), &body).unwrap();
    let mut app = App::new(
        Viewer::open(&vault_dir.path().join("walk.md"), 80).unwrap(),
        Some(vault_dir.path().to_path_buf()),
        Some(shared_index(&vault_dir)),
    )
    .unwrap();
    app.viewer.viewport_height = 10;

    // No search has run: `n` and `N` do nothing.
    app.viewer.set_scroll(5);
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, 5, "no active search: no move");
    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.viewer.scroll, 5, "no active search: no move");

    run_search(&mut app, "needle");
    assert!(app.active_search.is_some(), "the search stays active");
    app.viewer.scroll_top();

    let first = landing(&app, "needle 20");
    let second = landing(&app, "needle 45");
    assert!(
        first > 0 && second > first,
        "the fixture scrolls: {first}, {second}"
    );

    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, first, "n goes to the first occurrence");
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, second, "n goes to the next one");
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, first, "n wraps to the first at the end");

    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.viewer.scroll, second, "N wraps to the last at the top");
    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.viewer.scroll, first, "N goes to the previous one");
    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.viewer.scroll, second, "N wraps again");

    // The position is the scroll, not the last match: from between the two
    // occurrences, `n` finds the later one again.
    app.viewer.set_scroll(second - 1);
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(
        app.viewer.scroll, second,
        "n works from where the reader is"
    );
}

#[test]
fn n_and_big_n_do_nothing_without_an_occurrence_in_the_open_note() {
    // The vault has the needle; the open note has none.
    let vault_dir = search_vault();
    let file = TempFile::new("app-search-none.md", &"para\n\n".repeat(40));
    let mut app = App::new(
        Viewer::open(file.path(), 80).unwrap(),
        Some(vault_dir.path().to_path_buf()),
        Some(shared_index(&vault_dir)),
    )
    .unwrap();
    app.viewer.viewport_height = 10;
    run_search(&mut app, "needle");
    assert!(app.active_search.is_some());
    assert!(active_lines(&app).is_empty(), "the open note has no match");

    app.viewer.set_scroll(10);
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, 10, "no occurrence: no move");
    app.handle_key(key(KeyCode::Char('N')));
    assert_eq!(app.viewer.scroll, 10, "no occurrence: no move");

    // The keys are the note pane's: with the sidebar focused they are not
    // `n`/`N` for the note.
    app.focus = Focus::Sidebar;
    app.handle_key(key(KeyCode::Char('n')));
    assert_eq!(app.viewer.scroll, 10, "not the note pane: no move");
}

#[test]
fn each_executed_search_replaces_the_active_one() {
    let vault_dir = TempDir::new("app-search-replace");
    let body = "alpha one\n\nbeta two\n\nalpha three\n";
    std::fs::write(vault_dir.path().join("note.md"), body).unwrap();
    let mut app = App::new(
        Viewer::open(&vault_dir.path().join("note.md"), 80).unwrap(),
        Some(vault_dir.path().to_path_buf()),
        Some(shared_index(&vault_dir)),
    )
    .unwrap();

    run_search(&mut app, "alpha");
    assert_eq!(active_lines(&app), vec![0, 4]);
    run_search(&mut app, "beta");
    assert_eq!(active_lines(&app), vec![2], "a new search replaces it");

    // A query that never ran (an invalid pattern) keeps the previous one.
    run_search(&mut app, "re:(");
    assert_eq!(active_lines(&app), vec![2]);

    // A bare `re:` runs (the popup shows its empty results) but the empty
    // pattern never becomes an active search: it clears the previous one.
    run_search(&mut app, "re:");
    assert!(
        app.active_search.is_none(),
        "a bare re: clears the active search rather than replacing it"
    );
}

/// The occurrences are the search's own: a hit in the source text counts
/// even when the rendered text no longer carries it (a heading's `##`).
/// And a regular-expression search is walked the same way.
#[test]
fn n_follows_a_regex_search_in_the_note_source_text() {
    let vault_dir = TempDir::new("app-search-regex-walk");
    let body: String = (0..30)
        .map(|i| format!("para {i}\n\n"))
        .chain(["## needle heading\n\n".to_string()])
        .chain((30..60).map(|i| format!("para {i}\n\n")))
        .collect();
    std::fs::write(vault_dir.path().join("head.md"), &body).unwrap();
    let mut app = App::new(
        Viewer::open(&vault_dir.path().join("head.md"), 80).unwrap(),
        Some(vault_dir.path().to_path_buf()),
        Some(shared_index(&vault_dir)),
    )
    .unwrap();
    app.viewer.viewport_height = 10;

    run_search(&mut app, "re:## needle");
    app.viewer.scroll_top();
    app.handle_key(key(KeyCode::Char('n')));

    let target = landing(&app, "needle heading");
    assert!(target > 0, "the heading is deep in the note");
    assert_eq!(
        app.viewer.scroll, target,
        "the source-only match is a hit (the rendered line drops the ##)"
    );
}

// --- M4: git sync status ------------------------------------------

/// A real (tiny) git repository as a vault fixture. Fixture setup only:
/// the app's own git usage stays inside the read-only allowlist.
fn git_vault(name: &str) -> TempDir {
    use std::process::Command;
    let dir = TempDir::new(name);
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init", "-b", "main", "-q"]);
    std::fs::write(dir.path().join("a.md"), "A\n\n".repeat(50)).unwrap();
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "initial"]);
    dir
}

#[test]
fn startup_reads_git_status_clean() {
    let vault_dir = git_vault("app-git-clean");
    let (_file, mut app) = app(&vault_dir);
    app.wait_git();
    let git = app.git.as_ref().expect("git status read at startup");
    assert!(git.clean);
    assert!(git.changed_files.is_empty());
    assert_eq!((git.ahead, git.behind), (0, 0));
}

#[test]
fn u_key_refreshes_and_marks_dirty() {
    let vault_dir = git_vault("app-git-dirty");
    let (_f, mut app) = app(&vault_dir);
    app.wait_git();
    assert!(app.git.as_ref().unwrap().clean);

    std::fs::write(vault_dir.path().join("a.md"), "changed\n").unwrap();
    // The debounced poll path may skip (within the 5s window of the
    // startup refresh); the explicit `u` key must not.
    app.poll();
    app.handle_key(key(KeyCode::Char('u')));
    app.wait_git();
    let git = app.git.as_ref().unwrap();
    assert!(!git.clean);
    assert_eq!(
        git.changed_files,
        std::collections::BTreeSet::from([vault_dir.path().join("a.md")])
    );
}

#[test]
fn opening_a_note_refreshes_git_status() {
    // SPEC: git status is "refreshed on open". An external commit
    // fires no watch event (only `.git` changes), so without this the
    // markers/badge stay stale until `u`.
    let vault_dir = git_vault("app-git-onopen");
    let (_f, mut app) = app(&vault_dir);
    app.wait_git();
    assert!(app.git.as_ref().unwrap().clean);

    // External commit: a brand-new note appears committed; no vault
    // file the watcher tracks is touched by app code.
    std::fs::write(vault_dir.path().join("b.md"), "B\n").unwrap();
    let run = |args: &[&str]| {
        use std::process::Command;
        let out = Command::new("git")
            .args(args)
            .current_dir(vault_dir.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "external"]);

    // Age the debounce clock so the open-triggered refresh spawns.
    app.last_git_refresh = Some(Instant::now() - GIT_DEBOUNCE);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.wait_git();
    assert!(
        app.git.as_ref().unwrap().clean,
        "external commit picked up when a note is opened"
    );

    // The debounce still applies: an immediate further open does not
    // respawn (a fresh dirty state would otherwise flip the badge
    // without `u`).
    std::fs::write(vault_dir.path().join("a.md"), "dirty\n").unwrap();
    app.last_git_refresh = Some(Instant::now() - GIT_DEBOUNCE / 2);
    app.open_note(&vault_dir.path().join("b.md")).unwrap();
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.wait_git();
    assert!(
        app.git.as_ref().unwrap().clean,
        "open-refresh is debounce-gated like the watch path"
    );
}

#[test]
fn watch_reindex_refresh_is_debounced() {
    let vault_dir = git_vault("app-git-debounce");
    let (file, mut app) = app(&vault_dir);
    // Let the (background) startup query finish on the clean tree before
    // the repo is dirtied below.
    app.wait_git();
    // Fake a very recent refresh: watch-triggered refreshes within
    // GIT_DEBOUNCE must be skipped entirely.
    app.last_git_refresh = Some(Instant::now());
    file.rewrite("dirty now\n");
    std::fs::write(vault_dir.path().join("fresh.md"), "new\n").unwrap();
    app.poll();
    app.wait_git();
    assert!(
        app.git.as_ref().unwrap().clean,
        "debounced refresh must not respawn within 5s"
    );
}

#[test]
fn git_refresh_requested_in_flight_reruns_and_lands_the_latest_state() {
    let vault_dir = git_vault("app-git-rerun");
    let (_file, mut app) = app(&vault_dir);
    app.wait_git();
    assert!(app.git.as_ref().unwrap().clean);

    // First request starts the worker; dirty the tree, then ask again while
    // it may still be running: the second must not be dropped.
    app.refresh_git(true);
    std::fs::write(vault_dir.path().join("late.md"), "late\n").unwrap();
    app.refresh_git(true);
    app.wait_git();
    assert!(
        !app.git.as_ref().unwrap().clean,
        "the queued re-run saw the dirty tree"
    );
    assert!(
        app.git_job.is_none() && !app.git_rerun,
        "nothing left queued"
    );
}

#[test]
fn non_repo_vault_shows_no_git_status() {
    let vault_dir = vault(); // plain TempDir, no git
    let (_f, app) = app(&vault_dir);
    assert!(app.git.is_none(), "silent degrade outside a repository");
}

// --- post-v1 loop 4: guarded checkbox toggling -----------------------

/// A note carrying the vault's three task states plus an `updated:`
/// field (the bump target).
fn task_note() -> String {
    "---\nstatus: active\nupdated: 2026-01-01\n---\n\n- [ ] alpha task\n\n- [~] beta task\n\n- [x] gamma task\n\n".to_string()
        + &"body\n\n".repeat(30)
}

fn task_app(dir: &TempDir) -> App {
    let viewer = Viewer::open(&dir.path().join("note.md"), 80).unwrap();
    let index = shared_index(dir);
    App::new(viewer, Some(dir.path().to_path_buf()), Some(index)).unwrap()
}

fn task_vault() -> (TempDir, App) {
    let dir = TempDir::new("app-tasks");
    std::fs::write(dir.path().join("note.md"), task_note()).unwrap();
    let app = task_app(&dir);
    (dir, app)
}

/// Click coordinates of a task's glyph with the faked viewport
/// geometry (mirrors what ui::draw would record).
fn task_click(app: &mut App, index: usize) -> (u16, u16) {
    app.layout.note_viewport = Rect::new(0, 0, 40, 24);
    let task = app.viewer.tasks[index];
    let row = task.line as u32 - app.viewer.scroll;
    (task.start as u16, row as u16)
}

#[test]
fn click_on_checkbox_glyph_toggles_and_pipeline_repaints() {
    let (dir, mut app) = task_vault();
    let path = dir.path().join("note.md");
    assert!(app.viewer.disk_matches_parse());

    // Click the unchecked glyph: cycle writes `- [~]` (in-progress).
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("- [~] alpha task\n"), "{text}");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert!(
        text.contains(&format!("updated: {today}\n")),
        "updated field bumped to today: {text}"
    );
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );

    // The existing watch/reload + reindex pipeline picks the write up
    // on the next poll: the render and the index reflect the new state.
    app.viewer.invalidate_stamp();
    app.poll();
    assert_eq!(
        app.viewer.tasks[0].state,
        vault::tasks::TaskState::InProgress
    );
    let rendered: String = app.viewer.lines[app.viewer.tasks[0].line]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        rendered.contains("◐"),
        "in-progress renders visibly: {rendered}"
    );
    let index = app.index.as_ref().unwrap().borrow();
    assert!(
        index.note(&path).is_some(),
        "the note stayed indexed after the write"
    );
}

#[test]
fn clicking_cycles_through_all_three_states() {
    let (dir, mut app) = task_vault();
    let path = dir.path().join("note.md");
    let (col, row) = task_click(&mut app, 0);
    let states = ["- [~] alpha task", "- [x] alpha task", "- [ ] alpha task"];
    for expected in states {
        app.handle_click(col, row);
        assert!(
            std::fs::read_to_string(&path).unwrap().contains(expected),
            "expected {expected} on disk"
        );
        // Reload before the next click (the poll does this live).
        app.viewer.invalidate_stamp();
        app.poll();
    }
}

#[test]
fn toggle_without_an_updated_field_reports_through_the_info_channel() {
    let dir = TempDir::new("app-tasks-no-updated");
    std::fs::write(
        dir.path().join("note.md"),
        "---\nstatus: active\n---\n\n- [ ] alpha task\n\n",
    )
    .unwrap();
    let mut app = task_app(&dir);

    app.toggle_task(0);

    // The write succeeded: the notice about the missing `updated:` field
    // is neutral and expiring, not the red never-expiring error channel.
    assert!(
        app.viewer.last_error.is_none(),
        "a successful write is not an error: {:?}",
        app.viewer.last_error
    );
    assert_eq!(
        app.live_status_at(Instant::now()),
        Some("toggled (no updated: field in frontmatter to bump)"),
        "the notice goes through the info channel"
    );
    assert!(
        std::fs::read_to_string(dir.path().join("note.md"))
            .unwrap()
            .contains("- [~] alpha task"),
        "the marker toggle still stands"
    );

    // It expires like every other info message.
    app.expire_status_at(Instant::now() + STATUS_MESSAGE_TTL);
    assert!(app.last_status.is_none());
}

#[test]
fn toggle_refuses_when_the_file_changed_since_parse() {
    let (dir, mut app) = task_vault();
    let path = dir.path().join("note.md");
    // An external edit lands after the parse: the mtime+size guard
    // must refuse (the poll would reload; the user clicked first).
    std::fs::write(&path, task_note().replace("alpha", "renamed")).unwrap();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("not writing")),
        "refusal surfaced: {:?}",
        app.viewer.last_error
    );
    assert!(
        app.last_status.is_none(),
        "a refusal stays an error, never an info message"
    );
    assert!(
        std::fs::read_to_string(&path).unwrap().contains("renamed"),
        "the external edit was not overwritten"
    );
    let _ = &dir;
}

#[test]
fn toggle_refuses_on_a_dirty_git_tree_and_recovers_after_commit() {
    use std::process::Command;
    let dir = TempDir::new("app-tasks-git");
    std::fs::write(dir.path().join("note.md"), task_note()).unwrap();
    let run = |args: &[&str], cwd: &Path| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    };
    run(&["init", "-b", "main", "-q"], dir.path());
    run(&["add", "-A"], dir.path());
    run(&["commit", "-q", "-m", "initial"], dir.path());
    let mut app = task_app(&dir);
    let path = dir.path().join("note.md");

    // Clean tree: the toggle succeeds and `git diff` shows EXACTLY
    // the two intended changes (marker + updated), nothing else.
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    let diff = Command::new("git")
        .args(["diff", "--no-color"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let diff = String::from_utf8_lossy(&diff.stdout).into_owned();
    let mut removed: Vec<&str> = diff
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .collect();
    let mut added: Vec<&str> = diff
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .collect();
    removed.sort_unstable();
    added.sort_unstable();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    assert_eq!(
        removed,
        vec!["-- [ ] alpha task", "-updated: 2026-01-01"],
        "exactly the marker and the updated field removed"
    );
    assert_eq!(
        added,
        vec!["+- [~] alpha task", &format!("+updated: {today}")],
        "exactly the marker and the updated field added"
    );

    // Now the tree is dirty: a further toggle must be refused.
    app.viewer.invalidate_stamp();
    app.poll();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("dirty git tree")),
        "dirty-tree refusal: {:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [~] alpha task"),
        "no second write happened"
    );

    // Committing everything restores the write permission.
    run(&["add", "-A"], dir.path());
    run(&["commit", "-q", "-m", "toggle"], dir.path());
    app.handle_click(col, row);
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [x] alpha task"),
        "the toggle went through on the clean tree"
    );
}

#[test]
fn toggle_refuses_when_the_branch_is_ahead_or_behind() {
    use std::process::Command;
    let dir = TempDir::new("app-tasks-ahead");
    std::fs::write(dir.path().join("note.md"), task_note()).unwrap();
    let run = |args: &[&str], cwd: &Path| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    };
    run(&["init", "-b", "main", "-q"], dir.path());
    run(&["add", "-A"], dir.path());
    run(&["commit", "-q", "-m", "initial"], dir.path());
    let origin = TempDir::new("app-tasks-origin");
    run(
        &[
            "init",
            "--bare",
            "-b",
            "main",
            "-q",
            origin.path().to_str().unwrap(),
        ],
        dir.path(),
    );
    run(
        &["remote", "add", "origin", origin.path().to_str().unwrap()],
        dir.path(),
    );
    run(&["push", "-q", "-u", "origin", "main"], dir.path());
    let mut app = task_app(&dir);

    // A local commit the upstream does not have: ahead → refuse.
    std::fs::write(dir.path().join("other.md"), "extra\n").unwrap();
    run(&["add", "-A"], dir.path());
    run(&["commit", "-q", "-m", "local only"], dir.path());
    app.viewer.invalidate_stamp();
    app.poll();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("ahead of upstream")),
        "ahead refusal: {:?}",
        app.viewer.last_error
    );

    // The upstream moves ahead of us: behind → refuse too. First
    // discard the local ahead commit (fixture reset), then have the
    // clone commit and push so the upstream gains a commit.
    run(&["reset", "-q", "--hard", "origin/main"], dir.path());
    let clone = TempDir::new("app-tasks-clone");
    run(
        &[
            "clone",
            "-q",
            origin.path().to_str().unwrap(),
            clone.path().to_str().unwrap(),
        ],
        dir.path(),
    );
    std::fs::write(clone.path().join("remote.md"), "from elsewhere\n").unwrap();
    run(&["add", "-A"], clone.path());
    run(&["commit", "-q", "-m", "remote commit"], clone.path());
    run(&["push", "-q", "origin", "main"], clone.path());
    run(&["fetch", "-q", "origin"], dir.path());
    app.viewer.invalidate_stamp();
    app.poll();
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("behind upstream")),
        "behind refusal: {:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(dir.path().join("note.md"))
            .unwrap()
            .contains("- [ ] alpha task"),
        "no write happened while out of sync"
    );
}

#[test]
fn toggle_outside_the_vault_root_and_outside_any_repo_needs_no_git_guard() {
    // Documented decision: the git guard follows the note's own
    // repository. A note opened from outside the vault root that is
    // not inside any repository (the orphan rule) is toggleable with
    // the mtime guard alone — there is no git state to protect.
    let vault_dir = vault();
    let outside = TempDir::new("app-tasks-orphan");
    std::fs::write(outside.path().join("note.md"), task_note()).unwrap();
    let viewer = Viewer::open(&outside.path().join("note.md"), 80).unwrap();
    let index = shared_index(&vault_dir);
    let mut app = App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(outside.path().join("note.md"))
            .unwrap()
            .contains("- [~] alpha task"),
        "orphan toggle allowed with the mtime guard only"
    );
}

#[test]
fn toggle_outside_the_vault_root_but_inside_a_dirty_repo_is_refused() {
    // The guard follows the note: a dirty repository containing the
    // note is refused even when the note sits outside the vault root
    // — the note's own repo status is what describes the write.
    use std::process::Command;
    let vault_dir = vault();
    let outside = TempDir::new("app-tasks-orphan-repo");
    std::fs::write(outside.path().join("note.md"), task_note()).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(outside.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init", "-b", "main", "-q"]);
    // note.md is written but NOT committed: a dirty tree around the
    // note itself, so the toggle must be refused.
    let viewer = Viewer::open(&outside.path().join("note.md"), 80).unwrap();
    let index = shared_index(&vault_dir);
    let mut app = App::new(viewer, Some(vault_dir.path().to_path_buf()), Some(index)).unwrap();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("dirty git tree")),
        "own-repo dirty refusal outside the vault: {:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(outside.path().join("note.md"))
            .unwrap()
            .contains("- [ ] alpha task"),
        "no write happened in the dirty repo"
    );
}

#[test]
fn toggle_in_a_nested_repo_is_guarded_by_the_note_s_own_repo() {
    // Orchestrator decision (review F1): a note inside a nested
    // repository is guarded by *that* repo's status, not the vault
    // root's — outer clean + inner dirty refuses, inner clean
    // proceeds. Fixture: outer repo clean (the inner repo committed
    // as a gitlink), the inner repo holding the note.
    use std::process::Command;
    let run = |args: &[&str], cwd: &Path| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    };
    let dir = TempDir::new("app-tasks-nested");
    std::fs::write(dir.path().join("outer.md"), "outer\n").unwrap();
    let inner = dir.path().join("inner");
    std::fs::create_dir(&inner).unwrap();
    run(&["init", "-b", "main", "-q"], &inner);
    std::fs::write(inner.join("note.md"), task_note()).unwrap();
    run(&["add", "-A"], &inner);
    run(&["commit", "-q", "-m", "inner initial"], &inner);
    // Outer: commit the gitlink so the outer tree is clean too.
    run(&["init", "-b", "main", "-q"], dir.path());
    run(&["add", "-A"], dir.path());
    run(&["commit", "-q", "-m", "outer initial"], dir.path());

    let viewer = Viewer::open(&inner.join("note.md"), 80).unwrap();
    let index = shared_index(&dir);
    let mut app = App::new(viewer, Some(dir.path().to_path_buf()), Some(index)).unwrap();
    let path = inner.join("note.md");

    // Inner DIRTY first (an untracked extra file; the note itself
    // stays untouched): the toggle must be refused even though the
    // outer repo is still clean — the note's own repo is what is
    // consulted.
    let (col, row) = task_click(&mut app, 0);
    std::fs::write(inner.join("scratch.md"), "untracked\n").unwrap();
    app.handle_click(col, row);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("dirty git tree")),
        "inner-dirty refusal: {:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [ ] alpha task"),
        "no write happened while the inner repo was dirty"
    );

    // Removing the stray file restores the inner-clean state: the
    // toggle goes through.
    std::fs::remove_file(inner.join("scratch.md")).unwrap();
    app.viewer.invalidate_stamp();
    app.poll();
    app.handle_click(col, row);
    assert!(
        app.viewer.last_error.is_none(),
        "inner clean: {:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [~] alpha task"),
        "the toggle went through once the inner repo was clean"
    );
}

#[test]
fn toggle_works_without_any_vault() {
    // Single-file mode (no --vault): no git guard exists at all; the
    // mtime guard still applies.
    let file = TempFile::new("solo-task.md", &task_note());
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );
    assert!(
        std::fs::read_to_string(file.path())
            .unwrap()
            .contains("- [~] alpha task"),
        "single-file toggle allowed"
    );
}

#[test]
fn enter_toggles_a_focused_checkbox_and_x_reaches_tasks() {
    let (dir, mut app) = task_vault();
    let path = dir.path().join("note.md");
    // The note has no links: the first `x` lands on checkbox 0.
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(app.viewer.focus_task, Some(0));
    app.handle_key(key(KeyCode::Enter));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [~] alpha task"),
        "Enter cycled the focused checkbox"
    );
    // Enter again (focus preserved through the reload... poll first):
    app.viewer.invalidate_stamp();
    app.poll();
    assert_eq!(app.viewer.focus_task, Some(0), "focus survives the reload");
    app.handle_key(key(KeyCode::Enter));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("- [x] alpha task"),
        "second Enter: in-progress → checked"
    );
    let _ = &dir;
}

#[test]
fn x_cycles_links_and_checkboxes_in_rendered_order() {
    let dir = TempDir::new("app-tasks-links");
    std::fs::write(dir.path().join("target.md"), "target\n").unwrap();
    std::fs::write(
        dir.path().join("note.md"),
        "See [[target]] first\n\n- [ ] task one\n- [ ] task two\n",
    )
    .unwrap();
    let mut app = task_app(&dir);
    let seq = |app: &App| match app.viewer.focused_item() {
        Some(Focusable::Link(i)) => format!("link{i}"),
        Some(Focusable::Task(i)) => format!("task{i}"),
        None => "none".to_string(),
    };
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(seq(&app), "link0");
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(seq(&app), "task0");
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(seq(&app), "task1");
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(seq(&app), "link0", "wraps back to the first focusable");
    app.handle_key(key(KeyCode::Char('X')));
    assert_eq!(seq(&app), "task1");
    app.handle_key(key(KeyCode::Char('X')));
    assert_eq!(seq(&app), "task0");
    app.handle_key(key(KeyCode::Char('X')));
    assert_eq!(seq(&app), "link0");
}

#[test]
fn clicking_a_link_region_still_opens_even_on_a_task_line() {
    // A task whose text contains a wikilink: only the glyph columns
    // toggle; the link text keeps its open semantics.
    let dir = TempDir::new("app-tasks-mixed");
    std::fs::write(dir.path().join("target.md"), "target\n").unwrap();
    std::fs::write(dir.path().join("note.md"), "- [ ] see [[target]] now\n").unwrap();
    let mut app = task_app(&dir);
    let task = app.viewer.tasks[0];
    let link = app
        .viewer
        .links
        .iter()
        .find(|l| l.wikilink.as_ref().is_some_and(|w| w.target == "target"))
        .expect("link inside the task renders")
        .clone();
    assert!(
        link.regions[0].start >= task.end,
        "link starts past the glyph"
    );
    app.layout.note_viewport = Rect::new(0, 0, 40, 24);
    let r = link.regions[0];
    app.handle_click(r.start as u16, r.line as u16);
    assert_eq!(app.viewer.path, dir.path().join("target.md"), "link opened");
    // And the glyph click toggles instead.
    app.open_note(&dir.path().join("note.md")).unwrap();
    let (col, row) = task_click(&mut app, 0);
    app.handle_click(col, row);
    assert!(
        std::fs::read_to_string(dir.path().join("note.md"))
            .unwrap()
            .contains("- [~] see"),
        "glyph click toggled"
    );
}

// --- post-v1 loop 4: search popup clicks and wheels ------------------

fn search_state_app() -> (TempDir, TempFile, App) {
    let vault_dir = TempDir::new("app-search-click");
    std::fs::write(vault_dir.path().join("a.md"), "one two three\n").unwrap();
    std::fs::write(vault_dir.path().join("b.md"), "bee\n").unwrap();
    std::fs::write(vault_dir.path().join("c.md"), "sea\n").unwrap();
    let (file, mut app) = app(&vault_dir);
    app.search.open = true;
    app.search.mode = SearchMode::Results;
    app.search.hits = ["a.md", "b.md", "c.md"]
        .iter()
        .enumerate()
        .map(|(i, name)| vault::SearchHit {
            path: vault_dir.path().join(name),
            line: i,
            text: "hit".into(),
            start: 0,
            end: 3,
        })
        .collect();
    app.search.selected = 0;
    (vault_dir, file, app)
}

/// 3 hits → popup height 6 (3 list rows + borders + footer) at a
/// hand-chosen position; list rows are the screen rows 6..9.
fn search_popup() -> Rect {
    Rect::new(30, 5, 70, 6)
}

#[test]
fn click_selects_a_result_row_and_clicking_it_again_opens() {
    let (_dir, file, mut app) = search_state_app();
    let popup = search_popup();
    app.layout.search_area = Some(popup);
    // Click row 1 (the second hit): selects it, popup stays open.
    app.handle_click(40, popup.y + 2);
    assert_eq!(app.search.selected, 1, "click selected the row");
    assert!(app.search.open);
    assert_eq!(app.viewer.path, file.path(), "no open on first click");
    // Clicking the already-selected row opens it (Enter semantics).
    app.handle_click(40, popup.y + 2);
    assert_eq!(app.viewer.path, app.search.hits[1].path);
    assert!(!app.search.open, "popup closed after the open");
    assert!(app.nav.can_go_back(), "opened through the nav stack");
}

#[test]
fn search_popup_border_footer_and_outside_clicks_do_nothing() {
    let (_dir, file, mut app) = search_state_app();
    let popup = search_popup();
    app.layout.search_area = Some(popup);
    // Top border, footer row, and a point outside the popup: no
    // selection change, no open.
    app.handle_click(40, popup.y); // top border
    app.handle_click(40, popup.y + 4); // footer row (index 3 = out of range)
    app.handle_click(2, popup.y + 2); // outside the popup (left of it)
    app.handle_click(40, popup.y + popup.height); // below the popup
    assert_eq!(app.search.selected, 0);
    assert!(app.search.open);
    assert_eq!(app.viewer.path, file.path());
    // Input mode: clicks are inert there too.
    app.search.mode = SearchMode::Input;
    app.handle_click(40, popup.y + 1);
    assert_eq!(app.viewer.path, file.path());
}

/// Overflowing results: 10 hits do not fit the three list rows of a 6-row
/// popup, so the footer row maps to a real hit index that is *not on
/// screen*. Clicking it must be a no-op, or a later Enter opens a row the
/// reader never saw. (The other footer test uses 3 hits, where the list
/// fits the popup and the misroute cannot happen.)
#[test]
fn footer_click_is_a_no_op_when_the_results_overflow_the_popup() {
    let (_dir, _file, mut app) = search_state_app();
    app.search.hits = (0..10)
        .map(|i| vault::SearchHit {
            path: format!("{i}.md").into(),
            line: i,
            text: "hit".into(),
            start: 0,
            end: 3,
        })
        .collect();
    app.search.selected = 0;
    let popup = Rect::new(30, 5, 70, 6); // 3 list rows: popup.y + 1..=+3
    app.layout.search_area = Some(popup);

    // A visible row selects that row's hit; the footer does nothing.
    app.handle_click(40, popup.y + 3);
    assert_eq!(app.search.selected, 2, "the last visible row is hit 2");
    assert!(app.search.open, "a first click only selects");
    app.handle_click(40, popup.y + 4); // footer row
    assert_eq!(app.search.selected, 2, "the footer changes nothing");
    assert!(app.search.open, "the footer does not open a hit");

    // With the window scrolled to the bottom, the same holds and the
    // window does not jump. (Selection 8 → anchor 6, so the footer row
    // would map to hit 9 here — a hidden row, as above.)
    app.search.selected = 8;
    let list = crate::ui::PopupList::new(
        popup,
        crate::ui::PopupChrome::FOOTER,
        app.search.selected,
        app.search.hits.len(),
    );
    assert_eq!(list.start, 6, "precondition: the window is scrolled");
    app.handle_click(40, list.first_row);
    assert_eq!(app.search.selected, 6, "the top visible row is hit 6");
    let before = crate::ui::PopupList::new(
        popup,
        crate::ui::PopupChrome::FOOTER,
        app.search.selected,
        app.search.hits.len(),
    )
    .start;
    assert_eq!(before, 4);
    app.handle_click(40, popup.y + 4); // footer row
    assert_eq!(app.search.selected, 6, "footer click is still inert");
    let after = crate::ui::PopupList::new(
        popup,
        crate::ui::PopupChrome::FOOTER,
        app.search.selected,
        app.search.hits.len(),
    )
    .start;
    assert_eq!(after, before, "the list did not jump");
}

#[test]
fn wheel_over_search_results_scrolls_the_selection_not_the_content() {
    let (_dir, file, mut app) = search_state_app();
    let popup = search_popup();
    app.layout.search_area = Some(popup);
    app.viewer.viewport_height = 10;
    // Wheel down over the popup: the result selection moves; the
    // note behind the popup does not scroll.
    app.handle_wheel_at(40, popup.y + 2, true);
    assert_eq!(app.search.selected, 2, "selection scrolled down (clamped)");
    assert_eq!(app.viewer.scroll, 0, "content did not scroll");
    assert_eq!(app.viewer.path, file.path(), "no open happened");
    // Wheel up over the popup scrolls back.
    app.handle_wheel_at(40, popup.y + 2, false);
    assert_eq!(app.search.selected, 0);
    // Wheel outside the popup while it is open: nothing at all.
    app.handle_wheel_at(5, 2, true);
    app.handle_wheel_at(40, 1, true); // over the popup's top border row
    assert_eq!(app.search.selected, 0);
    assert_eq!(app.viewer.scroll, 0);
}

// --- theme support --------------------------------------------------

fn theme_app() -> (TempFile, App) {
    let file = TempFile::new("app-theme.md", "# Heading\n\nbody\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let app = App::new(viewer, None, None).unwrap();
    (file, app)
}

// --- post-v1 loop 6: .base table view --------------------------------

/// A tasks-like vault: three open tasks (priorities 1/2/3), one done
/// task, one project (wrong type) + a two-view tasks.base.
fn base_vault() -> TempDir {
    let dir = TempDir::new("app-base");
    std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
    std::fs::create_dir_all(dir.path().join("projects")).unwrap();
    let note = |front: &str| format!("---\n{front}---\nbody\n\n");
    std::fs::write(
        dir.path().join("tasks/a.md"),
        note("type: task\nstatus: active\npriority: 1\ntitle: Task A\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tasks/b.md"),
        note("type: task\nstatus: pending\npriority: 3\ntitle: Task B\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tasks/c.md"),
        note("type: task\nstatus: active\npriority: 2\ntitle: Task C\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tasks/done.md"),
        note("type: task\nstatus: complete\npriority: 9\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("projects/p.md"),
        note("type: project\nstatus: active\n"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tasks.base"),
        "filters:\n  and:\n    - file.inFolder(\"tasks\")\n    - 'type == \"task\"'\nviews:\n  - type: table\n    name: \"Open\"\n    filters:\n      or:\n        - 'status == \"pending\"'\n        - 'status == \"active\"'\n    order:\n      - file.name\n      - status\n    sort:\n      - property: priority\n        direction: DESC\n  - type: table\n    name: \"Done\"\n    filters:\n      and:\n        - 'status == \"complete\"'\n    order:\n      - file.name\n      - status\n",
    )
    .unwrap();
    dir
}

/// An app whose visible content is the vault's tasks.base (the
/// startup `.base`-argument shape).
fn base_app(dir: &TempDir) -> App {
    let viewer = Viewer::open(&dir.path().join("tasks.base"), 80).unwrap();
    let index = shared_index(dir);
    let mut app = App::new(viewer, Some(dir.path().to_path_buf()), Some(index)).unwrap();
    app.open_base_at(&dir.path().join("tasks.base"), 0).unwrap();
    app
}

#[test]
fn opening_a_base_shows_the_table_and_the_status_bar_reflects_it() {
    let dir = base_vault();
    let app = base_app(&dir);
    assert!(app.base.is_some());
    assert_eq!(
        app.current_path(),
        dir.path().join("tasks.base"),
        "the base file is the current file"
    );
    // Hand-computed: the Open view holds exactly the three
    // pending/active tasks (the done task and the project fail the
    // view filter), sorted by priority DESC: b, c, a.
    let base = app.base.as_ref().unwrap();
    assert_eq!(base.table.note_rows(), 3);
    let stems: Vec<&str> = base
        .table
        .rows
        .iter()
        .filter_map(|r| match r {
            vault::base::TableRow::Note(n) => n.path.file_stem().and_then(|s| s.to_str()),
            _ => None,
        })
        .collect();
    assert_eq!(stems, vec!["b", "c", "a"]);
}

#[test]
fn viewer_actions_are_inert_while_a_base_table_is_shown() {
    // The reviewer's sequence: a note is loaded and focused, then a
    // base table covers it (base → click note → x → Backspace).
    // x/X/Enter must not act on the invisible note — in particular
    // Enter must not fire the guarded checkbox write.
    let dir = base_vault();
    let note_path = dir.path().join("tasks/a.md");
    std::fs::write(
        &note_path,
        "---\ntype: task\nstatus: active\npriority: 1\ntitle: Task A\n---\nbody\n\n- [ ] alpha task\n",
    )
    .unwrap();
    let viewer = Viewer::open(&note_path, 80).unwrap();
    let index = shared_index(&dir);
    let mut app = App::new(viewer, Some(dir.path().to_path_buf()), Some(index)).unwrap();
    app.handle_key(key(KeyCode::Char('x')));
    assert_eq!(
        app.viewer.focus_task,
        Some(0),
        "with the note visible, x focuses the checkbox"
    );
    let before = std::fs::read_to_string(&note_path).unwrap();
    app.open_base_at(&dir.path().join("tasks.base"), 0).unwrap();
    app.handle_key(key(KeyCode::Char('x')));
    app.handle_key(key(KeyCode::Char('X')));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.viewer.focus_task,
        Some(0),
        "x/X must not move the hidden focus"
    );
    assert_eq!(
        std::fs::read_to_string(&note_path).unwrap(),
        before,
        "no toggle may fire on the invisible note"
    );
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );
    // The table interactions still work while the note is hidden.
    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.base.as_ref().unwrap().view_name(), "Done");
}

#[test]
fn a_failed_open_keeps_the_base_table() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    // Direct open of a missing note: the table must stay up.
    assert!(app.open_note(&dir.path().join("tasks/missing.md")).is_err());
    assert!(app.base.is_some(), "the table survives a failed open");
    assert_eq!(app.current_path(), dir.path().join("tasks.base"));
    // History path: forward to a since-deleted note must also keep
    // the table (and the stacks intact).
    app.open_note(&dir.path().join("tasks/a.md")).unwrap();
    assert!(app.base.is_none());
    std::fs::remove_file(dir.path().join("tasks/a.md")).unwrap();
    app.go_back().unwrap(); // back to the table
    assert!(app.base.is_some());
    app.go_forward().expect_err("the note is gone");
    assert!(app.base.is_some(), "the table survives the failed forward");
    assert_eq!(app.current_path(), dir.path().join("tasks.base"));
    assert!(
        app.nav.can_go_forward(),
        "the failed forward entry is still queued"
    );
}

#[test]
fn v_and_digit_keys_switch_base_views() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.base.as_ref().unwrap().view_name(), "Done");
    assert_eq!(app.base.as_ref().unwrap().table.note_rows(), 1);
    // Digit 1 jumps back to the first view; 9 clamps to the last.
    app.handle_key(key(KeyCode::Char('1')));
    assert_eq!(app.base.as_ref().unwrap().view_name(), "Open");
    app.handle_key(key(KeyCode::Char('9')));
    assert_eq!(app.base.as_ref().unwrap().view_name(), "Done");
    // v wraps from the last view back to the first.
    app.handle_key(key(KeyCode::Char('v')));
    assert_eq!(app.base.as_ref().unwrap().view_name(), "Open");
    // Without an open base the keys are inert (v does nothing).
    app.base = None;
    app.handle_key(key(KeyCode::Char('v')));
}

#[test]
fn clicking_a_name_cell_opens_the_note_and_back_returns_to_the_table() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    // Draw once so the click regions exist (sidebar hidden: clicks
    // must reach the full-width content area).
    app.sidebar_visible = false;
    app.layout.note_viewport = Rect::new(0, 0, 80, 24);
    {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    }
    // First data row = table line 2 → screen row area.y+1+2 = 3;
    // the name cell starts at inner column 0 → screen column 1.
    app.handle_click(1, 3);
    assert_eq!(
        app.viewer.path,
        dir.path().join("tasks/b.md"),
        "the clicked cell opened its note"
    );
    assert!(app.nav.can_go_back(), "opened through the nav stack");
    assert!(app.base.is_none(), "the table closed for the note");

    // Backspace returns to the table.
    app.handle_key(key(KeyCode::Backspace));
    assert!(app.base.is_some());
    assert_eq!(app.current_path(), dir.path().join("tasks.base"));
}

#[test]
fn editing_a_note_status_moves_rows_between_views_on_the_next_poll() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    app.base.as_mut().unwrap().set_view(1, None, "2026-01-01");
    // Rebuild against the real index (set_view(None) cleared rows).
    app.refresh_base_table();
    assert_eq!(app.base.as_ref().unwrap().table.note_rows(), 1);

    // done.md flips to active on disk (as the guarded toggle would);
    // the watcher's next poll reindexes it and the table re-derives.
    std::fs::write(
        dir.path().join("tasks/done.md"),
        "---\ntype: task\nstatus: active\npriority: 9\n---\nbody\n\n",
    )
    .unwrap();
    app.poll();
    assert_eq!(
        app.base.as_ref().unwrap().table.note_rows(),
        0,
        "the task left the Done view"
    );
    app.base_set_view(0);
    assert_eq!(
        app.base.as_ref().unwrap().table.note_rows(),
        4,
        "and joined the Open view"
    );
}

#[test]
fn editing_the_base_file_reparses_on_the_next_poll() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    assert_eq!(app.base.as_ref().unwrap().view_count(), 2);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        dir.path().join("tasks.base"),
        "views:\n  - type: table\n    name: Solo\n    order:\n      - file.name\n",
    )
    .unwrap();
    app.poll();
    let base = app.base.as_ref().unwrap();
    assert_eq!(base.view_count(), 1);
    assert_eq!(base.view_name(), "Solo");
    assert_eq!(
        base.table.note_rows(),
        5,
        "fail-open: the view has no filter"
    );
}

#[test]
fn base_history_round_trips_through_notes() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    // base → note (scroll preserved on the note side is trivially 0).
    app.open_note(&dir.path().join("tasks/a.md")).unwrap();
    assert!(app.base.is_none());
    // note → base again via open_base: nav stacks grow, back works.
    app.open_note(&dir.path().join("tasks.base")).unwrap();
    assert!(app.base.is_some());
    // Scroll the table, leave, come back via Backspace: restored.
    // (A frame first so the table's viewport is measured, like the
    // live draw loop.)
    {
        let backend = ratatui::backend::TestBackend::new(80, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    }
    if let Some(base) = &mut app.base {
        base.scroll_down(1);
    }
    app.open_note(&dir.path().join("tasks/c.md")).unwrap();
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.current_path(), dir.path().join("tasks.base"));
    assert_eq!(app.current_scroll(), 1, "table scroll restored");
}

#[test]
fn base_scroll_keys_move_the_table() {
    let dir = base_vault();
    let mut app = base_app(&dir);
    // Enough rows to scroll: give the table a tiny viewport.
    app.base.as_mut().unwrap().viewport = 3;
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.current_scroll(), 1);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.current_scroll(), 0);
    app.handle_key(key(KeyCode::Char('G')));
    let max = app.base.as_ref().unwrap().max_scroll();
    assert_eq!(app.current_scroll(), max);
    app.handle_key(key(KeyCode::Char('g')));
    assert_eq!(app.current_scroll(), 0);
}

#[test]
fn unparsable_base_fails_open_and_warns() {
    let dir = base_vault();
    std::fs::write(dir.path().join("broken.base"), "!!! not yaml [[\n").unwrap();
    let viewer = Viewer::open(&dir.path().join("broken.base"), 80).unwrap();
    let index = shared_index(&dir);
    let mut app = App::new(viewer, Some(dir.path().to_path_buf()), Some(index)).unwrap();
    app.open_base_at(&dir.path().join("broken.base"), 0)
        .unwrap();
    let base = app.base.as_ref().unwrap();
    assert!(
        !base.warnings.is_empty(),
        "parse warning surfaced: {:?}",
        base.warnings
    );
    assert_eq!(
        base.table.note_rows(),
        5,
        "fail-open: every vault note shows (never narrowed)"
    );
}

#[test]
fn help_documents_base_view_switching() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(key(KeyCode::Char('?')));
    let backend = ratatui::backend::TestBackend::new(100, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    let screen = terminal.backend().to_string();
    assert!(
        screen.contains("switch table view"),
        "the v/1-9 help line is documented: {screen}"
    );
}

#[test]
fn shift_t_cycles_presets_then_customs_and_wraps() {
    let dir = TempDir::new("app-cycle");
    std::fs::write(
        dir.path().join("zeta.toml"),
        "base = \"ocean\"\nappearance = \"dark\"\n",
    )
    .unwrap();
    let custom = crate::theme::discover_custom_themes(&[dir.path().to_path_buf()]);
    let dirs = vec![dir.path().to_path_buf()];
    let (_f, mut app) = theme_app();
    let (theme, warnings, load_errors) = app.install_theme_choices("ocean", &dirs, &custom);
    app.apply_theme(&theme, &warnings);
    assert!(warnings.is_empty());
    assert!(load_errors.is_empty());
    assert_eq!(app.theme_name(), "ocean");

    // Cycle forward through the rest of the presets, then the built-ins
    // and the custom.
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert_eq!(app.theme_name(), "solarized");
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert_eq!(
        app.theme_name(),
        "catppuccin-frappe",
        "built-ins follow the presets"
    );
    // Cycle to the end of the built-ins; the custom follows them.
    let mut guard = 0;
    while app.theme_name() != "tokyonight-storm" {
        app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
        guard += 1;
        assert!(guard <= 20, "cycle does not reach the built-in end");
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert_eq!(
        app.theme_name(),
        "zeta",
        "custom themes after the built-ins"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT));
    assert_eq!(app.theme_name(), "arctic", "wraps to the first preset");
    // Plain `t` is untouched (sidebar mode toggle); cycling is Shift+T.
    app.handle_key(key(KeyCode::Char('T'))); // no shift: uppercase via caps-lock style
    assert_eq!(app.theme_name(), "forest", "plain uppercase T still cycles");
}

/// Cycling re-parses with the new markdown palette: the heading color
/// visibly changes (the instant re-render requirement).
#[test]
fn cycling_reparses_with_the_new_markdown_palette() {
    let (_f, mut app) = theme_app();
    let (theme, _, _) = app.install_theme_choices("ocean", &[], &Default::default());
    app.apply_theme(&theme, &[]);
    let ocean_heading =
        engine::markdown_theme_by_preset(engine::MarkdownThemePreset::OceanDark).heading_1;
    assert_eq!(
        app.viewer.lines[0].spans[0].style.fg,
        Some(ocean_heading),
        "the applied theme colors the rendered content"
    );
    app.cycle_theme(); // → solarized
    let solarized_heading =
        engine::markdown_theme_by_preset(engine::MarkdownThemePreset::SolarizedDark).heading_1;
    assert_ne!(ocean_heading, solarized_heading);
    assert_eq!(
        app.viewer.lines[0].spans[0].style.fg,
        Some(solarized_heading)
    );
}

#[test]
fn unknown_startup_theme_warns_and_falls_back_to_default() {
    let (_f, mut app) = theme_app();
    let (theme, warnings, _load_errors) =
        app.install_theme_choices("does-not-exist", &[], &Default::default());
    assert_eq!(app.theme_name(), "ocean", "fallback to the default");
    assert!(
        warnings.iter().any(|w| w.contains("unknown theme")),
        "{warnings:?}"
    );
    assert_eq!(theme.name, "ocean");
}

#[test]
fn unknown_syntax_theme_falls_back_with_a_warning() {
    let (_f, mut app) = theme_app();
    let mut theme = crate::theme::preset_theme(engine::MarkdownThemePreset::OceanDark);
    theme.syntax = Some("no-such-syntax-theme".to_string());
    app.apply_theme(&theme, &[]);
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("unknown syntax theme")),
        "warning surfaced: {:?}",
        app.viewer.last_error
    );
    // A valid syntax theme clears the warning again.
    theme.syntax = Some("InspiredGitHub".to_string());
    app.apply_theme(&theme, &[]);
    assert!(app.viewer.last_error.is_none());
}

#[test]
fn config_hot_reload_applies_the_theme_key() {
    let (_f, mut app) = theme_app();
    let dir = TempDir::new("app-hotreload");
    let path = dir.path().join("config.toml");
    // Armed before the file exists (startup shape): the first poll
    // after it appears applies its theme.
    app.watch_config(path.clone());
    assert_eq!(app.theme_name(), "ocean", "watch_config does not apply");
    std::fs::write(&path, "theme = \"arctic\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "arctic", "poll picks up the file");

    // Change the theme key (different length → different stamp).
    std::fs::write(&path, "theme = \"solarized-dark\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "solarized");

    // Same theme key again: nothing to apply.
    app.poll_config();
    assert_eq!(app.theme_name(), "solarized");
}

#[test]
fn config_hot_reload_ignores_vault_and_watch_keys() {
    let (_f, mut app) = theme_app();
    let dir = TempDir::new("app-hotreload-vault");
    let path = dir.path().join("config.toml");
    app.watch_config(path.clone());
    std::fs::write(&path, "theme = \"forest\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "forest");

    // Only vault/watch edited: no theme change.
    std::fs::write(
        &path,
        "theme = \"forest\"\nvault = \"/tmp/other\"\nwatch = false\n",
    )
    .unwrap();
    app.poll_config();
    assert_eq!(
        app.theme_name(),
        "forest",
        "vault/watch keys are not live-applied"
    );
}

#[test]
fn config_hot_reload_malformed_keeps_theme_and_warns() {
    let (_f, mut app) = theme_app();
    let dir = TempDir::new("app-hotreload-bad");
    let path = dir.path().join("config.toml");
    app.watch_config(path.clone());
    std::fs::write(&path, "theme = \"arctic\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "arctic");

    // A different length from the previous payload: the config stamp is
    // mtime+size, and two quick writes can share an mtime tick.
    std::fs::write(&path, "!!! this is not valid toml !!!\n").unwrap();
    app.poll_config();
    assert_eq!(
        app.theme_name(),
        "arctic",
        "malformed re-read keeps the theme"
    );
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("could not parse config")),
        "warning surfaced: {:?}",
        app.viewer.last_error
    );

    // Repairing the file picks the new theme up (stamp changes again).
    std::fs::write(&path, "theme = \"solarized\"\nextra = 1\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "solarized");
}

#[test]
fn config_hot_reload_rediscovers_a_theme_added_mid_session() {
    let (_f, mut app) = theme_app();
    // Startup with no custom themes (empty discovery).
    let (_theme, _warnings, _load_errors) =
        app.install_theme_choices("ocean", &[], &Default::default());
    let dir = TempDir::new("app-hotreload-rediscover");
    let themes = dir.path().join("themes");
    std::fs::create_dir(&themes).unwrap();
    app.theme_dirs = vec![themes.clone()];
    let path = dir.path().join("config.toml");
    app.watch_config(path.clone());

    // The config names a theme that does not exist yet: the apply
    // fails (rediscovery finds nothing either) and warns.
    std::fs::write(&path, "theme = \"latecomer\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "ocean", "unknown theme: default kept");
    assert!(
        app.viewer
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("unknown theme")),
        "{:?}",
        app.viewer.last_error
    );

    // The theme file appears mid-session; the very next config tick
    // (no config change needed) re-checks the pending name, re-scans
    // the theme dirs, and applies it.
    std::fs::write(
        themes.join("latecomer.toml"),
        "base = \"forest\"\nappearance = \"dark\"\n",
    )
    .unwrap();
    app.poll_config();
    assert_eq!(
        app.theme_name(),
        "latecomer",
        "hot-reload re-discovered the theme added mid-session"
    );
    assert!(
        app.viewer.last_error.is_none(),
        "{:?}",
        app.viewer.last_error
    );
    // Once resolved, the pending name is cleared: further ticks are
    // quiet even before the file existed-check would matter.
    app.poll_config();
    assert_eq!(app.theme_name(), "latecomer");
}

/// F1 regression (review 2026-11-11-11): a config change to a
/// *different valid* theme must clear a stale pending name — the
/// config's intent is the new theme, so a pending theme file
/// appearing later must not hot-apply over it.
#[test]
fn config_change_to_valid_theme_clears_a_stale_pending() {
    let (_f, mut app) = theme_app();
    // Startup with no custom themes (empty discovery).
    let (_theme, _warnings, _load_errors) =
        app.install_theme_choices("ocean", &[], &Default::default());
    let dir = TempDir::new("app-hotreload-stale-pending");
    let themes = dir.path().join("themes");
    std::fs::create_dir(&themes).unwrap();
    app.theme_dirs = vec![themes.clone()];
    let path = dir.path().join("config.toml");
    app.watch_config(path.clone());

    // The config names a missing theme X: default kept, X pending.
    std::fs::write(&path, "theme = \"missing-x\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "ocean", "unknown theme: default kept");
    assert_eq!(
        app.pending_theme.as_deref(),
        Some("missing-x"),
        "missing config theme goes pending"
    );

    // Config edit to a valid Y: Y applies AND the pending name is
    // cleared — the pending name is by definition no longer the
    // config's intent.
    std::fs::write(&path, "theme = \"forest\"\n").unwrap();
    app.poll_config();
    assert_eq!(app.theme_name(), "forest", "the valid config theme applies");
    assert!(
        app.pending_theme.is_none(),
        "a changed poll that applies a theme clears the stale pending"
    );

    // X's file appears later: the next tick must apply nothing (Y
    // stays) — the stale pending no longer exists.
    std::fs::write(
        themes.join("missing-x.toml"),
        "base = \"forest\"\nappearance = \"dark\"\n",
    )
    .unwrap();
    app.poll_config();
    assert_eq!(
        app.theme_name(),
        "forest",
        "the cleared pending must not hot-apply over the config's theme"
    );
}

#[test]
fn shift_t_at_cycle_end_redisovers_new_custom_themes() {
    let dir = TempDir::new("app-cycle-rediscover");
    let themes = dir.path().join("themes");
    std::fs::create_dir(&themes).unwrap();
    std::fs::write(
        themes.join("alpha.toml"),
        "base = \"ocean\"\nappearance = \"dark\"\n",
    )
    .unwrap();
    let custom = crate::theme::discover_custom_themes(std::slice::from_ref(&themes));
    let (_f, mut app) = theme_app();
    let (_theme, _warnings, _load_errors) =
        app.install_theme_choices("ocean", std::slice::from_ref(&themes), &custom);
    // arctic → forest → ocean → solarized → the 14 built-ins → alpha
    // (last entry): cycle until we are on alpha.
    let mut guard = 0;
    while app.theme_name() != "alpha" {
        app.cycle_theme();
        guard += 1;
        assert!(guard <= 20, "cycle does not reach the custom theme");
    }

    // A second custom theme file appears; the cycle is at its end,
    // so the next Shift+T re-scans and rotates into the new theme.
    std::fs::write(
        themes.join("zulu.toml"),
        "base = \"forest\"\nappearance = \"dark\"\n",
    )
    .unwrap();
    app.cycle_theme();
    assert_eq!(
        app.theme_name(),
        "zulu",
        "wrapping past the end re-discovered the new custom theme"
    );
    // And the rebuilt cycle still wraps correctly (arctic next: the
    // rebuilt entries are presets first, then built-ins, then customs
    // alphabetically).
    app.cycle_theme();
    assert_eq!(app.theme_name(), "arctic");
}

// --- status line loop: transient info message, copy path -------------

/// Shift+T sets the `theme: <name>` info message (not an error).
#[test]
fn ctrl_t_changes_only_the_group_and_shift_t_moves_the_theme() {
    let (_f, mut app) = theme_app();
    let (theme, warnings, _) = app.install_theme_choices("ocean", &[], &BTreeMap::new());
    app.apply_theme(&theme, &warnings);
    let ctrl_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let shift_t = KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT);
    assert_eq!(app.theme_rotation, ThemeRotation::All);

    // all → light: the group changes, the theme on screen does not.
    app.handle_key(ctrl_t);
    assert_eq!(app.theme_rotation, ThemeRotation::Light);
    assert_eq!(app.theme_name(), "ocean", "Ctrl+T does not move the theme");
    let status = app.last_status.clone().expect("status set");
    assert_eq!(status.text, "theme group: light (5 themes)");

    // Shift+T moves into the light group.
    app.handle_key(shift_t);
    assert_eq!(
        app.theme_choices.current().theme.appearance,
        Appearance::Light
    );

    // light → dark: Shift+T then cycles dark themes.
    app.handle_key(ctrl_t);
    assert_eq!(app.theme_rotation, ThemeRotation::Dark);
    app.handle_key(shift_t);
    assert_eq!(
        app.theme_choices.current().theme.appearance,
        Appearance::Dark
    );
}

#[test]
fn cycle_theme_sets_status_message() {
    let (_f, mut app) = theme_app();
    app.cycle_theme();
    let status = app.last_status.clone().expect("status set");
    assert_eq!(status.text, format!("theme: {}", app.theme_name()));
    assert!(app.viewer.last_error.is_none(), "info, not an error");
}

/// The message is live for the TTL and gone once the poll tick has
/// seen it age past it. Fabricated instants: no sleeping.
#[test]
fn status_message_expires_after_ttl_on_poll_tick() {
    let (_f, mut app) = theme_app();
    let t0 = Instant::now();
    app.set_status_at("theme: x", t0);
    let just_before = t0 + STATUS_MESSAGE_TTL - Duration::from_millis(1);
    assert_eq!(app.live_status_at(just_before), Some("theme: x"));
    app.expire_status_at(just_before);
    assert!(app.last_status.is_some(), "still live before 3s");
    let expired = t0 + STATUS_MESSAGE_TTL;
    app.expire_status_at(expired);
    assert!(app.last_status.is_none(), "cleared after 3s");
    assert_eq!(app.live_status_at(expired), None);
}

/// Errors keep `last_error` (red) and do not touch the info message;
/// the status waits while the error is set (checked in the renderer —
/// here: the state the renderer reads).
#[test]
fn error_is_independent_of_status_message() {
    let (_f, mut app) = theme_app();
    let t0 = Instant::now();
    app.set_status_at("theme: x", t0);
    app.viewer.last_error = Some("boom".to_string());
    assert_eq!(app.live_status_at(t0), Some("theme: x"));
    assert_eq!(app.viewer.last_error.as_deref(), Some("boom"));
    // Clearing the error leaves a still-live status to show.
    app.viewer.last_error = None;
    assert_eq!(app.live_status_at(t0), Some("theme: x"));
}

/// Config hot-reload to a new theme sets the same info message.
#[test]
fn hot_reload_theme_sets_status_message() {
    let (_f, mut app) = theme_app();
    let dir = TempDir::new("app-status-hotreload");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "theme = \"solarized\"\n").unwrap();
    app.watch_config(path.clone());
    std::fs::write(&path, "theme = \"ocean\"\n").unwrap();
    app.poll_config();
    let status = app.last_status.clone().expect("hot-reload status set");
    assert_eq!(status.text, format!("theme: {}", app.theme_name()));
}

/// Clicking the status-bar name copies the vault-relative path (OSC 52
/// staged) and sets `copied: <rel>`; the write goes to `flush_clipboard`.
#[test]
fn click_on_status_name_copies_relative_path() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("sub").join("b.md"))
        .unwrap();
    // The name segment rect, as the renderer would record it.
    app.layout.status_name_area = Some(Rect::new(0, 23, 20, 1));
    app.handle_click(3, 23);
    assert!(app.pending_clipboard.is_some(), "copy staged");
    let status = app.last_status.clone().expect("copy status set");
    assert_eq!(status.text, "copied: sub/b.md");
    let mut out: Vec<u8> = Vec::new();
    app.flush_clipboard(&mut out);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        crate::clipboard::osc52_sequence("sub/b.md")
    );
    assert!(app.pending_clipboard.is_none(), "written once");
}

/// A click elsewhere on the status bar (outside the name segment) does
/// nothing: no copy, no message.
#[test]
fn click_elsewhere_on_status_bar_does_nothing() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("sub").join("b.md"))
        .unwrap();
    app.layout.status_name_area = Some(Rect::new(0, 23, 10, 1));
    app.handle_click(60, 23);
    assert!(app.pending_clipboard.is_none());
    assert!(app.last_status.is_none());
}

/// A failed OSC 52 write surfaces as an error (`last_error`), and the
/// info message is dropped so the error is what shows.
#[test]
fn failed_clipboard_write_becomes_an_error() {
    struct Broken;
    impl io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("no tty"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (_f, mut app) = theme_app();
    app.pending_clipboard = Some(crate::clipboard::osc52_sequence("x"));
    app.set_status("copied: x");
    app.flush_clipboard(&mut Broken);
    assert!(app.last_status.is_none());
    let err = app.viewer.last_error.clone().expect("error surfaced");
    assert!(err.starts_with("copy failed"), "{err}");
}

// --- the quick note switcher (Ctrl+P) --------------------------------------

/// `Ctrl+P` opens the switcher over every note in the vault; typing
/// filters the list live.
#[test]
fn ctrl_p_opens_the_switcher_and_filters_live() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    assert_eq!(app.handle_key(ctrl(KeyCode::Char('p'))), Flow::Continue);
    assert!(app.quick.open, "the switcher is open");
    assert_eq!(
        app.quick.items.len(),
        3,
        "every note is listed before the query"
    );

    app.handle_key(key(KeyCode::Char('b')));
    assert_eq!(app.quick.query, "b");
    // `b` matches `sub/b.md` (in the file name) and `sub/c.md` (the
    // folder name contains a `b`); the file-name match ranks first.
    assert_eq!(app.quick.items.len(), 2);
    assert_eq!(app.quick.items[0].label, "sub/b.md");
}

/// Enter opens the selected note through the normal nav stack, so
/// Backspace returns to the previous one.
#[test]
fn enter_in_the_switcher_opens_the_note_and_pushes_history() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(ctrl(KeyCode::Char('p')));
    app.handle_key(key(KeyCode::Char('b')));
    assert_eq!(app.quick.items[0].label, "sub/b.md");
    app.handle_key(key(KeyCode::Enter));

    assert!(!app.quick.open, "the popup closes after the open");
    assert_eq!(app.focus, Focus::Content);
    assert!(
        app.current_path().ends_with("sub/b.md"),
        "the matched note is open: {:?}",
        app.current_path()
    );
    // Backspace returns to `a.md` (the nav stack got the push).
    app.handle_key(key(KeyCode::Backspace));
    assert!(
        app.current_path().ends_with("a.md"),
        "{:?}",
        app.current_path()
    );
}

/// The switcher is modal: a typed `q` goes into the query, it never quits.
#[test]
fn the_switcher_is_modal() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Flow::Continue);
    assert!(app.quick.open, "still open");
    assert_eq!(app.quick.query, "q");
    // Ctrl+C still quits from the popup.
    assert_eq!(app.handle_key(ctrl(KeyCode::Char('c'))), Flow::Quit);
}

/// Esc closes the switcher without opening anything.
#[test]
fn escape_closes_the_switcher() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    let before = app.current_path();
    app.handle_key(ctrl(KeyCode::Char('p')));
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.quick.open);
    assert_eq!(app.current_path(), before, "nothing was opened");
}

/// `↑`/`↓` move the selection; the selection is clamped to the list.
#[test]
fn the_switcher_selection_moves_with_the_arrows() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert_eq!(app.quick.selected, 0);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.quick.selected, 1);
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.quick.selected, 0);
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.quick.selected, 0, "clamped at the top");
    app.handle_key(key(KeyCode::PageDown));
    assert_eq!(app.quick.selected, 2, "a page down jumps to the last item");
}

/// Without a vault index there is nothing to jump to: `Ctrl+P` is inert.
#[test]
fn ctrl_p_without_a_vault_does_nothing() {
    let file = TempFile::new("app-noquick.md", "MAIN\n");
    let viewer = Viewer::open(file.path(), 80).unwrap();
    let mut app = App::new(viewer, None, None).unwrap();
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(!app.quick.open);
}

/// A quick-switcher popup of height 4 (a terminal six rows or fewer) has
/// no room for a list row: the footer row is chrome, so clicking it must
/// not select (and then open) the first note.
#[test]
fn a_degenerate_quick_popup_ignores_clicks_on_its_footer() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.open_note(&vault_dir.path().join("a.md")).unwrap();
    app.handle_key(ctrl(KeyCode::Char('p')));
    let area = Rect::new(0, 0, 100, 4);
    let popup = crate::quick::popup_geometry(area, app.quick.items.len());
    assert_eq!(popup.height, 4, "precondition: borders + prompt + footer");
    app.layout.quick_area = Some(popup);
    let before = app.current_path();

    for row in popup.y..popup.y + popup.height {
        app.handle_click(popup.x + 2, row);
    }

    assert!(app.quick.open, "no click opened a note");
    assert_eq!(
        app.current_path(),
        before,
        "nothing behind the popup opened"
    );
}

/// A click on a switcher row selects it; a second click on the selected
/// row opens it.
#[test]
fn clicking_a_switcher_row_selects_then_opens_it() {
    let vault_dir = vault();
    let (_f, mut app) = app(&vault_dir);
    app.handle_key(ctrl(KeyCode::Char('p')));
    // The popup geometry the renderer records: 70 wide, centered.
    let area = Rect::new(0, 0, 100, 24);
    app.layout.quick_area = Some(crate::quick::popup_geometry(area, app.quick.items.len()));
    let popup = app.layout.quick_area.unwrap();
    // First inner row is the prompt, the rows start one below it.
    let row = popup.y + 2;

    app.handle_click(popup.x + 2, row + 1);
    assert_eq!(app.quick.selected, 1, "the clicked row is selected");
    assert!(app.quick.open, "a first click only selects");
    app.handle_click(popup.x + 2, row + 1);
    assert!(!app.quick.open, "clicking the selected row opens it");
    assert!(
        app.current_path().ends_with("sub/b.md"),
        "the clicked row is open: {:?}",
        app.current_path()
    );
}
