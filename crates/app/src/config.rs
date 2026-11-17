//! `~/.config/frond/config.toml`: persistent settings read at startup.
//!
//! Keys: `theme` (string), `vault` (path), `watch` (bool), `mouse` (bool),
//! `sidebar_width` (columns), `metadata` (bool). Missing file = defaults;
//! malformed file = warning + defaults (never a crash). Unknown keys are
//! warning + defaults (never a crash). Unknown keys are ignored but
//! reported (never silently). The `FROND_CONFIG` env var overrides the path
//! (tests and smokes only — they must never write to the real `~/.config`).
//!
//! Precedence with the CLI: `--vault`/`--theme`/`--watch`/`--mouse` flags
//! override the config values (see `main.rs`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::viewer::FileStamp;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub theme: Option<String>,
    pub vault: Option<PathBuf>,
    pub watch: Option<bool>,
    /// Mouse capture on/off (off = native terminal text selection).
    pub mouse: Option<bool>,
    /// Sidebar width override in terminal columns (clamped 24–120).
    pub sidebar_width: Option<u16>,
    /// Whether the note's frontmatter metadata block renders at the top.
    pub metadata: Option<bool>,
    /// Startup theme group for `Shift+T`: "all", "light" or "dark".
    pub theme_rotation: Option<String>,
    /// Blank columns on each side of the note text (0–`MAX_MARGIN`).
    pub margin: Option<u16>,
    /// Status-bar icon set: "nerd" or "unicode".
    pub icons: Option<String>,
}

/// Glyph set for the status bar's note icons. `nerd` (the default) needs
/// a Nerd Font in the terminal; `unicode` uses only standard glyphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSet {
    #[default]
    Nerd,
    Unicode,
}

impl IconSet {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nerd" => Some(Self::Nerd),
            "unicode" => Some(Self::Unicode),
            _ => None,
        }
    }

    /// Icon beside a `.md` note's name (Nerd: markdown, U+E73E).
    pub fn note(self) -> &'static str {
        match self {
            Self::Nerd => "\u{e73e}",
            Self::Unicode => "¶",
        }
    }

    /// Icon beside a `.base` table's name (Nerd: material table, U+F04EB;
    /// unicode: ▦).
    pub fn base(self) -> &'static str {
        match self {
            Self::Nerd => "\u{f04eb}",
            Self::Unicode => "▦",
        }
    }
}

/// Widest note margin a config may ask for, per side, in columns.
pub const MAX_MARGIN: u16 = 10;
/// Note margin when the config does not set one, per side, in columns.
pub const DEFAULT_MARGIN: u16 = 2;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigRaw {
    theme: Option<String>,
    vault: Option<PathBuf>,
    watch: Option<bool>,
    mouse: Option<bool>,
    sidebar_width: Option<u16>,
    metadata: Option<bool>,
    theme_rotation: Option<String>,
    margin: Option<u16>,
    icons: Option<String>,
    /// Keys outside our vocabulary — captured so they can be reported.
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// The config file path: `FROND_CONFIG` when set, else `~/.config/frond/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("FROND_CONFIG") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("frond")
                .join("config.toml")
        })
}

/// `frond --config-show`: a fully commented default config file, printed
/// to stdout so it can be redirected to `~/.config/frond/config.toml`.
/// Every key is commented out — uncommenting a line activates it; missing
/// keys fall back to the defaults noted in the comments. The drift test
/// below pins this text to the loader's real key vocabulary.
pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# frond configuration — ~/.config/frond/config.toml
#
# Every key below is commented out: uncomment a line to activate it.
# Missing keys fall back to the defaults noted in the comments. CLI flags
# (--vault, --theme, --watch, --mouse) override this file. The file is
# re-read live for the `theme` key only.

# Theme by name: 14 built-ins (catppuccin-latte, catppuccin-frappe,
# catppuccin-macchiato, catppuccin-mocha, dracula, dracula-alucard,
# gruvbox, nord, rose-pine, rose-pine-dawn, rose-pine-moon,
# tokyonight-day, tokyonight-night, tokyonight-storm) or custom theme
# files in ~/.config/frond/themes/ (each must set appearance = "light"
# or "dark"; see docs/COLORS.md). Default: ocean.
# theme = "catppuccin-macchiato"

