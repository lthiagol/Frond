//! Task-list (checkbox) scanning and the guarded toggle write — the app's
//! one and only write feature.
//!
//! Design (orchestrator-approved, post-v1 loop 4): a toggle is a
//! byte-level replacement of the marker state character on the exact raw
//! source line the rendered checkbox came from — everything else in the
//! file stays byte-identical. Additionally the frontmatter `updated:`
//! field is bumped to today (YYYY-MM-DD) **only when the field already
//! exists** (vault convention: all notes have it; an absent field is
//! never added, just reported as not-bumped). The write itself is atomic
//! (temp file + rename in the same directory).
//!
//! Scanning rules (mirroring `wikilink.rs`'s code-block handling):
//! - fenced code blocks (``` / ~~~) are skipped,
//! - the leading `---` frontmatter block is skipped (an unterminated
//!   fence means there is no frontmatter, matching
//!   `frontmatter::split_frontmatter`),
//! - markers are recognized after optional indentation (any depth —
//!   nested list items are real tasks; non-task shapes never become
//!   clickable because the app additionally requires the engine to have
//!   rendered a checkbox glyph for the line), an optional run of `> `
//!   blockquote prefixes, and a `-`/`*`/`+` bullet.
//!
//! Guard decisions (dirty tree, ahead/behind, mtime race) live in the
//! app layer; this module only refuses when the on-disk line no longer
//! carries the expected marker.

use std::fs;
use std::io;
use std::path::Path;

/// The three task states the vault uses. `[~]` (in-progress) is
/// nonstandard markdown — the engine renders it like an unchecked box, so
/// the app's preprocessing rewrites it to `[ ]` for rendering while the
/// on-disk marker stays `[~]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Unchecked,
    InProgress,
    Checked,
}

impl TaskState {
    /// Click/key cycle: `- [ ]` → `- [~]` → `- [x]` → `- [ ]`.
    pub fn next(self) -> TaskState {
        match self {
            TaskState::Unchecked => TaskState::InProgress,
            TaskState::InProgress => TaskState::Checked,
            TaskState::Checked => TaskState::Unchecked,
        }
    }

    /// The marker's state character as written on disk.
    pub fn marker_byte(self) -> u8 {
        match self {
            TaskState::Unchecked => b' ',
            TaskState::InProgress => b'~',
            TaskState::Checked => b'x',
        }
    }

    /// Recognizes a state character (`X` counts as checked, like GFM).
    pub fn from_marker_byte(byte: u8) -> Option<TaskState> {
        match byte {
            b' ' => Some(TaskState::Unchecked),
            b'~' => Some(TaskState::InProgress),
            b'x' | b'X' => Some(TaskState::Checked),
            _ => None,
        }
    }
}

/// One task marker found by [`scan`]: the 0-based raw source line, the
/// byte offset of the bullet within that line, and the marker state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskHit {
    pub line: usize,
    pub col: usize,
    pub state: TaskState,
}

/// Scans a whole note for task markers (document order). See the module
/// docs for the skip rules.
pub fn scan(source: &str) -> Vec<TaskHit> {
    let lines: Vec<&str> = source.split('\n').collect();
    let start = frontmatter_end(&lines);
    let mut hits = Vec::new();
    let mut in_fence: Option<&'static str> = None;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        let trimmed = line.trim_start();
        if let Some(marker) = in_fence {
            if trimmed.starts_with(marker) {
                in_fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = Some("```");
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = Some("~~~");
            continue;
        }
        if let Some((col, state)) = find_marker(line) {
            hits.push(TaskHit {
                line: idx,
                col,
                state,
            });
        }
    }
    hits
}

/// Number of leading lines to skip because they are the frontmatter
/// block (closing-fence index + 1). An unterminated opening fence means
/// there is no frontmatter — the whole file is body (same convention as
/// `frontmatter::split_frontmatter`).
fn frontmatter_end(lines: &[&str]) -> usize {
    let Some(first) = lines.first() else {
        return 0;
    };
    let first = first
        .trim_end_matches(['\r'])
        .trim_start_matches('\u{feff}');
    if first != "---" {
        return 0;
    }
    for (idx, line) in lines.iter().enumerate().skip(1) {
        if line.trim_end_matches(['\r']) == "---" {
            return idx + 1;
        }
    }
    0
}

