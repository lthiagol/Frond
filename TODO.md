# frond — the work in front of us

The project's state: what is in flight, what is open, and the order. It is
**tracked**, so it travels with the repository.

**What is done is done.** The changelog says what shipped and the git log
says how; neither is copied here. A list that is kept up to date is a list
that goes stale, and this project's own history is a run of stale lists
(the header's height, the open work, the carried lows). One fact, one
home:

| what | where it lives |
|---|---|
| the rules the work follows | `AGENTS.md` |
| what the app does | `SPEC.md` |
| what shipped | `CHANGELOG.md`, and the git log |
| what is in flight, what is open, the order | this file |

## How to resume

Read `AGENTS.md`, then this file. The work in flight is under **In
flight**; the open work is under **Open**, with the order under **Next**.

The flow's own rules are what made the last cycle work — keep them: one
unit at a time, a fresh builder and fresh reviewers per unit, the gate run
by the orchestrator and never taken on trust, and the release notes
covering every user-visible change.

## In flight

Nothing. The cycle's last unit is committed and pushed; 0.6.6 is released
and the tree is clean. What the cycle carries:

- **0.6.6** is released (2026-11-21) and pushed. `v0.6.6` is
  deliberately **not** tagged — the version lives in the tree, and the tag
  can follow whenever.
- It carries the sidebar's header (the vault's name in the tree and tag
  views, centred, sitting square in the panel), the outline's label, its
  numbering, its scroll-following and its wheel scrolling its view, and the
  two vault warnings that no longer go quiet — a failed walk, and a file
  that turns unreadable without changing.
- `## [Unreleased]` is empty and `Cargo.toml` says 0.6.6.

## Open

- **A vault with no `index.md` cannot open**, even with a saved session
  for another note: the start reads `index.md` before the session is
  restored (SPEC "Data flow" step 1 says so). Obsidian vaults often have
  no `index.md`; the start should restore first and fall back to an empty
  note pane. Found in u3, pre-existing.
- **A typed path in the vault picker** (next, the human's choices
  2026-11-26): a filter starting with `/`, `~` or `.` shows one row,
  `open <path>` (Tab completes directories), and Enter opens it; a name
  still filters. Vaults opened from a typed path are remembered in the
  app's state dir (a short recent list, never `config.toml`) and listed
  under the config entries. The stdin path prompt goes: with no
  `[vaults]` and no `--vault`, the picker opens with only the path entry
  and the recent paths.

## Next

1. The human's requests.
2. The findings of a cycle live in `/tmp/opencode/review<N>/` and die with
   the machine. Moving them into the repository would keep the reasoning
   behind each decision - the skill calls it a stopgap, and this project
   has now lost its review record twice.

## Past investigations

The record of the investigations behind shipped work. Kept for the
measurements, not as a task list — everything here is done.

### 1. Changelog / repo hygiene — DONE
- [x] Merge the duplicated `Added`/`Changed`/`Fixed` sections in the 0.6.1
      block of `CHANGELOG.md` (Keep a Changelog shape).
- [x] Add an `## [Unreleased]` header for post-`v0.6.1` work.
- Commit: `ed2f38f`.

### 2. Test gap on the newest features — DONE
- [x] New pty smoke suite `smoke_m6` covering:
  - [x] `Ctrl+T` theme-group cycling (all → light → dark) and `Shift+T`
        (the theme counts are derived from `builtin_themes.rs`, so a new
        built-in theme does not silently break the expectations)
  - [x] `icons = "unicode"` status-bar icon (`¶` for a note, `▦` for a base)
  - [x] opening a `.base` file from the sidebar tree (Enter + click,
        `v` / digit view switching)
  - [x] the `Ctrl+P` quick switcher (open, live filter, Enter)
- [x] Wired into `Makefile` (`make smoke`) — CI runs `make smoke`, so it
      is covered there too.
- Commit: `ed2f38f`.

### 3. Next user-facing feature — quick switcher (`Ctrl+P`) — DONE
- [x] Fuzzy note jump over the vault index (note names + paths).
      New module `crates/app/src/quick.rs`: the state + the matcher
      (case-insensitive subsequence match over the path and the file
      name; the file-name match wins).
- [x] Reuses the popup machinery (own module, own geometry helper).
- [x] `Enter` opens the note through the normal navigation stack.
- [x] Updated `SPEC.md`, `README.md`, the help popup and the CHANGELOG.
- Commit: `fbb9b78`.

### 4. SPEC reconciliation — DONE
- [x] Reconciled `SPEC.md` with the code (status bar segments and path
      label rules, the poll-based watcher, the wikilink resolution and
      ambiguity rules, the sidebar width clamps, the `h`/`l`/`b` focus
      rules, the help popup layout, the `watch` config key, the custom
      theme file format, the milestone list).
