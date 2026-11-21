//! Vault-wide change detection for the poll-based watcher.
//!
//! Every vault-wide poll walks the index's root (cheap: hidden entries and
//! directory symlinks are skipped, `vault::walk_vault_with_warnings` — it
//! lists `.base` files alongside `.md` notes and reports directories it
//! could not read), stats each file (mtime + size, same fingerprint as the
//! single-file watcher) and feeds every added/modified/removed file into
//! [`VaultIndex::reindex_one`]. The watcher itself holds no index state —
//! the index owns the data, the watcher only decides *when* to refresh.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use vault::walk::{walk_vault_with_warnings, WalkEntry};
use vault::{IndexWarning, VaultIndex};

use crate::viewer::FileStamp;

/// One poll's outcome: the paths whose index entries changed, plus the
/// walk's warnings (directories that could not be read this tick). The
/// caller must surface the warnings — a directory dropping out of the
/// index is never silent.
#[derive(Debug, Default)]
pub struct PollOutcome {
    pub changed: Vec<PathBuf>,
    pub warnings: Vec<IndexWarning>,
}

/// Fingerprint of every currently-indexed note.
pub struct VaultWatcher {
    stamps: BTreeMap<PathBuf, FileStamp>,
}

impl VaultWatcher {
    /// Seeds the fingerprint map from the index's own note and base lists
    /// (not a second disk walk): a file created between
    /// `VaultIndex::build` and this seed is then absent from `stamps`, so
    /// the first poll reports it as an add instead of silently matching
    /// an equal stamp.
    pub fn new(index: &VaultIndex) -> Self {
        let mut stamps = BTreeMap::new();
        for note in index.notes() {
            if let Ok(stamp) = FileStamp::of(&note.path) {
                stamps.insert(note.path.clone(), stamp);
            }
        }
        for base in index.bases() {
            if let Ok(stamp) = FileStamp::of(base) {
                stamps.insert(base.to_path_buf(), stamp);
            }
        }
        Self { stamps }
    }

    /// One vault-wide poll: reindexes every changed/new/removed file and
    /// returns the changed paths plus the walk's warnings (empty
    /// `changed` = nothing happened). A root that cannot be walked at all
    /// is skipped entirely (retry next tick).
    pub fn poll(&mut self, index: &mut VaultIndex) -> PollOutcome {
        let Ok(report) = walk_vault_with_warnings(index.root()) else {
            return PollOutcome::default();
        };
        PollOutcome {
            changed: self.reconcile(index, &report.entries),
            warnings: report
                .warnings
                .into_iter()
                .map(IndexWarning::from)
                .collect(),
        }
    }

    /// Compares one walk's results against the stamps and feeds every
    /// difference into `reindex_one`. Split from [`Self::poll`] so tests
    /// can replay a walk whose files became unstatable afterwards (the
    /// transient-stat-failure case, see the `Err` arm below).
    fn reconcile(&mut self, index: &mut VaultIndex, entries: &[WalkEntry]) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        // Every path the walk still reports, stat success or not: a path
        // in here must never count as removed.
        let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
        for entry in entries {
            seen.insert(entry.path.clone());
            let Ok(stamp) = FileStamp::of(&entry.path) else {
                // Unreadable right now (e.g. mid-replace): NOT a removal.
                // Keep the old stamp and index entry untouched; only the
                // stamp comparison is skipped this tick.
                continue;
            };
            if self.stamps.get(&entry.path) != Some(&stamp) {
                self.stamps.insert(entry.path.clone(), stamp);
                changed.push(entry.path.clone());
            }
        }
        let removed: Vec<PathBuf> = self
            .stamps
            .keys()
            .filter(|path| !seen.contains(*path))
            .cloned()
            .collect();
        for path in removed {
            self.stamps.remove(&path);
            changed.push(path);
        }
        index.reindex_many(&changed);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn wait_mtime_tick() {
        // Ensure mtime+size actually differ across writes.
        thread::sleep(Duration::from_millis(20));
    }

    #[test]
    fn detects_add_modify_and_remove() {
        let dir = TempDir::new("watch-vault");
        write(&dir.path().join("a.md"), "---\nstatus: active\n---\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);
        assert_eq!(index.len(), 1);

        // ADD: a new note appears.
        wait_mtime_tick();
        write(&dir.path().join("b.md"), "---\nstatus: draft\n---\n");
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![dir.path().join("b.md")]);
        assert_eq!(index.len(), 2);

        // MODIFY: content change (same length) is caught by mtime.
        wait_mtime_tick();
        write(&dir.path().join("a.md"), "---\nstatus: resolved\n---\n");
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![dir.path().join("a.md")]);
        assert_eq!(
            index
                .note(&dir.path().join("a.md"))
                .unwrap()
                .status
                .as_deref(),
            Some("resolved")
        );

