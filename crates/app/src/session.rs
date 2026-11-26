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
//! `last-vault` (same directory) holds the canonical root of the last vault
//! open at a clean quit: the vault picker's preselection, nothing more.
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
    /// Writes the snapshot atomically ([`write_atomic`]).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        write_atomic(path, &text)
    }

    /// Reads a snapshot: `None` when the file is missing or malformed —
    /// both are the "no session" state, never an error surface.
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }
}

/// Temp file + rename in the same directory, so a crash mid-write cannot
/// leave a truncated file.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // pid-suffixed: two instances quitting at once must not truncate each
    // other's temp file.
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// The `last-vault` file; `None` when no state directory is resolvable.
fn last_vault_file() -> Option<PathBuf> {
    Some(state_dir()?.join("last-vault"))
}

/// Records `vault_root` (canonicalized) as the last vault opened.
pub fn save_last_vault(vault_root: &Path) -> io::Result<()> {
    let path = last_vault_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state directory"))?;
    let canon = vault_root
        .canonicalize()
        .unwrap_or_else(|_| vault_root.to_path_buf());
    write_atomic(&path, &format!("{}\n", canon.display()))
}

/// The last vault opened, as recorded; `None` when missing or empty.
pub fn load_last_vault() -> Option<PathBuf> {
    let text = fs::read_to_string(last_vault_file()?).ok()?;
    let line = text.lines().next()?.trim_end();
    (!line.is_empty()).then(|| PathBuf::from(line))
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
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");
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
        let dir = TempDir::new("sess-dir");
        let mut env = crate::testutil::EnvGuard::lock();
        env.set("FROND_SESSION", dir.path());
        let a = session_file(Path::new("/tmp/vault-a")).unwrap();
        let b = session_file(Path::new("/tmp/vault-b")).unwrap();
        // Spelling noise does not change the key (canonicalized input).
        let a_again = session_file(Path::new("/tmp/vault-a")).unwrap();
        env.remove("FROND_SESSION");
        assert_ne!(a, b, "distinct vaults get distinct files");
        assert!(a.starts_with(dir.path()), "override directory respected");
        assert_eq!(a, a_again);
    }

    #[test]
    fn last_vault_round_trips_canonically() {
        let state = TempDir::new("sess-last");
        let vault = TempDir::new("sess-last-vault");
        let mut env = crate::testutil::EnvGuard::lock();
        env.set("FROND_SESSION", state.path());
        assert_eq!(load_last_vault(), None, "no file yet");
        let noisy = vault.path().join(".");
        save_last_vault(&noisy).unwrap();
        let loaded = load_last_vault();
        env.remove("FROND_SESSION");
        assert_eq!(loaded, Some(vault.path().canonicalize().unwrap()));
        assert!(state.path().join("last-vault").is_file());
        let leftovers = fs::read_dir(state.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "atomic write leaves no temp file");
    }

    #[test]
    fn state_dir_follows_the_xdg_precedence() {
        let xdg = TempDir::new("sess-xdg");
        let home = TempDir::new("sess-home");
        let mut env = crate::testutil::EnvGuard::lock();
        env.set("XDG_STATE_HOME", xdg.path());
        env.set("HOME", home.path());
        let with_xdg = session_file(Path::new("/tmp/vault-a")).unwrap();
        assert!(
            with_xdg.starts_with(xdg.path()),
            "XDG_STATE_HOME wins over HOME: {with_xdg:?}"
        );
        env.remove("XDG_STATE_HOME");
        let with_home = session_file(Path::new("/tmp/vault-a")).unwrap();
        assert!(
            with_home.starts_with(home.path().join(".local/state/frond")),
            "falls back to HOME/.local/state: {with_home:?}"
        );
    }
}
