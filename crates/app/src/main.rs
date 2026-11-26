//! Read-only TUI shell with sidebar (tree/tags), backlinks panel,
//! wikilink navigation, navigation history, watch mode.
//!
//! Usage: `frond [--vault <dir|name>] [file.md]` — `frond --help` for all
//! flags (`--config-show` prints a commented default config file).
//!
//! Which vault opens is SPEC.md "Data flow" step 1 ([`choose_vault`]): with
//! two or more config `[vaults]` and neither `--vault` nor a file argument,
//! the vault picker ([`vault_picker`]) runs first; with nothing to go on,
//! the app prompts for the vault directory. `v` switches vaults while the
//! app runs: [`run_app`] leaves the open vault as a clean quit does and
//! [`start_app`] opens the chosen one, the path the first start takes. A file argument without a vault is
//! opened on its own, without a sidebar; the vault index is then the
//! current directory. With a vault, `Tab` shows/hides the sidebar tree; `f` moves the focus
//! between the panes; `Backspace`/`Ctrl+O` walk the navigation history.
//!
//! Keys: j/k, arrows, PgUp/PgDn, g/G, Home/End, mouse wheel to scroll;
//! `w` toggles watch; `s` is the global vault search; `Ctrl+K` is the
//! quick jumper; `u` refreshes the
//! git sync status; `Shift+T` cycles the theme; `?` toggles the help
//! popup; `v` switches vaults; `]`/`[`/`1`–`9` switch table views while a `.base` file is open;
//! `q`/Ctrl+C quits.
//!
//! Vault index: booted at startup from `--vault` when given, otherwise
//! from the current working directory (the opened file's parent chain is
//! deliberately NOT searched — either explicit `--vault` or cwd default).
//!
//! Git sync status: read-only (`vault::git` allowlist), spawned on demand
//! at startup, on `u`, and debounced after watch-triggered reindexes;
//! git-absent / not-a-repo vaults show no status segment at all.

mod app;
mod baseview;
mod builtin_themes;
mod clipboard;
mod config;
mod links;
mod nav;
mod outline;
mod quick;
mod search;
mod session;
mod sidebar;
mod tags;
mod tasks;
#[cfg(test)]
mod testutil;
mod theme;
mod ui;
mod vault_picker;
mod viewer;
mod watch;

use std::cell::RefCell;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind, MouseButton, MouseEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use vault::VaultIndex;

use crate::app::App;
use crate::viewer::{SharedIndex, Viewer};

/// Watch poll interval. The file is stat-ed whenever this much time has
/// elapsed since the last poll (deadline-based, see `run_app`).
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Fallback terminal size when the real size is unavailable.
const FALLBACK_SIZE: (u16, u16) = (100, 40);

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Parsed command line.
struct Args {
    file: Option<PathBuf>,
    vault: Option<PathBuf>,
    theme: Option<String>,
    watch: Option<bool>,
    mouse: Option<bool>,
    /// Non-TUI informational flag (`--help`/`--version`/`--config-show`):
    /// print and exit before any terminal setup.
    info: Option<Info>,
}

/// Informational flags served without starting the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Info {
    Help,
    Version,
    ConfigShow,
}

/// Usage text for `--help` (and the on-error usage line).
const HELP_TEXT: &str = "\
frond — a terminal markdown vault reader

Usage: frond [--vault <dir|name>] [--theme <name>] [file.md | file.base]

Options:
  --vault <dir|name>    Vault directory to index, or a [vaults] name
                        (config: vault)
  --theme <name>        Theme by name (config: theme)
  --watch[=true|false]  Live re-render on vault changes (config: watch)
  --mouse[=true|false]  Mouse capture on/off; off = native text selection
                        (config: mouse, `M` toggles at runtime)
  --config-show         Print a commented default config.toml and exit
  --version             Print the version and exit
  --help                Print this help and exit

With two or more config [vaults] and neither --vault nor a file argument,
frond opens on a vault picker. With no vault at all and no file argument,
it prompts for the vault directory. Config file:
~/.config/frond/config.toml (frond --config-show prints a template).
";

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// Arg parsing over an explicit iterator (unit-testable; `parse_args`
/// feeds it the real argv). Collected into a slice for the lookahead the
/// boolean flags need: a bare `--watch`/`--mouse` followed by something
/// that parses as a bool is almost always the space-form mistake
/// (`--mouse false`), so it errors with the `=`-form hint instead of
/// letting `false` silently become the file argument.
fn parse_args_from(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut vault: Option<PathBuf> = None;
    let mut theme: Option<String> = None;
    let mut watch: Option<bool> = None;
    let mut mouse: Option<bool> = None;
    let mut info: Option<Info> = None;
    let mut file: Option<PathBuf> = None;
    let args: Vec<String> = args.collect();
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--vault" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--vault requires a directory or vault name argument")?;
                index += 1;
                vault = Some(value.into());
            }
            _ if arg.starts_with("--vault=") => {
                vault = Some(arg["--vault=".len()..].to_string().into());
            }
            "--theme" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--theme requires a name argument")?;
                index += 1;
                theme = Some(value.to_string());
            }
            _ if arg.starts_with("--theme=") => {
                theme = Some(arg["--theme=".len()..].to_string());
            }
            "--watch" => {
                if bool_lookahead(args.get(index + 1)) {
                    return Err("--watch takes no value; use --watch=<true|false>".to_string());
                }
                watch = Some(true);
            }
            _ if arg.starts_with("--watch=") => {
                let value = arg["--watch=".len()..].to_string();
                watch = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("--watch expects true or false, got \"{value}\""))?,
                );
            }
            "--mouse" => {
                if bool_lookahead(args.get(index + 1)) {
                    return Err("--mouse takes no value; use --mouse=<true|false>".to_string());
                }
                mouse = Some(true);
            }
            _ if arg.starts_with("--mouse=") => {
                let value = arg["--mouse=".len()..].to_string();
                mouse = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("--mouse expects true or false, got \"{value}\""))?,
                );
            }
            "--help" | "-h" => info = Some(Info::Help),
            "--version" | "-V" => info = Some(Info::Version),
            "--config-show" => info = Some(Info::ConfigShow),
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => {
                if file.is_some() {
                    return Err("only one file may be given".to_string());
                }
                file = Some(arg.into());
            }
        }
        index += 1;
    }
    Ok(Args {
        file,
        vault,
        theme,
        watch,
        mouse,
        info,
    })
}

