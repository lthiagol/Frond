//! `.base` table view state and rendering.
//!
//! The app keeps at most one open base file (`App::base`). The parse and
//! the table model come from `vault::base`; this module owns the state
//! (selected view, scroll, file stamp) and the rendering into styled
//! lines plus click regions. The base file itself is deliberately NOT an
//! index entry: the index walks `.md` notes only, and the few `.base`
//! files are instead re-read here when their mtime+size stamp changes
//! (same `FileStamp` scheme as the viewer). Filters re-evaluate against
//! the live index on every rebuild, so note edits move rows between
//! views via the normal watch/reindex pipeline.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;
use vault::base::{self, BaseFile, BaseTable, TableRow};
use vault::VaultIndex;

use crate::sidebar::truncate_ellipsis;
use crate::theme::UiTheme;
use crate::viewer::FileStamp;

/// Column width cap before shrinking starts: a column never claims more
/// than this many cells even when every value is long.
const MAX_COL_WIDTH: usize = 40;
/// Minimum width a column keeps while shrinking (an ellipsis still fits).
const MIN_COL_WIDTH: usize = 3;
/// Space between columns.
const COL_SPACING: usize = 2;

/// Where a clickable cell was last rendered: absolute table line index
/// (0 = header row), inner-column range, and the note the cell opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseLinkRegion {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub path: PathBuf,
}

/// State of the currently open `.base` file.
pub struct BaseState {
    pub path: PathBuf,
    pub base: BaseFile,
    /// Selected view index (`]`/`[` step, `1`–`9` jumps; clamped).
    pub view: usize,
    /// The current view's table (rebuilt on open, base-file change, and
    /// every index change — filters re-evaluate live).
    pub table: BaseTable,
    pub scroll: usize,
    pub viewport: usize,
    stamp: Option<FileStamp>,
    /// Parse warnings of the loaded file (surfaced in the status bar).
    pub warnings: Vec<String>,
    /// Click regions of the last [`Self::render`] (empty before the
    /// first frame). Stored on the state so the click handler (which
    /// runs between frames) sees the same geometry the renderer used.
    pub regions: Vec<BaseLinkRegion>,
    /// Content-capped natural width of each column, computed once per
    /// [`Self::rebuild`] (it scans every row) instead of on every frame.
    natural_widths: Vec<usize>,
}

/// True when `path` is a `.base` file (the app's second content kind).
pub fn is_base_file(path: &Path) -> bool {
    vault::walk::is_base_path(path)
}

