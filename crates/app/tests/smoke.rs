//! End-to-end smoke test: shells out to `scripts/smoke_tui.py`, which drives
//! the real TUI binary through a pty with `pyte` terminal emulation (19
//! checks: status bar, scrolling, watch reload on/off/catch-up, help popup,
//! clean exit).
//!
//! Ignored by default because it needs:
//! - `python3` with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`), and
//! - a debug build of the app (`cargo build -p frond`) — the binary is located
//!   via cargo's `CARGO_BIN_EXE_frond` and passed to the script as `FROND_APP_BIN`.
//!
//! Run it with: `cargo test -p frond --test smoke -- --ignored`
//!
//! The script never writes to the vault: it copies its source note
//! (`testvault/index.md` by default, override with `FROND_SMOKE_SRC`) to
//! `/tmp/watchtest.md` and operates on that copy only.

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_tui_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_tui.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
