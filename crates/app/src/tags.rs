//! Tag view: the sidebar's tag-list mode (`t` toggles tree ⇄ tags).
//!
//! A flat list of tags (frontmatter tags, from the vault index), each
//! expandable into its notes; opening a note reuses the same open path as
//! the tree. The model is terminal-independent; rendering lives in `ui.rs`
//! and shares the sidebar's scroll/focus plumbing shape.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use vault::VaultIndex;

/// One tag with its notes (path order, from the index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagGroup {
    pub tag: String,
    pub notes: Vec<PathBuf>,
}

/// Selection: either a tag row or a note row under an expanded tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Tag(String),
    Note { tag: String, path: PathBuf },
}

/// One flattened display row.
#[derive(Debug)]
pub struct TagRow<'a> {
    pub group: &'a TagGroup,
    /// `Some` for a note row under an expanded tag.
    pub note: Option<&'a PathBuf>,
}

/// Tag list built from a [`VaultIndex`] snapshot.
pub struct TagList {
    pub groups: Vec<TagGroup>,
    pub expanded: HashSet<String>,
    pub selected: Option<Selection>,
    /// Viewport height fed back from `ui::draw` (inner height in rows).
    pub viewport: usize,
    pub scroll: usize,
}

impl Default for TagList {
    fn default() -> Self {
        Self::new()
    }
}

