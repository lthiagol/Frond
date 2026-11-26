//! Vault-root walker: lists every indexable file — `.md` notes and
//! `.base` table definitions (the bases are path-only entries; the index
//! never parses their content).
//!
//! [`read_dir_entries`] is the one place the vault's listing rules live,
//! shared by [`walk_vault`] and the app's sidebar tree: hidden entries
//! (`.obsidian`, `.git`, …) are skipped, *directory* symlinks are skipped
//! (cycle / outside-vault safety) while file symlinks pointing at
//! `.md`/`.base` files are kept, recursion is depth-capped as
//! belt-and-braces, and folders sort before files, each group in
//! case-insensitive name order.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Recursion cap: directories at this depth are not listed.
pub const MAX_DEPTH: usize = 32;

/// What a listed entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    /// A `.md` note.
    Note,
    /// A `.base` table file.
    Base,
}

/// One entry of a directory listing (see [`read_dir_entries`]).
#[derive(Clone, Debug)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

/// One note or `.base` file found by the walk. Per-entry problems
/// (vanished file, unreadable directory) are skipped, so a bad entry never
/// aborts the walk.
#[derive(Debug)]
pub struct WalkEntry {
    pub path: PathBuf,
}

/// A directory the walk could not descend into. The walk skips it and
/// keeps going; the caller surfaces the problem (the index turns it into
/// an `IndexWarning`) so the notes inside it do not vanish silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkWarning {
    pub path: PathBuf,
    pub message: String,
}

/// Message prefix shared by every warning about a path that could not be
/// read at all (the walk's directories, the index's files). It is the
/// spelling the user reads, not a classification: an index warning's kind
/// says what the warning is about.
pub(crate) const UNREADABLE_PREFIX: &str = "unreadable: ";

/// Walks `root` depth-first collecting `.md` and `.base` files, folders
/// before files, case-insensitive order within each group. Fails only if
/// `root` itself cannot be read; unreadable subdirectories are dropped
/// without a report (use [`walk_vault_with_warnings`] when the caller can
/// surface them).
pub fn walk_vault(root: &Path) -> io::Result<Vec<WalkEntry>> {
    Ok(collect(root)?.entries)
}

/// [`walk_vault`] plus one [`WalkWarning`] per subdirectory that could not
/// be read. Its entries are skipped exactly as before; the warning is what
/// makes the omission visible.
pub fn walk_vault_with_warnings(root: &Path) -> io::Result<WalkReport> {
    collect(root)
}

/// The complete result of one walk: the indexable files and the
/// directories that could not be read.
#[derive(Debug)]
pub struct WalkReport {
    pub entries: Vec<WalkEntry>,
    pub warnings: Vec<WalkWarning>,
}

fn collect(root: &Path) -> io::Result<WalkReport> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    walk_dir(root, 0, &mut entries, &mut warnings)?;
    Ok(WalkReport { entries, warnings })
}

/// The file kind a name denotes: `.md` → note, `.base` → base table
/// (suffix compared ASCII-case-insensitively), anything else `None`.
pub fn classify_file_name(name: &str) -> Option<EntryKind> {
    if ends_with_ignore_case(name, ".md") {
        Some(EntryKind::Note)
    } else if ends_with_ignore_case(name, ".base") {
        Some(EntryKind::Base)
    } else {
        None
    }
}

/// Lists `dir`'s immediate entries under the vault rules (module docs):
/// directories first, then indexable files, each group in
/// case-insensitive name order. Fails only if `dir` itself cannot be
/// read; per-entry errors (vanished file, non-UTF-8 name) skip the entry.
/// At `depth >= MAX_DEPTH` directories are omitted.
pub fn read_dir_entries(dir: &Path, depth: usize) -> io::Result<Vec<DirEntryInfo>> {
    let mut dirs: Vec<DirEntryInfo> = Vec::new();
    let mut files: Vec<DirEntryInfo> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(name) = entry.file_name().into_string() else {
            continue; // non-UTF8 name: skip
        };
        if name.starts_with('.') {
            continue;
        }
        // `symlink_metadata` does not follow the link, so we can tell a
        // symlink apart from a real directory before recursing.
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue; // vanished between read_dir and stat
        };
        if meta.file_type().is_symlink() {
            // File symlinks stay (the real vault has CLAUDE.md →
            // AGENTS.md); directory symlinks are skipped entirely.
            let is_dir = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(true); // broken link: skip
            if !is_dir {
                if let Some(kind) = classify_file_name(&name) {
                    files.push(DirEntryInfo { name, path, kind });
                }
            }
            continue;
        }
        if meta.is_dir() {
            if depth >= MAX_DEPTH {
                continue; // pathological nesting: refuse to recurse further
            }
            dirs.push(DirEntryInfo {
                name,
                path,
                kind: EntryKind::Dir,
            });
        } else if let Some(kind) = classify_file_name(&name) {
            files.push(DirEntryInfo { name, path, kind });
        }
    }
    dirs.sort_by_cached_key(|e| e.name.to_lowercase());
    files.sort_by_cached_key(|e| e.name.to_lowercase());
    dirs.extend(files);
    Ok(dirs)
}