        // REMOVE: deleted file drops out of the index.
        wait_mtime_tick();
        fs::remove_file(dir.path().join("b.md")).unwrap();
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![dir.path().join("b.md")]);
        assert_eq!(index.len(), 1);

        // Quiet tick: no changes.
        assert!(watcher.poll(&mut index).changed.is_empty());
    }

    #[test]
    fn detects_base_add_and_remove() {
        let dir = TempDir::new("watch-base");
        write(&dir.path().join("a.md"), "a\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);
        assert!(watcher.poll(&mut index).changed.is_empty());

        // ADD: a new `.base` file is picked up (path-only in the index).
        wait_mtime_tick();
        let base = dir.path().join("tasks.base");
        write(&base, "filters: []\n");
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![base.clone()]);
        assert_eq!(index.bases().collect::<Vec<_>>(), vec![base.clone()]);

        // MODIFY: content changes re-feed the path; the listing stays.
        wait_mtime_tick();
        write(&base, "filters:\n  and: []\n");
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![base.clone()]);
        assert_eq!(index.bases().count(), 1);

        // REMOVE: the base drops out.
        wait_mtime_tick();
        fs::remove_file(&base).unwrap();
        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![base]);
        assert!(index.bases().next().is_none());
    }

    #[test]
    fn new_note_links_are_picked_up_by_the_backlink_map() {
        let dir = TempDir::new("watch-links");
        write(&dir.path().join("target.md"), "t\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);

        wait_mtime_tick();
        write(
            &dir.path().join("source.md"),
            "---\nstatus: active\n---\n[[target]]\n",
        );
        watcher.poll(&mut index);
        assert_eq!(
            index.backlinks_to(&dir.path().join("target.md")),
            vec![dir.path().join("source.md")]
        );
    }

    /// A transient stat failure must NOT count as a removal: the walk
    /// still reported the path, so only its stamp comparison is skipped
    /// and the index entry (with its backlinks) stays put.
    #[test]
    fn transient_stat_failure_is_not_a_removal() {
        let dir = TempDir::new("watch-statfail");
        write(&dir.path().join("a.md"), "---\nstatus: active\n---\n");
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        write(&dir.path().join("sub").join("secret.md"), "s\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        assert_eq!(index.len(), 2);
        let mut watcher = VaultWatcher::new(&index);

        // Simulate "stat failed after the walk listed it": capture the
        // walk's entries, then make the parent directory unsearchable so
        // `FileStamp::of` fails with EACCES while the entry list still
        // contains the path.
        let entries = walk_vault_with_warnings(dir.path()).unwrap().entries;
        assert_eq!(entries.len(), 2);
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path().join("sub"), fs::Permissions::from_mode(0o000)).unwrap();
        }
        let changed = watcher.reconcile(&mut index, &entries);
        assert!(
            changed.is_empty(),
            "stat failure must not report a change: {changed:?}"
        );
        assert_eq!(
            index.len(),
            2,
            "no removal flicker for the stat-failed note"
        );
        assert!(
            index
                .note(&dir.path().join("sub").join("secret.md"))
                .is_some(),
            "the stat-failed note stays indexed"
        );

        // Once readable again, the watcher is back in sync (no phantom
        // add/modify from the skipped tick).
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path().join("sub"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(watcher.poll(&mut index).changed.is_empty());
    }

    /// Startup race: a file created between `VaultIndex::build` and the
    /// watcher seed must still be picked up by the first poll. Seeding
    /// from the index's own note list leaves the new file out of `stamps`,
    /// so the poll reports it as an add instead of matching an equal stamp
    /// forever (the old walk-seed made it permanently unindexed).
    #[test]
    fn file_created_between_index_build_and_seed_is_indexed() {
        let dir = TempDir::new("watch-race");
        write(&dir.path().join("a.md"), "a\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        assert_eq!(index.len(), 1);

        // The new note appears in the window between the index walk and
        // the watcher seed.
        write(&dir.path().join("late.md"), "---\nstatus: draft\n---\n");
        let mut watcher = VaultWatcher::new(&index);

        let changed = watcher.poll(&mut index).changed;
        assert_eq!(changed, vec![dir.path().join("late.md")]);
        assert_eq!(index.len(), 2, "the late file got indexed");
        assert_eq!(
            index
                .note(&dir.path().join("late.md"))
                .unwrap()
                .status
                .as_deref(),
            Some("draft")
        );
        // And the next poll is quiet.
        assert!(watcher.poll(&mut index).changed.is_empty());
    }

    /// A directory that becomes unreadable *after* startup must produce a
    /// warning on the watch path too: its notes vanish from the walk, and
    /// that must never be silent (the startup walk already reports it).
    #[test]
    fn a_directory_locked_after_startup_is_reported() {
        let dir = TempDir::new("watch-locked");
        write(&dir.path().join("a.md"), "a\n");
        write(&dir.path().join("locked/inside.md"), "h\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        assert_eq!(index.len(), 2);
        let mut watcher = VaultWatcher::new(&index);
        assert!(
            index
                .warnings()
                .iter()
                .all(|w| !w.message.contains("unreadable")),
            "readable at startup: {:?}",
            index.warnings()
        );

        let locked = dir.path().join("locked");
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let outcome = watcher.poll(&mut index);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.path == locked && w.message.contains("unreadable")),
            "the locked directory is reported: {:?}",
            outcome.warnings
        );

        // Readable again: the warning clears and the note comes back.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let outcome = watcher.poll(&mut index);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        assert_eq!(index.len(), 2, "the note is back in the index");
    }
}
