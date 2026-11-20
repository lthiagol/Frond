//! Global vault search (`s`): a modal popup with an input prompt and a
//! results list.
//!
//! Flow: `s` opens the prompt (input mode); typing builds the query;
//! Enter runs the search against the vault index's cached raw lines and
//! switches to results mode; `j`/`k` move the selection; Enter on a hit
//! opens that note scrolled to the matched raw line (through the normal
//! navigation stack); Esc closes. In results mode, typing a character
//! returns to input mode with that character appended (refine in place).
//!
//! The popup is terminal-independent state; rendering lives in `ui.rs`
//! ([`crate::ui::draw_search`]).

use unicode_width::UnicodeWidthChar;
use vault::{SearchHit, SearchPattern, SearchResults};

use crate::ui::PopupChrome;

/// Which phase the search popup is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    /// Typing the query.
    Input,
    /// Browsing the matches of the last executed search.
    Results,
}

/// A parsed query: `re:` selects the regular-expression engine and is
/// stripped; every other query is the literal substring search. The popup
/// owns this split so the vault's two search paths stay separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchQuery<'a> {
    Literal(&'a str),
    Regex(&'a str),
}

impl<'a> SearchQuery<'a> {
    pub fn parse(query: &'a str) -> Self {
        match query.strip_prefix("re:") {
            Some(pattern) => Self::Regex(pattern),
            None => Self::Literal(query),
        }
    }
}

pub struct SearchState {
    pub open: bool,
    pub mode: SearchMode,
    pub query: String,
    pub hits: Vec<SearchHit>,
    /// `true` when the last search hit the vault crate's result cap
    /// ([`vault::SEARCH_RESULT_CAP`]) — the popup title shows "N+".
    pub capped: bool,
    pub selected: usize,
    /// Why the last query could not run (an invalid regular expression).
    /// Shown in the popup footer while the popup stays in the input mode.
    pub error: Option<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            open: false,
            mode: SearchMode::Input,
            query: String::new(),
            hits: Vec::new(),
            capped: false,
            selected: 0,
            error: None,
        }
    }
}

impl SearchState {
    /// `s`: opens the popup fresh (empty query, input mode).
    pub fn open_input(&mut self) {
        self.open = true;
        self.mode = SearchMode::Input;
        self.query.clear();
        self.hits.clear();
        self.capped = false;
        self.selected = 0;
        self.error = None;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Appends one typed character and drops a stale error message: the
    /// footer describes the last attempted query, not the one being typed.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.error = None;
    }

    /// Edits the query with Backspace, dropping a stale error message.
    pub fn backspace(&mut self) {
        self.query.pop();
        self.error = None;
    }

