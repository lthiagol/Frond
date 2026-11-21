//! Application state: current note (`Viewer`), sidebar (tree / tags),
//! backlinks panel, focus, navigation history, and the disambiguation
//! popup. Terminal-independent; `main.rs` runs the event loop, `ui.rs`
//! draws.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use vault::{GitStatus, SearchResults};

use crate::config::ConfigWatch;
use crate::links::{LinkState, RenderedLink};
use crate::nav::Nav;
use crate::outline::Outline;
use crate::search::{ActiveSearch, SearchMode, SearchQuery, SearchState};
use crate::sidebar::{NodeKind, Sidebar};
use crate::tags::TagList;
use crate::theme::{Theme, ThemeChoices, ThemeRotation};
use crate::ui;
use crate::viewer::{Focusable, SharedIndex, Viewer};
use crate::watch::VaultWatcher;

/// Sidebar width bounds for the `<`/`>` override (columns). The floor
/// matches the proportional default's clamp in `ui::sidebar_width`.
pub const SIDEBAR_MIN_WIDTH: u16 = 24;
pub const SIDEBAR_MAX_WIDTH: u16 = 120;

pub use crate::baseview::BaseState;

/// Interval between vault-wide watch scans. The current file is polled
/// more often (`viewer.rs` poll interval); the full-vault scan is the
/// expensive-ish part (walk + stat of every note), so it runs slower.
/// Cheap change detector for the theme directories: every entry's name,
/// mtime and size (a `stat` each, no file reads or parsing).
fn theme_dirs_signature(dirs: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
    let mut sig = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let meta = entry.metadata().ok();
            sig.push((
                entry.path(),
                meta.as_ref().and_then(|m| m.modified().ok()),
                meta.map_or(0, |m| m.len()),
            ));
        }
    }
    sig.sort();
    sig
}

pub const VAULT_SCAN_INTERVAL: Duration = Duration::from_secs(1);

/// Minimum time between git status spawns. Watch-triggered refreshes are
/// debounced by this much; `u` (explicit refresh) and startup bypass it.
pub const GIT_DEBOUNCE: Duration = Duration::from_secs(5);

/// How long an info status message (`last_status`) stays on the status
/// bar. Expired by the poll tick (`App::poll`), no timer thread.
pub const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(3);

/// A transient, neutral-colored info message for the status bar (e.g.
/// `theme: solarized`). Separate from `viewer.last_error`, which is red
/// and wins while set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMessage {
    pub text: String,
    pub shown_at: Instant,
}

/// Which pane receives navigation keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Content,
    Backlinks,
}

/// What the sidebar shows: the folder tree, the tag list (`t` toggles) or
/// the open note's outline (`o` toggles over the other two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Tree,
    Tags,
    Outline,
}

/// The scrollbar that owns an active left-button drag. Each bar is a
/// position control for its own pane; the target says which pane follows
/// the pointer until the release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarTarget {
    Note,
    /// The open `.base` table: its bar is a document position, like the
    /// note pane's, not a selection cursor.
    Base,
    /// The sidebar panel: the tree, tag list or outline currently shown.
    Sidebar,
    Backlinks,
}

/// A still-ambiguous wikilink awaiting the user's pick (shown as a popup;
/// only reached when the same-folder/nearest-ancestor hint found nothing).
pub struct Disambiguation {
    pub candidates: Vec<PathBuf>,
    pub selected: usize,
    /// Heading fragment of the activated link, applied after opening.
    pub heading: Option<String>,
}

/// Loop control returned by `handle_key`.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

/// Widget geometry as of the last draw. `ui::draw` refreshes it every
/// frame; click and wheel handlers (which run between frames) route by it.
#[derive(Debug, Default)]
pub struct Layout {
    /// The note viewport, for click → link mapping.
    pub note_viewport: Rect,
    /// The note pane's scrollbar column (`None` while no bar is drawn):
    /// routes the press and drag that make it a position control.
    pub note_scrollbar_area: Option<Rect>,
    /// The `.base` table's scrollbar column (`None` while no bar is drawn):
    /// routes the press and drag that make it a position control.
    pub base_scrollbar_area: Option<Rect>,
    /// The sidebar widget (`None` while hidden).
    pub sidebar_area: Option<Rect>,
    /// The sidebar panel's scrollbar column (`None` while its rows fit):
    /// its bar positions the shown list's selection (tree, tags or outline
    /// - the three share one panel and one bar).
    pub sidebar_scrollbar_area: Option<Rect>,
    /// The backlinks panel (`None` while closed).
    pub backlinks_area: Option<Rect>,
    /// The backlinks panel's scrollbar column (`None` while its rows fit):
    /// its bar positions the panel's selection.
    pub backlinks_scrollbar_area: Option<Rect>,
    /// The search popup (`None` while closed): routes result-row clicks and
    /// wheels by position.
    pub search_area: Option<Rect>,
    /// The quick-switcher popup (`None` while closed): routes result-row
    /// clicks and wheels by position.
    pub quick_area: Option<Rect>,
    /// The status bar's note-name segment (`None` when no name segment is
    /// drawn): routes the copy-path click.
    pub status_name_area: Option<Rect>,
}

pub struct App {
    pub viewer: Viewer,
    /// `Some` only when `--vault` was passed — without it there is no
    /// sidebar (the vault *index* is still built, from the current
    /// directory).
    pub sidebar: Option<Sidebar>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
    /// The tree or tag view the outline (`o`) overlays; `o` again restores
    /// it. Only ever `Tree` or `Tags`.
    outline_prev: SidebarMode,
    /// Outline view state (used when `sidebar_mode == Outline`).
    pub outline: Outline,
    /// Tag view state (used when `sidebar_mode == Tags`).
    pub tags: TagList,
    /// Vault index (booted from `--vault`, or from the cwd when no
    /// `--vault` was given — the opened file's ancestors are NOT searched).
    pub index: Option<SharedIndex>,
    pub focus: Focus,
    pub nav: Nav,
    /// Backlinks panel state (`b` toggles; entries = notes linking here).
    pub backlinks_open: bool,
    pub backlinks: Vec<PathBuf>,
    pub backlink_sel: usize,
    pub backlink_scroll: usize,
    pub backlink_viewport: usize,
    /// Disambiguation popup for a wikilink the hint could not narrow.
    pub disambiguation: Option<Disambiguation>,
    /// Global search popup (`s`): input prompt + results.
    pub search: SearchState,
    /// The last search the popup executed: `n`/`N` walk its occurrences in
    /// the open note. Each executed search replaces it; a query that never
    /// ran (blank, invalid pattern) keeps the previous one.
    pub active_search: Option<ActiveSearch>,
    /// Quick note switcher (`Ctrl+P`): fuzzy note jump over the index.
    pub quick: crate::quick::QuickState,
    /// Last read-only git sync status (`None` = unavailable: no git, not
    /// a repository, or every command failed — the status bar shows
    /// nothing in that case).
    pub git: Option<GitStatus>,
    /// Vault-wide change detection (poll-based; see `watch.rs`).
    pub watcher: Option<VaultWatcher>,
    /// Widget geometry as of the last draw (see [`Layout`]).
    pub layout: Layout,
    /// Last `viewer.last_reload` value we reacted to with a sidebar rescan
    /// (loose refresh: the tree is re-walked only after a watch reload).
    last_rescan: Option<SystemTime>,
    last_vault_scan: Option<Instant>,
    /// When the git status was last actually spawned (debounce clock).
    last_git_refresh: Option<Instant>,
    /// In-flight background `git status` query (see [`Self::refresh_git`]).
    git_job: Option<std::sync::mpsc::Receiver<Option<GitStatus>>>,
    /// A refresh was requested while one was in flight: run once more when
    /// it lands, so the final state is never missed.
    git_rerun: bool,
    /// Active theme (chrome + markdown palette). Cycled with `Shift+T`,
    /// replaced by config hot-reloads; session-only, never persisted.
    pub theme_ui: crate::theme::UiTheme,
    /// Whether the app currently wants mouse capture on (`m` toggles, the
    /// config `mouse` key sets the startup default; main.rs executes the
    /// actual capture switch when this diverges from the live state). Off
    /// = the terminal handles the mouse natively (text selection/copy).
    pub mouse_capture: bool,
    /// The scrollbar whose drag owns the pointer, if any: the press on a bar
    /// starts it, every drag event keeps that pane following the pointer, and
    /// the release that ends it is consumed instead of being handled as a
    /// click. See [`Self::handle_press`].
    bar_drag: Option<BarTarget>,
    /// The pointer's last known cell. `None` until the first mouse event and
    /// again while mouse capture is off: no events arrive then, so a stale
    /// cell must not keep a bar's hover lit. Drives the scrollbar thumb
    /// hover (`ui::draw_scrollbar`); every mouse event reports it, not just
    /// the presses, so the highlight follows the pointer.
    pub pointer: Option<Position>,
    /// Sidebar width override in terminal columns (`None` = the default
    /// 30% of the terminal, clamped 24–60; see `ui::sidebar_width`). Set
    /// by `<`/`>` and the config `sidebar_width` key, clamped 24–120.
    pub sidebar_width: Option<u16>,
    theme_choices: ThemeChoices,
    /// Which appearances `Shift+T` cycles (session-only; `Ctrl+T` advances
    /// it, `theme_rotation` in config.toml sets the startup value).
    pub theme_rotation: ThemeRotation,
    /// Blank columns on each side of the note text (config `margin`).
    pub note_margin: u16,
    /// Blank rows above and below the note text (config `margin_y`).
    pub note_margin_y: u16,
    /// Whether the note pane draws a line-number gutter (config
    /// `line_numbers`; off = exactly the pre-gutter layout).
    pub line_numbers: bool,
    /// Status-bar icon set (config `icons`).
    pub icons: crate::config::IconSet,
    /// Discovered custom theme files (name → path), reused by config
    /// hot-reloads without re-walking the theme dirs.
    discovered_themes: BTreeMap<String, PathBuf>,
    /// Config-file hot reload state (`None` when there is no config path).
    config_watch: Option<ConfigWatch>,
    /// Cadence gate for the config mtime poll (the 1 s watcher tick).
    last_config_check: Option<Instant>,
    /// Theme directory list captured at startup (where custom theme files
    /// are discovered; re-discovery scans these same dirs mid-session —
    /// the dirs are fixed, the files inside them are not).
    theme_dirs: Vec<PathBuf>,
    /// Theme name the config asks for but that does not resolve (yet):
    /// re-checked on every config tick, so a custom theme file added
    /// mid-session is picked up without a config change. `None` when the
    /// current config theme resolves (or names nothing).
    pending_theme: Option<String>,
    /// Stat-only signature of the theme dirs at the last re-discovery for
    /// `pending_theme`; re-discovery (a parse of every theme file) only
    /// re-runs when it changes. `None` = not scanned for this pending name.
    pending_theme_scan: Option<Vec<(PathBuf, Option<SystemTime>, u64)>>,
    /// The currently open `.base` table (`Some` while a base file is the
    /// visible content; `None` renders the viewer's note). See
    /// `baseview.rs`.
    pub base: Option<BaseState>,
    /// Transient info message for the status bar (`STATUS_MESSAGE_TTL`).
    /// Shown only while no error-ish message (the real error, a `.base`
    /// warning or the walk warning) outranks it.
    pub last_status: Option<StatusMessage>,
    /// The vault's unreadable-path warning (a directory the walk cannot
    /// read, or a file the index could not read), rendered on the status
    /// bar after the real error and the `.base` warnings. It is persistent
    /// state, not an event: it tracks the condition and clears itself when
    /// the condition ends, so it never replaces a newer error the user
    /// still needs. `None` = every path readable.
    pub walk_warning: Option<String>,
    /// A clipboard copy requested by a click, waiting for the event loop
    /// to write its OSC 52 sequence to the terminal (kept out of the
    /// render buffer; see `main.rs`).
    pub pending_clipboard: Option<String>,
}

/// Whether an index warning about a file still stands. The index records
/// one only when it could not read the path; a path that vanished names no
/// skipped note any more. A file is not walked, and a permission fix
/// changes no stamp, so the read the index would do is the test.
fn file_warning_still_stands(warning: &vault::IndexWarning) -> bool {
    warning.path.exists() && std::fs::read_to_string(&warning.path).is_err()
}

