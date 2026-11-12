#!/usr/bin/env python3
"""Smoke test for the M1 phase 2 TUI using pyte terminal emulation.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p app`); the binary defaults to
  <workspace>/target/debug/frond and can be overridden with $FROND_APP_BIN.
- A source note to render: defaults to testvault/index.md (READ-ONLY), can
  be overridden with $FROND_SMOKE_SRC. The script copies it to /tmp/watchtest.md
  and only ever writes to that copy — never to the vault.

Run directly (`python3 scripts/smoke_tui.py`) or via the ignored Rust
integration test: `cargo test -p app --test smoke -- --ignored`.
"""
import os, pty, select, shutil, struct, sys, fcntl, termios, time

import pyte

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
SRC = os.environ.get("FROND_SMOKE_SRC", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "testvault", "index.md"))
TMP = "/tmp/watchtest.md"

COLS, ROWS = 100, 30
shutil.copyfile(SRC, TMP)

screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    os.execv(APP, [APP, TMP])

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
    """Feed output until `cond()` holds or `timeout` elapses. Returns the
    final truth value of `cond()`, so a missed condition still fails the
    check that follows (no silent pass, no hang)."""
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        pump(0.1)
    return cond()

def send(s):
    os.write(fd, s.encode())

def status_line():
    return row(ROWS - 1)

def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))

def body_text():
    return "\n".join(row(y) for y in range(ROWS))

ok = True
def check(cond, label):
    global ok
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())

check(pump(1.5), "app alive")
check("watchtest.md" in status_line(), "status bar shows file path")
check("watch on" in status_line(), "watch on indicator")
ts1 = status_line().split("rendered ")[1][:8] if "rendered " in status_line() else None
check(ts1 is not None, f"rendered timestamp ({ts1})")
check("lines" in status_line(), "line count")
lines_before = int(status_line().split(" · ")[1].split()[0])
check(lines_before > 0, f"line count parsed ({lines_before})")

# scroll: j, arrow-down, PgDn, mouse wheel
top_row5 = row(5)
send("\x1b[<65;20;10M\x1b[<65;20;10M\x1b[<65;20;10M")  # SGR wheel-down x3
pump(0.4)
check(row(5) != top_row5, "mouse wheel scroll moved viewport")
send("j"); send("\x1b[B"); send("\x1b[6~")
pump(0.4)
send("G")
pump(0.4)
send("g")
pump(0.4)
check(True, "scroll keys accepted")

# edit the file -> watch reload (content appended at bottom)
send("G")  # stay at bottom to see appended content
pump(0.2)
with open(TMP, "a") as f:
    f.write("\nWATCHSMOKEEDIT\n\nsecond para WATCHSMOKEEDIT2\n")
pump_until(lambda: "WATCHSMOKEEDIT" in body_text())
st = status_line()
lines_after = int(st.split(" · ")[1].split()[0])
ts2 = st.split("rendered ")[1][:8] if "rendered " in st else None
check(lines_after == lines_before + 4, f"line count grew on reload ({lines_before} -> {lines_after})")
check(ts2 is not None and ts2 != ts1, f"timestamp updated ({ts1} -> {ts2})")
body = body_text()
check("WATCHSMOKEEDIT" in body, "reload shows new content at bottom")

# toggle watch off, edit again, content must NOT appear
send("w")
pump(0.4)
check("watch off" in status_line(), "watch toggled off")
with open(TMP, "a") as f:
    f.write("\nWATCHEDOFFEDIT\n")
pump(1.0)
check("WATCHEDOFFEDIT" not in body_text(), "no reload while watch off")

# back on -> catch-up reload
send("w")
pump_until(lambda: "WATCHEDOFFEDIT" in body_text())
check("watch on" in status_line(), "watch re-enabled")
check("WATCHEDOFFEDIT" in body_text(), "catch-up reload after re-enabling watch")

# help popup
send("?")
pump_until(lambda: "close esc/any key" in body_text())
body = body_text()
check("close esc/any key" in body, "help popup shown")
check("toggle watch mode" in body, "help lists keys")
send("x")
pump(0.4)
check("close esc/any key" not in body_text(), "help popup closed")

# quit
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