# Vault directory the sidebar indexes. Default: the current directory.
# vault = "~/notes"

# Re-render live on vault changes (watcher). Default: true.
# watch = true

# Sidebar width in terminal columns, clamped 24-120. Default: 30% of the
# terminal width, clamped 24-60.
# sidebar_width = 40

# Capture the mouse for in-app clicks and wheel scrolling. Off = the
# terminal keeps the mouse (native text selection/copy); `m` toggles at
# runtime. Default: true.
# mouse = true

# Render the note's frontmatter as a metadata block at the top (`f`
# toggles at runtime; tags/status in the status bar always show).
# Default: true.
# metadata = true

# Which themes `Shift+T` cycles through at startup: "all", "light" or
# "dark". Ctrl+T changes it for the session only. Default: "all".
# theme_rotation = "all"

# Blank columns on each side of the note text, 0-10. Default: 2.
# margin = 2

# Status-bar icons beside the note name: "nerd" needs a Nerd Font in the
# terminal; "unicode" uses only standard glyphs. Default: "nerd".
# icons = "nerd"
"#;

/// Loads the config from `path`. Returns the config plus warnings:
/// a missing file is silently default (that is the normal first-run
/// state), a malformed file or unknown keys produce warnings but never
/// an error.
pub fn load(path: &Path) -> (Config, Vec<String>) {
    let (config, warnings, _) = load_status(path);
    (config, warnings)
}

/// `load` plus a parse-success flag: `malformed` is true only when the
/// file existed but could not be parsed (a valid parse with an absent
/// theme key is NOT malformed — hot-reload must treat the two
/// differently: malformed keeps the current theme, a valid theme-less
/// config reverts to the default).
fn load_status(path: &Path) -> (Config, Vec<String>, bool) {
    let Ok(text) = fs::read_to_string(path) else {
        // Missing (or unreadable) file: defaults, no warning.
        return (Config::default(), Vec::new(), false);
    };
    match toml::from_str::<ConfigRaw>(&text) {
        Ok(raw) => {
            let mut warnings = Vec::new();
            let unknown: Vec<String> = raw.unknown.keys().cloned().collect();
            if !unknown.is_empty() {
                warnings.push(format!(
                    "config: unknown keys ignored: {}",
                    unknown.join(", ")
                ));
            }
            (
                Config {
                    theme: raw.theme,
                    vault: raw.vault,
                    watch: raw.watch,
                    mouse: raw.mouse,
                    sidebar_width: raw.sidebar_width,
                    metadata: raw.metadata,
                    theme_rotation: raw.theme_rotation,
                    margin: raw.margin,
                    icons: raw.icons,
                },
                warnings,
                false,
            )
        }
        Err(err) => (
            Config::default(),
            vec![format!(
                "could not parse config \"{}\": {err}",
                path.display()
            )],
            true,
        ),
    }
}

impl Config {
    /// True unless the config (or a flag) explicitly disabled watch mode.
    pub fn watch_default(&self) -> bool {
        self.watch.unwrap_or(true)
    }
}

/// Hot-reload state for the config file: its mtime stamp plus the theme
/// key we last saw. Polled on the app's 1 s watcher tick.
pub struct ConfigWatch {
    pub path: PathBuf,
    stamp: Option<FileStamp>,
    /// The raw `theme` key of the last successfully parsed config
    /// (`None` = absent/malformed — the effective name is then the default).
    theme_key: Option<String>,
}

/// Outcome of one config poll.
pub struct ConfigPoll {
    /// The file's stamp changed since the last poll.
    pub changed: bool,
    /// New theme name to live-apply (`None` = nothing to apply: unchanged
    /// key, or the file is malformed — keep the current theme).
    pub theme: Option<String>,
    /// Warnings to surface (malformed file, unknown keys).
    pub warnings: Vec<String>,
    /// The raw `theme` key as of this poll (`None` when malformed or
    /// absent). Lets the caller track a config that names a theme the
    /// app cannot resolve *yet* (a custom file that may appear later).
    pub theme_named: Option<String>,
}

impl ConfigWatch {
    pub fn new(path: PathBuf) -> Self {
        let (config, _, _) = load_status(&path);
        Self {
            theme_key: config.theme,
            stamp: FileStamp::of(&path).ok(),
            path,
        }
    }

