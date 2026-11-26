#!/usr/bin/env python3
"""Smoke test for M5: cross-run session memory, using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script copies the sample vault (testvault/, see docs/TESTVAULT.md) into
the scratch dir and runs the app FOUR times against that copy, always with
FROND_SESSION pointed at the scratch dir so the real ~/.local/state is never
touched. It writes ONLY its copy, never testvault/.

1. run 1 shapes a session: expands the `archive` and `projects` folders,
   opens reading/long-read.md, scrolls it 16 lines, quits with `q`. Asserts
   the session file was written.
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
SESSION_DIR = smoke_tmp.path("frond-m5-session")

COLS, ROWS = 100, 30
SIDEBAR_W = 30

# --- vault copy + session dir ----------------------------------------------
VAULT = smoke_tmp.copy_testvault("frond-m5-vault")
shutil.rmtree(SESSION_DIR, ignore_errors=True)


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


def down(n):
    for _ in range(n):
        send("j")
        pump(0.15)


def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))


def body_text():
    return "\n".join(row(y) for y in range(ROWS))


def status_line():
    return row(ROWS - 1)


def sidebar_has(text):
    return any(text in row(y)[:SIDEBAR_W] for y in range(ROWS - 1))


def content_has(text):
    return any(text in row(y)[SIDEBAR_W:] for y in range(ROWS - 1))


def content_top():
    # Row 0 is the content pane's top margin; the first text row is row 1.
    return row(1)[SIDEBAR_W:].strip()


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
check(content_has("Lorem Index"), "run 1: index.md rendered")

# Sidebar tree rows (folders first, then files, docs/TESTVAULT.md Layout):
# archive is row 0. Focus the tree and expand it (the selected first row).
send("f")
pump(0.3)
send("\r")
pump(0.3)
check(sidebar_has("old-note.md"), "run 1: archive expanded, old-note.md visible")

# Nine rows down to `projects` (archive's three children, then dashboards
# through links), expand it.
down(9)
send("\r")
pump(0.3)
check(sidebar_has("beta.md"), "run 1: projects expanded, beta.md visible")

# Nine rows down to `reading` (projects' children first, clients folded),
# expand it, one row down onto long-read.md and open it (Enter: opens + moves
# focus to the content pane).
down(9)
send("\r")
pump(0.3)
down(1)
send("\r")
check(
    pump_until(lambda: "long-read.md" in status_line()),
    "run 1: long-read.md opened (status bar)",
)
check(
    pump_until(lambda: content_has("Long read")),
    "run 1: long-read.md content rendered (Long read heading)",
)

# Scroll 16 lines: Paragraph 2 heads the first text row and the `Long read`
# heading scrolls off.
down(16)
pump(0.4)
check(
    content_top().startswith("Paragraph 2:"),
    "run 1: scrolled to Paragraph 2 (first text row)",
)
check(not content_has("Long read"), "run 1: heading scrolled off")

check(wait_exit(), "run 1: clean exit code 0")

# The quit-time save fired: exactly one per-vault session file exists.
sessions = [f for f in os.listdir(SESSION_DIR) if f.startswith("session-")]
check(len(sessions) == 1, f"run 1: session file written ({sessions})")

# --- run 2: restart with no note argument — the session comes back ----------
start_run(["--vault", VAULT])
check(pump(1.5), "run 2: app alive")
check("long-read.md" in status_line(), "run 2: the last note is restored")
check(
    content_top().startswith("Paragraph 2:"),
    "run 2: scroll restored (Paragraph 2 on the first text row)",
)
check(not content_has("Long read"), "run 2: scroll restored (heading still off)")
check(sidebar_has("old-note.md"), "run 2: archive still expanded in the tree")
check(
    not content_has("Lorem Index"),
    "run 2: the index content did not come back (session note wins)",
)
check(wait_exit(), "run 2: clean exit code 0")

# --- run 3: an explicit note argument wins over the session ------------------
start_run(["--vault", VAULT, f"{VAULT}/index.md"])
check(pump(1.5), "run 3: app alive")
check(content_has("Lorem Index"), "run 3: the explicit argument opened")
check("long-read.md" not in status_line(), "run 3: session note NOT restored")
check(wait_exit(), "run 3: clean exit code 0")

# --- run 4: a corrupted session file degrades silently -----------------------
session_file = os.path.join(SESSION_DIR, sessions[0])
with open(session_file, "w") as f:
    f.write("note = 42 !!! not toml !!!")
start_run(["--vault", VAULT])
check(pump(1.5), "run 4: app alive")
check(
    content_has("Lorem Index"),
    "run 4: corrupt session falls back to the index",
)
check("long-read.md" not in status_line(), "run 4: nothing restored from garbage")
check(wait_exit(), "run 4: clean exit code 0")

print("SMOKE M5:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