impl App {
    pub fn new(
        viewer: Viewer,
        vault: Option<PathBuf>,
        index: Option<SharedIndex>,
    ) -> io::Result<Self> {
        let sidebar = match vault {
            Some(root) => Some(Sidebar::new(&root)?),
            None => None,
        };
        let has_vault = sidebar.is_some();
        let mut viewer = viewer;
        viewer.set_index(index.clone());
        let watcher = index.as_ref().map(|i| VaultWatcher::new(&i.borrow()));
        let mut app = Self {
            viewer,
            sidebar,
            // With a vault the sidebar starts visible (M2 layout); without
            // one there is nothing to show.
            sidebar_visible: has_vault,
            sidebar_mode: SidebarMode::Tree,
            outline_prev: SidebarMode::Tree,
            outline: Outline::new(),
            tags: TagList::new(),
            index,
            focus: Focus::Content,
            nav: Nav::default(),
            backlinks_open: false,
            backlinks: Vec::new(),
            backlink_sel: 0,
            backlink_scroll: 0,
            backlink_viewport: 8,
            disambiguation: None,
            search: SearchState::default(),
            active_search: None,
            quick: crate::quick::QuickState::default(),
            git: None,
            watcher,
            mouse_capture: true,
            bar_drag: None,
            pointer: None,
            sidebar_width: None,
            layout: Layout::default(),
            last_rescan: None,
            last_vault_scan: None,
            last_git_refresh: None,
            git_job: None,
            git_rerun: false,
            // Default theme (ocean) until main applies the resolved one.
            theme_ui: crate::theme::ui_by_preset(engine::MarkdownThemePreset::default()),
            theme_choices: ThemeChoices::default(),
            theme_rotation: ThemeRotation::default(),
            note_margin: crate::config::DEFAULT_MARGIN,
            note_margin_y: crate::config::DEFAULT_MARGIN_Y,
            line_numbers: false,
            icons: crate::config::IconSet::default(),
            discovered_themes: BTreeMap::new(),
            config_watch: None,
            last_config_check: None,
            theme_dirs: Vec::new(),
            pending_theme: None,
            pending_theme_scan: None,
            base: None,
            last_status: None,
            walk_warning: None,
            pending_clipboard: None,
        };
        // The first frame is drawn before the first walk runs: show the
        // index's snapshot now, then let the walk tick take over.
        app.surface_startup_warnings();
        app.refresh_tags();
        app.refresh_backlinks();
        app.refresh_git(true);
        Ok(app)
    }

    /// Starts the read-only git status queries for the vault root on a
    /// worker thread; [`Self::poll_git`] (from `poll`) installs the result,
    /// so a slow `git status` on a large repository never stalls the UI.
    /// At most one query runs at a time; a request made meanwhile re-runs
    /// once when it finishes. (The write guard, `git_write_guard`, stays
    /// synchronous on purpose: it must see the exact current state.)
    ///
    /// On demand only: startup (`force`), the `u` key (`force`), every
    /// successful note open (SPEC: status is "refreshed on open, watch
    /// events, and `u`"), and watch-triggered reindexes — the latter two
    /// debounced: skipped when the last run is less than
    /// [`GIT_DEBOUNCE`] ago (opens included, so rapidly hopping between
    /// notes queries at most once per window). Never run continuously.
    /// Without a vault index there is nothing to ask about.
    pub fn refresh_git(&mut self, force: bool) {
        let Some(index) = &self.index else {
            return;
        };
        let now = Instant::now();
        if !force
            && self
                .last_git_refresh
                .is_some_and(|t| now.duration_since(t) < GIT_DEBOUNCE)
        {
            return;
        }
        self.last_git_refresh = Some(now);
        if self.git_job.is_some() {
            self.git_rerun = true;
            return;
        }
        let root = index.borrow().root().to_path_buf();
        self.spawn_git(root);
    }

