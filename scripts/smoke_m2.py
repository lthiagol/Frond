#!/usr/bin/env python3
"""Smoke test for the M2 shell (sidebar + navigation history) using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script builds a small fixture vault under /tmp/frond-m2-vault (writes
ONLY there, never to testvault/) and drives the app with `--vault`.

Theme checks run against /tmp theme+config fixtures injected via the
FROND_THEMES/FROND_CONFIG env overrides (startup config theme, Shift+T
cycling, config hot-reload) — the real ~/.config is never touched.
"""
import os, pty, select, struct, sys, shutil, time, fcntl, termios

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
VAULT = smoke_tmp.path("frond-m2-vault")

COLS, ROWS = 100, 30

# --- fixture vault -------------------------------------------------------
shutil.rmtree(VAULT, ignore_errors=True)
LONG = "a-note-with-a-really-long-name-that-must-truncate.md"

# Theme fixtures (also /tmp-only): a custom theme (frond TOML format) whose
# heading_1 is a distinctive #ff00ff, plus a config.toml selecting it. The app under
# test is started with FROND_CONFIG/FROND_THEMES pointing here — the real
# ~/.config is never touched.
THEME_DIR = smoke_tmp.path("frond-m2-themes", "themes")
CONFIG_FILE = smoke_tmp.path("frond-m2-themes", "config.toml")
shutil.rmtree(smoke_tmp.path("frond-m2-themes"), ignore_errors=True)
os.makedirs(THEME_DIR, exist_ok=True)
with open(os.path.join(THEME_DIR, "nightshade.toml"), "w") as f:
    f.write(
        'base = "ocean"\n'
        'appearance = "dark"\n'
        'syntax = "base16-ocean.dark"\n'
        "\n[ui]\n"
        'content_bg = "#010203"\n'
        "\n[markdown]\n"
        'heading_1 = "#ff00ff"\n'
    )
with open(CONFIG_FILE, "w") as f:
    f.write('theme = "nightshade"\n')

for path, body in [
    ("index.md", "# Vault index\n\n" + "intro paragraph\n\n" * 60),
    ("alpha.md", "# Alpha\n\n" + "alpha body\n\n" * 60),
    (f"projects/{LONG}", "# Long\n\n" + "long body\n\n" * 60 + "END-OF-LONG-NOTE\n"),
    ("notes.txt", "not a note"),
]:
    full = os.path.join(VAULT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(body)
os.makedirs(f"{VAULT}/.obsidian", exist_ok=True)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    # Theme checks assert real colors: crossterm honors NO_COLOR, so the
    # child must not inherit a color-stripping environment.
    os.environ.pop("NO_COLOR", None)
    # Theme fixture injection (tests/smokes only): config path + theme dir
    # redirected into /tmp. The quit-time session save is redirected too.
    os.environ["FROND_CONFIG"] = CONFIG_FILE
    os.environ["FROND_THEMES"] = THEME_DIR
    os.environ["FROND_SESSION"] = smoke_tmp.path("frond-m2-session")
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

def pump_until(cond, timeout=5.0):
    """Feed output until `cond()` holds or `timeout` elapses; returns the
    final truth of `cond()` so a missed condition still fails its check."""
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        pump(0.1)
    return cond()

def send(s):
    os.write(fd, s.encode())

def click(col, row):
    # SGR mouse encoding (SGR 1006, crossterm default): press + release,
    # button 0 = left, coordinates 1-based.
    send(f"\x1b[<0;{col + 1};{row + 1}M\x1b[<0;{col + 1};{row + 1}m")

def wheel(down, col, row):
    # 65 = wheel down, 64 = wheel up; scroll events have no release.
    send(f"\x1b[<{65 if down else 64};{col + 1};{row + 1}M")

def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))

def body_text():
    return "\n".join(row(y) for y in range(ROWS))

def status_line():
    return row(ROWS - 1)

ok = True
def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())

