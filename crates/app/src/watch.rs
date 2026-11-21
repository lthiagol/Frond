//! Vault-wide change detection for the poll-based watcher.
//!
//! Every vault-wide poll walks the index's root (cheap: hidden entries and
//! directory symlinks are skipped, `vault::walk_vault_with_warnings` — it
//! lists `.base` files alongside `.md` notes and reports directories it
//! could not read), stats each file (mtime + size, same fingerprint as the
//! single-file watcher) and feeds every added/modified/removed file into
//! [`VaultIndex::reindex_one`]. The open note is the one file a tick also
//! re-reads: a permission change moves neither its mtime nor its size, so
//! the stamp comparison alone would never notice it. The watcher itself
//! holds no index state — the index owns the data, the watcher only decides
//! *when* to refresh.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use vault::walk::{walk_vault_with_warnings, WalkEntry};
use vault::{IndexWarning, VaultIndex};

use crate::viewer::FileStamp;

/// One poll's outcome: the paths whose index entries changed, plus what the
/// walk could say about directories that could not be read this tick. The
/// caller must surface the warnings — a directory dropping out of the
/// index is never silent.
#[derive(Debug)]
pub struct PollOutcome {
    pub changed: Vec<PathBuf>,
    /// The walk's warnings; `None` when the walk failed outright (the root
    /// itself could not be read). The two are opposites to the caller: an
    /// empty list is evidence that every directory read, so a warning's
    /// condition ended; `None` is evidence of nothing, so it must not clear
    /// a warning.
    pub warnings: Option<Vec<IndexWarning>>,
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
    /// `changed` = nothing happened). `open_note` is re-read even when its
    /// stamp did not move (see [`Self::reconcile`]). A root that cannot be
    /// walked at all is skipped entirely (retry next tick) and reports no
    /// warnings: the walk could not look, which is not the same as finding
    /// every directory readable.
    pub fn poll(&mut self, index: &mut VaultIndex, open_note: Option<&Path>) -> PollOutcome {
        let Ok(report) = walk_vault_with_warnings(index.root()) else {
            return PollOutcome {
                changed: Vec::new(),
                warnings: None,
            };
        };
        PollOutcome {
            changed: self.reconcile(index, &report.entries, open_note),
            warnings: Some(
                report
                    .warnings
                    .into_iter()
                    .map(IndexWarning::from)
                    .collect(),
            ),
        }
    }

    /// Compares one walk's results against the stamps and feeds every
    /// difference into `reindex_one`. Split from [`Self::poll`] so tests
    /// can replay a walk whose files became unstatable afterwards (the
    /// transient-stat-failure case, see the `Err` arm below).
    ///
    /// The open note is the one file whose stamp match still costs a read:
    /// a permission change moves neither its mtime nor its size, so a
    /// failed read has to be fed to the index explicitly (its reindex
    /// records the same unreadable warning a startup skip leaves) — once,
    /// while the index holds no warning for it; the record persists on its
    /// own afterwards. Every other file stays stat-only — a vault-wide read
    /// per tick is the cost this avoids.
    fn reconcile(
        &mut self,
        index: &mut VaultIndex,
        entries: &[WalkEntry],
        open_note: Option<&Path>,
    ) -> Vec<PathBuf> {
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
            } else if open_note == Some(entry.path.as_path())
                && fs::read_to_string(&entry.path).is_err()
                && !index
                    .warnings()
                    .iter()
                    .any(|warning| warning.path == entry.path && warning.is_unreadable())
            {
                // A changed path was already read by `reindex_many`
                // below; an unchanged one has not, and its unreadability
                // is what the stamp can never show. Re-feed it only while
                // the index has no warning for it: once recorded, the
                // warning persists on its own, so a second feed would
                // change nothing in the report and would turn every quiet
                // tick into a full re-render.
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
        let changed = watcher.poll(&mut index, None).changed;
        assert_eq!(changed, vec![dir.path().join("b.md")]);
        assert_eq!(index.len(), 2);

        // MODIFY: content change (same length) is caught by mtime.
        wait_mtime_tick();
        write(&dir.path().join("a.md"), "---\nstatus: resolved\n---\n");
        let changed = watcher.poll(&mut index, None).changed;
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
        let changed = watcher.poll(&mut index, None).changed;
        assert_eq!(changed, vec![dir.path().join("b.md")]);
        assert_eq!(index.len(), 1);

        // Quiet tick: no changes.
        assert!(watcher.poll(&mut index, None).changed.is_empty());
    }