/// True when `next` (the token after a bare boolean flag) parses as a
/// bool — the space-form mistake the `=`-only flag vocabulary rejects.
fn bool_lookahead(next: Option<&String>) -> bool {
    next.is_some_and(|value| value.parse::<bool>().is_ok())
}

/// Mouse capture startup state: `--mouse` flag > config `mouse` key >
/// on (the historical default).
fn effective_mouse(flag: Option<bool>, config: Option<bool>) -> bool {
    flag.or(config).unwrap_or(true)
}

/// Effective vertical note margin: the config `margin_y` value, clamped to
/// the same 0–`MAX_MARGIN_Y` range the horizontal margin uses, or the
/// default when the key is absent.
fn effective_margin_y(config: Option<u16>) -> u16 {
    config
        .unwrap_or(config::DEFAULT_MARGIN_Y)
        .min(config::MAX_MARGIN_Y)
}

/// Checks that `input` names an existing directory and returns its absolute
/// form. Empty input is an error (the prompt has no default).
fn validate_vault_dir(input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("no vault path given".to_string());
    }
    // Expand a leading `~/` (a shell would normally do this for us).
    let expanded = match trimmed.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(trimmed),
        },
        None => PathBuf::from(trimmed),
    };
    if !is_vault_dir(&expanded) {
        return Err(format!("not a directory: {}", expanded.display()));
    }
    Ok(absolutize(&expanded))
}

/// The one check for "is this a vault directory": the prompt
/// ([`validate_vault_dir`]), a chosen startup path (`main`) and the picker's
/// Enter (`vault_picker`) all ask it.
fn is_vault_dir(path: &Path) -> bool {
    path.is_dir()
}

/// Interactively asks for the vault directory on stdin/stdout.
fn prompt_vault_path() -> Result<PathBuf, String> {
    use std::io::{BufRead, Write};
    let mut out = io::stdout();
    write!(out, "Vault path (directory): ").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    validate_vault_dir(&line)
}

