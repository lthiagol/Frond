//! Outline view: the sidebar's heading-list mode (`o`).
//!
//! The rows are the open note's heading table (`viewer.toc`), in document
//! order; the indentation is relative to the shallowest heading level in
//! that note. This module owns the selection cursor and its scroll window
//! (the same shape as `tags::TagList`); the rows themselves stay in the
//! viewer so the outline always follows the note on screen. Rendering lives
//! in `ui.rs`.

use engine::TocEntry;

/// One display row: a heading plus its indentation depth (the shallowest
/// level is depth 0).
pub struct Row<'a> {
    pub entry: &'a TocEntry,
    pub depth: usize,
}

/// The outline's rows for a note: document order, indented by the heading
/// level relative to the shallowest level present (level 1, 4 → depths
/// 0, 3). An empty table has no rows.
pub fn rows(toc: &[TocEntry]) -> Vec<Row<'_>> {
    let shallowest = toc.iter().map(|entry| entry.level).min();
    toc.iter()
        .map(|entry| Row {
            entry,
            depth: shallowest.map_or(0, |level| usize::from(entry.level.saturating_sub(level))),
        })
        .collect()
}

/// Selection cursor over the open note's heading rows.
pub struct Outline {
    /// Index of the selected row.
    pub selected: usize,
    /// Viewport height fed back from `ui::draw` (inner height in rows).
    pub viewport: usize,
    /// First visible row index.
    pub scroll: usize,
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
            viewport: 20,
            scroll: 0,
        }
    }

    /// Re-clamps the cursor to the current row count: the open note changed
    /// (or its headings were rebuilt), so the selection must stay valid.
    /// The selection survives by index where it can and is clamped to the
    /// last row otherwise; the scroll follows it.
    pub fn sync(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(len - 1);
        self.scroll_selection_into_view(len);
    }

    /// Selects a row (a click) and keeps it visible. Out-of-range indices
    /// are ignored.
    pub fn select(&mut self, index: usize, len: usize) {
        if index >= len {
            return;
        }
        self.selected = index;
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
        self.scroll_selection_into_view(len);
    }

    /// Called from `ui::draw` with the sidebar's inner height.
    pub fn on_viewport_height(&mut self, height: usize, len: usize) {
        self.viewport = height.max(1);
        self.scroll_selection_into_view(len);
    }

    pub fn scroll_selection_into_view(&mut self, len: usize) {
        if self.selected >= self.scroll + self.viewport {
            self.scroll = self.selected + 1 - self.viewport;
        } else if self.selected < self.scroll {
            self.scroll = self.selected;
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

    #[test]
    fn rows_are_indented_relative_to_the_shallowest_level() {
        let entries = toc(&[2, 3, 4, 2]);
        let rows = rows(&entries);
        let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 0], "level 2 is flush left");
        assert_eq!(rows[1].entry.title, "h1", "document order is kept");
    }

    #[test]
    fn rows_of_an_empty_toc_are_empty() {
        assert!(rows(&[]).is_empty());
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
        assert_eq!(outline.scroll, 1, "scroll follows the clamped selection");
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
