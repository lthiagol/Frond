//! Wikilink preprocessing: turns `[[…]]` links into plain markdown links
//! with a `vault://` URL before the engine parses the note, and provides
//! the URL codec + post-parse link styling helpers.
//!
//! Why preprocess instead of teaching the engine about wikilinks: the
//! vendored leaf markdown renders standard markdown links (styled text +
//! `LinkSpan`/`link_urls` bookkeeping) already, so a rewrite
//! `[[target|alias]]` → `[alias](vault://N/target)` reuses all of that for
//! free — including wrapping and tables (frontmatter links render inside
//! the frontmatter table). The `N` occurrence index makes each URL unique
//! so rendered links can be mapped back to their resolution state even if
//! the note also contains regular markdown links.
//!
//! Embeds (`![[…]]`) are left as raw text: rendering them is a v1 non-goal
//! (SPEC), and turning them into links would be wrong.

use std::fmt::Write as _;
use std::path::PathBuf;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

use crate::theme::UiTheme;
use engine::LinkSpan;
use vault::wikilink::{extract_wikilink_matches, LinkRef};

/// URL scheme marking a preprocessed wikilink.
pub const VAULT_SCHEME: &str = "vault://";

/// Resolution state of one rendered wikilink, computed at parse time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Single candidate (possibly narrowed from an ambiguity by the
    /// same-folder/nearest-ancestor hint).
    Resolved(PathBuf),
    /// Still ambiguous after the hint — the app shows a disambiguation
    /// popup on activation.
    Ambiguous(Vec<PathBuf>),
    /// No note matches; rendered dimmed/strikethrough, not activatable.
    NotFound,
}

/// A preprocessed wikilink: what it points at and how it resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikilinkTarget {
    pub target: String,
    pub heading: Option<String>,
    pub state: LinkState,
}

/// Rewrites every non-embed wikilink in `source` into a markdown link
/// whose URL is `vault://<occurrence>/<encoded-target>` plus an optional
/// `#<encoded-heading>` fragment. Returns the rewritten source and the
/// non-embed links in document order (occurrence `i` in the rewritten
/// source is `links[i]`).
pub fn preprocess_with_links(source: &str) -> (String, Vec<LinkRef>) {
    let matches = extract_wikilink_matches(source);
    let links: Vec<LinkRef> = matches
        .iter()
        .filter(|m| !m.link.is_embed)
        .map(|m| m.link.clone())
        .collect();
    let mut out = String::with_capacity(source.len());
    let mut last = 0usize;
    let mut occurrence = 0usize;
    for m in &matches {
        if m.link.is_embed {
            continue; // embeds stay raw (v1 non-goal)
        }
        out.push_str(&source[last..m.start]);
        let display = display_text(&m.link);
        let url = vault_url(occurrence, &m.link.target, m.link.heading.as_deref());
        out.push('[');
        out.push_str(&escape_link_text(&display));
        out.push_str("](");
        out.push_str(&url);
        out.push(')');
        last = m.end;
        occurrence += 1;
    }
    out.push_str(&source[last..]);
    (out, links)
}

/// The displayed text for a wikilink: the alias if present, else
/// `target#heading` (or just the heading for `[[#heading]]` self-links).
fn display_text(link: &LinkRef) -> String {
    if let Some(alias) = &link.alias {
        return alias.clone();
    }
    match (&link.heading, link.target.is_empty()) {
        (Some(h), true) => h.clone(),
        (Some(h), false) => format!("{}#{}", link.target, h),
        (None, _) => link.target.clone(),
    }
}

/// Escapes markdown-active characters in link display text so the inline
/// parser keeps the intended label (a `]` inside `[[a]b]]` would
/// otherwise terminate the link text early). Backslashes are escaped
/// first: a display text ending in `\` would otherwise pair with the
/// closing `]` (`\]` is an escape sequence) and the link would never
/// close.
fn escape_link_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// Builds the `vault://` URL for wikilink occurrence `occurrence`.
pub fn vault_url(occurrence: usize, target: &str, heading: Option<&str>) -> String {
    let mut url = format!("{}{}/{}", VAULT_SCHEME, occurrence, percent_encode(target));
    if let Some(heading) = heading {
        url.push('#');
        url.push_str(&percent_encode(heading));
    }
    url
}

/// Parses a `vault://` URL back into `(occurrence, target, heading)`.
/// The first `#` separates the fragment (the target itself was
/// percent-encoded, so any literal `#` in it became `%23`).
pub fn parse_vault_url(url: &str) -> Option<(usize, String, Option<String>)> {
    let rest = url.strip_prefix(VAULT_SCHEME)?;
    let (index, rest) = rest.split_once('/')?;
    let index = index.parse::<usize>().ok()?;
    let (target, heading) = match rest.split_once('#') {
        Some((t, h)) => (t, Some(h)),
        None => (rest, None),
    };
    Some((index, percent_decode(target), heading.map(percent_decode)))
}

