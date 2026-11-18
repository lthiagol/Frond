//! End-to-end smoke test for M5 (cross-run session memory): shells out to
//! `scripts/smoke_m5.py`, which drives the real TUI binary through a pty
//! with `pyte` terminal emulation across FOUR separate runs against the
//! same fixture vault under the smoke scratch dir (`scripts/smoke_tmp.py`):
//! run 1 shapes a session (expanded folder, opened note, scrolled) and
//! quits; run 2 restarts with no note argument and must come back exactly
//! there; run 3 restarts with an explicit note argument (which wins over
//! the restore); run 4 restarts with a corrupted session file and must
//! fall back silently to the vault index.
//!
//! Ignored by default because it needs:
//! - `python3` with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`), and
//! - a debug build of the app (`cargo build -p frond`) — the binary is
//!   located via cargo's `CARGO_BIN_EXE_frond` and passed as `FROND_APP_BIN`.
//!
//! Run it with: `cargo test -p frond --test smoke_m5 -- --ignored`
//!
//! The script never touches `testvault/` and never the real
//! `~/.local/state` — `FROND_SESSION` redirects the session dir into /tmp.

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_m5_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_m5.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
