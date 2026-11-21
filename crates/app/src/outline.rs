//! Outline view: the sidebar's heading-list mode (`o`).
//!
//! The rows are the open note's heading table (`viewer.toc`), in document
//! order; the indentation is relative to the shallowest heading level in
//! that note. This module owns the two cursors — the reader's selection and
//! the active heading the note's scroll moves — and the one scroll window
//! they share, which has three owners: the note's scroll, the reader's
//! cursor, and the reader's wheel (the selection's shape is
//! `tags::TagList`'s). The rows themselves stay in the viewer so the
//! outline always follows the note on screen. Rendering lives in `ui.rs`.

use engine::TocEntry;

/// One display row: a heading plus its indentation depth (the shallowest
/// level is depth 0).
pub struct Row<'a> {
    pub entry: &'a TocEntry,
    pub depth: usize,
    /// The row's place among the note's depth-0 headings, 1-based; `None`
    /// for a deeper row. The panel draws it as the two-digit index leaf's
    /// table of contents carries, and draws a bullet instead when absent.
    pub index: Option<usize>,
}

/// The outline's rows for a note: document order, indented by the heading
/// level relative to the shallowest level present (level 1, 4 → depths
/// 0, 3), the depth-0 rows indexed in that same order. An empty table has
/// no rows.
pub fn rows(toc: &[TocEntry]) -> Vec<Row<'_>> {
    let shallowest = toc.iter().map(|entry| entry.level).min();
    let mut top_level = 0;
    toc.iter()
        .map(|entry| {
            let depth =
                shallowest.map_or(0, |level| usize::from(entry.level.saturating_sub(level)));
            let index = (depth == 0).then(|| {
                top_level += 1;
                top_level
            });
            Row {
                entry,
                depth,
                index,
            }
        })
        .collect()
}

/// The row the reader is looking at: the last heading at or above the top
/// of the note's viewport (`entry.line <= scroll`). At the note's end
/// (`at_end`) it is the last heading — the final lines cannot scroll up to
/// meet a heading near the bottom; above the first heading it is the first
/// one. `None` for a headingless note.
///
/// `TocEntry::line` is the heading's rendered line, the same space as the
/// viewer's scroll, so the two compare directly.
pub fn active_row(entries: &[TocEntry], scroll: u32, at_end: bool) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    if at_end {
        return Some(entries.len() - 1);
    }
    if scroll < entries[0].line as u32 {
        return Some(0);
    }
    entries
        .iter()
        .rposition(|entry| entry.line as u32 <= scroll)
}

/// Who positions the outline's window: the note's scroll (the panel follows
/// the active heading), the reader's cursor (`j`/`k`, a click, the panel's
/// bar), or the reader's wheel (the view stays where they put it, leaf's
/// `TocScrollMode::Manual`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowOwner {
    Note,
    Cursor,
    Wheel,
}

/// The outline's two cursors over the open note's heading rows.
pub struct Outline {
    /// Index of the selected row: the reader's cursor (`j`/`k`, a click,
    /// the panel's bar) and the thing `Enter` jumps to. A note scroll takes
    /// it to the active row; until then it stays where the reader put it.
    pub selected: usize,
    /// Index of the active row: the heading the reader is looking at, which
    /// the note's scroll moves. It is marked, and a note scroll selects it.
    pub active: usize,
    /// Viewport height fed back from `ui::draw` (inner height in rows).
    pub viewport: usize,
    /// First visible row index.
    pub scroll: usize,
    /// Heading a jump pinned as active, until the reader scrolls the note.
    /// The pin is the heading's title plus its occurrence among the rows
    /// carrying that title, so a reparse can name a different same-titled
    /// heading — no textual key can tell those apart. The rule that keeps
    /// or drops the pin lives in [`Self::pinned_row`], and the release
    /// belongs to [`Self::note_scrolled`].
    pin: Option<(String, usize)>,
    /// Who positions the window. The note's scroll hands it to
    /// [`WindowOwner::Note`], the reader's own cursor moves, a click and the
    /// panel's bar to [`WindowOwner::Cursor`], and the wheel to
    /// [`WindowOwner::Wheel`]; a note scroll hands it back to the note.
    owner: WindowOwner,
    /// A note scroll has happened and the selection must take the active
    /// row on the next [`Self::follow_note`]: the bar marks the heading the
    /// reader is looking at. A reader move or a note change cancels it, so a
    /// redraw before the next scroll cannot pull a fresh cursor away.
    take_selection: bool,
}