/// Percent-encodes everything outside the unreserved set plus `/`
/// (path separators inside `dir/name` targets stay readable).
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(*byte as char)
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Decodes `%XX` sequences; invalid sequences pass through verbatim and
/// non-UTF-8 bytes are replaced (lossy), matching best-effort rendering.
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where one rendered link lives on the parsed lines (a link may wrap
/// across several line ranges).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkRegion {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// One link in the parsed note: its engine link id, URL, where it
/// renders, and — for preprocessed wikilinks — what it points at and how
/// it resolved.
#[derive(Clone, Debug)]
pub struct RenderedLink {
    pub url: String,
    pub regions: Vec<LinkRegion>,
    /// `None` for links that were not produced by the preprocessing hook
    /// (regular markdown links in the note).
    pub wikilink: Option<WikilinkTarget>,
}

/// Groups engine `LinkSpan`s per link id (a wrapped link yields several
/// spans with the same id) and pairs each group with the corresponding
/// preprocessed wikilink: the URL's occurrence index selects
/// `wikilinks[i]`, but only when the URL is exactly the one the
/// preprocessing generated for that occurrence (a hand-authored
/// `vault://N/…` markdown link stays a plain link). Groups appear in
/// first-appearance order.
pub fn build_rendered_links(
    link_urls: &[String],
    link_spans: &[LinkSpan],
    wikilinks: &[WikilinkTarget],
) -> Vec<RenderedLink> {
    let mut links: Vec<RenderedLink> = Vec::new();
    // link id → position in `links`, so grouping stays linear in the spans.
    let mut by_id: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for span in link_spans {
        let Some(url) = link_urls.get(span.link_id.0) else {
            continue;
        };
        let region = LinkRegion {
            line: span.line_idx,
            start: span.start_col,
            end: span.end_col,
        };
        if let Some(&at) = by_id.get(&span.link_id.0) {
            links[at].regions.push(region);
            continue;
        }
        let wikilink = parse_vault_url(url).and_then(|(index, _target, _heading)| {
            // Only URLs the preprocessing produced *this session* count as
            // wikilinks: the occurrence must be in bounds and the URL must
            // be exactly what `vault_url` generates for it. A hand-authored
            // `vault://N/…` markdown link (or one with a literal
            // percent-escape decoded a level too eagerly) then fails the
            // exact-match check and stays a plain link instead of
            // inheriting wikilink N's resolution state.
            let wikilink = wikilinks.get(index)?;
            let expected = vault_url(index, &wikilink.target, wikilink.heading.as_deref());
            (url == &expected).then(|| WikilinkTarget {
                target: wikilink.target.clone(),
                heading: wikilink.heading.clone(),
                state: wikilink.state.clone(),
            })
        });
        by_id.insert(span.link_id.0, links.len());
        links.push(RenderedLink {
            url: url.clone(),
            regions: vec![region],
            wikilink,
        });
    }
    links
}

/// Link/checkbox state colors, derived from the active [`UiTheme`] roles so
/// they stay legible on every theme (including light ones): unresolved
/// links reuse the dimmed role, ambiguous ones the warning role, and the
/// focus highlight (links and checkboxes, tasks.rs) the selection
/// background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkColors {
    pub unresolved_fg: Color,
    pub ambiguous_fg: Color,
    pub focus_bg: Color,
}

impl LinkColors {
    pub fn from_ui(ui: UiTheme) -> Self {
        Self {
            unresolved_fg: ui.dimmed_fg,
            ambiguous_fg: ui.status_dirty_fg,
            focus_bg: ui.selection_bg,
        }
    }
}

impl Default for LinkColors {
    fn default() -> Self {
        Self::from_ui(crate::theme::ui_by_preset(
            engine::MarkdownThemePreset::default(),
        ))
    }
}

/// The base style for a link region given its resolution state.
fn state_style(state: &LinkState, base: Style, colors: LinkColors) -> Style {
    match state {
        // Resolved: keep the engine's link styling.
        LinkState::Resolved(_) => base,
        LinkState::NotFound => base
            .fg(colors.unresolved_fg)
            .add_modifier(Modifier::CROSSED_OUT),
        // Ambiguous after the hint: warning color.
        LinkState::Ambiguous(_) => base.fg(colors.ambiguous_fg),
    }
}