/// Today as `YYYY-MM-DD` (the caller's clock for date filters).
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl BaseState {
    /// Reads and parses `path`, building the first view's table.
    pub fn open(path: &Path, index: Option<&VaultIndex>, today: &str) -> io::Result<Self> {
        let source = fs::read_to_string(path)?;
        let base = base::parse_base(&source);
        let warnings = base.warnings.clone();
        let mut state = Self {
            path: path.to_path_buf(),
            base,
            view: 0,
            table: BaseTable {
                columns: Vec::new(),
                rows: Vec::new(),
            },
            scroll: 0,
            viewport: 24,
            stamp: FileStamp::of(path).ok(),
            warnings,
            regions: Vec::new(),
            natural_widths: Vec::new(),
        };
        state.rebuild(index, today);
        Ok(state)
    }

    /// Watch tick: re-reads the file when its stamp changed. Returns
    /// `true` when the file was re-parsed (the caller rebuilds).
    pub fn poll_file(&mut self) -> bool {
        let changed = FileStamp::of(&self.path)
            .ok()
            .is_some_and(|stamp| self.stamp != Some(stamp));
        if changed {
            match fs::read_to_string(&self.path) {
                Ok(source) => {
                    self.base = base::parse_base(&source);
                    self.warnings = self.base.warnings.clone();
                    self.view = self.view.min(self.view_count() - 1);
                    self.stamp = FileStamp::of(&self.path).ok();
                }
                // Unreadable (mid-editor-replace): keep the last good
                // parse; the next tick retries.
                Err(_) => return false,
            }
        }
        changed
    }

    /// Re-evaluates the current view against `index` (cheap: one filter
    /// pass over the indexed notes). Without an index the table keeps
    /// its columns but shows no rows.
    pub fn rebuild(&mut self, index: Option<&VaultIndex>, today: &str) {
        self.table = match index {
            Some(index) => base::build_table(index, &self.base, self.view, today),
            None => BaseTable {
                columns: self
                    .base
                    .views
                    .get(self.view)
                    .map(|view| {
                        view.order
                            .iter()
                            .map(|key| vault::base::Column {
                                key: key.clone(),
                                display: key.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                rows: Vec::new(),
            },
        };
        self.natural_widths = natural_column_widths(&self.table);
        self.clamp_scroll();
    }

    /// The number of views (always ≥ 1: parsing falls back to a default
    /// view when the file has no table views).
    pub fn view_count(&self) -> usize {
        debug_assert!(!self.base.views.is_empty(), "parse_base synthesises a view");
        self.base.views.len()
    }

    pub fn view_name(&self) -> &str {
        debug_assert!(self.view < self.view_count());
        &self.base.views[self.view].name
    }

    /// Switches to view `index` (clamped) and rebuilds the table at the
    /// top.
    pub fn set_view(&mut self, index: usize, vault_index: Option<&VaultIndex>, today: &str) {
        self.view = index.min(self.view_count() - 1);
        self.scroll = 0;
        self.rebuild(vault_index, today);
    }

    /// `]` / `[`: the next (`delta` 1) or previous (`delta` -1) view,
    /// wrapping at both ends.
    pub fn step_view(&mut self, delta: isize, vault_index: Option<&VaultIndex>, today: &str) {
        let count = self.view_count() as isize;
        let next = (self.view as isize + delta).rem_euclid(count) as usize;
        self.set_view(next, vault_index, today);
    }

    pub fn viewport_height(&mut self, height: usize) {
        self.viewport = height.max(1);
        self.clamp_scroll();
    }

    /// Total table lines: header + rule + one line per row. The renderer
    /// needs it before drawing to decide whether the table overflows.
    pub fn line_count(&self) -> usize {
        self.table.rows.len() + 2
    }

    /// The scroll clamp maximum.
    pub fn max_scroll(&self) -> usize {
        self.line_count()
            .saturating_sub(self.viewport.min(self.line_count()))
    }

    pub fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.max_scroll());
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    /// Sets the document position, clamped to the scroll range. The bar's
    /// track-row mapping already produces a value in range; the clamp keeps
    /// the invariant if a resize shrank the range since the last frame.
    pub fn set_scroll(&mut self, value: usize) {
        self.scroll = value.min(self.max_scroll());
    }

    /// The click target at an absolute table line + inner column, if any.
    pub fn link_at(&self, line: usize, col: usize) -> Option<PathBuf> {
        self.regions
            .iter()
            .find(|r| r.line == line && col >= r.start && col < r.end)
            .map(|r| r.path.clone())
    }

    /// Renders the visible window of the table for an inner area of
    /// `width`×`height`, refreshing `regions` for the click handler.
    pub fn render(&mut self, ui: UiTheme, width: usize, height: usize) -> Vec<Line<'static>> {
        let inner_width = width.max(1);
        let inner_height = height.max(1);
        self.viewport_height(inner_height);
        let widths = fit_column_widths(&self.natural_widths, inner_width);

        // Header + rule.
        let mut lines = vec![
            header_line(&self.table, &widths, ui),
            rule_line(inner_width, ui),
        ];
        let mut regions = Vec::new();
        let visible = inner_height.saturating_sub(2);
        for (row_index, row) in self
            .table
            .rows
            .iter()
            .skip(self.scroll)
            .take(visible)
            .enumerate()
        {
            let absolute_line = self.scroll + row_index + 2;
            match row {
                TableRow::Group(label) => lines.push(group_line(label, ui)),
                TableRow::Note(note_row) => {
                    let (line, region) =
                        note_line(note_row, &self.table.columns, &widths, absolute_line, ui);
                    lines.push(line);
                    regions.extend(region);
                }
            }
        }
        self.regions = regions;
        lines
    }
}

/// Natural column widths: header or widest cell, capped at
/// [`MAX_COL_WIDTH`]. Scans every row, so callers cache the result.
fn natural_column_widths(table: &BaseTable) -> Vec<usize> {
    table
        .columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let mut natural = column.display.width();
            for row in &table.rows {
                if let TableRow::Note(note) = row {
                    if let Some(cell) = note.cells.get(i) {
                        natural = natural.max(cell.text.width());
                    }
                }
            }
            natural.clamp(1, MAX_COL_WIDTH)
        })
        .collect()
}