/// Finds the task marker in one line: returns the byte offset of the
/// bullet character and the marker state. Recognition shape: any leading
/// whitespace, optional `> ` blockquote prefixes, then `-`/`*`/`+`, one
/// space, `[`, a state character, `]`.
///
/// Indentation is deliberately unlimited: nested list items are real
/// tasks, and a task-looking line that is actually rendered as code can
/// never be toggled — the app only wires a click when the engine really
/// rendered a checkbox glyph for that exact raw line.
pub fn find_marker(line: &str) -> Option<(usize, TaskState)> {
    find_marker_bytes(line.as_bytes())
}

fn find_marker_bytes(line: &[u8]) -> Option<(usize, TaskState)> {
    let mut i = 0usize;
    // Leading whitespace of any depth (nested list items); tabs count as
    // one column each.
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    // Optional blockquote prefixes (`>`, possibly several, each
    // optionally followed by one space). Indentation after a quote
    // marker is a fresh line context, so the 3-space cap no longer
    // applies.
    loop {
        while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
            i += 1;
        }
        if i < line.len() && line[i] == b'>' {
            i += 1;
            if i < line.len() && line[i] == b' ' {
                i += 1;
            }
        } else {
            break;
        }
    }
    let bullet = i;
    match line.get(i) {
        Some(b'-') | Some(b'*') | Some(b'+') => i += 1,
        _ => return None,
    }
    if line.get(i) != Some(&b' ') {
        return None;
    }
    i += 1;
    if line.get(i) != Some(&b'[') {
        return None;
    }
    let state = TaskState::from_marker_byte(*line.get(i + 1)?)?;
    if line.get(i + 2) != Some(&b']') {
        return None;
    }
    Some((bullet, state))
}

/// Outcome of a successful toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToggleReport {
    /// Whether the frontmatter `updated:` field existed and was bumped
    /// to `today`. An absent field is never added.
    pub updated_bumped: bool,
}

/// Why a toggle was refused (no bytes touched in every case).
#[derive(Debug)]
pub enum ToggleError {
    /// The requested raw line does not exist in the file.
    OutOfRange,
    /// The line carries no task marker (anymore).
    MarkerNotFound,
    /// The line's marker state differs from what the caller rendered —
    /// the view is stale, so writing would silently overwrite a change.
    StateMismatch { found: TaskState },
    /// Filesystem error (read or atomic write).
    Io(io::Error),
}

impl std::fmt::Display for ToggleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToggleError::OutOfRange => write!(f, "task line no longer exists in the file"),
            ToggleError::MarkerNotFound => write!(f, "no checkbox marker on that line"),
            ToggleError::StateMismatch { found } => {
                write!(f, "checkbox on disk is {found:?}, not the rendered state")
            }
            ToggleError::Io(err) => write!(f, "{err}"),
        }
    }
}

/// Applies a toggle to raw file bytes (pure; no disk access). Returns
/// the new bytes plus a report.
///
/// Re-scans `line` (0-based) on the given bytes for the marker and
/// refuses unless its state is exactly `expected` — the caller rendered
/// `expected`, so a mismatch means the view is stale. Only the single
/// state byte is replaced; every other byte passes through untouched.
/// `today` (YYYY-MM-DD) replaces the frontmatter `updated:` value when
/// that field exists.
pub fn apply_toggle(
    bytes: &[u8],
    line: usize,
    expected: TaskState,
    today: &str,
) -> Result<(Vec<u8>, ToggleReport), ToggleError> {
    let (start, end) = line_range(bytes, line).ok_or(ToggleError::OutOfRange)?;
    let (bullet, found) =
        find_marker_bytes(&bytes[start..end]).ok_or(ToggleError::MarkerNotFound)?;
    if found != expected {
        return Err(ToggleError::StateMismatch { found });
    }
    let mut out = bytes.to_vec();
    // The state character sits at bullet + 3 (`-`, ` `, `[`, state, `]`);
    // all states are single ASCII bytes, so the file length never changes.
    out[start + bullet + 3] = expected.next().marker_byte();
    let updated_bumped = bump_updated(&mut out, today);
    Ok((out, ToggleReport { updated_bumped }))
}

