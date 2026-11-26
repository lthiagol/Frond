#!/usr/bin/env python3
"""Smoke test for the startup vault picker (SPEC "Vault picker").

- a config with two `[vaults]` entries opens frond on the picker; typing a
  filter and pressing Enter opens the chosen vault (its index note on screen);
- `Esc` in the picker quits with exit status 0;
- the clean quit records `last-vault`, which preselects on the next start;
- `v` in the app (SPEC "Vault picker in the app") switches to another
  vault and back, the left vault's session restored, `last-vault` the one
  open at the quit;
- with no `[vaults]` and no `vault` key the picker opens on its path
  entry: a typed path opens that vault, recorded in `recent-vaults` and
  listed as a recent row on the next start;
- a vault with no `index.md` and no session starts with no note; a note
  opened from the sidebar reopens on the next start;
- a config `vault` that names a missing directory exits 2 with the one
  `error: vault not found: …` line on stderr (SPEC "Data flow" step 1).

The vaults are copies of the sample vault (`testvault/`, see
docs/TESTVAULT.md) made by `copy_testvault`; each copy's folder name is its
marker, since the sidebar header shows it.

Prerequisites: python3 with `pyte` (`uv run --with pyte`) and a debug build
(`cargo build -p frond`; binary path overridable with $FROND_APP_BIN).

The copies and the config/session/theme dirs live under the private smoke
scratch dir (`scripts/smoke_tmp.py`); FROND_CONFIG / FROND_SESSION /
FROND_THEMES keep the real `~/.config` and `~/.local/state` untouched.
"""
import fcntl
import os
import pty
import select
import shutil
import struct
import subprocess
import sys
import termios
import time

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond"
)
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
ROOT = smoke_tmp.path("frond-m7")
CONFIG_FILE = os.path.join(ROOT, "config.toml")
MISSING_CONFIG = os.path.join(ROOT, "missing-config.toml")
SESSION_DIR = os.path.join(ROOT, "session")
THEME_DIR = os.path.join(ROOT, "themes")
COLS, ROWS = 100, 30
SIDEBAR_W = 30

# --- vault copies (testvault/, docs/TESTVAULT.md) ---------------------------
# A copy's folder name is the marker: the sidebar header shows it, and the
# picker's recent row is named by it.
shutil.rmtree(ROOT, ignore_errors=True)
ALPHA_NAME, BETA_NAME, NOINDEX_NAME = "frond-m7-alpha", "frond-m7-beta", "frond-m7-noindex"
ALPHA = smoke_tmp.copy_testvault(ALPHA_NAME)
BETA = smoke_tmp.copy_testvault(BETA_NAME)
NOINDEX = smoke_tmp.copy_testvault(NOINDEX_NAME)
os.remove(os.path.join(NOINDEX, "index.md"))
TYPED_BETA = os.path.join(os.path.dirname(BETA), "frond-m7-be")

# --- config and theme dir: not vault content, built by the suite -----------
os.makedirs(THEME_DIR, exist_ok=True)
with open(CONFIG_FILE, "w") as f:
    f.write(
        "[vaults]\n"
        f'alpha = "{ALPHA}"\n'
        f'beta = "{BETA}"\n'
    )

ok = True


def check(cond, label, screen=None):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond and screen is not None:
        print("--- screen ---")
        print("\n".join(screen.display))


class Run:
    def __init__(self, *args, config=CONFIG_FILE, session=SESSION_DIR):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["FROND_CONFIG"] = config
            os.environ["FROND_SESSION"] = session
            os.environ["FROND_THEMES"] = THEME_DIR
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
            os.execv(APP, [APP, *args])
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)

    def pump(self, sec):
        end = time.time() + sec
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.1)
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return False
                if not data:
                    return False
                self.stream.feed(data)
        return True

    def until(self, cond, timeout=5.0):
        end = time.time() + timeout
        while time.time() < end:
            if cond():
                return True
            self.pump(0.1)
        return cond()

    def text(self):
        return "\n".join(self.screen.display)

    def status(self):
        """The status bar: the selected sidebar row's name, then its lines."""
        return self.screen.display[ROWS - 1]

    def sidebar(self):
        """The sidebar column, whose header is the open vault's folder name."""
        return "\n".join(line[:SIDEBAR_W] for line in self.screen.display)

    def heading(self, text):
        """A note-pane row is exactly `text`: a heading line of the open note
        (the pane is right of the sidebar and above the status bar)."""
        return any(
            line[SIDEBAR_W:].strip(" █│") == text for line in self.screen.display[:-1]
        )

    def send(self, s):
        os.write(self.fd, s.encode())

    def selected_row(self):
        """The picker row drawn with the selection bar: its name cell's
        background differs from the popup's (the border cell left of it).
        A recent row is named by its folder."""
        for y, line in enumerate(self.screen.display):
            for name in ("alpha", "beta", BETA_NAME):
                x = line.find("│" + name)
                if x >= 0:
                    cells = self.screen.buffer[y]
                    if cells[x + 1].bg != cells[x].bg or cells[x + 1].reverse:
                        return name
        return None

    def exit_status(self):
        self.pump(0.8)
        _, status = os.waitpid(self.pid, 0)
        return os.WEXITSTATUS(status) if os.WIFEXITED(status) else None

    def wait_exit(self):
        return self.exit_status() == 0