    fn spawn_git(&mut self, root: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // The receiver may be gone (app quit): nothing to report to.
            let _ = tx.send(vault::git::read_status(&root));
        });
        self.git_job = Some(rx);
    }

    /// Installs a finished background git query, and starts the queued
    /// re-run if one was requested meanwhile. Called every `poll` tick.
    fn poll_git(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.git_job else {
            return;
        };
        match rx.try_recv() {
            Ok(status) => self.git = status,
            Err(TryRecvError::Empty) => return,
            // The worker died without a result: keep the last status.
            Err(TryRecvError::Disconnected) => {}
        }
        self.git_job = None;
        if std::mem::take(&mut self.git_rerun) {
            if let Some(index) = &self.index {
                let root = index.borrow().root().to_path_buf();
                self.spawn_git(root);
            }
        }
    }

    /// Test helper: blocks until no git query is in flight or queued.
    #[cfg(test)]
    pub fn wait_git(&mut self) {
        while let Some(rx) = self.git_job.take() {
            if let Ok(status) = rx.recv() {
                self.git = status;
            }
            if std::mem::take(&mut self.git_rerun) {
                if let Some(index) = &self.index {
                    let root = index.borrow().root().to_path_buf();
                    self.spawn_git(root);
                }
            }
        }
    }

    /// The displayed file's path relative to the vault root (the note's
    /// own path when there is no vault index, or it lies outside the
    /// root). Used by the status-bar name copy and the ambiguous-name
    /// label.
    pub fn current_relative_path(&self) -> PathBuf {
        let current = self.current_path();
        match &self.index {
            Some(index) => {
                let root = index.borrow().root().to_path_buf();
                current
                    .strip_prefix(&root)
                    .map(Path::to_path_buf)
                    .unwrap_or(current)
            }
            None => current,
        }
    }

    /// Click on the status-bar note name: copies the vault-relative path
    /// (OSC 52, written by the event loop) and reports it. A write
    /// failure later replaces this with an error.
    fn copy_current_relative_path(&mut self) {
        let rel = self.current_relative_path().display().to_string();
        self.pending_clipboard = Some(crate::clipboard::osc52_sequence(&rel));
        self.set_status(format!("copied: {rel}"));
    }

    /// Writes a pending OSC 52 copy to `out` (the terminal). On a write
    /// failure the status message is replaced by an error (`last_error`).
    pub fn flush_clipboard(&mut self, out: &mut impl io::Write) {
        let Some(sequence) = self.pending_clipboard.take() else {
            return;
        };
        if let Err(err) = out
            .write_all(sequence.as_bytes())
            .and_then(|()| out.flush())
        {
            self.last_status = None;
            self.viewer.last_error = Some(format!("copy failed: {err}"));
        }
    }

    // --- status message ---------------------------------------------------

    /// Shows an info message for [`STATUS_MESSAGE_TTL`] (neutral color).
    pub fn set_status(&mut self, text: impl Into<String>) {
        self.set_status_at(text, Instant::now());
    }

    /// [`Self::set_status`] with an explicit clock (tests pass fabricated
    /// instants instead of sleeping).
    pub fn set_status_at(&mut self, text: impl Into<String>, now: Instant) {
        self.last_status = Some(StatusMessage {
            text: text.into(),
            shown_at: now,
        });
    }

    /// Clears the info message once it has been shown for the TTL.
    /// Called from the poll tick with the current time.
    pub fn expire_status_at(&mut self, now: Instant) {
        if let Some(status) = &self.last_status {
            if now.saturating_duration_since(status.shown_at) >= STATUS_MESSAGE_TTL {
                self.last_status = None;
            }
        }
    }

    /// The live info message, if any (`None` once expired by the poll
    /// tick). Errors are not considered here: callers pick error first.
    pub fn live_status_at(&self, now: Instant) -> Option<&str> {
        self.last_status.as_ref().and_then(|status| {
            (now.saturating_duration_since(status.shown_at) < STATUS_MESSAGE_TTL)
                .then_some(status.text.as_str())
        })
    }

    // --- theme support ----------------------------------------------------

    /// Applies a theme: chrome palette immediately, markdown palette via a
    /// full engine reparse (instant re-render), syntect theme when the
    /// theme names one (unknown names fall back to the default with a
    /// warning). Warnings surface in the status bar — never silently.
    pub fn apply_theme(&mut self, theme: &Theme, warnings: &[String]) {
        self.theme_ui = theme.ui;
        self.viewer
            .set_link_colors(crate::links::LinkColors::from_ui(theme.ui));
        self.viewer.set_markdown_theme(theme.markdown);
        let syntax_ok = match &theme.syntax {
            Some(name) => self.viewer.set_syntax_theme(name),
            None => {
                self.viewer.reset_syntax_theme();
                true
            }
        };
        let mut messages: Vec<String> = warnings.to_vec();
        if let Some(name) = &theme.syntax {
            if !syntax_ok {
                messages.push(format!(
                    "unknown syntax theme \"{name}\", using default syntax colors"
                ));
            }
        }
        if messages.is_empty() {
            self.viewer.last_error = None;
        } else {
            self.viewer.last_error = Some(messages.join("; "));
        }
    }

    /// Boots theme cycling: presets + discovered custom themes, positioned
    /// on `name` when it is one of them. Returns the theme to start with,
    /// that theme's own warnings (unknown keys; unknown-name fallback —
    /// these surface in the status bar AND stderr at startup), and the
    /// load errors of *non-selected* custom theme files (stderr only — a
    /// theme that merely failed to load must not spam the status bar).
    /// `dirs` is remembered for mid-session re-discovery (the dirs
    /// themselves are fixed; the files inside them are not).
    pub fn install_theme_choices(
        &mut self,
        name: &str,
        dirs: &[PathBuf],
        custom: &BTreeMap<String, PathBuf>,
    ) -> (Theme, Vec<String>, Vec<String>) {
        let (mut choices, load_errors) = ThemeChoices::build(custom);
        let mut theme_warnings;
        let resolved = if choices.select(name) {
            let current = choices.current().clone();
            theme_warnings = current.warnings;
            current.theme
        } else {
            theme_warnings = Vec::new();
            theme_warnings.push(format!(
                "unknown theme \"{name}\", using default (\"{}\")",
                crate::theme::DEFAULT_THEME
            ));
            choices.select(crate::theme::DEFAULT_THEME);
            choices.current().theme.clone()
        };
        self.theme_choices = choices;
        self.theme_dirs = dirs.to_vec();
        self.discovered_themes = custom.clone();
        (resolved, theme_warnings, load_errors)
    }

    /// Re-scans the theme directories (captured at startup) and rebuilds
    /// the `Shift+T` cycle, keeping the current theme selected when it
    /// still exists (otherwise the default). Returns the load errors of
    /// unparsable custom theme files (the caller surfaces them; see
    /// `install_theme_choices`).
    fn rediscover_themes(&mut self) -> Vec<String> {
        let dirs = self.theme_dirs.clone();
        self.rediscover_themes_in(&dirs)
    }

    /// The injectable core of [`Self::rediscover_themes`] (tests pass
    /// fixture directories directly; production uses the dirs captured
    /// at startup).
    fn rediscover_themes_in(&mut self, dirs: &[PathBuf]) -> Vec<String> {
        let custom = crate::theme::discover_custom_themes(dirs);
        let current = self.theme_choices.current().theme.name.clone();
        let (mut choices, load_errors) = ThemeChoices::build(&custom);
        if !choices.select(&current) {
            choices.select(crate::theme::DEFAULT_THEME);
        }
        self.theme_choices = choices;
        self.discovered_themes = custom;
        load_errors
    }

    /// `Shift+T`: cycles to the next theme the rotation allows (presets,
    /// then built-ins, then discovered custom themes; wrapping) and applies
    /// it instantly. Session-only — never writes the config. The newly
    /// selected theme's warnings surface in the status bar. Wrapping past
    /// the last entry re-scans the theme dirs first, so custom themes added
    /// mid-session join the rotation (discovery parses every custom theme
    /// file eagerly inside `ThemeChoices::build` — a dir listing plus the
    /// parses, cheap at realistic theme counts).
    pub fn cycle_theme(&mut self) {
        let mut load_errors = Vec::new();
        if self.theme_choices.next_wraps(self.theme_rotation) {
            load_errors = self.rediscover_themes();
        }
        let choice = self.theme_choices.cycle(self.theme_rotation).clone();
        self.apply_theme(&choice.theme, &choice.warnings);
        self.set_status(format!("theme: {}", choice.theme.name));
        // Re-discovery load errors surface when the applied theme itself
        // had nothing to say (theme warnings win — same precedence as
        // startup, where load errors are stderr-only).
        if !load_errors.is_empty() && self.viewer.last_error.is_none() {
            self.viewer.last_error = Some(load_errors.join("; "));
        }
    }

    /// `Ctrl+T`: advances the `Shift+T` rotation (all → light → dark →
    /// all) for this session. Only the group changes: the theme on screen
    /// stays, and `Shift+T` is what moves to a theme in the group.
    /// Session-only, like `Shift+T`.
    pub fn rotate_theme_group(&mut self) {
        self.theme_rotation = self.theme_rotation.next();
        let count = self.theme_choices.count_in(self.theme_rotation);
        self.set_status(format!(
            "theme group: {} ({count} themes)",
            self.theme_rotation.name()
        ));
    }

    /// The active theme's name (used by the tests; also cheap status
    /// diagnostics).
    #[cfg(test)]
    pub fn theme_name(&self) -> &str {
        &self.theme_choices.current().theme.name
    }

    /// Arms config hot-reload (mtime polled on the 1 s watcher tick).
    pub fn watch_config(&mut self, path: PathBuf) {
        self.config_watch = Some(ConfigWatch::new(path));
        self.last_config_check = Some(Instant::now());
    }

    /// Rebuilds the tag list from the index (cheap; called on index
    /// changes and when switching the sidebar into tag mode).
    pub fn refresh_tags(&mut self) {
        if let Some(index) = &self.index {
            self.tags.refresh(&index.borrow());
        }
    }

    /// The outline's row count, from the one place that decides whether
    /// there are rows at all (see [`outline_rows`]).
    fn outline_len(&self) -> usize {
        outline_rows(&self.base, &self.viewer).len()
    }

    /// Enter on an outline row (or a click): scroll the note to the selected
    /// heading and hand the focus to the content pane. Empty lists and open
    /// `.base` tables have nothing to jump to — the rows come from
    /// [`outline_rows`], the one place that decides.
    fn jump_to_outline_heading(&mut self) {
        let Some(line) = outline_rows(&self.base, &self.viewer)
            .get(self.outline.selected)
            .map(|entry| entry.line as u32)
        else {
            return;
        };
        self.viewer.set_scroll(line);
        self.focus = Focus::Content;
    }

    /// Re-reads the backlinks of the open note from the index.
    pub fn refresh_backlinks(&mut self) {
        self.backlinks = match &self.index {
            Some(index) => index.borrow().backlinks_to(&self.viewer.path).to_vec(),
            None => Vec::new(),
        };
        self.backlink_sel = self
            .backlink_sel
            .min(self.backlinks.len().saturating_sub(1));
        self.clamp_backlink_scroll();
    }

    fn clamp_backlink_scroll(&mut self) {
        let len = self.backlinks.len();
        let viewport = self.backlink_viewport.max(1);
        self.backlink_scroll = self
            .backlink_scroll
            .min(len.saturating_sub(viewport.min(len)));
    }

    /// Called from `ui::draw` with the backlinks panel's inner height;
    /// keeps the selection visible and clamps the scroll.
    pub fn on_backlinks_viewport(&mut self, height: usize) {
        self.backlink_viewport = height.max(1);
        self.scroll_backlink_selection_into_view();
    }

    /// Selects a backlink row without opening it (the panel bar's position
    /// control) and keeps it visible. Out-of-range indices are ignored.
    fn select_backlink(&mut self, index: usize) {
        if index >= self.backlinks.len() {
            return;
        }
        self.backlink_sel = index;
        self.scroll_backlink_selection_into_view();
    }

    fn scroll_backlink_selection_into_view(&mut self) {
        if self.backlink_sel >= self.backlink_scroll + self.backlink_viewport {
            self.backlink_scroll = self.backlink_sel + 1 - self.backlink_viewport;
        } else if self.backlink_sel < self.backlink_scroll {
            self.backlink_scroll = self.backlink_sel;
        }
        self.clamp_backlink_scroll();
    }

    /// The currently visible file's path (the base file when a `.base`
    /// table is open, else the viewer's note).
    pub fn current_path(&self) -> PathBuf {
        match &self.base {
            Some(base) => base.path.clone(),
            None => self.viewer.path.clone(),
        }
    }

    /// The currently visible file's scroll offset (table rows for a base).
    pub fn current_scroll(&self) -> usize {
        match &self.base {
            Some(base) => base.scroll,
            None => self.viewer.scroll as usize,
        }
    }

    /// Opens a note, pushing the outgoing note (with its scroll position)
    /// onto the back stack. Re-opening the note that is already on screen
    /// does not touch the history. A `.base` path opens the table view
    /// instead (same history semantics).
    ///
    /// Per SPEC the git sync status is "refreshed on open": an external
    /// commit fires no watch event, so without this the `*` markers and
    /// the `●` badge stay stale until `u`. The refresh goes through the
    /// normal debounced path (a spawn at most once per [`GIT_DEBOUNCE`]),
    /// so rapid note-hopping cannot hammer git.
    pub fn open_note(&mut self, path: &Path) -> io::Result<()> {
        if crate::baseview::is_base_file(path) {
            return self.open_base(path);
        }
        if self.base.is_none() && path == self.viewer.path {
            return Ok(());
        }
        let outgoing = (self.current_path(), self.current_scroll() as u32);
        if self.base.is_some() {
            // Leaving the base table for a note: the viewer (which kept
            // its previous note) is re-opened explicitly. The open is
            // fallible — the table stays up when it fails (the note side
            // of a failed open keeps the old note visible too).
            self.viewer.open_file(path)?;
            self.base = None;
        } else {
            self.viewer.open_file(path)?;
        }
        self.nav.push(outgoing);
        // Reveal the opened note in the sidebar tree: expand its ancestor
        // folders and select it (no focus change). Link follows, search
        // hits, backlinks and session restores all land here; sidebar
        // opens re-select the row they just clicked — a harmless no-op.
        // A `.base` table only expands its parent folder (no tree row).
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.reveal(path);
        }
        self.refresh_backlinks();
        // The outline lists the note in the content pane: keep its cursor
        // valid against the new heading table right away (the next draw
        // would sync it too, but a key can arrive before that).
        self.outline.sync(self.outline_len());
        // Debounce-gated ("rate-limited trivially"): skipped entirely when
        // the last spawn is recent — the status cannot have gone stale in
        // that window through anything this app did not already see.
        self.refresh_git(false);
        Ok(())
    }

    /// Opens a `.base` file's table view (history push like a note open).
    pub fn open_base(&mut self, path: &Path) -> io::Result<()> {
        if self.base.as_ref().is_some_and(|b| b.path == path) {
            return Ok(());
        }
        let outgoing = (self.current_path(), self.current_scroll() as u32);
        self.open_base_at(path, 0)?;
        self.nav.push(outgoing);
        // A `.base` file has no tree row: reveal just expands its parent
        // folder (see `Sidebar::reveal`).
        if let Some(sidebar) = &mut self.sidebar {
            sidebar.reveal(path);
        }
        Ok(())
    }

    /// The push-free core of [`Self::open_base`] (startup and history
    /// restoration). Restores `scroll` (clamped to the table).
    pub fn open_base_at(&mut self, path: &Path, scroll: usize) -> io::Result<()> {
        let index = self.index.as_ref().map(|i| i.borrow());
        let mut state = BaseState::open(path, index.as_deref(), &crate::baseview::today())?;
        // Restored scroll is clamped by the next render (the viewport is
        // only known once the frame is measured); no clamp here.
        state.scroll = scroll;
        self.base = Some(state);
        // The note is hidden behind the table: its link hover cannot stay lit.
        self.viewer.clear_hover_link();
        Ok(())
    }

    /// Opens the target of a resolved wikilink, scrolling to its heading
    /// anchor when the link carried one (best-effort: unknown headings
    /// leave the note at the top). A `.base` target opens the table view
    /// instead — bases have no headings, so the scroll is skipped.
    fn open_wikilink_target(&mut self, path: &Path, heading: Option<&str>) {
        if let Err(err) = self.open_note(path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        if let Some(heading) = heading {
            if self.base.is_none() {
                self.viewer.scroll_to_heading(heading);
            }
        }
    }

    /// Activates the currently focused link (`x`/`X` navigation + Enter,
    /// or a mouse click). Unresolved links surface an error, ambiguous
    /// ones (no hint applied) open the disambiguation popup.
    pub fn activate_focused_link(&mut self) {
        let Some(link) = self.viewer.focused_link().cloned() else {
            return;
        };
        self.activate_rendered_link(&link);
    }

    /// Enter on the content pane: activates the focused item — a link
    /// opens it, a checkbox cycles it through the guarded toggle.
    /// Inert while a `.base` table is displayed (the note is invisible;
    /// belt-and-braces alongside the key gate so no toggle path can act
    /// on the hidden note).
    pub fn activate_focused(&mut self) {
        if self.base.is_some() {
            return;
        }
        match self.viewer.focused_item() {
            Some(Focusable::Task(index)) => self.toggle_task(index),
            Some(Focusable::Link(_)) => self.activate_focused_link(),
            None => {}
        }
    }

    /// Cycles the checkbox `index` through `- [ ]` → `- [~]` → `- [x]` →
    /// `- [ ]` (the app's one write feature, guarded — every precondition
    /// failure surfaces a status-bar error and touches nothing):
    ///
    /// 1. the on-disk mtime+size must still match what was parsed (no
    ///    racing external edit);
    /// 2. the repository that directly contains the note must be clean
    ///    and its branch neither ahead nor behind — syncing is the
    ///    user's job, the viewer never commits/pulls. The guard follows
    ///    the note's own repo (`repo_toplevel` from the note's
    ///    directory), so a note inside a nested repository is guarded by
    ///    that repo, not the vault root's; a note not inside any
    ///    repository has no git state to protect and skips the guard;
    /// 3. the write itself re-scans the exact source line and refuses on
    ///    a state mismatch, then replaces the marker state byte and bumps
    ///    the frontmatter `updated:` field (only when present) atomically.
    ///
    /// The existing watch/reload + reindex pipeline picks the change up
    /// on the next poll; the git badge is refreshed immediately.
    pub fn toggle_task(&mut self, index: usize) {
        let Some(task) = self.viewer.tasks.get(index).copied() else {
            return;
        };
        let path = self.viewer.path.clone();
        if !self.viewer.disk_matches_parse() {
            self.viewer.last_error =
                Some("not writing: file changed since it was loaded".to_string());
            return;
        }
        if let Err(message) = self.git_write_guard(&path) {
            self.viewer.last_error = Some(message);
            return;
        }
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        match vault::tasks::write_toggle(&path, task.raw_line, task.state, &today) {
            Ok(report) => {
                self.viewer.last_error = None;
                if !report.updated_bumped {
                    // Vault convention is that every note carries
                    // `updated:`; when it is missing the marker toggle
                    // still stands — record it once, quietly, through the
                    // neutral expiring info channel (not the red error).
                    self.set_status("toggled (no updated: field in frontmatter to bump)");
                }
                self.refresh_git(true);
            }
            Err(err) => self.viewer.last_error = Some(format!("not writing: {err}")),
        }
    }

    /// The git precondition of a guarded write: the note's *own*
    /// repository (nested repos included — the vault root's repo is
    /// irrelevant when the note lives in one nested inside it) must have
    /// a readable, clean, fully synced status; a note not inside any
    /// repository has no git state to protect. A status that cannot be
    /// read is a refusal, not a silent pass.
    fn git_write_guard(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        match vault::git::repo_toplevel(dir) {
            None => Ok(()), // not inside any repository: nothing to protect
            Some(toplevel) => match vault::git::read_status(&toplevel) {
                None => Err("not writing: git status unavailable".to_string()),
                Some(status) if !status.clean => Err("not writing: dirty git tree".to_string()),
                Some(status) if status.ahead > 0 => {
                    Err("not writing: branch ahead of upstream".to_string())
                }
                Some(status) if status.behind > 0 => {
                    Err("not writing: branch behind upstream".to_string())
                }
                Some(_) => Ok(()),
            },
        }
    }

    fn activate_rendered_link(&mut self, link: &RenderedLink) {
        let Some(wikilink) = &link.wikilink else {
            self.viewer.last_error = Some(format!("external link: {} (not supported)", link.url));
            return;
        };
        match &wikilink.state {
            LinkState::NotFound => {
                self.viewer.last_error = Some(format!("unresolved link: {}", wikilink.target));
            }
            LinkState::Ambiguous(candidates) => {
                self.disambiguation = Some(Disambiguation {
                    candidates: candidates.clone(),
                    selected: 0,
                    heading: wikilink.heading.clone(),
                });
                // The popup owns the pointer now: no link behind it may stay
                // highlighted.
                self.viewer.clear_hover_link();
            }
            LinkState::Resolved(path) => {
                self.open_wikilink_target(path, wikilink.heading.as_deref());
            }
        }
    }

    /// True while a modal popup owns input (help, disambiguation). The
    /// search popup is position-routed instead (its result rows are
    /// click-selectable); used by both click and wheel gating so their
    /// behavior stays aligned.
    fn any_popup_open(&self) -> bool {
        self.disambiguation.is_some() || self.viewer.help_open
    }

    /// Maps a row of the note pane's scrollbar track onto a scroll
    /// position: the track's first row is the document's top, its last row
    /// the bottom, and the rows between interpolate linearly over the
    /// scroll range. Rows above/below the track clamp to the ends (a drag
    /// can leave the column). `None` while no bar is drawn.
    fn bar_scroll_at(&self, row: u16) -> Option<u32> {
        let area = self.layout.note_scrollbar_area?;
        Some(bar_value_at(area, row, self.viewer.max_scroll() as usize) as u32)
    }

    /// Maps a row of the `.base` table's scrollbar track onto a document
    /// position, with the same linear mapping as the note pane's bar.
    /// `None` while no table (or no bar) is drawn.
    fn base_bar_scroll_at(&self, row: u16) -> Option<usize> {
        let area = self.layout.base_scrollbar_area?;
        let base = self.base.as_ref()?;
        Some(bar_value_at(area, row, base.max_scroll()))
    }

    /// Moves the shown sidebar list's selection to the row the panel's bar
    /// names (the tree, the tag list or the outline). Selecting never
    /// activates: a bar press does not open a note, toggle a folder or jump
    /// to an outline heading.
    fn select_sidebar_bar_row(&mut self, area: Rect, row: u16) {
        match self.sidebar_mode {
            SidebarMode::Tree => {
                let Some(sidebar) = &mut self.sidebar else {
                    return;
                };
                if let Some(index) = bar_list_index(area, row, sidebar.row_count()) {
                    sidebar.select(index);
                }
            }
            SidebarMode::Tags => {
                if let Some(index) = bar_list_index(area, row, self.tags.rows().len()) {
                    self.tags.select(index);
                }
            }
            SidebarMode::Outline => {
                let len = self.outline_len();
                if let Some(index) = bar_list_index(area, row, len) {
                    self.outline.select(index, len);
                }
            }
        }
    }

    /// Moves the backlinks panel's selection to the row its bar names.
    /// Selecting never opens the link.
    fn select_backlink_bar_row(&mut self, area: Rect, row: u16) {
        if let Some(index) = bar_list_index(area, row, self.backlinks.len()) {
            self.select_backlink(index);
        }
    }

    /// Remembers where the pointer is. Every mouse event carries its cell
    /// (the terminal's any-motion mode reports moves with no button held),
    /// and the bars' hover highlight is decided from this in
    /// `ui::draw_scrollbar`. With mouse capture off no events arrive, and
    /// the cell is dropped rather than kept (see
    /// [`Self::toggle_mouse_capture`]).
    pub fn on_mouse_position(&mut self, col: u16, row: u16) {
        if !self.mouse_capture {
            return;
        }
        self.pointer = Some(Position::new(col, row));
        self.sync_link_hover();
    }

    /// Maps a screen cell to the note's rendered `(line, column)`. One
    /// home for the hover lookup and the click hit-test: the hover names
    /// the link a click would open, so the two mappings must never drift.
    fn note_cell_at(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.layout.note_viewport;
        if !point_in_rect(col, row, area) {
            return None;
        }
        let line = self.viewer.scroll as usize + usize::from(row - area.y);
        let col = usize::from(col - area.x);
        Some((line, col))
    }

    /// The link under the pointer, mapped through the last frame's note
    /// viewport and the viewer's scroll. `None` outside the note pane,
    /// while a popup owns the pointer, while a `.base` table covers the
    /// note, and while mouse capture is off.
    fn hovered_link_at_pointer(&self) -> Option<usize> {
        if !self.mouse_capture || self.base.is_some() {
            return None;
        }
        if self.quick.open || self.search.open || self.any_popup_open() {
            return None;
        }
        let pointer = self.pointer?;
        let (line, col) = self.note_cell_at(pointer.x, pointer.y)?;
        self.viewer.link_at(line, col)
    }

    /// Pushes the link under the pointer into the viewer, which restyles
    /// only when the hovered link changes. Called on every pointer event
    /// and again from each frame draw, so a pointer that stays still is
    /// still true of what is under it after the state around it moves (a
    /// scroll, a popup closing).
    pub(crate) fn sync_link_hover(&mut self) {
        let hovered = self.hovered_link_at_pointer();
        self.viewer.set_hover_link(hovered);
    }

    /// Left-button press. On any scrollbar column (the note pane, the
    /// `.base` table, the sidebar panel, the backlinks panel) it jumps that
    /// pane to the mapped position and starts a drag: the bars are position
    /// controls, so the press never moves the keyboard focus, and a list
    /// bar's press moves the selection without opening it. Any other press
    /// is inert: clicks keep their release semantics, and popups keep
    /// owning the pointer while open. With mouse capture off the app ignores
    /// the mouse entirely, independent of what the terminal forwards.
    pub fn handle_press(&mut self, col: u16, row: u16) {
        // A new press supersedes any drag state a lost release left behind:
        // whatever this press starts (or does not), the next release is
        // judged on it, not on a stale drag.
        self.bar_drag = None;
        if !self.mouse_capture {
            return;
        }
        if self.quick.open || self.search.open || self.any_popup_open() {
            return;
        }
        if let Some(area) = self.layout.note_scrollbar_area {
            if point_in_rect(col, row, area) {
                self.bar_drag = Some(BarTarget::Note);
                if let Some(scroll) = self.bar_scroll_at(row) {
                    self.viewer.set_scroll(scroll);
                }
                return;
            }
        }
        if let Some(area) = self.layout.base_scrollbar_area {
            if point_in_rect(col, row, area) {
                self.bar_drag = Some(BarTarget::Base);
                if let Some(scroll) = self.base_bar_scroll_at(row) {
                    if let Some(base) = &mut self.base {
                        base.set_scroll(scroll);
                    }
                }
                return;
            }
        }
        if self.sidebar_visible {
            if let Some(area) = self.layout.sidebar_scrollbar_area {
                if point_in_rect(col, row, area) {
                    self.bar_drag = Some(BarTarget::Sidebar);
                    self.select_sidebar_bar_row(area, row);
                    return;
                }
            }
        }
        if let Some(area) = self.layout.backlinks_scrollbar_area {
            if point_in_rect(col, row, area) {
                self.bar_drag = Some(BarTarget::Backlinks);
                self.select_backlink_bar_row(area, row);
            }
        }
    }

    /// Left-button drag: while a bar drag is active its pane follows the
    /// pointer's row (the column may leave the bar; only the row matters).
    /// A drag that did not start on a bar does nothing, and a popup that
    /// opened mid-drag owns the pointer: the pane behind it must not keep
    /// moving (the release still ends the drag).
    pub fn handle_drag(&mut self, _col: u16, row: u16) {
        let Some(target) = self.bar_drag else {
            return;
        };
        if !self.mouse_capture {
            return;
        }
        if self.quick.open || self.search.open || self.any_popup_open() {
            return;
        }
        match target {
            BarTarget::Note => {
                if let Some(scroll) = self.bar_scroll_at(row) {
                    self.viewer.set_scroll(scroll);
                }
            }
            BarTarget::Base => {
                if let Some(scroll) = self.base_bar_scroll_at(row) {
                    if let Some(base) = &mut self.base {
                        base.set_scroll(scroll);
                    }
                }
            }
            BarTarget::Sidebar => {
                if let Some(area) = self.layout.sidebar_scrollbar_area {
                    self.select_sidebar_bar_row(area, row);
                }
            }
            BarTarget::Backlinks => {
                if let Some(area) = self.layout.backlinks_scrollbar_area {
                    self.select_backlink_bar_row(area, row);
                }
            }
        }
    }

    /// Left-button release. The release that ends a bar drag is consumed:
    /// it clears the drag and never acts as a click, so a link, note row or
    /// folder under the pointer does not open. Every other release keeps
    /// the click semantics (including a bar column, where a click is a
    /// no-op because the bar is not a row target).
    pub fn handle_release(&mut self, col: u16, row: u16) {
        if self.bar_drag.is_some() {
            self.bar_drag = None;
            return;
        }
        self.handle_click(col, row);
    }

    /// Mouse click. Pane focus rule (documented): a click on a pane moves
    /// keyboard focus (`Focus`) to that pane — sidebar → `Sidebar`, the
    /// note or base-table content → `Content`, the backlinks panel →
    /// `Backlinks`. The pane focus is assigned **after** the click's
    /// action runs, with one exception: a click that *opens a note or
    /// base table* (a sidebar note row, or a backlink row) hands focus to
    /// `Content` (the reader context moves with the open; folder toggles
    /// keep `Sidebar`, a backlink click on empty space keeps
    /// `Backlinks`). The
    /// clicked-pane-wins rule otherwise holds: an action that does not
    /// open anything (folder toggle, checkbox) cannot leave a different
    /// pane focused. Item-level focus (the focused link/checkbox used by
    /// `x`/`X` + Enter) is still set by link/checkbox clicks exactly like
    /// [`Self::activate_focused_link`]. Wheel events are position-routed
    /// instead ([`Self::handle_wheel_at`]) and do not move pane focus.
    ///
    /// The event loop reaches this through [`Self::handle_release`], never
    /// directly on the button-up event: that wrapper consumes the release
    /// which ends a scrollbar drag.
    ///
    /// Routing: the search popup (when open) owns clicks inside its
    /// rectangle; while the help/disambiguation popups are open clicks
    /// are ignored entirely — those popups are modal and a click must
    /// never act on what is behind them (focus unchanged). While the
    /// sidebar is visible its column owns the click (no click-through
    /// into the note viewport, which shares the same screen rows).
    pub fn handle_click(&mut self, col: u16, row: u16) {
        // The quick switcher is position-routed (result rows are
        // click-selectable); clicks elsewhere stay inert while it is open.
        if self.quick.open {
            self.quick_click(col, row);
            return;
        }
        // The search popup is position-routed (result rows are
        // click-selectable); clicks elsewhere stay inert while it is open.
        if self.search.open {
            self.search_click(col, row);
            return;
        }
        if self.any_popup_open() {
            return;
        }
        // A bar column is a position control, not a row target: a bare
        // release there (a press/drag pair is consumed by
        // `handle_release`) must not open the row under the pointer or
        // move focus. The backlinks click rect already stops before its
        // bar; the sidebar's covers the whole panel, and the `.base`
        // table's viewport includes its bar.
        if self.sidebar_visible
            && self
                .layout
                .sidebar_scrollbar_area
                .is_some_and(|area| point_in_rect(col, row, area))
        {
            return;
        }
        if self
            .layout
            .backlinks_scrollbar_area
            .is_some_and(|area| point_in_rect(col, row, area))
        {
            return;
        }
        if self
            .layout
            .base_scrollbar_area
            .is_some_and(|area| point_in_rect(col, row, area))
        {
            return;
        }
        if let Some(area) = self.layout.status_name_area {
            if point_in_rect(col, row, area) {
                self.copy_current_relative_path();
                return;
            }
        }
        if self.sidebar_visible {
            if let Some(area) = self.layout.sidebar_area {
                if point_in_rect(col, row, area) {
                    let opened = self.sidebar_click(row, area);
                    // Clicked-pane-wins, except that an open moves the
                    // reader's context to the content pane (matching
                    // Enter); a folder toggle or no-op keeps `Sidebar`.
                    if !opened {
                        self.focus = Focus::Sidebar;
                    }
                    return;
                }
            }
        }
        if let Some(area) = self.layout.backlinks_area {
            if point_in_rect(col, row, area) {
                self.focus = Focus::Backlinks;
                // A click on a listed row selects it and opens it (SPEC:
                // "click/Enter opens"); the row mapping mirrors the
                // sidebar's (border rows and empty space are focus-only).
                if let Some(index) = crate::sidebar::list_row_at(
                    row,
                    area,
                    self.backlink_scroll,
                    self.backlinks.len(),
                ) {
                    self.backlink_sel = index;
                    self.open_from_backlinks();
                }
                return;
            }
        }
        if self.base.is_some() {
            self.base_click(col, row);
            self.focus = Focus::Content;
            return;
        }
        let Some((line, col)) = self.note_cell_at(col, row) else {
            return;
        };
        self.focus = Focus::Content;
        // Checkboxes first: the glyph is not a link region, but a task
        // line can also contain links — the glyph columns are matched
        // before the link regions so the click target is unambiguous.
        if let Some(task) = self.viewer.task_at(line, col) {
            self.viewer.set_focus_task(Some(task));
            self.toggle_task(task);
            self.focus = Focus::Content;
            return;
        }
        let index = self.viewer.link_at(line, col);
        if let Some(index) = index {
            self.viewer.set_focus_link(Some(index));
            self.activate_focused_link();
            self.focus = Focus::Content;
        }
    }

    /// A click inside the open base table: maps the screen position to a
    /// table line + column through the last render's geometry (the same
    /// math `ui::draw` used: 1-cell border ring around the inner rows),
    /// and opens a clicked link cell through the normal nav-stack path.
    fn base_click(&mut self, col: u16, row: u16) {
        let Some(base) = &self.base else {
            return;
        };
        let area = self.layout.note_viewport;
        if area.width < 3
            || area.height < 3
            || col < area.x + 1
            || col >= area.x + area.width - 1
            || row < area.y + 1
            || row >= area.y + area.height - 1
        {
            return; // border rows/columns: no cell lives there
        }
        let line = base.scroll + (row - area.y - 1) as usize;
        let col = (col - area.x - 1) as usize;
        let target = base.link_at(line, col);
        if let Some(path) = target {
            if let Err(err) = self.open_note(&path) {
                self.viewer.last_error = Some(format!("open failed: {err}"));
            }
        }
    }

    /// A click inside the quick switcher: a click on a result row selects
    /// it, and clicking the already-selected row opens it (the keyboard's
    /// Enter semantics). The prompt row, the footer and clicks outside the
    /// popup are no-ops.
    fn quick_click(&mut self, col: u16, row: u16) {
        let Some(popup) = self.layout.quick_area else {
            return;
        };
        if !point_in_rect(col, row, popup) {
            return;
        }
        let list = crate::quick::CHROME.list(popup, self.quick.selected, self.quick.items.len());
        let Some(index) = list.index_at(row) else {
            return;
        };
        if index == self.quick.selected {
            self.open_quick_selection();
        } else {
            self.quick.selected = index;
        }
    }

    /// A click inside the search popup: in results mode a click selects
    /// the hit row, and clicking the already-selected row opens it (the
    /// keyboard's Enter semantics). Border/footer/empty rows and input
    /// mode are no-ops; clicks outside the popup do nothing at all.
    fn search_click(&mut self, col: u16, row: u16) {
        let Some(popup) = self.layout.search_area else {
            return;
        };
        if !point_in_rect(col, row, popup) {
            return;
        }
        if self.search.mode != SearchMode::Results {
            return;
        }
        let list =
            crate::search::RESULTS_CHROME.list(popup, self.search.selected, self.search.hits.len());
        let Some(index) = list.index_at(row) else {
            return;
        };
        if index == self.search.selected {
            self.open_search_hit();
        } else {
            self.search.selected = index;
        }
    }

    /// A click inside the sidebar widget: maps the screen row to a list
    /// row (border rows and past-the-end clicks are no-ops), selects it,
    /// toggles folders/tags, and opens notes via the normal `open_note`
    /// path (nav push, scroll reset, error surfacing). Returns whether a
    /// note/base table was opened (and focus moved to `Content`).
    fn sidebar_click(&mut self, row: u16, area: Rect) -> bool {
        match self.sidebar_mode {
            SidebarMode::Tree => {
                let clicked = self.sidebar.as_mut().and_then(|sidebar| {
                    let index = sidebar.row_index_at_screen_row(area, row)?;
                    sidebar.click(index)
                });
                self.open_sidebar_path(clicked)
            }
            SidebarMode::Tags => {
                let clicked = {
                    let scroll = self.tags.scroll;
                    let len = self.tags.rows().len();
                    crate::sidebar::list_row_at(row, area, scroll, len)
                        .and_then(|index| self.tags.click(index))
                };
                self.open_sidebar_path(clicked)
            }
            SidebarMode::Outline => {
                let len = self.outline_len();
                let Some(index) = crate::sidebar::list_row_at(row, area, self.outline.scroll, len)
                else {
                    return false; // border row or past the last heading
                };
                self.outline.select(index, len);
                self.jump_to_outline_heading();
                // A row click jumps like Enter, so the reader's context
                // moves to the content pane (handle_click only keeps the
                // sidebar focused when this returns false).
                self.focus == Focus::Content
            }
        }
    }

    /// Opens a sidebar-reported note path, surfacing failures like the
    /// Enter path does; `None` (folder toggle, border click) is a no-op.
    /// On success the focus moves to the content pane, matching Enter.
    fn open_sidebar_path(&mut self, path: Option<PathBuf>) -> bool {
        let Some(path) = path else {
            return false;
        };
        if let Err(err) = self.open_note(&path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return false;
        }
        self.focus = Focus::Content;
        true
    }

    /// `Backspace`: restore the previous note (or `.base` table) with its
    /// scroll position.
    ///
    /// Peek-then-commit: the stacks are mutated only *after* the open
    /// succeeds, so a deleted/renamed history target surfaces an error
    /// and leaves the history fully intact.
    pub fn go_back(&mut self) -> io::Result<()> {
        if !self.nav.can_go_back() {
            return Ok(());
        }
        let Some(entry) = self.nav.peek_back().cloned() else {
            return Ok(());
        };
        let current = (self.current_path(), self.current_scroll() as u32);
        self.open_history_entry(&entry.0, entry.1)?; // on failure: stacks untouched
        if let Some((path, scroll)) = self.nav.pop_back(current) {
            debug_assert_eq!(path, entry.0);
            if !crate::baseview::is_base_file(&path) {
                self.viewer.set_scroll(scroll);
            }
        }
        self.refresh_backlinks();
        Ok(())
    }

    /// `Ctrl+O`: the mirror image of `go_back` (same peek-then-commit).
    pub fn go_forward(&mut self) -> io::Result<()> {
        if !self.nav.can_go_forward() {
            return Ok(());
        }
        let Some(entry) = self.nav.peek_forward().cloned() else {
            return Ok(());
        };
        let current = (self.current_path(), self.current_scroll() as u32);
        self.open_history_entry(&entry.0, entry.1)?; // on failure: stacks untouched
        if let Some((path, scroll)) = self.nav.pop_forward(current) {
            debug_assert_eq!(path, entry.0);
            if !crate::baseview::is_base_file(&path) {
                self.viewer.set_scroll(scroll);
            }
        }
        self.refresh_backlinks();
        Ok(())
    }

    /// Opens a history entry: a note through the viewer, a `.base`
    /// through the table state (scroll restored, clamped). The base
    /// table is cleared only after the note open succeeds, so a failed
    /// open keeps the currently displayed table.
    fn open_history_entry(&mut self, path: &Path, scroll: u32) -> io::Result<()> {
        if crate::baseview::is_base_file(path) {
            self.open_base_at(path, scroll as usize)
        } else {
            self.viewer.open_file(path)?;
            self.base = None;
            Ok(())
        }
    }

    /// Watch tick: poll the current file, keep the index current (the
    /// file's own reindex runs immediately after a reload; the whole vault
    /// is rescanned every [`VAULT_SCAN_INTERVAL`]), then refresh dependent
    /// views (sidebar tree, tags, backlinks) and re-style the note if the
    /// index changed.
    pub fn poll(&mut self) {
        // Config hot-reload rides the same 1 s cadence as the vault scan.
        let now = Instant::now();
        self.expire_status_at(now);
        self.poll_git();
        if self
            .last_config_check
            .is_none_or(|t| now.duration_since(t) >= VAULT_SCAN_INTERVAL)
        {
            self.last_config_check = Some(now);
            self.poll_config();
        }
        let previous_reload = self.viewer.last_reload;
        self.viewer.poll_file();
        let current_reloaded = self.viewer.last_reload != previous_reload;

        let mut vault_changed = false;
        // `Some` only when a scan actually ran this tick: a poll between
        // scans must not look like "no warnings" and clear the condition.
        let mut walk_outcome = None;
        if let Some(index) = &self.index {
            if current_reloaded {
                index.borrow_mut().reindex_one(&self.viewer.path);
                vault_changed = true;
            }
            let now = Instant::now();
            if self
                .last_vault_scan
                .is_none_or(|t| now.duration_since(t) >= VAULT_SCAN_INTERVAL)
            {
                self.last_vault_scan = Some(now);
                if let Some(watcher) = &mut self.watcher {
                    walk_outcome = Some(watcher.poll(&mut index.borrow_mut()));
                }
            }
        }
        if let Some(outcome) = walk_outcome {
            vault_changed |= !outcome.changed.is_empty();
            vault_changed |= self.surface_walk_warnings(outcome.warnings.as_deref());
        }

        if vault_changed {
            // Link states and glyphs may have changed with the index.
            self.viewer.parse_source();
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.rescan();
            }
            self.refresh_tags();
            self.last_rescan = self.viewer.last_reload;
            // A vault change may also mean a git change; debounced (at
            // most one spawn per GIT_DEBOUNCE).
            self.refresh_git(false);
        }
        // The open `.base` table: its file is watched by stamp (it is not
        // an index entry — see baseview.rs), and its filters re-evaluate
        // against the live index on every vault change.
        let mut base_changed = false;
        if let Some(base) = &mut self.base {
            base_changed = base.poll_file();
        }
        if vault_changed || base_changed {
            self.refresh_base_table();
        }
        // Normally `vault_changed` above already rescanned for a reload of
        // the open note. This catches the degraded mode where the vault was
        // given but `VaultIndex::build` failed (no `index`, yet a sidebar),
        // so a reload of the open note still refreshes the tree.
        if self.viewer.last_reload != self.last_rescan {
            self.last_rescan = self.viewer.last_reload;
            if let Some(sidebar) = &mut self.sidebar {
                sidebar.rescan();
            }
        }
        if self.backlinks_open {
            self.refresh_backlinks();
        }
    }

    /// Records the vault's unreadable-path warning for the status bar: the
    /// warnings from the walk that just ran plus the index's own unreadable
    /// files. A directory is re-reported by every walk, so a walk that ran
    /// is authoritative for it; a file is not part of the walk, so the
    /// index's record of it is the report — from the startup build, or from
    /// any later reindex that could not read it. The same path from both
    /// sources is one message, not two.
    ///
    /// `walk` is `None` when the walk itself failed (the root could not be
    /// read). That is not an empty report: the walk could not look, and a
    /// root that cannot be walked leaves every path under it unreadable, so
    /// it is never evidence that a warning's condition ended. The slot is
    /// left exactly as it is until a walk can check again.
    ///
    /// This is state, not an event: an empty set clears the slot, and an
    /// unchanged set leaves it untouched. A file that now reads is
    /// reindexed before its warning drops, so the note returns with it; the
    /// return value reports whether that happened (the caller then refreshes
    /// the views that read the index). It deliberately does not go into
    /// `viewer.last_error` — a background condition must never replace a
    /// newer error (a refused write, a failed open) that still needs the
    /// user.
    fn surface_walk_warnings(&mut self, walk: Option<&[vault::IndexWarning]>) -> bool {
        let Some(walk) = walk else {
            return false;
        };
        // A file that reads again is healed, not merely silent: reindex it
        // so the note is back in the index when its warning goes. A
        // permission fix changes no stamp, so nothing else re-feeds it.
        let healed: Vec<PathBuf> = self
            .index_unreadable_files()
            .into_iter()
            .filter(|warning| !file_warning_still_stands(warning))
            .map(|warning| warning.path)
            .collect();
        let index_changed = !healed.is_empty();
        if let Some(index) = &self.index {
            let mut index = index.borrow_mut();
            for path in healed {
                index.reindex_one(&path);
            }
        }
        let mut warnings = walk.to_vec();
        for warning in self.index_unreadable_files() {
            if !warnings.iter().any(|current| current.path == warning.path) {
                warnings.push(warning);
            }
        }
        self.store_walk_warning(&warnings);
        index_changed
    }

    /// The index's own unreadable-path warnings: the paths the startup
    /// build skipped, and any a later reindex could not read.
    fn index_unreadable(&self) -> Vec<vault::IndexWarning> {
        self.index
            .as_ref()
            .map(|index| {
                index
                    .borrow()
                    .warnings()
                    .iter()
                    .filter(|warning| warning.is_unreadable())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// [`Self::index_unreadable`] minus directories: every walk re-reports
    /// a directory and is its authority, while the index records one only
    /// at build and never clears it when the directory reads again.
    fn index_unreadable_files(&self) -> Vec<vault::IndexWarning> {
        let mut warnings = self.index_unreadable();
        warnings.retain(|warning| !warning.path.is_dir());
        warnings
    }

    /// Shows the index's unreadable paths before the first walk has run.
    /// Directories are re-reported by that walk; files are kept from the
    /// index until `surface_walk_warnings` re-checks them.
    fn surface_startup_warnings(&mut self) {
        let warnings = self.index_unreadable();
        self.store_walk_warning(&warnings);
    }

    /// Formats one warning set for the sticky slot. Paths are relative to
    /// the vault root: the root is the same for every warning, so it carries
    /// no information the user needs.
    fn store_walk_warning(&mut self, warnings: &[vault::IndexWarning]) {
        if warnings.is_empty() {
            self.walk_warning = None;
            return;
        }
        let root = self.index.as_ref().map(|i| i.borrow().root().to_path_buf());
        let text = warnings
            .iter()
            .map(|w| {
                let path = root
                    .as_deref()
                    .and_then(|root| w.path.strip_prefix(root).ok())
                    .unwrap_or(w.path.as_path());
                format!("{}: {}", path.display(), w.message)
            })
            .collect::<Vec<_>>()
            .join("; ");
        if self.walk_warning.as_deref() != Some(text.as_str()) {
            self.walk_warning = Some(text);
        }
    }

    /// Rebuilds the open base table against the current index (filters
    /// re-evaluated). No-op without an open base or without an index.
    pub fn refresh_base_table(&mut self) {
        let Some(base) = &mut self.base else {
            return;
        };
        let index = self.index.as_ref().map(|i| i.borrow());
        base.rebuild(index.as_deref(), &crate::baseview::today());
    }

    /// `v` / `1`–`9` while a base table is open: switch views.
    pub fn base_cycle_view(&mut self) {
        let index = self.index.as_ref().map(|i| i.borrow());
        if let Some(base) = &mut self.base {
            base.cycle_view(index.as_deref(), &crate::baseview::today());
        }
    }

    pub fn base_set_view(&mut self, index: usize) {
        let vault_index = self.index.as_ref().map(|i| i.borrow());
        if let Some(base) = &mut self.base {
            base.set_view(index, vault_index.as_deref(), &crate::baseview::today());
        }
    }

    /// Enter on the sidebar (tree, tag or outline row); on read failure the
    /// error is surfaced in the status bar and the current note stays put
    /// (with the sidebar still focused). On success the focus moves to the
    /// content pane; the outline jumps to the selected heading instead.
    fn open_from_sidebar(&mut self) {
        let path = match self.sidebar_mode {
            SidebarMode::Tree => self.sidebar.as_mut().and_then(|sb| sb.activate()),
            SidebarMode::Tags => self.tags.activate(),
            SidebarMode::Outline => {
                self.jump_to_outline_heading();
                return;
            }
        };
        let Some(path) = path else {
            return;
        };
        if let Err(err) = self.open_note(&path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        // A note was opened: the reader's context moves to the content
        // pane, so the next j/k/PgDn/wheel scrolls the note, not the
        // sidebar the selection came from. Failed opens keep the sidebar
        // focused (the selection is still the object of attention).
        self.focus = Focus::Content;
    }

    /// Enter on the backlinks panel: open the selected backlink. On
    /// success the focus moves to the content pane, matching the sidebar
    /// open paths; a failed open keeps the panel focused.
    fn open_from_backlinks(&mut self) {
        let Some(path) = self.backlinks.get(self.backlink_sel).cloned() else {
            return;
        };
        if let Err(err) = self.open_note(&path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        self.focus = Focus::Content;
    }

    /// Config hot-reload tick: on an mtime change re-read the config and
    /// live-apply its `theme` key only (vault path changes apply on the
    /// next launch; `watch` stays a `w`-key session toggle). Malformed
    /// re-reads warn and keep the current theme. A theme the config names
    /// but that does not resolve stays pending and is re-checked on every
    /// tick (including a re-scan of the theme dirs), so a custom theme
    /// file added mid-session is picked up without a config change.
    /// A later config change that applies a *different* valid theme
    /// clears the pending name (the config's intent has moved on).
    fn poll_config(&mut self) {
        let Some(mut watch) = self.config_watch.take() else {
            return;
        };
        let poll = watch.poll();
        self.config_watch = Some(watch);
        if poll.changed {
            match &poll.theme {
                Some(name) => {
                    let custom = self.discovered_themes.clone();
                    match crate::theme::resolve_theme(name, &custom) {
                        Ok(resolved) => {
                            // The config now names a resolvable theme:
                            // any stale pending name is superseded —
                            // the config's intent is this theme, not
                            // the one a previous edit was waiting for.
                            self.pending_theme = None;
                            self.apply_resolved_theme(resolved, &poll.warnings);
                        }
                        Err(err) => {
                            // The config names a theme that does not
                            // resolve (yet): remember it — the retry
                            // below re-checks on every tick, so a custom
                            // theme file added mid-session is picked up
                            // without a config change.
                            self.pending_theme = Some(name.clone());
                            self.pending_theme_scan = None;
                            self.viewer.last_error = Some(format!("theme: {err}"));
                        }
                    }
                }
                None => {
                    // Malformed file (keep theme, warn) or the theme key
                    // removed (default applies): nothing pending either
                    // way. An unchanged theme key keeps any pending name.
                    if poll.theme_named.is_none() {
                        self.pending_theme = None;
                    }
                    if !poll.warnings.is_empty() {
                        self.viewer.last_error = Some(poll.warnings.join("; "));
                    }
                }
            }
        }
        // The config names a theme we could not resolve: re-check once
        // per tick. Resolution against the discovered map is cheap; the
        // expensive re-discovery (an eager parse of every theme file inside
        // `ThemeChoices::build`) runs only when a stat-only signature of
        // the theme dirs changed since the last attempt, so a typo in
        // `theme = "..."` does not cost a parse storm every second for the
        // rest of the session. Stops as soon as it resolves.
        if let Some(name) = self.pending_theme.clone() {
            let custom = self.discovered_themes.clone();
            match crate::theme::resolve_theme(&name, &custom) {
                Ok(resolved) => self.apply_resolved_theme(resolved, &[]),
                Err(_)
                    if {
                        let scan = theme_dirs_signature(&self.theme_dirs);
                        let unchanged = self.pending_theme_scan.as_ref() == Some(&scan);
                        self.pending_theme_scan = Some(scan);
                        unchanged
                    } => {}
                Err(_) => {
                    let load_errors = self.rediscover_themes();
                    let custom = self.discovered_themes.clone();
                    if let Ok(resolved) = crate::theme::resolve_theme(&name, &custom) {
                        self.apply_resolved_theme(resolved, &load_errors);
                        self.pending_theme = None;
                    }
                }
            }
        }
    }

    /// Applies a hot-reloaded theme, keeps the cycle positioned on it (by
    /// its canonical name — aliases like "solarized-dark" resolve to
    /// "solarized" — so Shift+T continues from it), and surfaces the
    /// config's unknown-key warnings / re-discovery load errors when the
    /// theme itself had nothing to say (same visibility as the
    /// unchanged-theme-key path).
    fn apply_resolved_theme(&mut self, resolved: crate::theme::ResolvedTheme, extra: &[String]) {
        self.apply_theme(&resolved.theme, &resolved.warnings);
        self.set_status(format!("theme: {}", resolved.theme.name));
        self.theme_choices.select(&resolved.theme.name);
        if self.viewer.last_error.is_none() && !extra.is_empty() {
            self.viewer.last_error = Some(extra.join("; "));
        }
    }

    /// Keys while the disambiguation popup is open: j/k move, Enter opens
    /// the chosen note, Ctrl+C quits, anything else cancels the popup.
    fn handle_disambiguation_key(&mut self, key: KeyEvent) -> Flow {
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        let Some(d) = &mut self.disambiguation else {
            return Flow::Continue;
        };
        let last = d.candidates.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down if plain => {
                d.selected = (d.selected + 1).min(last);
            }
            KeyCode::Char('k') | KeyCode::Up if plain => {
                d.selected = d.selected.saturating_sub(1);
            }
            KeyCode::Enter if plain => {
                if let Some(chosen) = self.disambiguation.take() {
                    self.open_wikilink_target(
                        &chosen.candidates[chosen.selected],
                        chosen.heading.as_deref(),
                    );
                }
            }
            // Ctrl+C quits even while the popup is open (mirrors the
            // main loop; everything else cancels the popup).
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
            _ => self.disambiguation = None,
        }
        Flow::Continue
    }

    /// Keys while the help popup is open: scrolling keys scroll it, every
    /// other key closes it.
    fn handle_help_key(&mut self, key: KeyEvent) -> Flow {
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        // Help scrolls with j/k/arrows, PgUp/PgDn, g/G; every other key
        // closes it (as before). Scroll positions are clamped at draw
        // time (`ui::draw_help`), so the step here can overshoot safely.
        let page = crate::ui::HELP_PAGE_STEP;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down if plain => {
                self.viewer.help_scroll = self.viewer.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up if plain => {
                self.viewer.help_scroll = self.viewer.help_scroll.saturating_sub(1);
            }
            KeyCode::PageDown if plain => {
                self.viewer.help_scroll = self.viewer.help_scroll.saturating_add(page);
            }
            KeyCode::PageUp if plain => {
                self.viewer.help_scroll = self.viewer.help_scroll.saturating_sub(page);
            }
            KeyCode::Char('g') | KeyCode::Home if plain => self.viewer.help_scroll = 0,
            KeyCode::Char('G') | KeyCode::End if plain => {
                self.viewer.help_scroll = usize::MAX;
            }
            _ => {
                self.viewer.help_open = false;
                self.viewer.help_scroll = 0;
            }
        }
        Flow::Continue
    }

    /// Handles one key press. The quick switcher (if open) gets the key
    /// first; then the search popup; then the disambiguation popup; then
    /// any key closes the help popup; `q` and Ctrl+C quit; everything
    /// else is focus-dependent.
    pub fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // "Plain" keys: no modifiers, or Shift only (Shift is how terminals
        // deliver uppercase letters like `G`).
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        if self.quick.open {
            return self.handle_quick_key(key);
        }
        if self.search.open {
            return self.handle_search_key(key);
        }
        if self.disambiguation.is_some() {
            return self.handle_disambiguation_key(key);
        }
        if self.viewer.help_open {
            return self.handle_help_key(key);
        }
        match key.code {
            KeyCode::Char('q') if plain => return Flow::Quit,
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
            KeyCode::Char('?') => {
                self.viewer.help_open = true;
                self.viewer.help_scroll = 0;
            }
            KeyCode::Tab if plain => self.toggle_sidebar(),
            KeyCode::Char('t') if plain && self.sidebar.is_some() => self.toggle_sidebar_mode(),
            KeyCode::Char('o') if plain && self.sidebar.is_some() => self.toggle_outline(),
            KeyCode::Char('m') if plain => self.toggle_mouse_capture(),
            KeyCode::Char('f') if plain => self.viewer.toggle_metadata(),
            KeyCode::Char('>') if plain => self.widen_sidebar(),
            KeyCode::Char('<') if plain => self.narrow_sidebar(),
            KeyCode::Char('b') if plain => self.toggle_backlinks(),
            KeyCode::Char('s') if plain && self.index.is_some() => self.search.open_input(),
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => self.open_quick(),
            KeyCode::Char('u') if plain => self.refresh_git(true),
            // While a `.base` table is displayed the viewer's note is
            // invisible: viewer-focus actions (x/X link focusing, Enter
            // activate — including the checkbox toggle write) must be
            // inert so they cannot act on the hidden note. Only table
            // interactions (v/1-9/scroll/clicks) act while a base is open.
            KeyCode::Char('x') if plain && self.focus == Focus::Content && self.base.is_none() => {
                self.viewer.focus_next_link();
            }
            KeyCode::Char('X') if plain && self.focus == Focus::Content && self.base.is_none() => {
                self.viewer.focus_prev_link();
            }
            KeyCode::Enter if plain && self.focus == Focus::Content && self.base.is_none() => {
                self.activate_focused()
            }
            KeyCode::Backspace if plain => {
                if let Err(err) = self.go_back() {
                    self.viewer.last_error = Some(format!("back failed: {err}"));
                }
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                if let Err(err) = self.go_forward() {
                    self.viewer.last_error = Some(format!("forward failed: {err}"));
                }
            }
            KeyCode::Char('h') | KeyCode::Left if plain => self.key_left(),
            KeyCode::Char('l') | KeyCode::Right if plain => self.key_right(),
            KeyCode::Char('j') | KeyCode::Down if plain => match self.focus {
                Focus::Sidebar => self.sidebar_move(1),
                Focus::Backlinks => self.backlink_move(1),
                Focus::Content => self.scroll_content_down(1),
            },
            KeyCode::Char('k') | KeyCode::Up if plain => match self.focus {
                Focus::Sidebar => self.sidebar_move(-1),
                Focus::Backlinks => self.backlink_move(-1),
                Focus::Content => self.scroll_content_up(1),
            },
            KeyCode::PageDown if plain && self.focus == Focus::Content => {
                self.scroll_content_down(self.viewer.page_step());
            }
            KeyCode::PageUp if plain && self.focus == Focus::Content => {
                self.scroll_content_up(self.viewer.page_step());
            }
            KeyCode::Char('g') | KeyCode::Home if plain && self.focus == Focus::Content => {
                self.scroll_content_to(false)
            }
            KeyCode::Char('G') | KeyCode::End if plain && self.focus == Focus::Content => {
                self.scroll_content_to(true)
            }
            // `n`/`N`: walk the active search's occurrences in the open
            // note. The modal popups above already took their keys, and a
            // `.base` table hides the note, so this is the note pane only.
            KeyCode::Char('n') if plain && self.focus == Focus::Content && self.base.is_none() => {
                self.goto_search_match(true);
            }
            KeyCode::Char('N') if plain && self.focus == Focus::Content && self.base.is_none() => {
                self.goto_search_match(false);
            }
            KeyCode::Char('w') if plain && self.focus == Focus::Content => {
                self.viewer.toggle_watch()
            }
            // `Shift+T`: theme cycling (session-only). `t` stays the
            // sidebar-mode toggle; Shift is how terminals deliver `T`.
            KeyCode::Char('T') if plain => self.cycle_theme(),
            KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
                self.rotate_theme_group()
            }
            // `.base` table view switching: `v` cycles, digits jump.
            KeyCode::Char('v') if plain && self.base.is_some() => self.base_cycle_view(),
            KeyCode::Char(c) if plain && self.base.is_some() && c.is_ascii_digit() && c != '0' => {
                self.base_set_view(c.to_digit(10).unwrap() as usize - 1);
            }
            KeyCode::Enter if plain && self.focus == Focus::Sidebar => self.open_from_sidebar(),
            KeyCode::Enter if plain && self.focus == Focus::Backlinks => self.open_from_backlinks(),
            _ => {}
        }
        // A key may have opened a popup (`?`, `s`, Ctrl+P): the popup owns
        // the pointer, so no link behind it may stay highlighted.
        if self.quick.open || self.search.open || self.any_popup_open() {
            self.viewer.clear_hover_link();
        }
        Flow::Continue
    }

    /// Mouse wheel, routed to the focused pane. While the search popup
    /// is open the wheel is position-routed by [`Self::handle_wheel_at`]
    /// (over the results list it scrolls the selection); without a
    /// position (this focus-based fallback) it must not act — the popup
    /// covers content it could otherwise scroll. Help/disambiguation
    /// popups are modal: the wheel is ignored while they are open.
    pub fn handle_wheel(&mut self, down: bool) {
        if self.search.open || self.any_popup_open() {
            return;
        }
        match self.focus {
            Focus::Sidebar => self.sidebar_move(if down { 3 } else { -3 }),
            Focus::Backlinks => self.backlink_move(if down { 3 } else { -3 }),
            Focus::Content if down => self.scroll_content_down(3),
            Focus::Content => self.scroll_content_up(3),
        }
    }

    /// Scrolls the content pane — the open `.base` table if any, else the
    /// note — down by `n` rows.
    fn scroll_content_down(&mut self, n: u32) {
        match &mut self.base {
            Some(base) => base.scroll_down(n as usize),
            None => self.viewer.scroll_down(n),
        }
    }

    /// Mirror of [`Self::scroll_content_down`].
    fn scroll_content_up(&mut self, n: u32) {
        match &mut self.base {
            Some(base) => base.scroll_up(n as usize),
            None => self.viewer.scroll_up(n),
        }
    }

    /// Jumps the content pane to its last (`bottom`) or first row.
    fn scroll_content_to(&mut self, bottom: bool) {
        match (&mut self.base, bottom) {
            (Some(base), true) => base.scroll_bottom(),
            (Some(base), false) => base.scroll_top(),
            (None, true) => self.viewer.scroll_bottom(),
            (None, false) => self.viewer.scroll_top(),
        }
    }

    /// `n` / `N`: moves the note pane to the next / previous occurrence of
    /// the active search in the open note, scrolling it into view.
    ///
    /// The position is the scroll, not the last match visited: an
    /// occurrence counts as "next" when jumping to it would scroll *below*
    /// where the reader is now (and "previous" when above), so the
    /// occurrence already on screen is never jumped to again. With none
    /// left in that direction the search wraps to the far occurrence. No
    /// active search, or no occurrence in this note, moves nothing.
    fn goto_search_match(&mut self, forward: bool) {
        let Some(active) = self.active_search.as_ref() else {
            return;
        };
        let occurrences = active.occurrences(&self.viewer.source);
        if occurrences.is_empty() {
            return;
        }
        // The landing spot of each occurrence, in document order, is
        // non-decreasing (the source-line map is monotonic), so the first
        // one past (or before) the current scroll is the nearest in that
        // direction.
        let target = if forward {
            occurrences
                .iter()
                .copied()
                .find(|&line| self.viewer.scroll_target_for_source_line(line) > self.viewer.scroll)
                .unwrap_or(occurrences[0])
        } else {
            occurrences
                .iter()
                .rev()
                .copied()
                .find(|&line| self.viewer.scroll_target_for_source_line(line) < self.viewer.scroll)
                .unwrap_or(occurrences[occurrences.len() - 1])
        };
        self.viewer.scroll_to_source_line(target);
    }

    /// Mouse wheel with the cursor position: a wheel over the visible
    /// sidebar scrolls the sidebar list regardless of where the keyboard
    /// focus sits; a wheel over the open search popup scrolls the result
    /// selection; anywhere else the focused pane scrolls (the M2
    /// behavior, kept via [`Self::handle_wheel`]). Help/disambiguation
    /// popups gate the wheel like clicks.
    pub fn handle_wheel_at(&mut self, col: u16, row: u16, down: bool) {
        if self.quick.open {
            if let Some(popup) = self.layout.quick_area {
                if point_in_rect(col, row, popup) {
                    self.quick.move_selection(if down { 3 } else { -3 });
                }
            }
            return;
        }
        if self.search.open {
            if let Some(popup) = self.layout.search_area {
                if point_in_rect(col, row, popup) {
                    self.search.move_selection(if down { 3 } else { -3 });
                }
            }
            return;
        }
        if self.any_popup_open() {
            return;
        }
        if self.sidebar_visible {
            if let Some(area) = self.layout.sidebar_area {
                if point_in_rect(col, row, area) {
                    self.sidebar_move(if down { 3 } else { -3 });
                    return;
                }
            }
        }
        self.handle_wheel(down);
    }

    /// `Ctrl+P`: opens the quick note switcher over the vault index. The
    /// popup lists every note (fuzzy-matched against the typed query) and
    /// `Enter` opens the selected one through the normal navigation stack.
    pub fn open_quick(&mut self) {
        let Some(index) = self.index.as_ref() else {
            return; // no vault: nothing to jump to
        };
        let candidates: Vec<crate::quick::QuickCandidate> = index
            .borrow()
            .notes()
            .map(crate::quick::QuickCandidate::from_note)
            .collect();
        self.quick.open(candidates);
    }

    /// Keys while the quick switcher is open (modal: typed characters go
    /// into the query, never into the app). Ctrl+C still quits.
    fn handle_quick_key(&mut self, key: KeyEvent) -> Flow {
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
            KeyCode::Esc => self.quick.close(),
            KeyCode::Enter if plain => self.open_quick_selection(),
            KeyCode::Backspace if plain => self.quick.backspace(),
            KeyCode::Char('j') | KeyCode::Down if plain => self.quick.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up if plain => self.quick.move_selection(-1),
            KeyCode::PageDown if plain => {
                self.quick.move_selection(self.viewer.page_step() as isize)
            }
            KeyCode::PageUp if plain => self
                .quick
                .move_selection(-(self.viewer.page_step() as isize)),
            KeyCode::Char(c) if plain => self.quick.push_char(c),
            _ => {}
        }
        Flow::Continue
    }

    /// Enter on a quick-switcher row: opens that note through the normal
    /// navigation stack (so `Backspace` returns to the previous note) and
    /// closes the popup. A failed open surfaces the error and keeps the
    /// popup open.
    fn open_quick_selection(&mut self) {
        let Some(item) = self.quick.selected_item().cloned() else {
            return;
        };
        if let Err(err) = self.open_note(&item.path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        self.quick.close();
        self.focus = Focus::Content;
    }

    /// Keys while the search popup is open (it is modal: nothing else
    /// receives keys, so typed `q`/`j`/… never leak into the app).
    /// Ctrl+C still quits from either mode.
    fn handle_search_key(&mut self, key: KeyEvent) -> Flow {
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        match self.search.mode {
            SearchMode::Input => match key.code {
                KeyCode::Enter if plain => {
                    let index = self.index.clone();
                    let ran = self.search.run(|query| match &index {
                        Some(index) => match query {
                            SearchQuery::Literal(text) => Ok(index.borrow().search(text)),
                            SearchQuery::Regex(pattern) => index
                                .borrow()
                                .search_regex(pattern)
                                .map_err(|err| regex_error_line(&err.to_string())),
                        },
                        None => Ok(SearchResults::default()),
                    });
                    if ran {
                        self.active_search = ActiveSearch::from_query(&self.search.query);
                    }
                }
                KeyCode::Backspace if plain => {
                    self.search.backspace();
                }
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
                KeyCode::Esc => self.search.close(),
                KeyCode::Char(c) if plain => self.search.push_char(c),
                _ => {}
            },
            SearchMode::Results => match key.code {
                KeyCode::Char('j') | KeyCode::Down if plain => self.search.move_selection(1),
                KeyCode::Char('k') | KeyCode::Up if plain => self.search.move_selection(-1),
                KeyCode::PageDown if plain => {
                    self.search.move_selection(self.viewer.page_step() as isize)
                }
                KeyCode::PageUp if plain => self
                    .search
                    .move_selection(-(self.viewer.page_step() as isize)),
                KeyCode::Enter if plain => self.open_search_hit(),
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
                KeyCode::Esc => self.search.close(),
                // Any typed character refines the query in place.
                KeyCode::Char(c) if plain => {
                    self.search.mode = SearchMode::Input;
                    self.search.push_char(c);
                }
                _ => {}
            },
        }
        Flow::Continue
    }

    /// Enter on a search hit: opens the note through the normal
    /// navigation stack (so Backspace returns to the previous note with
    /// its scroll) and scrolls the matched raw line into view. The popup
    /// closes after a successful open (with the focus moving to the
    /// content pane, like every other open path); a failed open surfaces
    /// the error and keeps the popup.
    fn open_search_hit(&mut self) {
        let Some(hit) = self.search.selected_hit().cloned() else {
            return;
        };
        if let Err(err) = self.open_note(&hit.path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        self.viewer.scroll_to_source_line(hit.line);
        self.search.close();
        // Consistent with the sidebar/backlinks open paths: the reader's
        // context moves to the opened note.
        self.focus = Focus::Content;
    }

    fn backlink_move(&mut self, delta: isize) {
        let len = self.backlinks.len();
        if len == 0 {
            return;
        }
        let next = (self.backlink_sel as isize + delta).clamp(0, len as isize - 1) as usize;
        self.select_backlink(next);
    }

    fn toggle_sidebar(&mut self) {
        if self.sidebar.is_none() {
            return; // no vault: nothing to show
        }
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus = Focus::Content;
        }
    }

    /// `t`: the sidebar switches between the folder tree and the tag list.
    /// From the outline it toggles the pair underneath and leaves the
    /// outline (the outline remembers which of the two it was showing).
    fn toggle_sidebar_mode(&mut self) {
        if self.sidebar.is_none() {
            return;
        }
        self.sidebar_mode = match self.sidebar_mode {
            SidebarMode::Tree => SidebarMode::Tags,
            SidebarMode::Tags => SidebarMode::Tree,
            SidebarMode::Outline => match self.outline_prev {
                SidebarMode::Tree => SidebarMode::Tags,
                _ => SidebarMode::Tree,
            },
        };
        if self.sidebar_mode == SidebarMode::Tags {
            self.refresh_tags();
        }
        self.outline_prev = self.sidebar_mode;
    }

    /// `o`: shows the open note's outline over the tree/tag view; `o` again
    /// restores whichever of the two was last shown. A no-op without a
    /// sidebar, like `t`.
    fn toggle_outline(&mut self) {
        if self.sidebar.is_none() {
            return;
        }
        match self.sidebar_mode {
            SidebarMode::Outline => self.sidebar_mode = self.outline_prev,
            mode => {
                self.outline_prev = mode;
                self.sidebar_mode = SidebarMode::Outline;
                self.outline.sync(self.outline_len());
            }
        }
    }

    /// `b`: show/hide the backlinks panel. Opening focuses it when the
    /// content pane has focus (so j/k + Enter work immediately).
    fn toggle_backlinks(&mut self) {
        self.backlinks_open = !self.backlinks_open;
        if self.backlinks_open {
            self.refresh_backlinks();
            self.backlink_sel = 0;
            self.backlink_scroll = 0;
            // Opening always moves focus to the panel, whatever pane had it
            // (e.g. the sidebar after a click), so j/k/Enter act on it.
            self.focus = Focus::Backlinks;
        } else if self.focus == Focus::Backlinks {
            self.focus = Focus::Content;
        }
    }

    /// `m`: toggle mouse capture. Only flips the intent here — `main.rs`'s
    /// event loop owns the terminal and executes the actual
    /// `EnableMouseCapture`/`DisableMouseCapture` when it sees the change.
    /// Off, the terminal handles the mouse natively (text selection/copy)
    /// and the app ignores clicks/wheels and drops the pointer it had
    /// tracked (no events arrive to move a stale cell, and a bar must not
    /// stay highlighted); the status bar shows `mouse off`.
    pub fn toggle_mouse_capture(&mut self) {
        self.mouse_capture = !self.mouse_capture;
        if !self.mouse_capture {
            self.pointer = None;
            self.viewer.clear_hover_link();
        }
    }

    /// `<` / `>`: narrow/widen the sidebar by 2 columns. The override is
    /// absolute columns (clamped 24–120), kept across hides (`Tab`) and
    /// resizes; `None` restores the proportional default. Content re-parses
    /// automatically because the layout feeds the width back every frame.
    pub fn narrow_sidebar(&mut self) {
        self.set_sidebar_width(self.current_sidebar_width().saturating_sub(2));
    }

    pub fn widen_sidebar(&mut self) {
        self.set_sidebar_width(self.current_sidebar_width() + 2);
    }

    /// The width the next `<`/`>` step starts from: the override when set,
    /// else the width of the last drawn sidebar (the proportional default).
    fn current_sidebar_width(&self) -> u16 {
        self.sidebar_width
            .or_else(|| self.layout.sidebar_area.map(|a| a.width))
            .unwrap_or_else(|| ui::sidebar_width(u16::MAX))
    }

    fn set_sidebar_width(&mut self, cols: u16) {
        self.sidebar_width = Some(cols.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH));
    }

    /// Snapshot for the session file: the open note (vault-relative) with
    /// its scroll, and the sidebar's mode/selection/expansion. Paths that
    /// escape the vault root (a file opened directly) are stored verbatim.
    pub fn session_state(&self, root: &Path) -> crate::session::SessionState {
        fn rel(root: &Path, path: Option<&Path>) -> Option<String> {
            let path = path?;
            Some(
                path.strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned(),
            )
        }
        crate::session::SessionState {
            note: rel(root, Some(self.current_path().as_path())),
            scroll: self.current_scroll() as u32,
            mode: Some(
                match self.sidebar_mode {
                    SidebarMode::Tree => "tree",
                    SidebarMode::Tags => "tags",
                    // The outline is a temporary overlay: persist the view
                    // it was drawn over, so a restore lands on tree/tags.
                    SidebarMode::Outline => match self.outline_prev {
                        SidebarMode::Tags => "tags",
                        _ => "tree",
                    },
                }
                .to_string(),
            ),
            selected: rel(
                root,
                self.sidebar.as_ref().and_then(|s| s.selected.as_deref()),
            ),
            expanded: self
                .sidebar
                .as_ref()
                .map(|s| {
                    let mut dirs: Vec<String> = s
                        .expanded
                        .iter()
                        .filter_map(|p| rel(root, Some(p.as_path())))
                        .collect();
                    dirs.sort();
                    dirs
                })
                .unwrap_or_default(),
        }
    }

    /// Apply a session snapshot, best-effort: vanished files, entries the
    /// tree no longer has, and open failures are skipped silently — the
    /// fallback is the normal startup state (vault index at the top, the
    /// first sidebar row selected). Only *vault-relative* entries are
    /// honored: the session file lives outside the vault, so it must not
    /// smuggle opens — absolute paths and `..` escapes in the file are
    /// skipped (the snapshot's own `session_state` never writes them, but
    /// the file is user-editable). The restored scroll is clamped by the
    /// next render (`run_app`'s initial `on_resize`); a `.base` note
    /// reopens with the table's row offset.
    pub fn restore_session(&mut self, state: &crate::session::SessionState, root: &Path) {
        use std::path::Component;
        let within = |rel: &str| -> Option<PathBuf> {
            let path = Path::new(rel);
            if path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
                || rel.is_empty()
            {
                return None;
            }
            Some(root.join(path))
        };
        if let Some(sidebar) = &mut self.sidebar {
            for dir in &state.expanded {
                if let Some(path) = within(dir) {
                    sidebar.expanded.insert(path);
                }
            }
            if let Some(selected) = state.selected.as_deref().and_then(within) {
                sidebar.selected = Some(selected);
                sidebar.scroll_selection_into_view();
            }
        }
        match state.mode.as_deref() {
            Some("tags") if self.sidebar.is_some() => {
                self.sidebar_mode = SidebarMode::Tags;
                self.outline_prev = SidebarMode::Tags;
                self.refresh_tags();
            }
            _ => {}
        }
        if let Some(note) = state.note.as_deref().and_then(within) {
            let scroll = state.scroll;
            if crate::baseview::is_base_file(&note) {
                // A `.base` table: the scroll is the table's row offset
                // (clamped by the next render, like every base open).
                let _ = self.open_base_at(&note, scroll as usize);
            } else if self.open_note(&note).is_ok() {
                self.viewer.scroll = scroll;
            }
        }
    }

    /// `h`: from the content pane, move focus to the sidebar; from the
    /// sidebar, collapse (the selected folder/tag, or the parent of a
    /// note); from the backlinks panel, back to the content.
    fn key_left(&mut self) {
        match self.focus {
            Focus::Content => {
                if self.sidebar_visible {
                    self.focus = Focus::Sidebar;
                }
            }
            Focus::Backlinks => self.focus = Focus::Content,
            Focus::Sidebar => match self.sidebar_mode {
                SidebarMode::Tree => {
                    if let Some(sidebar) = &mut self.sidebar {
                        sidebar.collapse_or_parent();
                    }
                }
                SidebarMode::Tags => self.tags.collapse_or_parent(),
                // A heading list has no hierarchy to collapse.
                SidebarMode::Outline => {}
            },
        }
    }

    /// `l`: from the sidebar, expand a folder (on a folder) or move focus
    /// to the content pane (on a note). From the content pane there is
    /// nothing to the right, so it is a no-op.
    fn key_right(&mut self) {
        if self.focus != Focus::Sidebar {
            return;
        }
        match self.sidebar_mode {
            SidebarMode::Tree => {
                let Some(sidebar) = &mut self.sidebar else {
                    return;
                };
                match sidebar.selected_kind() {
                    Some(NodeKind::Dir) => {
                        sidebar.expand_or_activate();
                    }
                    // Notes and `.base` tables both hand focus to content.
                    Some(NodeKind::Note) | Some(NodeKind::Base) => self.focus = Focus::Content,
                    None => {}
                }
            }
            SidebarMode::Tags => {
                // On a tag: expand; on a note: focus the content pane.
                if self.tags.expand_or_activate().is_some() {
                    self.focus = Focus::Content;
                }
            }
            SidebarMode::Outline => self.focus = Focus::Content,
        }
    }

    fn sidebar_move(&mut self, delta: isize) {
        match self.sidebar_mode {
            SidebarMode::Tree => {
                if let Some(sidebar) = &mut self.sidebar {
                    if delta >= 0 {
                        sidebar.move_down(delta as usize);
                    } else {
                        sidebar.move_up((-delta) as usize);
                    }
                }
            }
            SidebarMode::Tags => {
                if delta >= 0 {
                    self.tags.move_down(delta as usize);
                } else {
                    self.tags.move_up((-delta) as usize);
                }
            }
            SidebarMode::Outline => {
                let len = self.outline_len();
                if delta >= 0 {
                    self.outline.move_down(delta as usize, len);
                } else {
                    self.outline.move_up((-delta) as usize, len);
                }
            }
        }
    }
}

