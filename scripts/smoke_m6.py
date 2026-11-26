#!/usr/bin/env python3
"""Smoke test for the 0.6.x feature set that only had unit coverage:

- **theme groups** (`Ctrl+T` changes the `Shift+T` rotation group, all →
  light → dark; `Shift+T` moves the theme *inside* the group),
- **status-bar icons** (config `icons = "unicode"` → `¶` for a note,
  `▦` for a `.base` table),
- **`.base` tables in the sidebar tree** (the row opens the table view,
  `]` / `1` switch views).

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script builds a fixture vault under the private smoke scratch dir
(`scripts/smoke_tmp.py`) and drives the app with `--vault`. The config and
the theme directory are redirected into the same scratch root
(FROND_CONFIG / FROND_THEMES), so the real `~/.config` is never touched —
which also keeps the theme counts deterministic (no custom themes).
"""
import os
import pty
import select
import struct
import sys
import shutil
import time
import fcntl
import termios
import re

import pyte
import smoke_tmp

_APP_DEFAULT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "frond"
)
APP = os.environ.get("FROND_APP_BIN", _APP_DEFAULT)
VAULT = smoke_tmp.path("frond-m6-vault")
CONFIG_FILE = smoke_tmp.path("frond-m6-config.toml")
THEME_DIR = smoke_tmp.path("frond-m6-themes")
SESSION_DIR = smoke_tmp.path("frond-m6-session")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

COLS, ROWS = 100, 30

# --- theme inventory (kept in sync with the app's own tables) --------------
# The 4 presets (crates/app/src/theme.rs): arctic is light, the other three
# are dark. The 14 built-ins are parsed from the source so a new built-in
# theme does not silently break the expected group counts.
PRESET_LIGHT = 1  # arctic
PRESET_DARK = 3  # forest, ocean, solarized


def builtin_appearances():
    """{name: "light" | "dark"} for the built-in themes.

    Parsed from crates/app/src/builtin_themes.rs: every entry lists its
    `name:` followed by its `appearance: Appearance::Light|Dark`.
    """
    src = open(
        os.path.join(REPO, "crates", "app", "src", "builtin_themes.rs"),
        encoding="utf-8",
    ).read()
    pairs = re.findall(
        r'name:\s*"([^"]+)",\s*appearance:\s*Appearance::(Light|Dark)', src
    )
    if not pairs:
        raise SystemExit("could not parse the built-in theme table")
    return {name: appearance.lower() for name, appearance in pairs}


BUILTIN = builtin_appearances()
LIGHT_COUNT = PRESET_LIGHT + sum(1 for a in BUILTIN.values() if a == "light")
DARK_COUNT = PRESET_DARK + sum(1 for a in BUILTIN.values() if a == "dark")
ALL_COUNT = LIGHT_COUNT + DARK_COUNT
LIGHT_NAMES = {n for n, a in BUILTIN.items() if a == "light"} | {"arctic"}
DARK_NAMES = {n for n, a in BUILTIN.items() if a == "dark"} | {
    "forest",
    "ocean",
    "solarized",
}

# --- fixture vault ---------------------------------------------------------
shutil.rmtree(VAULT, ignore_errors=True)
os.makedirs(VAULT, exist_ok=True)

NOTES = {
    "index.md": (
        "---\nstatus: complete\n---\n\n"
        "# Smoke Heading\n\nA body paragraph for the heading color check.\n\n"
        + "Filler paragraph for the scroll target.\n\n" * 20
        + "## Second Section\n\nJump target.\n\n"
        "### Third Section\n\nDeeper heading for the indentation check.\n\n"
        + "Tail paragraph after the jump target.\n\n" * 25
    ),
    "alpha.md": "---\ntype: task\nstatus: active\n---\n\n# Alpha\n\nalpha body\n",
    "beta.md": "---\ntype: task\nstatus: draft\n---\n\n# Beta\n\nbeta body\n",
}
for name, body in NOTES.items():
    with open(os.path.join(VAULT, name), "w") as f:
        f.write(body)

BASE = """views:
  - type: table
    name: Tasks
    filters:
      and:
        - 'type == "task"'
    order:
      - file.name
      - status
  - type: table
    name: All
    order:
      - file.name
"""
with open(os.path.join(VAULT, "tasks.base"), "w") as f:
    f.write(BASE)

# Config fixture: unicode status-bar icons and the "all" rotation group.
os.makedirs(THEME_DIR, exist_ok=True)
with open(CONFIG_FILE, "w") as f:
    f.write('icons = "unicode"\ntheme_rotation = "all"\n')

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    # The color checks need real SGR colors: crossterm honors NO_COLOR.
    os.environ.pop("NO_COLOR", None)
    # Test/smoke-only redirections: never touch the real ~/.config, the
    # real theme dir or the real session dir.
    os.environ["FROND_CONFIG"] = CONFIG_FILE
    os.environ["FROND_THEMES"] = THEME_DIR
    os.environ["FROND_SESSION"] = SESSION_DIR
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


