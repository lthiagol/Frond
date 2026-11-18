//! Sidebar folder tree: a collapsible listing of the vault's `.md` notes
//! and `.base` tables.
//!
//! The model is terminal-independent; rendering lives in `ui.rs`.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use ratatui::layout::Rect;
use vault::walk::EntryKind;

/// One entry in the tree. Directory nodes own their subtree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Dir,
    Note,
    /// A `.base` table file: opens the table view, not the note viewer.
    Base,
}

/// A row of the sidebar as displayed: a node plus its indentation depth.
#[derive(Debug)]
pub struct Row<'a> {
    pub node: &'a Node,
    pub depth: usize,
}

/// Collapsible tree of the vault's `.md` notes and `.base` tables.
///
/// Expansion state lives in a `HashSet` of directory paths (not on the
/// nodes), so a rescan — which rebuilds `nodes` from disk — automatically
/// preserves which folders were open. The selection is likewise stored as a
/// path so it survives rescans that do not touch the selected file.
pub struct Sidebar {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    pub expanded: HashSet<PathBuf>,
    /// Path of the selected row; `None` only while the vault has no notes.
    pub selected: Option<PathBuf>,
    /// Viewport height fed back from `ui::draw` (inner height in rows).
    pub viewport: usize,
    /// First visible row index.
    pub scroll: usize,
}

impl Sidebar {
    /// Walks the vault root once and builds the tree. All folders start
    /// collapsed. Fails only if `root` itself cannot be read; per-entry
    /// errors (unreadable subdirectory, vanished file) are skipped.
    pub fn new(root: &Path) -> io::Result<Self> {
        let nodes = read_children(root, 0)?;
        let mut sidebar = Self {
            root: root.to_path_buf(),
            nodes,
            expanded: HashSet::new(),
            selected: None,
            viewport: 20,
            scroll: 0,
        };
        // Select the first row if the vault has one, so Enter works right
        // away and the status bar has something to show.
        sidebar.selected = sidebar.rows().first().map(|r| r.node.path.clone());
        Ok(sidebar)
    }

    /// Re-walks the vault. Expansion state survives by construction; the
    /// selection survives too, falling back to the first row if its file
    /// disappeared.
    pub fn rescan(&mut self) {
        match read_children(&self.root, 0) {
            Ok(nodes) => self.nodes = nodes,
            Err(_) => return, // vault dir temporarily unreadable: keep old tree
        }
        let still_there = self
            .selected
            .as_ref()
            .is_some_and(|sel| self.find(sel).is_some());
        if !still_there {
            self.selected = self.rows().first().map(|r| r.node.path.clone());
            self.scroll = 0;
        }
        self.clamp_scroll(self.row_count());
    }