/// Restyles the parsed lines for every link: per-state base styling, plus
/// the focus highlight on `focused` (an index into `links`).
pub fn apply_link_styles(
    lines: &mut [Line<'static>],
    links: &[RenderedLink],
    focused: Option<usize>,
    colors: LinkColors,
) {
    for (index, link) in links.iter().enumerate() {
        let focused = focused == Some(index);
        for region in &link.regions {
            let Some(line) = lines.get_mut(region.line) else {
                continue;
            };
            restyle_region(line, region.start, region.end, |style| {
                let mut style = match &link.wikilink {
                    Some(w) => state_style(&w.state, style, colors),
                    None => style,
                };
                if focused {
                    style = style.bg(colors.focus_bg).add_modifier(Modifier::BOLD);
                }
                style
            });
        }
    }
}

/// Applies `f` to the style of every span overlapping the column range
/// `[start, end)` of `line`. Spans that straddle a boundary are styled in
/// full (the engine's wrapping keeps link spans contiguous, so the
/// overreach is at most one adjacent unlinked span on a boundary).
fn restyle_region(line: &mut Line<'static>, start: usize, end: usize, f: impl Fn(Style) -> Style) {
    for_spans_in_columns(line, start, end, |span| span.style = f(span.style));
}

/// Calls `f` on every non-empty span of `line` overlapping the display
/// column range `[start, end)` (columns accumulate over span widths). The
/// shared column walk behind link and checkbox restyling (tasks.rs).
pub(crate) fn for_spans_in_columns(
    line: &mut Line<'static>,
    start: usize,
    end: usize,
    mut f: impl FnMut(&mut ratatui::text::Span<'static>),
) {
    use unicode_width::UnicodeWidthStr;
    let mut col = 0usize;
    for span in &mut line.spans {
        let width = span.content.width();
        let span_end = col + width;
        if span_end > start && col < end && width > 0 {
            f(span);
        }
        col = span_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn preprocess_rewrites_basic_forms() {
        let (out, links) = preprocess_with_links("See [[alpha]] and [[beta|B]] and [[gamma#H]].");
        assert_eq!(
            out,
            "See [alpha](vault://0/alpha) and [B](vault://1/beta) and [gamma#H](vault://2/gamma#H)."
        );
        assert_eq!(links.len(), 3);
        assert_eq!(links[1].alias.as_deref(), Some("B"));
        assert_eq!(links[2].heading.as_deref(), Some("H"));
    }

    #[test]
    fn preprocess_self_links_and_encodes_spaces() {
        let (out, links) = preprocess_with_links("[[#My Heading]] and [[my note]]");
        assert_eq!(
            out,
            "[My Heading](vault://0/#My%20Heading) and [my note](vault://1/my%20note)"
        );
        assert_eq!(links[0].target, "");
        assert_eq!(links[0].heading.as_deref(), Some("My Heading"));
    }

    #[test]
    fn preprocess_leaves_embeds_and_code_untouched() {
        let source = "![[board.base]] keeps `[[code]]`\n```\n[[fenced]]\n```\n[[real]]";
        let (out, links) = preprocess_with_links(source);
        assert!(out.contains("![[board.base]]"), "embed stays raw");
        assert!(out.contains("`[[code]]`"), "inline code stays raw");
        assert!(out.contains("[[fenced]]"), "fenced stays raw");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
        assert!(out.contains("[real](vault://0/real)"));
    }

    #[test]
    fn preprocess_escapes_brackets_in_display_text() {
        let (out, _) = preprocess_with_links("[[a]b]]");
        assert!(out.contains("\\]"), "display `]` is escaped");
    }

    /// A display text ending in `\` must have the backslash escaped too:
    /// unescaped, `\]` reads as an escaped closing bracket and the link
    /// never closes (the rendered text loses the link entirely).
    #[test]
    fn preprocess_escapes_trailing_backslash_in_display_text() {
        let (out, links) = preprocess_with_links("[[note|alias\\]]");
        assert_eq!(links.len(), 1);
        assert!(out.ends_with("\\\\](vault://0/note)"), "got: {out}");
        // End-to-end: the engine must still see a well-formed link.
        let assets = engine::SyntaxAssets::load_defaults();
        let result = engine::parse_markdown_with_width(
            &out,
            assets.syntax_set,
            &assets.theme,
            80,
            &engine::default_markdown_theme(),
            false,
            true,
        );
        assert_eq!(
            result.link_urls,
            vec!["vault://0/note".to_string()],
            "the escaped link survives the engine parse"
        );
    }

    /// Only URLs the preprocessing produced this session count as
    /// wikilinks: a hand-authored markdown link with a `vault://N/…` URL
    /// must not inherit wikilink N's resolution state.
    #[test]
    fn hand_authored_vault_url_is_not_misattributed() {
        use engine::LinkId;
        let wikilinks = vec![WikilinkTarget {
            target: "alpha".into(),
            heading: None,
            state: LinkState::Resolved(PathBuf::from("/v/alpha.md")),
        }];
        let span_for = || LinkSpan {
            line_idx: 0,
            start_col: 0,
            end_col: 4,
            link_id: LinkId(0),
        };
        let rendered = |url: &str| {
            build_rendered_links(&[url.to_string()], &[span_for()], &wikilinks).remove(0)
        };

        // The exact URL the preprocessing produces for occurrence 0:
        // attributed (indistinguishable from ours).
        assert!(rendered("vault://0/alpha").wikilink.is_some());
        // A different target behind occurrence 0: plain link, no state.
        assert!(
            rendered("vault://0/other").wikilink.is_none(),
            "hand-authored URL must not inherit occurrence 0"
        );
        // Out-of-bounds occurrence.
        assert!(rendered("vault://7/alpha").wikilink.is_none());
        // A literal percent-escape (would have been decoded one level too
        // eagerly before): plain link.
        assert!(rendered("vault://0/alpha%20x").wikilink.is_none());
        // A fragment that does not match: plain link.
        assert!(rendered("vault://0/alpha#Extra").wikilink.is_none());
    }

    #[test]
    fn percent_codec_roundtrip() {
        for value in [
            "plain",
            "my note",
            "dir/nested/name",
            "100% done",
            "a#b",
            "héllo",
            "a|b",
        ] {
            assert_eq!(percent_decode(&percent_encode(value)), value);
        }
        assert_eq!(percent_encode("my note"), "my%20note");
        assert_eq!(percent_encode("a/b"), "a/b", "slash stays readable");
        assert_eq!(
            percent_decode("bad%2"),
            "bad%2",
            "invalid sequence passes through"
        );
    }

    #[test]
    fn vault_url_roundtrip() {
        let url = vault_url(3, "my note", Some("Sect #2"));
        assert_eq!(url, "vault://3/my%20note#Sect%20%232");
        assert_eq!(
            parse_vault_url(&url),
            Some((3, "my note".to_string(), Some("Sect #2".to_string())))
        );
        assert_eq!(
            parse_vault_url("vault://0/alpha"),
            Some((0, "alpha".to_string(), None))
        );
        assert_eq!(parse_vault_url("https://example.com"), None);
        assert_eq!(parse_vault_url("vault://nope/x"), None);
    }

    fn line_of(text: &str) -> Line<'_> {
        Line::from(vec![Span::raw("see "), Span::raw(text), Span::raw(" end")])
    }

    fn span_style(line: &Line<'_>, index: usize) -> Style {
        line.spans[index].style
    }

    #[test]
    fn styles_applied_per_state() {
        let colors = LinkColors::default();
        let mut line = line_of("link");
        let links = vec![RenderedLink {
            url: "vault://0/x".into(),
            regions: vec![LinkRegion {
                line: 0,
                start: 4,
                end: 8,
            }],
            wikilink: Some(WikilinkTarget {
                target: "x".into(),
                heading: None,
                state: LinkState::NotFound,
            }),
        }];
        apply_link_styles(std::slice::from_mut(&mut line), &links, None, colors);
        let style = span_style(&line, 1);
        assert_eq!(style.fg, Some(colors.unresolved_fg));
        assert!(style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn focus_highlight_and_unfocus() {
        let colors = LinkColors::default();
        let pristine = line_of("link");
        let links = vec![RenderedLink {
            url: "vault://0/x".into(),
            regions: vec![LinkRegion {
                line: 0,
                start: 4,
                end: 8,
            }],
            wikilink: Some(WikilinkTarget {
                target: "x".into(),
                heading: None,
                state: LinkState::Resolved(PathBuf::from("/v/x.md")),
            }),
        }];
        // Focused: highlighted. (Styling always starts from the pristine
        // engine output in the viewer; here we assert each application
        // against a fresh copy.)
        let mut line = pristine.clone();
        apply_link_styles(std::slice::from_mut(&mut line), &links, Some(0), colors);
        assert_eq!(span_style(&line, 1).bg, Some(colors.focus_bg));
        // Unfocused: plain engine style.
        let mut line = pristine.clone();
        apply_link_styles(std::slice::from_mut(&mut line), &links, None, colors);
        assert_eq!(span_style(&line, 1).bg, None);
        // The untouched neighbor spans stay raw.
        assert_eq!(span_style(&line, 0).bg, None);
    }
}