# --- 1: filter + Enter opens the chosen vault -------------------------------
run = Run()
check(run.until(lambda: "vaults: 2" in run.text()), "the picker lists both vaults", run.screen)
check(run.selected_row() == "alpha", "with no last vault, the first entry is selected", run.screen)
check(not run.heading("Lorem Index"), "no vault opened before the choice", run.screen)
run.send("bet")
check(run.until(lambda: "vaults: 1" in run.text()), "the filter narrows to one entry", run.screen)
run.send("\r")
check(
    run.until(lambda: run.heading("Lorem Index") and BETA_NAME in run.sidebar()),
    "Enter opened the beta copy: its index.md heading, its sidebar header",
    run.screen,
)
run.send("q")
check(run.wait_exit(), "clean quit, exit status 0")
with open(os.path.join(SESSION_DIR, "last-vault")) as f:
    recorded = f.read().strip()
check(
    recorded == os.path.realpath(BETA),
    f"last-vault records the beta vault ({recorded})",
)

# --- 2: the last vault is preselected; Esc quits with status 0 --------------
run = Run()
check(run.until(lambda: "vaults: 2" in run.text()), "the picker shows again", run.screen)
check(run.until(lambda: run.selected_row() == "beta"), "the last vault is preselected", run.screen)
run.send("\x1b")
check(run.wait_exit(), "Esc quits with exit status 0")

# --- 3: `v` switches vaults in the running app -----------------------------
run = Run("--vault", "alpha")
check(
    run.until(lambda: run.heading("Lorem Index") and ALPHA_NAME in run.sidebar()),
    "alpha opened by name: its index.md heading, its sidebar header",
    run.screen,
)
run.send("\x0b")
run.send("roadmap")
run.pump(0.3)
run.send("\r")
check(
    run.until(lambda: run.heading("Roadmap")),
    "Ctrl+K opened projects/roadmap.md in alpha (its H1 in the note pane)",
    run.screen,
)
run.send("v")
check(run.until(lambda: "vaults: 2" in run.text()), "v opens the picker", run.screen)
check(run.until(lambda: run.selected_row() == "alpha"), "the open vault is preselected", run.screen)
check(run.heading("Roadmap"), "the main view stays behind the popup", run.screen)
run.send("\x1b[B")
run.send("\r")
check(
    run.until(lambda: run.heading("Lorem Index") and BETA_NAME in run.sidebar()),
    "switched to beta: its index.md heading, its sidebar header",
    run.screen,
)
check(
    not run.heading("Roadmap") and ALPHA_NAME not in run.sidebar(),
    "nothing of alpha is left (no roadmap H1, no alpha header)",
    run.screen,
)
check("vaults: 2" not in run.text(), "the popup closed", run.screen)
run.send("v")
check(run.until(lambda: run.selected_row() == "beta"), "beta is now preselected", run.screen)
run.send("\x1b[A")
run.send("\r")
check(
    run.until(lambda: run.heading("Roadmap") and ALPHA_NAME in run.sidebar()),
    "back in alpha, its session restored (the roadmap note)",
    run.screen,
)
run.send("q")
check(run.wait_exit(), "clean quit after the switches, exit status 0")
with open(os.path.join(SESSION_DIR, "last-vault")) as f:
    recorded = f.read().strip()
check(
    recorded == os.path.realpath(ALPHA),
    f"last-vault records the vault open at the quit ({recorded})",
)