    /// Flattened display order: depth-first, folders before files,
    /// case-insensitive name order within each group (see `read_children`).
    /// Children appear only while their folder is expanded.
    pub fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        self.collect_rows(&self.nodes, 0, &mut rows);
        rows
    }

    fn collect_rows<'a>(&'a self, nodes: &'a [Node], depth: usize, out: &mut Vec<Row<'a>>) {
        for node in nodes {
            out.push(Row { node, depth });
            if node.kind == NodeKind::Dir && self.expanded.contains(&node.path) {
                self.collect_rows(&node.children, depth + 1, out);
            }
        }
    }

    /// Index of the selected row in `rows()`, if it is still present.
    #[cfg(test)]
    pub fn selection_index(&self) -> Option<usize> {
        self.selection_and_len().0
    }

    /// Number of visible rows (`rows().len()` without building the list).
    pub fn row_count(&self) -> usize {
        self.selection_and_len().1
    }

    /// Selected row index and total row count from a single tree walk that
    /// allocates nothing — scroll bookkeeping needs both on every key press
    /// and every frame.
    fn selection_and_len(&self) -> (Option<usize>, usize) {
        fn walk(sidebar: &Sidebar, nodes: &[Node], count: &mut usize, found: &mut Option<usize>) {
            for node in nodes {
                if found.is_none() && sidebar.selected.as_deref() == Some(node.path.as_path()) {
                    *found = Some(*count);
                }
                *count += 1;
                if node.kind == NodeKind::Dir && sidebar.expanded.contains(&node.path) {
                    walk(sidebar, &node.children, count, found);
                }
            }
        }
        let (mut count, mut found) = (0, None);
        walk(self, &self.nodes, &mut count, &mut found);
        (found, count)
    }

    /// The selected node's full path, for the status bar.
    pub fn selected_path(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    /// Kind of the selected node, if any (drives `l`'s behavior).
    pub fn selected_kind(&self) -> Option<NodeKind> {
        let sel = self.selected.as_ref()?;
        self.find(sel).map(|n| n.kind)
    }

    pub fn move_down(&mut self, n: usize) {
        self.move_selection(n as isize);
    }

    pub fn move_up(&mut self, n: usize) {
        self.move_selection(-(n as isize));
    }

    fn move_selection(&mut self, delta: isize) {
        let (cur, len) = self.selection_and_len();
        if len == 0 {
            return;
        }
        let next = (cur.unwrap_or(0) as isize + delta).clamp(0, len as isize - 1) as usize;
        let path = self.rows()[next].node.path.clone();
        self.selected = Some(path);
        self.scroll_selection_into_view();
    }

    /// Enter: toggle a folder, or report the note to open.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let sel = self.selected.clone()?;
        if self.is_dir(&sel) {
            if !self.expanded.remove(&sel) {
                self.expanded.insert(sel);
            }
            None
        } else {
            Some(sel)
        }
    }

    /// Reveal `path` in the tree: expand every ancestor folder and select
    /// the note or `.base` table, scrolled into view. Called when a file
    /// is opened from anywhere but the tree itself (link follow, search
    /// hit, backlink, session restore), so the sidebar shows where the
    /// reader landed. Paths that are not tree nodes (a non-`.md`/`.base`
    /// file, or something outside the vault) still expand their ancestors
    /// below the root, but the selection stays put — there is no row for
    /// them. No focus change; the reader stays on the content pane.
    pub fn reveal(&mut self, path: &Path) {
        // Non-normalized paths (`..` components) are lexically inside the
        // root but semantically outside: no reveal at all.
        if path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return;
        }
        for ancestor in path.ancestors().skip(1) {
            if ancestor == self.root || !ancestor.starts_with(&self.root) {
                break;
            }
            self.expanded.insert(ancestor.to_path_buf());
        }
        if self.find(path).is_some() {
            self.selected = Some(path.to_path_buf());
            self.scroll_selection_into_view();
        }
    }

    /// `l`/Right on a folder: expand it. (Only called for directories; on
    /// a note the app moves focus to the content pane instead.)
    pub fn expand_or_activate(&mut self) -> Option<PathBuf> {
        let sel = self.selected.clone()?;
        if self.is_dir(&sel) {
            self.expanded.insert(sel);
            None
        } else {
            Some(sel)
        }
    }

    /// `h`/Left on an expanded folder: collapse it. On a collapsed folder or
    /// a note: collapse (and select) its parent folder, if any.
    pub fn collapse_or_parent(&mut self) {
        let sel = match &self.selected {
            Some(s) => s.clone(),
            None => return,
        };
        if self.is_dir(&sel) && self.expanded.contains(&sel) {
            self.expanded.remove(&sel);
            return;
        }
        // Notes and already-collapsed folders: fold up one level.
        let parent = match sel.parent() {
            Some(p) if p != self.root => p.to_path_buf(),
            _ => return, // top level: nothing to collapse
        };
        self.expanded.remove(&parent);
        self.selected = Some(parent);
        self.scroll_selection_into_view();
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.find(path).map(|n| n.kind) == Some(NodeKind::Dir)
    }

    fn find(&self, path: &Path) -> Option<&Node> {
        self.find_in(&self.nodes, path)
    }

    fn find_in<'a>(&'a self, nodes: &'a [Node], path: &Path) -> Option<&'a Node> {
        for node in nodes {
            if node.path == path {
                return Some(node);
            }
            if let Some(found) = self.find_in(&node.children, path) {
                return Some(found);
            }
        }
        None
    }

    /// Called from `ui::draw` with the sidebar's inner height.
    pub fn on_viewport_height(&mut self, height: usize) {
        self.viewport = height.max(1);
        self.clamp_scroll(self.row_count());
    }

    /// Scroll so the selection is visible, then clamp.
    pub fn scroll_selection_into_view(&mut self) {
        let (selection, len) = self.selection_and_len();
        if let Some(idx) = selection {
            if idx >= self.scroll + self.viewport {
                self.scroll = idx + 1 - self.viewport;
            } else if idx < self.scroll {
                self.scroll = idx;
            }
        }
        self.clamp_scroll(len);
    }

    fn clamp_scroll(&mut self, len: usize) {
        self.scroll = self.scroll.min(len.saturating_sub(self.viewport.min(len)));
    }

    /// Click on the row at `index` (from mouse support): selects it, and
    /// mirrors Enter's semantics — a folder toggles its expansion, a note
    /// reports its path to open.
    pub fn click(&mut self, index: usize) -> Option<PathBuf> {
        let rows = self.rows();
        let (path, kind) = {
            let row = rows.get(index)?;
            (row.node.path.clone(), row.node.kind)
        };
        self.selected = Some(path.clone());
        self.scroll_selection_into_view();
        if kind == NodeKind::Dir {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            None
        } else {
            Some(path)
        }
    }

    /// Maps an absolute terminal row (0-based) to a row index in `rows()`
    /// for this sidebar rendered in `area`, accounting for the top border
    /// row and the scroll offset. `None` when the click is on a border row
    /// or below the last visible row.
    pub fn row_index_at_screen_row(&self, area: Rect, screen_row: u16) -> Option<usize> {
        list_row_at(screen_row, area, self.scroll, self.row_count())
    }
}

