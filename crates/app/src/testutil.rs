//! Test-only helpers shared by the unit tests in this crate.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;
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

// `TempDir` and `copy_testvault` live in the `testkit` crate, the one home
// for the sample vault's copy; the re-export keeps the tests' import path.
pub use testkit::{copy_testvault, TempDir};