/// Canonicalizes `path` so path comparisons become identity comparisons.
/// Falls back to a lexical absolutization (cwd-joined) when canonicalize
/// fails, so a not-yet-existing file keeps its usual open-error path.
/// A leading `~` expands to $HOME first (`~/notes`, or `~` alone);
/// `~user` is left literal (no passwd lookup).
fn absolutize(path: &Path) -> PathBuf {
    let path = &expand_tilde(path);
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Expands a leading `~` to $HOME (`~/notes` → `$HOME/notes`, `~` →
/// `$HOME`), so `~` in the config `vault` key or on the CLI works like in
/// a shell. Without a resolvable $HOME the path is returned unchanged.
fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.as_bytes().first() != Some(&b'~') {
        return path.to_path_buf();
    }
    if text.len() > 1 && text.as_bytes()[1] != b'/' {
        return path.to_path_buf(); // `~user`: left literal
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join(text[1..].trim_start_matches('/')),
        _ => path.to_path_buf(),
    }
}

/// Where the vault comes from (SPEC.md "Data flow" step 1).
#[derive(Debug, PartialEq, Eq)]
enum VaultChoice {
    /// This path (a `[vaults]` name already resolved; not yet absolutized),
    /// and the setting it was read from.
    Path { path: PathBuf, source: VaultSource },
    /// Two or more `[vaults]` entries: ask in the vault picker.
    Picker,
    /// Nothing to go on and no file argument: prompt on stdin.
    Prompt,
    /// A file argument without a vault: the cwd indexes.
    None,
}

/// The setting a [`VaultChoice::Path`] came from, named in the not-found
/// error. A `--vault`/`vault` value that is a `[vaults]` name reports the
/// setting the user typed, not the entry.
#[derive(Debug, PartialEq, Eq)]
enum VaultSource {
    /// `--vault`.
    Flag,
    /// The config `vault` key.
    Key,
    /// The only `[vaults]` entry, by its name.
    Entry(String),
}

fn choose_vault(flag: Option<&Path>, has_file: bool, config: &config::Config) -> VaultChoice {
    let resolve = |value: &Path, source: VaultSource| VaultChoice::Path {
        path: resolve_vault_name(value, &config.vaults),
        source,
    };
    if let Some(flag) = flag {
        return resolve(flag, VaultSource::Flag);
    }
    if !has_file && config.vaults.len() >= 2 {
        return VaultChoice::Picker;
    }
    if let Some(key) = &config.vault {
        return resolve(key, VaultSource::Key);
    }
    if config.vaults.len() == 1 {
        if let Some((name, path)) = config.vaults.iter().next() {
            return VaultChoice::Path {
                path: path.clone(),
                source: VaultSource::Entry(name.clone()),
            };
        }
    }
    if has_file {
        VaultChoice::None
    } else {
        VaultChoice::Prompt
    }
}

/// The startup error for a chosen vault path that is not a directory.
/// `path` is shown as the setting wrote it (`~` kept). `main` prints it as
/// `error: …` with status 2 (SPEC.md "Data flow" step 1).
fn vault_not_found(path: &Path, source: &VaultSource) -> String {
    let source = match source {
        VaultSource::Flag => "--vault".to_string(),
        VaultSource::Key => "config vault".to_string(),
        VaultSource::Entry(name) => format!("config [vaults] {name}"),
    };
    format!("vault not found: {} ({source})", path.display())
}

/// A `--vault`/`vault` value: a `[vaults]` name resolves to its entry's
/// path; anything else is a path. A name wins over a same-named relative
/// path (`./work` reaches the directory).
fn resolve_vault_name(
    value: &Path,
    vaults: &std::collections::BTreeMap<String, PathBuf>,
) -> PathBuf {
    value
        .to_str()
        .and_then(|name| vaults.get(name))
        .unwrap_or(&value.to_path_buf())
        .clone()
}

/// `$HOME`, when it is set and not empty.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `path` as the picker lists paths: `$HOME` shown as `~`.
fn display_path(path: &Path) -> String {
    vault_picker::home_relative(path, home_dir().as_deref())
}

/// The picker's entries, in name order, `~` expanded.
fn picker_entries(config: &config::Config) -> Vec<vault_picker::VaultEntry> {
    let home = home_dir();
    config
        .vaults
        .iter()
        .map(|(name, path)| {
            vault_picker::VaultEntry::new(name.clone(), expand_tilde(path), home.as_deref())
        })
        .collect()
}

/// The picker's preselected entry (`vault_picker::preselect` owns the
/// order). The `vault` key's `~` is expanded first, as the entries' paths
/// are, so a `~` path key finds its entry.
fn picker_preselection(entries: &[vault_picker::VaultEntry], config: &config::Config) -> usize {
    let last = session::load_last_vault();
    let key = config.vault.as_deref().map(expand_tilde);
    vault_picker::preselect(entries, last.as_deref(), key.as_deref())
}

/// The UI palette of the startup theme, for the picker (drawn before the
/// App exists). Theme warnings are left to the App's own resolution.
fn picker_palette(theme_name: &str) -> theme::UiTheme {
    let custom = theme::discover_custom_themes(&theme::default_theme_dirs());
    theme::resolve_theme(theme_name, &custom)
        .or_else(|_| theme::resolve_theme(theme::DEFAULT_THEME, &custom))
        .map(|resolved| resolved.theme.ui)
        .unwrap_or_else(|_| theme::preset_theme(Default::default()).ui)
}

/// The startup picker loop. `Ok(None)` = the user quit.
fn run_picker(
    terminal: &mut Tui,
    picker: &mut vault_picker::PickerState,
    ui: theme::UiTheme,
) -> io::Result<Option<PathBuf>> {
    loop {
        terminal.draw(|f| ui::draw_vault_picker(f, picker, ui))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match picker.handle_key(key) {
                vault_picker::PickerFlow::Continue => {}
                vault_picker::PickerFlow::Open(path) => return Ok(Some(path)),
                vault_picker::PickerFlow::Close | vault_picker::PickerFlow::Quit => {
                    return Ok(None)
                }
            }
        }
    }
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("usage: frond [--vault <dir|name>] [--theme <name>] [--watch[=true|false]] [--mouse[=true|false]] [file.md | file.base]");
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };
    // Informational flags: print and exit before any terminal setup, so
    // output can be redirected and no alternate-screen state leaks.
    match args.info {
        Some(Info::Help) => {
            print!("{HELP_TEXT}");
            return;
        }
        Some(Info::Version) => {
            println!("frond {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some(Info::ConfigShow) => {
            print!("{}", config::DEFAULT_CONFIG_TEMPLATE);
            return;
        }
        None => {}
    }

    // Config file (~/.config/frond/config.toml; FROND_CONFIG overrides for
    // tests/smokes). Missing = defaults; malformed = warning + defaults.
    let (config, config_warnings) = match config::config_path() {
        Some(path) => config::load(&path),
        None => (config::Config::default(), Vec::new()),
    };
    for warning in &config_warnings {
        eprintln!("warning: {warning}");
    }

    // Precedence: CLI flags > config values (theme: flag > FROND_THEME env
    // > config > default).
    let theme_name = theme::effective_theme_name(
        args.theme.as_deref(),
        std::env::var("FROND_THEME").ok().as_deref(),
        config.theme.as_deref(),
    );
    let mouse = effective_mouse(args.mouse, config.mouse);
    install_panic_hook();
    // The picker runs in the TUI, so the terminal is set up before the
    // vault is known; the App then reuses it.
    let mut early_terminal: Option<Tui> = None;
    let vault = match choose_vault(args.vault.as_deref(), args.file.is_some(), &config) {
        VaultChoice::Path { path, source } => {
            // Checked before the terminal and the index: a missing vault is
            // one line and status 2, not an index warning after the TUI.
            let root = absolutize(&path);
            if !is_vault_dir(&root) {
                eprintln!("error: {}", vault_not_found(&path, &source));
                std::process::exit(2);
            }
            Some(root)
        }
        VaultChoice::Picker => {
            let entries = picker_entries(&config);
            let pre = picker_preselection(&entries, &config);
            let mut picker = vault_picker::PickerState::new(entries, pre);
            let mut terminal = setup_terminal_or_exit(mouse);
            match run_picker(&mut terminal, &mut picker, picker_palette(&theme_name)) {
                Ok(Some(v)) => {
                    early_terminal = Some(terminal);
                    Some(absolutize(&v))
                }
                Ok(None) => {
                    // Esc/Ctrl+C: a quiet exit, status 0.
                    let _ = restore_terminal(&mut terminal);
                    return;
                }
                Err(err) => {
                    let _ = restore_terminal(&mut terminal);
                    eprintln!("error: {err}");
                    std::process::exit(1);
                }
            }
        }
        // Before raw mode, so the line editor works normally.
        VaultChoice::Prompt => match prompt_vault_path() {
            Ok(v) => Some(v),
            Err(msg) => {
                eprintln!("error: {msg}");
                std::process::exit(2);
            }
        },
        VaultChoice::None => None,
    };
    let watch = args.watch.unwrap_or_else(|| config.watch_default());
    // Mouse capture: CLI flag > config `mouse` key > on (the historical
    // default). Off = the terminal keeps the mouse for native text
    // selection; `M` toggles at runtime either way.

    // Default the note to the vault's index, matching Obsidian's entry note.
    // Whether the note came from an explicit argument matters later (an
    // explicit file wins over the session restore), so capture it first.
    let explicit_note = args.file.is_some();
    let file = match (args.file, &vault) {
        (Some(f), _) => absolutize(&f),
        (None, Some(v)) => absolutize(&v.join("index.md")),
        (None, None) => {
            eprintln!("error: missing file argument");
            std::process::exit(2);
        }
    };

    // Vault index: `--vault`/config wins; otherwise the cwd is the vault
    // root (the opened file's ancestors are NOT walked up). A failed build
    // disables wikilink resolution/backlinks/tags rather than the app.
    let index: Option<SharedIndex> = {
        let root = vault
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        match build_index(&root) {
            Ok(index) => Some(index),
            Err(err) => {
                eprintln!("warning: no vault index ({err}); wikilinks/tags/backlinks disabled");
                None
            }
        }
    };

    let mut warnings = Vec::new();
    let display = startup_display(&config, theme_name, mouse, watch, &mut warnings);
    let vaults = {
        let entries = picker_entries(&config);
        let startup_preselect = picker_preselection(&entries, &config);
        vault_picker::VaultList {
            entries,
            startup_preselect,
        }
    };

    let mut terminal = match early_terminal {
        Some(terminal) => terminal,
        None => setup_terminal_or_exit(mouse),
    };

    let first = Start {
        vault,
        file,
        explicit_note,
        index,
    };
    let result = start_app(first, &display, &config, &vaults, &mut warnings).and_then(|app| {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        let mut app = app;
        // A switch is a clean quit of the open vault (`run_app` saves it)
        // followed by a start of the chosen one through `start_app`, the
        // same path the first start took.
        loop {
            match run_app(&mut terminal, app, &mut |root, display| {
                switch_vault(root, display, &config, &vaults)
            })? {
                Outcome::Quit => return Ok(()),
                Outcome::Switch(next) => app = *next,
            }
        }
    });

    if let Err(err) = restore_terminal(&mut terminal) {
        eprintln!("error restoring terminal: {err}");
    }
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

/// The vault index for `root`.
fn build_index(root: &Path) -> io::Result<SharedIndex> {
    VaultIndex::build(root).map(|index| Rc::new(RefCell::new(index)))
}

/// What a start opens: the first start's from the arguments, a switch's
/// the chosen vault with no note argument.
struct Start {
    vault: Option<PathBuf>,
    file: PathBuf,
    /// The note came from an argument (it wins over the session restore).
    explicit_note: bool,
    index: Option<SharedIndex>,
}

/// The startup display state, from the flags and the config. Config
/// values that do not parse leave a warning in `warnings`.
fn startup_display(
    config: &config::Config,
    theme: String,
    mouse: bool,
    watch: bool,
    warnings: &mut Vec<String>,
) -> app::DisplayState {
    // Theme group for `Shift+T` (config `theme_rotation`; `Ctrl+T` changes
    // it for the session). An unknown value warns and keeps "all".
    let mut theme_rotation = theme::ThemeRotation::default();
    if let Some(value) = config.theme_rotation.as_deref() {
        match theme::ThemeRotation::parse(value) {
            Some(rotation) => theme_rotation = rotation,
            None => warnings.push(format!(
                "config theme_rotation \"{value}\" is not all, light or dark; using all"
            )),
        }
    }
    app::DisplayState {
        theme,
        theme_rotation,
        // Mouse capture default (config/flag): the App field drives the
        // runtime toggle (`M`); setup_terminal already applied the startup
        // state.
        mouse_capture: mouse,
        watch,
        // Sidebar width override (config `sidebar_width`), same bounds as
        // the `<`/`>` keys clamp to.
        sidebar_width: config
            .sidebar_width
            .map(|w| w.clamp(app::SIDEBAR_MIN_WIDTH, app::SIDEBAR_MAX_WIDTH)),
        sidebar_visible: true,
        // Metadata block default (config `metadata` key; `m` toggles).
        show_metadata: config.metadata.unwrap_or(true),
        // Line-number gutter (config `line_numbers`, default off).
        line_numbers: config.line_numbers.unwrap_or(false),
        // Note margin (config `margin`, blank columns per side).
        note_margin: config
            .margin
            .unwrap_or(config::DEFAULT_MARGIN)
            .min(config::MAX_MARGIN),
        // Vertical note margin (config `margin_y`, blank rows above and
        // below the note text).
        note_margin_y: effective_margin_y(config.margin_y),
    }
}

/// Builds the App for a start: the first start and every vault switch
/// come through here. Warnings meant for stderr land in `warnings` (the
/// first start prints them; a switch has no stderr to print to).
fn start_app(
    start: Start,
    display: &app::DisplayState,
    config: &config::Config,
    vaults: &vault_picker::VaultList,
    warnings: &mut Vec<String>,
) -> io::Result<App> {
    let width = crossterm::terminal::size()
        .map(|(w, _)| w.saturating_sub(2).max(20))
        .unwrap_or(FALLBACK_SIZE.0);
    // The note is the start's first read: a failure names it (the message
    // `main` prints after the terminal is restored, SPEC "Data flow" step 1).
    let viewer = Viewer::open(&start.file, width as usize).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("cannot open {}: {err}", display_path(&start.file)),
        )
    })?;
    let mut app = App::new(viewer, start.vault.clone(), start.index)?;
    app.vaults = vaults.clone();
    // Theme: presets + discovered custom themes; the resolved name
    // positions the `Shift+T` cycle. The selected theme's own warnings
    // (unknown keys, unknown theme) go to stderr AND the status bar; other
    // themes' load errors stay stderr-only so a broken custom file cannot
    // spam the status bar at startup. Never silent, never fatal.
    let dirs = theme::default_theme_dirs();
    let custom = theme::discover_custom_themes(&dirs);
    let (resolved, theme_warnings, load_errors) =
        app.install_theme_choices(&display.theme, &dirs, &custom);
    warnings.extend(theme_warnings.iter().chain(&load_errors).cloned());
    app.apply_theme(&resolved, &theme_warnings);
    // Status-bar icons (config `icons`). An unknown value warns and keeps
    // the default, nerd.
    if let Some(value) = config.icons.as_deref() {
        match config::IconSet::parse(value) {
            Some(icons) => app.icons = icons,
            None => warnings.push(format!(
                "config icons \"{value}\" is not nerd or unicode; using nerd"
            )),
        }
    }
    app.apply_display(display);
    // Session restore: only when the note came from the defaults (no
    // explicit file argument — that argument is the user's intent for this
    // run). A missing or malformed session file is the normal first-run
    // state, never a warning.
    if !start.explicit_note {
        if let Some(root) = &start.vault {
            if let Some(file) =
                session::session_file(root).and_then(|path| session::SessionState::load(&path))
            {
                app.restore_session(&file, root);
            }
        }
    }
    // Config hot-reload: the theme key live-applies on change.
    if let Some(path) = config::config_path() {
        app.watch_config(path);
    }
    // A `.base` file argument renders its table view instead of the
    // markdown viewer (the Viewer parsed it as plain text, harmlessly
    // unused from here on).
    if baseview::is_base_file(&start.file) {
        if let Err(err) = app.open_base_at(&start.file, 0) {
            app.viewer.last_error = Some(format!("open failed: {err}"));
        }
    }
    Ok(app)
}

