//! `~/.config/frond/config.toml`: persistent settings read at startup.
//!
//! Keys: `theme` (string), `vault` (path or `[vaults]` name), `vaults`
//! (table of `name = "path"`: the vault picker's entries), `watch` (bool),
//! `mouse` (bool),
//! `sidebar_width` (columns), `metadata` (bool), `theme_rotation`
//! ("all"/"light"/"dark"), `margin` (columns), `margin_y` (rows), `icons`
//! ("nerd"/"unicode"), `line_numbers` (bool). Missing file = defaults;
//! malformed file = warning + defaults (never a crash). Unknown keys are
//! ignored but reported (never silently). The `FROND_CONFIG` env var overrides the path
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
    /// Named vaults (`[vaults]`), in name order. Paths as written: `~` is
    /// expanded where they are resolved (`main.rs`).
    pub vaults: BTreeMap<String, PathBuf>,
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
    /// Blank rows above and below the note text (0–`MAX_MARGIN_Y`).
    pub margin_y: Option<u16>,
    /// Status-bar icon set: "nerd" or "unicode".
    pub icons: Option<String>,
    /// Whether the note pane shows a line-number gutter.
    pub line_numbers: Option<bool>,
}

/// Glyph set for the status bar's note, table and clock icons. `nerd` (the
/// default) needs a Nerd Font in the terminal; `unicode` uses only standard
/// glyphs.
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

    /// Icon before the last render time (Nerd: clock, U+F017; unicode: ◷).
    pub fn clock(self) -> &'static str {
        match self {
            Self::Nerd => "\u{f017}",
            Self::Unicode => "\u{25f7}",
        }
    }
}

