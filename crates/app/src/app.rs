//! Application state: current note (`Viewer`), sidebar (tree / tags),
//! backlinks panel, focus, navigation history, and the disambiguation
//! popup. Terminal-independent; `main.rs` runs the event loop, `ui.rs`
//! draws.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use vault::{GitStatus, SearchResults};

use crate::config::ConfigWatch;
use crate::links::{LinkState, RenderedLink};
use crate::nav::Nav;
use crate::search::{SearchMode, SearchState};
use crate::sidebar::{NodeKind, Sidebar};
use crate::tags::TagList;
use crate::theme::{Theme, ThemeChoices};
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

/// What the sidebar shows: the folder tree or the tag list (`t` toggles).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Tree,
    Tags,
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

pub struct App {
    pub viewer: Viewer,
    /// `Some` only when `--vault` was passed; the sidebar is hidden until
    /// shown with Tab.
    pub sidebar: Option<Sidebar>,
    pub sidebar_visible: bool,
    pub sidebar_mode: SidebarMode,
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
    /// Last read-only git sync status (`None` = unavailable: no git, not
    /// a repository, or every command failed — the status bar shows
    /// nothing in that case).
    pub git: Option<GitStatus>,
    /// Vault-wide change detection (poll-based; see `watch.rs`).
    pub watcher: Option<VaultWatcher>,
    /// The note viewport's geometry as of the last draw, for click →
    /// link mapping (set by `ui::draw` every frame).
    pub note_viewport: Rect,
    /// The sidebar widget's geometry as of the last draw (`None` while
    /// the sidebar is hidden); routes sidebar clicks/wheels by position.
    pub sidebar_area: Option<Rect>,
    /// The backlinks panel's geometry as of the last draw (`None` while
    /// the panel is closed); routes panel clicks to focus.
    pub backlinks_area: Option<Rect>,
    /// The search popup's geometry as of the last draw (`None` while the
    /// popup is closed); routes result-row clicks and wheels by position.
    pub search_area: Option<Rect>,
    /// Last `viewer.last_reload` value we reacted to with a sidebar rescan
    /// (loose refresh: the tree is re-walked only after a watch reload).
    last_rescan: Option<SystemTime>,
    last_vault_scan: Option<Instant>,
    /// When the git status was last actually spawned (debounce clock).
    last_git_refresh: Option<Instant>,
    /// Active theme (chrome + markdown palette). Cycled with `Shift+T`,
    /// replaced by config hot-reloads; session-only, never persisted.
    pub theme_ui: crate::theme::UiTheme,
    /// Whether the app currently wants mouse capture on (`m` toggles, the
    /// config `mouse` key sets the startup default; main.rs executes the
    /// actual capture switch when this diverges from the live state). Off
    /// = the terminal handles the mouse natively (text selection/copy).
    pub mouse_capture: bool,
    /// Sidebar width override in terminal columns (`None` = the default
    /// 30% of the terminal, clamped 24–60; see `ui::sidebar_width`). Set
    /// by `<`/`>` and the config `sidebar_width` key, clamped 24–120.
    pub sidebar_width: Option<u16>,
    theme_choices: ThemeChoices,
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
    /// The currently open `.base` table (`Some` while a base file is the
    /// visible content; `None` renders the viewer's note). See
    /// `baseview.rs`.
    pub base: Option<BaseState>,
    /// Transient info message for the status bar (`STATUS_MESSAGE_TTL`).
    /// Shown only while `viewer.last_error` is `None`.
    pub last_status: Option<StatusMessage>,
    /// The status bar's note-name segment as of the last draw (`None`
    /// when no name segment is drawn); routes the copy-path click.
    pub status_name_area: Option<Rect>,
    /// A clipboard copy requested by a click, waiting for the event loop
    /// to write its OSC 52 sequence to the terminal (kept out of the
    /// render buffer; see `main.rs`).
    pub pending_clipboard: Option<String>,
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
            git: None,
            watcher,
            mouse_capture: true,
            sidebar_width: None,
            note_viewport: Rect::default(),
            sidebar_area: None,
            backlinks_area: None,
            search_area: None,
            last_rescan: None,
            last_vault_scan: None,
            last_git_refresh: None,
            // Default theme (ocean) until main applies the resolved one.
            theme_ui: crate::theme::ui_by_preset(engine::MarkdownThemePreset::default()),
            theme_choices: ThemeChoices::default(),
            discovered_themes: BTreeMap::new(),
            config_watch: None,
            last_config_check: None,
            theme_dirs: Vec::new(),
            pending_theme: None,
            base: None,
            last_status: None,
            status_name_area: None,
            pending_clipboard: None,
        };
        app.refresh_tags();
        app.refresh_backlinks();
        app.refresh_git(true);
        Ok(app)
    }

    /// Runs the read-only git status queries for the vault root.
    ///
    /// On demand only: startup (`force`), the `u` key (`force`), every
    /// successful note open (SPEC: status is "refreshed on open, watch
    /// events, and `u`"), and watch-triggered reindexes — the latter two
    /// debounced: skipped when the last spawn is less than
    /// [`GIT_DEBOUNCE`] ago (opens included, so rapidly hopping between
    /// notes spawns at most once per window). Never spawned continuously.
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
        // Extract the root before spawning (no borrow may cross the git
        // subprocess calls).
        let root = index.borrow().root().to_path_buf();
        self.git = vault::git::read_status(&root);
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

    /// `Shift+T`: cycles to the next theme (presets, then discovered
    /// custom themes, wrapping) and applies it instantly. Session-only —
    /// never writes the config. The newly selected theme's warnings
    /// surface in the status bar. Wrapping past the last entry re-scans
    /// the theme dirs first, so custom themes added mid-session join the
    /// rotation (discovery parses every custom theme file eagerly inside
    /// `ThemeChoices::build` — a dir listing plus the parses, cheap at
    /// realistic theme counts).
    pub fn cycle_theme(&mut self) {
        let mut load_errors = Vec::new();
        if self.theme_choices.index + 1 >= self.theme_choices.entries.len() {
            load_errors = self.rediscover_themes();
        }
        let choice = self.theme_choices.cycle().clone();
        self.apply_theme(&choice.theme, &choice.warnings);
        self.set_status(format!("theme: {}", choice.theme.name));
        // Re-discovery load errors surface when the applied theme itself
        // had nothing to say (theme warnings win — same precedence as
        // startup, where load errors are stderr-only).
        if !load_errors.is_empty() && self.viewer.last_error.is_none() {
            self.viewer.last_error = Some(load_errors.join("; "));
        }
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
        Ok(())
    }

    /// Opens the target of a resolved wikilink, scrolling to its heading
    /// anchor when the link carried one (best-effort: unknown headings
    /// leave the note at the top).
    fn open_wikilink_target(&mut self, path: &Path, heading: Option<&str>) {
        if let Err(err) = self.open_note(path) {
            self.viewer.last_error = Some(format!("open failed: {err}"));
            return;
        }
        if let Some(heading) = heading {
            self.viewer.scroll_to_heading(heading);
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
                    // still stands — record it once, quietly.
                    self.viewer.last_error =
                        Some("toggled (no updated: field in frontmatter to bump)".to_string());
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
    /// Routing: the search popup (when open) owns clicks inside its
    /// rectangle; while the help/disambiguation popups are open clicks
    /// are ignored entirely — those popups are modal and a click must
    /// never act on what is behind them (focus unchanged). While the
    /// sidebar is visible its column owns the click (no click-through
    /// into the note viewport, which shares the same screen rows).
    pub fn handle_click(&mut self, col: u16, row: u16) {
        // The search popup is position-routed (result rows are
        // click-selectable); clicks elsewhere stay inert while it is open.
        if self.search.open {
            self.search_click(col, row);
            return;
        }
        if self.any_popup_open() {
            return;
        }
        if let Some(area) = self.status_name_area {
            if point_in_rect(col, row, area) {
                self.copy_current_relative_path();
                return;
            }
        }
        if self.sidebar_visible {
            if let Some(area) = self.sidebar_area {
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
        if let Some(area) = self.backlinks_area {
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
        let area = self.note_viewport;
        // Right edge included: the scrollbar column sits just past the
        // text rect and must never reach the link/checkbox hit-test.
        if !point_in_rect(col, row, area) {
            return;
        }
        self.focus = Focus::Content;
        let line = self.viewer.scroll as usize + (row - area.y) as usize;
        let col = (col - area.x) as usize;
        // Checkboxes first: the glyph is not a link region, but a task
        // line can also contain links — the glyph columns are matched
        // before the link regions so the click target is unambiguous.
        if let Some(task) = self.viewer.task_at(line, col) {
            self.viewer.set_focus_task(Some(task));
            self.toggle_task(task);
            self.focus = Focus::Content;
            return;
        }
        let index = self.viewer.links.iter().position(|link| {
            link.regions
                .iter()
                .any(|r| r.line == line && col >= r.start && col < r.end)
        });
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
        let area = self.note_viewport;
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

    /// A click inside the search popup: in results mode a click selects
    /// the hit row, and clicking the already-selected row opens it (the
    /// keyboard's Enter semantics). Border/footer/empty rows and input
    /// mode are no-ops; clicks outside the popup do nothing at all.
    fn search_click(&mut self, col: u16, row: u16) {
        let Some(popup) = self.search_area else {
            return;
        };
        if !point_in_rect(col, row, popup) || row <= popup.y {
            return;
        }
        if self.search.mode != SearchMode::Results {
            return;
        }
        if self.search.hits.is_empty() {
            return;
        }
        // Same geometry the renderer used (list rows = popup height minus
        // the two borders and the footer row).
        let list_rows = popup.height.saturating_sub(3).max(1) as usize;
        let start = self
            .search
            .selected
            .saturating_sub(list_rows.saturating_sub(1));
        let index = start + (row - popup.y - 1) as usize;
        if index >= self.search.hits.len() {
            return; // footer row or below the list
        }
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
                    vault_changed |= !watcher.poll(&mut index.borrow_mut()).is_empty();
                }
            }
        }

        if vault_changed {
            // Link states and glyphs may have changed with the index.
            self.viewer.reparse();
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

    /// Opens a note selected in the sidebar (tree or tag mode); on read
    /// failure the error is surfaced in the status bar and the current
    /// note stays put (with the sidebar still focused). On success the
    /// focus moves to the content pane.
    fn open_from_sidebar(&mut self) {
        let path = match self.sidebar_mode {
            SidebarMode::Tree => self.sidebar.as_mut().and_then(|sb| sb.activate()),
            SidebarMode::Tags => self.tags.activate(),
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
        // theme dirs are re-scanned only while the theme is still
        // unresolved (a dir listing plus an eager parse of every theme
        // file inside `ThemeChoices::build` — cheap at realistic
        // counts). Stops as soon as it resolves.
        if let Some(name) = self.pending_theme.clone() {
            let custom = self.discovered_themes.clone();
            match crate::theme::resolve_theme(&name, &custom) {
                Ok(resolved) => self.apply_resolved_theme(resolved, &[]),
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

    /// Handles one key press. The search popup (if open) gets the key
    /// first; then the disambiguation popup; then any key closes the help
    /// popup; `q` and Ctrl+C quit; everything else is focus-dependent.
    pub fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // "Plain" keys: no modifiers, or Shift only (Shift is how terminals
        // deliver uppercase letters like `G`).
        let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
        if self.search.open {
            return self.handle_search_key(key);
        }
        if self.disambiguation.is_some() {
            let last = self
                .disambiguation
                .as_ref()
                .expect("just checked")
                .candidates
                .len()
                .saturating_sub(1);
            match key.code {
                KeyCode::Char('j') | KeyCode::Down if plain => {
                    let d = self.disambiguation.as_mut().expect("just checked");
                    d.selected = (d.selected + 1).min(last);
                }
                KeyCode::Char('k') | KeyCode::Up if plain => {
                    let d = self.disambiguation.as_mut().expect("just checked");
                    d.selected = d.selected.saturating_sub(1);
                }
                KeyCode::Enter if plain => {
                    let candidate = self.disambiguation.take().expect("just checked");
                    self.open_wikilink_target(
                        &candidate.candidates[candidate.selected],
                        candidate.heading.as_deref(),
                    );
                }
                // Ctrl+C quits even while the popup is open (mirrors the
                // main loop; everything else cancels the popup).
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
                _ => self.disambiguation = None,
            }
            return Flow::Continue;
        }
        if self.viewer.help_open {
            // Help scrolls with j/k/arrows, PgUp/PgDn, g/G; every other key
            // closes it (as before). Scroll positions are clamped at draw
            // time (`ui::draw_help`), so the step here can overshoot safely.
            let page = crate::ui::help_page_step();
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
            return Flow::Continue;
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
            KeyCode::Char('m') if plain => self.toggle_mouse_capture(),
            KeyCode::Char('f') if plain => self.viewer.toggle_metadata(),
            KeyCode::Char('>') if plain => self.widen_sidebar(),
            KeyCode::Char('<') if plain => self.narrow_sidebar(),
            KeyCode::Char('b') if plain => self.toggle_backlinks(),
            KeyCode::Char('s') if plain && self.index.is_some() => self.search.open_input(),
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
                Focus::Content => {
                    if let Some(base) = &mut self.base {
                        base.scroll_down(1);
                    } else {
                        self.viewer.scroll_down(1);
                    }
                }
            },
            KeyCode::Char('k') | KeyCode::Up if plain => match self.focus {
                Focus::Sidebar => self.sidebar_move(-1),
                Focus::Backlinks => self.backlink_move(-1),
                Focus::Content => {
                    if let Some(base) = &mut self.base {
                        base.scroll_up(1);
                    } else {
                        self.viewer.scroll_up(1);
                    }
                }
            },
            KeyCode::PageDown if plain && self.focus == Focus::Content => {
                let step = self.viewer.page_step();
                if let Some(base) = &mut self.base {
                    base.scroll_down(step as usize);
                } else {
                    self.viewer.scroll_down(step)
                }
            }
            KeyCode::PageUp if plain && self.focus == Focus::Content => {
                let step = self.viewer.page_step();
                if let Some(base) = &mut self.base {
                    base.scroll_up(step as usize);
                } else {
                    self.viewer.scroll_up(step)
                }
            }
            KeyCode::Char('g') | KeyCode::Home if plain && self.focus == Focus::Content => {
                if let Some(base) = &mut self.base {
                    base.scroll_top();
                } else {
                    self.viewer.scroll_top()
                }
            }
            KeyCode::Char('G') | KeyCode::End if plain && self.focus == Focus::Content => {
                if let Some(base) = &mut self.base {
                    base.scroll_bottom();
                } else {
                    self.viewer.scroll_bottom()
                }
            }
            KeyCode::Char('w') if plain && self.focus == Focus::Content => {
                self.viewer.toggle_watch()
            }
            // `Shift+T`: theme cycling (session-only). `t` stays the
            // sidebar-mode toggle; Shift is how terminals deliver `T`.
            KeyCode::Char('T') if plain => self.cycle_theme(),
            // `.base` table view switching: `v` cycles, digits jump.
            KeyCode::Char('v') if plain && self.base.is_some() => self.base_cycle_view(),
            KeyCode::Char(c) if plain && self.base.is_some() && c.is_ascii_digit() && c != '0' => {
                self.base_set_view(c.to_digit(10).unwrap() as usize - 1);
            }
            KeyCode::Enter if plain && self.focus == Focus::Sidebar => self.open_from_sidebar(),
            KeyCode::Enter if plain && self.focus == Focus::Backlinks => self.open_from_backlinks(),
            _ => {}
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
            Focus::Content => {
                if let Some(base) = &mut self.base {
                    if down {
                        base.scroll_down(3);
                    } else {
                        base.scroll_up(3);
                    }
                } else if down {
                    self.viewer.scroll_down(3)
                } else {
                    self.viewer.scroll_up(3)
                }
            }
        }
    }

    /// Mouse wheel with the cursor position: a wheel over the visible
    /// sidebar scrolls the sidebar list regardless of where the keyboard
    /// focus sits; a wheel over the open search popup scrolls the result
    /// selection; anywhere else the focused pane scrolls (the M2
    /// behavior, kept via [`Self::handle_wheel`]). Help/disambiguation
    /// popups gate the wheel like clicks.
    pub fn handle_wheel_at(&mut self, col: u16, row: u16, down: bool) {
        if self.search.open {
            if let Some(popup) = self.search_area {
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
            if let Some(area) = self.sidebar_area {
                if point_in_rect(col, row, area) {
                    self.sidebar_move(if down { 3 } else { -3 });
                    return;
                }
            }
        }
        self.handle_wheel(down);
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
                    self.search.run(|query| match &index {
                        Some(index) => index.borrow().search(query),
                        None => SearchResults::default(),
                    });
                }
                KeyCode::Backspace if plain => {
                    self.search.query.pop();
                }
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => return Flow::Quit,
                KeyCode::Esc => self.search.close(),
                KeyCode::Char(c) if plain => self.search.query.push(c),
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
                    self.search.query.push(c);
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
        self.backlink_sel = next;
        if next >= self.backlink_scroll + self.backlink_viewport {
            self.backlink_scroll = next + 1 - self.backlink_viewport;
        } else if next < self.backlink_scroll {
            self.backlink_scroll = next;
        }
        self.clamp_backlink_scroll();
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
    fn toggle_sidebar_mode(&mut self) {
        if self.sidebar.is_none() {
            return;
        }
        self.sidebar_mode = match self.sidebar_mode {
            SidebarMode::Tree => SidebarMode::Tags,
            SidebarMode::Tags => SidebarMode::Tree,
        };
        if self.sidebar_mode == SidebarMode::Tags {
            self.refresh_tags();
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
    /// and the app ignores clicks/wheels; the status bar shows `mouse off`.
    pub fn toggle_mouse_capture(&mut self) {
        self.mouse_capture = !self.mouse_capture;
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
            .or_else(|| self.sidebar_area.map(|a| a.width))
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
                    Some(NodeKind::Note) => self.focus = Focus::Content,
                    None => {}
                }
            }
            SidebarMode::Tags => {
                // On a tag: expand; on a note: focus the content pane.
                if self.tags.expand_or_activate().is_some() {
                    self.focus = Focus::Content;
                }
            }
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
        }
    }
}

/// Is `(col, row)` inside `area`?
fn point_in_rect(col: u16, row: u16, area: Rect) -> bool {
    col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use vault::VaultIndex;

    use crate::testutil::{TempDir, TempFile};

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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 1, 20));
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
        app.note_viewport = Rect::new(0, 0, 40, 20);
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
        app.note_viewport = Rect::new(0, 0, 40, 20);
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
        app.note_viewport = Rect::new(0, 0, 40, 20);
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.note_viewport = Rect::new(30, 0, 70, 20);
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.note_viewport = Rect::new(30, 0, 70, 20);
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.note_viewport = Rect::new(30, 0, 70, 20);
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.note_viewport = Rect::new(30, 0, 70, 15);
        app.backlinks_open = true;
        app.backlinks_area = Some(Rect::new(30, 15, 70, 5));
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
        assert!(app.backlinks_area.is_none());
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
        let hit = app.backlinks_area.expect("panel drawn");
        // The bar is the panel's last inner column (one left of the border);
        // the hit rect stops right before it.
        let bar_col = hit.right();
        assert_eq!(bar_col, 98, "bar column is inside the panel, border at 99");
        let buffer = term.backend().buffer();
        assert!(
            (hit.y + 1..hit.bottom() - 1).any(|y| {
                matches!(buffer[(bar_col, y)].symbol(), "█" | "░" | "▲" | "▼")
            }),
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
        let hit = app.backlinks_area.expect("panel drawn");
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
        let hit = app.backlinks_area.expect("panel drawn");
        app.handle_click(hit.x + 1, hit.bottom() - 2);
        assert_eq!(app.viewer.path, vault_dir.path().join("bl04.md"));

        // The panel's own border rows keep the focus-only behavior.
        let mut app = overflowing_backlinks_app(&vault_dir);
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let hit = app.backlinks_area.expect("panel drawn");
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
        let hit = app.backlinks_area.expect("panel drawn");
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
        let hit = app.backlinks_area.expect("panel drawn");
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.focus = Focus::Content;
        app.viewer.help_open = true;
        app.handle_click(3, 2);
        assert_eq!(app.focus, Focus::Content, "modal popup ignores the click");
    }

    #[test]
    fn sidebar_click_maps_screen_row_through_scroll() {
        let vault_dir = vault();
        let (_f, mut app) = app(&vault_dir);
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
        app.note_viewport = Rect::new(30, 0, 70, 20);
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));
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
        app.sidebar_area = Some(Rect::new(0, 0, 30, 20));

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
        let (_file, app) = app(&vault_dir);
        let git = app.git.as_ref().expect("git status read at startup");
        assert!(git.clean);
        assert!(git.changed_files.is_empty());
        assert_eq!((git.ahead, git.behind), (0, 0));
    }

    #[test]
    fn u_key_refreshes_and_marks_dirty() {
        let vault_dir = git_vault("app-git-dirty");
        let (_f, mut app) = app(&vault_dir);
        assert!(app.git.as_ref().unwrap().clean);

        std::fs::write(vault_dir.path().join("a.md"), "changed\n").unwrap();
        // The debounced poll path may skip (within the 5s window of the
        // startup refresh); the explicit `u` key must not.
        app.poll();
        app.handle_key(key(KeyCode::Char('u')));
        let git = app.git.as_ref().unwrap();
        assert!(!git.clean);
        assert_eq!(git.changed_files, vec![vault_dir.path().join("a.md")]);
    }

    #[test]
    fn opening_a_note_refreshes_git_status() {
        // SPEC: git status is "refreshed on open". An external commit
        // fires no watch event (only `.git` changes), so without this the
        // markers/badge stay stale until `u`.
        let vault_dir = git_vault("app-git-onopen");
        let (_f, mut app) = app(&vault_dir);
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
        assert!(
            app.git.as_ref().unwrap().clean,
            "open-refresh is debounce-gated like the watch path"
        );
    }

    #[test]
    fn watch_reindex_refresh_is_debounced() {
        let vault_dir = git_vault("app-git-debounce");
        let (file, mut app) = app(&vault_dir);
        // Fake a very recent refresh: watch-triggered refreshes within
        // GIT_DEBOUNCE must be skipped entirely.
        app.last_git_refresh = Some(Instant::now());
        file.rewrite("dirty now\n");
        std::fs::write(vault_dir.path().join("fresh.md"), "new\n").unwrap();
        app.poll();
        assert!(
            app.git.as_ref().unwrap().clean,
            "debounced refresh must not respawn within 5s"
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
        app.note_viewport = Rect::new(0, 0, 40, 24);
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
        app.note_viewport = Rect::new(0, 0, 40, 24);
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
        app.search_area = Some(popup);
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
        app.search_area = Some(popup);
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

    #[test]
    fn wheel_over_search_results_scrolls_the_selection_not_the_content() {
        let (_dir, file, mut app) = search_state_app();
        let popup = search_popup();
        app.search_area = Some(popup);
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
        app.note_viewport = Rect::new(0, 0, 80, 24);
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
        std::fs::write(dir.path().join("zeta.toml"), "base = \"ocean\"\n").unwrap();
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

        std::fs::write(&path, "!!! not toml !!!\n").unwrap();
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
        std::fs::write(themes.join("latecomer.toml"), "base = \"forest\"\n").unwrap();
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
        std::fs::write(themes.join("missing-x.toml"), "base = \"forest\"\n").unwrap();
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
        std::fs::write(themes.join("alpha.toml"), "base = \"ocean\"\n").unwrap();
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
        std::fs::write(themes.join("zulu.toml"), "base = \"forest\"\n").unwrap();
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
        app.status_name_area = Some(Rect::new(0, 23, 20, 1));
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
        app.status_name_area = Some(Rect::new(0, 23, 10, 1));
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
}
