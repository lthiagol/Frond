//! Read-only TUI shell with sidebar (tree/tags), backlinks panel,
//! wikilink navigation, navigation history, watch mode.
//!
//! Usage: `frond [--vault <dir>] [file.md]` — `frond --help` for all flags
//! (`--config-show` prints a commented default config file).
//!
//! With neither `--vault` (nor a config `vault`) nor a file argument, the app
//! prompts for the vault directory. A file argument without a vault is
//! opened on its own, without a sidebar; the vault index is then the
//! current directory. With a vault, `Tab` shows/hides the sidebar tree; `h`/`l` move focus between
//! sidebar and content; `Backspace`/`Ctrl+O` walk the navigation history.
//!
//! Keys: j/k, arrows, PgUp/PgDn, g/G, Home/End, mouse wheel to scroll;
//! `w` toggles watch; `s` is the global vault search; `Ctrl+P` is the
//! quick note switcher; `u` refreshes the
//! git sync status; `Shift+T` cycles the theme; `?` toggles the help
//! popup; `v`/`1`–`9` switch table views while a `.base` file is open;
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

Usage: frond [--vault <dir>] [--theme <name>] [file.md | file.base]

Options:
  --vault <dir>         Vault directory to index (config: vault)
  --theme <name>        Theme by name (config: theme)
  --watch[=true|false]  Live re-render on vault changes (config: watch)
  --mouse[=true|false]  Mouse capture on/off; off = native text selection
                        (config: mouse, `m` toggles at runtime)
  --config-show         Print a commented default config.toml and exit
  --version             Print the version and exit
  --help                Print this help and exit

With neither --vault (nor a config vault) nor a file argument, frond
prompts for the vault directory. Config file:
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
                    .ok_or("--vault requires a directory argument")?;
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
    if !expanded.is_dir() {
        return Err(format!("not a directory: {}", expanded.display()));
    }
    Ok(absolutize(&expanded))
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

