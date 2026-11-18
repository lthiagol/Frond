//! End-to-end smoke test for M4: shells out to `scripts/smoke_m4.py`,
//! which drives the real TUI binary through a pty with `pyte` terminal
//! emulation (global search prompt → results → open-at-line, git clean/
//! dirty status segment, sidebar changed-file markers, help, clean exit)
//! against a fixture vault built (and `git init`ed) under
//! the smoke scratch dir (`scripts/smoke_tmp.py`).
//!
//! Ignored by default because it needs:
//! - `python3` with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`), and
//! - a debug build of the app (`cargo build -p frond`) — the binary is
//!   located via cargo's `CARGO_BIN_EXE_frond` and passed as `FROND_APP_BIN`.
//!
//! Run it with: `cargo test -p frond --test smoke_m4 -- --ignored`
//!
//! The script never touches `testvault/`; it writes only its /tmp fixture
//! (which it turns into a git repository for the sync-status checks).

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_m4_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_m4.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
