//! Theme support: app-level themes = engine `MarkdownTheme` (note content)
//! + `UiTheme` (chrome: sidebar, status bar, popups).
//!
//! Three sources, in resolution order:
//! - **Paired presets** — the 4 engine presets (arctic/forest/ocean/
//!   solarized) get an app-level `UiTheme` hand-mapped from leaf's preset
//!   `UiTheme` VALUES (`toc_bg` → `sidebar_bg`, `status_bg` → `status_bg`;
//!   leaf keys that still have no counterpart here — the rest of the TOC
//!   panel, brand/percent segments — are unused; our own key vocabulary
//!   below).
//! - **Built-in themes** — 14 themes ported from the leaf theme files in
//!   the user's dotfiles into an in-binary table
//!   ([`crate::builtin_themes`]): same file shape (`base` + `syntax` +
//!   `[ui]` + `[markdown]`), with the `[ui]` keys translated to this
//!   app's vocabulary at port time.
//! - **Custom theme files** — TOML files in the frond format (`base`
//!   (inherit a preset) + `syntax` (syntect theme name) + `[ui]` +
//!   `[markdown]` sections). `[markdown]`/`base`/`syntax` load as-is
//!   (field names identical to the engine's `MarkdownTheme`); `[ui]` uses
//!   OUR OWN key vocabulary, matching what this app actually styles
//!   (`sidebar_*`, `status_*`, `popup_*`). Leaf's `[ui]` key names are
//!   NOT accepted — an frond file is app vocabulary only; unknown
//!   `[ui]`/`[diff]` (and `[markdown]`) keys are IGNORED but reported as
//!   warnings — never silently. There is no diff view, so `[diff]` keys
//!   are all ignored (and warned).
//!
//! Theme directory: `~/.config/frond/themes/` (the leaf compat directory is
//! no longer read). `FROND_THEMES` overrides it with a single directory
//! (tests and smokes only — never writes there).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use engine::{MarkdownTheme, MarkdownThemePreset};
use ratatui::style::Color;
use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;

/// Chrome palette: every color the app shell styles, with app-owned key
/// names (custom theme files use these names under `[ui]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiTheme {
    pub sidebar_border_focused: Color,
    pub sidebar_border_unfocused: Color,
    pub selection_bg: Color,
    /// Sidebar (tree/tags) and backlinks-panel background.
    pub sidebar_bg: Color,
    pub changed_marker_fg: Color,
    pub dimmed_fg: Color,
    pub content_bg: Color,
    /// Status bar background.
    pub status_bg: Color,
    pub status_watch_on_fg: Color,
    pub status_watch_off_fg: Color,
    pub status_success_fg: Color,
    pub status_dirty_fg: Color,
    pub status_ahead_fg: Color,
    pub status_behind_fg: Color,
    pub status_error_fg: Color,
    pub popup_border_fg: Color,
    pub popup_dimmed_fg: Color,
    pub popup_match_fg: Color,
    pub popup_match_bg: Color,
}

/// A fully resolved theme: markdown palette + chrome palette (+ the
/// syntect theme name a custom file asked for, if any).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    /// Declared background family; `Shift+T` / `Ctrl+T` filter on it.
    pub appearance: Appearance,
    pub markdown: MarkdownTheme,
    pub ui: UiTheme,
    pub syntax: Option<String>,
}

/// Whether a theme is light or dark. Every theme declares one: presets
/// and built-ins in code, custom theme files via `appearance = "light"`
/// or `"dark"` (required — see `docs/COLORS.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Which appearances `Shift+T` cycles through (`Ctrl+T` changes it for the
/// session; `theme_rotation` in config.toml sets the startup value).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeRotation {
    #[default]
    All,
    Light,
    Dark,
}

impl ThemeRotation {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// The next group in `Ctrl+T` order: all → light → dark → all.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::All,
        }
    }

    pub fn allows(self, appearance: Appearance) -> bool {
        matches!(
            (self, appearance),
            (Self::All, _) | (Self::Light, Appearance::Light) | (Self::Dark, Appearance::Dark)
        )
    }
}

// --- color parsing (ported from leaf resolution.rs) -----------------------

/// Parses a theme color value: `#rgb`/`#rrggbb` hex, `rgb(r, g, b)`, or a
/// named ratatui color (leaf parity).
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(color) = parse_hex_color(value) {
        return Some(color);
    }
    if let Some(color) = parse_rgb_color(value) {
        return Some(color);
    }
    use std::str::FromStr;
    Color::from_str(value).ok()
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    match hex.len() {
        3 if hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            let mut expanded = String::with_capacity(6);
            for ch in hex.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            parse_hex_color(&expanded)
        }
        6 if hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn parse_rgb_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let inner = value.strip_prefix("rgb(")?.strip_suffix(')')?;
    let mut components = inner.split(',').map(str::trim);
    let r = components.next()?.parse::<u8>().ok()?;
    let g = components.next()?.parse::<u8>().ok()?;
    let b = components.next()?.parse::<u8>().ok()?;
    if components.next().is_some() {
        return None;
    }
    Some(Color::Rgb(r, g, b))
}

/// Newtype so `Option<ThemeColor>` fields deserialize through
/// [`parse_color`] (ported from leaf's `serde.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColor(pub Color);

impl<'de> Deserialize<'de> for ThemeColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = ThemeColor;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a color name, hex color string, rgb(...) string, or [r, g, b]")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                parse_color(value)
                    .map(ThemeColor)
                    .ok_or_else(|| E::custom(format!("invalid theme color: {value}")))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let read = |seq: &mut A| -> Result<u8, A::Error> {
                    let v = seq
                        .next_element::<u16>()?
                        .ok_or_else(|| A::Error::custom("missing theme color component"))?;
                    u8::try_from(v)
                        .map_err(|_| A::Error::custom("theme color component out of range"))
                };
                let r = read(&mut seq)?;
                let g = read(&mut seq)?;
                let b = read(&mut seq)?;
                if seq.next_element::<u16>()?.is_some() {
                    return Err(A::Error::custom(
                        "theme RGB array must have exactly 3 values",
                    ));
                }
                Ok(ThemeColor(Color::Rgb(r, g, b)))
            }
        }

        deserializer.deserialize_any(ColorVisitor)
    }
}

// --- TOML overrides (ported from leaf's serde.rs, with unknown-key
// --- capture so nothing is ignored silently) ------------------------------

macro_rules! theme_overrides {
    ($name:ident for $target:ty { $($field:ident),+ $(,)? }) => {
        #[derive(Debug, Default, Deserialize)]
        #[serde(default)]
        struct $name {
            $(pub $field: Option<ThemeColor>,)+
            /// Keys that are not part of our vocabulary — captured (via
            /// `flatten`) so they can be reported, never silently dropped.
            #[serde(flatten)]
            pub unknown: BTreeMap<String, toml::Value>,
        }

        impl $name {
            fn apply_to(&self, theme: &mut $target) {
                $(
                    if let Some(color) = self.$field {
                        theme.$field = color.0;
                    }
                )+
            }
        }
    };
}

/// The captured unknown keys, sorted.
fn unknown_keys(map: &BTreeMap<String, toml::Value>) -> Vec<String> {
    map.keys().cloned().collect()
}

theme_overrides!(UiOverrides for UiTheme {
    sidebar_border_focused,
    sidebar_border_unfocused,
    selection_bg,
    sidebar_bg,
    changed_marker_fg,
    dimmed_fg,
    content_bg,
    status_bg,
    status_watch_on_fg,
    status_watch_off_fg,
    status_success_fg,
    status_dirty_fg,
    status_ahead_fg,
    status_behind_fg,
    status_error_fg,
    popup_border_fg,
    popup_dimmed_fg,
    popup_match_fg,
    popup_match_bg,
});

