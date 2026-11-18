//! Viewer state: loaded note, parsed styled lines, scroll position, watch mode.

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use engine::{parse_markdown_with_width_opts, MarkdownTheme, ParseResult, SyntaxAssets, TocEntry};
use ratatui::text::Line;
use vault::{Resolution, VaultIndex};

use crate::links::{
    apply_link_styles, build_rendered_links, preprocess_with_links, LinkColors, LinkState,
    RenderedLink, WikilinkTarget,
};
use crate::tasks::{apply_task_styles, map_tasks, preprocess_tasks, RenderedTask};

/// One focusable renderable: a wikilink or a checkbox glyph. The
/// `x`/`X` cycle walks both in document (rendered) order; Enter
/// activates whichever kind is focused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focusable {
    Link(usize),
    Task(usize),
}

/// The vault index shared between the app (which mutates it on watch
/// events) and the viewer (which reads it at parse time to classify
/// wikilinks). `None` in single-file mode without a usable vault.
/// Invariant: never hold a `Ref`/`RefMut` across a call that can reparse
/// or re-borrow it (single-threaded event loop; see review 2026-11-11-1).
pub type SharedIndex = Rc<RefCell<VaultIndex>>;

/// Cheap file-change fingerprint used by the poll-based watcher.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileStamp {
    mtime: SystemTime,
    len: u64,
}

impl FileStamp {
    pub fn of(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        Ok(Self {
            mtime: meta.modified()?,
            len: meta.len(),
        })
    }
}

/// Holds everything needed to render one note. Terminal-independent so the
/// scroll/reload logic is unit-testable without a backend.
pub struct Viewer {
    pub path: PathBuf,
    /// Raw markdown source, kept so resizes can re-parse without re-reading.
    pub source: String,
    /// Engine-rendered styled lines (pre-wrapped at `render_width`),
    /// already restyled per wikilink resolution state (see `links`).
    pub lines: Vec<Line<'static>>,
    /// Pristine engine output (before wikilink restyling). Restyling is
    /// reapplied from this copy so focus changes are idempotent.
    base_lines: Vec<Line<'static>>,
    /// Heading table of the parsed note (rendered-line indices), used to
    /// scroll to `[[note#heading]]` targets.
    pub toc: Vec<TocEntry>,
    /// Every link in the parsed note (in first-appearance order), with
    /// per-wikilink resolution state.
    pub links: Vec<RenderedLink>,
    /// Every task-list (checkbox) item of the parsed note, mapped to its
    /// rendered glyph position — the click/keyboard target for the
    /// guarded toggle. Empty when the note has no tasks.
    pub tasks: Vec<RenderedTask>,
    /// Focused checkbox (index into `tasks`) for the shared `x`/`X`
    /// focus cycle; mutually exclusive with `focus_link`.
    pub focus_task: Option<usize>,
    /// Rendered-line index → raw source line number (1-based), straight
    /// from the engine's `source_line_map`. Used to scroll to a raw line
    /// found by the global search.
    pub source_line_map: Vec<usize>,
    /// Focused link for the `x`/`X` keyboard navigation, as an index into
    /// `links`.
    pub focus_link: Option<usize>,
    /// Scroll offsets are `u32`/`usize`: files can render to more than
    /// `u16::MAX` lines. Narrowing happens only at the ratatui
    /// `Paragraph::scroll` boundary (see `ui::draw`).
    pub scroll: u32,
    pub viewport_height: u32,
    pub render_width: usize,
    pub watch: bool,
    /// Whether the note's frontmatter metadata block renders at the top
    /// (`f` toggles; the config `metadata` key sets the startup default).
    /// Off, the body renders as if the frontmatter were absent — purely
    /// visual: tags/status/wikilinks come from the vault index either way.
    pub show_metadata: bool,
    pub help_open: bool,
    /// First visible body line of the help popup (scrolls when the help is
    /// taller than the popup). Reset each time the popup opens.
    pub help_scroll: usize,
    pub last_reload: Option<SystemTime>,
    pub last_error: Option<String>,
    assets: SyntaxAssets,
    theme: MarkdownTheme,
    stamp: Option<FileStamp>,
    /// Vault index used to resolve wikilinks at parse time.
    index: Option<SharedIndex>,
    /// Rendered line counts per wrap width, see [`Viewer::line_count_at`].
    line_counts: Option<LineCounts>,
    /// Bumped whenever `source` is replaced; lets caches check freshness
    /// with an integer compare instead of comparing/cloning the text.
    source_generation: u64,
    /// Unresolved/ambiguous/focus colors, from the active UI theme.
    link_colors: LinkColors,
}

/// Line counts of one source under one theme and metadata setting, per
/// wrap width. Valid only while all three inputs are unchanged.
struct LineCounts {
    source_generation: u64,
    show_metadata: bool,
    theme: MarkdownTheme,
    by_width: Vec<(usize, usize)>,
}

