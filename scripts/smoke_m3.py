#!/usr/bin/env python3
"""Smoke test for M3 phase 2 (vault integration) using pyte.

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p app`); binary path overridable
  with $FROND_APP_BIN.

The script builds a small fixture vault with cross-links under
/tmp/frond-m3-vault (writes ONLY there, never to testvault/) and drives the
app with `--vault`, exercising wikilink focus/open, the backlinks panel,
the tag view, and the status glyphs. Later loops added: mouse-click
activation of a wikilink (SGR 1006 injection, as in smoke_m2), the
disambiguation popup end-to-end, `[[note#Heading]]` anchor navigation,
and backlinks panel selection.
"""
import os, pty, select, struct, sys, shutil, time, fcntl, termios

import pyte

_APP_DEFAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond")
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
VAULT = "/tmp/frond-m3-vault"

COLS, ROWS = 100, 30

# --- fixture vault -------------------------------------------------------
shutil.rmtree(VAULT, ignore_errors=True)
BODY = "filler paragraph\n\n" * 40
for path, body in [
    ("index.md", "# Index\n\nSee [[alpha]] and [[beta]] and [[nowhere]]. Pick [[dup]], and jump [[alpha#Deep Section]] by anchor.\n\n" + BODY),
    ("alpha.md", "---\nstatus: complete\ntags: [rust, mocktag]\n---\n# Alpha\n\nBack to [[index]].\n\n" + BODY + "\n## Deep Section\n\ndeep-section-anchor-line\n\n" + BODY),
    ("beta.md", "---\nstatus: pending\ntags: [rust]\n---\n# Beta\n\nHome: [[index|home]].\n\n" + BODY),
    # Two same-stem notes in different folders: [[dup]] from index.md (the
    # vault root, no folder hint) must stay ambiguous -> popup.
    ("docs/dup.md", "# Dup One\n\nunique-one-body\n\n" + BODY),
    ("refs/dup.md", "# Dup Two\n\nunique-two-body\n\n" + BODY),
]:
    full = os.path.join(VAULT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(body)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    # The quit-time session save must not touch the real ~/.local/state.
    os.environ["FROND_SESSION"] = "/tmp/frond-m3-session"
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

ok = True
def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())

# --- boot ----------------------------------------------------------------
check(pump(1.5), "app alive")
check("Index" in body_text(), "index.md content rendered")
check("alpha" in body_text(), "wikilink display text rendered (not raw brackets)")
check("[[" not in body_text(), "no raw [[...]] syntax left in the body")
check("nowhere" in body_text(), "unresolved link text still visible")
check("alpha.md" in body_text(), "sidebar lists notes")
check("●" in body_text(), "complete glyph (●) in the tree")
check("○" in body_text(), "pending glyph (○) in the tree")

# --- wikilink focus + open ------------------------------------------------
send("x")    # focus the first link ([[alpha]])
pump(0.3)
send("\r")   # Enter opens it
pump(0.8)
check("alpha.md" in status_line(), "Enter opened the focused wikilink (alpha.md)")
check("Alpha" in body_text(), "alpha note content shown")

send("x")    # alpha's first link is [[index]]
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
check("backlinks (2)" in body_text(), "backlinks panel with count (alpha+beta link here)")
check("alpha.md" in body_text() and "beta.md" in body_text(), "both backlinks listed")
send("\r")   # open the selected backlink (alpha.md)
pump(0.8)
check("alpha.md" in status_line(), "Enter opened the selected backlink")
send("b")
pump(0.3)
check("backlinks (" not in body_text(), "b closes the panel")

# --- tag view -------------------------------------------------------------
send("t")    # sidebar: tree -> tags
pump(0.4)
check("#rust (2)" in body_text(), "tag list shows #rust with count 2")
check("#mocktag (1)" in body_text(), "tag list shows #mocktag")
# The tree block is replaced by the tags block in tag mode (the " vault "
# title only renders in tree mode; "alpha.md" alone is no good here — the
# opened note's path shows it in the status bar).
check(" vault " not in body_text(), "sidebar switched to tags (tree block gone)")
send("\r")   # expand the selected tag
pump(0.3)
check("alpha.md" in body_text(), "expanded tag lists its notes")
send("j")    # onto a note row
pump(0.2)
send("\r")   # open it
pump(0.8)
check(
    "alpha.md" in status_line() or "beta.md" in status_line(),
    "tag view opened a note",
)
send("t")    # back to tree mode
pump(0.3)
# A real tree row: the glyph+name rows only render in tree mode (the
# status bar path alone could not prove the sidebar switched back).
check(
    " vault " in body_text() and "● alpha.md" in body_text(),
    "tree mode restored: tree rows (with glyphs) return",
)

# --- click activation + disambiguation popup end-to-end ------------------
# Back to index.md via a sidebar click: find the index.md row in the
# sidebar column (x < 30) of the tree and click it.
def sidebar_pos_of(text):
    for y in range(ROWS - 1):
        idx = row(y).find(text, 0, 30)
        if idx != -1:
            return y, idx
    return None

def content_pos_of(text):
    # First occurrence in the content pane only (the sidebar starts at
    # x=30; the status line is excluded by the range).
    for y in range(ROWS - 1):
        idx = row(y).find(text, 30)
        if idx != -1:
            return y, idx
    return None

pos = sidebar_pos_of("index.md")
check(pos is not None, "click section precondition: index.md row in the sidebar")
click(pos[1] + 1, pos[0])
pump(0.8)
check("index.md" in status_line(), "sidebar click opened index.md")
send("g")   # top of the note
pump(0.4)

# [[dup]] has two same-stem candidates (docs/dup.md, refs/dup.md) and
# index.md sits at the vault root (no folder hint): clicking the rendered
# link must open the disambiguation popup, not guess.
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
check("docs/dup.md" in status_line(), "status bar shows the disambiguated path")

# Same link again, second candidate this time: j moves the popup
# selection, Enter opens refs/dup.md.
pos = sidebar_pos_of("index.md")
click(pos[1] + 1, pos[0])
pump(0.8)
send("g")
pump(0.4)
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
check("refs/dup.md" in status_line(), "status bar shows refs/dup.md")

# --- heading anchor: [[alpha#Deep Section]] opens scrolled ----------------
pos = sidebar_pos_of("index.md")
click(pos[1] + 1, pos[0])
pump(0.8)
send("g")
pump(0.4)
# The anchor link renders as "alpha#Deep Section" (with the link icon
# prefix); the display may wrap between "Deep" and "Section", so anchor
# the click on "Deep" — still inside the link's clickable region.
pos = content_pos_of("Deep")
check(pos is not None, "anchor link rendered in the content pane")
click(pos[1] + 2, pos[0])
pump(0.8)
check("alpha.md" in status_line(), "anchor link opened alpha.md")
check("Deep Section" in body_text(), "opened scrolled to the anchored section")
check("deep-section-anchor-line" in body_text(), "the section body line is visible")
check("Alpha" not in body_text(), "the rendered H1 'Alpha' is NOT on screen (really scrolled)")

# --- backlinks panel: open, move the selection, open ----------------------
# index.md has two backlinks (alpha, beta): select the second and open it.
pos = sidebar_pos_of("index.md")
click(pos[1] + 1, pos[0])
pump(0.8)
send("b")
pump(0.4)
check("backlinks (2)" in body_text(), "backlinks panel reopened on index.md")
send("j")    # onto the second backlink row
pump(0.3)
send("\r")
pump(0.8)
check(
    "beta.md" in status_line() and "Beta" in body_text(),
    "backlinks j + Enter opened the second backlink (beta.md)",
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
