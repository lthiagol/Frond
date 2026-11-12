//! End-to-end smoke test for the M2 shell: shells out to
//! `scripts/smoke_m2.py`, which builds a fixture vault under /tmp and drives
//! the real TUI binary through a pty with `pyte` terminal emulation.
//! checks: sidebar layout/tree/fold/truncation, Tab toggle, h/l focus,
//! Enter-opens-note, Backspace/Ctrl+O history, watch reload + loose sidebar
//! rescan, help popup, mouse clicks/wheel, theme support (fixture config +
//! custom theme via FROND_CONFIG/FROND_THEMES: startup theme, Shift+T cycling,
//! config hot-reload), clean exit.
//!
//! Ignored by default because it needs:
//! - `python3` with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`), and
//! - a debug build of the app (`cargo build -p app`) — the binary is located
//!   via cargo's `CARGO_BIN_EXE_frond` and passed to the script as `FROND_APP_BIN`.
//!
//! Run it with: `cargo test -p app --test smoke_m2 -- --ignored`
//!
//! The script writes only to its own /tmp fixture vault, never to the
//! user's vault.

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_m2_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_m2.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