# --- 4: no vaults configured: a typed path, then its recent row ----------
EMPTY_CONFIG = os.path.join(ROOT, "empty-config.toml")
EMPTY_SESSION = os.path.join(ROOT, "empty-session")
with open(EMPTY_CONFIG, "w") as f:
    f.write("")
run = Run(config=EMPTY_CONFIG, session=EMPTY_SESSION)
check(
    run.until(lambda: "type a path to a vault (/, ~ or .)" in run.text()),
    "no vaults: the picker opens on its empty state, no stdin prompt",
    run.screen,
)
run.send(TYPED_BETA)
run.pump(0.3)
run.send("\t")
check(
    run.until(lambda: "open " + BETA + "/" in run.text()),
    "Tab completed the typed path",
    run.screen,
)
run.send("\r")
check(
    run.until(lambda: run.heading("Lorem Index") and BETA_NAME in run.sidebar()),
    "Enter opened the typed vault (the beta copy)",
    run.screen,
)
run.send("q")
check(run.wait_exit(), "clean quit from the typed vault, exit status 0")
with open(os.path.join(EMPTY_SESSION, "recent-vaults")) as f:
    recorded = f.read().strip()
check(
    recorded == os.path.realpath(BETA),
    f"recent-vaults records the typed vault ({recorded})",
)
run = Run(config=EMPTY_CONFIG, session=EMPTY_SESSION)
check(
    run.until(lambda: "vaults: 1" in run.text() and "recent" in run.text()),
    "the next start lists the recent row",
    run.screen,
)
check(run.selected_row() == BETA_NAME, "the recent row is selected", run.screen)
run.send("\r")
check(
    run.until(lambda: run.heading("Lorem Index") and BETA_NAME in run.sidebar()),
    "Enter on the recent row opens it",
    run.screen,
)
run.send("q")
check(run.wait_exit(), "clean quit, exit status 0")

# --- 4b: a vault with no index.md opens with no note -----------------------
# SPEC "Data flow" step 1: no session and no index.md is the no-note state,
# the sidebar focused; a note opened there is the session's next start.
# `alpha.md`'s place in the sidebar, and so how many `j` presses reach it:
# docs/TESTVAULT.md, "Smoke suites".
NOINDEX_SESSION = os.path.join(ROOT, "noindex-session")
run = Run("--vault", NOINDEX, config=EMPTY_CONFIG, session=NOINDEX_SESSION)
check(
    run.until(lambda: "no note open — pick one in the sidebar" in run.text()),
    "a vault without index.md starts with no note",
    run.screen,
)
for _ in range(10):
    run.send("j")
    run.pump(0.1)
check(
    run.until(lambda: "alpha.md ·" in run.status()),
    "ten j presses select the root note alpha.md (the status bar names it)",
    run.screen,
)
run.send("\r")
check(
    run.until(lambda: run.heading("Alpha")),
    "Enter on the sidebar note opens it (alpha's H1 in the note pane)",
    run.screen,
)
check("no note open" not in run.text(), "the empty state is gone", run.screen)
run.send("q")
check(run.wait_exit(), "clean quit from the no-index vault, exit status 0")
run = Run("--vault", NOINDEX, config=EMPTY_CONFIG, session=NOINDEX_SESSION)
check(
    run.until(lambda: run.heading("Alpha")),
    "the restart reopens the session's note (alpha.md)",
    run.screen,
)
run.send("q")
check(run.wait_exit(), "clean quit after the restart, exit status 0")

# --- 5: a config `vault` that is not a directory stops the start ------------
# The check runs before the terminal is set up, so a pipe is enough: status
# 2, exactly the one error line on stderr, nothing on stdout.
MISSING = os.path.join(ROOT, "missing")
with open(MISSING_CONFIG, "w") as f:
    f.write(f'vault = "{MISSING}"\n')
probe = subprocess.run(
    [APP],
    env={**os.environ, "FROND_CONFIG": MISSING_CONFIG, "FROND_SESSION": SESSION_DIR},
    stdin=subprocess.DEVNULL,
    capture_output=True,
    timeout=10,
)
check(probe.returncode == 2, f"a missing config vault exits 2 ({probe.returncode})")
check(
    probe.stderr.decode() == f"error: vault not found: {MISSING} (config vault)\n",
    f"a missing config vault prints exactly its error line ({probe.stderr!r})",
)
check(probe.stdout == b"", "a missing config vault prints nothing on stdout")

print("SMOKE M7:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
