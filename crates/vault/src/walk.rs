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
/// (unreadable directory, vanished file) are skipped, so a bad entry never
/// aborts the walk.
#[derive(Debug)]
pub struct WalkEntry {
    pub path: PathBuf,
}

/// Walks `root` depth-first collecting `.md` and `.base` files, folders
/// before files, case-insensitive order within each group. Fails only if
/// `root` itself cannot be read.
pub fn walk_vault(root: &Path) -> io::Result<Vec<WalkEntry>> {
    let mut out = Vec::new();
    walk_dir(root, 0, &mut out)?;
    Ok(out)
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

fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<WalkEntry>) -> io::Result<()> {
    for entry in read_dir_entries(dir, depth)? {
        match entry.kind {
            // Unreadable subdirectory: skip it, don't abort the walk.
            EntryKind::Dir => {
                let _ = walk_dir(&entry.path, depth + 1, out);
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
    let (name, suffix) = (name.as_bytes(), suffix.as_bytes());
    name.len() >= suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
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
    fn is_indexable_checks_suffix_and_containment() {
        let root = Path::new("/v");
        assert!(is_indexable(root, Path::new("/v/a.md")));
        assert!(!is_indexable(root, Path::new("/v/a.txt")));
        assert!(!is_indexable(root, Path::new("/elsewhere/a.md")));
    }
}