/// The single list of `MarkdownTheme` fields a theme can set. Invoked with
/// a callback macro that receives the comma-separated field names; the
/// config-file overrides below and the built-in themes' keys
/// (`builtin_themes::md`) are both generated from it. A field named here that
/// the engine struct lacks fails to compile; one the engine adds but this list
/// misses is caught by `markdown_field_list_covers_the_engine_struct`.
macro_rules! with_markdown_fields {
    ($callback:ident) => {
        $callback! {
            search_highlight_bg,
            search_match_bg,
            code_gutter,
            blockquote_marker,
            list_level_1,
            list_level_2,
            list_level_3,
            ordered_list,
            table_border,
            table_separator,
            table_header,
            table_cell,
            heading_1,
            heading_2,
            heading_3,
            heading_4,
            heading_other,
            heading_underline,
            code_frame,
            code_label,
            inline_code_fg,
            inline_code_bg,
            rule,
            link_icon,
            link_text,
            link_hover,
            blockquote_text,
            text,
            strong_text,
            latex_inline_fg,
            latex_inline_bg,
            latex_block_fg,
            mermaid_keyword,
            mermaid_arrow,
            mermaid_label,
            mermaid_block_fg,
            mark_fg,
            mark_bg,
            task_checked,
            task_unchecked,
            alert_note,
            alert_tip,
            alert_important,
            alert_warning,
            alert_caution,
            footnote_ref,
            footnote_text,
        }
    };
}
pub(crate) use with_markdown_fields;

macro_rules! markdown_overrides {
    ($($field:ident),+ $(,)?) => {
        theme_overrides!(MarkdownOverrides for MarkdownTheme { $($field),+ });
    };
}
with_markdown_fields!(markdown_overrides);

/// `[diff]` exists in leaf theme files but this app has no diff view:
/// every key is captured here only so it can be listed in the warning.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DiffOverrides {
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// Raw shape of an frond theme file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemeFileRaw {
    base: Option<String>,
    appearance: Option<String>,
    syntax: Option<String>,
    ui: UiOverrides,
    markdown: MarkdownOverrides,
    diff: DiffOverrides,
}

/// Builds a [`Theme`] from a parsed theme file, applying overrides over
/// the inherited base preset and collecting the unknown-key warnings.
fn build_theme(name: &str, raw: ThemeFileRaw) -> Result<(Theme, Vec<String>), String> {
    let base = match raw.base.as_deref() {
        Some(base) => parse_preset_name(base.trim())
            .ok_or_else(|| format!("unknown base theme \"{base}\" for custom theme \"{name}\""))?,
        None => MarkdownThemePreset::default(),
    };
    // Required: a custom theme must declare its background family so the
    // light/dark rotation can place it. Without it the file is skipped
    // with this message (never guessed from the colors).
    let appearance = raw
        .appearance
        .as_deref()
        .and_then(Appearance::parse)
        .ok_or_else(|| {
            format!("custom theme \"{name}\" must set appearance = \"light\" or \"dark\"")
        })?;

    let mut markdown = engine::markdown_theme_by_preset(base);
    raw.markdown.apply_to(&mut markdown);
    let mut ui = ui_by_preset(base);
    raw.ui.apply_to(&mut ui);

    let mut warnings = Vec::new();
    let unknown = unknown_keys(&raw.ui.unknown);
    if !unknown.is_empty() {
        warnings.push(format!(
            "theme \"{name}\": unknown [ui] keys ignored: {}",
            unknown.join(", ")
        ));
    }
    let unknown = unknown_keys(&raw.markdown.unknown);
    if !unknown.is_empty() {
        warnings.push(format!(
            "theme \"{name}\": unknown [markdown] keys ignored: {}",
            unknown.join(", ")
        ));
    }
    let unknown = unknown_keys(&raw.diff.unknown);
    if !unknown.is_empty() {
        warnings.push(format!(
            "theme \"{name}\": [diff] has no diff view here; keys ignored: {}",
            unknown.join(", ")
        ));
    }

    Ok((
        Theme {
            name: name.to_string(),
            appearance,
            markdown,
            ui,
            syntax: raw.syntax,
        },
        warnings,
    ))
}

/// Loads one theme TOML file (`name` = display/selection name, usually the
/// file stem). Returns the theme plus any unknown-key warnings.
pub fn load_theme_file(path: &Path, name: &str) -> Result<(Theme, Vec<String>), String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("cannot read theme file \"{}\": {err}", path.display()))?;
    let raw: ThemeFileRaw = toml::from_str(&content)
        .map_err(|err| format!("could not parse theme file \"{}\": {err}", path.display()))?;
    build_theme(name, raw)
}

// --- presets (paired: leaf markdown palette + hand-mapped leaf ui values) --

/// Chrome palette for a built-in preset, hand-mapped from leaf's preset
/// `UiTheme` VALUES (`/tmp/leaf/src/theme/presets.rs`): `toc_bg` →
/// `sidebar_bg`, `status_bg` → `status_bg`, `toc_accent` → the
/// focused/selection accent, `toc_border` → unfocused/popup borders,
/// `status_*` → the status-bar segments, `status_search_*` → the search
/// match highlight. Leaf keys with no counterpart here (the rest of the
/// TOC panel, brand/percent segments) are simply not mapped.
pub fn ui_by_preset(preset: MarkdownThemePreset) -> UiTheme {
    match preset {
        // BASE_LIGHT_UI (leaf presets.rs, values verbatim).
        MarkdownThemePreset::Arctic => UiTheme {
            sidebar_border_focused: Color::Rgb(76, 122, 168), // toc_accent
            sidebar_border_unfocused: Color::Rgb(170, 182, 194), // toc_border
            selection_bg: Color::Rgb(76, 122, 168),           // toc_accent
            sidebar_bg: Color::Rgb(232, 239, 245),            // toc_bg
            changed_marker_fg: Color::Rgb(180, 142, 28),      // status_warning_fg
            dimmed_fg: Color::Rgb(98, 116, 134),              // status_shortcut_fg
            content_bg: Color::Rgb(242, 247, 250),
            status_bg: Color::Rgb(224, 233, 240), // status_bg
            status_watch_on_fg: Color::Rgb(48, 140, 98), // status_watch_fg
            status_watch_off_fg: Color::Rgb(98, 116, 134), // status_shortcut_fg
            status_success_fg: Color::Rgb(48, 140, 98),
            status_dirty_fg: Color::Rgb(180, 142, 28), // status_warning_fg
            status_ahead_fg: Color::Rgb(48, 140, 98),  // status_success_fg
            status_behind_fg: Color::Rgb(188, 74, 74), // status_error_fg
            status_error_fg: Color::Rgb(188, 74, 74),
            popup_border_fg: Color::Rgb(170, 182, 194), // toc_border
            popup_dimmed_fg: Color::Rgb(98, 116, 134),  // status_shortcut_fg
            popup_match_fg: Color::Rgb(142, 114, 24),   // status_search_fg
            popup_match_bg: Color::Rgb(240, 234, 200),  // status_search_bg
        },
        // BASE_DARK_UI (leaf presets.rs, values verbatim) — also the
        // default (ocean).
        MarkdownThemePreset::OceanDark => UiTheme {
            sidebar_border_focused: Color::Rgb(123, 109, 255), // toc_accent
            sidebar_border_unfocused: Color::Rgb(52, 52, 58),  // toc_border
            selection_bg: Color::Rgb(123, 109, 255),           // toc_accent
            sidebar_bg: Color::Rgb(18, 18, 22),                // toc_bg
            changed_marker_fg: Color::Rgb(240, 200, 60),       // status_warning_fg
            dimmed_fg: Color::Rgb(58, 68, 98),                 // status_shortcut_fg
            content_bg: Color::Rgb(18, 20, 28),
            status_bg: Color::Rgb(18, 20, 32), // status_bg
            status_watch_on_fg: Color::Rgb(95, 200, 148), // status_watch_fg
            status_watch_off_fg: Color::Rgb(58, 68, 98), // status_shortcut_fg
            status_success_fg: Color::Rgb(120, 210, 170),
            status_dirty_fg: Color::Rgb(240, 200, 60), // status_warning_fg
            status_ahead_fg: Color::Rgb(120, 210, 170), // status_success_fg
            status_behind_fg: Color::Rgb(218, 95, 95), // status_error_fg
            status_error_fg: Color::Rgb(218, 95, 95),
            popup_border_fg: Color::Rgb(52, 52, 58), // toc_border
            popup_dimmed_fg: Color::Rgb(58, 68, 98), // status_shortcut_fg
            popup_match_fg: Color::Rgb(240, 210, 95), // status_search_fg
            popup_match_bg: Color::Rgb(36, 32, 16),  // status_search_bg
        },
        // FOREST_THEME.ui (leaf presets.rs, values verbatim).
        MarkdownThemePreset::Forest => UiTheme {
            sidebar_border_focused: Color::Rgb(127, 179, 255), // toc_accent
            sidebar_border_unfocused: Color::Rgb(50, 66, 54),  // toc_border
            selection_bg: Color::Rgb(127, 179, 255),           // toc_accent
            sidebar_bg: Color::Rgb(16, 22, 18),                // toc_bg
            changed_marker_fg: Color::Rgb(236, 214, 123),      // status_warning_fg
            dimmed_fg: Color::Rgb(82, 104, 92),                // status_shortcut_fg
            content_bg: Color::Rgb(19, 26, 22),
            status_bg: Color::Rgb(18, 27, 24), // status_bg
            status_watch_on_fg: Color::Rgb(132, 214, 154), // status_watch_fg
            status_watch_off_fg: Color::Rgb(82, 104, 92), // status_shortcut_fg
            status_success_fg: Color::Rgb(120, 214, 170),
            status_dirty_fg: Color::Rgb(236, 214, 123), // status_warning_fg
            status_ahead_fg: Color::Rgb(120, 214, 170), // status_success_fg
            status_behind_fg: Color::Rgb(224, 120, 120), // status_error_fg
            status_error_fg: Color::Rgb(224, 120, 120),
            popup_border_fg: Color::Rgb(50, 66, 54), // toc_border
            popup_dimmed_fg: Color::Rgb(82, 104, 92), // status_shortcut_fg
            popup_match_fg: Color::Rgb(236, 214, 123), // status_search_fg
            popup_match_bg: Color::Rgb(38, 34, 18),  // status_search_bg
        },
        // SOLARIZED_DARK_THEME.ui (leaf presets.rs, values verbatim).
        MarkdownThemePreset::SolarizedDark => UiTheme {
            sidebar_border_focused: Color::Rgb(38, 139, 210), // toc_accent
            sidebar_border_unfocused: Color::Rgb(88, 110, 117), // toc_border
            selection_bg: Color::Rgb(38, 139, 210),           // toc_accent
            sidebar_bg: Color::Rgb(7, 54, 66),                // toc_bg
            changed_marker_fg: Color::Rgb(181, 137, 0),       // status_warning_fg
            dimmed_fg: Color::Rgb(88, 110, 117),              // status_shortcut_fg
            content_bg: Color::Rgb(0, 43, 54),
            status_bg: Color::Rgb(0, 43, 54),            // status_bg
            status_watch_on_fg: Color::Rgb(133, 153, 0), // status_watch_fg
            status_watch_off_fg: Color::Rgb(88, 110, 117), // status_shortcut_fg
            status_success_fg: Color::Rgb(42, 161, 152),
            status_dirty_fg: Color::Rgb(181, 137, 0), // status_warning_fg
            status_ahead_fg: Color::Rgb(42, 161, 152), // status_success_fg
            status_behind_fg: Color::Rgb(220, 50, 47), // status_error_fg
            status_error_fg: Color::Rgb(220, 50, 47),
            popup_border_fg: Color::Rgb(88, 110, 117), // toc_border
            popup_dimmed_fg: Color::Rgb(88, 110, 117), // status_shortcut_fg
            popup_match_fg: Color::Rgb(181, 137, 0),   // status_search_fg
            popup_match_bg: Color::Rgb(32, 28, 0),     // status_search_bg
        },
    }
}