/// Shrinks `natural` widths to fit `inner_width` by repeatedly trimming
/// the widest column.
fn fit_column_widths(natural: &[usize], inner_width: usize) -> Vec<usize> {
    let mut widths = natural.to_vec();
    let total = |ws: &[usize]| ws.iter().sum::<usize>() + ws.len().saturating_sub(1) * COL_SPACING;
    while total(&widths) > inner_width {
        let Some((widest, _)) = widths
            .iter()
            .enumerate()
            .max_by_key(|(i, w)| (**w, std::cmp::Reverse(*i)))
        else {
            break;
        };
        if widths[widest] <= MIN_COL_WIDTH {
            break;
        }
        widths[widest] -= 1;
    }
    widths
}

/// The header row, padded per column.
fn header_line(table: &BaseTable, widths: &[usize], ui: UiTheme) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, column) in table.columns.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let text = truncate_ellipsis(&column.display, widths[i]);
        spans.push(Span::styled(
            pad(text, widths[i]),
            Style::new()
                .add_modifier(Modifier::BOLD)
                .fg(ui.popup_border_fg),
        ));
    }
    Line::from(spans)
}

/// The full-width separator under the header.
fn rule_line(inner_width: usize, ui: UiTheme) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(inner_width),
        Style::new().fg(ui.dimmed_fg),
    ))
}

/// A group header row.
fn group_line(label: &str, ui: UiTheme) -> Line<'static> {
    Line::from(Span::styled(
        format!("▸ {label}"),
        Style::new()
            .add_modifier(Modifier::BOLD)
            .fg(ui.popup_border_fg),
    ))
}

/// One note row; returns the line and its clickable regions.
fn note_line(
    row: &vault::base::BaseNoteRow,
    columns: &[vault::base::Column],
    widths: &[usize],
    absolute_line: usize,
    ui: UiTheme,
) -> (Line<'static>, Vec<BaseLinkRegion>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut regions = Vec::new();
    let mut col = 0usize;
    for (i, column) in columns.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
            col += COL_SPACING;
        }
        let cell = row.cells.get(i);
        let raw_text = cell.map(|c| c.text.as_str()).unwrap_or("");
        let width = widths.get(i).copied().unwrap_or(0);
        let is_link = cell.is_some_and(|c| c.link);
        // Status columns show the shared glyph table's glyph before the
        // value (the vault layer emits the plain status string).
        if column.key == "status" {
            if let Some((glyph, style)) = crate::ui::status_glyph(Some(raw_text)) {
                let room = width.saturating_sub(2);
                spans.push(Span::styled(format!("{glyph} "), style.glyph_style(ui)));
                spans.push(Span::styled(
                    pad(truncate_ellipsis(raw_text, room), room),
                    Style::new(),
                ));
                col += width;
                continue;
            }
        }
        let text = truncate_ellipsis(raw_text, width);
        let start = col;
        let style = if is_link && !text.is_empty() {
            Style::new().add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        spans.push(Span::styled(pad(text, width), style));
        if is_link && !raw_text.is_empty() {
            regions.push(BaseLinkRegion {
                line: absolute_line,
                start,
                end: start + raw_text.width().min(width),
                path: row.path.clone(),
            });
        }
        col += width;
    }
    (Line::from(spans), regions)
}

