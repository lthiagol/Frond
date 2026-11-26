#!/usr/bin/env python3
"""Smoke test for M5: cross-run session memory, using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script builds a small fixture vault under /tmp/frond-m5-vault (writes
ONLY there, never to testvault/) and runs the app FOUR times against it,
always with FROND_SESSION pointed at /tmp/frond-m5-session so the real
~/.local/state is never touched:

1. run 1 shapes a session: expands the `archive` folder, opens
   projects/alpha.md, scrolls it 4 lines, quits with `q`. Asserts the
   session file was written.
2. run 2 restarts with NO note argument: the same note must come back
   with its scroll, and the sidebar must still show archive's child.
3. run 3 restarts WITH an explicit note argument: that argument wins
   over the session restore.
4. run 4 restarts with a corrupted session file: silent fallback to the
   vault index, no crash.
"""
import os, pty, select, struct, sys, shutil, time, fcntl, termios

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
VAULT = smoke_tmp.path("frond-m5-vault")
SESSION_DIR = smoke_tmp.path("frond-m5-session")

COLS, ROWS = 100, 30

# --- fixture vault ---------------------------------------------------------
shutil.rmtree(VAULT, ignore_errors=True)
shutil.rmtree(SESSION_DIR, ignore_errors=True)
BODY = "\n".join(f"alpha body {i}" for i in range(1, 81))
ALPHA = "# Alpha\n\nintro\n\nMARKER-FIVE\n\n" + BODY + "\n"
for path, body in [
    ("index.md", "# Index\n\nThe vault entry note.\n\n" + "index filler\n\n" * 30),
    ("projects/alpha.md", ALPHA),
    ("projects/beta.md", "# Beta\n\nuntouched note\n"),
    ("archive/old.md", "# Old\n\narchived note\n"),
]:
    full = os.path.join(VAULT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(body)


# --- pty harness: the app is spawned fresh for every run --------------------
pid = None
fd = None
screen = None
stream = None


def start_run(args):
    global pid, fd, screen, stream
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        # The quit-time session save must never touch the real state dir.
        os.environ["FROND_SESSION"] = SESSION_DIR
        # Never the real ~/.config: a missing file is the default config.
        os.environ["FROND_CONFIG"] = smoke_tmp.path("frond-no-config.toml")
        # NO_COLOR would strip colors; nothing here compares them, but keep
        # the child deterministic anyway.
        os.environ.pop("NO_COLOR", None)
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        os.execv(APP, [APP, *args])
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)


def pump(sec):
    end = time.time() + sec
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                return False
            if not data:
                return False
            stream.feed(data)
    return True


def pump_until(cond, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        pump(0.1)
    return False


def send(s):
    os.write(fd, s.encode())


def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))


def body_text():
    return "\n".join(row(y) for y in range(ROWS))


def status_line():
    return row(ROWS - 1)


def wait_exit():
    send("q")
    pump(0.8)
    _, status = os.waitpid(pid, 0)
    return os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    return False


ok = True


def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())


# --- run 1: shape the session ----------------------------------------------
start_run(["--vault", VAULT])
check(pump(1.5), "run 1: app alive")
check("Index" in body_text(), "run 1: index.md rendered")

# Sidebar tree (dirs first): 0 archive, 1 projects, 2 alpha.md, ...
# Focus it and expand `archive` (the selected first row).
send("h")
pump(0.3)
send("\r")
pump(0.3)
check("old.md" in body_text(), "run 1: archive expanded, old.md visible")

# Two rows down to `projects` (old.md, projects), expand it, one more row
# onto alpha.md, open it (Enter: opens + moves focus to the content pane).
send("j")
pump(0.2)
send("j")
pump(0.2)
send("\r")
pump(0.3)
check("beta.md" in body_text(), "run 1: projects expanded")
send("j")
pump(0.2)
send("\r")
check(
    pump_until(lambda: "alpha.md" in status_line()),
    "run 1: alpha.md opened (status bar)",
)
check(
    pump_until(lambda: "Alpha" in body_text()),
    "run 1: alpha.md content rendered",
)

# Scroll 4 lines: MARKER-FIVE (raw line 4) lands on the first content row
# and the # Alpha heading scrolls off.
send("j")
pump(0.15)
send("j")
pump(0.15)
send("j")
pump(0.15)
send("j")
pump(0.4)
check("MARKER-FIVE" in body_text(), "run 1: scrolled to MARKER-FIVE")
check("# Alpha" not in body_text(), "run 1: heading scrolled off")

check(wait_exit(), "run 1: clean exit code 0")

# The quit-time save fired: exactly one per-vault session file exists.
sessions = [f for f in os.listdir(SESSION_DIR) if f.startswith("session-")]
check(len(sessions) == 1, f"run 1: session file written ({sessions})")

# --- run 2: restart with no note argument — the session comes back ----------
start_run(["--vault", VAULT])
check(pump(1.5), "run 2: app alive")
check("alpha.md" in status_line(), "run 2: the last note is restored")
check("MARKER-FIVE" in body_text(), "run 2: scroll restored (marker visible)")
check("# Alpha" not in body_text(), "run 2: scroll restored (heading still off)")
check("old.md" in body_text(), "run 2: archive still expanded in the tree")
check(
    "The vault entry note." not in body_text(),
    "run 2: the index content did not come back (session note wins)",
)
check(wait_exit(), "run 2: clean exit code 0")

# --- run 3: an explicit note argument wins over the session ------------------
start_run(["--vault", VAULT, f"{VAULT}/index.md"])
check(pump(1.5), "run 3: app alive")
check("Index" in body_text(), "run 3: the explicit argument opened")
check("alpha.md" not in status_line(), "run 3: session note NOT restored")
check(wait_exit(), "run 3: clean exit code 0")

# --- run 4: a corrupted session file degrades silently -----------------------
session_file = os.path.join(SESSION_DIR, sessions[0])
with open(session_file, "w") as f:
    f.write("note = 42 !!! not toml !!!")
start_run(["--vault", VAULT])
check(pump(1.5), "run 4: app alive")
check("Index" in body_text(), "run 4: corrupt session falls back to the index")
check("alpha.md" not in status_line(), "run 4: nothing restored from garbage")
check(wait_exit(), "run 4: clean exit code 0")

print("SMOKE M5:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
