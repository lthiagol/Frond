//! End-to-end smoke test for the vault picker: at startup, two `[vaults]`
//! entries, a filter, Enter opens the chosen vault, Esc quits with status
//! 0; in the app, `v` switches vaults and back; with no vaults configured,
//! a typed path opens and comes back as a recent row; a vault with no
//! `index.md` opens with no note.
//! Shells out to `scripts/smoke_m7.py` (pyte through a pty); see
//! `smoke_m6.rs` for the prerequisites.
//!
//! Run it with: `cargo test -p frond --test smoke_m7 -- --ignored`

use std::process::Command;

#[test]
#[ignore = "requires python3 + pyte (uv pip install pyte) and a debug build; run with `cargo test -- --ignored`"]
fn smoke_m7_script() {
    // Fail early with an actionable message if pyte is missing.
    let probe = Command::new("python3")
        .arg("-c")
        .arg("import pyte")
        .status();
    match probe {
        Ok(status) if status.success() => {}
        _ => panic!("python3 with `pyte` is required; install it with `uv pip install pyte`"),
    }

    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/smoke_m7.py");
    let status = Command::new("python3")
        .env("FROND_APP_BIN", env!("CARGO_BIN_EXE_frond"))
        .arg(script)
        .status()
        .expect("failed to spawn python3");
    assert!(status.success(), "smoke script failed: {status}");
}