impl Viewer {
    /// Reads and parses the note. Fails only if the file cannot be read at
    /// startup; later reload failures are surfaced via `last_error`.
    pub fn open(path: &Path, render_width: usize) -> io::Result<Self> {
        let mut viewer = Self {
            path: path.to_path_buf(),
            source: String::new(),
            lines: Vec::new(),
            base_lines: Vec::new(),
            toc: Vec::new(),
            links: Vec::new(),
            tasks: Vec::new(),
            focus_task: None,
            source_line_map: Vec::new(),
            focus_link: None,
            scroll: 0,
            viewport_height: 24,
            render_width,
            watch: true,
            show_metadata: true,
            help_open: false,
            help_scroll: 0,
            last_reload: None,
            last_error: None,
            assets: SyntaxAssets::load_defaults(),
            theme: engine::default_markdown_theme(),
            stamp: None,
            index: None,
            line_counts: None,
            source_generation: 0,
            link_colors: LinkColors::default(),
        };
        viewer.open_file(path)?;
        Ok(viewer)
    }

    /// Attaches the vault index used for wikilink resolution and re-parses
    /// the current note with it (the initial open parsed before any index
    /// existed).
    pub fn set_index(&mut self, index: Option<SharedIndex>) {
        self.index = index;
        self.parse_source();
    }

    /// Sets the link/checkbox state colors (applied on the next restyle —
    /// the theme swap that follows re-parses).
    pub fn set_link_colors(&mut self, colors: LinkColors) {
        self.link_colors = colors;
    }

    /// Swaps the markdown palette and re-parses with it (theme cycling /
    /// hot-reload path: the re-render is instant, scroll preserved).
    pub fn set_markdown_theme(&mut self, theme: MarkdownTheme) {
        self.theme = theme;
        self.parse_source();
    }

    /// Swaps the syntect highlighting theme by name (code-block colors).
    /// Returns `false` when the name is unknown in syntect's default theme
    /// set — the current theme is kept and the caller warns/falls back.
    pub fn set_syntax_theme(&mut self, name: &str) -> bool {
        self.assets.set_syntax_theme(name)
    }

    /// Back to the engine default syntect theme (`base16-ocean.dark`).
    pub fn reset_syntax_theme(&mut self) {
        self.assets = SyntaxAssets::load_defaults();
    }

    /// Swaps in a different note: reads, parses, resets scroll, arms the
    /// watch stamp. The previous render is kept untouched if the read fails.
    pub fn open_file(&mut self, path: &Path) -> io::Result<()> {
        let source = fs::read_to_string(path)?;
        self.path = path.to_path_buf();
        self.source = source;
        self.source_generation += 1;
        self.focus_link = None; // a new note starts with no focused link
        self.focus_task = None;
        self.parse_source();
        self.stamp = FileStamp::of(path).ok();
        self.last_reload = Some(SystemTime::now());
        self.last_error = None;
        self.scroll = 0;
        self.clamp_scroll();
        Ok(())
    }

    /// Re-runs the parse (and wikilink classification) without touching
    /// the scroll position — also used after the index changed so link
    /// styling and resolution stay current. The `x`/`X` focus is preserved
    /// across reparses (clamped to the new link list).
    pub fn parse_source(&mut self) {
        // Task preprocessing runs first: `- [~]` markers are rewritten to
        // `- [ ]` on the preprocessed text (same length, line-preserving)
        // so the engine renders a clickable checkbox glyph for them; the
        // raw hits keep the on-disk state for the guarded write.
        let (task_preprocessed, task_hits) = preprocess_tasks(&self.source);
        // Wikilink preprocessing happens before the engine sees the text:
        // `[[target|alias]]` becomes `[alias](vault://N/target)` so the
        // engine's regular markdown-link pipeline (styling, wrapping,
        // tables, LinkSpan bookkeeping) handles it.
        let (preprocessed, wikilink_refs) = preprocess_with_links(&task_preprocessed);
        let result = self.render_preprocessed(&preprocessed, self.render_width);
        self.toc = result.toc;
        self.source_line_map = result.source_line_map;
        let states = self.classify_links(&wikilink_refs);
        self.links = build_rendered_links(&result.link_urls, &result.link_spans, &states);
        self.tasks = map_tasks(&task_hits, &self.source_line_map, &result.lines);
        // Preserve the `x`/`X` focus across reparses, clamped to the new
        // link list, so an unrelated vault-wide change (or a reload) does
        // not silently drop the focused link. `open_file` clears it before
        // parsing a new note.
        if let Some(i) = self.focus_link {
            self.focus_link = if self.links.is_empty() {
                None
            } else {
                Some(i.min(self.links.len() - 1))
            };
        }
        if let Some(i) = self.focus_task {
            self.focus_task = if self.tasks.is_empty() {
                None
            } else {
                Some(i.min(self.tasks.len() - 1))
            };
        }
        self.base_lines = result.lines;
        self.restyle_links();
    }