/// The guarded disk write: read → [`apply_toggle`] → atomic replace
/// (temp file in the same directory + rename, original permissions
/// preserved). On any error the original file is untouched.
pub fn write_toggle(
    path: &Path,
    line: usize,
    expected: TaskState,
    today: &str,
) -> Result<ToggleReport, ToggleError> {
    let bytes = fs::read(path).map_err(ToggleError::Io)?;
    // Permissions are captured together with the content read, *before*
    // the temp file exists: if the metadata cannot be read the write is
    // refused outright instead of renaming a temp file with
    // umask-derived (potentially wider) permissions.
    let perms = fs::metadata(path).map_err(ToggleError::Io)?.permissions();
    let (new_bytes, report) = apply_toggle(&bytes, line, expected, today)?;
    atomic_replace(path, &new_bytes, perms).map_err(ToggleError::Io)?;
    Ok(report)
}

/// Writes `bytes` to `path` atomically: temp file (dot-prefixed, so
/// walkers skip it) in the same directory, `perms` applied before the
/// rename, data flushed with `sync_all` before the rename (a crash
/// between rename and flush must not leave a zero-length note), then
/// rename over the target. Cleans up the temp file on failure.
///
/// The temp name carries the pid and a counter, so two frond processes (or
/// two toggles) writing the same note never share a temp file. The parent
/// directory is not fsynced after the rename: for a notes viewer, losing the
/// rename itself in a crash (old content survives intact) is acceptable.
fn atomic_replace(path: &Path, bytes: &[u8], perms: fs::Permissions) -> io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note.md".to_string());
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tmp = parent.join(format!(
        ".{name}.{}-{}.frond-tmp",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&tmp, perms)?;
        fs::rename(&tmp, path)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// Byte range `[start, end)` of the 0-based line `n`'s content
/// (terminator excluded). `None` when the file has fewer lines.
fn line_range(bytes: &[u8], n: usize) -> Option<(usize, usize)> {
    let mut start = 0usize;
    let mut idx = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            if idx == n {
                return Some((start, i));
            }
            idx += 1;
            start = i + 1;
        }
    }
    if idx == n {
        return Some((start, bytes.len()));
    }
    None
}

/// Byte bounds of the line starting at `start`: its content `[start, end)`
/// (terminator excluded) and the start of the following line, when there
/// is one. Callers keep `start <= bytes.len()`.
fn line_bounds(bytes: &[u8], start: usize) -> (usize, Option<usize>) {
    match bytes[start..].iter().position(|&b| b == b'\n') {
        Some(offset) => (start + offset, Some(start + offset + 1)),
        None => (bytes.len(), None),
    }
}

/// True when the line is a frontmatter fence. The frontmatter parser trims
/// every trailing `\r` before comparing (`\r` is a CRLF terminator, or a
/// repeated one), so `---\r` and `---\r\r` are fences too. The caller
/// strips a leading UTF-8 BOM from the file's first line, exactly as
/// `frontmatter::split_frontmatter` does.
fn is_fence(line: &[u8]) -> bool {
    let mut end = line.len();
    while end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end] == b"---"
}

/// Content byte ranges of the lines inside the leading frontmatter block
/// (opening and closing fences excluded, in source order). Empty when the
/// file does not open with a closed `---` fence — an unterminated fence
/// means there is no frontmatter at all, matching `scan` and
/// `frontmatter::split_frontmatter`.
///
/// The walk continues from the previous line's end and stops at the
/// closing fence, so a note's body is never examined: this runs on the
/// checkbox toggle path, which is the single-threaded UI thread.
fn frontmatter_lines(bytes: &[u8]) -> Vec<(usize, usize)> {
    let (first_end, mut next) = line_bounds(bytes, 0);
    let first = bytes[..first_end]
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(&bytes[..first_end]);
    if !is_fence(first) {
        return Vec::new();
    }
    let mut lines = Vec::new();
    while let Some(start) = next {
        let (end, following) = line_bounds(bytes, start);
        if is_fence(&bytes[start..end]) {
            return lines;
        }
        lines.push((start, end));
        next = following;
    }
    Vec::new()
}