def row(y):
    return "".join(screen.buffer[y][x].data or " " for x in range(COLS))


def body_text():
    return "\n".join(row(y) for y in range(ROWS))


def status_line():
    return row(ROWS - 1)


def heading_fg():
    """fg color of the note's H1, if it is on screen."""
    for y in range(ROWS - 1):
        text = row(y)
        if "Smoke Heading" in text:
            x = text.index("Smoke Heading")
            return screen.buffer[y][x].fg
    return None


def sidebar_row(needle):
    """Screen row of a sidebar entry (the tree lives in the left columns)."""
    for y in range(1, ROWS - 1):
        text = "".join(screen.buffer[y][x].data or " " for x in range(29))
        if needle in text:
            return y
    return None


def popup_result_rows():
    """The result rows of the open quick-jumper popup (between the query
    prompt and the footer). The sidebar still lists every note, so the
    checks must look at the popup only."""
    prompt = footer = None
    for y in range(ROWS):
        text = row(y)
        if ">▏" in text or ">b▏" in text:
            prompt = y
        if "↑/↓ move" in text:
            footer = y
    if prompt is None or footer is None:
        return ""
    return "\n".join(row(y) for y in range(prompt + 1, footer))


def click(col, r):
    # SGR mouse encoding (SGR 1006, crossterm default): press + release,
    # button 0 = left, 1-based coordinates.
    send(f"\x1b[<0;{col + 1};{r + 1}M\x1b[<0;{col + 1};{r + 1}m")


ok = True


def check(cond, label):
    global ok
    ok = ok and bool(cond)
    print(("PASS" if cond else "FAIL"), label)
    if not cond:
        print("--- status line:", repr(status_line()))
        print("--- screen ---")
        print(body_text())


# --- boot ------------------------------------------------------------------
check(pump(1.5), "app alive")
check("Smoke Heading" in body_text(), "index.md rendered")

# --- status-bar icons (config `icons = "unicode"`) -------------------------
check(
    "¶ index.md" in status_line(),
    "unicode note icon (¶) beside the note name",
)
check(
    "\ue73e" not in status_line(),
    "the nerd-font note glyph (U+E73E) is not used in unicode mode",
)

# --- the outline (o) -------------------------------------------------------
send("f")  # focus the sidebar (tree view)
pump(0.3)
send("o")
pump(0.4)
check("outline" in body_text(), "o switches the sidebar to the outline")
check(
    sidebar_row("Smoke Heading") is not None,
    "the open note's first heading is listed",
)
check(
    sidebar_row("Second Section") is not None,
    "every heading is listed in document order",
)
check(
    sidebar_row("alpha.md") is None,
    "tree rows are gone in outline mode",
)

send("j")  # select the second heading
pump(0.3)
check(
    "Second Section" in status_line(),
    "the status bar shows the selected heading",
)
send("\r")  # Enter jumps to it
pump(0.5)
check(
    "Second Section" in row(1),  # the default margin_y = 1: text starts here
    "Enter scrolled the note to the selected heading",
)

send("o")  # outline off: back to the view it was over (the tree)
pump(0.4)
check(
    sidebar_row("alpha.md") is not None,
    "o again returns to the tree",
)
check("outline" not in body_text(), "the outline panel is gone")
send("g")  # back to the top of the note for the theme color checks
pump(0.3)

# --- theme groups (Ctrl+T) -------------------------------------------------
OCEAN_HEADING = heading_fg()
check(OCEAN_HEADING is not None, "the H1 is on screen")
# The default theme is `ocean` (dark preset): heading_1 = rgb(140,190,255).
check(
    OCEAN_HEADING == "8cbeff",
    f"the default theme is ocean (heading fg {OCEAN_HEADING})",
)

send("\x14")  # Ctrl+T -> group: light
pump(0.4)
check(
    f"theme group: light ({LIGHT_COUNT} themes)" in status_line(),
    f"Ctrl+T moved the group to light ({LIGHT_COUNT} themes)",
)
check(
    heading_fg() == OCEAN_HEADING,
    "Ctrl+T changed the group only: the theme on screen did not move",
)

send("\x14")  # Ctrl+T -> group: dark
pump(0.4)
check(
    f"theme group: dark ({DARK_COUNT} themes)" in status_line(),
    f"Ctrl+T moved the group to dark ({DARK_COUNT} themes)",
)

