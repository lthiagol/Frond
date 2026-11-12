//! App-side task (checkbox) support: preprocessing for rendering and the
//! rendered-position mapping that makes checkboxes clickable.
//!
//! The vault crate's [`vault::tasks`] owns the marker *syntax* (scan +
//! guarded write); this module owns the *rendering* bridge:
//!
//! - [`preprocess_tasks`] rewrites the nonstandard in-progress marker
//!   `- [~]` into `- [ ]` on the preprocessed text, so the engine
//!   renders a real checkbox glyph for it (pulldown-cmark only treats
//!   `[ ]`/`[x]`/`[X]` as task markers). The rewrite is same-length and
//!   line-preserving; the raw source is never modified — the guarded
//!   write scans the raw line on disk.
//! - [`map_tasks`] pairs every scanned task with its rendered position:
//!   the first rendered line whose `source_line_map` entry is the task's
//!   raw line, plus the glyph's column range on that line (found by
//!   scanning the line's spans for a checkbox glyph).
//! - [`apply_task_styles`] gives in-progress tasks a visible `◐` glyph
//!   (the same glyph the sidebar uses for in-progress statuses) and
//!   highlights the focused checkbox exactly like a focused link.

use ratatui::text::Line;
use vault::tasks::{TaskHit, TaskState};

use crate::links::FOCUS_BG;

/// A task marker with its rendered position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedTask {
    /// 0-based raw source line of the `- [ ]` marker.
    pub raw_line: usize,
    /// Marker state as it is on disk (the source of truth for toggling).
    pub state: TaskState,
    /// Rendered line index carrying the checkbox glyph.
    pub line: usize,
    /// Glyph's column range on the rendered line (glyph + trailing
    /// space — the click target).
    pub start: usize,
    pub end: usize,
}

/// Rewrites every in-progress marker into an unchecked one on the
/// preprocessed text (rendering only) and returns the rewritten source
/// together with the scanned raw-source task hits.
pub fn preprocess_tasks(source: &str) -> (String, Vec<TaskHit>) {
    let hits = vault::tasks::scan(source);
    let mut out = source.to_string();
    // Byte offset where each raw source line starts (the rewrite is
    // byte-local: the state character at hit.col + 3 is ASCII, so the
    // replacement is always on a char boundary and same-length).
    let mut offset = 0usize;
    let mut next_hit = 0usize;
    for (idx, line) in source.split('\n').enumerate() {
        while next_hit < hits.len() && hits[next_hit].line < idx {
            next_hit += 1;
        }
        if next_hit < hits.len()
            && hits[next_hit].line == idx
            && hits[next_hit].state == TaskState::InProgress
        {
            let at = offset + hits[next_hit].col + 3;
            out.replace_range(at..at + 1, " ");
        }
        offset += line.len() + 1;
    }
    (out, hits)
}