/// The outline's rows for the content pane: the open note's heading table.
/// A `.base` table in the content pane is not a note, so it has no outline
/// (the viewer's last note is hidden behind the table). Borrows the two
/// fields rather than `&App` so the drawer can hold these rows alongside
/// the outline cursor.
pub(crate) fn outline_rows<'a>(
    base: &Option<BaseState>,
    viewer: &'a Viewer,
) -> &'a [engine::TocEntry] {
    if base.is_some() {
        &[]
    } else {
        &viewer.toc
    }
}

/// Maps a row of a vertical bar's track onto a value in `0..=max`: the
/// track's first row is 0, its last row `max`, the rows between interpolate
/// linearly. Rows above/below the track clamp to the ends (a drag can leave
/// the column). This is the one place a bar's track-row mapping lives; the
/// note pane maps onto its scroll range, a list panel onto its row indices.
fn bar_value_at(area: Rect, row: u16, max: usize) -> usize {
    let last = usize::from(area.height.saturating_sub(1));
    if last == 0 {
        return 0;
    }
    let offset = usize::from(row.saturating_sub(area.y)).min(last);
    (offset as u64 * max as u64 / last as u64) as usize
}

/// Maps a row of a list panel's bar onto a row index: the track's first row
/// is the list's first row, its last row the list's last, linear between.
/// `None` when the list is empty (there is no row to select).
fn bar_list_index(area: Rect, row: u16, len: usize) -> Option<usize> {
    Some(bar_value_at(area, row, len.checked_sub(1)?))
}

/// One footer line for a regular-expression compile error. The engine's
/// detailed diagnostic is multi-line; its last non-empty line names the
/// problem, which is what the popup's single footer row has room for.
fn regex_error_line(message: &str) -> String {
    let detail = message
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(message)
        .trim();
    let detail = detail.strip_prefix("error: ").unwrap_or(detail);
    format!("invalid regex: {detail}")
}

/// Is `(col, row)` inside `area`?
fn point_in_rect(col: u16, row: u16, area: Rect) -> bool {
    col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height
}

#[cfg(test)]
mod tests;