    /// The engine render of already-preprocessed source at `width`.
    fn render_preprocessed(&self, preprocessed: &str, width: usize) -> ParseResult {
        // The engine takes these as positional bools; name them here.
        let file_mode = false; // the viewer renders notes, not a leaf file view
        let code_line_numbers = true;
        parse_markdown_with_width_opts(
            preprocessed,
            self.assets.syntax_set,
            &self.assets.theme,
            width,
            &self.theme,
            file_mode,
            code_line_numbers,
            self.show_metadata,
        )
    }

    /// Rendered line count at `width`, cached per width while the source,
    /// theme and metadata setting are unchanged. `ui::draw` asks for the
    /// full-width count on every frame to decide the scrollbar; this keeps
    /// that from re-rendering the note on each redraw.
    pub fn line_count_at(&mut self, width: usize) -> usize {
        let width = width.max(20);
        let fresh = self.line_counts.as_ref().is_some_and(|cache| {
            cache.show_metadata == self.show_metadata
                && cache.theme == self.theme
                && cache.source_generation == self.source_generation
        });
        if !fresh {
            self.line_counts = Some(LineCounts {
                source_generation: self.source_generation,
                show_metadata: self.show_metadata,
                theme: self.theme,
                by_width: Vec::new(),
            });
        }
        let cache = self.line_counts.as_ref().expect("cache set above");
        if let Some(&(_, count)) = cache.by_width.iter().find(|(w, _)| *w == width) {
            return count;
        }
        let (task_preprocessed, _) = preprocess_tasks(&self.source);
        let (preprocessed, _) = preprocess_with_links(&task_preprocessed);
        let count = self.render_preprocessed(&preprocessed, width).lines.len();
        if let Some(cache) = self.line_counts.as_mut() {
            cache.by_width.push((width, count));
        }
        count
    }

    /// Resolution state for every preprocessed wikilink (document order).
    /// Self-links (`[[#heading]]`) always resolve to the containing note.
    /// With no index everything is unresolved (single-file mode).
    fn classify_links(&self, links: &[vault::LinkRef]) -> Vec<WikilinkTarget> {
        let index = self.index.as_ref().map(|i| i.borrow());
        links
            .iter()
            .map(|link| {
                let state = if link.target.is_empty() {
                    LinkState::Resolved(self.path.clone())
                } else {
                    match &index {
                        None => LinkState::NotFound,
                        Some(index) => match index.resolve_with_hint(&self.path, &link.target) {
                            Resolution::Found(path) => LinkState::Resolved(path),
                            Resolution::Ambiguous(candidates) => LinkState::Ambiguous(candidates),
                            Resolution::NotFound => LinkState::NotFound,
                        },
                    }
                };
                WikilinkTarget {
                    target: link.target.clone(),
                    heading: link.heading.clone(),
                    state,
                }
            })
            .collect()
    }

    /// `x`: focus the next focusable (links and checkboxes, in rendered
    /// document order, wrapping); selects the first one if nothing was
    /// focused. The focused item is scrolled into view. Notes without
    /// checkboxes behave exactly as before (links only).
    pub fn focus_next_link(&mut self) {
        self.step_focus(1);
    }

    /// `X`: the mirror image of `focus_next_link`.
    pub fn focus_prev_link(&mut self) {
        self.step_focus(-1);
    }

    /// The currently focused item (link or checkbox), if any.
    pub fn focused_item(&self) -> Option<Focusable> {
        if let Some(i) = self.focus_link {
            if self.links.get(i).is_some() {
                return Some(Focusable::Link(i));
            }
        }
        if let Some(i) = self.focus_task {
            if self.tasks.get(i).is_some() {
                return Some(Focusable::Task(i));
            }
        }
        None
    }