    /// Enter in input mode: runs the query against the index and switches
    /// to results mode. An empty query is a no-op (nothing to search for).
    /// A query starting with `re:` is matched as a regular expression; an
    /// invalid pattern is reported in `error`, nothing is searched, and
    /// the popup stays in the input mode with the query intact.
    ///
    /// Returns `true` when the query ran (the popup is in results mode),
    /// `false` when it never ran — the app keeps only executed searches as
    /// its active search.
    pub fn run(
        &mut self,
        search: impl FnOnce(SearchQuery<'_>) -> Result<SearchResults, String>,
    ) -> bool {
        if self.query.trim().is_empty() {
            return false;
        }
        match search(SearchQuery::parse(&self.query)) {
            Ok(results) => {
                self.hits = results.hits;
                self.capped = results.capped;
                self.mode = SearchMode::Results;
                self.selected = 0;
                self.error = None;
                true
            }
            Err(message) => {
                self.hits.clear();
                self.capped = false;
                self.selected = 0;
                self.mode = SearchMode::Input;
                self.error = Some(message);
                false
            }
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let len = self.hits.len();
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.hits.get(self.selected)
    }
}

/// The last executed search, kept after the popup closes so the note
/// pane's `n`/`N` can walk its occurrences in the open note. The pattern is
/// the vault's own search rule ([`vault::SearchPattern`]), so a query finds
/// the same lines in the note as it does in the vault.
#[derive(Clone, Debug)]
pub struct ActiveSearch {
    pattern: SearchPattern,
}

impl ActiveSearch {
    /// The active search for a query the popup executed, or `None` when
    /// the query can never match: a blank query, a bare `re:`, or a
    /// regular expression that does not compile (it ran with a fresh build,
    /// so the last case should not happen).
    pub fn from_query(query: &str) -> Option<Self> {
        let pattern = match SearchQuery::parse(query) {
            SearchQuery::Literal(text) => SearchPattern::literal(text),
            SearchQuery::Regex(regex) => SearchPattern::regex(regex).ok()?,
        };
        (!pattern.is_empty()).then_some(Self { pattern })
    }

    /// The 0-based source lines of `source` the search finds, in line
    /// order.
    pub fn occurrences(&self, source: &str) -> Vec<usize> {
        self.pattern.matching_lines(source)
    }
}

/// The results list's chrome: no prompt row above it (the prompt was input
/// mode), one footer row below. [`popup_geometry`] sizes the popup from it
/// and the renderer/click mappers take their [`crate::ui::PopupList`] from
/// it, so the height and the chrome cannot disagree.
pub const RESULTS_CHROME: PopupChrome = PopupChrome::FOOTER;

/// Input mode's chrome: the prompt row above and the footer below, with no
/// list rows between them.
const INPUT_CHROME: PopupChrome = PopupChrome::PROMPT_AND_FOOTER;

/// Geometry of the search popup within the terminal `area`: the popup
/// rectangle and how many hit rows it shows. The renderer
/// (`ui::draw_search`, which records the popup rect for the mouse
/// handlers) and the click mapper in `app.rs` both take their list rows
/// from `ui::PopupList`, so what is drawn and what is clickable cannot
/// drift apart.
///
/// Input mode: a fixed 4-row popup (borders + prompt + footer). Results
/// mode: one row per hit plus the footer, clamped to the terminal. The
/// empty results state still needs one row for "no matches".
pub fn popup_geometry(
    area: ratatui::layout::Rect,
    mode: SearchMode,
    hits_len: usize,
) -> ratatui::layout::Rect {
    use ratatui::layout::Rect;
    let width = 70.min(area.width.saturating_sub(2)).max(1);
    let height = match mode {
        SearchMode::Input => INPUT_CHROME.height(0),
        SearchMode::Results => RESULTS_CHROME
            .height(hits_len.max(1) as u16)
            .min(area.height.saturating_sub(2))
            .max(RESULTS_CHROME.height(1)),
    };
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A snippet of `text` around the `[start, end)` byte-range match, laid
/// out as spans with the match highlighted, truncated to `width` display
/// cells (unicode-aware). The result never exceeds `width` cells. When the
/// line does not fit, a window centered on the match is shown with `…`
/// markers on the cut sides; a match wider than the window is itself cut,
/// so the highlight covers its visible part. The match highlight style is
/// themed (`popup_match_fg`/`popup_match_bg`).
pub fn snippet_line(
    text: &str,
    start: usize,
    end: usize,
    width: usize,
    match_style: ratatui::style::Style,
) -> ratatui::text::Line<'static> {
    use ratatui::style::Style;
    use ratatui::text::Span;

    if width == 0 {
        return ratatui::text::Line::default();
    }
    let chars: Vec<char> = text.chars().collect();
    // Char index of the (boundary-adjusted) byte offsets.
    let match_start = text[..start].chars().count();
    let match_end = text[..end].chars().count();

    let widths: Vec<usize> = chars.iter().map(|c| c.width().unwrap_or(0)).collect();
    let total: usize = widths.iter().sum();
    let ellipsis = '…'.width().unwrap_or(1);

    // Full window [from, to) in char indices, plus which sides are cut.
    let (from, to, cut_left, cut_right) = if total <= width {
        (0, chars.len(), false, false)
    } else {
        let budget = width.saturating_sub(2 * ellipsis);
        if budget < ellipsis {
            // Too narrow for content and both cut markers: one marker
            // stands for the elided line.
            return ratatui::text::Line::from(Span::raw("…".to_string()));
        }
        // Expand from the match outward, alternating, until the budget is
        // spent. A character is added only when its cells fit: the window
        // must never exceed the budget, or the selection bar would stop
        // short of the popup's edge.
        let mut left = match_start;
        let mut right = match_end.max(match_start + 1).min(chars.len());
        let mut used = widths[left..right].iter().sum::<usize>();
        // The match alone can be wider than the budget: drop its right
        // edge so the visible part still fits.
        while used > budget && right > left {
            right -= 1;
            used -= widths[right];
        }
        while used < budget {
            let mut grew = false;
            if left > 0 && used + widths[left - 1] <= budget {
                left -= 1;
                used += widths[left];
                grew = true;
            }
            if used < budget && right < chars.len() && used + widths[right] <= budget {
                used += widths[right];
                right += 1;
                grew = true;
            }
            if !grew {
                break;
            }
        }
        (left, right, left > 0, right < chars.len())
    };

    let mut spans: Vec<Span<'static>> = Vec::new();
    if cut_left {
        spans.push(Span::raw("…".to_string()));
    }
    // Accumulate runs (matched vs not) inside the window.
    let mut runs: Vec<(bool, String)> = Vec::new();
    for (i, c) in chars.iter().enumerate().skip(from).take(to - from) {
        let matched = i >= match_start && i < match_end;
        match runs.last_mut() {
            Some((m, text)) if *m == matched => text.push(*c),
            _ => runs.push((matched, c.to_string())),
        }
    }
    for (matched, text) in runs {
        let style = if matched { match_style } else { Style::new() };
        spans.push(Span::styled(text, style));
    }
    if cut_right {
        spans.push(Span::raw("…".to_string()));
    }
    ratatui::text::Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;
    use unicode_width::UnicodeWidthStr;

    /// The themed match highlight used across these tests (ui.rs passes
    /// the theme's `popup_match_fg`/`popup_match_bg`).
    const MATCH: Style = Style::new()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Yellow);

    fn spans_text(line: &ratatui::text::Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The display width of a rendered line, the way the popup lays it out.
    fn rendered_width(line: &ratatui::text::Line<'_>) -> usize {
        line.spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    fn highlighted_text(line: &ratatui::text::Line<'_>) -> String {
        line.spans
            .iter()
            .filter(|s| s.style == MATCH)
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn state_defaults_and_open_resets() {
        let mut s = SearchState::default();
        assert!(!s.open);
        s.query = "old".into();
        s.hits = vec![SearchHit {
            path: "a".into(),
            line: 0,
            text: "t".into(),
            start: 0,
            end: 1,
        }];
        s.selected = 1;
        s.error = Some("old".into());
        s.open_input();
        assert!(s.open);
        assert_eq!(s.mode, SearchMode::Input);
        assert!(s.query.is_empty());
        assert!(s.hits.is_empty());
        assert_eq!(s.selected, 0);
        assert_eq!(s.error, None, "a fresh popup has no stale error");
    }

    #[test]
    fn run_ignores_blank_query_and_clamps_selection() {
        let mut s = SearchState::default();
        s.open_input();
        s.query = "   ".into();
        s.run(|_| panic!("blank query must not run"));
        assert_eq!(s.mode, SearchMode::Input);

        s.query = "needle".into();
        s.run(|q| {
            assert_eq!(q, SearchQuery::Literal("needle"));
            Ok(SearchResults {
                hits: (0..5)
                    .map(|i| SearchHit {
                        path: format!("f{i}").into(),
                        line: 0,
                        text: "x".into(),
                        start: 0,
                        end: 1,
                    })
                    .collect(),
                capped: false,
            })
        });
        assert_eq!(s.mode, SearchMode::Results);
        assert_eq!(s.hits.len(), 5);
        s.move_selection(100);
        assert_eq!(s.selected, 4, "clamped to the last hit");
        s.move_selection(-100);
        assert_eq!(s.selected, 0);
        s.move_selection(-3); // stays at 0, no panic
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn run_records_and_resets_the_capped_flag() {
        let mut s = SearchState::default();
        s.open_input();
        s.query = "x".into();
        s.run(|_| {
            Ok(SearchResults {
                hits: Vec::new(),
                capped: true,
            })
        });
        assert!(s.capped, "capping is propagated from the index");
        s.open_input();
        assert!(!s.capped, "a fresh popup starts uncapped");
    }

    #[test]
    fn the_regex_prefix_selects_the_regex_path_and_is_stripped() {
        assert_eq!(SearchQuery::parse("needle"), SearchQuery::Literal("needle"));
        assert_eq!(SearchQuery::parse("re:\\d+"), SearchQuery::Regex("\\d+"));
        assert_eq!(
            SearchQuery::parse("RE:upper"),
            SearchQuery::Literal("RE:upper"),
            "the prefix is lowercase, like the documented query"
        );
    }

    #[test]
    fn invalid_regex_reports_the_error_and_keeps_the_input_mode() {
        let mut s = SearchState::default();
        s.open_input();
        s.query = "re:(".into();
        s.run(|query| {
            assert_eq!(query, SearchQuery::Regex("("));
            Err("invalid regex: unclosed group".to_string())
        });
        assert_eq!(s.mode, SearchMode::Input, "nothing was searched");
        assert_eq!(s.query, "re:(", "the query stays intact");
        assert_eq!(
            s.error.as_deref(),
            Some("invalid regex: unclosed group"),
            "the footer gets the reason"
        );
        assert!(s.hits.is_empty());

        // Editing drops the stale error; a successful run clears it too.
        s.push_char(')');
        assert_eq!(s.error, None);
        s.run(|_| Ok(SearchResults::default()));
        assert_eq!(s.mode, SearchMode::Results);
        assert_eq!(s.error, None);
    }

    #[test]
    fn active_search_keeps_only_queries_that_can_match() {
        assert!(ActiveSearch::from_query("").is_none());
        assert!(
            ActiveSearch::from_query("re:").is_none(),
            "a bare re: searches nothing"
        );
        assert!(
            ActiveSearch::from_query("re:(").is_none(),
            "a pattern that does not compile never ran"
        );

        let literal = ActiveSearch::from_query("needle").unwrap();
        assert_eq!(literal.occurrences("a\nNeedle\nb\nneedle\n"), vec![1, 3]);
        let regex = ActiveSearch::from_query("re:nee\\w+").unwrap();
        assert_eq!(regex.occurrences("a\nNeedle\nb\n"), vec![1]);
    }

    #[test]
    fn snippet_highlights_the_match() {
        let line = snippet_line("hello world", 6, 11, 40, MATCH);
        assert_eq!(spans_text(&line), "hello world");
        assert_eq!(highlighted_text(&line), "world");
    }

    #[test]
    fn snippet_truncates_around_the_match() {
        let text = "a".repeat(30) + "NEEDLE" + &"b".repeat(30);
        let start = 30;
        let end = 36;
        let line = snippet_line(&text, start, end, 20, MATCH);
        let rendered = spans_text(&line);
        assert!(rendered.starts_with('…'), "left side cut: {rendered}");
        assert!(rendered.ends_with('…'), "right side cut");
        assert_eq!(highlighted_text(&line), "NEEDLE");
        assert!(rendered.chars().count() <= 20 + 2);
    }

    #[test]
    fn snippet_handles_wide_and_zero_width() {
        // CJK match text: widths counted in cells, not chars. Each CJK
        // char is 3 bytes, so bytes 3..9 are chars 1..3 = "んに".
        let line = snippet_line("こんにちは世界", 3, 9, 40, MATCH);
        assert_eq!(highlighted_text(&line), "んに");
        assert_eq!(
            snippet_line("abc", 1, 2, 0, MATCH).spans.len(),
            0,
            "no width: nothing"
        );
    }

    /// The window adds a character only when its cells fit. A two-cell
    /// character added at the budget's edge used to overshoot by one cell,
    /// and the popup's selection bar then stopped short of its inner right
    /// edge.
    #[test]
    fn snippet_never_exceeds_its_width_with_wide_characters() {
        let texts = [
            format!("{}needle{}", "漢".repeat(50), "漢".repeat(50)),
            format!("{}needle{}", "x".repeat(40), "漢".repeat(40)),
            format!("{}needle{}", "漢".repeat(40), "x".repeat(40)),
            "漢".repeat(60),
            "abc漢def".repeat(12),
        ];
        for text in texts {
            let (start, end) = match text.find("needle") {
                Some(start) => (start, start + "needle".len()),
                None => (0, text.len()),
            };
            for width in 1..=60 {
                let line = snippet_line(&text, start, end, width, MATCH);
                assert!(
                    rendered_width(&line) <= width,
                    "width {width} overshot with {text:?}: {:?}",
                    spans_text(&line)
                );
            }
        }
    }

    /// A match wider than the window is cut to the window's budget; the
    /// visible part stays highlighted and the cut side gets its marker.
    #[test]
    fn snippet_cuts_a_match_wider_than_the_window() {
        let text = "漢".repeat(30);
        let line = snippet_line(&text, 0, text.len(), 10, MATCH);
        assert_eq!(rendered_width(&line), 9, "8 cells of match plus the marker");
        assert_eq!(highlighted_text(&line), "漢漢漢漢");
        assert!(spans_text(&line).ends_with('…'));
    }

    /// The regression's shape at the popup's real snippet width: a wide
    /// character at the budget's edge, with both sides cut.
    #[test]
    fn snippet_fits_the_budget_with_a_wide_character_at_its_edge() {
        let text = format!("{}needle{}", "漢".repeat(50), "漢".repeat(50));
        let start = "漢".repeat(50).len();
        let line = snippet_line(&text, start, start + 6, 58, MATCH);
        assert!(rendered_width(&line) <= 58);
        assert_eq!(highlighted_text(&line), "needle");
        let rendered = spans_text(&line);
        assert!(rendered.starts_with('…'), "{rendered}");
        assert!(rendered.ends_with('…'), "{rendered}");
    }

    #[test]
    fn snippet_match_at_line_edges() {
        let line = snippet_line("needle tail", 0, 6, 40, MATCH);
        assert_eq!(highlighted_text(&line), "needle");
        let line = snippet_line("head needle", 5, 11, 40, MATCH);
        assert_eq!(highlighted_text(&line), "needle");
    }

    #[test]
    fn popup_height_is_the_chrome_plus_the_visible_rows() {
        let area = ratatui::layout::Rect::new(0, 0, 100, 40);
        let input = popup_geometry(area, SearchMode::Input, 0);
        assert_eq!(
            input.height,
            INPUT_CHROME.height(0),
            "input mode is prompt + footer, no list rows"
        );
        for hits in [0, 1, 5, 30] {
            let popup = popup_geometry(area, SearchMode::Results, hits);
            let rows = hits.max(1) as u16; // the empty state says "no matches"
            assert_eq!(
                popup.height,
                RESULTS_CHROME.height(rows),
                "the height derives from the one results chrome ({hits} hits)"
            );
            assert_eq!(
                RESULTS_CHROME.list(popup, 0, hits).rows,
                rows as usize,
                "the list window shows exactly the rows the height made room for"
            );
        }
    }

    /// A terminal too short for the results popup still gets the empty
    /// state's one list row: the floor is the chrome's height, not a bare
    /// number left behind by a chrome change.
    #[test]
    fn results_height_floor_is_the_chrome() {
        let tiny = ratatui::layout::Rect::new(0, 0, 100, 5);
        let popup = popup_geometry(tiny, SearchMode::Results, 0);
        assert_eq!(popup.height, RESULTS_CHROME.height(1));
        assert_eq!(RESULTS_CHROME.list(popup, 0, 0).rows, 1);
    }
}
