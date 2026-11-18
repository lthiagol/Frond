//! Quick note switcher (`Ctrl+P`): a modal popup that fuzzy-matches the
//! typed query against the vault's notes and opens the selected one.
//!
//! Flow: `Ctrl+P` opens the popup with an empty query (every note is
//! listed); typing filters the list live (no Enter needed — the matches
//! update on every keystroke); `↑`/`↓` move the selection, `Enter` opens
//! the selected note through the normal navigation stack (so `Backspace`
//! returns to the previous note), `Esc` closes the popup.
//!
//! Matching is a case-insensitive *subsequence* match (like every other
//! fuzzy finder): the query's characters must appear in the note's path in
//! order, but not necessarily next to each other. Two haystacks are tried
//! per note — the whole vault-relative path and the bare file name — and
//! the better one wins; a match inside the file name gets a bonus, so
//! typing `alpha` surfaces `alpha.md` before `archive/alpha-notes.md`.
//!
//! The popup is terminal-independent state; rendering lives in `ui.rs`
//! ([`crate::ui::draw_quick`]).

use std::path::PathBuf;

use vault::NoteMeta;

/// Score of one matched character.
const SCORE_MATCH: i32 = 16;
/// The matched character had the same case as the query.
const SCORE_CASE: i32 = 2;
/// The match sits on a word boundary (path separator, dash, underscore,
/// dot, space) or a camelCase hump.
const SCORE_BOUNDARY: i32 = 8;
/// The match continues the previous one (consecutive characters).
const SCORE_CONSECUTIVE: i32 = 10;
/// The query appears as a contiguous substring of the haystack.
const SCORE_SUBSTRING: i32 = 60;
/// The haystack starts with the query.
const SCORE_PREFIX: i32 = 40;
/// Penalty per character skipped between two matches (capped).
const SCORE_GAP: i32 = 3;
/// Penalty for a first match that starts late in the haystack (capped).
const SCORE_LATE_START: i32 = 4;
/// Bonus for matching inside the file name rather than the whole path.
const SCORE_STEM: i32 = 100;

/// How many result rows the popup shows at once.
pub const VISIBLE_ROWS: usize = 10;

/// Geometry of the quick-switcher popup within the terminal `area`.
///
/// The popup is: a border, the query prompt, up to [`VISIBLE_ROWS`]
/// result rows and a footer line. Shared by the renderer (`ui::draw_quick`,
/// which records the popup rect for the mouse handlers) and by the
/// click/wheel mapping in `app.rs`, so the two cannot drift.
pub fn popup_geometry(area: ratatui::layout::Rect, item_count: usize) -> ratatui::layout::Rect {
    use ratatui::layout::Rect;
    let width = 70.min(area.width.saturating_sub(2)).max(1);
    // borders (2) + prompt (1) + footer (1) + the visible rows.
    let rows = item_count.min(VISIBLE_ROWS) as u16;
    let height = (rows + 4).min(area.height.saturating_sub(2)).max(4);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A note the switcher can jump to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickCandidate {
    /// Absolute path (opened through the normal nav stack).
    pub path: PathBuf,
    /// Vault-relative path as displayed (`projects/alpha.md`).
    pub label: String,
    /// The path without its extension — the matching haystack. Always a
    /// prefix of `label`, so the match positions carry over to it.
    pub target: String,
    /// The file name without extension — matched with a bonus. Always a
    /// suffix of `target`.
    pub stem: String,
}

impl QuickCandidate {
    /// Builds a candidate from a path and its vault-relative label.
    ///
    /// `ext` is the extension to strip (`.md`, `.base`); the label keeps
    /// it (it is what the user sees).
    pub fn new(path: PathBuf, label: String, ext: &str) -> Self {
        let target = label
            .strip_suffix(ext)
            .map(str::to_string)
            .unwrap_or_else(|| label.clone());
        let stem = target.rsplit('/').next().unwrap_or(&target).to_string();
        Self {
            path,
            label,
            target,
            stem,
        }
    }

    /// Builds a candidate from an indexed note.
    pub fn from_note(note: &NoteMeta) -> Self {
        Self::new(note.path.clone(), note.rel_path.clone(), ".md")
    }

    /// How many characters of `target` precede the file name.
    fn stem_offset(&self) -> usize {
        self.target
            .chars()
            .count()
            .saturating_sub(self.stem.chars().count())
    }
}

/// One fuzzy match: the score and the matched character positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    /// Character indices into the haystack the match was computed on.
    pub positions: Vec<usize>,
}

/// A ranked result row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickItem {
    pub path: PathBuf,
    pub label: String,
    /// Matched character indices into `label` (for the highlight).
    pub positions: Vec<usize>,
    pub score: i32,
}