# --- sidebar layout ------------------------------------------------------
check(pump(1.5), "app alive")
check("vault" in row(0), "sidebar visible by default with --vault")
check("index.md" in body_text(), "sidebar lists notes")
check("projects" in body_text(), "sidebar lists folders")
check("notes.txt" not in body_text(), "non-md files skipped")
check(".obsidian" not in body_text(), "hidden dirs skipped")
check("Vault index" in body_text(), "content shows index.md")

def screen_no_status():
    return "\n".join(row(y) for y in range(ROWS - 1))

# --- tab toggle ----------------------------------------------------------
send("\t")
pump(0.4)
check("index.md" not in screen_no_status(), "Tab hides sidebar")
send("\t")
pump(0.4)
check("index.md" in screen_no_status(), "Tab shows sidebar again")

# --- folder expand + open a note -----------------------------------------
send("h")  # focus sidebar
pump(0.3)
check(
    "projects ·" in status_line() and f"{VAULT}/projects" not in status_line(),
    "status bar shows the focused folder name, not its path",
)
send("l")  # expand projects
pump(0.3)
check(LONG[:20] in body_text(), "folder expanded, child visible")
check("…" in body_text(), "long name truncated with ellipsis")
send("j")  # onto the long note
pump(0.2)
send("\r")  # Enter opens it
pump_until(lambda: "Long" in body_text())
check(
    "Long" in body_text() and "END-OF-LONG-NOTE" not in body_text(),
    "Enter opened the nested note at the top (end marker not yet visible)",
)
check(LONG in status_line(), "status bar shows opened note path")

# --- history: back / forward ---------------------------------------------
# (focus is already on the content pane: Enter on a note moves it)
send("l")  # no-op from the content pane
pump(0.3)
send("G")  # bottom of the nested note
pump(0.4)
send("\x7f")  # Backspace -> index.md
pump_until(lambda: "Vault index" in body_text())
check("Vault index" in body_text(), "Backspace returned to index.md")
send("\x0f")  # Ctrl+O -> forward to the long note (scroll restored: bottom)
pump_until(lambda: "END-OF-LONG-NOTE" in body_text())
check(
    LONG in status_line() and "END-OF-LONG-NOTE" in body_text(),
    "Ctrl+O went forward: path restored AND last line visible (scroll at bottom)",
)

# --- watch reload still works with the new layout ------------------------
send("\t")  # hide sidebar (focus snaps to content)
pump(0.3)
check("alpha.md" not in screen_no_status(), "sidebar hidden again")
send("G")
pump(0.3)
with open(f"{VAULT}/projects/{LONG}", "a") as f:
    f.write("\nM2WATCHEDIT\n\nsecond para M2WATCHEDIT2\n")
pump(1.6)  # the watch tick (1 s) reloads the note; the viewport stays pinned
send("G")  # appended lines land below the pinned viewport; scroll to them
pump_until(lambda: "M2WATCHEDIT" in body_text())
check("M2WATCHEDIT" in body_text(), "watch reload works with sidebar layout")

# help popup documents the new keys
send("?")
pump_until(lambda: "close esc/any key" in body_text())
body = body_text()
check("close esc/any key" in body, "help popup shown")
check("Backspace" in body and "Ctrl+O" in body, "help documents history keys")
check("show / hide sidebar" in body, "help documents Tab")
send("x")
pump(0.4)
check("close esc/any key" not in body_text(), "help popup closed")

# --- sidebar reflects new file (loose rescan on watch reload) ------------
with open(f"{VAULT}/beta.md", "w") as f:
    f.write("# Beta\n")
send("\t")  # show sidebar
pump(0.3)
send("h")  # focus sidebar
pump(0.2)
with open(f"{VAULT}/projects/{LONG}", "a") as f:
    f.write("\nrescan tick\n")
pump_until(lambda: "beta.md" in body_text())
check("beta.md" in body_text(), "sidebar rescan picked up new file")