/// Each preset's syntect theme name (leaf parity, `presets.rs`).
fn preset_syntax(preset: MarkdownThemePreset) -> Option<&'static str> {
    match preset {
        MarkdownThemePreset::Arctic => Some("base16-ocean.light"),
        MarkdownThemePreset::Forest => Some("InspiredGitHub"),
        MarkdownThemePreset::OceanDark => Some("base16-ocean.dark"),
        MarkdownThemePreset::SolarizedDark => Some("Solarized (dark)"),
    }
}

/// Accepts leaf's preset names (parse_theme_preset parity).
pub fn parse_preset_name(name: &str) -> Option<MarkdownThemePreset> {
    match name {
        "arctic" => Some(MarkdownThemePreset::Arctic),
        "ocean" | "ocean-dark" | "dark" => Some(MarkdownThemePreset::OceanDark),
        "forest" => Some(MarkdownThemePreset::Forest),
        "solarized" | "solarized-dark" => Some(MarkdownThemePreset::SolarizedDark),
        _ => None,
    }
}

/// Canonical preset name (what the config/flag accepts back).
pub fn preset_name(preset: MarkdownThemePreset) -> &'static str {
    match preset {
        MarkdownThemePreset::Arctic => "arctic",
        MarkdownThemePreset::Forest => "forest",
        MarkdownThemePreset::OceanDark => "ocean",
        MarkdownThemePreset::SolarizedDark => "solarized",
    }
}

/// Background family of a preset: arctic is the light base, the other
/// three are dark (`ui_by_preset` pairs each with its base palette).
pub fn preset_appearance(preset: MarkdownThemePreset) -> Appearance {
    match preset {
        MarkdownThemePreset::Arctic => Appearance::Light,
        MarkdownThemePreset::Forest
        | MarkdownThemePreset::OceanDark
        | MarkdownThemePreset::SolarizedDark => Appearance::Dark,
    }
}

/// Preset theme with its paired chrome palette.
pub fn preset_theme(preset: MarkdownThemePreset) -> Theme {
    Theme {
        name: preset_name(preset).to_string(),
        appearance: preset_appearance(preset),
        markdown: engine::markdown_theme_by_preset(preset),
        ui: ui_by_preset(preset),
        syntax: preset_syntax(preset).map(str::to_string),
    }
}

pub const DEFAULT_THEME: &str = "ocean";

// --- built-in themes ------------------------------------------------------

/// The 14 built-in themes ported from the leaf theme files
/// ([`crate::builtin_themes`]), resolved by name.
fn builtin_theme(name: &str) -> Option<Theme> {
    let spec = crate::builtin_themes::BUILTIN_THEMES
        .iter()
        .find(|spec| spec.name == name)?;
    let mut markdown = engine::markdown_theme_by_preset(spec.base);
    for (key, color) in spec.md {
        key.apply(&mut markdown, *color);
    }
    Some(Theme {
        name: spec.name.to_string(),
        appearance: spec.appearance,
        markdown,
        ui: spec.ui,
        syntax: spec.syntax.map(str::to_string),
    })
}

// --- discovery ------------------------------------------------------------

/// The theme directory: `~/.config/frond/themes/` (custom frond-format theme
/// files only — there is no leaf compat directory anymore).
pub fn default_theme_dirs() -> Vec<PathBuf> {
    if let Ok(dir) = std::env::var("FROND_THEMES") {
        return vec![PathBuf::from(dir)];
    }
    let Some(home) = std::env::var("HOME").ok().filter(|h| !h.is_empty()) else {
        return Vec::new();
    };
    vec![PathBuf::from(home)
        .join(".config")
        .join("frond")
        .join("themes")]
}

/// Custom themes by name (file stem), discovered across `dirs` in order:
/// the first directory containing a given name wins. Only `*.toml` files;
/// unreadable/missing directories are skipped.
pub fn discover_custom_themes(dirs: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut found = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            })
            .collect();
        files.sort();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            found.entry(stem.to_string()).or_insert(path);
        }
    }
    found
}

// --- resolution -----------------------------------------------------------

/// A resolved theme plus its non-fatal warnings (unknown keys, syntax
/// fallback). Warnings are surfaced by the caller (startup → stderr,
/// runtime → status bar); they are never dropped silently.
pub struct ResolvedTheme {
    pub theme: Theme,
    pub warnings: Vec<String>,
}

/// Resolves a theme name: preset names (leaf aliases included) first,
/// then the built-in ported themes, then discovered custom themes.
/// A custom file whose name matches a preset/built-in is never loaded —
/// the reserved name wins and a warning says so (never silently).
/// Unknown names are an error — callers warn and fall back to the default.
pub fn resolve_theme(
    name: &str,
    custom: &BTreeMap<String, PathBuf>,
) -> Result<ResolvedTheme, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("theme name cannot be empty".to_string());
    }
    if let Some(preset) = parse_preset_name(name) {
        let warnings = if custom.contains_key(name) {
            vec![shadow_warning(name, "preset")]
        } else {
            Vec::new()
        };
        return Ok(ResolvedTheme {
            theme: preset_theme(preset),
            warnings,
        });
    }
    if let Some(theme) = builtin_theme(name) {
        let warnings = if custom.contains_key(name) {
            vec![shadow_warning(name, "built-in")]
        } else {
            Vec::new()
        };
        return Ok(ResolvedTheme { theme, warnings });
    }
    if let Some(path) = custom.get(name) {
        let (theme, warnings) = load_theme_file(path, name)?;
        return Ok(ResolvedTheme { theme, warnings });
    }
    Err(format!("unknown theme \"{name}\""))
}

