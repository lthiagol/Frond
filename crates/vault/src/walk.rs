//! Vault-root walker: lists every indexable file — `.md` notes and
//! `.base` table definitions (the bases are path-only entries; the index
//! never parses their content).
//!
//! Reuses the sidebar's hard-won lessons (`crates/app/src/sidebar.rs`):
//! hidden entries (`.obsidian`, `.git`, …) are skipped, *directory*
//! symlinks are skipped (cycle / outside-vault safety) while file
//! symlinks pointing at `.md`/`.base` files are kept, and recursion is
//! depth-capped as belt-and-braces.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Recursion cap, same rationale as the sidebar's `MAX_DEPTH`.
const MAX_DEPTH: usize = 32;

/// One note found by the walk. `warnings` carries per-entry problems
/// (unreadable file, non-UTF8 name) so a bad entry never aborts the walk.
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

/// True when a file name is one the index cares about (`.md` or `.base`,
/// case-insensitive).
fn is_indexable_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".base")
}

fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<WalkEntry>) -> io::Result<()> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
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
            if !fs::metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(true) // broken link: skip
                && is_indexable_name(&name)
            {
                files.push(path);
            }
            continue;
        }
        if meta.is_dir() {
            if depth >= MAX_DEPTH {
                continue; // pathological nesting: refuse to recurse further
            }
            dirs.push(path);
        } else if is_indexable_name(&name) {
            files.push(path);
        }
    }
    sort_paths(&mut dirs);
    sort_paths(&mut files);
    for dir in dirs {
        // Unreadable subdirectory: skip it, don't abort the walk.
        let _ = walk_dir(&dir, depth + 1, out);
    }
    out.extend(files.into_iter().map(|path| WalkEntry { path }));
    Ok(())
}

fn sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_lowercase())
            .unwrap_or_default()
    });
}

/// True when `path` sits under `root` (lexically) and is a `.md` file.
pub fn is_indexable(root: &Path, path: &Path) -> bool {
    let is_md = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".md"));
    is_md && path.strip_prefix(root).is_ok()
}

/// True when `path`'s file name is a `.base` table (case-insensitive).
pub fn is_base_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".base"))
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