impl Default for Outline {
    fn default() -> Self {
        Self::new()
    }
}

impl Outline {
    pub fn new() -> Self {
        Self {
            selected: 0,
            active: 0,
            viewport: 20,
            scroll: 0,
            pin: None,
            owner: WindowOwner::Note,
            take_selection: false,
        }
    }

    /// Re-clamps the cursors to the current row count: the open note or its
    /// headings changed, so both indices must stay valid. The selection is
    /// clamped to the last row. A pinned heading is resolved by title and
    /// occurrence on the next [`Self::follow_note`]. The window is
    /// positioned by that call (from the drawer), by the selection's own
    /// moves, or by the wheel.
    pub fn sync(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            self.active = 0;
            self.scroll = 0;
            self.pin = None;
            return;
        }
        self.selected = self.selected.min(len - 1);
        self.active = self.active.min(len - 1);
        self.clamp_scroll(len);
    }

    /// The open note changed: both cursors start at the new note's first
    /// heading. The selection's index means nothing across notes — row 7 of
    /// the new note is a different heading — so it does not survive; the
    /// pin belonged to the old note and is dropped, and the window follows
    /// the note again.
    pub fn on_note_change(&mut self, len: usize) {
        self.pin = None;
        self.owner = WindowOwner::Note;
        self.take_selection = false;
        self.selected = 0;
        self.active = 0;
        self.scroll = 0;
        self.sync(len);
    }

    /// The reader moved the note themselves (a scroll, a search jump, a
    /// link's heading): the outline follows the note again, a jump's pin is
    /// released, and the selection goes with the scroll — the bar marks the
    /// heading being read from the next frame on. This also takes the
    /// window back from the wheel, so the panel follows the note again.
    pub fn note_scrolled(&mut self) {
        self.pin = None;
        self.owner = WindowOwner::Note;
        self.take_selection = true;
    }

    /// The reader's wheel: scrolls the window itself by `delta` rows and
    /// hands it to [`WindowOwner::Wheel`]. The selection does not move —
    /// leaf's table of contents leaves it where it was, even when the view
    /// scrolls it out of sight — and the panel stops following the note
    /// until the reader scrolls the note themselves
    /// ([`Self::note_scrolled`]).
    pub fn wheel(&mut self, delta: isize, len: usize) {
        self.owner = WindowOwner::Wheel;
        self.take_selection = false;
        self.scroll = (self.scroll as isize + delta).max(0) as usize;
        self.clamp_scroll(len);
    }

    /// A jump to row `index` of `entries` pinned it as the active row: the
    /// computed row is overridden until the reader scrolls the note
    /// ([`Self::note_scrolled`]). The pin holds the heading's title and
    /// which occurrence of that title it is, so a reparse resolves the same
    /// heading rather than the first row carrying the title. A jump is a
    /// request to see the heading, so the window follows the pinned mark —
    /// including after the wheel has scrolled it out of sight.
    pub fn pin_active(&mut self, entries: &[TocEntry], index: usize) {
        let Some(entry) = entries.get(index) else {
            return;
        };
        self.owner = WindowOwner::Note;
        let occurrence = entries[..index]
            .iter()
            .filter(|other| other.title == entry.title)
            .count();
        self.pin = Some((entry.title.clone(), occurrence));
        self.active = index;
    }

    /// Recomputes the active row from the note's scroll position (a pin
    /// wins), takes the selection with a pending note scroll, and, when
    /// the note owns the window, scrolls it into view. Called from the
    /// drawer on every frame: the note's scroll and the viewport are known
    /// there, and the frame is what the reader sees.
    pub fn follow_note(&mut self, entries: &[TocEntry], scroll: u32, at_end: bool) {
        self.sync(entries.len());
        let Some(active) = self
            .pinned_row(entries)
            .or_else(|| active_row(entries, scroll, at_end))
        else {
            self.active = 0;
            return;
        };
        self.active = active;
        if self.take_selection {
            self.selected = self.active;
            self.take_selection = false;
        }
        if self.owner == WindowOwner::Note {
            self.scroll_row_into_view(self.active, entries.len());
        }
    }

    /// The pinned heading's row, matched by title and occurrence in
    /// document order: the correct one of two identically-titled headings.
    /// A title or occurrence no heading carries drops the pin and yields
    /// `None`, so the note's scroll names the active row again.
    fn pinned_row(&mut self, entries: &[TocEntry]) -> Option<usize> {
        let (title, occurrence) = self.pin.as_ref()?;
        let row = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.title == *title)
            .nth(*occurrence)
            .map(|(row, _)| row);
        if row.is_none() {
            self.pin = None;
        }
        row
    }

    /// Selects a row (a click) and keeps it visible. Out-of-range indices
    /// are ignored.
    pub fn select(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        self.selected = index;
        self.owner = WindowOwner::Cursor;
        self.take_selection = false;
        self.scroll_selection_into_view(len);
    }

    pub fn move_down(&mut self, n: usize, len: usize) {
        self.move_by(n as isize, len);
    }

    pub fn move_up(&mut self, n: usize, len: usize) {
        self.move_by(-(n as isize), len);
    }

    fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.owner = WindowOwner::Cursor;
        self.take_selection = false;
        self.scroll_selection_into_view(len);
    }

    /// Called from `ui::draw` with the sidebar's inner height: the new
    /// viewport may move the window, which follows whichever cursor owns
    /// it. A window the wheel owns stays where it is, clamped to the list.
    pub fn on_viewport_height(&mut self, height: usize, len: usize) {
        self.viewport = height.max(1);
        match self.owner {
            WindowOwner::Note => self.scroll_row_into_view(self.active, len),
            WindowOwner::Cursor => self.scroll_row_into_view(self.selected, len),
            WindowOwner::Wheel => self.clamp_scroll(len),
        }
    }

    fn scroll_selection_into_view(&mut self, len: usize) {
        self.scroll_row_into_view(self.selected, len);
    }

    fn scroll_row_into_view(&mut self, row: usize, len: usize) {
        if row >= self.scroll + self.viewport {
            self.scroll = row + 1 - self.viewport;
        } else if row < self.scroll {
            self.scroll = row;
        }
        self.clamp_scroll(len);
    }

    fn clamp_scroll(&mut self, len: usize) {
        self.scroll = self.scroll.min(len.saturating_sub(self.viewport.min(len)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toc(levels: &[u8]) -> Vec<TocEntry> {
        levels
            .iter()
            .enumerate()
            .map(|(i, &level)| TocEntry {
                level,
                title: format!("h{i}"),
                line: i,
            })
            .collect()
    }

    /// Headings `n` rendered lines apart, so the scroll and the rows line
    /// up the way a note with body text spaces them.
    fn spaced(lines: &[usize]) -> Vec<TocEntry> {
        lines
            .iter()
            .enumerate()
            .map(|(i, &line)| TocEntry {
                level: 1,
                title: format!("h{i}"),
                line,
            })
            .collect()
    }

    /// One row per title, all level 1: a reparse builds a new table from a
    /// changed note, and the titles are what a pin follows.
    fn titled(titles: &[&str]) -> Vec<TocEntry> {
        titles
            .iter()
            .enumerate()
            .map(|(i, &title)| TocEntry {
                level: 1,
                title: title.to_string(),
                line: i,
            })
            .collect()
    }

    #[test]
    fn rows_are_indented_relative_to_the_shallowest_level() {
        let entries = toc(&[2, 3, 4, 2]);
        let rows = rows(&entries);
        let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 0], "level 2 is flush left");
        assert_eq!(rows[1].entry.title, "h1", "document order is kept");
    }

    #[test]
    fn rows_number_the_depth_zero_headings_in_document_order() {
        let entries = toc(&[1, 2, 2, 1, 3]);
        let rows = rows(&entries);
        let indices: Vec<Option<usize>> = rows.iter().map(|row| row.index).collect();
        assert_eq!(
            indices,
            vec![Some(1), None, None, Some(2), None],
            "the flush-left headings carry the count; the deeper rows a bullet"
        );
    }

    #[test]
    fn rows_number_the_shallowest_level_when_the_note_has_no_level_1() {
        let entries = toc(&[3, 4, 3]);
        let rows = rows(&entries);
        let indices: Vec<Option<usize>> = rows.iter().map(|row| row.index).collect();
        assert_eq!(
            indices,
            vec![Some(1), None, Some(2)],
            "a note whose top level is 3 numbers its own top level"
        );
    }

    #[test]
    fn rows_of_an_empty_toc_are_empty() {
        assert!(rows(&[]).is_empty());
    }

    #[test]
    fn active_row_is_the_last_heading_at_or_above_the_top() {
        let entries = spaced(&[0, 10, 20]);
        assert_eq!(active_row(&entries, 0, false), Some(0));
        assert_eq!(active_row(&entries, 5, false), Some(0), "between headings");
        assert_eq!(active_row(&entries, 10, false), Some(1), "on a heading");
        assert_eq!(active_row(&entries, 19, false), Some(1));
        assert_eq!(active_row(&entries, 25, false), Some(2));
    }

    #[test]
    fn active_row_above_the_first_heading_is_the_first_one() {
        let entries = spaced(&[5, 15]);
        assert_eq!(
            active_row(&entries, 0, false),
            Some(0),
            "the note's title is above the first heading"
        );
    }

    #[test]
    fn active_row_at_the_end_is_the_last_heading() {
        // The last heading is the note's final line, below the deepest
        // scroll the viewport allows: without the end rule the top of the
        // viewport would name an earlier heading.
        let entries = spaced(&[0, 50]);
        assert_eq!(active_row(&entries, 40, false), Some(0));
        assert_eq!(
            active_row(&entries, 40, true),
            Some(1),
            "the end of the note is the last heading"
        );
    }

    #[test]
    fn active_row_of_a_headingless_note_is_none() {
        assert_eq!(active_row(&[], 0, false), None);
        assert_eq!(active_row(&[], 0, true), None);
    }

    #[test]
    fn follow_note_scrolls_the_active_row_into_view_and_takes_the_selection() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);

        outline.note_scrolled();
        outline.follow_note(&entries, 70, false);
        assert_eq!(outline.active, 7, "the heading at or above the top");
        assert_eq!(outline.scroll, 5, "the active row is the last visible one");
        assert_eq!(
            outline.selected, 7,
            "the bar marks the heading the reader is looking at"
        );

        outline.note_scrolled();
        outline.follow_note(&entries, 0, false);
        assert_eq!(outline.active, 0);
        assert_eq!(outline.selected, 0, "scrolling up brings the bar back");
        assert_eq!(outline.scroll, 0, "scrolling up brings the heading back");

        outline.note_scrolled();
        outline.follow_note(&entries, 0, true);
        assert_eq!(outline.active, 11, "the end of the note is its last row");
        assert_eq!(outline.selected, 11);
        assert_eq!(outline.scroll, 9);
    }

    #[test]
    fn a_jump_pin_holds_until_the_reader_scrolls_the_note() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);

        outline.select(11, entries.len());
        outline.pin_active(&entries, 11);
        outline.follow_note(&entries, 20, false);
        assert_eq!(
            outline.active, 11,
            "the pinned heading stays active under the note's scroll"
        );
        assert_eq!(outline.selected, 11, "and it stays the selection");

        outline.note_scrolled();
        outline.follow_note(&entries, 20, false);
        assert_eq!(outline.active, 2, "the reader's scroll releases the pin");
        assert_eq!(
            outline.selected, 2,
            "and the scroll takes the selection with it"
        );
    }

    #[test]
    fn a_selection_move_keeps_the_window_and_a_note_scroll_takes_it_back() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);

        outline.note_scrolled();
        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.active, 9);
        outline.move_up(100, entries.len());
        assert_eq!(outline.selected, 0, "clamped at the first row");
        assert_eq!(outline.scroll, 0, "the window follows the reader's move");

        outline.follow_note(&entries, 90, false);
        assert_eq!(
            outline.scroll, 0,
            "a redraw alone does not drag the window to the active row"
        );
        assert_eq!(
            outline.selected, 0,
            "nor does a redraw move the reader's own cursor"
        );

        outline.note_scrolled();
        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.scroll, 7, "the note's scroll took the window back");
        assert_eq!(
            outline.selected, 9,
            "and the selection went with the scroll"
        );
    }

    #[test]
    fn the_wheel_scrolls_the_window_and_leaves_the_selection() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);
        outline.note_scrolled();
        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.selected, 9, "precondition: the note put it there");
        assert_eq!(outline.scroll, 7);

        outline.wheel(3, entries.len());
        outline.follow_note(&entries, 90, false);
        assert_eq!(
            outline.scroll, 9,
            "the wheel moved the window, redraw and all"
        );
        assert_eq!(
            outline.selected, 9,
            "the reading position stays where the note put it"
        );
        assert_eq!(outline.active, 9, "and the active heading does not move");

        outline.wheel(3, entries.len());
        assert_eq!(outline.scroll, 9, "clamped at the last window");
        outline.wheel(-4, entries.len());
        assert_eq!(outline.scroll, 5);
        assert_eq!(
            outline.selected, 9,
            "a selection the view scrolled away stays selected"
        );
    }

    #[test]
    fn a_note_scroll_releases_the_wheel_window() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);
        outline.note_scrolled();
        outline.follow_note(&entries, 90, false);
        outline.wheel(-5, entries.len());
        assert_eq!(outline.scroll, 2);
        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.scroll, 2, "the panel stopped following the note");
        assert_eq!(outline.selected, 9);

        outline.note_scrolled();
        outline.follow_note(&entries, 20, false);
        assert_eq!(outline.active, 2);
        assert_eq!(outline.selected, 2, "the scroll took the selection back");
        assert_eq!(outline.scroll, 2, "and the window follows the note again");

        outline.note_scrolled();
        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.scroll, 7, "a later scroll follows as before");
    }

    #[test]
    fn keys_and_a_click_take_the_window_back_from_the_wheel() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);
        outline.wheel(5, entries.len());
        assert_eq!(outline.scroll, 5);

        outline.move_down(1, entries.len());
        assert_eq!(outline.selected, 1);
        assert_eq!(
            outline.scroll, 1,
            "a key's move brings the cursor into view"
        );
        outline.follow_note(&entries, 90, false);
        assert_eq!(
            outline.scroll, 1,
            "a redraw does not hand the window back to the note"
        );
        assert_eq!(outline.selected, 1, "the reader's cursor stays put");

        outline.select(10, entries.len());
        assert_eq!(outline.scroll, 8, "a click scrolls the window to the row");
        outline.follow_note(&entries, 0, false);
        assert_eq!(outline.scroll, 8, "and a redraw keeps it there");
        assert_eq!(outline.selected, 10);
    }

    #[test]
    fn a_wheel_window_survives_a_viewport_change_clamped_to_the_list() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let entries = spaced(&(0..12).map(|i| i * 10).collect::<Vec<_>>());
        outline.wheel(5, entries.len());
        assert_eq!(outline.scroll, 5);

        outline.on_viewport_height(4, entries.len());
        assert_eq!(outline.scroll, 5, "the drawer does not snap it to a cursor");

        // A shorter list clamps a stale wheel window to its end.
        outline.on_viewport_height(4, 6);
        assert_eq!(outline.scroll, 2);
        outline.wheel(1, 6);
        assert_eq!(outline.scroll, 2, "and the wheel stays clamped there");
    }

    #[test]
    fn a_jump_after_a_wheel_brings_the_heading_back_into_view() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let entries = spaced(&(0..12).map(|i| i * 10).collect::<Vec<_>>());
        outline.wheel(9, entries.len());
        outline.follow_note(&entries, 10, false);
        assert_eq!(outline.scroll, 9, "precondition: the wheel window stays");

        // The reader jumps to a heading above the wheeled window: the
        // window follows the pinned mark, not the scroll the wheel left.
        outline.pin_active(&entries, 1);
        outline.follow_note(&entries, 10, false);
        assert_eq!(outline.active, 1);
        assert_eq!(outline.scroll, 1, "the jumped-to heading is in view");
    }

    #[test]
    fn a_note_change_resets_both_cursors_and_drops_the_pin() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let lines: Vec<usize> = (0..12).map(|i| i * 10).collect();
        let entries = spaced(&lines);

        outline.select(11, entries.len());
        outline.pin_active(&entries, 11);
        // The new note has as many headings as the old one, so the old
        // selection index would still be a valid row: only a reset puts the
        // reader back at the top.
        outline.on_note_change(12);
        assert_eq!(
            outline.selected, 0,
            "the selection's index means nothing across notes"
        );
        assert_eq!(outline.active, 0);
        assert_eq!(outline.scroll, 0);

        outline.follow_note(&entries, 90, false);
        assert_eq!(outline.active, 9, "the old note's pin is gone");
    }

    #[test]
    fn a_pin_follows_its_heading_by_title_when_a_reparse_shifts_the_rows() {
        let mut outline = Outline::new();
        outline.viewport = 4;
        let original = titled(&["zero", "one", "two", "three"]);
        outline.pin_active(&original, 3);

        // An external edit inserted a heading above the pinned one: the
        // rows moved, the heading did not.
        let inserted = titled(&["zero", "inserted", "one", "two", "three"]);
        outline.follow_note(&inserted, 0, false);
        assert_eq!(
            outline.active, 4,
            "the pin named the heading, not the old row index"
        );

        // The heading itself is gone: the pin is dropped and the note's
        // scroll names the active row again.
        let removed = titled(&["zero", "inserted", "one", "two"]);
        outline.follow_note(&removed, 0, false);
        assert_eq!(outline.active, 0, "the dropped pin yields to the scroll");

        // A later reparse that brings the title back does not revive it:
        // the jump's pin ended with its heading.
        let restored = titled(&["zero", "inserted", "one", "two", "three"]);
        outline.follow_note(&restored, 1, false);
        assert_eq!(outline.active, 1, "the dropped pin does not come back");
    }

    #[test]
    fn a_pin_to_a_duplicate_title_marks_the_right_occurrence() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        let entries = titled(&["dup", "other", "dup"]);
        outline.pin_active(&entries, 2);
        outline.follow_note(&entries, 0, false);
        assert_eq!(
            outline.active, 2,
            "the second duplicate, not the first row carrying the title"
        );

        // An edit inserts a differently-titled heading above: the mark
        // stays on the same occurrence of the title.
        let inserted = titled(&["new", "dup", "other", "dup"]);
        outline.follow_note(&inserted, 0, false);
        assert_eq!(
            outline.active, 3,
            "the title and its occurrence name the heading, not the row"
        );

        // Removing the second duplicate leaves the occurrence unclaimed:
        // the pin is dropped rather than sliding onto the first one.
        let removed = titled(&["new", "dup", "other"]);
        outline.follow_note(&removed, 0, false);
        assert_eq!(outline.active, 0, "the pinned occurrence is gone");
    }

    #[test]
    fn follow_note_of_an_empty_list_keeps_every_cursor_at_the_top() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        outline.selected = 4;
        outline.scroll = 2;
        outline.follow_note(&[], 40, true);
        assert_eq!(outline.selected, 0);
        assert_eq!(outline.active, 0);
        assert_eq!(outline.scroll, 0);
        outline.on_viewport_height(3, 0); // must not panic or divide
    }

    #[test]
    fn move_clamps_and_scrolls_the_selection_into_view() {
        let mut outline = Outline::new();
        outline.viewport = 3;
        outline.move_down(10, 12);
        assert_eq!(outline.selected, 10);
        assert_eq!(outline.scroll, 8, "the selected row is the last visible");
        outline.move_down(10, 12);
        assert_eq!(outline.selected, 11, "clamped at the last row");
        assert_eq!(outline.scroll, 9);
        outline.move_up(100, 12);
        assert_eq!(outline.selected, 0);
        assert_eq!(outline.scroll, 0);
        outline.move_down(1, 0); // an empty list must not panic
        assert_eq!(outline.selected, 0);
    }

    #[test]
    fn sync_clamps_a_stale_selection_when_the_note_shrinks() {
        let mut outline = Outline::new();
        outline.viewport = 2;
        outline.selected = 7;
        outline.scroll = 5;
        outline.sync(3);
        assert_eq!(outline.selected, 2, "selection clamped to the new last row");
        assert_eq!(outline.scroll, 1, "the window is clamped to the list too");
        outline.sync(0);
        assert_eq!(outline.selected, 0);
        assert_eq!(outline.scroll, 0);
    }

    #[test]
    fn select_ignores_out_of_range_rows() {
        let mut outline = Outline::new();
        outline.select(3, 2);
        assert_eq!(outline.selected, 0);
        outline.select(1, 2);
        assert_eq!(outline.selected, 1);
    }
}
