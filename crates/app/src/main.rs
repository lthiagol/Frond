//! M3: read-only TUI shell with sidebar (tree/tags), backlinks panel,
//! wikilink navigation, navigation history, watch mode.
//!
//! Usage: `frond [--vault <dir>] [file.md]`
//!
//! With neither `--vault` (nor a config `vault`) nor a file argument, the app
//! prompts for the vault directory. A file argument without a vault is a
//! single-file viewer. With a vault, `Tab` shows/hides the sidebar tree; `h`/`l` move focus between
//! sidebar and content; `Backspace`/`Ctrl+O` walk the navigation history.
//!
//! Keys: j/k, arrows, PgUp/PgDn, g/G, Home/End, mouse wheel to scroll;
//! `w` toggles watch; `s` is the global vault search; `u` refreshes the
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
mod search;
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
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

/// Arg parsing over an explicit iterator (unit-testable; `parse_args`
/// feeds it the real argv).
fn parse_args_from(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut vault: Option<PathBuf> = None;
    let mut theme: Option<String> = None;
    let mut watch: Option<bool> = None;
    let mut file: Option<PathBuf> = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--vault" => {
                vault = Some(
                    args.next()
                        .ok_or("--vault requires a directory argument")?
                        .into(),
                )
            }
            _ if arg.starts_with("--vault=") => {
                vault = Some(arg["--vault=".len()..].to_string().into());
            }
            "--theme" => {
                theme = Some(
                    args.next()
                        .ok_or("--theme requires a name argument")?
                        .to_string(),
                )
            }
            _ if arg.starts_with("--theme=") => {
                theme = Some(arg["--theme=".len()..].to_string());
            }
            "--watch" => watch = Some(true),
            _ if arg.starts_with("--watch=") => {
                let value = arg["--watch=".len()..].to_string();
                watch = Some(
                    value
                        .parse::<bool>()
                        .map_err(|_| format!("--watch expects true or false, got \"{value}\""))?,
                );
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => {
                if file.is_some() {
                    return Err("only one file may be given".to_string());
                }
                file = Some(arg.into());
            }
        }
    }
    Ok(Args {
        file,
        vault,
        theme,
        watch,
    })
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
fn absolutize(path: &Path) -> PathBuf {
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

fn main() {
    let mut args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprintln!(
                "usage: frond [--vault <dir>] [--theme <name>] [--watch[=true|false]] [file.md | file.base]"
            );
            eprintln!("error: {msg}");
            std::process::exit(2);
        }
    };

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
    let theme_name = theme::effective_theme_name(
        args.theme.as_deref(),
        std::env::var("FROND_THEME").ok().as_deref(),
        config.theme.as_deref(),
    );

    // Default the note to the vault's index, matching Obsidian's entry note.
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
    let mut terminal = match setup_terminal() {
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
            run_app(&mut terminal, &mut app)
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

fn setup_terminal() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(event::EnableMouseCapture)?;
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

fn run_app(terminal: &mut Tui, app: &mut App) -> io::Result<()> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or(FALLBACK_SIZE);
    app.viewer.on_resize(cols, rows);

    // Deadline-based watch polling: the file is stat-ed whenever 250 ms have
    // elapsed, regardless of the event stream. A pure timeout-reset tick
    // would starve the poll while events (e.g. mouse motion) keep arriving.
    let mut last_poll = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(POLL_INTERVAL)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let app::Flow::Quit = app.handle_key(key) {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        app.handle_wheel_at(mouse.column, mouse.row, true)
                    }
                    MouseEventKind::ScrollUp => app.handle_wheel_at(mouse.column, mouse.row, false),
                    MouseEventKind::Up(MouseButton::Left) => {
                        app.handle_click(mouse.column, mouse.row);
                        app.flush_clipboard(&mut io::stdout());
                    }
                    _ => {}
                },
                Event::Resize(cols, rows) => app.viewer.on_resize(cols, rows),
                _ => {}
            }
        }
        if last_poll.elapsed() >= POLL_INTERVAL {
            app.poll();
            last_poll = Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

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
}
