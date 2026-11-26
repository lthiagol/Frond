//! Test-only helpers shared by the unit tests in this crate.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Serialises tests that touch process environment variables. Unit tests
/// share one process and run on a thread pool, so an unguarded `set_var`
/// is visible to every other test in flight. Holding the guard keeps other
/// *guarded* tests out; the original values are restored on drop, even
/// when an assert fails.
pub struct EnvGuard {
    saved: Vec<(OsString, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub fn lock() -> Self {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        Self {
            saved: Vec::new(),
            // A panicking test poisons the lock; the data is `()`, so ignore.
            _lock: ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    fn remember(&mut self, key: &OsStr) {
        if !self.saved.iter().any(|(k, _)| k == key) {
            self.saved.push((key.to_os_string(), std::env::var_os(key)));
        }
    }

    pub fn set(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        self.remember(key.as_ref());
        std::env::set_var(key, value);
    }

    pub fn remove(&mut self, key: impl AsRef<OsStr>) {
        self.remember(key.as_ref());
        std::env::remove_var(key);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
    }
}

/// An empty session dir for the vault picker's recent paths and last vault,
/// so a test never reads the real state. Hold it while the picker opens
/// (`v`); the dir is removed when the returned `TempDir` drops.
#[must_use]
pub fn isolated_session() -> (EnvGuard, TempDir) {
    let dir = TempDir::new("session");
    let mut env = EnvGuard::lock();
    env.set("FROND_SESSION", dir.path());
    (env, dir)
}

/// Drop-guarded uniquely named temp file (no tempfile dependency), so a
/// failing assert never leaves `frond-test-*` files behind in `/tmp`. A
/// process-wide counter keeps concurrent test threads from colliding.
pub struct TempFile(PathBuf);

impl TempFile {
    pub fn new(name: &str, content: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "frond-test-{}-{}-{name}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, content).unwrap();
        Self(path)
    }

    pub fn rewrite(&self, content: &str) {
        fs::write(&self.0, content).unwrap();
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Drop-guarded uniquely named temp directory (no tempfile dependency), for
/// tests that build small vault trees. A process-wide counter keeps
/// concurrent test threads from colliding on the same name.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "frond-test-{}-{}-{name}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Copies the sample vault (`testvault/`, the repository's) into a fresh
/// temp directory and returns it. A test that mutates a vault (a checkbox
/// toggle, a file edit for the watcher, a `git init`) mutates this copy,
/// never the checked-in vault. What the vault holds: `docs/TESTVAULT.md`.
pub fn copy_testvault(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testvault");
    copy_tree(&source, dir.path());
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        // `symlink_metadata` does not follow the link: the sample vault's file
        // symlink is recreated as a link, as `copy_testvault` in
        // `scripts/smoke_tmp.py` keeps it, so both copies hold the same tree.
        let kind = fs::symlink_metadata(entry.path()).unwrap();
        if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), &target).unwrap();
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_testvault_copies_every_file_into_a_mutable_tree() {
        let copy = copy_testvault("copy-testvault");
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testvault");
        assert_eq!(
            fs::read_to_string(copy.path().join("index.md")).unwrap(),
            fs::read_to_string(source.join("index.md")).unwrap(),
        );
        assert!(copy
            .path()
            .join("projects/clients/acme/kickoff.md")
            .is_file());
        assert!(copy.path().join(".trash/deleted.md").is_file());
        // The file symlink stays a link to the same target, so an edit to
        // alpha.md shows through it in the copy, as it does in the checkout.
        let link = copy.path().join("links/alpha-link.md");
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("../alpha.md"));
        fs::write(copy.path().join("alpha.md"), "edited alpha").unwrap();
        assert_eq!(fs::read_to_string(&link).unwrap(), "edited alpha");
        // The copy is writable and leaves the checked-in vault alone.
        fs::write(copy.path().join("index.md"), "edited").unwrap();
        assert_ne!(
            fs::read_to_string(source.join("index.md")).unwrap(),
            "edited"
        );
        assert_ne!(
            fs::read_to_string(source.join("alpha.md")).unwrap(),
            "edited alpha"
        );
    }
}