    #[test]
    fn detects_base_add_and_remove() {
        let dir = TempDir::new("watch-base");
        write(&dir.path().join("a.md"), "a\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);
        assert!(watcher.poll(&mut index, None).changed.is_empty());

        // ADD: a new `.base` file is picked up (path-only in the index).
        wait_mtime_tick();
        let base = dir.path().join("tasks.base");
        write(&base, "filters: []\n");
        let changed = watcher.poll(&mut index, None).changed;
        assert_eq!(changed, vec![base.clone()]);
        assert_eq!(index.bases().collect::<Vec<_>>(), vec![base.clone()]);

        // MODIFY: content changes re-feed the path; the listing stays.
        wait_mtime_tick();
        write(&base, "filters:\n  and: []\n");
        let changed = watcher.poll(&mut index, None).changed;
        assert_eq!(changed, vec![base.clone()]);
        assert_eq!(index.bases().count(), 1);

        // REMOVE: the base drops out.
        wait_mtime_tick();
        fs::remove_file(&base).unwrap();
        let changed = watcher.poll(&mut index, None).changed;
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
        watcher.poll(&mut index, None);
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
        let changed = watcher.reconcile(&mut index, &entries, None);
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
        assert!(watcher.poll(&mut index, None).changed.is_empty());
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

        let changed = watcher.poll(&mut index, None).changed;
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
        assert!(watcher.poll(&mut index, None).changed.is_empty());
    }

    /// The open note is the one file a tick re-reads: a permission change
    /// moves neither its mtime nor its size, so the stamp comparison alone
    /// would never feed it to the index. Every other file stays stat-only —
    /// a vault-wide read per tick is the cost this scope rules out.
    #[test]
    fn only_the_open_note_is_re_read_when_its_stamp_is_unchanged() {
        let dir = TempDir::new("watch-open-note");
        write(&dir.path().join("open.md"), "open\n");
        write(&dir.path().join("other.md"), "other\n");
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);
        let open = dir.path().join("open.md");
        let other = dir.path().join("other.md");

        // Both locked with no content change: no stamp moved.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&open, fs::Permissions::from_mode(0o000)).unwrap();
            fs::set_permissions(&other, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let changed = watcher.poll(&mut index, Some(&open)).changed;
        assert_eq!(
            changed,
            vec![open.clone()],
            "only the open note is read (and so fed to the index)"
        );
        assert!(
            index
                .warnings()
                .iter()
                .any(|w| w.path == open && w.is_unreadable()),
            "the open note's failed read is recorded: {:?}",
            index.warnings()
        );
        assert!(
            !index
                .warnings()
                .iter()
                .any(|w| w.path == other && w.is_unreadable()),
            "a file that is not the open note is not read: {:?}",
            index.warnings()
        );
        assert!(
            index.notes().any(|note| note.path == other),
            "the unread other note keeps its index entry: nothing read it"
        );

        // Still locked: the warning is recorded, so the next tick re-reads
        // the note but feeds nothing. A re-feed would report the same
        // warning again while turning every quiet tick into a re-render.
        let changed = watcher.poll(&mut index, Some(&open)).changed;
        assert!(
            changed.is_empty(),
            "a recorded unreadable warning needs no re-feed: {changed:?}"
        );

        // Restore so the fixture's files can be unlinked.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&open, fs::Permissions::from_mode(0o644)).unwrap();
            fs::set_permissions(&other, fs::Permissions::from_mode(0o644)).unwrap();
        }
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
        let outcome = watcher.poll(&mut index, None);
        let warnings = outcome
            .warnings
            .expect("the walk could read the root and reported");
        assert!(
            warnings
                .iter()
                .any(|w| w.path == locked && w.message.contains("unreadable")),
            "the locked directory is reported: {warnings:?}"
        );

        // Readable again: the warning clears and the note comes back.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let outcome = watcher.poll(&mut index, None);
        let warnings = outcome
            .warnings
            .expect("the walk could read the root and reported");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(index.len(), 2, "the note is back in the index");
    }

    /// A root the walk cannot read fails the whole walk: it must say so,
    /// because an empty warning list would mean "every directory read" and
    /// would clear a warning whose condition still holds.
    #[test]
    fn a_root_that_cannot_be_walked_reports_none_not_an_empty_list() {
        let dir = TempDir::new("watch-rootfail");
        write(&dir.path().join("a.md"), "a\n");
        std::fs::create_dir_all(dir.path().join("locked")).unwrap();
        let mut index = VaultIndex::build(dir.path()).unwrap();
        let mut watcher = VaultWatcher::new(&index);

        let sub = dir.path().join("locked");
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o000)).unwrap();
        }
        // The first poll reports the locked directory (the root reads).
        let outcome = watcher.poll(&mut index, None);
        assert!(outcome.warnings.is_some(), "the walk ran");

        // Now the root itself cannot be read: the walk fails outright.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o000)).unwrap();
        }
        let outcome = watcher.poll(&mut index, None);
        assert!(
            outcome.warnings.is_none(),
            "a failed walk is not an empty report: {:?}",
            outcome.warnings
        );
        assert!(outcome.changed.is_empty(), "and nothing was reindexed");

        // Restore so the fixture can be removed.
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
            fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}
