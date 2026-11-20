//! Read-only git sync status for the repository containing the vault.
//!
//! Runs `git status --porcelain -uall` (changed files, clean/dirty;
//! `-uall` so untracked directories are expanded per file instead of
//! collapsing to `?? dir/` — markers must reach notes inside brand-new
//! folders) and `git rev-list --left-right --count <upstream>...HEAD`
//! (ahead/behind; `<upstream>` is the branch's configured upstream via
//! `@{upstream}`, falling back to `origin/main`) **on demand** — the
//! caller decides when (startup, note open, `u` key, debounced after
//! watch-triggered reindexes); nothing here polls or watches.
//!
//! Safety rails:
//! - an explicit allowlist of subcommands guards every spawn (`status`,
//!   `rev-list`, `rev-parse` — all read-only); no write command
//!   (add/commit/push/checkout/…) can ever be executed through this
//!   module, and args are always passed as a fixed argv (no shell).
//! - every failure path (git binary absent, not a repository, no
//!   `origin/main`, malformed output) degrades to `None` — the caller
//!   shows nothing instead of crashing.
//!
//! `changed_files` are absolute paths (repository toplevel joined with
//! the porcelain path), so callers can compare them against note paths
//! directly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The only git subcommands this module may spawn. All of them are
/// read-only; anything else (add/commit/push/checkout/…) is refused.
/// (`diff` was dropped: no call site ever spawned it — a dead allowlist
/// entry invites the assumption that diff output is parsed somewhere.)
const ALLOWED: &[&str] = &["status", "rev-list", "rev-parse"];

/// Sync status of the vault's repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// `true` when the work tree has no modified/staged/untracked files.
    pub clean: bool,
    /// Commits on the current branch that the upstream ref (or the
    /// `origin/main` fallback) does not have.
    pub ahead: u32,
    /// Commits on the upstream ref (or `origin/main` fallback) that the
    /// current branch does not have.
    pub behind: u32,
    /// Every path `git status --porcelain -uall` reports (modified, staged,
    /// untracked — untracked directories expanded per file — and renamed —
    /// both rename sides), as absolute paths.
    pub changed_files: BTreeSet<PathBuf>,
}

/// Reads the sync status of the repository containing `vault_root`.
///
/// `None` on any degradation: git absent, vault outside a work tree,
/// command failure, unparseable output. Never panics, never writes.
pub fn read_status(vault_root: &Path) -> Option<GitStatus> {
    let toplevel = repo_toplevel(vault_root)?;
    let changed_files = changed_files(&toplevel)?;
    let (ahead, behind) = ahead_behind(&toplevel).unwrap_or((0, 0));
    Some(GitStatus {
        clean: changed_files.is_empty(),
        ahead,
        behind,
        changed_files,
    })
}

