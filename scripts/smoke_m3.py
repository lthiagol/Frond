#!/usr/bin/env python3
"""Smoke test for M3 phase 2 (vault integration) using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script copies the sample vault (testvault/, see docs/TESTVAULT.md) into
the scratch dir and drives the app with `--vault` on that copy, exercising
wikilink focus/open, the backlinks panel, the tag view, the status glyphs,
the ambiguous-link popup, and `[[note#Heading]]` anchor navigation. It writes
ONLY the copy, never testvault/.
"""
import os, pty, select, struct, sys, time, fcntl, termios

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)

COLS, ROWS = 100, 30
SIDEBAR_W = 30

# --- vault copy ----------------------------------------------------------
VAULT = smoke_tmp.copy_testvault("frond-m3-vault")

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    # The quit-time session save must not touch the real ~/.local/state.
    os.environ["FROND_SESSION"] = smoke_tmp.path("frond-m3-session")
    # Never the real ~/.config: a missing file is the default config.
    os.environ["FROND_CONFIG"] = smoke_tmp.path("frond-no-config.toml")
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

def click(col, row):
    # SGR mouse encoding (SGR 1006, crossterm default): press + release,
    # button 0 = left, coordinates 1-based (same helper as smoke_m2).
    send(f"\x1b[<0;{col + 1};{row + 1}M\x1b[<0;{col + 1};{row + 1}m")

def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))

def body_text():
    return "\n".join(row(y) for y in range(ROWS))

def status_line():
    return row(ROWS - 1)

def sidebar_has(text):
    return any(text in row(y)[:SIDEBAR_W] for y in range(ROWS - 1))

ok = True
def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())

def sidebar_pos_of(text):
    for y in range(ROWS - 1):
        idx = row(y).find(text, 0, SIDEBAR_W)
        if idx != -1:
            return y, idx
    return None

def content_pos_of(text):
    # First occurrence in the content pane only (the sidebar starts at
    # x=30; the status line is excluded by the range).
    for y in range(ROWS - 1):
        idx = row(y).find(text, SIDEBAR_W)
        if idx != -1:
            return y, idx
    return None

def content_has(text):
    return any(text in row(y)[SIDEBAR_W:] for y in range(ROWS - 1))

def open_index_from_sidebar(page=False):
    pos = sidebar_pos_of("index.md")
    check(pos is not None, "click section precondition: index.md row in the sidebar")
    if pos is None:
        return
    click(pos[1] + 1, pos[0])
    pump(0.8)
    check("index.md" in status_line(), "sidebar click opened index.md")
    send("g")   # top of the note
    pump(0.4)
    if page:    # the lower links sit past the first screen
        send("\x1b[6~")  # PageDown
        pump(0.4)

def back_to_index(page=False):
    # Backspace pops the nav stack back to index.md. Used after a link has
    # opened another note: the tree may have scrolled index.md out of view.
    send("\x7f")
    pump(0.8)
    check("index.md" in status_line(), "Backspace returned to index.md")
    send("g")
    pump(0.4)
    if page:
        send("\x1b[6~")  # PageDown
        pump(0.4)

# --- boot ----------------------------------------------------------------
check(pump(1.5), "app alive")
check("Lorem Index" in body_text(), "index.md heading rendered")
check(content_has("#alpha"), "wikilink display text rendered (not raw brackets)")
check("[[" not in body_text(), "no raw [[...]] syntax left in the body")
check(sidebar_has("alpha.md"), "sidebar lists notes")
check(sidebar_has("● alpha.md"), "complete glyph (●) in the tree")

# --- wikilink focus + open ------------------------------------------------
send("x")    # focus the first link ([[alpha]])
pump(0.3)
send("\r")   # Enter opens it
pump(0.8)
check("alpha.md" in status_line(), "Enter opened the focused wikilink (alpha.md)")
check("Alpha" in body_text(), "alpha note content shown")

send("x")    # alpha's only link is [[index]]
pump(0.3)
send("\r")
pump(0.8)
check("index.md" in status_line(), "second wikilink hop returns to index.md")

# Backspace still walks the nav stack (index -> alpha -> index).
send("\x7f")
pump(0.6)
check("alpha.md" in status_line(), "Backspace went back to alpha.md")
send("\x0f")
pump(0.6)
check("index.md" in status_line(), "Ctrl+O forward again")

# --- backlinks panel ------------------------------------------------------
send("b")
pump(0.4)
check("backlinks (3)" in body_text(), "backlinks panel with count")
check(content_has("alpha.md") and content_has("crlf.md"), "both backlinks listed in the panel")
send("\r")   # open the selected backlink (alpha.md)
pump(0.8)
check("alpha.md" in status_line(), "Enter opened the selected backlink")
send("b")
pump(0.3)
check("backlinks (" not in body_text(), "b closes the panel")
open_index_from_sidebar()