/// The App for the vault the in-app picker chose, built while the open
/// one is still untouched: a vault that cannot be indexed (or opened) is
/// an error for the picker, not a warning, so nothing is left half-open.
fn switch_vault(
    root: PathBuf,
    display: app::DisplayState,
    config: &config::Config,
    vaults: &vault_picker::VaultList,
) -> io::Result<App> {
    let root = absolutize(&root);
    let index = build_index(&root)?;
    let start = Start {
        file: absolutize(&root.join("index.md")),
        vault: Some(root),
        explicit_note: false,
        index: Some(index),
    };
    start_app(start, &display, config, vaults, &mut Vec::new())
}

fn setup_terminal_or_exit(capture_mouse: bool) -> Tui {
    match setup_terminal(capture_mouse) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("cannot enter terminal mode: {err}");
            // Setup may have partially succeeded (e.g. raw mode on, alt
            // screen failed): best-effort restore before exiting.
            if let Err(restore_err) = restore_terminal_state() {
                eprintln!("error restoring terminal: {restore_err}");
            }
            std::process::exit(1);
        }
    }
}

fn setup_terminal(capture_mouse: bool) -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    if capture_mouse {
        stdout.execute(event::EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// Best-effort teardown of the terminal *state* (raw mode, alternate screen,
/// mouse capture). Every step runs even if an earlier one fails; the first
/// error is returned. Usable without a `Terminal`, so it also covers the
/// partial-setup failure path in `main`.
fn restore_terminal_state() -> io::Result<()> {
    let mut first_err: Option<io::Error> = None;
    if let Err(err) = disable_raw_mode() {
        first_err = first_err.or(Some(err));
    }
    let mut stdout = io::stdout();
    if let Err(err) = stdout.execute(LeaveAlternateScreen) {
        first_err = first_err.or(Some(err));
    }
    if let Err(err) = stdout.execute(event::DisableMouseCapture) {
        first_err = first_err.or(Some(err));
    }
    match first_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn restore_terminal(terminal: &mut Tui) -> io::Result<()> {
    // Best-effort across all steps: errors are combined instead of
    // short-circuiting, so e.g. a `disable_raw_mode` failure cannot skip
    // leaving the alternate screen or showing the cursor.
    let mut errors: Vec<String> = Vec::new();
    if let Err(err) = restore_terminal_state() {
        errors.push(err.to_string());
    }
    if let Err(err) = terminal.show_cursor() {
        errors.push(err.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(errors.join("; ")))
    }
}

/// Restores the terminal even on panic, then chains to the previous hook.
fn install_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal_state();
        prev_hook(info);
    }));
}