/// Screen-row → list-row mapping shared by the sidebar tree and the tag
/// list (both render as a bordered list scrolled by `scroll`).
///
/// The inner list starts one row below `area.y` (top border) and spans
/// `area.height - 2` rows; the first visible row is `scroll`. Returns the
/// index into the full row list, or `None` for border rows, rows past the
/// widget, and rows past the last row of a short list.
pub fn list_row_at(screen_row: u16, area: Rect, scroll: usize, rows_len: usize) -> Option<usize> {
    let inner_y = u32::from(area.y) + 1;
    let inner_height = u32::from(area.height.saturating_sub(2));
    let row = u32::from(screen_row);
    if row < inner_y || row >= inner_y + inner_height {
        return None;
    }
    let index = scroll + (row - inner_y) as usize;
    (index < rows_len).then_some(index)
}

/// Lists `dir`'s immediate entries as nodes; directories carry their
/// subtree recursively. The listing rules (folders first then files,
/// case-insensitive order, hidden entries and non-indexable files skipped,
/// directory symlinks refused, depth cap) are the vault's own, see
/// [`vault::walk::read_dir_entries`].
fn read_children(dir: &Path, depth: usize) -> io::Result<Vec<Node>> {
    Ok(vault::walk::read_dir_entries(dir, depth)?
        .into_iter()
        .map(|entry| {
            let (kind, children) = match entry.kind {
                EntryKind::Dir => (
                    NodeKind::Dir,
                    read_children(&entry.path, depth + 1).unwrap_or_default(),
                ),
                EntryKind::Note => (NodeKind::Note, Vec::new()),
                EntryKind::Base => (NodeKind::Base, Vec::new()),
            };
            Node {
                name: entry.name,
                path: entry.path,
                kind,
                children,
            }
        })
        .collect())
}

