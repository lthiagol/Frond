#!/usr/bin/env python3
"""Smoke test for the 0.6.x feature set that only had unit coverage:

- **theme groups** (`Ctrl+T` changes the `Shift+T` rotation group, all →
  light → dark; `Shift+T` moves the theme *inside* the group),
- **status-bar icons** (config `icons = "unicode"` → `¶` for a note,
  `▦` for a `.base` table),
- **the outline** (`o` lists the open note's headings; `Enter` jumps to one),
- **`.base` tables in the sidebar tree** (the row opens the table view,
  `]` / `1` switch views).

Prerequisites:
- python3 with `pyte` installed (`uv pip install pyte`, or run via `uv run --with pyte`).
- A debug build of the app (`cargo build -p frond`); binary path overridable
  with $FROND_APP_BIN.

The script copies the sample vault (`testvault/`, see docs/TESTVAULT.md) into
the private smoke scratch dir (`scripts/smoke_tmp.py`) and drives the app with
`--vault` on that copy; it never writes `testvault/`. The config and the theme
directory are redirected into the same scratch root (FROND_CONFIG /
FROND_THEMES), so the real `~/.config` is never touched — which also keeps the
theme counts deterministic (no custom themes).
"""
import os
import pty
import select
import struct
import sys
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
CONFIG_FILE = smoke_tmp.path("frond-m6-config.toml")
THEME_DIR = smoke_tmp.path("frond-m6-themes")
SESSION_DIR = smoke_tmp.path("frond-m6-session")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

COLS, ROWS = 100, 30
SIDEBAR_W = 30

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

# --- vault copy -------------------------------------------------------------
VAULT = smoke_tmp.copy_testvault("frond-m6-vault")

# --- config and theme dir: not vault content, built by the suite -----------
os.makedirs(THEME_DIR, exist_ok=True)
# Config fixture: unicode status-bar icons and the "all" rotation group.
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


def content_has(text):
    """`text` on some row of the content pane (right of the sidebar, above
    the status bar)."""
    return any(text in row(y)[SIDEBAR_W:] for y in range(ROWS - 1))


def content_row_has(*parts):
    """Some content-pane row holds every one of `parts`."""
    return any(
        all(part in row(y)[SIDEBAR_W:] for part in parts) for y in range(ROWS - 1)
    )


def heading_row(text):
    """Screen row of a content-pane line that begins with `text`, or None.
    The top text row is row 1 (the viewer has a one-row margin)."""
    words = text.split()
    for y in range(1, ROWS - 1):
        if row(y)[SIDEBAR_W:].split()[: len(words)] == words:
            return y
    return None


def heading_fg(text):
    """fg color of the content heading `text`, if it is on screen."""
    y = heading_row(text)
    if y is None:
        return None
    return screen.buffer[y][row(y).index(text, SIDEBAR_W)].fg


def sidebar_row(needle):
    """Screen row of a sidebar entry (the tree lives in the left columns)."""
    for y in range(1, ROWS - 1):
        text = "".join(screen.buffer[y][x].data or " " for x in range(SIDEBAR_W - 1))
        if needle in text:
            return y
    return None


def sidebar_has(text):
    """`text` on some row of the sidebar column (the panel's title counts)."""
    return any(text in row(y)[:SIDEBAR_W] for y in range(ROWS - 1))


def popup_result_rows():
    """The result rows of the open quick-jumper popup (between the query
    prompt and the footer). The sidebar still lists every note, so the
    checks must look at the popup only."""
    prompt = footer = None
    for y in range(ROWS):
        text = row(y)
        if re.search(r">[^▏│]*▏", text):
            prompt = y
        if "↑/↓ move" in text:
            footer = y
    if prompt is None or footer is None:
        return ""
    return "\n".join(row(y) for y in range(prompt + 1, footer))


def select_by_status(label, key, presses=12):
    """Press `key` until the status bar names `label`: the sidebar's
    selected row (the tree's entry, the outline's heading). True when it
    does, so a missed selection still fails the check that follows."""
    for _ in range(presses):
        if label in status_line():
            return True
        send(key)
        pump(0.2)
    return label in status_line()


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
check(heading_row("Lorem Index") is not None, "index.md rendered (its H1 in the content pane)")

# --- status-bar icons (config `icons = "unicode"`) -------------------------
check(
    "¶ index.md" in status_line(),
    "unicode note icon (¶) beside the note name",
)
check(
    "" not in status_line(),
    "the nerd-font note glyph (U+E73E) is not used in unicode mode",
)

# --- the outline (o) on index.md -------------------------------------------
send("f")  # focus the sidebar (tree view)
pump(0.3)
send("o")
pump(0.4)
check(sidebar_has("outline"), "o switches the sidebar to the outline")
check(
    sidebar_row("Lorem Index") is not None,
    "the open note's first heading is listed",
)
outline_rows = [
    sidebar_row(h) for h in ("Lorem Index", "Projects", "Notes and references", "Tasks")
]
check(
    None not in outline_rows and outline_rows == sorted(outline_rows),
    "every heading is listed in document order",
)
check(
    sidebar_row("alpha.md") is None,
    "tree rows are gone in outline mode",
)