# --- mouse: sidebar clicks and wheel (SGR 1006 injected into the pty) ---
# Sidebar geometry at 100x30: width 30 (x 0..29), inner rows start at
# y=1 (below the top border). Visible tree rows (projects expanded):
# y=1 projects, y=2 LONG, y=3 alpha.md, y=4 beta.md, y=5 index.md.
check("a-note-with-a-really" in body_text(), "mouse section precondition: LONG row visible")

click(5, 1)  # on the `projects` folder row
pump(0.4)
check(
    "a-note-with-a-really" not in screen_no_status(),
    "click on a folder row collapses it",
)
click(5, 1)  # again: expands
pump(0.4)
check(
    "a-note-with-a-really" in screen_no_status(),
    "click on a collapsed folder row expands it",
)

click(5, 3)  # on the alpha.md row: opens it
pump_until(lambda: "alpha body" in body_text())
check("Alpha" in body_text() and "alpha body" in body_text(), "click on a note row opens it")
check("alpha.md" in status_line(), "status bar reflects the clicked note")

# Opening a note moves focus to the content pane: j scrolls the note (the
# top heading scrolls off), not the sidebar selection.
send("j")
pump(0.3)
check(
    "alpha.md" in status_line() and "Alpha" not in body_text(),
    "click opens the note focused on content (j scrolled the note)",
)

send("g")  # back to the top of alpha.md
pump(0.3)
check("Alpha" in body_text(), "g returns to the top of the note")
wheel(True, 5, 10)  # wheel down over the sidebar column
pump(0.3)
check("Alpha" in body_text(), "wheel over the sidebar does not scroll the content")
wheel(True, 60, 10)  # wheel down over the content pane
pump(0.3)
check("Alpha" not in body_text(), "wheel over the content pane scrolls the content")

# Clicks are gated by the help popup.
send("?")
pump(0.4)
check("close esc/any key" in body_text(), "help popup open for click gating")
click(5, 3)
pump(0.4)
check(
    "close esc/any key" in body_text(),
    "click while the help popup is open is ignored",
)
send("x")  # close the help popup
pump(0.3)

# --- theme support (fixture config + custom theme, all under /tmp) -------
# Back to index.md at the top: the theme checks read the '# Vault index'
# heading, whose fg proves the configured theme really applied. The nav
# stack is index -> LONG -> alpha here, so two Backspaces reach index.md.
send("\x7f")  # Backspace: back to the LONG note
pump(0.5)
send("\x7f")  # Backspace: back to index.md
pump(0.6)
send("g")  # top
pump(0.4)
check("Vault index" in body_text(), "theme section precondition: index.md on screen")

# The app runs with FROND_CONFIG -> fixture config.toml (theme = "nightshade")
# and FROND_THEMES -> the fixture theme dir. pyte keeps per-cell colors, so
# the rendered heading's fg proves the theme really applied.
def heading_fg():
    """fg color of the 'V' of the '# Vault index' heading, if on screen."""
    for y in range(ROWS - 1):
        text = row(y)
        if "Vault index" in text:
            x = text.index("Vault index")
            return screen.buffer[y][x].fg
    return None

check(heading_fg() == "ff00ff", "startup config theme applied (heading fg #ff00ff)")
check(
    any(
        screen.buffer[y][x].bg == "010203"
        for y in range(ROWS - 1)
        for x in range(30, COLS)
    ),
    "custom [ui] content_bg applied to the content area",
)

# Shift+T cycles to the next theme (nightshade -> arctic preset): the
# heading re-renders instantly with arctic's heading_1 #3a6ca8.
send("T")
pump(0.5)
check(heading_fg() == "3a6ca8", "Shift+T cycled to the next theme (arctic)")

# Hot reload: rewrite the fixture config with another theme; within the
# 1s config poll the app re-reads it and live-applies the theme key.
with open(CONFIG_FILE, "w") as f:
    f.write('theme = "forest"\n')
pump_until(lambda: heading_fg() == "94ccff")
check(heading_fg() == "94ccff", "config hot-reload live-applied the new theme")

# quit
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE M2:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