/// Pads `text` with spaces to `width` (truncation already applied).
fn pad(text: String, width: usize) -> String {
    let text_width = text.width();
    if text_width >= width {
        text
    } else {
        let mut out = text;
        out.push_str(&" ".repeat(width - text_width));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::cell::RefCell;
    use std::rc::Rc;

    const TASKS_LIKE: &str = "\
filters:
  and:
    - file.inFolder(\"tasks\")
    - 'type == \"task\"'

views:
  - type: table
    name: \"Open\"
    filters:
      or:
        - 'status == \"pending\"'
        - 'status == \"active\"'
    order:
      - file.name
      - status
      - priority
    sort:
      - property: priority
        direction: DESC

  - type: table
    name: \"Done\"
    filters:
      and:
        - 'status == \"complete\"'
    order:
      - file.name
      - status
";

    fn note(_name: &str, frontmatter: &str) -> String {
        format!("---\n{frontmatter}---\nbody\n")
    }

    /// tasks-like vault: three open tasks (priorities 1/2/3), one done
    /// task, one project (wrong type).
    fn fixture() -> TempDir {
        let dir = TempDir::new("bv-vault");
        fs::create_dir_all(dir.path().join("tasks")).unwrap();
        fs::write(
            dir.path().join("tasks/a.md"),
            note(
                "a",
                "type: task\nstatus: active\npriority: 1\ntitle: Task A\n",
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join("tasks/b.md"),
            note(
                "b",
                "type: task\nstatus: pending\npriority: 3\ntitle: Task B\n",
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join("tasks/c.md"),
            note(
                "c",
                "type: task\nstatus: active\npriority: 2\ntitle: Task C\n",
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join("tasks/done.md"),
            note("done", "type: task\nstatus: complete\npriority: 9\n"),
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("projects")).unwrap();
        fs::write(
            dir.path().join("projects/p.md"),
            note("p", "type: project\nstatus: active\n"),
        )
        .unwrap();
        dir
    }

    fn state_for(dir: &TempDir) -> BaseState {
        let index = VaultIndex::build(dir.path()).unwrap();
        BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap()
    }

    fn write_base(dir: &TempDir, content: &str) {
        fs::write(dir.path().join("tasks.base"), content).unwrap();
    }

    #[test]
    fn open_builds_the_first_view_with_expected_counts() {
        let dir = fixture();
        write_base(&dir, TASKS_LIKE);
        let state = state_for(&dir);
        // Open view: the three tasks with pending/active status; the
        // done task and the project are filtered out.
        assert_eq!(state.table.note_rows(), 3);
        assert_eq!(state.view_name(), "Open");
        // Sorted by priority DESC: b (3), c (2), a (1).
        let names: Vec<String> = state
            .table
            .rows
            .iter()
            .filter_map(|r| match r {
                TableRow::Note(n) => {
                    Some(n.path.file_name().unwrap().to_string_lossy().into_owned())
                }
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["b.md", "c.md", "a.md"]);
        // Link cells: the file.name column is clickable and opens the note.
        let TableRow::Note(first) = &state.table.rows[0] else {
            panic!("note row");
        };
        assert!(first.cells[0].link);
        assert_eq!(first.cells[0].text, "b");
    }

    #[test]
    fn view_switch_changes_the_row_set() {
        let dir = fixture();
        write_base(&dir, TASKS_LIKE);
        let index = VaultIndex::build(dir.path()).unwrap();
        let mut state =
            BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap();
        assert_eq!(state.table.note_rows(), 3);
        state.set_view(1, Some(&index), "2026-01-01");
        assert_eq!(state.view_name(), "Done");
        assert_eq!(state.table.note_rows(), 1);
        // v cycles (wrapping back to Open).
        state.step_view(1, Some(&index), "2026-01-01");
        assert_eq!(state.view_name(), "Open");
        // Out-of-range jumps clamp.
        state.set_view(99, Some(&index), "2026-01-01");
        assert_eq!(state.view_name(), "Done");
    }

    #[test]
    fn editing_a_note_status_moves_rows_between_views_on_rebuild() {
        let dir = fixture();
        write_base(&dir, TASKS_LIKE);
        let index = Rc::new(RefCell::new(VaultIndex::build(dir.path()).unwrap()));
        let mut state = BaseState::open(
            &dir.path().join("tasks.base"),
            Some(&index.borrow()),
            "2026-01-01",
        )
        .unwrap();
        assert_eq!(state.table.note_rows(), 3);
        state.set_view(1, Some(&index.borrow()), "2026-01-01");
        assert_eq!(state.table.note_rows(), 1);

        // The watched edit: done.md flips to active (as the guarded
        // toggle would write it, then the poll reindexes).
        fs::write(
            dir.path().join("tasks/done.md"),
            note("done", "type: task\nstatus: active\npriority: 9\n"),
        )
        .unwrap();
        index
            .borrow_mut()
            .reindex_one(&dir.path().join("tasks/done.md"));
        state.rebuild(Some(&index.borrow()), "2026-01-01");
        assert_eq!(
            state.table.note_rows(),
            0,
            "the re-statused task left the Done view"
        );
        // And joined the Open view.
        state.set_view(0, Some(&index.borrow()), "2026-01-01");
        assert_eq!(state.table.note_rows(), 4);
    }

    #[test]
    fn editing_the_base_file_reparses_on_poll() {
        let dir = fixture();
        write_base(&dir, TASKS_LIKE);
        let index = VaultIndex::build(dir.path()).unwrap();
        let mut state =
            BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap();
        assert_eq!(state.view_count(), 2);
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_base(
            &dir,
            "views:\n  - type: table\n    name: Only\n    order:\n      - file.name\n",
        );
        assert!(state.poll_file(), "the stamp changed");
        assert_eq!(state.view_count(), 1);
        assert_eq!(state.view_name(), "Only");
        // Quiet tick: no change reported.
        assert!(!state.poll_file());
    }

    #[test]
    fn unparsable_base_fails_open_with_a_warning_and_all_notes() {
        let dir = fixture();
        write_base(&dir, "not a base file at all\n");
        let index = VaultIndex::build(dir.path()).unwrap();
        let state =
            BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap();
        assert!(!state.warnings.is_empty(), "warning surfaced");
        assert_eq!(state.table.note_rows(), 5, "fail-open: every note shows");
    }

    #[test]
    fn rendering_produces_clickable_regions_at_computed_columns() {
        let dir = fixture();
        write_base(&dir, TASKS_LIKE);
        let index = VaultIndex::build(dir.path()).unwrap();
        let mut state =
            BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap();
        let lines = state.render(
            crate::theme::ui_by_preset(engine::MarkdownThemePreset::default()),
            80,
            24,
        );
        // Header + rule + 3 rows.
        assert_eq!(lines.len(), 5);
        assert_eq!(state.regions.len(), 3, "one link region per row");
        // The first data row is table line 2; its region starts at 0.
        let first = &state.regions[0];
        assert_eq!(first.line, 2);
        assert_eq!(first.start, 0);
        assert!(first.end > 0);
        // Click mapping round-trips through link_at.
        assert_eq!(state.link_at(2, 0), Some(dir.path().join("tasks/b.md")));
        assert_eq!(state.link_at(2, 999), None);
        // Scrolling shifts which rows render (and their absolute lines).
        state.scroll_down(1);
        let lines = state.render(
            crate::theme::ui_by_preset(engine::MarkdownThemePreset::default()),
            80,
            4,
        );
        assert_eq!(
            lines.len(),
            4,
            "window renders the 2 remaining rows + header + rule"
        );
        assert!(
            state.regions.iter().all(|r| r.line >= state.scroll + 2),
            "regions keep absolute line indices"
        );
    }

    #[test]
    fn group_by_renders_header_rows() {
        let dir = TempDir::new("bv-group");
        fs::create_dir_all(dir.path().join("tasks")).unwrap();
        fs::write(
            dir.path().join("tasks/a.md"),
            note("a", "type: task\nstatus: active\narea: ops\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("tasks/b.md"),
            note("b", "type: task\nstatus: active\narea: ops\n"),
        )
        .unwrap();
        fs::write(
            dir.path().join("tasks/c.md"),
            note("c", "type: task\nstatus: active\narea: infra\n"),
        )
        .unwrap();
        write_base(
            &dir,
            "views:\n  - type: table\n    name: G\n    order:\n      - file.name\n    groupBy:\n      property: area\n      direction: ASC\n",
        );
        let index = VaultIndex::build(dir.path()).unwrap();
        let mut state =
            BaseState::open(&dir.path().join("tasks.base"), Some(&index), "2026-01-01").unwrap();
        assert_eq!(state.table.rows.len(), 5, "2 group headers + 3 rows");
        let lines = state.render(
            crate::theme::ui_by_preset(engine::MarkdownThemePreset::default()),
            60,
            12,
        );
        assert_eq!(lines.len(), 2 + 5);
        let infra_header = lines
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.contains("▸ infra")))
            .expect("group header rendered");
        assert!(infra_header >= 2, "headers render among the rows");
    }
}