check(
    "Lorem Index" in status_line(),
    "the outline opens with its first heading selected",
)
send("j")  # one press: the second heading
pump(0.3)
check(
    "Projects" in status_line(),
    "j selects the second heading; the status bar shows it",
)

# --- the outline's jump (Enter) on long-read.md ----------------------------
# index.md is too short to scroll a heading to the top, so the jump is
# checked on the long note, opened from the quick jumper.
send("\x0b")  # Ctrl+K
pump(0.5)
check("jump: 30" in body_text(), "Ctrl+K opened the quick jumper over all 30 notes")
check(">▏" in body_text(), "the jumper shows an empty query prompt")
check("alpha.md" in popup_result_rows(), "the first note (alpha.md) is listed before typing")

for c in "long-read":
    send(c)
pump(0.4)
check(">long-read▏" in body_text(), "the query is typed into the prompt")
results = popup_result_rows()
check("reading/long-read.md" in results, "the matching note is listed")
check("alpha.md" not in results, "the non-matching note is filtered out")
check("jump: 1" in body_text(), "the query narrows the jumper to one note")

send("\r")  # Enter opens the selected note
pump(0.8)
check("¶ long-read.md" in status_line(), "Enter opened the selected note")
check(heading_row("Long read") is not None, "the opened note is rendered (its H1 is on screen)")

send("f")  # focus the outline, which now lists long-read.md's headings
pump(0.3)
check(
    select_by_status("Part 3", "j", presses=6),
    "j selects the heading Part 3; the status bar shows it",
)
send("\r")  # Enter jumps to it
pump(0.5)
check(
    heading_row("Part 3") == 1,
    "Enter scrolled the note: Part 3 heads the top text row",
)

send("o")  # outline off: back to the view it was over (the tree)
pump(0.4)
check(
    sidebar_row("alpha.md") is not None,
    "o again returns to the tree",
)
check(not sidebar_has("outline"), "the outline panel is gone")
send("g")  # back to the top of the note for the theme color checks
pump(0.3)

# --- theme groups (Ctrl+T) -------------------------------------------------
OCEAN_HEADING = heading_fg("Long read")
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
    heading_fg("Long read") == OCEAN_HEADING,
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
LIGHT_HEADING = heading_fg("Long read")
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
DARK_HEADING = heading_fg("Long read")
check(
    DARK_HEADING is not None and DARK_HEADING != LIGHT_HEADING,
    f"the dark theme really applied (heading fg {DARK_HEADING})",
)

# --- .base tables in the sidebar tree -------------------------------------
# dashboards/projects.base: four table views (docs/TESTVAULT.md, Bases).
send("f")  # focus the sidebar (tree view)
pump(0.3)
check(
    select_by_status("dashboards", "k"),
    "k walks the tree up to the dashboards folder",
)
send("\r")  # Enter expands the folder
pump(0.3)
check(
    select_by_status("▦ projects.base", "j", presses=4),
    "the projects.base row can be selected in the tree",
)
check(
    "▦ projects.base" in status_line(),
    "unicode base icon (▦) beside the selected .base row",
)
check(sidebar_row("projects.base") is not None, "the .base file is a tree row")

send("\r")  # Enter opens the table
pump(0.8)
check(
    content_has("base · view 1/4: By status"),
    "Enter on the .base row opened the table view",
)
check(
    "8 rows · view 1/4: By status" in status_line(),
    "the status bar shows the table state (8 rows, view 1/4)",
)
check(
    "▦ projects.base" in status_line(),
    "the status bar shows the base icon beside the open table",
)
check(
    content_row_has("beta", "2000-01-01"),
    "the table shows its rows (the projects/beta.md row)",
)

send("]")  # next view
pump(0.5)
check(
    "view 2/4: Hubs" in status_line(),
    "] moved to the second view",
)
check(
    "1 row · view 2/4: Hubs" in status_line(),
    "the second view lists its one row",
)
check(
    content_row_has("gamma"),
    "the hub view holds projects/gamma.md",
)

send("1")  # jump back to the first view
pump(0.5)
check(
    "8 rows · view 1/4: By status" in status_line(),
    "the digit key jumped back to view 1",
)

# --- clicking the .base row opens the table too ---------------------------
send("\x7f")  # Backspace -> back to the note
pump(0.8)
check("¶ long-read.md" in status_line(), "Backspace returned to the note")
base_row = sidebar_row("projects.base")
check(base_row is not None, "the .base row is still visible")
if base_row is not None:
    click(5, base_row)
    pump(0.8)
    check(
        content_has("base · view 1/4: By status"),
        "clicking the .base row opens the table view",
    )

# --- quit ------------------------------------------------------------------
send("q")
pump(0.8)
_, status = os.waitpid(pid, 0)
check(os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, "clean exit code 0")

print("SMOKE M6:", "OK" if ok else "FAILED")
sys.exit(0 if ok else 1)