/// Widest note margin a config may ask for, per side, in columns.
pub const MAX_MARGIN: u16 = 10;
/// Note margin when the config does not set one, per side, in columns.
pub const DEFAULT_MARGIN: u16 = 2;
/// Tallest vertical note margin a config may ask for, per side, in rows.
pub const MAX_MARGIN_Y: u16 = 5;
/// Vertical note margin when the config does not set one, per side, in rows.
pub const DEFAULT_MARGIN_Y: u16 = 1;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigRaw {
    theme: Option<String>,
    vault: Option<PathBuf>,
    vaults: BTreeMap<String, PathBuf>,
    watch: Option<bool>,
    mouse: Option<bool>,
    sidebar_width: Option<u16>,
    metadata: Option<bool>,
    theme_rotation: Option<String>,
    margin: Option<u16>,
    margin_y: Option<u16>,
    icons: Option<String>,
    line_numbers: Option<bool>,
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

# Vault directory the sidebar indexes: a path, or a name from [vaults].
# Default: the current directory.
# vault = "~/notes"

# Re-render live on vault changes (watcher). Default: true.
# watch = true

# Sidebar width in terminal columns, clamped 24-120. Default: 30% of the
# terminal width, clamped 24-60.
# sidebar_width = 40

# Capture the mouse for in-app clicks and wheel scrolling. Off = the
# terminal keeps the mouse (native text selection/copy); `M` toggles at
# runtime. Default: true.
# mouse = true

# Render the note's frontmatter as a metadata block at the top (`m`
# toggles at runtime; tags/status in the status bar always show).
# Default: true.
# metadata = true

# Which themes `Shift+T` cycles through at startup: "all", "light" or
# "dark". Ctrl+T changes it for the session only. Default: "all".
# theme_rotation = "all"

# Blank columns on each side of the note text, 0-10. Default: 2.
# margin = 2

# Blank rows above and below the note text, 0-5. Default: 1.
# margin_y = 1

# Status-bar icons beside the note name: "nerd" needs a Nerd Font in the
# terminal; "unicode" uses only standard glyphs. Default: "nerd".
# icons = "nerd"

# Show a line-number gutter left of the note text: each rendered line's
# source line number (a wrapped continuation shows an empty cell). The text
# is narrower by the gutter's width. Default: false.
# line_numbers = false

# Named vaults. With two or more and neither --vault nor a file argument,
# frond opens on a vault picker (the `vault` key only preselects); with
# exactly one and no `vault` key, frond opens it. --vault takes a name too.
# A table: keep it last in the file (TOML puts the keys after it inside it).
# [vaults]
# notes = "~/notes"
# work = "~/work/notes"
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
                    vaults: raw.vaults,
                    watch: raw.watch,
                    mouse: raw.mouse,
                    sidebar_width: raw.sidebar_width,
                    metadata: raw.metadata,
                    theme_rotation: raw.theme_rotation,
                    margin: raw.margin,
                    margin_y: raw.margin_y,
                    icons: raw.icons,
                    line_numbers: raw.line_numbers,
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
    /// `theme` key is live-applied — `vault` and `vaults` apply on the next
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
        assert_eq!(IconSet::Nerd.clock(), "\u{f017}");
        assert_eq!(IconSet::Unicode.clock(), "\u{25f7}");
    }

    #[test]
    fn theme_rotation_and_margin_load_from_config() {
        let dir = TempDir::new("config-rotation");
        std::fs::write(
            dir.path().join("config.toml"),
            "theme_rotation = \"dark\"\nmargin = 4\nmargin_y = 3\n",
        )
        .unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert!(warnings.is_empty());
        assert_eq!(config.theme_rotation.as_deref(), Some("dark"));
        assert_eq!(config.margin, Some(4));
        assert_eq!(config.margin_y, Some(3));
    }

    /// The template's `# key = value` (and `# [table]`) lines with the
    /// leading `# ` removed; the value lines the drift test below feeds
    /// back to the loader. Other lines stay empty.
    fn template_active_lines() -> Vec<String> {
        DEFAULT_CONFIG_TEMPLATE
            .lines()
            .map(|line| {
                if let Some(table) = line.strip_prefix("# [") {
                    if table.ends_with(']') {
                        return line.replacen("# ", "", 1);
                    }
                }
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
            .collect()
    }

    /// The keys the `--config-show` template documents. The drift test
    /// pins the template to the loader; the module-header guard takes its
    /// expected list from here, so a new key cannot pass the guard with a
    /// stale header.
    /// A `[table]` header counts as its key; the entries inside it are
    /// the table's values, not keys of their own.
    fn template_keys() -> Vec<String> {
        let mut keys = Vec::new();
        let mut in_table = false;
        for line in template_active_lines() {
            if let Some(table) = line.strip_prefix('[') {
                keys.push(table.trim_end_matches(']').to_string());
                in_table = true;
            } else if let Some((key, _)) = line.split_once('=') {
                if !in_table {
                    keys.push(key.trim().to_string());
                }
            }
        }
        keys
    }

    #[test]
    fn config_show_template_matches_the_real_vocabulary() {
        let active = template_active_lines().join("\n");
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
        assert_eq!(config.margin_y, Some(1));
        assert_eq!(config.line_numbers, Some(false));
        assert_eq!(
            config.vaults.get("work").map(PathBuf::as_path),
            Some(Path::new("~/work/notes"))
        );
    }

    #[test]
    fn vaults_table_parses_in_name_order() {
        let dir = TempDir::new("cfg-vaults");
        std::fs::write(
            dir.path().join("config.toml"),
            "vault = \"work\"\n[vaults]\nwork = \"~/w\"\nhome = \"/srv/home\"\n",
        )
        .unwrap();
        let (config, warnings) = load_at(&dir, "config.toml");
        assert!(warnings.is_empty(), "vaults is a known key: {warnings:?}");
        assert_eq!(config.vault.as_deref(), Some(Path::new("work")));
        let entries: Vec<(&str, &Path)> = config
            .vaults
            .iter()
            .map(|(name, path)| (name.as_str(), path.as_path()))
            .collect();
        assert_eq!(
            entries,
            vec![("home", Path::new("/srv/home")), ("work", Path::new("~/w"))],
            "name order; `~` kept as written (expanded at resolution)"
        );
    }

    #[test]
    fn malformed_vaults_value_is_a_malformed_file() {
        let dir = TempDir::new("cfg-vaults-bad");
        for text in ["vaults = \"~/notes\"\n", "[vaults]\nwork = 3\n"] {
            std::fs::write(dir.path().join("config.toml"), text).unwrap();
            let (config, warnings) = load_at(&dir, "config.toml");
            assert_eq!(config, Config::default(), "{text}");
            assert!(warnings[0].contains("could not parse config"), "{text}");
        }
    }

    #[test]
    fn line_numbers_parses_and_defaults_off() {
        let dir = TempDir::new("cfg-line-numbers");
        let (config, warnings) = load_at(&dir, "config.toml");
        assert!(warnings.is_empty());
        assert_eq!(
            config.line_numbers, None,
            "an absent key leaves the default off to the app"
        );
        std::fs::write(dir.path().join("config.toml"), "line_numbers = true\n").unwrap();
        let (config, _) = load_at(&dir, "config.toml");
        assert_eq!(config.line_numbers, Some(true));
    }

    #[test]
    fn module_header_names_every_loader_key() {
        // The module header is the first thing a future key change reads;
        // pin it to the loader's vocabulary so it cannot fall behind. The
        // expected list comes from the `--config-show` template (already
        // pinned to the loader above), not a hand-kept copy.
        let source = include_str!("config.rs");
        let header: String = source
            .lines()
            .take_while(|line| line.starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n");
        let keys = template_keys();
        assert!(!keys.is_empty(), "the template documents no keys");
        for key in keys {
            assert!(
                header.contains(&format!("`{key}`")),
                "the module header omits the `{key}` key"
            );
        }
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
        let mut env = crate::testutil::EnvGuard::lock();
        env.set("FROND_CONFIG", dir.path().join("mine.toml"));
        let path = config_path();
        env.remove("FROND_CONFIG");
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