# --- tag view -------------------------------------------------------------
send("t")    # sidebar: tree -> tags
pump(0.4)
check("#rust (2)" in body_text(), "tag list shows #rust with count 2")
check("#mocktag, weird (1)" in body_text(), "tag list shows the comma tag #mocktag, weird")
# The tree block is replaced by the tags block in tag mode (the " vault "
# title only renders in tree mode; "alpha.md" alone is no good here — the
# opened note's path shows it in the status bar).
check(row(0).startswith("┌ tags"), "sidebar switched to tags (tree block gone)")
send("f")    # focus the sidebar: Enter acts on the sidebar only
pump(0.3)
send("\r")   # expand the selected tag (the first row; docs/TESTVAULT.md)
pump(0.3)
check(sidebar_has("alpha.md"), "expanded tag lists its notes in the sidebar")
send("j")    # onto the first note row
pump(0.2)
send("\r")   # open it
pump(0.8)
check("alpha.md" in status_line(), "tag view opened a note (alpha.md)")
send("t")    # back to tree mode
pump(0.3)
# A real tree row: the glyph+name rows only render in tree mode (the
# status bar path alone could not prove the sidebar switched back).
check(
    row(0).startswith("┌ vault") and sidebar_has("● alpha.md"),
    "tree mode restored: tree rows (with glyphs) return",
)

# --- pending glyph: a folder's notes only show once the folder is open ----
pos = sidebar_pos_of("▸ projects")
check(pos is not None, "projects folder row in the sidebar")
click(pos[1] + 1, pos[0])
pump(0.4)
check(sidebar_has("○"), "pending glyph (○) in the tree")

# --- click activation + disambiguation popup end-to-end ------------------
# Back to index.md via a sidebar click: find the index.md row in the
# sidebar column of the tree and click it.
open_index_from_sidebar(page=True)
check(content_has("#nowhere"), "broken link renders as #nowhere")
check("[[" not in body_text(), "no raw [[...]] in the lower links either")

# [[dup]] from index.md is ambiguous (docs/TESTVAULT.md, Links): the click
# must open the disambiguation popup, not guess.
pos = content_pos_of("dup")
check(pos is not None, "dup link rendered in the content pane")
click(pos[1] + 2, pos[0])
pump(0.5)
check("ambiguous link" in body_text(), "click on an ambiguous wikilink opens the popup")
check(
    "docs/dup.md" in body_text() and "refs/dup.md" in body_text(),
    "popup lists both same-stem candidates",
)
send("\r")   # Enter opens the selected candidate (first row)
pump(0.8)
check(
    "Dup One" in body_text() and "unique-one-body" in body_text(),
    "popup Enter opened the first candidate (docs/dup.md)",
)
check(
    "dup.md" in status_line() and "docs/dup.md" not in status_line(),
    "status bar shows the bare name of the opened note",
)

# Same link again, second candidate this time: j moves the popup
# selection, Enter opens refs/dup.md.
back_to_index(page=True)
pos = content_pos_of("dup")
click(pos[1] + 2, pos[0])
pump(0.5)
check("ambiguous link" in body_text(), "popup reopened for the second round")
send("j")    # selection onto the second candidate
pump(0.3)
send("\r")
pump(0.8)
check(
    "Dup Two" in body_text() and "unique-two-body" in body_text(),
    "j + Enter opened the second candidate (refs/dup.md)",
)
check(
    "dup.md" in status_line() and "refs/dup.md" not in status_line(),
    "status bar shows the bare name, not refs/dup.md",
)

# --- heading anchor: [[alpha#Deep Section]] opens scrolled ----------------
back_to_index()
# The link text may wrap between "Deep" and "Section": click on "Deep",
# still inside the link's clickable region.
pos = content_pos_of("Deep")
check(pos is not None, "anchor link rendered in the content pane")
click(pos[1] + 2, pos[0])
pump(0.8)
check("alpha.md" in status_line(), "anchor link opened alpha.md")
check("Deep Section" in body_text(), "opened scrolled to the anchored section")
check("deep-section-anchor-line" in body_text(), "the section body line is visible")
check("Alpha" not in body_text(), "the rendered H1 'Alpha' is NOT on screen (really scrolled)")

# [[long-read#Part 3]]: the anchor into a long note (docs/TESTVAULT.md,
# Links). Row 0 is the pane's top margin, so the heading is row 1.
back_to_index(page=True)
pos = content_pos_of("long-read")
check(pos is not None, "long-read anchor rendered in the content pane")
click(pos[1] + 2, pos[0])
pump(0.8)
check("long-read.md" in status_line(), "anchor link opened long-read.md")
check(
    row(1)[SIDEBAR_W:COLS - 1].strip() == "Part 3",
    "opened with the Part 3 heading on the top text row",
)

# --- backlinks panel: open, move the selection, open ----------------------
# index.md's backlinks (docs/TESTVAULT.md, Smoke suites): open the second.
back_to_index()
send("b")
pump(0.4)
check("backlinks (3)" in body_text(), "backlinks panel reopened on index.md")
send("j")    # onto the second backlink row
pump(0.3)
send("\r")
pump(0.8)
check(
    "crlf.md" in status_line() and "CRLF note" in body_text(),
    "backlinks j + Enter opened the second backlink (edge/crlf.md)",
)

# help documents the new keys
send("?")
pump(0.4)
body = body_text()
check("close esc/any key" in body, "help popup shown")
check("focus next / previous link" in body, "help documents x/X")
check("toggle backlinks panel" in body, "help documents b")
check("tree ⇄ tags" in body, "help documents t")
send("x")
pump(0.4)
check("close esc/any key" not in body_text(), "help popup closed")

# quit
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE M3:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