    /// Polls the config file's stamp; on a change re-reads it. Only the
    /// `theme` key is live-applied — the `vault` path applies on the next
    /// launch (rebooting the index/watcher mid-session is not safe) and
    /// `watch` toggling is left to the `w` key. A **malformed** re-read
    /// keeps the current theme and only warns (the key is not updated, so
    /// fixing the file re-triggers an apply); a **valid** re-read applies
    /// the resolution chain's result — an absent `theme` key means the
    /// default theme, even when unknown keys are still present.
    pub fn poll(&mut self) -> ConfigPoll {
        let stamp = FileStamp::of(&self.path).ok();
        if stamp == self.stamp {
            return ConfigPoll {
                changed: false,
                theme: None,
                warnings: Vec::new(),
                theme_named: None,
            };
        }
        self.stamp = stamp;
        let (config, warnings, malformed) = load_status(&self.path);
        if malformed {
            // Malformed file: keep the current theme, just warn.
            return ConfigPoll {
                changed: true,
                theme: None,
                warnings,
                theme_named: None,
            };
        }
        if config.theme == self.theme_key {
            // Theme key unchanged (e.g. only vault/watch edited): nothing
            // to live-apply.
            return ConfigPoll {
                changed: true,
                theme: None,
                warnings,
                theme_named: self.theme_key.clone(),
            };
        }
        let theme = Some(
            config
                .theme
                .clone()
                .unwrap_or_else(|| crate::theme::DEFAULT_THEME.to_string()),
        );
        self.theme_key = config.theme.clone();
        // Unknown-key warnings surface on the apply path too (the caller
        // shows them unless the theme apply had its own message) — the
        // unchanged-theme-key path and this path stay symmetric.
        ConfigPoll {
            changed: true,
            theme,
            warnings,
            theme_named: config.theme,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn load_at(dir: &TempDir, name: &str) -> (Config, Vec<String>) {
        load(&dir.path().join(name))
    }

    #[test]
    fn missing_file_is_silently_default() {
        let dir = TempDir::new("cfg-missing");
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty(), "a missing file is the normal state");
        assert!(config.watch_default(), "watch defaults to on");
    }

    #[test]
    fn values_are_parsed() {
        let dir = TempDir::new("cfg-values");
        std::fs::write(
            dir.path().join("config.toml"),
            "theme = \"dracula\"\nvault = \"/tmp/vault\"\nwatch = false\nmouse = false\n",
        )
        .unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(config.theme.as_deref(), Some("dracula"));
        assert_eq!(config.vault.as_deref(), Some(Path::new("/tmp/vault")));
        assert_eq!(config.watch, Some(false));
        assert!(!config.watch_default());
        assert_eq!(config.mouse, Some(false), "mouse key parses");
        assert_eq!(config.sidebar_width, None, "absent key stays default");
        assert!(warnings.is_empty());
    }

    #[test]
    fn sidebar_width_is_parsed_and_typed() {
        let dir = TempDir::new("cfg-sbw");
        std::fs::write(dir.path().join("config.toml"), "sidebar_width = 42\n").unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(config.sidebar_width, Some(42));
        assert!(warnings.is_empty());
        // Non-integer values are malformed (warning + defaults).
        std::fs::write(dir.path().join("config.toml"), "sidebar_width = \"wide\"\n").unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(config, Config::default());
        assert!(!warnings.is_empty());
    }

    /// Drift test for `--config-show`: with every `# key = value` line
    /// uncommented, the template must parse cleanly (no unknown keys, no
    /// malformed-file warning) and set every documented key.
    #[test]
    fn icons_parse_nerd_and_unicode_and_default_to_nerd() {
        assert_eq!(IconSet::parse("Unicode"), Some(IconSet::Unicode));
        assert_eq!(IconSet::parse(" nerd "), Some(IconSet::Nerd));
        assert_eq!(IconSet::parse("emoji"), None);
        assert_eq!(IconSet::default(), IconSet::Nerd);
        assert_eq!(IconSet::Unicode.note(), "¶");
        assert_eq!(IconSet::Nerd.note(), "\u{e73e}");
        assert_eq!(IconSet::Nerd.base(), "\u{f04eb}");
        assert_eq!(IconSet::Unicode.base(), "▦");
    }

    #[test]
    fn theme_rotation_and_margin_load_from_config() {
        let dir = TempDir::new("config-rotation");
        std::fs::write(
            dir.path().join("config.toml"),
            "theme_rotation = \"dark\"\nmargin = 4\n",
        )
        .unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert!(warnings.is_empty());
        assert_eq!(config.theme_rotation.as_deref(), Some("dark"));
        assert_eq!(config.margin, Some(4));
    }

    #[test]
    fn config_show_template_matches_the_real_vocabulary() {
        let active: String = DEFAULT_CONFIG_TEMPLATE
            .lines()
            .map(|line| {
                match line
                    .strip_prefix("# ")
                    .and_then(|rest| rest.split_once('='))
                {
                    Some((key, _))
                        if !key.trim().is_empty()
                            && key
                                .trim()
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
                    {
                        line.replacen("# ", "", 1)
                    }
                    _ => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let dir = TempDir::new("cfg-show");
        std::fs::write(dir.path().join("config.toml"), &active).unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert!(
            warnings.is_empty(),
            "template must use only known keys: {warnings:?}"
        );
        assert_eq!(config.theme.as_deref(), Some("catppuccin-macchiato"));
        assert_eq!(config.vault.as_deref(), Some(Path::new("~/notes")));
        assert_eq!(config.watch, Some(true));
        assert_eq!(config.sidebar_width, Some(40));
        assert_eq!(config.mouse, Some(true));
        assert_eq!(config.metadata, Some(true));
    }

    #[test]
    fn malformed_file_warns_and_defaults() {
        let dir = TempDir::new("cfg-malformed");
        std::fs::write(dir.path().join("config.toml"), "!!! not toml !!!\n").unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(
            config,
            Config::default(),
            "malformed: defaults, never a crash"
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("could not parse config"),
            "{}",
            warnings[0]
        );

        // Wrong value types are malformed too.
        std::fs::write(dir.path().join("config.toml"), "watch = \"yes\"\n").unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(config, Config::default());
        assert!(!warnings.is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored_but_warned() {
        let dir = TempDir::new("cfg-unknown");
        std::fs::write(
            dir.path().join("config.toml"),
            "theme = \"ocean\"\nfutures_scale = 0.5\n",
        )
        .unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert_eq!(
            config.theme.as_deref(),
            Some("ocean"),
            "known keys still load"
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("unknown keys ignored: futures_scale"),
            "{}",
            warnings[0]
        );
    }

    #[test]
    fn path_override_points_somewhere_else() {
        // `FROND_CONFIG` redirects the lookup (smoke/tests only); the real
        // `~/.config` is never touched by tests.
        let dir = TempDir::new("cfg-override");
        std::fs::write(dir.path().join("mine.toml"), "theme = \"nord\"\n").unwrap();
        // SAFETY of the env dance: single-threaded unit test process.
        std::env::set_var("FROND_CONFIG", dir.path().join("mine.toml"));
        let path = config_path();
        std::env::remove_var("FROND_CONFIG");
        assert_eq!(path, Some(dir.path().join("mine.toml")));
        let (config, _) = load(&path.unwrap());
        assert_eq!(config.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn valid_themeless_config_with_unknown_keys_applies_the_default() {
        // A valid parse whose `theme` key was removed must live-apply the
        // default — unlike a malformed file, which keeps the current theme.
        let dir = TempDir::new("cfg-themeless");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = \"dracula\"\n").unwrap();
        let mut watch = ConfigWatch::new(path.clone());
        assert_eq!(
            watch.poll().theme,
            None,
            "initial state: the file is unchanged, nothing to apply"
        );

        // Rewrite: theme key removed, unknown key still present — a valid
        // parse, not a malformed one (different length → different stamp).
        std::fs::write(&path, "futures_scale = 0.5\n").unwrap();
        let poll = watch.poll();
        assert_eq!(
            poll.theme.as_deref(),
            Some(crate::theme::DEFAULT_THEME),
            "valid theme-less config applies the resolution chain's default"
        );
        assert!(
            !poll.warnings.is_empty(),
            "unknown keys surface on the apply path too"
        );

        // The next poll is quiet (the key is now tracked as absent).
        assert_eq!(watch.poll().theme, None);
    }
}