/// How `run_app` ended.
enum Outcome {
    Quit,
    /// The in-app vault picker chose another vault: its App, built.
    Switch(Box<App>),
}

/// The event loop for one vault. `switch` builds the App for a vault the
/// in-app picker chose; until it succeeds the open vault is untouched (its
/// error goes into the picker). Either way out, the open vault is left as
/// a clean quit leaves it.
fn run_app(
    terminal: &mut Tui,
    mut app: App,
    switch: &mut dyn FnMut(PathBuf, app::DisplayState) -> io::Result<App>,
) -> io::Result<Outcome> {
    let mut outcome = Outcome::Quit;
    let (cols, rows) = crossterm::terminal::size().unwrap_or(FALLBACK_SIZE);
    app.viewer.on_resize(cols, rows);

    // Deadline-based watch polling: the file is stat-ed whenever 250 ms have
    // elapsed, regardless of the event stream. A pure timeout-reset tick
    // would starve the poll while events (e.g. mouse motion) keep arriving.
    let mut last_poll = Instant::now();
    // Live mouse-capture state, kept in sync with `app.mouse_capture` (the
    // `M` toggle): when they diverge, the switch is executed here — the
    // App cannot write to the terminal itself.
    let mut capture_on = app.mouse_capture;

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if app.mouse_capture != capture_on {
            let mut stdout = io::stdout();
            let res = if app.mouse_capture {
                stdout.execute(event::EnableMouseCapture)
            } else {
                stdout.execute(event::DisableMouseCapture)
            };
            // Best-effort: a failed switch leaves the current mode; the
            // next toggle retries. Not worth tearing down the session.
            let _ = res;
            capture_on = app.mouse_capture;
        }

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match app.handle_key(key) {
                    app::Flow::Continue => {}
                    app::Flow::Quit => break,
                    app::Flow::SwitchVault(root) => match switch(root, app.display_state()) {
                        Ok(next) => {
                            outcome = Outcome::Switch(Box::new(next));
                            break;
                        }
                        Err(err) => app.vault_switch_failed(err),
                    },
                },
                Event::Mouse(mouse) => {
                    // Capture off: the status bar promises the app ignores
                    // the mouse, so gate here too — it must not depend on
                    // the terminal actually withholding events.
                    if !app.mouse_capture {
                        continue;
                    }
                    // Every mouse event carries the pointer's cell; the bars'
                    // hover highlight follows it, so record it before the
                    // event's own action.
                    app.on_mouse_position(mouse.column, mouse.row);
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            app.handle_wheel_at(mouse.column, mouse.row, true)
                        }
                        MouseEventKind::ScrollUp => {
                            app.handle_wheel_at(mouse.column, mouse.row, false)
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.handle_press(mouse.column, mouse.row)
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            app.handle_drag(mouse.column, mouse.row)
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            // A release that ends a scrollbar drag is not a
                            // click (it must not open a link under the
                            // pointer); the App decides, the loop just
                            // flushes any copy the click requested.
                            app.handle_release(mouse.column, mouse.row);
                            app.flush_clipboard(&mut io::stdout());
                        }
                        _ => {}
                    }
                }
                Event::Resize(cols, rows) => app.viewer.on_resize(cols, rows),
                _ => {}
            }
        }
        if last_poll.elapsed() >= POLL_INTERVAL {
            app.poll();
            last_poll = Instant::now();
        }
    }
    // Clean quit: persist the session (last note + scroll, sidebar mode,
    // selection and expansion) and the last vault (the picker's
    // preselection) for the next run. Best-effort — a failed write must
    // never turn a quit into an error.
    if let Some(root) = app.vault_root() {
        if let Some(path) = session::session_file(root) {
            let _ = app.session_state(root).save(&path);
        }
        let _ = session::save_last_vault(root);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn mouse_flag_parses_and_defaults() {
        assert!(parse(&["--mouse"]).unwrap().mouse.unwrap());
        assert!(!parse(&["--mouse=false"]).unwrap().mouse.unwrap());
        assert!(parse(&["--mouse=true"]).unwrap().mouse.unwrap());
        assert_eq!(parse(&["x.md"]).unwrap().mouse, None, "unset by default");
        assert!(parse(&["--mouse=maybe"]).is_err(), "boolean values only");
    }

    #[test]
    fn info_flags_parse_without_value_consumption() {
        assert_eq!(parse(&["--help"]).unwrap().info, Some(Info::Help));
        assert_eq!(parse(&["-h"]).unwrap().info, Some(Info::Help));
        assert_eq!(parse(&["--version"]).unwrap().info, Some(Info::Version));
        assert_eq!(parse(&["-V"]).unwrap().info, Some(Info::Version));
        assert_eq!(
            parse(&["--config-show"]).unwrap().info,
            Some(Info::ConfigShow)
        );
        assert_eq!(parse(&["x.md"]).unwrap().info, None);
        // The flag does not swallow the positional file.
        let args = parse(&["--config-show", "x.md"]).unwrap();
        assert_eq!(args.info, Some(Info::ConfigShow));
        assert_eq!(args.file.as_deref(), Some(Path::new("x.md")));
    }

    /// The space-form mistake (`--mouse false`) must error with the
    /// `=`-form hint instead of `false` becoming the file argument — and
    /// `frond --mouse note.md` (next token not a bool) keeps working.
    #[test]
    fn boolean_flags_reject_the_space_form() {
        assert!(parse(&["--mouse", "false"]).is_err());
        assert!(parse(&["--watch", "false"]).is_err());
        // A non-bool next token is NOT consumed: bare flag + positional.
        let args = parse(&["--mouse", "x.md"]).unwrap();
        assert_eq!(args.mouse, Some(true));
        assert_eq!(args.file.as_deref(), Some(Path::new("x.md")));
    }

    #[test]
    fn mouse_precedence_is_flag_over_config_over_on() {
        assert!(!effective_mouse(Some(false), Some(true)), "flag wins");
        assert!(effective_mouse(Some(true), Some(false)), "flag wins");
        assert!(!effective_mouse(None, Some(false)), "config next");
        assert!(effective_mouse(None, None), "default on");
    }

    #[test]
    fn vertical_margin_defaults_to_one_and_clamps_to_five() {
        assert_eq!(effective_margin_y(None), 1, "absent key: default");
        assert_eq!(effective_margin_y(Some(0)), 0);
        assert_eq!(effective_margin_y(Some(3)), 3);
        assert_eq!(effective_margin_y(Some(5)), 5);
        assert_eq!(effective_margin_y(Some(6)), 5, "over the max: clamped");
    }

    #[test]
    fn absolutize_resolves_existing_paths_and_symlinks() {
        let dir = TempDir::new("absz-dir");
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let file = dir.path().join("note.md");
        std::fs::write(&file, "x\n").unwrap();
        // Existing file with noisy spelling: identical to what canonicalize
        // reports (resolves `.`/`..` components and symlinks), so the
        // same-note guard in `open_note` compares identity, not spelling.
        let weird = dir.path().join(".").join("sub").join("..").join("note.md");
        assert_eq!(absolutize(&weird), file.canonicalize().unwrap());
        // Vault root: canonical form too.
        assert_eq!(absolutize(dir.path()), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn validate_vault_dir_accepts_existing_directory() {
        let dir = TempDir::new("vdir-ok");
        let input = format!("  {}\n", dir.path().display());
        assert_eq!(
            validate_vault_dir(&input).unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn validate_vault_dir_rejects_empty_missing_and_files() {
        assert!(validate_vault_dir("   \n").is_err());
        assert!(validate_vault_dir("/nonexistent/frond/vault").is_err());
        let dir = TempDir::new("vdir-file");
        let file = dir.path().join("note.md");
        std::fs::write(&file, "x\n").unwrap();
        assert!(validate_vault_dir(&file.display().to_string()).is_err());
    }

    #[test]
    fn absolutize_falls_back_when_canonicalize_fails() {
        // Missing file: lexical absolutization against the cwd.
        let missing = PathBuf::from("does-not-exist-frond.md");
        assert_eq!(
            absolutize(&missing),
            std::env::current_dir().unwrap().join(&missing)
        );
        // Already-absolute missing path: returned unchanged.
        let abs_missing = PathBuf::from("/nonexistent/frond/missing.md");
        assert_eq!(absolutize(&abs_missing), abs_missing);
        // Relative spelling with dot components: the fallback is lexical
        // (cwd-joined verbatim, no normalization), so a not-yet-existing
        // file keeps its usual open-error path.
        let dotted = PathBuf::from("./sub/../other.md");
        assert_eq!(
            absolutize(&dotted),
            std::env::current_dir().unwrap().join(&dotted)
        );
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = TempDir::new("tilde-home");
        let mut env = testutil::EnvGuard::lock();
        env.set("HOME", home.path());
        assert_eq!(
            expand_tilde(Path::new("~/notes")),
            home.path().join("notes")
        );
        assert_eq!(expand_tilde(Path::new("~")), home.path());
        // `~user` and plain paths stay literal.
        assert_eq!(expand_tilde(Path::new("~other/x")), Path::new("~other/x"));
        assert_eq!(expand_tilde(Path::new("rel/x")), Path::new("rel/x"));
        // No usable $HOME: unchanged.
        env.remove("HOME");
        assert_eq!(expand_tilde(Path::new("~/notes")), Path::new("~/notes"));
    }

    fn config_with(vault: Option<&str>, vaults: &[(&str, &str)]) -> config::Config {
        config::Config {
            vault: vault.map(PathBuf::from),
            vaults: vaults
                .iter()
                .map(|(name, path)| (name.to_string(), PathBuf::from(path)))
                .collect(),
            ..config::Config::default()
        }
    }

    fn chosen(path: &str, source: VaultSource) -> VaultChoice {
        VaultChoice::Path {
            path: PathBuf::from(path),
            source,
        }
    }

    #[test]
    fn vault_choice_follows_the_spec_order() {
        let two = config_with(Some("work"), &[("home", "~/h"), ("work", "/w")]);
        // --vault wins, a name resolving through [vaults].
        assert_eq!(
            choose_vault(Some(Path::new("home")), false, &two),
            chosen("~/h", VaultSource::Flag)
        );
        assert_eq!(
            choose_vault(Some(Path::new("/x")), false, &two),
            chosen("/x", VaultSource::Flag)
        );
        // Two or more entries, no file: the picker, even with a vault key.
        assert_eq!(choose_vault(None, false, &two), VaultChoice::Picker);
        // A file argument skips the picker: the vault key, by name.
        assert_eq!(
            choose_vault(None, true, &two),
            chosen("/w", VaultSource::Key)
        );
        let two_no_key = config_with(None, &[("home", "~/h"), ("work", "/w")]);
        assert_eq!(choose_vault(None, true, &two_no_key), VaultChoice::None);
        // Exactly one entry and no key: that entry, file or not.
        let one = config_with(None, &[("home", "~/h")]);
        let home_entry = || chosen("~/h", VaultSource::Entry("home".to_string()));
        assert_eq!(choose_vault(None, false, &one), home_entry());
        assert_eq!(choose_vault(None, true, &one), home_entry());
        // One entry and a key: the key.
        let one_key = config_with(Some("/k"), &[("home", "~/h")]);
        assert_eq!(
            choose_vault(None, false, &one_key),
            chosen("/k", VaultSource::Key)
        );
        // Zero entries: today's behaviour.
        let none = config_with(None, &[]);
        assert_eq!(choose_vault(None, false, &none), VaultChoice::Prompt);
        assert_eq!(choose_vault(None, true, &none), VaultChoice::None);
        assert_eq!(
            choose_vault(None, false, &config_with(Some("/k"), &[])),
            chosen("/k", VaultSource::Key)
        );
    }

    #[test]
    fn a_missing_vault_names_the_setting_it_came_from() {
        // Each source's line, with `~` kept as written.
        assert_eq!(
            vault_not_found(Path::new("~/gone"), &VaultSource::Flag),
            "vault not found: ~/gone (--vault)"
        );
        assert_eq!(
            vault_not_found(Path::new("/srv/gone"), &VaultSource::Key),
            "vault not found: /srv/gone (config vault)"
        );
        assert_eq!(
            vault_not_found(Path::new("~/gone"), &VaultSource::Entry("work".to_string())),
            "vault not found: ~/gone (config [vaults] work)"
        );
        // A `--vault` name is reported as the typed setting, with the path
        // it resolved to.
        let two = config_with(None, &[("work", "~/w"), ("home", "~/h")]);
        let VaultChoice::Path { path, source } = choose_vault(Some(Path::new("work")), false, &two)
        else {
            panic!("--vault names a path")
        };
        assert_eq!(
            vault_not_found(&path, &source),
            "vault not found: ~/w (--vault)"
        );
    }

    #[test]
    fn a_start_note_that_cannot_be_opened_names_the_file() {
        let (_env, _state) = isolated_env("start-note-missing");
        let dir = vault_dir("start-note-missing");
        let note = dir.path().join("gone.md");
        let start = Start {
            vault: Some(dir.path().to_path_buf()),
            file: note.clone(),
            explicit_note: true,
            index: None,
        };
        let config = config::Config::default();
        let display = startup_display(&config, "all".to_string(), true, true, &mut Vec::new());
        let err = start_app(
            start,
            &display,
            &config,
            &vault_picker::VaultList::default(),
            &mut Vec::new(),
        )
        .err()
        .expect("a missing note does not start");
        let prefix = format!("cannot open {}: ", display_path(&note));
        assert!(
            err.to_string().starts_with(&prefix),
            "got: {err}, expected prefix: {prefix}"
        );
    }

    #[test]
    fn a_name_wins_over_a_same_named_relative_path() {
        let vaults = config_with(None, &[("work", "/srv/work")]).vaults;
        assert_eq!(
            resolve_vault_name(Path::new("work"), &vaults),
            Path::new("/srv/work")
        );
        assert_eq!(
            resolve_vault_name(Path::new("./work"), &vaults),
            Path::new("./work")
        );
        assert_eq!(
            resolve_vault_name(Path::new("other"), &vaults),
            Path::new("other")
        );
    }

    #[test]
    fn picker_entries_expand_and_redisplay_the_tilde() {
        let home = TempDir::new("picker-home");
        let mut env = testutil::EnvGuard::lock();
        env.set("HOME", home.path());
        let config = config_with(None, &[("b", "~/notes"), ("a", "/srv/a")]);
        let entries = picker_entries(&config);
        env.remove("HOME");
        assert_eq!(entries[0].name, "a", "name order");
        assert_eq!(entries[1].path, home.path().join("notes"));
        assert_eq!(entries[1].display, "~/notes");
        assert_eq!(entries[0].display, "/srv/a");
    }

    #[test]
    fn picker_preselects_a_tilde_vault_key_by_its_path() {
        let home = TempDir::new("preselect-home");
        std::fs::create_dir_all(home.path().join("notes")).unwrap();
        std::fs::create_dir_all(home.path().join("work")).unwrap();
        let mut env = testutil::EnvGuard::lock();
        env.set("HOME", home.path());
        // No last-vault file: the key alone decides.
        env.set("FROND_SESSION", home.path().join("state"));
        let config = config_with(Some("~/work"), &[("notes", "~/notes"), ("work", "~/work")]);
        let entries = picker_entries(&config);
        assert_eq!(
            picker_preselection(&entries, &config),
            1,
            "the entry at ~/work (name order: notes, work), not the first"
        );
    }

    /// A switch reads the config, session and theme paths: point them at a
    /// temp dir under the env lock, so no test touches the developer's
    /// `~/.config` or `~/.local/state`. The guard is returned first so it
    /// outlives the test's use of the temp dir.
    fn isolated_env(name: &str) -> (testutil::EnvGuard, TempDir) {
        let dir = TempDir::new(name);
        let mut env = testutil::EnvGuard::lock();
        env.set("FROND_CONFIG", dir.path().join("config.toml"));
        env.set("FROND_SESSION", dir.path().join("state"));
        env.set("FROND_THEMES", dir.path().join("themes"));
        (env, dir)
    }

    fn vault_dir(name: &str) -> crate::testutil::TempDir {
        let dir = crate::testutil::TempDir::new(name);
        std::fs::write(dir.path().join("index.md"), format!("# {name}\n")).unwrap();
        dir
    }

    fn open_app(dir: &crate::testutil::TempDir) -> App {
        let root = dir.path().to_path_buf();
        let index = build_index(&root).unwrap();
        let viewer = Viewer::open(&root.join("index.md"), 80).unwrap();
        App::new(viewer, Some(root), Some(index)).unwrap()
    }

    #[test]
    fn a_switch_opens_the_chosen_vault_with_the_current_display_state() {
        let (_env, _state) = isolated_env("switch-state");
        let alpha = vault_dir("switch-alpha");
        let beta = vault_dir("switch-beta");
        let mut old = open_app(&alpha);
        let display = app::DisplayState {
            theme: "nord".to_string(),
            theme_rotation: theme::ThemeRotation::Light,
            mouse_capture: false,
            watch: false,
            sidebar_width: Some(41),
            sidebar_visible: false,
            show_metadata: false,
            line_numbers: true,
            note_margin: 4,
            note_margin_y: 2,
        };
        // As the user left them: toggled, not the config's values.
        let (resolved, warnings, _) =
            old.install_theme_choices(&display.theme, &[], &Default::default());
        old.apply_theme(&resolved, &warnings);
        old.apply_display(&display);
        let vaults = vault_picker::VaultList::default();
        let new = switch_vault(
            beta.path().to_path_buf(),
            old.display_state(),
            &config::Config::default(),
            &vaults,
        )
        .unwrap();
        assert_eq!(new.display_state(), display);
        assert_eq!(new.theme_name(), "nord");
        assert_eq!(new.vault_root(), Some(absolutize(beta.path()).as_path()));
        assert_eq!(new.viewer.path, absolutize(beta.path()).join("index.md"));
        assert_eq!(
            old.vault_root(),
            Some(alpha.path()),
            "the old App is untouched"
        );
    }

    #[test]
    fn a_vault_that_cannot_be_indexed_is_an_error_for_the_picker() {
        let (_env, _state) = isolated_env("switch-fail-state");
        let alpha = vault_dir("switch-fail");
        let old = open_app(&alpha);
        let gone = alpha.path().join("gone");
        let result = switch_vault(
            gone,
            old.display_state(),
            &config::Config::default(),
            &vault_picker::VaultList::default(),
        );
        assert!(result.is_err());
    }
}
