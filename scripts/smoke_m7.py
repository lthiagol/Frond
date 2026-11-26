#!/usr/bin/env python3
"""Smoke test for the startup vault picker (SPEC "Vault picker").

- a config with two `[vaults]` entries opens frond on the picker; typing a
  filter and pressing Enter opens the chosen vault (its note on screen);
- `Esc` in the picker quits with exit status 0;
- the clean quit records `last-vault`, which preselects on the next start.

Prerequisites: python3 with `pyte` (`uv run --with pyte`) and a debug build
(`cargo build -p frond`; binary path overridable with $FROND_APP_BIN).

Fixtures live under the private smoke scratch dir (`scripts/smoke_tmp.py`);
FROND_CONFIG / FROND_SESSION / FROND_THEMES keep the real `~/.config` and
`~/.local/state` untouched.
"""
import fcntl
import os
import pty
import select
import shutil
import struct
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
SESSION_DIR = os.path.join(ROOT, "session")
THEME_DIR = os.path.join(ROOT, "themes")
COLS, ROWS = 100, 30

shutil.rmtree(ROOT, ignore_errors=True)
for name, heading in (("alpha", "Alpha Vault Home"), ("beta", "Beta Vault Home")):
    os.makedirs(os.path.join(ROOT, name), exist_ok=True)
    with open(os.path.join(ROOT, name, "index.md"), "w") as f:
        f.write(f"# {heading}\n\nbody\n")
os.makedirs(THEME_DIR, exist_ok=True)
with open(CONFIG_FILE, "w") as f:
    f.write(
        "[vaults]\n"
        f'alpha = "{os.path.join(ROOT, "alpha")}"\n'
        f'beta = "{os.path.join(ROOT, "beta")}"\n'
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
    def __init__(self):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["FROND_CONFIG"] = CONFIG_FILE
            os.environ["FROND_SESSION"] = SESSION_DIR
            os.environ["FROND_THEMES"] = THEME_DIR
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
            os.execv(APP, [APP])
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

    def send(self, s):
        os.write(self.fd, s.encode())

    def selected_row(self):
        """The picker row drawn with the selection bar: its name cell's
        background differs from the popup's (the border cell left of it)."""
        for y, line in enumerate(self.screen.display):
            for name in ("alpha", "beta"):
                x = line.find("│" + name)
                if x >= 0:
                    cells = self.screen.buffer[y]
                    if cells[x + 1].bg != cells[x].bg or cells[x + 1].reverse:
                        return name
        return None

    def wait_exit(self):
        self.pump(0.8)
        _, status = os.waitpid(self.pid, 0)
        return os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0


# --- 1: filter + Enter opens the chosen vault -------------------------------
run = Run()
check(run.until(lambda: "vaults: 2" in run.text()), "the picker lists both vaults", run.screen)
check(run.selected_row() == "alpha", "with no last vault, the first entry is selected", run.screen)
check("Alpha Vault Home" not in run.text(), "no vault opened before the choice", run.screen)
run.send("bet")
check(run.until(lambda: "vaults: 1" in run.text()), "the filter narrows to one entry", run.screen)
run.send("\r")
check(run.until(lambda: "Beta Vault Home" in run.text()), "Enter opened the beta vault", run.screen)
run.send("q")
check(run.wait_exit(), "clean quit, exit status 0")
with open(os.path.join(SESSION_DIR, "last-vault")) as f:
    recorded = f.read().strip()
check(
    recorded == os.path.realpath(os.path.join(ROOT, "beta")),
    f"last-vault records the beta vault ({recorded})",
)

# --- 2: the last vault is preselected; Esc quits with status 0 --------------
run = Run()
check(run.until(lambda: "vaults: 2" in run.text()), "the picker shows again", run.screen)
check(run.until(lambda: run.selected_row() == "beta"), "the last vault is preselected", run.screen)
run.send("\x1b")
check(run.wait_exit(), "Esc quits with exit status 0")

print("SMOKE M7:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