- [x] README's "no vault" wording corrected.
- Commit: `42c9ee7`.

---

### 5. Scrollbar: the thumb never reaches the bottom of the track

**Status: investigated, reported, FIXED (0.6.2 - the thumb geometry; 0.6.5,
units 24-26 - leaf's interactive bars, section 5.8).** The analysis below
is the record of the investigation (2026-11-18). References are to this
repo and to leaf upstream (`github.com/rivolink/leaf`, tag `v1.29.2`) -
the line-precise pointers need a clone of that tag, and the old
`/tmp/leaf` copy does not survive a restart.

#### 5.1 The symptom

The user reported: at the top of a document the scrollbar is at the top,
but at the bottom it never touches the bottom of the scrolling area — "the
size of the bar is missing".

#### 5.2 Root cause

`crates/app/src/ui.rs:32` (`draw_scrollbar`) builds ratatui's scrollbar
state like this:

```rust
let mut state = ScrollbarState::new(total)          // content_length = the LINE COUNT
    .position(position)                              // position = the scroll offset
    .viewport_content_length(viewport);              // viewport = the visible rows
```

ratatui 0.30 (`ratatui-widgets-0.3.2/src/scrollbar.rs`, `part_lengths`)
then computes:

```
max_position          = content_length - 1
start_position        = position.clamp(0, max_position)
max_viewport_position = max_position + viewport_length      // = total - 1 + viewport
thumb_length = round(viewport_length * track_length / max_viewport_position)
thumb_start  = round(start_position * track_length / max_viewport_position)
```

The denominator is `total - 1 + viewport` where a scrollbar wants `total`.
Two consequences, both visible:

1. **The thumb is too short.** The visible fraction of the document is
   `viewport / total`, but ratatui computes `viewport / (total - 1 + viewport)`
   — smaller by a factor of about `(total - 1 + viewport) / total`.
2. **The thumb stops short of the end.** The position mapping uses the
   same inflated denominator, so even at the maximum scroll offset the
   thumb's end lands above the last track row.

Note this is a *usage* bug, not a ratatui bug: ratatui's formula is
self-consistent, but it assumes `content_length` counts the *scroll
positions* of a viewport — we feed it the line count instead.

#### 5.3 Measurements (the real app, not the math)

A throwaway probe (a `TestBackend` render of a 200-line note on a 100x24
terminal, reading the `█` cells of the bar column 99):

| | thumb rows | where the track ends |
|---|---|---|
| at the top | 1–2 | row 21 (row 0 = `▲`, row 22 = `▼`, row 23 = status bar) |
| at the bottom | **18–19** | row 21 |

So at the bottom the thumb ends **2 rows above the last track row** (3
above the `▼` arrow). It is also 2 rows long where the visible fraction
(23/200 ≈ 11%) wants 2–3. The existing test
`note_scrollbar_position_tracks_the_drawn_scroll`
(`crates/app/src/ui/tests.rs`) only asserts that the thumb *moved down*,
so it never caught this.

#### 5.4 Affected call sites (three)

1. `crates/app/src/ui.rs:179` — the **note** scrollbar (`draw_scrollbar`).
2. `crates/app/src/ui.rs:438` — the **list panels** (sidebar tree, tag
   list, backlinks panel) — same helper.
3. `crates/app/src/ui.rs:1493` — the **help popup** has its own inline
   `ScrollbarState::new(total).position(scroll).viewport_content_length(...)`
   inside `draw_help`; it has the same bug and needs the same fix.

(There is no scrollbar on the `.base` table today — it scrolls but shows
no bar. That may be worth adding at the same time; see 5.8.)

#### 5.5 What leaf does differently

`src/render/content.rs:178`:

```rust
let max_scroll = app.max_scroll();               // total - viewport
let mut scrollbar_state = ScrollbarState::new(max_scroll).position(app.scroll());
// note: viewport_content_length is NOT set -> ratatui uses the track height
```

Feeding the *scroll range* as `content_length` (and leaving the viewport
length unset) makes ratatui's denominator `max_scroll - 1 + track`,
which is the classic scrollbar denominator. Leaf's own hit-test math
(`src/render/content.rs:155-167`) uses the classic form too:
`thumb_offset = scroll * (track - thumb_size) / max_scroll`, which puts
the thumb flush with the bottom at the maximum scroll.

#### 5.6 The fix (one line per call site)

```rust
ScrollbarState::new(total.saturating_sub(viewport)).position(position)
```

`viewer.max_scroll()` (`crates/app/src/viewer.rs:626`) already computes
`lines.len() - viewport_height`, so the value is available; for the list
panels it is `rows_total - inner_height`.

**Verified**: patched in temporarily and re-measured with the probe — the
thumb at the bottom moved from rows 18–19 to rows **20–21**, i.e. it now
reaches the last track row. (The patch was reverted; this is a report.)

#### 5.7 Optional: leaf's symbols

Leaf draws a thin `│` track, a solid `█` thumb and **no arrow heads**
(`src/render/content.rs:169-173`):

```rust
Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .begin_symbol(None)
    .end_symbol(None)
    .track_symbol(Some("│"))
    .thumb_symbol("█")
```

We use ratatui's defaults (`║` double track, `▲`/`▼` arrow heads), which
cost 2 of the 23 rows and make the track look busier. With leaf's
symbols the track is the full 23 rows and the thumb (rows 20–22 at the
bottom) reaches the very bottom. Cosmetic, but it is the "works better"
half of the user's observation.

#### 5.8 leaf's scrollbar interaction — DONE (0.6.5, units 24-26)

Leaf makes the bar interactive:
- a press on the bar jumps to that row and starts a **drag**
  (`src/runtime/mouse.rs:173-176`, `459-469`;
  `scrollbar_scroll_to` = `offset * max_scroll / (height - 1)`);
- the thumb highlights with a dedicated `scrollbar_hover` colour while
  the mouse is over it (`src/render/content.rs:174-176`).

Shipped in 0.6.5:
- unit 24 — the note pane's bar is a position control: a press jumps the
  note (the track's rows map linearly onto the scroll range), a drag
  follows the pointer, and the drag's release is consumed instead of
  being handled as a click. `App::bar_scroll_at` is the one home of the
  mapping; the layout records the bar column.
- unit 25 — the list panels' bars position their **selection** (the
  sidebar's scroll is a cursor, not a document position): the same
  press/drag/release rules, and a press on the bar is not a row click
  any more.
- unit 26 — the thumb highlights under the mouse, in the theme's accent
  (the new theme role `scrollbar_hover_fg`, every theme's value is its
  accent).

The hover colour did **not** reuse `popup_dimmed_fg`: the track and the
thumb shared one colour, so the thumb needed its own role to stand out.

Closed from this section: the `.base` table's bar and the help
popup's bar are position controls now (units 39 and 40 - a
press jumps, a drag follows the pointer, the release that ends the
drag opens nothing). The "no hover" half of the old line was wrong
when it was written: both bars are drawn through the shared
`draw_scrollbar` helper, which has always painted the thumb in
`scrollbar_hover_fg` under the pointer.

#### 5.9 Acceptance criteria for the fix - all met (0.6.2)

- [x] The thumb's last row == the last track row when the note is
      scrolled to the bottom (and the first track row at the top) -
      `note_scrollbar_thumb_ends_track_the_scroll_position`.
- [x] The thumb's length is proportional to the visible fraction
      (±1 row of rounding) -
      `note_scrollbar_thumb_uses_the_text_viewport_with_a_vertical_margin`.
- [x] The same holds for the sidebar tree, the tag list, the backlinks
      panel and the help popup -
      `backlinks_scrollbar_tracks_the_drawn_scroll`,
      `base_scrollbar_thumb_ends_track_the_scroll_position`,
      `help_scrollbar_thumb_ends_track_the_scroll_position`.
- [x] A new test renders a long note, scrolls to the bottom and asserts
      the thumb's last row == the last track row (the existing
      `note_scrollbar_position_tracks_the_drawn_scroll` only checks that
      it moved).