    /// Links and checkboxes merged, in rendered order (stable: same-line
    /// items keep their relative order).
    fn focusables(&self) -> Vec<Focusable> {
        let mut items: Vec<(usize, Focusable)> = self
            .links
            .iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    l.regions.first().map(|r| r.line).unwrap_or(usize::MAX),
                    Focusable::Link(i),
                )
            })
            .chain(
                self.tasks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (t.line, Focusable::Task(i))),
            )
            .collect();
        items.sort_by_key(|(line, _)| *line);
        items.into_iter().map(|(_, f)| f).collect()
    }

    fn step_focus(&mut self, delta: isize) {
        let items = self.focusables();
        if items.is_empty() {
            return;
        }
        let next = match self
            .focused_item()
            .and_then(|current| items.iter().position(|&f| f == current))
        {
            Some(pos) => {
                let len = items.len() as isize;
                ((pos as isize + delta).rem_euclid(len)) as usize
            }
            None => {
                if delta > 0 {
                    0
                } else {
                    items.len() - 1
                }
            }
        };
        match items[next] {
            Focusable::Link(i) => self.set_focus_link(Some(i)),
            Focusable::Task(i) => self.set_focus_task(Some(i)),
        }
    }

    pub fn set_focus_link(&mut self, index: Option<usize>) {
        if let Some(i) = index {
            if i >= self.links.len() {
                return;
            }
        }
        self.focus_link = index;
        if index.is_some() {
            self.focus_task = None;
        }
        self.restyle_links();
        if let Some(i) = index {
            self.scroll_link_into_view(i);
        }
    }

    /// Focuses a checkbox (index into `tasks`); clears the link focus so
    /// only one item is ever focused. The checkbox is scrolled into view.
    pub fn set_focus_task(&mut self, index: Option<usize>) {
        if let Some(i) = index {
            if i >= self.tasks.len() {
                return;
            }
        }
        self.focus_task = index;
        if index.is_some() {
            self.focus_link = None;
        }
        self.restyle_links();
        if let Some(i) = index {
            let line = self.tasks[i].line as u32;
            let viewport = self.viewport_height;
            if line < self.scroll {
                self.scroll = line;
            } else if line >= self.scroll + viewport {
                self.scroll = (line + 1).saturating_sub(viewport);
            }
            self.clamp_scroll();
        }
    }

    /// The rendered checkbox at a rendered-line/column position, if any
    /// (the mouse-click target).
    pub fn task_at(&self, line: usize, col: usize) -> Option<usize> {
        self.tasks
            .iter()
            .position(|t| t.line == line && col >= t.start && col < t.end)
    }

    /// True when the file on disk still has the mtime+size it had when
    /// the current parse was made — the guarded checkbox write's
    /// precondition against racing external edits.
    pub fn disk_matches_parse(&self) -> bool {
        self.stamp
            .is_some_and(|stamp| FileStamp::of(&self.path).is_ok_and(|s| s == stamp))
    }

    /// Test helper: drops the parse stamp so the next poll reloads even
    /// when the filesystem's mtime granularity would swallow the change.
    #[cfg(test)]
    pub fn invalidate_stamp(&mut self) {
        self.stamp = None;
    }

    /// Re-applies per-state styling + the focus highlight, starting from
    /// the pristine engine output (so repeated applications are
    /// idempotent). Covers both links and checkboxes; the parsed lines
    /// carry the styles in place.
    fn restyle_links(&mut self) {
        self.lines = self.base_lines.clone();
        apply_link_styles(
            &mut self.lines,
            &self.links,
            self.focus_link,
            self.link_colors,
        );
        apply_task_styles(
            &mut self.lines,
            &self.tasks,
            self.focus_task,
            self.link_colors,
        );
    }

    fn scroll_link_into_view(&mut self, index: usize) {
        let Some(line) = self
            .links
            .get(index)
            .and_then(|l| l.regions.first().map(|r| r.line))
        else {
            return;
        };
        let line = line as u32;
        let viewport = self.viewport_height;
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + viewport {
            self.scroll = (line + 1).saturating_sub(viewport);
        }
        self.clamp_scroll();
    }

    /// The focused link, if any.
    pub fn focused_link(&self) -> Option<&RenderedLink> {
        self.focus_link.and_then(|i| self.links.get(i))
    }

    /// Best-effort scroll to a `[[note#heading]]` target: the first TOC
    /// entry whose title matches (case-insensitively, unicode-aware). No
    /// match leaves the scroll unchanged — the top-of-note fallback comes
    /// from `open_file`'s scroll reset, which runs before this is called.
    pub fn scroll_to_heading(&mut self, heading: &str) {
        let want = heading.trim().to_lowercase();
        if want.is_empty() {
            return;
        }
        if let Some(entry) = self
            .toc
            .iter()
            .find(|e| e.title.trim().to_lowercase() == want)
        {
            self.set_scroll(entry.line as u32);
        }
    }

    /// Scrolls so the rendered content of raw source line `raw_line`
    /// (0-based, as reported by the global search) is visible, with a
    /// couple of context lines above it.
    ///
    /// Approximate by construction: the engine maps each rendered line to
    /// the 1-based source line it *starts from*, and the first rendered
    /// line whose source line is at or after the target is used. That is
    /// exact for heading/list/table lines, but a hit deep inside a long
    /// wrapped paragraph can land noticeably low — at worst at the first
    /// rendered line of the block *after* the paragraph, which may be
    /// several rendered lines below the match (paragraph-internal lines
    /// share one source-line entry). Exact mid-paragraph positioning would
    /// need mapping through paragraph extents. Past-the-end targets
    /// (e.g. a trailing raw line that renders to nothing) scroll to the
    /// bottom.
    pub fn scroll_to_source_line(&mut self, raw_line: usize) {
        let target = raw_line.saturating_add(1); // engine map is 1-based
        let rendered = self
            .source_line_map
            .iter()
            .position(|&src| src >= target)
            .unwrap_or(self.lines.len().saturating_sub(1));
        self.set_scroll(rendered.saturating_sub(2) as u32);
    }

    /// Re-reads and re-parses the file, preserving the scroll position
    /// (clamped to the new content length).
    pub fn reload(&mut self) {
        match fs::read_to_string(&self.path) {
            Ok(source) => {
                self.source = source;
                self.source_generation += 1;
                self.parse_source();
                self.last_reload = Some(SystemTime::now());
                self.last_error = None;
                self.stamp = FileStamp::of(&self.path).ok();
                self.clamp_scroll();
            }
            Err(err) => {
                // Keep the last good render; retry on the next poll.
                self.last_error = Some(format!("reload failed: {err}"));
            }
        }
    }

    /// Watch tick: reload only if the file's mtime or size changed.
    pub fn poll_file(&mut self) {
        if !self.watch {
            return;
        }
        // File temporarily unreadable (e.g. mid-editor-replace): do nothing.
        if let Ok(stamp) = FileStamp::of(&self.path) {
            if self.stamp != Some(stamp) {
                self.reload();
            }
        }
    }

    /// Call once at startup and on every resize event. Only clamps the
    /// scroll — the render width is fed in from `ui::draw` (which knows the
    /// real content-area width, sidebar included or not) via
    /// `on_content_width`.
    pub fn on_resize(&mut self, _width: u16, _height: u16) {
        self.clamp_scroll();
    }

    /// Single source of truth for the render width: the content area's
    /// width (sidebar excluded), as measured in `ui::draw`. Re-parses when
    /// it changes (e.g. sidebar shown/hidden or a terminal resize).
    /// Does not clamp the scroll: `ui::draw` may set a provisional width
    /// (the full width, before the scrollbar is known) and the clamp must
    /// run against the final wrap, via `on_viewport_height`.
    pub fn on_content_width(&mut self, width: usize) {
        let new_width = width.max(20);
        if new_width != self.render_width {
            self.render_width = new_width;
            self.parse_source();
        }
    }

    /// Single source of truth for the viewport height: the actual frame
    /// area minus the 1-line status bar. Called from `ui::draw` with the
    /// frame height so clamping can never desync from what is rendered.
    pub fn on_viewport_height(&mut self, frame_height: u16) {
        self.viewport_height = u32::from(frame_height.saturating_sub(1)).max(1);
        self.clamp_scroll();
    }

    pub fn max_scroll(&self) -> u32 {
        self.lines
            .len()
            .saturating_sub(self.viewport_height as usize) as u32
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    pub fn scroll_down(&mut self, n: u32) {
        self.scroll = self.scroll.saturating_add(n).min(self.max_scroll());
    }

    pub fn scroll_up(&mut self, n: u32) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    /// Half-page step used by PgUp/PgDn.
    pub fn page_step(&self) -> u32 {
        (self.viewport_height / 2).max(1)
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    /// Sets the scroll position (clamped), used when restoring history
    /// entries whose file may have changed since it was saved.
    pub fn set_scroll(&mut self, value: u32) {
        self.scroll = value.min(self.max_scroll());
    }

    pub fn toggle_watch(&mut self) {
        self.watch = !self.watch;
        // Turning watch back on should immediately pick up any missed change.
        if self.watch {
            self.poll_file();
        }
    }

    /// `f`: show/hide the note's frontmatter metadata block. Purely a
    /// re-render: the vault index still sees the frontmatter, so tags,
    /// status, backlinks and frontmatter wikilinks are unaffected.
    pub fn toggle_metadata(&mut self) {
        self.show_metadata = !self.show_metadata;
        self.parse_source();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempFile;
    use std::fs;

    fn viewer(name: &str, content: &str) -> (TempFile, Viewer) {
        let file = TempFile::new(name, content);
        let viewer = Viewer::open(file.path(), 80).unwrap();
        (file, viewer)
    }

    /// The cached count must equal what the viewer actually renders, and must
    /// follow each input it depends on: the source, the wrap width, and the
    /// metadata setting.
    #[test]
    fn line_count_at_matches_the_render_and_follows_its_inputs() {
        let body = "word ".repeat(40);
        let (file, mut v) = viewer("count.md", &format!("---\nstatus: x\n---\n{body}\n"));
        assert_eq!(
            v.line_count_at(80),
            v.lines.len(),
            "matches the drawn lines"
        );
        assert!(
            v.line_count_at(30) > v.line_count_at(80),
            "narrower wraps to more lines"
        );
        assert_eq!(
            v.line_count_at(80),
            v.lines.len(),
            "cache keeps the width-80 count"
        );

        file.rewrite(&format!("{body}\n\n{body}\n"));
        v.reload();
        assert_eq!(v.line_count_at(80), v.lines.len(), "new source is counted");

        v.toggle_metadata();
        assert_eq!(
            v.line_count_at(80),
            v.lines.len(),
            "metadata toggle is counted"
        );
    }

    #[test]
    fn open_parses_lines() {
        let (_f, v) = viewer("open.md", "# Title\n\nSome *text*.\n");
        assert!(!v.lines.is_empty());
        assert_eq!(v.scroll, 0);
        assert!(v.watch);
        assert!(v.last_reload.is_some());
        assert_eq!(v.max_scroll(), 0);
    }

    #[test]
    fn scroll_clamps_to_content() {
        let body = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let (_f, mut v) = viewer("scroll.md", &body.join("\n"));
        v.viewport_height = 10;
        v.clamp_scroll();
        let total = v.lines.len() as u32;
        let max = v.max_scroll();
        assert_eq!(max, total - 10);
        v.scroll_down(1000);
        assert_eq!(v.scroll, max);
        v.scroll_up(1000);
        assert_eq!(v.scroll, 0);
        v.scroll_bottom();
        assert_eq!(v.scroll, max);
        v.scroll_top();
        assert_eq!(v.scroll, 0);
        v.scroll_down(5);
        v.scroll_up(2);
        assert_eq!(v.scroll, 3);
    }

    #[test]
    fn reload_preserves_scroll_when_line_count_unchanged() {
        let (file, mut v) = viewer("preserve.md", &"para\n".repeat(60));
        v.viewport_height = 10;
        v.scroll_down(30);
        let before = v.lines.len();
        // Same line count on disk, different content; force the stamp stale.
        file.rewrite(&"other\n".repeat(60));
        v.stamp = None;
        v.poll_file();
        assert_eq!(v.lines.len(), before);
        assert_eq!(v.scroll, 30);
        assert!(v.last_error.is_none());
    }

    #[test]
    fn reload_clamps_scroll_when_file_shrinks() {
        let (file, mut v) = viewer("shrink.md", &"para\n".repeat(60));
        v.viewport_height = 10;
        v.scroll_down(30);
        file.rewrite("only\nfew\nlines\n");
        v.stamp = None;
        v.poll_file();
        assert_eq!(v.max_scroll(), 0);
        assert_eq!(v.scroll, 0);
    }

    /// Regression test for the u16-scroll truncation bug: files rendering to
    /// more than `u16::MAX` lines used to wrap `max_scroll` modulo 2^16 and
    /// break scrolling entirely. Uses a synthetic >65536-line file in /tmp.
    #[test]
    fn scroll_does_not_truncate_past_u16_max() {
        // One blank line after each short paragraph → one rendered line each.
        let body = (0..70_000usize)
            .map(|i| format!("line {i}\n\n"))
            .collect::<String>();
        let (file, mut v) = viewer("huge.md", &body);
        assert!(
            v.lines.len() > u16::MAX as usize,
            "fixture must exceed u16 lines"
        );
        v.viewport_height = 10;
        v.clamp_scroll();
        let max = v.max_scroll();
        assert!(max > u16::MAX as u32, "max_scroll must exceed u16::MAX");
        v.scroll_bottom();
        assert_eq!(v.scroll, max);
        v.scroll_down(1000);
        assert_eq!(v.scroll, max, "clamped at bottom");
        v.scroll_top();
        assert_eq!(v.scroll, 0);
        // Keep the temp file alive until the end of the test.
        let _ = &file;
    }

    #[test]
    fn reload_failure_keeps_last_good_render() {
        let (file, mut v) = viewer("gone.md", "# Keep me\n");
        let lines_before = v.lines.len();
        fs::remove_file(file.path()).unwrap();
        v.stamp = None;
        v.reload();
        assert_eq!(v.lines.len(), lines_before);
        assert!(v.last_error.is_some());
        // Once the file is back, polling catches up.
        file.rewrite("# Back\n");
        v.poll_file();
        assert!(v.last_error.is_none());
        assert!(!v.lines.is_empty());
    }

    #[test]
    fn stamp_detects_change() {
        let now = SystemTime::now();
        let a = FileStamp {
            mtime: now,
            len: 10,
        };
        assert_eq!(
            a,
            FileStamp {
                mtime: now,
                len: 10
            }
        );
        assert_ne!(
            a,
            FileStamp {
                mtime: now,
                len: 11
            }
        );
        assert_ne!(
            a,
            FileStamp {
                mtime: now + std::time::Duration::from_secs(1),
                len: 10,
            }
        );
    }

    #[test]
    fn watch_toggle_gates_polling() {
        let (file, mut v) = viewer("toggle.md", "# T\n");
        v.toggle_watch();
        assert!(!v.watch);
        file.rewrite("# Changed\n");
        v.poll_file(); // watch off: must not reload
        assert!(!joined_line(&v.lines[0]).contains("Changed"));
        v.toggle_watch(); // back on: immediate catch-up reload
        assert!(v.watch);
        assert!(joined_line(&v.lines[0]).contains("Changed"));
    }

    fn joined_line(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    // --- M3: wikilink rendering, resolution, focus --------------------

    use std::cell::RefCell;
    use std::rc::Rc;

    use vault::VaultIndex;

    fn link_vault() -> crate::testutil::TempDir {
        let dir = crate::testutil::TempDir::new("viewer-links");
        std::fs::write(
            dir.path().join("target.md"),
            format!("# Target\n\n{}", "target body\n\n".repeat(30)),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("other.md"),
            "# Other\n\n## Section\n\nsection body\n\n",
        )
        .unwrap();
        dir
    }

    fn viewer_with_index(
        source: &str,
    ) -> (
        TempFile,
        Viewer,
        Rc<RefCell<VaultIndex>>,
        crate::testutil::TempDir,
    ) {
        let file = TempFile::new("viewer-src.md", source);
        let vault_dir = link_vault();
        let index = Rc::new(RefCell::new(VaultIndex::build(vault_dir.path()).unwrap()));
        let mut viewer = Viewer::open(file.path(), 80).unwrap();
        viewer.set_index(Some(index.clone()));
        (file, viewer, index, vault_dir)
    }

    #[test]
    fn wikilinks_render_as_links_with_states() {
        use crate::links::LinkState;
        let (_f, v, _i, vault_dir) = viewer_with_index("See [[target]] and [[missing]].\n");
        assert_eq!(v.links.len(), 2, "both wikilinks rendered as links");
        assert_eq!(
            v.links[0].wikilink.as_ref().unwrap().state,
            LinkState::Resolved(vault_dir.path().join("target.md")),
            "target resolves"
        );
        assert!(matches!(
            v.links[1].wikilink.as_ref().unwrap().state,
            LinkState::NotFound
        ));
    }

    #[test]
    fn unresolved_links_are_dimmed_and_crossed_out() {
        let (_f, v, _i, _dir) = viewer_with_index("See [[missing]].\n");
        let link = &v.links[0];
        let region = link.regions[0];
        let line = &v.lines[region.line];
        let styled: Vec<_> = line
            .spans
            .iter()
            .filter(|s| {
                s.style
                    .add_modifier
                    .contains(ratatui::style::Modifier::DIM | ratatui::style::Modifier::CROSSED_OUT)
            })
            .collect();
        assert!(
            !styled.is_empty(),
            "unresolved link text is dimmed and struck through"
        );
    }

    #[test]
    fn link_focus_cycles_and_wraps() {
        let (_f, mut v, _i, _dir) =
            viewer_with_index("[[target]] then [[missing]] then [[target|again]].\n");
        assert_eq!(v.links.len(), 3);
        assert!(v.focus_link.is_none());
        v.focus_next_link();
        assert_eq!(v.focus_link, Some(0));
        v.focus_next_link();
        assert_eq!(v.focus_link, Some(1));
        v.focus_prev_link();
        assert_eq!(v.focus_link, Some(0));
        v.focus_prev_link();
        assert_eq!(v.focus_link, Some(2), "wraps backwards to the last link");
        v.focus_next_link();
        assert_eq!(v.focus_link, Some(0), "wraps forwards past the end");
    }

    #[test]
    fn reparse_after_index_change_updates_states() {
        let (file, mut v, index, _dir) = viewer_with_index("See [[brand-new]].\n");
        use crate::links::LinkState;
        assert!(matches!(
            v.links[0].wikilink.as_ref().unwrap().state,
            LinkState::NotFound
        ));
        // The target note appears on disk and is indexed: a reparse must
        // now resolve the link (same path as the watcher-driven flow).
        let new_note = index.borrow().root().join("brand-new.md");
        std::fs::write(&new_note, "x\n").unwrap();
        index.borrow_mut().reindex_one(&new_note);
        v.parse_source();
        assert!(matches!(
            v.links[0].wikilink.as_ref().unwrap().state,
            LinkState::Resolved(_)
        ));
        let _ = &file;
    }

    #[test]
    fn scroll_to_heading_is_case_insensitive_and_falls_back() {
        let (file, mut v, _i, _dir) = viewer_with_index(
            &(format!("# Top\n\n{}", "filler\n\n".repeat(40))
                + "\n## My Section\n\n"
                + &"body\n\n".repeat(40)),
        );
        v.viewport_height = 10;
        v.scroll_to_heading("my section");
        assert!(v.scroll > 0, "scrolled to the section");
        let before = v.scroll;
        v.scroll_to_heading("no such heading");
        assert_eq!(
            v.scroll, before,
            "unknown heading: scroll unchanged (top fallback applies at open)"
        );
        v.scroll_top();
        v.scroll_to_heading("");
        assert_eq!(v.scroll, 0, "empty heading: no-op");
        let _ = &file;
    }

    /// Heading matching is unicode-aware case-insensitive (to_lowercase),
    /// not just ASCII: accented titles match their lowercased form.
    #[test]
    fn scroll_to_heading_is_unicode_case_insensitive() {
        let (file, mut v, _i, _dir) = viewer_with_index(
            &(format!("# Top\n\n{}", "filler\n\n".repeat(40))
                + "\n## Séction Ünïcode\n\n"
                + &"body\n\n".repeat(40)),
        );
        v.viewport_height = 10;
        v.scroll_to_heading("séction ünïcode");
        assert!(v.scroll > 0, "accented heading matched case-insensitively");
        let _ = &file;
    }

    // --- M4: source-line map + scroll-to-line -------------------------

    /// The engine's source_line_map is 1-based over the raw file (it even
    /// counts frontmatter lines); every rendered line maps to the source
    /// line it starts from.
    #[test]
    fn source_line_map_is_one_based_and_monotonic() {
        let body = "# Title\n\nalpha line\n\nbeta line\n";
        let (_f, v) = viewer("map.md", body);
        assert_eq!(v.source_line_map.len(), v.lines.len());
        for pair in v.source_line_map.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "map must be non-decreasing: {:?}",
                v.source_line_map
            );
        }
        assert!(v.source_line_map[0] >= 1, "1-based, counts frontmatter");
        assert!(
            *v.source_line_map.last().unwrap() <= body.lines().count(),
            "map stays within the raw line count"
        );
    }

    /// Scrolling to a raw search hit lands near it: the rendered line for
    /// a late raw line sits within a small window of the viewport top.
    #[test]
    fn scroll_to_source_line_lands_on_the_hit() {
        let body: String = (0..60).map(|i| format!("line {i}\n\n")).collect();
        let (_f, mut v) = viewer("scroll-line.md", &body);
        v.viewport_height = 10;
        // Each "line N\n\n" is one rendered line; raw line of "line 40" is
        // 2*40+1 = 81 (1-based, after the two leading lines).
        v.scroll_to_source_line(81);
        let rendered = v.source_line_map.iter().position(|&s| s >= 81).unwrap();
        let window: Vec<usize> = (v.scroll as usize..v.scroll as usize + 10).collect();
        assert!(
            window.contains(&rendered),
            "the hit's rendered line ({rendered}) is visible at scroll {}",
            v.scroll
        );
        // Past-the-end raw lines clamp to the bottom without panicking.
        v.scroll_to_source_line(10_000);
        assert_eq!(v.scroll, v.max_scroll());
    }

    /// The x/X focus survives vault-wide reparses (clamped to the new link
    /// list) instead of being silently dropped by an unrelated change.
    #[test]
    fn focus_survives_reparse_and_clamps_to_new_links() {
        let (file, mut v, _i, _dir) =
            viewer_with_index("[[target]] then [[missing]] then [[target|again]].\n");
        v.focus_next_link();
        v.focus_next_link();
        assert_eq!(v.focus_link, Some(1));

        // A vault-wide index change triggers a reparse: focus preserved.
        v.parse_source();
        assert_eq!(v.focus_link, Some(1), "reparse keeps the focused link");

        // The note itself loses links (reload path): focus clamps.
        file.rewrite("See [[target]].\n");
        v.stamp = None;
        v.poll_file();
        assert_eq!(v.links.len(), 1);
        assert_eq!(v.focus_link, Some(0), "clamped to the new link list");

        // All links gone: focus drops.
        file.rewrite("no links here\n");
        v.stamp = None;
        v.poll_file();
        assert_eq!(v.links.len(), 0);
        assert_eq!(v.focus_link, None, "no links: no focus");
    }

    // --- theme switching -----------------------------------------------

    /// A palette swap re-parses: the same heading renders with the new
    /// theme's color (the instant re-render behind Shift+T / hot-reload).
    #[test]
    fn set_markdown_theme_reparses_with_the_new_palette() {
        let (file, mut v) = viewer("theme.md", "# Heading\n\ntext\n");
        let ocean = engine::markdown_theme_by_preset(engine::MarkdownThemePreset::OceanDark);
        assert_eq!(
            v.lines[0].spans[0].style.fg,
            Some(ocean.heading_1),
            "starts on the default palette"
        );
        let solarized =
            engine::markdown_theme_by_preset(engine::MarkdownThemePreset::SolarizedDark);
        assert_ne!(ocean.heading_1, solarized.heading_1);
        v.set_markdown_theme(solarized);
        assert_eq!(v.lines[0].spans[0].style.fg, Some(solarized.heading_1));
        let _ = &file;
    }

    #[test]
    fn syntax_theme_swap_validates_and_resets() {
        let (_f, mut v) = viewer("syntax.md", "```rust\nfn main() {}\n```\n");
        assert!(v.set_syntax_theme("InspiredGitHub"), "known syntect theme");
        assert!(
            !v.set_syntax_theme("no-such-syntax-theme"),
            "unknown name: false, assets untouched"
        );
        v.reset_syntax_theme(); // back to base16-ocean.dark; no panic
    }
}