/// Pairs scanned tasks with their rendered positions (see module docs).
/// Tasks the engine did not render as checkbox items (mismatched shapes,
/// ordered-list tasks) are skipped: they stay non-clickable.
pub fn map_tasks(
    hits: &[TaskHit],
    source_line_map: &[usize],
    lines: &[Line<'_>],
) -> Vec<RenderedTask> {
    let mut rendered = Vec::with_capacity(hits.len());
    for hit in hits {
        // The engine's map is 1-based and counts frontmatter lines, so
        // the task's 0-based raw line is `hit.line + 1`.
        let want = hit.line + 1;
        let Some(line_idx) = source_line_map.iter().position(|&src| src == want) else {
            continue;
        };
        let Some((start, end)) = glyph_range(&lines[line_idx]) else {
            continue;
        };
        rendered.push(RenderedTask {
            raw_line: hit.line,
            state: hit.state,
            line: line_idx,
            start,
            end,
        });
    }
    rendered
}

/// Column range of the checkbox glyph in a rendered line: the first span
/// whose content starts with one of the engine's task glyphs. Glyph +
/// one trailing space form the click target.
fn glyph_range(line: &Line<'_>) -> Option<(usize, usize)> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let mut col = 0usize;
    for span in &line.spans {
        let text = span.content.as_ref();
        if let Some(glyph) = text.chars().next() {
            if matches!(glyph, '☐' | '☑' | '☒') {
                let width = glyph.width().unwrap_or(1).max(1);
                return Some((col, col + width + 1));
            }
        }
        col += text.width();
    }
    None
}

/// Restyles the rendered lines for the tasks: in-progress glyphs render
/// as `◐` (distinguishing them from plain unchecked boxes, which would
/// otherwise make the middle cycle state invisible), and the focused
/// checkbox gets the same highlight as a focused link.
pub fn apply_task_styles(
    lines: &mut [Line<'static>],
    tasks: &[RenderedTask],
    focused: Option<usize>,
) {
    use unicode_width::UnicodeWidthStr;
    for (index, task) in tasks.iter().enumerate() {
        let Some(line) = lines.get_mut(task.line) else {
            continue;
        };
        let focused = focused == Some(index);
        let mut col = 0usize;
        for span in &mut line.spans {
            let width = span.content.width();
            let span_end = col + width;
            if span_end > task.start && col < task.end && width > 0 {
                if task.state == TaskState::InProgress && span.content.starts_with('☐') {
                    // Same glyph the sidebar uses for in-progress states.
                    span.content = "◐ ".into();
                }
                if focused {
                    span.style = span
                        .style
                        .bg(FOCUS_BG)
                        .add_modifier(ratatui::style::Modifier::BOLD);
                }
            }
            col = span_end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(source: &str) -> (Vec<Line<'static>>, Vec<RenderedTask>) {
        let (pre, hits) = preprocess_tasks(source);
        let assets = engine::SyntaxAssets::load_defaults();
        let result = engine::parse_markdown_with_width(
            &pre,
            &assets.syntax_set,
            &assets.theme,
            80,
            &engine::default_markdown_theme(),
            false,
            true,
        );
        let tasks = map_tasks(&hits, &result.source_line_map, &result.lines);
        (result.lines, tasks)
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn preprocess_rewrites_only_in_progress_markers() {
        let (out, hits) = preprocess_tasks("- [ ] a\n- [~] b\n- [x] c\n");
        assert_eq!(out, "- [ ] a\n- [ ] b\n- [x] c\n");
        assert_eq!(
            hits.iter().map(|h| h.state).collect::<Vec<_>>(),
            vec![
                TaskState::Unchecked,
                TaskState::InProgress,
                TaskState::Checked
            ]
        );
        assert_eq!(
            out.len(),
            "- [ ] a\n- [~] b\n- [x] c\n".len(),
            "same length"
        );
    }

    #[test]
    fn preprocess_rewrites_indented_and_quoted_markers() {
        let (out, hits) = preprocess_tasks("  - [~] x\n> - [~] y\n");
        assert_eq!(out, "  - [ ] x\n> - [ ] y\n");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn tasks_map_to_their_rendered_glyph_positions() {
        let source = "# Title\n\n- [ ] first\n\n- [~] second\n- [x] third\n";
        let (lines, tasks) = rendered(source);
        assert_eq!(tasks.len(), 3, "all three rendered as checkbox glyphs");
        // Glyphs sit at the start of their item lines (depth 1: col 0).
        for task in &tasks {
            assert_eq!(task.start, 0);
            assert_eq!(task.end, 2, "glyph + trailing space");
            assert!(
                line_text(&lines[task.line]).contains('☐')
                    || line_text(&lines[task.line]).contains('☑')
            );
        }
        // In order, and each on its own rendered line.
        assert!(tasks[0].line < tasks[1].line && tasks[1].line < tasks[2].line);
        assert_eq!(tasks[1].state, TaskState::InProgress);
        assert_eq!(tasks[2].state, TaskState::Checked);
    }

    #[test]
    fn nested_tasks_map_with_indent() {
        let source = "- [ ] outer\n  - [~] inner\n";
        let (_lines, tasks) = rendered(source);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].start, 2, "depth-2 glyph after the 2-col indent");
    }

    /// A deeply indented task-shaped line that pulldown renders as CODE
    /// (verified: 6 spaces under a `- ` item = indented code) must not
    /// become clickable — the glyph-presence gate filters it out.
    #[test]
    fn code_rendered_task_shapes_stay_unclickable() {
        let source = "- [x] parent\n      - [ ] looks nested but is code\n";
        let (lines, tasks) = rendered(source);
        assert_eq!(tasks.len(), 1, "only the real task maps");
        assert!(tasks[0].raw_line == 0);
        assert!(
            !line_text(&lines[tasks[0].line + 1]).contains('☐'),
            "the code line renders without a checkbox glyph"
        );
    }

    #[test]
    fn in_progress_renders_as_the_progress_glyph() {
        let source = "- [~] working\n- [ ] idle\n";
        let (mut lines, tasks) = rendered(source);
        assert!(
            !line_text(&lines[tasks[0].line]).contains('◐'),
            "before styling"
        );
        apply_task_styles(&mut lines, &tasks, None);
        assert!(
            line_text(&lines[tasks[0].line]).contains("◐ "),
            "in-progress glyph visible: {}",
            line_text(&lines[tasks[0].line])
        );
        assert!(
            line_text(&lines[tasks[1].line]).contains("☐ "),
            "unchecked glyph unchanged"
        );
    }

    #[test]
    fn focused_task_gets_the_link_focus_highlight() {
        let source = "- [ ] a\n- [ ] b\n";
        let (mut lines, tasks) = rendered(source);
        apply_task_styles(&mut lines, &tasks, Some(1));
        let spans = &lines[tasks[1].line].spans;
        assert!(
            spans
                .iter()
                .any(|s| s.style.bg == Some(FOCUS_BG) && s.content.starts_with('☐')),
            "focused checkbox highlighted"
        );
        let spans0 = &lines[tasks[0].line].spans;
        assert!(
            !spans0.iter().any(|s| s.style.bg == Some(FOCUS_BG)),
            "the other checkbox is not highlighted"
        );
    }

    #[test]
    fn fenced_tasks_are_neither_rendered_nor_mapped() {
        let source = "```\n- [ ] fenced\n```\n\n- [ ] real\n";
        let (_lines, tasks) = rendered(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].state, TaskState::Unchecked);
    }

    #[test]
    fn blockquoted_tasks_map_too() {
        let source = "> - [ ] quoted task\n";
        let (lines, tasks) = rendered(source);
        assert_eq!(tasks.len(), 1);
        assert!(
            line_text(&lines[tasks[0].line]).contains('☐'),
            "glyph found after the blockquote prefix"
        );
    }
}