- [x] The `.base` table: it gets a scrollbar too - inside its border, in
      the last inner column, and the table is rendered one column
      narrower while it is drawn (SPEC, "Scrollbars").

---

### 6. Scroll-position counter in the status bar - DONE (0.6.2)

**Status: shipped in 0.6.2.** The questions below are kept as the record
of the decision; the answers are what shipped, and the spec carries them
(SPEC, "Status bar"). References are to leaf upstream (`github.com/rivolink/leaf`,
tag `v1.29.2`).

#### 6.1 What leaf does

- `src/app/navigation.rs:78-84`:

  ```rust
  pub(crate) fn scroll_percent(&self) -> u16 {
      let max = self.max_scroll();
      if max == 0 { return 100; }
      ((self.scroll * 100) / max).min(100) as u16
  }
  ```

- `src/render/status.rs:206-213` renders it as
  `format!("{:>3}% ", pct)` (right-aligned in 3 columns, trailing space)
  in a dedicated `status_percent_fg` colour.
- It is the **last (rightmost) section** of the status bar, after the
  shortcut hints (`src/render/status.rs:515`), and it is only
  pushed when a file is open.

#### 6.2 What frond has today

The percent ships. `scroll_percent` (`crates/app/src/ui.rs`) reads the
content pane's scroll offset as a percentage of the scroll range: `0` at
the top, `100` at the bottom, `100` when the content fits. The status bar
(`draw_status_bar`) shows it immediately after the size segment:

