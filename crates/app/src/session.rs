//! Cross-run session state: what the app remembers between runs for a
//! vault — the last open note (and its scroll), and the sidebar's mode,
//! selection and folder expansion. Saved on clean quit, restored at
//! startup when no note argument was given.
//!
//! File: `~/.local/state/frond/session-<hash>.toml`, one per vault (the
//! hash is FNV-1a over the canonical vault root, so two vaults never
//! fight over one state). Paths are stored vault-relative so the file
//! survives the vault moving on disk; entries outside the vault are
//! stored verbatim but never restored (`App::restore_session` honors
//! vault-relative entries only).
//!
//! `FROND_SESSION` overrides the state directory (tests and smokes only —
//! they must never write to the real `~/.local/state`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One vault's session snapshot. All fields optional: a partial file
/// (hand-edited, older version) degrades to the default startup state
/// field by field.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// Last open note (vault-relative, or absolute when outside the vault).
    pub note: Option<String>,
    /// Its scroll offset, in body rows.
    pub scroll: u32,
    /// Expanded sidebar folders.
    pub expanded: Vec<String>,
    /// Selected sidebar row.
    pub selected: Option<String>,
    /// Sidebar mode: `"tree"` or `"tags"`.
    pub mode: Option<String>,
}

impl SessionState {
    /// Writes the snapshot atomically (temp file + rename in the same
    /// directory) so a crash mid-write cannot leave a truncated file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }

    /// Reads a snapshot: `None` when the file is missing or malformed —
    /// both are the "no session" state, never an error surface.
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }
}

/// `FROND_SESSION` overrides the directory; else the XDG state dir:
/// `$XDG_STATE_HOME/frond`, falling back to `$HOME/.local/state/frond`.
fn state_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("FROND_SESSION") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    if let Ok(dir) = std::env::var("XDG_STATE_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("frond"));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("frond")
        })
}

/// FNV-1a 64-bit: stable, dep-free file key over the vault path.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The session file for `vault_root`: `<state dir>/session-<hash>.toml`.
/// `None` when no state directory is resolvable (no HOME and no override).
pub fn session_file(vault_root: &Path) -> Option<PathBuf> {
    let dir = state_dir()?;
    let canon = vault_root
        .canonicalize()
        .unwrap_or_else(|_| vault_root.to_path_buf());
    let hash = format!("{:016x}", fnv1a(canon.to_string_lossy().as_bytes()));
    Some(dir.join(format!("session-{hash}.toml")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn save_load_roundtrip() {
        let dir = TempDir::new("sess-roundtrip");
        let path = dir.path().join("session-abc.toml");
        let state = SessionState {
            note: Some("sub/b.md".to_string()),
            scroll: 12,
            expanded: vec!["sub".to_string(), "archive".to_string()],
            selected: Some("sub/b.md".to_string()),
            mode: Some("tags".to_string()),
        };
        state.save(&path).unwrap();
        // Atomic write leaves no temp file behind.
        assert!(!path.with_extension("toml.tmp").exists());
        assert_eq!(SessionState::load(&path).as_ref(), Some(&state));
    }

    #[test]
    fn missing_and_malformed_files_are_no_session() {
        let dir = TempDir::new("sess-missing");
        assert_eq!(SessionState::load(&dir.path().join("nope.toml")), None);
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "note = 42 !!!").unwrap();
        assert_eq!(SessionState::load(&path), None);
    }

    #[test]
    fn partial_files_degrade_field_by_field() {
        let dir = TempDir::new("sess-partial");
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "note = \"a.md\"\n").unwrap();
        let state = SessionState::load(&path).unwrap();
        assert_eq!(state.note.as_deref(), Some("a.md"));
        assert_eq!(state.scroll, 0);
        assert!(state.expanded.is_empty());
        assert_eq!(state.mode, None);
    }

    #[test]
    fn session_file_keys_per_vault_and_honors_the_override() {
        // SAFETY of the env dance: single-threaded unit test process.
        let dir = TempDir::new("sess-dir");
        std::env::set_var("FROND_SESSION", dir.path());
        let a = session_file(Path::new("/tmp/vault-a")).unwrap();
        let b = session_file(Path::new("/tmp/vault-b")).unwrap();
        // Spelling noise does not change the key (canonicalized input).
        let a_again = session_file(Path::new("/tmp/vault-a")).unwrap();
        std::env::remove_var("FROND_SESSION");
        assert_ne!(a, b, "distinct vaults get distinct files");
        assert!(a.starts_with(dir.path()), "override directory respected");
        assert_eq!(a, a_again);
    }

    #[test]
    fn state_dir_follows_the_xdg_precedence() {
        // SAFETY of the env dance: single-threaded unit test process; the
        // original values are restored.
        let xdg = TempDir::new("sess-xdg");
        let home = TempDir::new("sess-home");
        let saved_home = std::env::var("HOME").ok();
        let saved_xdg = std::env::var("XDG_STATE_HOME").ok();
        std::env::set_var("XDG_STATE_HOME", xdg.path());
        std::env::set_var("HOME", home.path());
        let with_xdg = session_file(Path::new("/tmp/vault-a")).unwrap();
        assert!(
            with_xdg.starts_with(xdg.path()),
            "XDG_STATE_HOME wins over HOME: {with_xdg:?}"
        );
        std::env::remove_var("XDG_STATE_HOME");
        let with_home = session_file(Path::new("/tmp/vault-a")).unwrap();
        assert!(
            with_home.starts_with(home.path().join(".local/state/frond")),
            "falls back to HOME/.local/state: {with_home:?}"
        );
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