/// A custom theme file whose name matches a preset or built-in theme is
/// never loaded: the reserved name wins resolution, and keeping the file
/// in the cycle would yield two indistinguishable same-named entries.
/// Callers surface the returned warning instead (never silently).
/// `Some(kind)` is the reserved-name kind for the message.
fn reserved_name_kind(name: &str) -> Option<&'static str> {
    if parse_preset_name(name).is_some() {
        Some("preset")
    } else if crate::builtin_themes::BUILTIN_THEMES
        .iter()
        .any(|spec| spec.name == name)
    {
        Some("built-in")
    } else {
        None
    }
}

fn shadow_warning(name: &str, kind: &str) -> String {
    format!("custom theme \"{name}\" is shadowed by the {kind} theme of the same name; not loaded")
}

// --- cycling --------------------------------------------------------------

/// One entry of the `Shift+T` cycle: a loaded theme plus its own
/// warnings (unknown keys etc.). Warnings are surfaced only when the
/// theme is actually applied — a theme merely present in the cycle never
/// spams the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeChoice {
    pub theme: Theme,
    pub warnings: Vec<String>,
}

/// The themes `Shift+T` cycles through: the four presets in leaf's
/// `THEME_PRESETS` order, then the 14 built-in themes alphabetically, then
/// discovered custom themes alphabetically.
/// All loaded eagerly (theme files are tiny); cycling is instant.
pub struct ThemeChoices {
    pub entries: Vec<ThemeChoice>,
    pub index: usize,
}

impl Default for ThemeChoices {
    fn default() -> Self {
        Self::presets_only()
    }
}

impl ThemeChoices {
    /// The four presets, positioned on the default (ocean).
    pub fn presets_only() -> Self {
        let entries: Vec<ThemeChoice> = [
            MarkdownThemePreset::Arctic,
            MarkdownThemePreset::Forest,
            MarkdownThemePreset::OceanDark,
            MarkdownThemePreset::SolarizedDark,
        ]
        .into_iter()
        .map(|preset| ThemeChoice {
            theme: preset_theme(preset),
            warnings: Vec::new(),
        })
        .collect();
        let index = entries
            .iter()
            .position(|choice| choice.theme.name == DEFAULT_THEME)
            .unwrap_or(0);
        Self { entries, index }
    }

    /// Presets + built-ins + discovered custom themes (loaded eagerly so
    /// cycling is instant). Per-file unknown-key warnings attach to their
    /// theme and surface only when it is applied; files that fail to parse
    /// are skipped and reported back to the caller (always surfaced — a
    /// file that cannot load at all would otherwise vanish silently from
    /// the cycle). The same applies to custom files whose name collides
    /// with a preset or built-in: skipped, with one load-error string per
    /// collision (a name that shadows a reserved theme must never produce
    /// a second same-named cycle entry).
    pub fn build(custom: &BTreeMap<String, PathBuf>) -> (Self, Vec<String>) {
        let mut choices = Self::presets_only();
        for spec in crate::builtin_themes::BUILTIN_THEMES {
            choices.entries.push(ThemeChoice {
                theme: builtin_theme(spec.name).expect("BUILTIN_THEMES entries always resolve"),
                warnings: Vec::new(),
            });
        }
        let mut load_errors = Vec::new();
        for (name, path) in custom {
            if let Some(kind) = reserved_name_kind(name) {
                load_errors.push(shadow_warning(name, kind));
                continue;
            }
            match load_theme_file(path, name) {
                Ok((theme, warnings)) => choices.entries.push(ThemeChoice { theme, warnings }),
                Err(err) => load_errors.push(err),
            }
        }
        (choices, load_errors)
    }

    pub fn current(&self) -> &ThemeChoice {
        // Invariant: the cycle always holds the presets, and `index` is
        // kept in range by `select` / `next_*`.
        debug_assert!(self.index < self.entries.len());
        &self.entries[self.index]
    }

    /// The first entry at or after `from` (wrapping) whose appearance the
    /// rotation allows; `None` when the rotation matches nothing.
    pub fn find_allowed(&self, from: usize, rotation: ThemeRotation) -> Option<usize> {
        let len = self.entries.len();
        (0..len)
            .map(|step| (from + step) % len)
            .find(|&i| rotation.allows(self.entries[i].theme.appearance))
    }

    /// Whether the next `Shift+T` step wraps past the end of the cycle
    /// (the caller re-scans the theme dirs before wrapping).
    pub fn next_wraps(&self, rotation: ThemeRotation) -> bool {
        let from = (self.index + 1) % self.entries.len();
        self.find_allowed(from, rotation)
            .is_none_or(|next| next <= self.index)
    }

    /// Advances to the next theme the rotation allows (wrapping) and
    /// returns it; stays put when nothing matches.
    pub fn cycle(&mut self, rotation: ThemeRotation) -> &ThemeChoice {
        let from = (self.index + 1) % self.entries.len();
        if let Some(next) = self.find_allowed(from, rotation) {
            self.index = next;
        }
        self.current()
    }

    /// How many entries the rotation allows (the status line reports it).
    pub fn count_in(&self, rotation: ThemeRotation) -> usize {
        self.entries
            .iter()
            .filter(|choice| rotation.allows(choice.theme.appearance))
            .count()
    }

    /// Positions the cycle on the named theme, if present.
    pub fn select(&mut self, name: &str) -> bool {
        match self
            .entries
            .iter()
            .position(|choice| choice.theme.name == name)
        {
            Some(index) => {
                self.index = index;
                true
            }
            None => false,
        }
    }
}

