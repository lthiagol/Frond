#!/usr/bin/env python3
"""Smoke test for M4 (global search, git sync status, changed-file
markers) plus the post-v1 loop-4 features (search-row click-select,
wheel-over-results, guarded checkbox toggling) using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- `git` on PATH (fixture repository setup only; the app itself only ever
  runs read-only git commands plus the guarded checkbox write).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script builds a small fixture vault under /tmp/frond-m4-vault, turns it
into a git repository (writes ONLY there, never to testvault/), and drives
the app with `--vault`, exercising the search popup flow (keyboard AND
mouse), the git status segment, the sidebar changed-file markers, the
guarded checkbox toggle (click → on-disk marker + `updated:` bump, and
its dirty-tree refusal), and help.
"""
import datetime
import os, pty, select, struct, sys, shutil, time, fcntl, termios

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
VAULT = smoke_tmp.path("frond-m4-vault")

COLS, ROWS = 100, 30

# --- fixture vault + git repository --------------------------------------
shutil.rmtree(VAULT, ignore_errors=True)
BODY = "filler paragraph\n\n" * 40
INDEX = (
    "---\nstatus: complete\nupdated: 2026-01-01\n---\n"
    "# Index\n\nSee [[alpha]].\n\nneedle in index\n\n- [ ] a task\n\n" + BODY
)
for path, body in [
    ("index.md", INDEX),
    ("alpha.md", "---\nstatus: complete\n---\n# Alpha\n\nThe needle lives here.\n\n" + BODY),
    ("beta.md", "# Beta\n\nuntouched note\n\n" + BODY),
]:
    full = os.path.join(VAULT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(body)


def git(*args):
    import subprocess
    env = dict(
        os.environ,
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@example.com",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@example.com",
    )
    out = subprocess.run(["git", *args], cwd=VAULT, env=env, capture_output=True)
    assert out.returncode == 0, f"git {args} failed: {out.stderr.decode()}"


git("init", "-b", "main", "-q")
git("add", "-A")
git("commit", "-q", "-m", "initial")

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    # The quit-time session save must not touch the real ~/.local/state.
    os.environ["FROND_SESSION"] = smoke_tmp.path("frond-m4-session")
    # NO_COLOR would strip every SGR color; the mouse-flow checks compare
    # cell backgrounds, so the child must emit colors.
    os.environ.pop("NO_COLOR", None)
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    os.execv(APP, [APP, "--vault", VAULT, f"{VAULT}/index.md"])

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


def send(s):
    os.write(fd, s.encode())


def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))


def body_text():
    return "\n".join(row(y) for y in range(ROWS))


def status_line():
    return row(ROWS - 1)


def click(col, r):
    # SGR mouse encoding (SGR 1006, crossterm default): press + release,
    # 1-based coordinates.
    send(f"\x1b[<0;{col + 1};{r + 1}M\x1b[<0;{col + 1};{r + 1}m")


def wheel(down, col, r):
    # SGR 64 = wheel up, 65 = wheel down.
    send(f"\x1b[<{65 if down else 64};{col + 1};{r + 1}M")


def find_row(needle):
    for y in range(ROWS):
        if needle in row(y):
            return y
    return None


def cell_bg(col, r):
    return screen.buffer[r][col].bg


ok = True


def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())


# --- boot: clean git status ----------------------------------------------
check(pump(1.5), "app alive")
check("Index" in body_text(), "index.md content rendered")
check("✓" in status_line(), "clean repo shows the check mark in the status bar")
check("●" not in status_line(), "no dirty dot on a clean repo")

# --- search flow ----------------------------------------------------------
send("s")
pump(0.3)
check("/▏" in body_text(), "search prompt open with cursor")
for c in "needle":
    send(c)
pump(0.3)
check("/needle▏" in body_text(), "query typed into the prompt")
send("\r")
pump(0.4)
check("2 hits" in body_text(), "results mode with hit count (index.md + alpha.md)")
check("alpha.md:6: " in body_text(), "hit row shows relpath:line prefix")
check("needle" in body_text(), "hit snippet visible")
send("\r")
pump(0.8)
check("alpha.md" in status_line(), "Enter opened the hit's note")
check("needle lives here" in body_text(), "the hit's line is scrolled into view")
check("Index" not in body_text(), "previous note scrolled off (hit near the note end)")

# --- search popup: wheel + click over the results --------------------------
# Popup geometry for 2 hits at 100x30: width 70 → x 15; height 5 → y 12;
# inner result rows are screen rows 13 (hit 0) and 14 (hit 1).
POP_X, ROW0, ROW1 = 15, 13, 14
send("s")
pump(0.3)
for c in "needle":
    send(c)
send("\r")
pump(0.4)
check("2 hits" in body_text(), "results open again for the mouse flow")