impl TagList {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            viewport: 20,
            scroll: 0,
        }
    }

    /// Rebuilds the tag groups from the index. Tags sort by note count
    /// (descending), then by tag name — the busiest tags first. The
    /// selection survives as long as its tag (and note, if any) still
    /// exists; otherwise it falls back to the first row.
    pub fn refresh(&mut self, index: &VaultIndex) {
        let mut groups: Vec<TagGroup> = index
            .tag_counts()
            .into_iter()
            .map(|(tag, _)| TagGroup {
                tag: tag.to_string(),
                notes: index
                    .notes_with_tag(tag)
                    .iter()
                    .map(|n| n.path.clone())
                    .collect(),
            })
            .collect();
        groups.sort_by(|a, b| b.notes.len().cmp(&a.notes.len()).then(a.tag.cmp(&b.tag)));
        self.groups = groups;
        self.validate_selection();
        self.clamp_scroll();
    }

    fn validate_selection(&mut self) {
        let valid = match &self.selected {
            Some(Selection::Tag(tag)) => self.groups.iter().any(|g| g.tag == *tag),
            Some(Selection::Note { tag, path }) => self
                .groups
                .iter()
                .any(|g| g.tag == *tag && g.notes.contains(path)),
            None => false,
        };
        if !valid {
            self.selected = self.rows().first().map(|r| match r.note {
                Some(note) => Selection::Note {
                    tag: r.group.tag.clone(),
                    path: (*note).clone(),
                },
                None => Selection::Tag(r.group.tag.clone()),
            });
            self.scroll = 0;
        }
    }

    /// Flattened display order: tag rows with their (expanded) notes
    /// directly beneath.
    pub fn rows(&self) -> Vec<TagRow<'_>> {
        let mut rows = Vec::new();
        for group in &self.groups {
            rows.push(TagRow { group, note: None });
            if self.expanded.contains(&group.tag) {
                for note in &group.notes {
                    rows.push(TagRow {
                        group,
                        note: Some(note),
                    });
                }
            }
        }
        rows
    }

    /// Does this row carry the selection?
    /// Does this row carry the selection? (The renderer's highlight test.)
    pub fn row_matches(row: &TagRow<'_>, selected: &Option<Selection>) -> bool {
        match (selected, row.note) {
            (Some(Selection::Tag(tag)), None) => *tag == row.group.tag,
            (Some(Selection::Note { tag, path }), Some(note)) => {
                *tag == row.group.tag && *path == **note
            }
            _ => false,
        }
    }

    /// Index of the selected row in `rows()`, if still present.
    pub fn selection_index(&self) -> Option<usize> {
        self.rows()
            .iter()
            .position(|r| Self::row_matches(r, &self.selected))
    }

    /// The selected note's path, for the status bar / opening.
    pub fn selected_path(&self) -> Option<&Path> {
        match &self.selected {
            Some(Selection::Note { path, .. }) => Some(path.as_path()),
            _ => None,
        }
    }

    /// Moves the selection cursor to `index` without activating it (the
    /// scrollbar's position control: it selects, never opens or toggles)
    /// and keeps the row visible. Out-of-range indices are ignored.
    pub fn select(&mut self, index: usize) {
        let rows = self.rows();
        let Some(row) = rows.get(index) else {
            return;
        };
        self.selected = Some(match row.note {
            Some(note) => Selection::Note {
                tag: row.group.tag.clone(),
                path: (*note).clone(),
            },
            None => Selection::Tag(row.group.tag.clone()),
        });
        self.scroll_selection_into_view();
    }

    fn move_selection(&mut self, delta: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let cur = self.selection_index().unwrap_or(0);
        let next = (cur as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.select(next);
    }

    pub fn move_down(&mut self, n: usize) {
        self.move_selection(n as isize);
    }

    pub fn move_up(&mut self, n: usize) {
        self.move_selection(-(n as isize));
    }

    /// Enter: toggle a tag's expansion, or report the note to open.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let sel = self.selected.clone()?;
        match sel {
            Selection::Tag(tag) => {
                if !self.expanded.remove(&tag) {
                    self.expanded.insert(tag);
                }
                None
            }
            Selection::Note { path, .. } => Some(path),
        }
    }

    /// `l`/Right on a collapsed tag: expand it (on a note the app moves
    /// focus to the content pane instead).
    pub fn expand_or_activate(&mut self) -> Option<PathBuf> {
        let sel = self.selected.clone()?;
        match sel {
            Selection::Tag(tag) => {
                self.expanded.insert(tag);
                None
            }
            Selection::Note { path, .. } => Some(path),
        }
    }

    /// `h`/Left on an expanded tag: collapse it. On a note or a collapsed
    /// tag: collapse (and select) the owning tag.
    pub fn collapse_or_parent(&mut self) {
        let sel = match &self.selected {
            Some(s) => s.clone(),
            None => return,
        };
        let tag = match &sel {
            Selection::Tag(tag) => tag.clone(),
            Selection::Note { tag, .. } => tag.clone(),
        };
        if self.expanded.contains(&tag) {
            self.expanded.remove(&tag);
        }
        self.selected = Some(Selection::Tag(tag));
        self.scroll_selection_into_view();
    }

    /// Click on the row at `index` (mouse support): selects it, and
    /// mirrors Enter's semantics — a tag row toggles its expansion, a
    /// note row reports its path to open.
    pub fn click(&mut self, index: usize) -> Option<PathBuf> {
        let rows = self.rows();
        let (tag, note) = {
            let row = rows.get(index)?;
            (row.group.tag.clone(), row.note.map(|n| (*n).clone()))
        };
        self.selected = Some(match &note {
            Some(path) => Selection::Note {
                tag: tag.clone(),
                path: path.clone(),
            },
            None => Selection::Tag(tag.clone()),
        });
        self.scroll_selection_into_view();
        match note {
            Some(path) => Some(path),
            None => {
                if !self.expanded.remove(&tag) {
                    self.expanded.insert(tag);
                }
                None
            }
        }
    }

    /// Called from `ui::draw` with the sidebar's inner height.
    pub fn on_viewport_height(&mut self, height: usize) {
        self.viewport = height.max(1);
        self.clamp_scroll();
    }

    pub fn scroll_selection_into_view(&mut self) {
        if let Some(idx) = self.selection_index() {
            if idx >= self.scroll + self.viewport {
                self.scroll = idx + 1 - self.viewport;
            } else if idx < self.scroll {
                self.scroll = idx;
            }
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let len = self.rows().len();
        self.scroll = self.scroll.min(len.saturating_sub(self.viewport.min(len)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;

    fn index_with_tags() -> (TempDir, VaultIndex) {
        let dir = TempDir::new("tag-vault");
        fs::write(
            dir.path().join("a.md"),
            "---\nstatus: active\ntags: [mocktag, rust]\n---\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\nstatus: active\ntags: [mocktag]\n---\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("c.md"),
            "---\nstatus: active\ntags: [rust, obsidian]\n---\n",
        )
        .unwrap();
        let index = VaultIndex::build(dir.path()).unwrap();
        (dir, index)
    }

    #[test]
    fn refresh_sorts_by_count_then_name() {
        let (_d, index) = index_with_tags();
        let mut list = TagList::new();
        list.refresh(&index);
        let tags: Vec<&str> = list.groups.iter().map(|g| g.tag.as_str()).collect();
        assert_eq!(tags, vec!["mocktag", "rust", "obsidian"]);
        assert_eq!(list.groups[0].notes.len(), 2);
    }

    #[test]
    fn selection_survives_refresh_and_falls_back() {
        let (dir, index) = index_with_tags();
        let mut list = TagList::new();
        list.refresh(&index);
        list.move_down(1);
        let sel = list.selected.clone();
        list.refresh(&index);
        assert_eq!(list.selected, sel, "refresh keeps the selection");

        list.selected = Some(Selection::Note {
            tag: "mocktag".into(),
            path: dir.path().join("gone.md"),
        });
        list.refresh(&index);
        assert!(list.selected.is_some(), "falls back to the first row");
        assert_ne!(
            list.selected,
            Some(Selection::Note {
                tag: "mocktag".into(),
                path: dir.path().join("gone.md")
            })
        );
    }

    #[test]
    fn expand_toggle_and_note_activation() {
        let (dir, index) = index_with_tags();
        let mut list = TagList::new();
        list.refresh(&index);
        // First row = busiest tag (mocktag).
        assert!(list.activate().is_none(), "tag row: expand, not open");
        assert!(list.expanded.contains("mocktag"));
        assert_eq!(list.rows().len(), 3 + 2, "3 tags + 2 mocktag notes");

        list.move_down(1);
        assert!(matches!(list.selected, Some(Selection::Note { .. })));
        let opened = list.activate().expect("note row opens");
        assert_eq!(opened, dir.path().join("a.md"));

        // h collapses back to the tag row.
        list.collapse_or_parent();
        assert!(matches!(list.selected, Some(Selection::Tag(_))));
        assert!(!list.expanded.contains("mocktag"));
    }

    #[test]
    fn move_clamps_and_scrolls_into_view() {
        let (_d, index) = index_with_tags();
        let mut list = TagList::new();
        list.refresh(&index);
        list.expanded.insert("mocktag".into());
        list.expanded.insert("rust".into());
        list.expanded.insert("obsidian".into());
        assert_eq!(list.rows().len(), 3 + 5, "3 tags + 5 notes");
        list.viewport = 2;
        list.move_down(100);
        assert_eq!(list.scroll, list.rows().len() - 2, "clamped at bottom");
        list.move_up(100);
        assert_eq!(list.scroll, 0);
    }

    #[test]
    fn empty_index_has_no_rows() {
        let dir = TempDir::new("tag-empty");
        let index = VaultIndex::build(dir.path()).unwrap();
        let mut list = TagList::new();
        list.refresh(&index);
        assert!(list.rows().is_empty());
        assert!(list.activate().is_none());
        assert!(list.click(0).is_none(), "empty list: click is a no-op");
        list.move_down(1); // must not panic
        list.collapse_or_parent(); // must not panic
    }

    #[test]
    fn click_selects_toggles_tags_and_reports_notes() {
        let (dir, index) = index_with_tags();
        let mut list = TagList::new();
        list.refresh(&index);
        // Row 0 = the busiest tag (mocktag): click selects and expands it.
        assert!(list.click(0).is_none(), "tag click opens nothing");
        assert_eq!(
            list.selected,
            Some(Selection::Tag("mocktag".into())),
            "click selected the tag row"
        );
        assert!(list.expanded.contains("mocktag"));
        assert_eq!(list.rows().len(), 3 + 2);

        // Clicking the tag row again collapses it, selection stays.
        assert!(list.click(0).is_none());
        assert!(!list.expanded.contains("mocktag"));

        // Expand, then click the note row underneath (index 1): opens.
        list.expanded.insert("mocktag".into());
        assert_eq!(list.click(1), Some(dir.path().join("a.md")));
        assert_eq!(
            list.selected,
            Some(Selection::Note {
                tag: "mocktag".into(),
                path: dir.path().join("a.md"),
            })
        );
        // Past the last row: no-op.
        assert!(list.click(99).is_none());
    }
}