/// The effective theme name: `--theme` flag > `FROND_THEME` env >
/// config.toml `theme` key > default ("ocean").
pub fn effective_theme_name(flag: Option<&str>, env: Option<&str>, config: Option<&str>) -> String {
    flag.or(env)
        .or(config)
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_THEME.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast of the colours the link/checkbox/glyph states draw
    /// with, on every built-in theme. The floor is deliberately low: it
    /// catches a role that becomes invisible (a selection-coloured focus
    /// background measured ~1.1 against the link colour), not taste.
    #[test]
    fn link_state_colours_stay_legible_on_every_theme() {
        use ratatui::style::Color;
        fn lum(c: Color) -> f64 {
            let Color::Rgb(r, g, b) = c else {
                panic!("built-in themes are all Rgb, got {c:?}")
            };
            let f = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
        }
        fn contrast(a: Color, b: Color) -> f64 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        const FLOOR: f64 = 2.0;
        for spec in crate::builtin_themes::BUILTIN_THEMES {
            let t = resolve_theme(spec.name, &Default::default()).unwrap().theme;
            let ui = t.ui;
            let colors = crate::links::LinkColors::from_ui(ui);
            // Focus is inverse video of the span's own colour: as legible as
            // the link text is on the content background.
            let focused = colors.focus(ratatui::style::Style::new().fg(t.markdown.link_text));
            let (fg, bg) = (focused.fg.unwrap(), focused.bg.unwrap());
            assert!(
                contrast(fg, bg) >= FLOOR,
                "{}: focused link {:.2}",
                spec.name,
                contrast(fg, bg)
            );
            assert!(
                contrast(colors.ambiguous_fg, ui.content_bg) >= FLOOR,
                "{}: ambiguous link {:.2}",
                spec.name,
                contrast(colors.ambiguous_fg, ui.content_bg)
            );
            assert!(
                contrast(ui.status_success_fg, ui.sidebar_bg) >= FLOOR,
                "{}: success glyph",
                spec.name
            );
            assert!(
                contrast(ui.status_error_fg, ui.sidebar_bg) >= FLOOR,
                "{}: error glyph",
                spec.name
            );
        }
    }

    /// Every `MarkdownTheme` field must be in `with_markdown_fields!`: set
    /// each listed field to a unique sentinel colour and check that no
    /// other colour remains (a field the engine added but the list missed
    /// would keep its preset colour).
    #[test]
    fn markdown_field_list_covers_the_engine_struct() {
        use ratatui::style::Color;
        let mut theme = engine::markdown_theme_by_preset(engine::MarkdownThemePreset::OceanDark);
        let mut count = 0u8;
        macro_rules! set_all {
            ($($field:ident),+ $(,)?) => {
                $( count += 1; theme.$field = Color::Rgb(250, 251, count); )+
            };
        }
        with_markdown_fields!(set_all);
        let debug = format!("{theme:?}");
        let colours = debug.matches("Rgb(").count() + debug.matches("Indexed(").count();
        let sentinels = debug.matches("Rgb(250, 251, ").count();
        assert_eq!(
            sentinels, colours,
            "MarkdownTheme has fields missing from with_markdown_fields!: {debug}"
        );
        assert_eq!(sentinels, usize::from(count));
    }
    use crate::testutil::TempDir;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb(r, g, b)
    }

    #[test]
    fn parse_color_accepts_hex_rgb_and_named_forms() {
        assert_eq!(parse_color("#ff00ff"), Some(rgb(255, 0, 255)));
        assert_eq!(parse_color("ff00ff"), Some(rgb(255, 0, 255)));
        assert_eq!(parse_color("#f0f"), Some(rgb(255, 0, 255)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(rgb(1, 2, 3)));
        assert_eq!(parse_color(" rgb(4,5,6) "), Some(rgb(4, 5, 6)));
        // Named ratatui colors (leaf parity via Color::from_str).
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("light-blue"), Some(Color::LightBlue));
        // Invalid forms: None, never a panic.
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(1,2)"), None);
        assert_eq!(parse_color("rgb(1,2,3,4)"), None);
        assert_eq!(parse_color("not-a-color"), None);
    }

    #[test]
    fn theme_color_deserializes_from_string_and_array() {
        #[derive(Deserialize)]
        struct Wrap {
            #[allow(dead_code)]
            c: ThemeColor,
        }
        let from_str: Wrap = toml::from_str("c = \"#aabbcc\"").unwrap();
        assert_eq!(from_str.c.0, rgb(0xaa, 0xbb, 0xcc));
        let from_seq: Wrap = toml::from_str("c = [1, 2, 3]").unwrap();
        assert_eq!(from_seq.c.0, rgb(1, 2, 3));
        assert!(toml::from_str::<Wrap>("c = \"nope\"").is_err());
        assert!(toml::from_str::<Wrap>("c = [1, 2, 300]").is_err());
    }

    #[test]
    fn preset_themes_pair_leaf_markdown_and_mapped_ui_values() {
        for preset in [
            MarkdownThemePreset::Arctic,
            MarkdownThemePreset::Forest,
            MarkdownThemePreset::OceanDark,
            MarkdownThemePreset::SolarizedDark,
        ] {
            let theme = preset_theme(preset);
            assert_eq!(
                theme.markdown,
                engine::markdown_theme_by_preset(preset),
                "preset markdown palette is leaf's, verbatim"
            );
            assert_eq!(theme.ui, ui_by_preset(preset));
            assert!(theme.syntax.is_some(), "presets carry a syntect theme");
        }
        // Spot-check the hand-mapped values against leaf's presets.
        let ocean = preset_theme(MarkdownThemePreset::OceanDark);
        assert_eq!(ocean.ui.content_bg, rgb(18, 20, 28));
        assert_eq!(ocean.ui.sidebar_border_focused, rgb(123, 109, 255));
        assert_eq!(ocean.ui.sidebar_bg, rgb(18, 18, 22), "BASE_DARK_UI toc_bg");
        assert_eq!(
            ocean.ui.status_bg,
            rgb(18, 20, 32),
            "BASE_DARK_UI status_bg"
        );
        let arctic = preset_theme(MarkdownThemePreset::Arctic);
        assert_eq!(arctic.ui.content_bg, rgb(242, 247, 250));
        assert_eq!(
            arctic.ui.sidebar_bg,
            rgb(232, 239, 245),
            "BASE_LIGHT_UI toc_bg"
        );
        assert_eq!(
            arctic.ui.status_bg,
            rgb(224, 233, 240),
            "BASE_LIGHT_UI status_bg"
        );
    }

    #[test]
    fn every_theme_sets_both_new_backgrounds_to_real_values() {
        // `UiTheme` has no `Default`, so the compiler forces every literal
        // to name all fields — but a port could still leave a placeholder
        // (a named/reset color). Every preset and built-in must carry real
        // RGB values for `sidebar_bg` and `status_bg`.
        for preset in [
            MarkdownThemePreset::Arctic,
            MarkdownThemePreset::Forest,
            MarkdownThemePreset::OceanDark,
            MarkdownThemePreset::SolarizedDark,
        ] {
            let ui = ui_by_preset(preset);
            assert!(
                matches!(ui.sidebar_bg, Color::Rgb(..)),
                "{preset:?} sidebar_bg placeholder"
            );
            assert!(
                matches!(ui.status_bg, Color::Rgb(..)),
                "{preset:?} status_bg placeholder"
            );
        }
        for spec in crate::builtin_themes::BUILTIN_THEMES {
            assert!(
                matches!(spec.ui.sidebar_bg, Color::Rgb(..)),
                "{} sidebar_bg placeholder",
                spec.name
            );
            assert!(
                matches!(spec.ui.status_bg, Color::Rgb(..)),
                "{} status_bg placeholder",
                spec.name
            );
        }
    }

    #[test]
    fn custom_theme_inherits_base_and_applies_overrides() {
        let dir = TempDir::new("theme-inherit");
        // Based on ocean (heading_1 = Rgb(140,190,255)), overriding two
        // markdown keys and three ui keys; unknown keys warned.
        let file = dir.path().join("sample.toml");
        std::fs::write(
            &file,
            "base = \"ocean\"\nappearance = \"dark\"\nsyntax = \"base16-ocean.dark\"\n\n\
             [ui]\ncontent_bg = \"#010203\"\nsidebar_bg = \"#030405\"\n\
             status_bg = \"#060708\"\nbogus_key = \"#ffffff\"\n\n\
             [markdown]\nheading_1 = \"#0a0b0c\"\ntext = \"#0d0e0f\"\n\
             unknown_md = \"#123456\"\n\n\
             [diff]\nfile_separator_fg = \"#ffffff\"\n",
        )
        .unwrap();
        let (theme, warnings) = load_theme_file(&file, "sample").unwrap();
        assert_eq!(theme.name, "sample");
        assert_eq!(theme.syntax.as_deref(), Some("base16-ocean.dark"));
        // Overrides applied…
        assert_eq!(theme.ui.content_bg, rgb(1, 2, 3));
        assert_eq!(theme.ui.sidebar_bg, rgb(3, 4, 5));
        assert_eq!(theme.ui.status_bg, rgb(6, 7, 8));
        assert_eq!(theme.markdown.heading_1, rgb(10, 11, 12));
        assert_eq!(theme.markdown.text, rgb(13, 14, 15));
        // …everything else inherited from the base preset verbatim.
        assert_eq!(
            theme.markdown.heading_2,
            engine::markdown_theme_by_preset(MarkdownThemePreset::OceanDark).heading_2
        );
        assert_eq!(
            theme.ui.sidebar_border_focused,
            ui_by_preset(MarkdownThemePreset::OceanDark).sidebar_border_focused
        );
        // Unknown keys warned per section, never silently dropped.
        let joined = warnings.join("\n");
        assert!(
            joined.contains("unknown [ui] keys ignored: bogus_key"),
            "{joined}"
        );
        assert!(
            joined.contains("unknown [markdown] keys ignored: unknown_md"),
            "{joined}"
        );
        assert!(
            joined.contains("[diff] has no diff view here; keys ignored: file_separator_fg"),
            "{joined}"
        );
    }

    #[test]
    fn leaf_vocabulary_file_in_the_frond_dir_warns_unknown_ui_keys() {
        // A leaf-vocabulary theme file dropped into the frond themes dir: its
        // leaf `[ui]` keys are NOT the app vocabulary anymore — every one of
        // them warns, the chrome keeps the base preset's values, and the
        // load never fails or panics.
        let dir = TempDir::new("theme-leaf-ui-in-frond");
        let file = dir.path().join("leafish.toml");
        std::fs::write(
            &file,
            "base = \"forest\"\nappearance = \"dark\"\n\
             [ui]\n\
             toc_accent = \"#010203\"\n\
             toc_border = \"#040506\"\n\
             status_warning_fg = \"#070809\"\n\
             status_shortcut_fg = \"#0a0b0c\"\n\
             status_watch_fg = \"#0d0e0f\"\n\
             status_search_fg = \"#101112\"\n\
             status_search_bg = \"#131415\"\n",
        )
        .unwrap();
        let (theme, warnings) = load_theme_file(&file, "leafish").unwrap();
        let forest = ui_by_preset(MarkdownThemePreset::Forest);
        assert_eq!(theme.ui, forest, "no leaf key styled the chrome");
        let joined = warnings.join("\n");
        for key in [
            "toc_accent",
            "toc_border",
            "status_warning_fg",
            "status_shortcut_fg",
            "status_watch_fg",
            "status_search_fg",
            "status_search_bg",
        ] {
            assert!(joined.contains(key), "{key} not warned: {joined}");
        }
        // The app-vocabulary keys in the same file still apply.
        let file = dir.path().join("mixed.toml");
        std::fs::write(
            &file,
            "appearance = \"dark\"\n[ui]\ntoc_accent = \"#010203\"\ncontent_bg = \"#040506\"\n",
        )
        .unwrap();
        let (theme, warnings) = load_theme_file(&file, "mixed").unwrap();
        assert_eq!(theme.ui.content_bg, rgb(4, 5, 6));
        assert!(warnings.join("\n").contains("toc_accent"));
        assert!(!warnings.join("\n").contains("content_bg"));
    }

    #[test]
    fn custom_theme_defaults_to_ocean_base_and_optional_syntax() {
        let dir = TempDir::new("theme-nobase");
        let file = dir.path().join("bare.toml");
        std::fs::write(
            &file,
            "appearance = \"dark\"\n[ui]\ncontent_bg = \"#111111\"\n",
        )
        .unwrap();
        let (theme, warnings) = load_theme_file(&file, "bare").unwrap();
        assert_eq!(theme.syntax, None, "no syntax key: no override");
        assert!(warnings.is_empty());
        // Default base = ocean (leaf's DEFAULT_PRESET).
        assert_eq!(
            theme.markdown,
            engine::markdown_theme_by_preset(MarkdownThemePreset::OceanDark)
        );
        assert_eq!(theme.ui.content_bg, rgb(0x11, 0x11, 0x11));
    }

    #[test]
    fn custom_theme_rejects_unknown_base() {
        let dir = TempDir::new("theme-badbase");
        let file = dir.path().join("bad.toml");
        std::fs::write(&file, "base = \"not-a-preset\"\nappearance = \"dark\"\n").unwrap();
        let err = load_theme_file(&file, "bad").unwrap_err();
        assert!(err.contains("unknown base theme"), "{err}");
    }

    #[test]
    fn malformed_theme_file_is_an_error_not_a_crash() {
        let dir = TempDir::new("theme-malformed");
        let file = dir.path().join("broken.toml");
        std::fs::write(&file, "[ui]\ncontent_bg = \"not-a-color\"\n").unwrap();
        assert!(load_theme_file(&file, "broken").is_err());
        std::fs::write(&file, "!!!not toml!!!").unwrap();
        assert!(load_theme_file(&file, "broken").is_err());
    }

    #[test]
    fn discovery_first_dir_wins_and_is_sorted() {
        let a = TempDir::new("theme-dir-a");
        let b = TempDir::new("theme-dir-b");
        for (dir, names) in [
            (a.path(), vec!["zeta", "shared"]),
            (b.path(), vec!["alpha", "shared"]),
        ] {
            for name in names {
                std::fs::write(dir.join(format!("{name}.toml")), "").unwrap();
            }
        }
        // Non-toml files and missing dirs are skipped.
        std::fs::write(a.path().join("notes.txt"), "x").unwrap();
        let missing = PathBuf::from("/nonexistent/frond-themes");
        let found =
            discover_custom_themes(&[a.path().to_path_buf(), b.path().to_path_buf(), missing]);
        let names: Vec<&String> = found.keys().collect();
        assert_eq!(
            names,
            vec!["alpha", "shared", "zeta"],
            "sorted, both dirs merged"
        );
        // First directory wins for a shared name.
        assert_eq!(found["shared"], a.path().join("shared.toml"));
    }

    #[test]
    fn resolve_theme_accepts_presets_builtins_and_customs() {
        let dir = TempDir::new("theme-resolve");
        std::fs::write(
            dir.path().join("myscheme.toml"),
            "base = \"ocean\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        let custom = discover_custom_themes(&[dir.path().to_path_buf()]);

        // Preset names (leaf's aliases included) all resolve.
        for name in [
            "arctic",
            "ocean",
            "ocean-dark",
            "dark",
            "forest",
            "solarized",
            "solarized-dark",
        ] {
            assert!(resolve_theme(name, &custom).is_ok(), "{name}");
        }
        assert_eq!(
            resolve_theme("ocean-dark", &custom).unwrap().theme.name,
            "ocean",
            "aliases map to the canonical preset"
        );
        // Built-in names resolve (no custom file needed)…
        for spec in crate::builtin_themes::BUILTIN_THEMES {
            let resolved = resolve_theme(spec.name, &custom).unwrap();
            assert_eq!(resolved.theme.name, spec.name);
            assert!(resolved.warnings.is_empty(), "{}", spec.name);
        }
        // …and win over a custom file of the same name (builtins before
        // customs in the resolution order).
        let resolved = resolve_theme("dracula", &custom).unwrap();
        assert_eq!(resolved.theme.name, "dracula");
        assert!(resolved.warnings.is_empty(), "the built-in, not the file");
        // Discovered custom name (not a builtin).
        let resolved = resolve_theme("myscheme", &custom).unwrap();
        assert_eq!(resolved.theme.name, "myscheme");
        assert!(resolved.warnings.is_empty());
        // Unknown names are errors (caller warns + falls back).
        assert!(resolve_theme("no-such-theme", &custom).is_err());
        assert!(resolve_theme("", &custom).is_err());
        assert!(resolve_theme("  ", &custom).is_err());
    }

    #[test]
    fn effective_theme_name_follows_the_precedence_chain() {
        assert_eq!(
            effective_theme_name(Some("arctic"), Some("forest"), Some("ocean")),
            "arctic"
        );
        assert_eq!(
            effective_theme_name(None, Some("forest"), Some("ocean")),
            "forest"
        );
        assert_eq!(effective_theme_name(None, None, Some("ocean")), "ocean");
        assert_eq!(effective_theme_name(None, None, None), "ocean");
    }

    #[test]
    fn default_theme_dirs_is_the_single_frond_dir() {
        // Without the FROND_THEMES override (tests never rely on the user's
        // real HOME): shape only — the one frond dir; the leaf compat
        // directory is no longer read.
        // (The FROND_THEMES branch is covered by the smoke suite.)
        let mut env = crate::testutil::EnvGuard::lock();
        env.remove("FROND_THEMES");
        let dirs = default_theme_dirs();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with(".config/frond/themes"), "{:?}", dirs[0]);
    }

    #[test]
    fn choices_are_presets_builtins_customs_and_cycle_wraps() {
        let dir = TempDir::new("theme-cycle");
        std::fs::write(
            dir.path().join("z-theme.toml"),
            "base = \"ocean\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("a-theme.toml"),
            "base = \"forest\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        let custom = discover_custom_themes(&[dir.path().to_path_buf()]);
        let (mut choices, warnings) = ThemeChoices::build(&custom);
        assert!(warnings.is_empty());
        let names: Vec<&str> = choices
            .entries
            .iter()
            .map(|choice| choice.theme.name.as_str())
            .collect();
        // leaf's THEME_PRESETS order, then the 14 built-ins alphabetically,
        // then customs alphabetically.
        let mut expected = vec!["arctic", "forest", "ocean", "solarized"];
        expected.extend(crate::builtin_themes::BUILTIN_THEMES.iter().map(|s| s.name));
        expected.extend(["a-theme", "z-theme"]);
        assert_eq!(names, expected);
        // Built-ins sit between the presets and the customs, and their
        // slice is strictly ascending — the table order must really be
        // alphabetical (the cycle order), not just mirrored from the table.
        assert_eq!(names[4], "catppuccin-frappe");
        assert_eq!(names[17], "tokyonight-storm");
        assert!(
            names[4..18].windows(2).all(|w| w[0] < w[1]),
            "built-ins cycle in alphabetical order: {:?}",
            &names[4..18]
        );
        // Default position is ocean (index 2).
        assert_eq!(choices.current().theme.name, "ocean");
        assert_eq!(choices.cycle(ThemeRotation::All).theme.name, "solarized");
        assert_eq!(
            choices.cycle(ThemeRotation::All).theme.name,
            "catppuccin-frappe",
            "built-ins follow the presets"
        );
        // Cycling through everything wraps back to the first preset.
        let mut guard = 0;
        while choices.current().theme.name != "arctic" {
            choices.cycle(ThemeRotation::All);
            guard += 1;
            assert!(guard <= choices.entries.len(), "cycle does not wrap");
        }
        assert_eq!(choices.cycle(ThemeRotation::All).theme.name, "forest");
        // select() positions on a name, unknown names report false.
        assert!(choices.select("ocean"));
        assert_eq!(choices.current().theme.name, "ocean");
        assert!(!choices.select("nope"));
        assert_eq!(choices.current().theme.name, "ocean");
    }

    #[test]
    fn unparsable_custom_theme_is_skipped_with_a_warning() {
        let dir = TempDir::new("theme-broken-file");
        std::fs::write(
            dir.path().join("good.toml"),
            "base = \"ocean\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.toml"),
            "base = \"nope\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("noisy.toml"),
            "appearance = \"dark\"\n[ui]\nbogus = \"#ffffff\"\n",
        )
        .unwrap();
        let custom = discover_custom_themes(&[dir.path().to_path_buf()]);
        let (choices, load_errors) = ThemeChoices::build(&custom);
        assert_eq!(
            choices.entries.len(),
            20,
            "4 presets + 14 built-ins + the good custom + the noisy one"
        );
        // Parse failures are reported to the caller (always surfaced)...
        assert_eq!(load_errors.len(), 1);
        assert!(load_errors[0].contains("bad"));
        // ...but unknown-key warnings attach to their theme and surface
        // only when it is applied, never at mere discovery time.
        let noisy = choices
            .entries
            .iter()
            .find(|choice| choice.theme.name == "noisy")
            .expect("noisy theme loaded into the cycle");
        assert_eq!(noisy.warnings.len(), 1, "attached, not dropped");
        let good = choices
            .entries
            .iter()
            .find(|choice| choice.theme.name == "good")
            .expect("good theme loaded");
        assert!(good.warnings.is_empty());
        assert!(
            choices.current().warnings.is_empty(),
            "presets: no warnings"
        );
    }

    #[test]
    fn custom_theme_named_like_a_reserved_theme_is_skipped_with_a_warning() {
        // A custom file whose name matches a preset or built-in must never
        // load: one cycle entry per name (the reserved theme), and the
        // shadowed file surfaces a warning — never silently.
        let dir = TempDir::new("theme-collision");
        std::fs::write(
            dir.path().join("dracula.toml"),
            "base = \"forest\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("ocean.toml"),
            "base = \"forest\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("mine.toml"),
            "base = \"ocean\"\nappearance = \"dark\"\n",
        )
        .unwrap();
        let custom = discover_custom_themes(&[dir.path().to_path_buf()]);

        // Cycle: single "dracula" entry (the built-in), the preset-named
        // file skipped, the non-colliding custom still loaded.
        let (choices, load_errors) = ThemeChoices::build(&custom);
        let names: Vec<&str> = choices
            .entries
            .iter()
            .map(|choice| choice.theme.name.as_str())
            .collect();
        assert_eq!(
            names.iter().filter(|name| **name == "dracula").count(),
            1,
            "one cycle entry per name: {names:?}"
        );
        assert!(
            names.iter().filter(|name| **name == "ocean").count() == 1,
            "one cycle entry per name: {names:?}"
        );
        // The single "ocean" is the preset (position 2), not the custom.
        assert_eq!(names[2], "ocean");
        assert_eq!(names.last(), Some(&"mine"), "non-colliding custom loads");
        // One warning per colliding file, reaching the load-error path.
        assert_eq!(load_errors.len(), 2, "{load_errors:?}");
        assert!(load_errors.iter().any(|err| {
            err.contains("\"dracula\"") && err.contains("built-in") && err.contains("not loaded")
        }));
        assert!(load_errors.iter().any(|err| {
            err.contains("\"ocean\"") && err.contains("preset") && err.contains("not loaded")
        }));

        // Resolution: the built-in wins and carries the warning (status
        // bar / stderr), proving the built-in palette is what applied.
        let resolved = resolve_theme("dracula", &custom).unwrap();
        assert_eq!(resolved.theme.name, "dracula");
        assert_eq!(
            resolved.warnings,
            vec![shadow_warning("dracula", "built-in")]
        );
        assert_eq!(
            resolved.theme.ui.content_bg,
            rgb(0x28, 0x2a, 0x36),
            "the built-in, not the custom file"
        );
        let resolved = resolve_theme("ocean", &custom).unwrap();
        assert_eq!(resolved.theme.name, "ocean");
        assert_eq!(resolved.warnings, vec![shadow_warning("ocean", "preset")]);
        // No collision → no warning (the plain built-in path stays quiet).
        let resolved = resolve_theme("nord", &custom).unwrap();
        assert_eq!(resolved.theme.name, "nord");
        assert!(resolved.warnings.is_empty());
    }

    #[test]
    fn every_builtin_theme_resolves_and_is_not_its_base_preset() {
        // Resolution: each built-in resolves by name (before any custom
        // discovery), carries its file's syntax key, and its chrome palette
        // is the ported file's — not preset-default leakage (at least one
        // field must differ from the base preset's `ui_by_preset`).
        let empty = BTreeMap::new();
        for spec in crate::builtin_themes::BUILTIN_THEMES {
            let resolved = resolve_theme(spec.name, &empty)
                .unwrap_or_else(|err| panic!("builtin \"{}\": {err}", spec.name));
            assert_eq!(resolved.theme.name, spec.name);
            assert!(resolved.warnings.is_empty(), "{}", spec.name);
            assert!(resolved.theme.syntax.is_some(), "{}", spec.name);
            assert_ne!(
                resolved.theme.ui,
                ui_by_preset(spec.base),
                "\"{}\" leaked its base preset's chrome",
                spec.name
            );
        }
    }

    #[test]
    fn builtin_ui_values_match_the_source_tomls() {
        // Spot-checks copied from the leaf theme files the table was ported
        // from (`content_bg`, `toc_accent` → sidebar_border_focused,
        // `status_success_fg` → status_ahead_fg, `heading_1`, `toc_bg` →
        // sidebar_bg, `status_bg` → status_bg), so a bad port shows up as
        // a mismatch against the source value.
        let cases: &[(&str, Color, Color, Color, Color, Color, Color)] = &[
            (
                "catppuccin-frappe",
                rgb(0x30, 0x34, 0x46),
                rgb(0xba, 0xbb, 0xf1),
                rgb(0xa6, 0xd1, 0x89),
                rgb(0xca, 0x9e, 0xe6),
                rgb(0x29, 0x2c, 0x3c),
                rgb(0x29, 0x2c, 0x3c),
            ),
            (
                "catppuccin-latte",
                rgb(0xef, 0xf1, 0xf5),
                rgb(0x72, 0x87, 0xfd),
                rgb(0x40, 0xa0, 0x2b),
                rgb(0x88, 0x39, 0xef),
                rgb(0xe6, 0xe9, 0xef),
                rgb(0xe6, 0xe9, 0xef),
            ),
            (
                "catppuccin-macchiato",
                rgb(0x24, 0x27, 0x3a),
                rgb(0xb7, 0xbd, 0xf8),
                rgb(0xa6, 0xda, 0x95),
                rgb(0xc6, 0xa0, 0xf6),
                rgb(0x1e, 0x20, 0x30),
                rgb(0x1e, 0x20, 0x30),
            ),
            (
                "catppuccin-mocha",
                rgb(0x1e, 0x1e, 0x2e),
                rgb(0xb4, 0xbe, 0xfe),
                rgb(0xa6, 0xe3, 0xa1),
                rgb(0xcb, 0xa6, 0xf7),
                rgb(0x18, 0x18, 0x25),
                rgb(0x18, 0x18, 0x25),
            ),
            (
                "dracula-alucard",
                rgb(0xff, 0xfb, 0xeb),
                rgb(0x64, 0x4a, 0xc9),
                rgb(0x14, 0x71, 0x0a),
                rgb(0xa3, 0x14, 0x4d),
                rgb(0xe2, 0xde, 0xca),
                rgb(0xe2, 0xde, 0xca),
            ),
            (
                "dracula",
                rgb(0x28, 0x2a, 0x36),
                rgb(0xbd, 0x93, 0xf9),
                rgb(0x50, 0xfa, 0x7b),
                rgb(0xff, 0x79, 0xc6),
                rgb(0x19, 0x1a, 0x21),
                rgb(0x19, 0x1a, 0x21),
            ),
            (
                "gruvbox",
                rgb(0x28, 0x28, 0x28),
                rgb(0xfe, 0x80, 0x19),
                rgb(0xb8, 0xbb, 0x26),
                rgb(0xfa, 0xbd, 0x2f),
                rgb(0x1d, 0x20, 0x21),
                rgb(0x1d, 0x20, 0x21),
            ),
            (
                "nord",
                rgb(0x2e, 0x34, 0x40),
                rgb(0x88, 0xc0, 0xd0),
                rgb(0xa3, 0xbe, 0x8c),
                rgb(0x88, 0xc0, 0xd0),
                rgb(0x3b, 0x42, 0x52),
                rgb(0x3b, 0x42, 0x52),
            ),
            (
                "rose-pine-dawn",
                rgb(0xfa, 0xf4, 0xed),
                rgb(0xd7, 0x82, 0x7e),
                rgb(0x56, 0x94, 0x9f),
                rgb(0xb4, 0x63, 0x7a),
                rgb(0xff, 0xfa, 0xf3),
                rgb(0xff, 0xfa, 0xf3),
            ),
            (
                "rose-pine-moon",
                rgb(0x23, 0x21, 0x36),
                rgb(0xea, 0x9a, 0x97),
                rgb(0x9c, 0xcf, 0xd8),
                rgb(0xeb, 0x6f, 0x92),
                rgb(0x2a, 0x27, 0x3f),
                rgb(0x2a, 0x27, 0x3f),
            ),
            (
                "rose-pine",
                rgb(0x19, 0x17, 0x24),
                rgb(0xeb, 0xbc, 0xba),
                rgb(0x9c, 0xcf, 0xd8),
                rgb(0xeb, 0x6f, 0x92),
                rgb(0x1f, 0x1d, 0x2e),
                rgb(0x1f, 0x1d, 0x2e),
            ),
            (
                "tokyonight-day",
                rgb(0xe1, 0xe2, 0xe7),
                rgb(0x2e, 0x7d, 0xe9),
                rgb(0x58, 0x75, 0x39),
                rgb(0x98, 0x54, 0xf1),
                rgb(0xd5, 0xd8, 0xe6),
                rgb(0xd5, 0xd8, 0xe6),
            ),
            (
                "tokyonight-night",
                rgb(0x1a, 0x1b, 0x26),
                rgb(0x7a, 0xa2, 0xf7),
                rgb(0x9e, 0xce, 0x6a),
                rgb(0xbb, 0x9a, 0xf7),
                rgb(0x16, 0x16, 0x1e),
                rgb(0x16, 0x16, 0x1e),
            ),
            (
                "tokyonight-storm",
                rgb(0x24, 0x28, 0x3b),
                rgb(0x7a, 0xa2, 0xf7),
                rgb(0x9e, 0xce, 0x6a),
                rgb(0xbb, 0x9a, 0xf7),
                rgb(0x1f, 0x23, 0x35),
                rgb(0x1f, 0x23, 0x35),
            ),
        ];
        for (name, content_bg, toc_accent, success, heading_1, sidebar_bg, status_bg) in cases {
            let theme = resolve_theme(name, &BTreeMap::new())
                .unwrap_or_else(|err| panic!("builtin \"{name}\": {err}"))
                .theme;
            assert_eq!(theme.ui.content_bg, *content_bg, "{name} content_bg");
            assert_eq!(
                theme.ui.sidebar_border_focused, *toc_accent,
                "{name} toc_accent"
            );
            assert_eq!(
                theme.ui.status_ahead_fg, *success,
                "{name} status_success_fg → ahead arrow"
            );
            assert_eq!(theme.markdown.heading_1, *heading_1, "{name} heading_1");
            assert_eq!(theme.ui.sidebar_bg, *sidebar_bg, "{name} sidebar_bg");
            assert_eq!(theme.ui.status_bg, *status_bg, "{name} status_bg");
        }
    }

    /// Every theme's declared appearance must match its own background:
    /// a light theme with a dark `content_bg` (or the reverse) fails here,
    /// so the labels cannot drift from the colors they describe.
    #[test]
    fn declared_appearance_matches_the_background_of_every_theme() {
        fn background_is_light(theme: &Theme) -> bool {
            let Color::Rgb(r, g, b) = theme.ui.content_bg else {
                panic!("{}: content_bg is not an RGB color", theme.name);
            };
            0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b) > 128.0
        }
        let presets = [
            MarkdownThemePreset::Arctic,
            MarkdownThemePreset::Forest,
            MarkdownThemePreset::OceanDark,
            MarkdownThemePreset::SolarizedDark,
        ];
        let mut themes: Vec<Theme> = presets.into_iter().map(preset_theme).collect();
        themes.extend(
            crate::builtin_themes::BUILTIN_THEMES
                .iter()
                .map(|spec| builtin_theme(spec.name).unwrap()),
        );
        for theme in &themes {
            assert_eq!(
                theme.appearance == Appearance::Light,
                background_is_light(theme),
                "{} declares {:?} but its background disagrees",
                theme.name,
                theme.appearance
            );
        }
    }

    #[test]
    fn custom_theme_must_declare_its_appearance() {
        let dir = TempDir::new("theme-appearance");
        let missing = dir.path().join("missing.toml");
        std::fs::write(&missing, "base = \"ocean\"\n").unwrap();
        let err = load_theme_file(&missing, "missing").unwrap_err();
        assert!(err.contains("must set appearance"), "{err}");

        let bogus = dir.path().join("bogus.toml");
        std::fs::write(&bogus, "appearance = \"sepia\"\n").unwrap();
        assert!(load_theme_file(&bogus, "bogus").is_err());

        let light = dir.path().join("light.toml");
        std::fs::write(&light, "appearance = \"Light\"\n").unwrap();
        let (theme, _) = load_theme_file(&light, "light").unwrap();
        assert_eq!(theme.appearance, Appearance::Light);
    }

    #[test]
    fn theme_rotation_parses_steps_and_filters() {
        assert_eq!(ThemeRotation::parse(" Dark "), Some(ThemeRotation::Dark));
        assert_eq!(ThemeRotation::parse("sepia"), None);
        assert_eq!(ThemeRotation::All.next(), ThemeRotation::Light);
        assert_eq!(ThemeRotation::Light.next(), ThemeRotation::Dark);
        assert_eq!(ThemeRotation::Dark.next(), ThemeRotation::All);
        assert!(ThemeRotation::Light.allows(Appearance::Light));
        assert!(!ThemeRotation::Light.allows(Appearance::Dark));
        assert!(ThemeRotation::All.allows(Appearance::Dark));
    }

    #[test]
    fn cycle_stays_inside_the_rotation_group_and_wraps() {
        let (mut choices, _) = ThemeChoices::build(&BTreeMap::new());
        let light: Vec<String> = choices
            .entries
            .iter()
            .filter(|c| c.theme.appearance == Appearance::Light)
            .map(|c| c.theme.name.clone())
            .collect();
        assert!(light.len() >= 2, "the light group has several themes");
        assert_eq!(choices.count_in(ThemeRotation::Light), light.len());

        // From the default (dark) theme, the first step enters the light group
        // at the first light theme after it; one full pass then covers the
        // group exactly once, and the pass wraps back to where it started.
        choices.cycle(ThemeRotation::Light);
        let start = choices.current().theme.name.clone();
        let mut seen = Vec::new();
        for _ in 0..light.len() {
            seen.push(choices.current().theme.name.clone());
            choices.cycle(ThemeRotation::Light);
        }
        let mut sorted = seen.clone();
        sorted.sort();
        let mut expected = light.clone();
        expected.sort();
        assert_eq!(sorted, expected, "one pass visits each light theme once");
        assert_eq!(
            choices.current().theme.name,
            start,
            "the pass wraps to its start"
        );
    }
}