/// Byte range of the frontmatter `updated:` line (content only), if the
/// file opens with a closed `---` fence and carries the field. The last
/// occurrence wins — matching the frontmatter parser's duplicate-key
/// semantics.
fn updated_field_line(bytes: &[u8]) -> Option<(usize, usize)> {
    let lines = frontmatter_lines(bytes);
    let idx = lines
        .iter()
        .rposition(|&(start, end)| bytes[start..end].starts_with(b"updated:"))?;
    Some(lines[idx])
}

/// Replaces the value of the frontmatter `updated:` line with `today`.
/// The `updated:` key, the whitespace after the colon, a trailing
/// comment (` #` or `<tab>#`), and the line terminator (including a
/// CRLF's `\r`) are all preserved; only the value token is replaced.
/// Returns `false` (and touches nothing) when the field does not exist.
fn bump_updated(bytes: &mut Vec<u8>, today: &str) -> bool {
    let Some((start, end)) = updated_field_line(bytes) else {
        return false;
    };
    let line = &bytes[start..end];
    let colon = match line.iter().position(|&b| b == b':') {
        Some(idx) => idx,
        None => return false,
    };
    let mut value_start = colon + 1;
    while value_start < line.len() && (line[value_start] == b' ' || line[value_start] == b'\t') {
        value_start += 1;
    }
    // A trailing `\r` is part of the line's CRLF terminator, not the
    // value: keep it out of the replaced span so a CRLF-terminated
    // `updated:` line stays CRLF-terminated (no mixed EOLs).
    let content_end = match line.last() {
        Some(b'\r') => line.len() - 1,
        _ => line.len(),
    };
    // A trailing comment (whitespace — space or tab — then `#`) stays;
    // only the value token is replaced.
    let value_end = line[value_start..content_end]
        .windows(2)
        .position(|w| w == b" #" || w == b"\t#")
        .map(|idx| value_start + idx)
        .unwrap_or(content_end);
    let mut new_line = line[..value_start].to_vec();
    new_line.extend_from_slice(today.as_bytes());
    new_line.extend_from_slice(&line[value_end..]);
    bytes.splice(start..end, new_line);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn hits(source: &str) -> Vec<(usize, TaskState)> {
        scan(source)
            .into_iter()
            .map(|h| (h.line, h.state))
            .collect()
    }

    #[test]
    fn next_state_cycles_through_all_three() {
        assert_eq!(TaskState::Unchecked.next(), TaskState::InProgress);
        assert_eq!(TaskState::InProgress.next(), TaskState::Checked);
        assert_eq!(TaskState::Checked.next(), TaskState::Unchecked);
    }

    #[test]
    fn scan_finds_all_three_states_and_bullets() {
        let src = "- [ ] a\n* [~] b\n+ [x] c\n- [X] d\n";
        assert_eq!(
            hits(src),
            vec![
                (0, TaskState::Unchecked),
                (1, TaskState::InProgress),
                (2, TaskState::Checked),
                (3, TaskState::Checked),
            ]
        );
    }

    #[test]
    fn scan_skips_fences_and_frontmatter() {
        let src = "---\nupdated: x\n- [ ] in frontmatter\n---\n\n- [ ] real\n```rust\n- [ ] fenced\n```\n~~~\n- [ ] tilde\n~~~\n- [x] last\n";
        assert_eq!(
            hits(src),
            vec![(5, TaskState::Unchecked), (12, TaskState::Checked)]
        );
    }

    #[test]
    fn scan_unterminated_frontmatter_means_no_frontmatter() {
        // Same convention as split_frontmatter: the whole file is body.
        let src = "---\nstatus: active\nno close\n- [ ] real task\n";
        assert_eq!(hits(src), vec![(3, TaskState::Unchecked)]);
    }

    #[test]
    fn scan_handles_blockquote_prefixes() {
        let src = "> - [ ] quoted\n> > - [~] nested quote\n- [x] plain\n";
        assert_eq!(
            hits(src),
            vec![
                (0, TaskState::Unchecked),
                (1, TaskState::InProgress),
                (2, TaskState::Checked),
            ]
        );
    }

    #[test]
    fn find_marker_rejects_non_tasks() {
        assert_eq!(find_marker("- [ ] ok"), Some((0, TaskState::Unchecked)));
        assert_eq!(
            find_marker("  - [~] indented"),
            Some((2, TaskState::InProgress))
        );
        assert_eq!(find_marker("text - [x] mid-line"), None);
        assert_eq!(find_marker("- [y] unknown state"), None);
        assert_eq!(find_marker("-[x] no space"), None);
        assert_eq!(find_marker("- [] empty state"), None);
        assert_eq!(find_marker("- [x"), None);
        // Any indentation is accepted (nested list items are real tasks);
        // a task-shaped line inside an indented code block is detected
        // here but can never be toggled — the app only wires clicks to
        // lines the engine actually rendered a checkbox glyph for.
        assert_eq!(
            find_marker("      - [ ] deep nested"),
            Some((6, TaskState::Unchecked))
        );
        assert_eq!(find_marker(""), None);
    }

    #[test]
    fn scan_line_indices_survive_crlf() {
        let src = "- [ ] a\r\n- [x] b\r\n";
        assert_eq!(
            hits(src),
            vec![(0, TaskState::Unchecked), (1, TaskState::Checked)]
        );
    }

    #[test]
    fn apply_toggle_replaces_only_the_state_byte() {
        let src = b"# Title\n\n- [ ] do a thing\n\nbody\n";
        let (out, report) = apply_toggle(src, 2, TaskState::Unchecked, "2026-11-11").unwrap();
        assert_eq!(
            report,
            ToggleReport {
                updated_bumped: false
            }
        );
        assert_eq!(
            std::str::from_utf8(&out).unwrap(),
            "# Title\n\n- [~] do a thing\n\nbody\n"
        );
    }

    #[test]
    fn apply_toggle_bumps_existing_updated_field() {
        let src =
            b"---\nstatus: active\ncreated: 2026-01-01\nupdated: 2026-10-23\n---\n\n- [x] task\n";
        let (out, report) = apply_toggle(src, 6, TaskState::Checked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(text.contains("updated: 2026-11-11\n"));
        assert!(
            text.contains("created: 2026-01-01\n"),
            "other fields intact"
        );
        assert!(text.contains("- [ ] task\n"), "checked cycles to unchecked");
    }

    #[test]
    fn apply_toggle_leaves_missing_updated_field_absent() {
        let src = b"---\nstatus: active\n---\n\n- [ ] task\n";
        let (out, report) = apply_toggle(src, 4, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(!report.updated_bumped, "absent field is not added");
        assert!(!std::str::from_utf8(&out).unwrap().contains("updated"));
    }

    #[test]
    fn apply_toggle_preserves_comment_and_terminator_on_the_updated_line() {
        let src = b"---\nupdated: 2026-10-23 # vault rule\n---\r\n- [ ] task\r\n";
        let (out, _) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        let text = std::str::from_utf8(&out).unwrap();
        assert!(text.contains("updated: 2026-11-11 # vault rule\n"));
        assert!(text.contains("- [~] task\r\n"), "CRLF preserved");
    }

    #[test]
    fn apply_toggle_preserves_crlf_on_the_updated_line_itself() {
        // Reviewer F2 exact combination: the `updated:` line is
        // CRLF-terminated and carries no comment — without the fix the
        // `\r` fell inside the replaced span and the line came out LF.
        let src = b"---\nupdated: 2026-10-23\r\n---\r\n- [ ] task\r\n";
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        assert!(
            std::str::from_utf8(&out)
                .unwrap()
                .contains("updated: 2026-11-11\r\n"),
            "CRLF on the updated line preserved"
        );
        assert!(
            !std::str::from_utf8(&out).unwrap().contains("\r\r"),
            "no doubled CR"
        );
    }

    #[test]
    fn apply_toggle_bumps_the_field_in_a_fully_crlf_note() {
        // The fences are `---\r` here; they are CRLF terminators, not
        // content, so this is a real frontmatter block (the parser and
        // `scan` trim the `\r` the same way).
        let src = b"---\r\nupdated: 2026-01-01\r\n---\r\n- [ ] t\r\n";
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(text.contains("updated: 2026-11-11\r\n"));
        assert!(!text.contains("\r\r"), "no doubled CR");
    }

    #[test]
    fn apply_toggle_bumps_updated_in_a_bom_prefixed_note() {
        // Audit carry-over: split_frontmatter and scan strip a UTF-8 BOM,
        // so the BOM-led fence is real frontmatter; the bump walk must
        // agree instead of leaving `updated:` untouched.
        let src = "\u{feff}---\nupdated: 2026-01-01\n---\n- [ ] t\n".as_bytes();
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(
            report.updated_bumped,
            "the BOM must not hide the frontmatter"
        );
        assert!(std::str::from_utf8(&out)
            .unwrap()
            .contains("updated: 2026-11-11\n"));
    }

    #[test]
    fn apply_toggle_bumps_updated_with_repeated_cr_fences() {
        // The parser trims every trailing `\r`, so `---\r\r` is a fence;
        // the bump walk must use the same rule.
        let src = b"---\r\r\nupdated: 2026-01-01\r\n---\r\r\n- [ ] t\r\n";
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        assert!(std::str::from_utf8(&out)
            .unwrap()
            .contains("updated: 2026-11-11\r\n"));
    }

    #[test]
    fn apply_toggle_preserves_a_tab_before_the_comment() {
        // Reviewer F3: the comment scan matched ` #` only, so a
        // tab-before-`#` comment was replaced along with the value.
        let src = b"---\nupdated: 2026-01-01\t# note\n---\n- [ ] t\n";
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains("updated: 2026-11-11\t# note\n"),
            "tab-form comment preserved"
        );
    }

    #[test]
    fn apply_toggle_bumps_last_duplicate_and_leaves_earlier_ones() {
        let src = b"---\nupdated: YYYY-MM-DD\nupdated: 2026-01-01\n---\n- [ ] t\n";
        let (out, report) = apply_toggle(src, 4, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        let text = std::str::from_utf8(&out).unwrap();
        // Last occurrence wins (matches the frontmatter parser); the
        // earlier one stays untouched.
        assert!(text.contains("updated: YYYY-MM-DD\n"));
        assert!(text.contains("updated: 2026-11-11\n"));
    }

    #[test]
    fn frontmatter_walk_stops_at_the_closing_fence() {
        // Audit B3: the old walk indexed every line of the note by
        // rescanning from byte 0 (O(lines × bytes)). The frontmatter is at
        // the top, so the walk stops at the closing fence: even a 50k-line
        // body whose every line is shaped like the field adds no range.
        let mut src = String::from("---\nupdated: 2026-01-01\n---\n");
        for _ in 0..50_000 {
            src.push_str("updated: body line\n");
        }
        assert_eq!(
            frontmatter_lines(src.as_bytes()),
            vec![(4, 23)],
            "only the frontmatter line is walked"
        );
        assert_eq!(
            updated_field_line(src.as_bytes()),
            Some((4, 23)),
            "a body line is not a frontmatter field"
        );
    }

    #[test]
    fn no_frontmatter_never_walks_the_body() {
        // The first line decides: without an opening fence there is no
        // frontmatter, so nothing below it is examined.
        let mut src = String::from("# Title\nupdated: not frontmatter\n");
        for _ in 0..50_000 {
            src.push_str("- [ ] line\n");
        }
        assert!(frontmatter_lines(src.as_bytes()).is_empty());
        assert_eq!(updated_field_line(src.as_bytes()), None);
    }

    #[test]
    fn an_unterminated_fence_is_no_frontmatter_so_nothing_is_bumped() {
        // Same convention as `scan` and `frontmatter::split_frontmatter`:
        // without a closing `---` the whole file is body, so a body line
        // that happens to start with `updated:` is not a field.
        let src = b"---\nstatus: active\nupdated: 2026-01-01\n- [ ] task\n";
        let (out, report) = apply_toggle(src, 3, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(!report.updated_bumped);
        assert!(std::str::from_utf8(&out)
            .unwrap()
            .contains("updated: 2026-01-01\n"));
    }

    #[test]
    fn toggle_on_a_large_note_is_bounded() {
        // Audit B3 measured the old quadratic index at ~24 s for 50k lines
        // (release build) on the single-threaded event loop. A linear pass
        // is milliseconds even in a debug test build; the bound is
        // generous against a loaded machine and still an order of
        // magnitude below the old shape.
        let target = 50_000usize;
        let mut src = String::from("---\nupdated: 2026-01-01\n---\n");
        for i in 0..target {
            if i + 1 == target {
                src.push_str("- [ ] last\n");
            } else {
                src.push_str("- [ ] line\n");
            }
        }
        let bytes = src.into_bytes();
        let started = std::time::Instant::now();
        let (out, report) =
            apply_toggle(&bytes, 3 + target - 1, TaskState::Unchecked, "2026-11-11")
                .expect("toggle the last line");
        let elapsed = started.elapsed();
        assert!(report.updated_bumped);
        assert!(std::str::from_utf8(&out).unwrap().ends_with("- [~] last\n"));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "50k-line toggle took {elapsed:?} (the old quadratic scan took ~24 s)"
        );
    }

    #[test]
    fn apply_toggle_is_byte_preserving_outside_the_two_edits() {
        // Bytes that are not part of either edit must survive verbatim
        // (binary junk around the edits).
        let src = b"\xff\xfe junk\n- [ ] task\n\x00binary\x00\n";
        let (out, _) = apply_toggle(src, 1, TaskState::Unchecked, "2026-11-11").unwrap();
        assert_eq!(&out[..8], b"\xff\xfe junk\n");
        assert_eq!(&out[out.len() - 9..], b"\x00binary\x00\n");
        assert_eq!(&out[8..12], b"- [~");
    }

    #[test]
    fn apply_toggle_refusals() {
        let src = b"# T\n\n- [ ] a\n";
        assert!(matches!(
            apply_toggle(src, 99, TaskState::Unchecked, "d"),
            Err(ToggleError::OutOfRange)
        ));
        assert!(matches!(
            apply_toggle(b"# no tasks\n", 0, TaskState::Unchecked, "d"),
            Err(ToggleError::MarkerNotFound)
        ));
        // Rendered state no longer matches the disk state.
        assert!(matches!(
            apply_toggle(src, 2, TaskState::Checked, "d"),
            Err(ToggleError::StateMismatch {
                found: TaskState::Unchecked
            })
        ));
    }

    #[test]
    fn write_toggle_roundtrips_through_disk_and_keeps_permissions() {
        let dir = TempDir::new("tasks-write");
        let path = dir.path().join("note.md");
        std::fs::write(&path, "---\nupdated: 2026-01-01\n---\n\n- [ ] task\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let report = write_toggle(&path, 4, TaskState::Unchecked, "2026-11-11").unwrap();
        assert!(report.updated_bumped);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("- [~] task\n"));
        assert!(text.contains("updated: 2026-11-11\n"));
        // The temp file is gone after the rename.
        assert_no_temp_files(dir.path());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "original permissions preserved");
        }
    }

    #[test]
    fn write_toggle_refusal_leaves_the_file_untouched() {
        let dir = TempDir::new("tasks-refuse");
        let path = dir.path().join("note.md");
        std::fs::write(&path, "- [x] task\n").unwrap();
        // The cycle from Checked is Unchecked, but the caller claims the
        // rendered state was Unchecked: mismatch, no write.
        assert!(matches!(
            write_toggle(&path, 0, TaskState::Unchecked, "2026-11-11"),
            Err(ToggleError::StateMismatch {
                found: TaskState::Checked
            })
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "- [x] task\n");
        assert_no_temp_files(dir.path());
    }

    #[test]
    fn atomic_replace_failure_cleans_up_its_temp_file() {
        let dir = TempDir::new("tasks-atomic-fail");
        // A non-empty directory at the target: the final rename must fail.
        let target = dir.path().join("note.md");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("child"), "x").unwrap();
        let perms = std::fs::metadata(dir.path()).unwrap().permissions();
        assert!(atomic_replace(&target, b"new", perms).is_err());
        assert_no_temp_files(dir.path());
        assert!(target.join("child").exists(), "target left untouched");
    }

    #[test]
    fn concurrent_writers_never_share_a_temp_file() {
        let dir = TempDir::new("tasks-atomic-race");
        let path = dir.path().join("note.md");
        std::fs::write(&path, "seed").unwrap();
        let perms = std::fs::metadata(&path).unwrap().permissions();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (path, perms) = (path.clone(), perms.clone());
                std::thread::spawn(move || {
                    atomic_replace(&path, format!("writer-{i}").as_bytes(), perms)
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().expect("every writer succeeds");
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("writer-"), "a whole write won: {text:?}");
        assert_no_temp_files(dir.path());
    }

    fn assert_no_temp_files(dir: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".frond-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
    }
}
