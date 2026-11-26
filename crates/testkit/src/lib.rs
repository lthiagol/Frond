//! Dev-only test helpers shared by the workspace's test code: the sample
//! vault's path and its copy, and a drop-guarded temp directory. What the
//! sample vault holds: `docs/TESTVAULT.md`.

use std::fs;
use std::path::{Path, PathBuf};

/// Drop-guarded uniquely named temp directory (no tempfile dependency),
/// for tests that build small vault trees. A process-wide counter keeps
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

/// The sample vault (`testvault/`) in the checkout. Read it in place only:
/// a test that writes takes a `copy_testvault` copy first.
pub fn testvault_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testvault")
}

/// Copies the sample vault into a fresh temp directory and returns it. A
/// test that mutates a vault (a checkbox toggle, a file edit for the
/// watcher, a `git init`) mutates this copy, never the checked-in vault.
pub fn copy_testvault(name: &str) -> TempDir {
    let dir = TempDir::new(name);
    copy_tree(&testvault_root(), dir.path());
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        // `symlink_metadata` does not follow the link: the sample vault's file
        // symlink is recreated as a link, so a copy edits the same way the
        // checkout does (`docs/TESTVAULT.md`).
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
        let source = testvault_root();
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