fn main() {
    let mut args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("usage: frond [--vault <dir>] [--theme <name>] [--watch[=true|false]] [--mouse[=true|false]] [file.md | file.base]");
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
    let (mut config, config_warnings) = match config::config_path() {
        Some(path) => config::load(&path),
        None => (config::Config::default(), Vec::new()),
    };
    for warning in &config_warnings {
        eprintln!("warning: {warning}");
    }

    // Precedence: CLI flags > config values (theme: flag > FROND_THEME env
    // > config > default).
    // No vault from flags/config and no file argument: prompt for one
    // (before raw mode, so the line editor works normally).
    let vault = match args.vault.take().or(config.vault.take()) {
        Some(v) => Some(absolutize(&v)),
        None if args.file.is_none() => match prompt_vault_path() {
            Ok(v) => Some(v),
            Err(msg) => {
                eprintln!("error: {msg}");
                std::process::exit(2);
            }
        },
        None => None,
    };
    let watch = args.watch.unwrap_or_else(|| config.watch_default());
    // Mouse capture: CLI flag > config `mouse` key > on (the historical
    // default). Off = the terminal keeps the mouse for native text
    // selection; `m` toggles at runtime either way.
    let mouse = effective_mouse(args.mouse, config.mouse);
    let theme_name = theme::effective_theme_name(
        args.theme.as_deref(),
        std::env::var("FROND_THEME").ok().as_deref(),
        config.theme.as_deref(),
    );

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
        match VaultIndex::build(&root) {
            Ok(index) => Some(Rc::new(RefCell::new(index))),
            Err(err) => {
                eprintln!("warning: no vault index ({err}); wikilinks/tags/backlinks disabled");
                None
            }
        }
    };

    install_panic_hook();
    let mut terminal = match setup_terminal(mouse) {
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
    };

    let width = crossterm::terminal::size()
        .map(|(w, _)| w.saturating_sub(2).max(20))
        .unwrap_or(FALLBACK_SIZE.0);
    let result = Viewer::open(&file, width as usize).and_then(|mut viewer| {
        viewer.watch = watch;
        App::new(viewer, vault.clone(), index).and_then(|mut app| {
            // Theme: presets + discovered custom themes; the resolved name
            // positions the `Shift+T` cycle. The selected theme's own
            // warnings (unknown keys, unknown theme) go to stderr AND the
            // status bar; other themes' load errors stay stderr-only so a
            // broken custom file cannot spam the status bar at startup.
            // Never silent, never fatal.
            let dirs = theme::default_theme_dirs();
            let custom = theme::discover_custom_themes(&dirs);
            let (resolved, theme_warnings, load_errors) =
                app.install_theme_choices(&theme_name, &dirs, &custom);
            for warning in theme_warnings.iter().chain(&load_errors) {
                eprintln!("warning: {warning}");
            }
            app.apply_theme(&resolved, &theme_warnings);
            // Theme group for `Shift+T` (config `theme_rotation`; `Ctrl+T`
            // changes it for the session). An unknown value warns and keeps
            // "all".
            if let Some(value) = config.theme_rotation.as_deref() {
                match theme::ThemeRotation::parse(value) {
                    Some(rotation) => app.theme_rotation = rotation,
                    None => eprintln!(
                        "warning: config theme_rotation \"{value}\" is not all, light or dark; using all"
                    ),
                }
            }
            // Status-bar icons (config `icons`). An unknown value warns and
            // keeps the default, nerd.
            if let Some(value) = config.icons.as_deref() {
                match config::IconSet::parse(value) {
                    Some(icons) => app.icons = icons,
                    None => eprintln!(
                        "warning: config icons \"{value}\" is not nerd or unicode; using nerd"
                    ),
                }
            }
            // Note margin (config `margin`, blank columns per side).
            app.note_margin = config
                .margin
                .unwrap_or(config::DEFAULT_MARGIN)
                .min(config::MAX_MARGIN);
            // Vertical note margin (config `margin_y`, blank rows above and
            // below the note text).
            app.note_margin_y = effective_margin_y(config.margin_y);
            // Line-number gutter (config `line_numbers`, default off).
            app.line_numbers = config.line_numbers.unwrap_or(false);
            // Mouse capture default (config/flag): the App field drives the
            // runtime toggle (`m`); setup_terminal already applied the
            // startup state.
            app.mouse_capture = mouse;
            // Sidebar width override (config `sidebar_width`), same bounds
            // as the `<`/`>` keys clamp to.
            app.sidebar_width = config
                .sidebar_width
                .map(|w| w.clamp(crate::app::SIDEBAR_MIN_WIDTH, crate::app::SIDEBAR_MAX_WIDTH));
            // Metadata block default (config `metadata` key; `f` toggles).
            app.viewer.show_metadata = config.metadata.unwrap_or(true);
            // Session restore: only when the note came from the defaults
            // (no explicit file argument — that argument is the user's
            // intent for this run). A missing or malformed session file is
            // the normal first-run state, never a warning.
            if !explicit_note {
                if let Some(root) = &vault {
                    if let Some(file) = session::session_file(root)
                        .and_then(|path| session::SessionState::load(&path))
                    {
                        app.restore_session(&file, root);
                    }
                }
            }
            // Config hot-reload: the theme key live-applies on change.
            if let Some(path) = config::config_path() {
                app.watch_config(path);
            }
            // A `.base` file argument renders its table view instead of
            // the markdown viewer (the Viewer parsed it as plain text,
            // harmlessly unused from here on).
            if baseview::is_base_file(&file) {
                if let Err(err) = app.open_base_at(&file, 0) {
                    app.viewer.last_error = Some(format!("open failed: {err}"));
                }
            }
            run_app(&mut terminal, &mut app, vault.as_deref())
        })
    });

    if let Err(err) = restore_terminal(&mut terminal) {
        eprintln!("error restoring terminal: {err}");
    }
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
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

fn run_app(terminal: &mut Tui, app: &mut App, vault_root: Option<&Path>) -> io::Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or(FALLBACK_SIZE);
    app.viewer.on_resize(cols, rows);

    // Deadline-based watch polling: the file is stat-ed whenever 250 ms have
    // elapsed, regardless of the event stream. A pure timeout-reset tick
    // would starve the poll while events (e.g. mouse motion) keep arriving.
    let mut last_poll = Instant::now();
    // Live mouse-capture state, kept in sync with `app.mouse_capture` (the
    // `m` toggle): when they diverge, the switch is executed here — the
    // App cannot write to the terminal itself.
    let mut capture_on = app.mouse_capture;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

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
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let app::Flow::Quit = app.handle_key(key) {
                        break;
                    }
                }
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
    // selection and expansion) for the next run. Best-effort — a failed
    // write must never turn a quit into an error.
    if let Some(root) = vault_root {
        if let Some(path) = session::session_file(root) {
            let _ = app.session_state(root).save(&path);
        }
    }
    Ok(())
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
}