send("\x14")  # Ctrl+T -> group: all
pump(0.4)
check(
    f"theme group: all ({ALL_COUNT} themes)" in status_line(),
    f"Ctrl+T moved the group to all ({ALL_COUNT} themes)",
)

# --- Shift+T inside the light group ---------------------------------------
send("\x14")  # back to light
pump(0.3)
send("T")  # Shift+T
pump(0.5)
light_theme = None
m = re.search(r"theme: (\S+)", status_line())
if m:
    light_theme = m.group(1)
check(light_theme is not None, "Shift+T reported the new theme")
check(
    light_theme in LIGHT_NAMES,
    f"Shift+T inside the light group landed on a light theme ({light_theme})",
)
LIGHT_HEADING = heading_fg()
check(
    LIGHT_HEADING is not None and LIGHT_HEADING != OCEAN_HEADING,
    f"the light theme really applied (heading fg {LIGHT_HEADING})",
)

# --- Shift+T inside the dark group ----------------------------------------
send("\x14")  # light -> dark
pump(0.3)
send("\x14")  # dark -> all
pump(0.3)
send("\x14")  # all -> light
pump(0.3)
send("\x14")  # light -> dark
pump(0.4)
check(
    f"theme group: dark ({DARK_COUNT} themes)" in status_line(),
    "the group walked all -> light -> dark again",
)
send("T")  # Shift+T
pump(0.5)
dark_theme = None
m = re.search(r"theme: (\S+)", status_line())
if m:
    dark_theme = m.group(1)
check(dark_theme is not None, "Shift+T reported the new theme")
check(
    dark_theme in DARK_NAMES,
    f"Shift+T inside the dark group landed on a dark theme ({dark_theme})",
)
DARK_HEADING = heading_fg()
check(
    DARK_HEADING is not None and DARK_HEADING != LIGHT_HEADING,
    f"the dark theme really applied (heading fg {DARK_HEADING})",
)

# --- the quick jumper (Ctrl+K) -------------------------------------------
send("\x0b")  # Ctrl+K
pump(0.5)
check("jump:" in body_text(), "Ctrl+K opened the quick jumper")
check(">▏" in body_text(), "the jumper shows an empty query prompt")
check("alpha.md" in body_text(), "every note is listed before typing")

send("b")
pump(0.4)
check(">b▏" in body_text(), "the query is typed into the prompt")
results = popup_result_rows()
check("beta.md" in results, "the matching note is listed")
check("alpha.md" not in results, "the non-matching note is filtered out")

send("\x1b[A")  # Up
pump(0.3)
send("\r")  # Enter opens the selected note
pump(0.8)
check("beta.md" in status_line(), "Enter opened the selected note")
check("Beta" in body_text(), "the opened note is rendered")

# --- .base tables in the sidebar tree -------------------------------------
check("tasks.base" in body_text(), "the .base file is a tree row")
check(
    sidebar_row("tasks.base") is not None,
    "the .base row is inside the sidebar column",
)

send("f")  # focus the sidebar
pump(0.3)
selected = False
for _ in range(6):
    if "tasks.base" in status_line():
        selected = True
        break
    send("j")
    pump(0.2)
check(selected, "the .base row can be selected in the tree")
check(
    "▦ tasks.base" in status_line(),
    "unicode base icon (▦) beside the selected .base row",
)

send("\r")  # Enter opens the table
pump(0.8)
check(
    "base · view 1/2: Tasks" in body_text(),
    "Enter on the .base row opened the table view",
)
check(
    "2 rows · view 1/2: Tasks" in status_line(),
    "the status bar shows the table state (2 rows, view 1/2)",
)
check(
    "▦ tasks.base" in status_line(),
    "the status bar shows the base icon beside the open table",
)
check("Name" in body_text() and "alpha" in body_text(), "the table shows its rows")

send("]")  # next view
pump(0.5)
check(
    "view 2/2: All" in status_line(),
    "] moved to the second view",
)
check("3 rows · view 2/2: All" in status_line(), "the second view lists every note")

send("1")  # jump back to the first view
pump(0.5)
check(
    "2 rows · view 1/2: Tasks" in status_line(),
    "the digit key jumped back to view 1",
)

# --- clicking the .base row opens the table too ---------------------------
send("\x7f")  # Backspace -> back to the note
pump(0.8)
check("beta.md" in status_line(), "Backspace returned to the note")
base_row = sidebar_row("tasks.base")
check(base_row is not None, "the .base row is still visible")
if base_row is not None:
    click(5, base_row)
    pump(0.8)
    check(
        "base · view 1/2: Tasks" in body_text(),
        "clicking the .base row opens the table view",
    )

# --- quit ------------------------------------------------------------------
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE M6:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