/// The popup's state.
#[derive(Clone, Debug, Default)]
pub struct QuickState {
    pub open: bool,
    pub query: String,
    /// Every note in the vault (captured when the popup opens).
    pub candidates: Vec<QuickCandidate>,
    /// The current query's matches, best first.
    pub items: Vec<QuickItem>,
    pub selected: usize,
}

impl QuickState {
    /// `Ctrl+P`: opens the popup over the given candidates.
    pub fn open(&mut self, candidates: Vec<QuickCandidate>) {
        self.open = true;
        self.query.clear();
        self.candidates = candidates;
        self.refresh();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Appends a typed character and re-filters.
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refresh();
    }

    /// Deletes the last character and re-filters.
    pub fn backspace(&mut self) {
        self.query.pop();
        self.refresh();
    }

    /// Recomputes the result list for the current query and resets the
    /// selection to the best match.
    pub fn refresh(&mut self) {
        let query = self.query.as_str();
        let mut items: Vec<QuickItem> = self
            .candidates
            .iter()
            .filter_map(|cand| {
                let m = score_candidate(query, cand)?;
                Some(QuickItem {
                    path: cand.path.clone(),
                    label: cand.label.clone(),
                    positions: m.positions,
                    score: m.score,
                })
            })
            .collect();
        // Best score first; equal scores keep a stable, alphabetical order.
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));
        self.items = items;
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.items.len();
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn selected_item(&self) -> Option<&QuickItem> {
        self.items.get(self.selected)
    }
}