/// Spawns one allowlisted read-only git subcommand in `dir` and returns
/// its stdout on success. Refuses non-allowlisted subcommands, and any
/// spawn/exit failure yields `None` (git absent, not a repository, …).
fn run_git(dir: &Path, subcommand: &str, args: &[&str]) -> Option<String> {
    if !ALLOWED.contains(&subcommand) {
        // Belt and braces: the allowlist is the contract; a caller (or
        // future refactor) trying to spawn something else gets nothing.
        return None;
    }
    // `--no-optional-locks`: a read-only viewer must never take
    // `.git/index.lock` (plain `git status` does, to refresh the index) —
    // it could make the user's own concurrent `git commit` fail.
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg(subcommand)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?; // git binary absent (or exec failed): silent degrade
    if !output.status.success() {
        return None; // e.g. not a repository (exit 128), unknown revision
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The repository work-tree root containing `dir`, canonicalized so the
/// absolute changed-file paths below agree with canonicalized note paths.
///
/// Public because the guarded checkbox toggle (see `tasks`) needs it to
/// tell "not a git repository" (no write guard possible or needed) apart
/// from "a repository whose status could not be read" (refuse to write).
pub fn repo_toplevel(dir: &Path) -> Option<PathBuf> {
    let out = run_git(dir, "rev-parse", &["--show-toplevel"])?;
    let line = out.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    let path = PathBuf::from(line);
    Some(path.canonicalize().unwrap_or(path))
}

/// Every changed path from `git status --porcelain -uall`, as absolute
/// paths. `-uall` expands untracked directories: without it a wholly
/// untracked folder reports as one `?? dir/` line and none of the notes
/// inside it would get a changed-file marker.
fn changed_files(toplevel: &Path) -> Option<BTreeSet<PathBuf>> {
    let out = run_git(toplevel, "status", &["--porcelain", "-uall"])?;
    let mut files = BTreeSet::new();
    for line in out.lines() {
        // Format: two status letters, one space, then the path (renames:
        // `R  old -> new`). Blank lines cannot occur in porcelain output.
        if line.len() < 4 {
            continue;
        }
        let rest = &line[3..];
        for part in split_rename(rest) {
            if let Some(path) = absolutize_path(toplevel, &part) {
                files.insert(path);
            }
        }
    }
    Some(files)
}

/// Splits a porcelain path field on the rename separator ` -> `, but only
/// where the separator lies *outside* a C-quoted path. Git quotes paths
/// containing spaces, so a filename like `"weird -> name.md"` embeds the
/// separator inside its quotes — splitting there would yield two garbage
/// paths (and unquote-first would not help: unquoting *creates* a bare
/// ` -> ` in the single-path case). Renames quote each side independently,
/// e.g. `R "a -> b.md" -> new.md`.
fn split_rename(field: &str) -> Vec<String> {
    match find_rename_separator(field) {
        Some(idx) => vec![
            field[..idx].to_string(),
            field[idx + " -> ".len()..].to_string(),
        ],
        None => vec![field.to_string()],
    }
}

/// Byte index of the first ` -> ` in `field` that is outside a
/// C-style-quoted section (quote state tracks `\"` escapes inside
/// quotes). `None` when the field is a single path.
fn find_rename_separator(field: &str) -> Option<usize> {
    let bytes = field.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // Inside quotes a backslash escapes the next byte (`\"`, `\\`,
            // octal escapes) — skip both so an escaped quote cannot flip
            // the quote state.
            b'\\' if in_quote => {
                i += 2;
                continue;
            }
            b'"' => in_quote = !in_quote,
            b' ' if !in_quote && field[i..].starts_with(" -> ") => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Joins a (possibly quoted, possibly escaped) repository-relative
/// porcelain path onto the toplevel. Absolute paths pass through.
///
/// A porcelain path is repository-relative whatever its first character:
/// a name starting with `~` is an ordinary file, not a home directory
/// (git expands no paths in its output). Rejecting such a name dropped
/// the entry, so a repository holding `~draft.md` reported clean and the
/// guarded write's git precondition passed on a dirty tree.
fn absolutize_path(toplevel: &Path, raw: &str) -> Option<PathBuf> {
    let unquoted = unquote_c(raw);
    let path = PathBuf::from(&unquoted);
    if path.is_absolute() {
        return Some(path);
    }
    if unquoted.is_empty() {
        return None;
    }
    Some(toplevel.join(path))
}

/// Reverses git's C-style path quoting (`"café.md"` with `\OOO` octal
/// escapes, `\"`, `\\`, `\t`, `\n`). Unquoted input passes through.
fn unquote_c(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') || bytes.len() < 2 {
        return raw.to_string();
    }
    let inner = &bytes[1..bytes.len() - 1];
    let mut out: Vec<u8> = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == b'\\' && i + 1 < inner.len() {
            match inner[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                    continue;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                d @ b'0'..=b'7' => {
                    // Up to three octal digits (git emits exactly three
                    // for non-ASCII bytes, but be tolerant).
                    let mut value: u32 = u32::from(d - b'0');
                    let mut taken = 1;
                    while taken < 3
                        && i + 1 + taken < inner.len()
                        && (b'0'..=b'7').contains(&inner[i + 1 + taken])
                    {
                        value = value * 8 + u32::from(inner[i + 1 + taken] - b'0');
                        taken += 1;
                    }
                    out.push(value as u8);
                    i += 1 + taken;
                    continue;
                }
                _ => {}
            }
        }
        out.push(inner[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(ahead, behind)` of the current branch versus its upstream ref.
/// The upstream is read with `rev-parse --abbrev-ref --symbolic-full-name
/// @{upstream}`; when no upstream is configured (or the read fails) the
/// fallback is the hardcoded `origin/main` — documented assumption,
/// correct for the production vault (`main` ↔ `origin/main`).
/// `(0, 0)`-shaped `None` handling is left to the caller: a missing
/// upstream (or any rev-list failure) simply yields `None`.
fn ahead_behind(toplevel: &Path) -> Option<(u32, u32)> {
    let upstream = upstream_ref(toplevel).unwrap_or_else(|| "origin/main".to_string());
    let out = run_git(
        toplevel,
        "rev-list",
        &["--left-right", "--count", &format!("{upstream}...HEAD")],
    )?;
    let line = out.lines().next()?.trim();
    let (left, right) = line.split_once('\t')?;
    // Left side is the upstream: commits only there = behind.
    Some((right.trim().parse().ok()?, left.trim().parse().ok()?))
}

/// The configured upstream of the current branch (`origin/main`,
/// `origin/master`, …), via `rev-parse --abbrev-ref --symbolic-full-name
/// @{upstream}`. `None` when unset or the command fails. The ref name
/// comes back from git itself and is passed as a rev-list argument, so it
/// is sanitized: anything option-shaped (leading `-`) is rejected.
fn upstream_ref(toplevel: &Path) -> Option<String> {
    let out = run_git(
        toplevel,
        "rev-parse",
        &["--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )?;
    let line = out.lines().next()?.trim();
    if line.is_empty() || line.starts_with('-') {
        return None;
    }
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::process::Command;

    /// Runs a git command in a fixture (fixture setup only — the allowlist
    /// governs what `git.rs` itself may spawn, not how /tmp test repos are
    /// built).
    fn fixture_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("git available for fixtures");
        assert!(
            out.status.success(),
            "fixture git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn init_repo(dir: &Path) {
        fixture_git(dir, &["init", "-b", "main", "-q"]);
    }

    fn commit_all(dir: &Path, message: &str) {
        fixture_git(dir, &["add", "-A"]);
        fixture_git(dir, &["commit", "-q", "-m", message]);
    }

    #[test]
    fn non_repo_directory_is_unavailable() {
        let dir = TempDir::new("git-nonrepo");
        write(&dir.path().join("a.md"), "a\n");
        assert_eq!(read_status(dir.path()), None);
    }

    #[test]
    fn clean_repo_reports_clean_and_synced() {
        let dir = TempDir::new("git-clean");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        let status = read_status(dir.path()).expect("repo readable");
        assert!(status.clean);
        assert!(status.changed_files.is_empty());
        assert_eq!((status.ahead, status.behind), (0, 0));
    }

    #[test]
    fn modified_untracked_and_staged_are_dirty_with_paths() {
        let dir = TempDir::new("git-dirty");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        write(&dir.path().join("sub").join("b.md"), "b\n");
        commit_all(dir.path(), "initial");

        write(&dir.path().join("a.md"), "changed\n"); // modified
        write(&dir.path().join("new.md"), "n\n"); // untracked
        write(&dir.path().join("sub").join("c.md"), "c\n"); // untracked, nested
        fixture_git(dir.path(), &["add", "sub/c.md"]); // staged

        let status = read_status(dir.path()).expect("repo readable");
        assert!(!status.clean);
        assert_eq!(status.changed_files.len(), 3);
        assert!(status.changed_files.contains(&dir.path().join("a.md")));
        assert!(status.changed_files.contains(&dir.path().join("new.md")));
        assert!(status
            .changed_files
            .contains(&dir.path().join("sub").join("c.md")));
    }

    #[test]
    fn tilde_prefixed_paths_are_changed_files_not_dropped() {
        // Porcelain paths are repository-relative: `~draft.md` is an
        // ordinary file. Dropping it made a dirty tree report clean, so
        // the status bar showed `✓` and the guarded write's git
        // precondition (which refuses only when `clean` is false) passed.
        let dir = TempDir::new("git-tilde");
        init_repo(dir.path());
        write(&dir.path().join("~kept.md"), "k\n");
        commit_all(dir.path(), "initial");

        write(&dir.path().join("~kept.md"), "changed\n"); // modified tracked
        write(&dir.path().join("~draft.md"), "d\n"); // untracked

        let status = read_status(dir.path()).expect("repo readable");
        assert!(!status.clean, "a `~`-prefixed change is dirty");
        assert_eq!(
            status.changed_files,
            BTreeSet::from([dir.path().join("~kept.md"), dir.path().join("~draft.md")]),
            "both `~` paths are in changed_files: {:?}",
            status.changed_files
        );
    }

    #[test]
    fn rename_reports_both_sides() {
        let dir = TempDir::new("git-rename");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        fixture_git(dir.path(), &["mv", "a.md", "b.md"]);
        // `git mv` stages the rename; the work tree matches the index.
        let status = read_status(dir.path()).expect("repo readable");
        assert!(status.changed_files.contains(&dir.path().join("a.md")));
        assert!(status.changed_files.contains(&dir.path().join("b.md")));
    }

    #[test]
    fn quoted_non_ascii_paths_are_unescaped() {
        let dir = TempDir::new("git-quoted");
        init_repo(dir.path());
        write(&dir.path().join("base.md"), "b\n");
        commit_all(dir.path(), "initial");
        write(&dir.path().join("café.md"), "c\n"); // untracked, quoted by git
        let status = read_status(dir.path()).expect("repo readable");
        assert!(
            status.changed_files.contains(&dir.path().join("café.md")),
            "octal-escaped porcelain path unescaped: {:?}",
            status.changed_files
        );
    }

    #[test]
    fn ahead_and_behind_versus_origin_main() {
        let dir = TempDir::new("git-ahead");
        let origin = TempDir::new("git-ahead-origin");
        fixture_git(origin.path(), &["init", "-q", "--bare"]);
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "one\n");
        commit_all(dir.path(), "one");
        fixture_git(
            dir.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        fixture_git(
            dir.path(),
            &["push", "-q", "origin", "main:refs/heads/main"],
        );

        // Second commit pushed, then the local branch rewound one commit:
        // behind 1. One new local commit on top: ahead 1 too (diverged).
        write(&dir.path().join("a.md"), "two\n");
        commit_all(dir.path(), "two");
        fixture_git(
            dir.path(),
            &["push", "-q", "origin", "main:refs/heads/main"],
        );
        fixture_git(dir.path(), &["reset", "-q", "--hard", "HEAD~1"]);
        write(&dir.path().join("a.md"), "three\n");
        commit_all(dir.path(), "three");

        let status = read_status(dir.path()).expect("repo readable");
        assert_eq!((status.ahead, status.behind), (1, 1));
        assert!(status.clean, "rewind+recommit leaves a clean tree");
    }

    #[test]
    fn no_origin_main_degrades_to_zero_counts() {
        let dir = TempDir::new("git-noorigin");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        let status = read_status(dir.path()).expect("repo readable");
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert!(status.clean);
    }

    #[test]
    fn vault_subdirectory_of_the_repo_still_works() {
        let dir = TempDir::new("git-subvault");
        let vault = dir.path().join("notes");
        std::fs::create_dir_all(&vault).unwrap();
        init_repo(dir.path());
        write(&vault.join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        write(&vault.join("a.md"), "changed\n");

        let status = read_status(&vault).expect("repo readable");
        assert!(!status.clean);
        assert_eq!(
            status.changed_files,
            BTreeSet::from([vault.join("a.md")]),
            "porcelain paths (repo-relative) resolved against the toplevel"
        );
    }

    /// The allowlist is the hard guard: even with a valid repository, a
    /// write subcommand can never be spawned through `run_git`.
    #[test]
    fn allowlist_refuses_write_subcommands() {
        let dir = TempDir::new("git-allowlist");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        assert_eq!(run_git(dir.path(), "add", &["-A"]), None);
        assert_eq!(run_git(dir.path(), "commit", &["-m", "x"]), None);
        assert_eq!(run_git(dir.path(), "push", &[]), None);
        assert_eq!(run_git(dir.path(), "checkout", &["-b", "x"]), None);
        assert_eq!(run_git(dir.path(), "reset", &["--hard"]), None);
        // Nothing changed on disk (status still clean) — nothing ran.
        let status = read_status(dir.path()).expect("repo readable");
        assert!(status.clean);
        // The read-only subcommands themselves stay allowed.
        assert!(run_git(dir.path(), "status", &["--porcelain"]).is_some());
        // `diff` was dropped from the allowlist (dead entry — nothing
        // ever spawned it), so it is now refused like any other.
        assert_eq!(run_git(dir.path(), "diff", &["--name-only"]), None);
    }

    #[test]
    fn untracked_directory_is_expanded_per_file() {
        // Without `-uall`, porcelain collapses a wholly untracked folder
        // into one `?? dir/` line — no note inside it would ever get a
        // `*` marker. `-uall` must list the individual files.
        let dir = TempDir::new("git-uall");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");

        let newdir = dir.path().join("brand-new");
        std::fs::create_dir_all(&newdir).unwrap();
        write(&newdir.join("n1.md"), "one\n");
        write(&newdir.join("n2.md"), "two\n");

        let status = read_status(dir.path()).expect("repo readable");
        assert!(!status.clean);
        assert_eq!(status.changed_files.len(), 2, "{:?}", status.changed_files);
        assert!(status.changed_files.contains(&newdir.join("n1.md")));
        assert!(status.changed_files.contains(&newdir.join("n2.md")));
    }

    #[test]
    fn quoted_path_containing_the_rename_separator_stays_one_file() {
        // `?? "weird -> name.md"`: git quotes the path (it contains a
        // space), so the field embeds ` -> ` inside its quotes. Splitting
        // on the raw separator yields two garbage paths; the quote-aware
        // split must keep one correct path.
        let dir = TempDir::new("git-weird-name");
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "a\n");
        commit_all(dir.path(), "initial");
        let weird = dir.path().join("weird -> name.md");
        write(&weird, "w\n");

        let status = read_status(dir.path()).expect("repo readable");
        assert_eq!(
            status.changed_files,
            BTreeSet::from([weird]),
            "quoted separator is not a rename: {:?}",
            status.changed_files
        );
    }

    #[test]
    fn rename_with_quoted_old_side_splits_on_the_real_separator() {
        let dir = TempDir::new("git-rename-quoted");
        init_repo(dir.path());
        let weird = dir.path().join("weird -> name.md");
        write(&weird, "a\n");
        commit_all(dir.path(), "initial");
        fixture_git(dir.path(), &["mv", "weird -> name.md", "plain.md"]);

        let status = read_status(dir.path()).expect("repo readable");
        assert!(
            status.changed_files.contains(&weird),
            "quoted old side unquoted correctly: {:?}",
            status.changed_files
        );
        assert!(status.changed_files.contains(&dir.path().join("plain.md")));
        assert_eq!(status.changed_files.len(), 2);
    }

    #[test]
    fn upstream_ref_is_preferred_over_the_origin_main_fallback() {
        // The local branch tracks `origin/master` (no `origin/main`
        // exists): the hardcoded fallback would find nothing and read
        // (0, 0) despite real divergence; `@{upstream}` must be used.
        let dir = TempDir::new("git-upstream");
        let origin = TempDir::new("git-upstream-origin");
        fixture_git(origin.path(), &["init", "-q", "--bare", "-b", "master"]);
        init_repo(dir.path());
        write(&dir.path().join("a.md"), "one\n");
        commit_all(dir.path(), "one");
        fixture_git(
            dir.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        fixture_git(
            dir.path(),
            &["push", "-q", "origin", "main:refs/heads/master"],
        );
        fixture_git(dir.path(), &["fetch", "-q", "origin"]);
        fixture_git(
            dir.path(),
            &["branch", "--set-upstream-to=origin/master", "main"],
        );
        assert_eq!(
            upstream_ref(dir.path()).as_deref(),
            Some("origin/master"),
            "configured upstream read via rev-parse"
        );

        // Diverge: one pushed commit rewound, one new local commit.
        write(&dir.path().join("a.md"), "two\n");
        commit_all(dir.path(), "two");
        fixture_git(
            dir.path(),
            &["push", "-q", "origin", "main:refs/heads/master"],
        );
        fixture_git(dir.path(), &["reset", "-q", "--hard", "HEAD~1"]);
        write(&dir.path().join("a.md"), "three\n");
        commit_all(dir.path(), "three");

        let status = read_status(dir.path()).expect("repo readable");
        assert_eq!(
            (status.ahead, status.behind),
            (1, 1),
            "divergence measured against the configured upstream"
        );
    }

    #[test]
    fn unquote_c_handles_escapes() {
        assert_eq!(unquote_c("plain.md"), "plain.md");
        assert_eq!(unquote_c("\"caf\\303\\251.md\""), "café.md");
        assert_eq!(unquote_c("\"a\\tb\\nc\\\\d\\\"e\""), "a\tb\nc\\d\"e");
        assert_eq!(unquote_c("\""), "\""); // not a quoted pair: verbatim
    }

    #[test]
    fn split_rename_is_quote_aware() {
        // Plain rename.
        assert_eq!(split_rename("a.md -> b.md"), vec!["a.md", "b.md"]);
        // Single path (no rename), quoted, containing the separator.
        assert_eq!(
            split_rename("\"weird -> name.md\""),
            vec!["\"weird -> name.md\""]
        );
        // Rename with a quoted old side embedding the separator.
        assert_eq!(
            split_rename("\"a -> b.md\" -> new.md"),
            vec!["\"a -> b.md\"", "new.md"]
        );
        // Rename where the NEW side is quoted and embeds the separator:
        // the first (unquoted) separator wins.
        assert_eq!(
            split_rename("old.md -> \"new -> name.md\""),
            vec!["old.md", "\"new -> name.md\""]
        );
        // Escaped quote inside the old side must not flip the quote state.
        assert_eq!(
            split_rename("\"a\\\" -> x.md\" -> b.md"),
            vec!["\"a\\\" -> x.md\"", "b.md"]
        );
    }
}
