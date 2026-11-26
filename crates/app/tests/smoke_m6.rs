//! End-to-end smoke test for the 0.6.x feature set that only had unit
//! coverage: theme groups (`Ctrl+T` / `Shift+T`), the status-bar icon set
//! (config `icons = "unicode"`), the outline (`o`, `Enter` jumps to a
//! heading) and `.base` tables opened from the sidebar tree. The test shells
//! out to `scripts/smoke_m6.py`, which drives the real TUI binary through a
//! pty with `pyte` terminal emulation against a copy of the sample vault
//! (`testvault/`, see `docs/TESTVAULT.md`) made in the smoke scratch dir
//! (`scripts/smoke_tmp.py`).
//!
//! Ignored by default because it needs:
//! - `python3` with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`), and
//! - a debug build of the app (`cargo build -p frond`) — the binary is
//!   located via cargo's `CARGO_BIN_EXE_frond` and passed as `FROND_APP_BIN`.
//!
//! Run it with: `cargo test -p frond --test smoke_m6 -- --ignored`
//!
//! The script writes only its copy, never `testvault/`, the real `~/.config`
//! (the config and theme dirs are redirected with FROND_CONFIG / FROND_THEMES)
//! or the real `~/.local/state` — `FROND_SESSION` redirects the session dir.

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_m6_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_m6.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