fn walk_dir(
    dir: &Path,
    depth: usize,
    out: &mut Vec<WalkEntry>,
    warnings: &mut Vec<WalkWarning>,
) -> io::Result<()> {
    for entry in read_dir_entries(dir, depth)? {
        match entry.kind {
            // Unreadable subdirectory: skip it, don't abort the walk — but
            // report it so its notes do not vanish without a word.
            EntryKind::Dir => {
                if let Err(err) = walk_dir(&entry.path, depth + 1, out, warnings) {
                    warnings.push(WalkWarning {
                        path: entry.path,
                        message: format!("{UNREADABLE_PREFIX}{err}"),
                    });
                }
            }
            EntryKind::Note | EntryKind::Base => out.push(WalkEntry { path: entry.path }),
        }
    }
    Ok(())
}

/// True when `path` sits under `root` (lexically) and is a `.md` file.
pub fn is_indexable(root: &Path, path: &Path) -> bool {
    let is_md = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| ends_with_ignore_case(n, ".md"));
    is_md && path.strip_prefix(root).is_ok()
}

/// True when `path`'s file name is a `.base` table (case-insensitive).
pub fn is_base_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| ends_with_ignore_case(n, ".base"))
}

/// ASCII-case-insensitive `ends_with` that neither allocates nor can panic
/// on a multi-byte name (compares bytes, not a `str` slice).
pub(crate) fn ends_with_ignore_case(name: &str, suffix: &str) -> bool {
    strip_suffix_ignore_case(name, suffix).is_some()
}

/// ASCII-case-insensitive counterpart of [`str::strip_suffix`]: the
/// suffix-stripping rule shared by the walker, the wikilink parser, the
/// resolver and the app's quick jumper, so a suffix that is *matched*
/// case-insensitively is also *stripped* case-insensitively.
pub fn strip_suffix_ignore_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let (value_len, suffix_len) = (value.len(), suffix.len());
    if value_len >= suffix_len
        && value.as_bytes()[value_len - suffix_len..].eq_ignore_ascii_case(suffix.as_bytes())
    {
        Some(&value[..value_len - suffix_len])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn walk_finds_md_and_base_skips_hidden_and_other_files() {
        let dir = TempDir::new("vk-walk");
        fs::write(dir.path().join("alpha.md"), "a").unwrap();
        fs::write(dir.path().join("tasks.base"), "b").unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        fs::create_dir_all(dir.path().join("Archive")).unwrap();
        fs::write(dir.path().join("Archive").join("old.md"), "o").unwrap();
        fs::write(dir.path().join("Archive").join("board.base"), "b").unwrap();
        fs::create_dir_all(dir.path().join(".obsidian")).unwrap();

        let paths = walk_vault(dir.path()).unwrap();
        let names: Vec<String> = paths
            .iter()
            .map(|e| {
                e.path
                    .strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "Archive/board.base",
                "Archive/old.md",
                "alpha.md",
                "tasks.base"
            ]
        );
    }

    #[test]
    fn walk_skips_dir_symlinks_keeps_file_symlinks() {
        let dir = TempDir::new("vk-walk-sym");
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("b.md"), "b").unwrap();
        std::os::unix::fs::symlink(".", dir.path().join("sub").join("self")).unwrap();
        std::os::unix::fs::symlink("..", dir.path().join("sub").join("up")).unwrap();
        std::os::unix::fs::symlink("a.md", dir.path().join("link.md")).unwrap();

        let paths = walk_vault(dir.path()).unwrap();
        let names: Vec<String> = paths
            .iter()
            .map(|e| {
                e.path
                    .strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["sub/b.md", "a.md", "link.md"]);
    }

    #[test]
    fn walk_is_depth_capped() {
        let dir = TempDir::new("vk-walk-deep");
        let mut path = dir.path().to_path_buf();
        for i in 0..40 {
            path.push(format!("d{i:02}"));
            fs::create_dir_all(&path).unwrap();
        }
        fs::write(path.join("bottom.md"), "b").unwrap();
        assert!(walk_vault(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn walk_reports_an_unreadable_directory_instead_of_dropping_it_silently() {
        let dir = TempDir::new("vk-walk-unreadable");
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::create_dir_all(dir.path().join("locked")).unwrap();
        fs::write(dir.path().join("locked").join("inside.md"), "h").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.path().join("locked");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            let report = walk_vault_with_warnings(dir.path()).unwrap();
            assert_eq!(report.entries.len(), 1, "the readable note is still walked");
            assert!(
                report
                    .warnings
                    .iter()
                    .any(|w| w.path.ends_with("locked") && w.message.contains("unreadable")),
                "unreadable directory reported: {:?}",
                report.warnings
            );
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn is_indexable_checks_suffix_and_containment() {
        let root = Path::new("/v");
        assert!(is_indexable(root, Path::new("/v/a.md")));
        assert!(!is_indexable(root, Path::new("/v/a.txt")));
        assert!(!is_indexable(root, Path::new("/elsewhere/a.md")));
    }
}