# Wheel down over the popup moves the result selection (bg highlight to
# row 1); the note behind the popup must not scroll.
wheel(True, POP_X + 5, ROW1)
pump(0.3)
check(
    cell_bg(POP_X + 5, ROW1) != cell_bg(POP_X + 5, ROW0),
    "wheel over the results moved the selection highlight to row 1",
)
check("Alpha" in body_text(), "wheel over the popup did not scroll the note")
# Wheel up restores the selection to hit 0.
wheel(False, POP_X + 5, ROW0)
pump(0.3)
check(
    cell_bg(POP_X + 5, ROW0) != cell_bg(POP_X + 5, ROW1),
    "wheel up moved the selection back to row 0",
)

# A click selects the row; clicking the already-selected row opens it.
click(POP_X + 5, ROW1)
pump(0.3)
check(
    cell_bg(POP_X + 5, ROW1) != cell_bg(POP_X + 5, ROW0),
    "clicking a result row selects it",
)
check("search: 2 hits" in body_text(), "the selection click did not open yet")
click(POP_X + 5, ROW1)
pump(0.8)
check("index.md" in status_line(), "clicking the selected row opened it")
check("needle in index" in body_text(), "opened at the clicked hit's line")
check(not ("/" in status_line() and "▏" in status_line()), "popup closed after open")

# --- guarded checkbox toggle (clean tree) ----------------------------------
task_row = find_row("a task")
check(task_row is not None, "task line rendered with its checkbox")
# Sidebar is 30 cols, then the note's left margin (default 2); a depth-1
# checkbox glyph sits at the first text column after it.
# Cycle: - [ ] -> - [~] (in-progress, rendered ◐) -> - [x] -> - [ ].
CONTENT_X = 30 + 2
click(CONTENT_X, task_row)
pump(0.8)
with open(os.path.join(VAULT, "index.md")) as f:
    on_disk = f.read()
today = datetime.date.today().isoformat()
check("- [~] a task" in on_disk, "first click wrote the in-progress state - [~]")
check(f"updated: {today}" in on_disk, "frontmatter updated: bumped to today")
check("updated: 2026-01-01" not in on_disk, "old updated value replaced")
check("◐ a task" in body_text(), "the reloaded render shows the in-progress glyph")
check("●" in status_line(), "git badge went dirty right after the write")
check("index.md *" in body_text(), "the tree marks the written note")

# A further toggle is refused while the tree is dirty — commit first, then
# the cycle continues to the checked state.
git("add", "-A")
git("commit", "-q", "-m", "toggle 1")
send("u")
pump(0.8)
check("✓" in status_line(), "clean again after committing the toggle")
click(CONTENT_X, task_row)
pump(0.8)
with open(os.path.join(VAULT, "index.md")) as f:
    on_disk = f.read()
check("- [x] a task" in on_disk, "second click wrote the checked state - [x]")
check("☑" in body_text(), "the reloaded render shows the checked glyph")
click(CONTENT_X, task_row)
pump(0.8)
with open(os.path.join(VAULT, "index.md")) as f:
    on_disk = f.read()
check("- [x] a task" in on_disk, "second click wrote the checked state - [x]")
check("☑" in body_text(), "the reloaded render shows the checked glyph")

# --- git dirty + changed-file markers -------------------------------------
with open(os.path.join(VAULT, "alpha.md"), "a") as f:
    f.write("dirty edit\n")
send("u")  # explicit refresh
pump(0.8)
check("●" in status_line(), "dirty dot after u on a modified repo")
check("✓" not in status_line(), "check mark gone when dirty")
check("alpha.md *" in body_text(), "changed note marked with * in the tree")
check("beta.md *" not in body_text(), "unchanged note unmarked")

# --- guarded toggle refuses on a dirty tree --------------------------------
click(CONTENT_X, find_row("a task"))
pump(0.5)
with open(os.path.join(VAULT, "index.md")) as f:
    on_disk = f.read()
check("not writing" in status_line(), "dirty tree refusal surfaced in the status bar")
check("- [x] a task" in on_disk and "- [~]" not in on_disk, "refusal touched nothing")
check("dirty" in status_line(), "refusal names the reason")

# --- commit everything: writes allowed again, status clean -----------------
git("add", "-A")
git("commit", "-q", "-m", "smoke")
send("u")
pump(0.8)
check("✓" in status_line(), "check mark back after committing")

# --- help documents the M4 + checkbox keys ---------------------------------
send("?")
pump(0.4)
body = body_text()
check("close esc/any key" in body, "help popup shown")
check("global search" in body, "help documents s")
check("refresh git status" in body, "help documents u")
check("changed since last sync" in body, "help documents the * marker")
check("unresolved (struck out, not activatable)" in body, "help documents unresolved links")
check("toggle checkbox" in body, "help documents the checkbox toggle")
send("x")  # any key closes help
pump(0.4)
check("close esc/any key" not in body_text(), "help popup closed")

# quit
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE M4:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