/// Scores a candidate against the query. `None` = no match at all.
///
/// The returned positions are indices into the candidate's *label* (what
/// the UI highlights), whichever haystack won.
pub fn score_candidate(query: &str, cand: &QuickCandidate) -> Option<Match> {
    if query.is_empty() {
        // No query: every note matches, ranked alphabetically by the caller.
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    // The whole path (without the `.md` suffix).
    let full = fuzzy_match(query, &cand.target).map(|m| Match {
        score: m.score,
        // `target` is a prefix of `label`, so the positions carry over.
        positions: m.positions,
    });
    // The bare file name, with its own bonus.
    let stem = fuzzy_match(query, &cand.stem).map(|m| {
        let offset = cand.stem_offset();
        Match {
            score: m.score + SCORE_STEM,
            positions: m.positions.into_iter().map(|p| p + offset).collect(),
        }
    });
    match (full, stem) {
        (None, None) => None,
        (Some(f), Some(s)) => Some(if s.score > f.score { s } else { f }),
        (Some(f), None) => Some(f),
        (None, Some(s)) => Some(s),
    }
}

/// Case-insensitive subsequence match of `query` in `haystack`.
///
/// The score rewards consecutive characters, word boundaries, a contiguous
/// substring match and an early start; `None` means the query does not
/// appear as a subsequence.
pub fn fuzzy_match(query: &str, haystack: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let hay: Vec<char> = haystack.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    let mut positions: Vec<usize> = Vec::with_capacity(needle.len());
    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut previous: Option<usize> = None;

    for &qc in &needle {
        let qc_lower = lower(qc);
        let found = hay[cursor..]
            .iter()
            .position(|c| lower(*c) == qc_lower)
            .map(|off| cursor + off)?;
        positions.push(found);
        score += SCORE_MATCH;
        if hay[found] == qc {
            score += SCORE_CASE;
        }
        if is_boundary(&hay, found) {
            score += SCORE_BOUNDARY;
        }
        match previous {
            Some(prev) if found == prev + 1 => score += SCORE_CONSECUTIVE,
            Some(prev) => {
                let gap = (found - prev - 1) as i32;
                score -= SCORE_GAP.min(gap);
            }
            None => score -= SCORE_LATE_START.min(found as i32),
        }
        previous = Some(found);
        cursor = found + 1;
    }

    // Whole-query bonuses: a contiguous substring beats a scattered
    // subsequence, and a prefix beats both.
    let hay_lower: String = hay.iter().map(|c| lower(*c)).collect();
    let query_lower: String = needle.iter().map(|c| lower(*c)).collect();
    if hay_lower.contains(&query_lower) {
        score += SCORE_SUBSTRING;
    }
    if hay_lower.starts_with(&query_lower) {
        score += SCORE_PREFIX;
    }

    Some(Match { score, positions })
}

/// The lowercase form of a character, keeping one char (so the matcher's
/// positions stay aligned with the original text).
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Is this position a word boundary (path separator, dash, underscore,
/// dot, space) or a camelCase hump?
fn is_boundary(hay: &[char], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = hay[pos - 1];
    if matches!(prev, '/' | '\\' | '-' | '_' | '.' | ' ') {
        return true;
    }
    // camelCase hump: lowercase/digit followed by an uppercase letter.
    prev.is_lowercase() && hay[pos].is_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(path: &str, label: &str) -> QuickCandidate {
        let stem = label.rsplit('/').next().unwrap_or(label);
        let stem = stem.strip_suffix(".md").unwrap_or(stem);
        let target = label.strip_suffix(".md").unwrap_or(label).to_string();
        QuickCandidate {
            path: PathBuf::from(path),
            label: label.to_string(),
            target,
            stem: stem.to_string(),
        }
    }

    #[test]
    fn match_requires_the_query_as_a_subsequence() {
        assert!(fuzzy_match("abc", "a-b-c").is_some());
        assert!(fuzzy_match("acb", "abc").is_none());
        assert!(fuzzy_match("zzz", "abc").is_none());
    }

    #[test]
    fn empty_query_matches_everything() {
        let m = fuzzy_match("", "whatever").expect("empty query matches");
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
    }

    #[test]
    fn contiguous_matches_score_higher_than_scattered_ones() {
        let tight = fuzzy_match("ab", "xxabxx").expect("match");
        let loose = fuzzy_match("ab", "axxbxx").expect("match");
        assert!(
            tight.score > loose.score,
            "{} vs {}",
            tight.score,
            loose.score
        );
    }

    #[test]
    fn a_prefix_beats_a_late_match() {
        let prefix = fuzzy_match("ab", "abc").expect("match");
        let late = fuzzy_match("ab", "zzab").expect("match");
        assert!(
            prefix.score > late.score,
            "{} vs {}",
            prefix.score,
            late.score
        );
    }

    #[test]
    fn positions_point_at_the_matched_characters() {
        let m = fuzzy_match("ac", "abc").expect("match");
        assert_eq!(m.positions, vec![0, 2]);
        let hay: Vec<char> = "abc".chars().collect();
        let matched: String = m.positions.iter().map(|&p| hay[p]).collect();
        assert_eq!(matched, "ac");
    }

    #[test]
    fn matching_is_case_insensitive_but_case_earns_a_bonus() {
        let exact = fuzzy_match("Alpha", "Alpha").expect("match");
        let folded = fuzzy_match("alpha", "Alpha").expect("match");
        assert!(
            exact.score > folded.score,
            "{} vs {}",
            exact.score,
            folded.score
        );
    }

    #[test]
    fn the_file_name_wins_over_the_folder_path() {
        let cand = candidate("/v/archive/alpha-notes.md", "archive/alpha-notes.md");
        let m = score_candidate("alpha", &cand).expect("match");
        // The match is inside the file name (`alpha-notes`), not in the
        // folder, so the stem bonus applies.
        assert!(m.score >= SCORE_STEM);
        let label: Vec<char> = cand.label.chars().collect();
        let matched: String = m.positions.iter().map(|&p| label[p]).collect();
        assert_eq!(matched, "alpha");
    }

    #[test]
    fn positions_are_label_indices_for_both_haystacks() {
        let cand = candidate("/v/projects/alpha.md", "projects/alpha.md");
        let m = score_candidate("alpha", &cand).expect("match");
        let label: Vec<char> = cand.label.chars().collect();
        assert_eq!(m.positions, vec![9, 10, 11, 12, 13]);
        let matched: String = m.positions.iter().map(|&p| label[p]).collect();
        assert_eq!(matched, "alpha");
    }

    #[test]
    fn a_non_matching_query_filters_the_candidate_out() {
        let cand = candidate("/v/alpha.md", "alpha.md");
        assert!(score_candidate("zzz", &cand).is_none());
    }

    #[test]
    fn state_filters_and_ranks_the_candidates() {
        let mut state = QuickState::default();
        state.open(vec![
            candidate("/v/alpha.md", "alpha.md"),
            candidate("/v/archive/beta.md", "archive/beta.md"),
            candidate("/v/gamma.md", "gamma.md"),
        ]);
        assert_eq!(state.items.len(), 3, "no query: every note is listed");
        assert_eq!(state.selected, 0);

        state.push_char('a');
        // A single `a` matches every note (a subsequence match).
        assert_eq!(state.items.len(), 3);
        state.push_char('l');
        // `al` only matches `alpha.md`: neither `archive/beta` nor `gamma`
        // contains an `l`.
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].label, "alpha.md");

        state.backspace();
        assert_eq!(state.items.len(), 3);
        assert_eq!(state.selected, 0, "the selection resets on every filter");
    }

    #[test]
    fn selection_is_clamped_to_the_item_list() {
        let mut state = QuickState::default();
        state.open(vec![
            candidate("/v/a.md", "a.md"),
            candidate("/v/b.md", "b.md"),
        ]);
        state.move_selection(10);
        assert_eq!(state.selected, 1);
        state.move_selection(-10);
        assert_eq!(state.selected, 0);
        state.items.clear();
        state.move_selection(3);
        assert_eq!(state.selected, 0, "an empty list has no selection");
    }
}