`name · size · 42% · watch on/off · rendered HH:MM:SS · git · [error|info]`
with the keyboard hints right-aligned — and the hints are hidden while a
message is shown.

#### 6.3 The decisions

1. **Where does it go?** Leaf puts the percent rightmost, after the
   hints. We have the hints on the right edge, and they disappear when a
   message shows. Options: (a) percent rightmost, hints before it;
   (b) percent rightmost, hints dropped when the percent is shown;
   (c) percent next to the size segment (`12 lines · 42%`).
   → **(c)** - the percent sits right after the size segment, and the
   hints keep their own rule (flush right, hidden while a message shows).
2. **Which pane?** Leaf has a single content area. We have two
   scrollables: the content pane (note or `.base` table) and the sidebar
   tree. The content pane is the natural one (it is the "document");
   the sidebar could get its own.
   → **the content pane** - the open note, or the `.base` table when one
   is open, regardless of which pane holds the focus.
3. **Which value?** The scroll *offset* percentage (leaf: 0% at the top,
   100% at the bottom) or the *visible* fraction (100% at the top,
   11% at the bottom)? Leaf uses the offset.
   → **the offset**, leaf's value.

#### 6.4 Colour role

leaf has a dedicated `status_percent_fg` (e.g. the accent colour in
each preset family). frond has **no** such role, and `crates/app/src/theme.rs`
already documents that leaf's "brand/percent segments" were not mapped.

Options:
- (a) reuse an existing role — `status_watch_off_fg` (the neutral dimmed
  one, already used for the `watch off` / `metadata off` hints) or
  `popup_dimmed_fg`. No theme-file churn, no new key.
- (b) add a real `status_percent_fg` to `UiTheme`: 4 presets
  (`ui_by_preset`, `crates/app/src/theme.rs:473`) + 14 built-ins
  (`crates/app/src/builtin_themes.rs`) + the custom-theme vocabulary +
  `docs/COLORS.md` + the `with_markdown_fields!`-style key list. Faithful
  to leaf, but a much bigger diff.

→ **(a)**, and not even a dimmed role: the percent is styled like the size
segment next to it. `UiTheme` keeps its 20 keys and the custom-theme
vocabulary is unchanged.

#### 6.5 Acceptance criteria - all met

- [x] The percent is shown on the status bar and updates while scrolling —
      `status_bar_shows_the_scroll_percentage`.
- [x] It reads 0% at the top and 100% at the bottom of a scrollable
      document; a document that fits reads 100% —
      `scroll_percent_reads_the_offset`,
      `status_bar_scroll_percentage_is_100_when_the_note_fits`.
- [x] It never collides with the keyboard hints or with a status message —
      it sits in a fixed four-column field, so the segments after it never
      move (`status_bar_percentage_field_is_fixed_width_and_does_not_move`).
- [x] A unit test covers the percentage function (0 / 50 / 100 and the
      fits-entirely case) — `scroll_percent_reads_the_offset`.

---

### Follow-ups
- [x] Release: cut `0.6.2` (2026-11-19), `0.6.3`, `0.6.4` and **`0.6.5`
      (2026-11-20)** — the last one carries the selection bar, the
      interactive scrollbars, the scrollbar hover, the link hover, the
      regex search, the match-next, the line-number gutter and the
      startup warnings.
- [x] Consider more features from the "ideas" list: an outline/TOC panel
      (`o`), a link hover preview (the **hover highlight**, in the markdown
      theme's `link_hover` — leaf's hover is a highlight, not a preview
      popup), regex + match-next navigation in the search popup (the `re:`
      prefix, and `n`/`N` walking the last search in the open note).
- [x] Leaf also has a line-number gutter (`is_line_number_visible`) —
      shipped as the config key `line_numbers` (the note's source line
      numbers, an empty cell on a wrapped continuation, the markdown
      theme's `code_gutter` colour).
- [x] The `.base` table's bar and the help popup's bar are position
      controls (units 39 and 40) - the last two inert bars.
- [x] The sidebar's scroll position as a document position: **dropped** by
      the human's decision - the sidebar's scroll is a selection cursor,
      not a document position, so the idea is closed rather than carried.