/// Truncates `text` to at most `max_width` display columns, replacing the
/// last visible cell with `…` when truncation happens.
pub fn truncate_ellipsis(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(1).max(1);
        if used + w > max_width - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;

    /// vault/
    ///   alpha.md
    ///   Archive/  (nested: old.md)
    ///   beta.md
    ///   .obsidian/ (skipped)
    ///   notes.txt  (skipped)
    fn fixture() -> TempDir {
        let dir = TempDir::new("sb-vault");
        fs::write(dir.path().join("alpha.md"), "a").unwrap();
        fs::write(dir.path().join("beta.md"), "b").unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        fs::create_dir_all(dir.path().join("Archive")).unwrap();
        fs::write(dir.path().join("Archive").join("old.md"), "o").unwrap();
        fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
        dir
    }

    #[test]
    fn build_lists_dirs_first_md_only_no_hidden() {
        let vault = fixture();
        let sb = Sidebar::new(vault.path()).unwrap();
        let names: Vec<&str> = sb.rows().iter().map(|r| r.node.name.as_str()).collect();
        assert_eq!(names, vec!["Archive", "alpha.md", "beta.md"]);
    }

    #[test]
    fn collapsed_dirs_hide_children() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        assert_eq!(sb.rows().len(), 3); // Archive collapsed
        sb.expanded.insert(vault.path().join("Archive"));
        let names: Vec<&str> = sb.rows().iter().map(|r| r.node.name.as_str()).collect();
        assert_eq!(names, vec!["Archive", "old.md", "alpha.md", "beta.md"]);
        assert_eq!(sb.rows()[1].depth, 1);
    }

    #[test]
    fn activate_toggles_dir_opens_note() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.selected = Some(vault.path().join("Archive"));
        assert!(sb.activate().is_none(), "dir: no note to open");
        assert!(sb.expanded.contains(&vault.path().join("Archive")));
        sb.activate();
        assert!(!sb.expanded.contains(&vault.path().join("Archive")));

        sb.selected = Some(vault.path().join("alpha.md"));
        assert_eq!(
            sb.activate(),
            Some(vault.path().join("alpha.md")),
            "note: returns path to open"
        );
    }

    #[test]
    fn expand_or_activate() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.selected = Some(vault.path().join("Archive"));
        assert!(sb.expand_or_activate().is_none());
        assert!(sb.expanded.contains(&vault.path().join("Archive")));
        sb.selected = Some(vault.path().join("beta.md"));
        assert_eq!(sb.expand_or_activate(), Some(vault.path().join("beta.md")));
    }

    #[test]
    fn collapse_or_parent_walks_up() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.expanded.insert(vault.path().join("Archive"));
        sb.selected = Some(vault.path().join("Archive").join("old.md"));
        sb.collapse_or_parent();
        // Parent Archive collapsed and now selected.
        assert_eq!(sb.selected, Some(vault.path().join("Archive")));
        assert!(!sb.expanded.contains(&vault.path().join("Archive")));
        assert_eq!(sb.rows().len(), 3);

        // On an expanded dir: collapses in place, selection stays.
        sb.selected = Some(vault.path().join("Archive"));
        sb.expanded.insert(vault.path().join("Archive"));
        sb.collapse_or_parent();
        assert_eq!(sb.selected, Some(vault.path().join("Archive")));
        assert!(!sb.expanded.contains(&vault.path().join("Archive")));
    }

    #[test]
    fn rescan_preserves_expansion_and_selection() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        let archive = vault.path().join("Archive");
        sb.expanded.insert(archive.clone());
        sb.selected = Some(archive.join("old.md"));

        // Filesystem changes: new file, selected file deleted.
        fs::write(vault.path().join("new.md"), "n").unwrap();
        fs::remove_file(archive.join("old.md")).unwrap();
        sb.rescan();

        assert!(sb.expanded.contains(&archive), "expansion survives rescan");
        let names: Vec<&str> = sb.rows().iter().map(|r| r.node.name.as_str()).collect();
        assert_eq!(names, vec!["Archive", "alpha.md", "beta.md", "new.md"]);
        // Selected file is gone: falls back to the first row.
        assert_eq!(sb.selected, Some(vault.path().join("Archive")));
    }

    #[test]
    fn rescan_keeps_selection_when_file_still_exists() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.selected = Some(vault.path().join("beta.md"));
        fs::write(vault.path().join("gamma.md"), "g").unwrap();
        sb.rescan();
        assert_eq!(sb.selected, Some(vault.path().join("beta.md")));
    }

    #[test]
    fn move_selection_clamps_and_scrolls() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.viewport = 2;
        sb.move_down(100);
        assert_eq!(
            sb.selected,
            Some(vault.path().join("beta.md")),
            "clamped at last row"
        );
        assert_eq!(sb.scroll, 1, "selection scrolled into view");
        sb.move_up(100);
        assert_eq!(sb.selected, Some(vault.path().join("Archive")));
        assert_eq!(sb.scroll, 0);
    }

    #[test]
    fn empty_vault_has_no_selection() {
        let dir = TempDir::new("sb-empty");
        let mut sb = Sidebar::new(dir.path()).unwrap();
        assert!(sb.rows().is_empty());
        assert!(sb.selected.is_none());
        assert!(sb.activate().is_none());
        sb.move_down(1); // must not panic
        sb.move_up(1);
        sb.rescan(); // must not panic
    }

    #[test]
    fn reveal_expands_ancestors_and_selects_the_note() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        sb.viewport = 1;

        // Deep note under a collapsed folder: ancestors expand, the note
        // is selected and scrolled into view.
        sb.reveal(&vault.path().join("Archive").join("old.md"));
        assert!(sb.expanded.contains(&vault.path().join("Archive")));
        assert_eq!(sb.selected, Some(vault.path().join("Archive/old.md")));
        // Rows: [Archive, old.md, alpha.md, beta.md]; viewport 1 → the
        // selection (index 1) must be the visible row.
        assert_eq!(sb.scroll, 1);
        // The root itself is never expanded or selected.
        assert!(!sb.expanded.contains(vault.path()));
    }

    #[test]
    fn reveal_base_and_outside_paths() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        fs::write(vault.path().join("Archive").join("notes.base"), "").unwrap();
        sb.rescan();
        sb.selected = Some(vault.path().join("alpha.md"));

        // A `.base` table now has a tree row: ancestors expand and the
        // base row is selected like a note.
        sb.reveal(&vault.path().join("Archive").join("notes.base"));
        assert!(sb.expanded.contains(&vault.path().join("Archive")));
        assert_eq!(sb.selected, Some(vault.path().join("Archive/notes.base")));

        // Something outside the vault (a `..` escape) touches nothing.
        sb.selected = Some(vault.path().join("alpha.md"));
        let outside = vault.path().join("..").join("elsewhere.md");
        sb.reveal(&outside);
        assert_eq!(sb.selected, Some(vault.path().join("alpha.md")));
        assert_eq!(sb.expanded.len(), 1, "no stray expansion (Archive only)");
    }

    #[test]
    fn base_files_are_tree_rows_that_open_the_table() {
        let vault = fixture();
        fs::write(vault.path().join("tasks.base"), "").unwrap();
        fs::write(vault.path().join("Archive").join("board.base"), "").unwrap();
        let mut sb = Sidebar::new(vault.path()).unwrap();

        // Files sort together case-insensitively: Archive, then its two
        // children, then alpha.md, beta.md, tasks.base (expanded below).
        sb.expanded.insert(vault.path().join("Archive"));
        let rows: Vec<(&str, NodeKind)> = sb
            .rows()
            .iter()
            .map(|r| (r.node.name.as_str(), r.node.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Archive", NodeKind::Dir),
                ("board.base", NodeKind::Base),
                ("old.md", NodeKind::Note),
                ("alpha.md", NodeKind::Note),
                ("beta.md", NodeKind::Note),
                ("tasks.base", NodeKind::Base),
            ]
        );

        // Enter on a base row reports the path (routed to the table view
        // by `App::open_note`).
        sb.selected = Some(vault.path().join("tasks.base"));
        assert_eq!(
            sb.activate(),
            Some(vault.path().join("tasks.base")),
            "base: returns path to open"
        );
        assert_eq!(
            sb.click(sb.selection_index().unwrap()),
            Some(vault.path().join("tasks.base")),
            "click on a base row opens it too"
        );
        // `l` on a base row is a note-like activate, not a folder toggle.
        assert_eq!(sb.selected_kind(), Some(NodeKind::Base));
        assert_eq!(
            sb.expand_or_activate(),
            Some(vault.path().join("tasks.base"))
        );
    }

    #[test]
    fn truncate_ellipsis_respects_display_width() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_ellipsis("hello", 5), "hello");
        assert_eq!(truncate_ellipsis("hello", 4), "hel…");
        assert_eq!(truncate_ellipsis("hi", 0), "");
        // CJK chars are 2 columns wide: 中(2) + …(1) = 3 columns.
        assert_eq!(truncate_ellipsis("中文abc", 3), "中…");
        assert_eq!(truncate_ellipsis("中文abc", 5), "中文…");
    }

    /// Regression test for the symlink-cycle stack overflow: `ln -s .`
    /// inside a directory and `ln -s ..` back to the vault root both form
    /// cycles. `Sidebar::new`/`rescan` must survive both (dir symlinks are
    /// skipped) while file symlinks still resolve to notes.
    #[test]
    fn symlink_dir_loops_do_not_crash_file_links_still_open() {
        let dir = TempDir::new("sb-loop");
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("b.md"), "b").unwrap();
        // Cycles: sub/self -> sub, sub/up -> vault root.
        std::os::unix::fs::symlink(".", dir.path().join("sub").join("self")).unwrap();
        std::os::unix::fs::symlink("..", dir.path().join("sub").join("up")).unwrap();
        // File symlink: kept (a vault may have CLAUDE.md -> AGENTS.md).
        std::os::unix::fs::symlink("a.md", dir.path().join("link.md")).unwrap();

        let sb = Sidebar::new(dir.path()).unwrap();
        // `sub` starts collapsed: expand it so its children show up.
        let mut sb = sb;
        sb.expanded.insert(dir.path().join("sub"));
        let names: Vec<&str> = sb.rows().iter().map(|r| r.node.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["sub", "b.md", "a.md", "link.md"],
            "dir symlinks skipped, file symlink kept"
        );

        // The watch-tick path (rescan) must be equally safe.
        sb.rescan();
        assert!(sb.expanded.contains(&dir.path().join("sub")));
        assert_eq!(sb.rows().len(), 4);
    }

    /// Belt-and-braces: real (non-symlink) nesting deeper than `MAX_DEPTH`
    /// is truncated instead of recursed into.
    #[test]
    fn deep_nesting_is_depth_capped() {
        let dir = TempDir::new("sb-deep");
        let mut path = dir.path().to_path_buf();
        for i in 0..40 {
            path.push(format!("d{i:02}"));
            fs::create_dir_all(&path).unwrap();
        }
        fs::write(path.join("bottom.md"), "b").unwrap();

        let sb = Sidebar::new(dir.path()).unwrap();
        let max_depth = sb.rows().iter().map(|r| r.depth).max().unwrap();
        assert!(
            max_depth < 40,
            "recursion must be capped, got depth {max_depth}"
        );
        assert!(sb.rows().last().is_some());
    }

    // --- mouse support: screen-row → row-index mapping -------------------

    /// A 30×10 widget: border rows 0 and 9, inner rows 1..=8 (8 rows).
    #[test]
    fn list_row_at_maps_inner_rows_and_rejects_borders() {
        let area = Rect::new(0, 0, 30, 10);
        // Border rows never map.
        assert_eq!(list_row_at(0, area, 0, 100), None);
        assert_eq!(list_row_at(9, area, 0, 100), None);
        // Outside the widget entirely.
        assert_eq!(list_row_at(10, area, 0, 100), None);
        assert_eq!(list_row_at(20, area, 0, 100), None);
        // Inner rows map 1:1 at scroll 0.
        assert_eq!(list_row_at(1, area, 0, 100), Some(0));
        assert_eq!(list_row_at(8, area, 0, 100), Some(7));
        // Past the last row of a short list: no mapping.
        assert_eq!(list_row_at(8, area, 0, 7), None);
        assert_eq!(list_row_at(1, area, 0, 0), None);
        // Scroll offset shifts the window; a short list scrolled to the
        // bottom maps the top inner row to `scroll`.
        assert_eq!(list_row_at(1, area, 2, 100), Some(2));
        assert_eq!(list_row_at(8, area, 2, 10), Some(9));
        assert_eq!(list_row_at(1, area, 95, 100), Some(95), "near the end");
        assert_eq!(list_row_at(8, area, 95, 100), None, "past the end");
        // Non-zero origin area (sidebar not at the screen's top-left).
        let shifted = Rect::new(40, 5, 30, 10);
        assert_eq!(list_row_at(5, shifted, 0, 100), None, "top border");
        assert_eq!(list_row_at(6, shifted, 0, 100), Some(0));
        assert_eq!(list_row_at(13, shifted, 0, 100), Some(7));
        // Degenerate widget (smaller than its borders).
        assert_eq!(list_row_at(0, Rect::new(0, 0, 5, 1), 0, 10), None);
    }

    #[test]
    fn click_selects_toggles_folders_and_reports_notes() {
        let vault = fixture(); // rows collapsed: [Archive, alpha.md, beta.md]
        let mut sb = Sidebar::new(vault.path()).unwrap();
        let archive = vault.path().join("Archive");

        // Click on the folder row: selects it and expands it.
        assert!(sb.click(0).is_none(), "folder click opens nothing");
        assert_eq!(sb.selected, Some(archive.clone()));
        assert!(sb.expanded.contains(&archive));
        assert_eq!(sb.rows().len(), 4, "child visible after expanding");

        // Clicking the folder again collapses it (Enter semantics).
        assert!(sb.click(0).is_none());
        assert!(!sb.expanded.contains(&archive));
        assert_eq!(sb.selected, Some(archive.clone()), "selection stays");

        // Click on a note: selects it and reports the path to open.
        assert_eq!(sb.click(1), Some(vault.path().join("alpha.md")));
        assert_eq!(sb.selected, Some(vault.path().join("alpha.md")));

        // Index past the last row: no-op.
        assert!(sb.click(99).is_none());
        assert_eq!(sb.selected, Some(vault.path().join("alpha.md")));
    }

    #[test]
    fn click_respects_scroll_and_collapse_state() {
        let vault = fixture();
        let mut sb = Sidebar::new(vault.path()).unwrap();
        let archive = vault.path().join("Archive");
        sb.expanded.insert(archive.clone());
        // Expanded rows: [Archive, old.md, alpha.md, beta.md]; scroll 1
        // hides the Archive header, so inner row 0 is old.md.
        sb.viewport = 3;
        sb.scroll = 1;
        assert_eq!(sb.rows().len(), 4);

        // Screen row 1 (first inner row) → index 1 = old.md → opens.
        let area = Rect::new(0, 0, 30, 10);
        assert_eq!(sb.row_index_at_screen_row(area, 1), Some(1));
        assert_eq!(
            sb.row_index_at_screen_row(area, 1)
                .and_then(|i| sb.click(i)),
            Some(archive.join("old.md")),
            "collapsed/scroll state shifts which row the click lands on"
        );

        // Collapsing changes the row list: the same screen row now maps
        // to alpha.md (and rows below the shortened list map to nothing).
        sb.expanded.remove(&archive);
        assert_eq!(sb.row_index_at_screen_row(area, 1), Some(1));
        assert_eq!(
            sb.row_index_at_screen_row(area, 1)
                .map(|i| sb.rows()[i].node.name.clone()),
            Some("alpha.md".to_string())
        );
        assert_eq!(sb.row_index_at_screen_row(area, 4), None, "rows len 3");
    }
}
