# The sample vault (`testvault/`)

One lorem-ipsum vault that covers every feature frond renders or indexes.
Tests read it or copy it; nothing writes into it.

- **Read in place:** `crates/vault/tests/testvault.rs` indexes it and pins
  the index and `.base` facts below. The rendering and screen facts are
  not pinned by a test yet; the smoke suites that run on this vault assert
  them, on a copy (see "Smoke suites").
- **Copy before mutating:** a test that writes (a checkbox toggle, a file
  edit for the watcher, `git init`) takes a copy:
  - Rust: `copy_testvault(name)` in `crates/app/src/testutil.rs`
  - Python: `copy_testvault(name)` in `scripts/smoke_tmp.py`
  - Both keep `links/alpha-link.md` as a symlink to `../alpha.md`, so a
    copy edits the same way the checkout does.
- **A vault with no `index.md`:** a copy with `index.md` removed.
- **Several vaults, no vault, a missing vault:** the test builds its own.
- **Git:** the vault has no `.git`. A test that needs a repository copies
  the vault and `git init`s the copy.
- **Dates:** the index's only clock-dependent results are the `.base`
  date windows, and those are pinned with sentinels: `2999-…` is inside
  any window, `2000-…` is outside any window. The base test passes a
  fixed `today`.

## Layout

```
testvault/
  index.md                      entry note (`# Lorem Index`): links, tasks, "needle", broken link
  alpha.md                      headings 1-4, table, rust fence, frontmatter
  gamma.md                      root gamma (the ambiguous "gamma" in the root)
  notes.txt                     plain text: not indexed
  .obsidian/app.json            hidden: not indexed
  .trash/deleted.md             hidden note: not indexed
  archive/                      superseded, archived, old note
  projects/                     beta, gamma, long name (scroll end marker), blocked, roadmap, pilot, launch
  projects/clients/acme/        nested: kickoff, contract
  tasks/                        tasks.md, tasks.base
  docs/                         dup (one), guide, math, rendering
  refs/                         dup (two)
  diagrams/                     flow, sequence, pie (mermaid)
  reading/long-read.md          120 paragraphs: scrolling
  edge/                         CRLF, BOM, unterminated fence, no frontmatter
  links/alpha-link.md           file symlink to ../alpha.md
  dashboards/                   projects.base, fail-open.base
```

The vault has 30 notes (the symlink counts), 3 `.base` files and one
folder per feature. The sidebar header shows the directory's name
(`testvault` in the checkout; a copy shows its own name).

## Index and walk

- Notes: 30 (`crates/vault/tests/testvault.rs::indexes_the_documented_notes_and_nothing_else`).
- Skipped: `.obsidian/` and `.trash/` (hidden), `notes.txt` (not `.md`).
- `links/alpha-link.md` is a file symlink; it is indexed as a note with
  alpha's content, so alpha's tags and links count twice.
- Bases: `tasks/tasks.base`, `dashboards/projects.base`,
  `dashboards/fail-open.base`.
- Index warnings (not shown in the UI): `docs/dup.md`, `refs/dup.md`,
  `edge/plain.md` (no frontmatter), `edge/unterminated.md` (fence never
  closed).

## Status glyphs

The sidebar's map (`status_glyph` in `crates/app/src/ui.rs`), with the
note that carries each value:

| glyph | values | note |
|---|---|---|
| `●` | `complete` | `alpha.md`, `archive/old-note.md`, `edge/bom.md`, `links/alpha-link.md` |
| `●` | `resolved` | `projects/launch.md` |
| `●` (green) | `active` | `index.md`, `gamma.md`, `projects/beta.md`, `tasks/tasks.md`, `edge/crlf.md` |
| `○` | `pending` | `projects/a-note-with-a-really-long-name-that-must-truncate-in-the-sidebar.md` |
| `○` | `planned` | `projects/roadmap.md` |
| `◐` | `in-progress` | `projects/gamma.md` |
| `◐` | `in progress` | `projects/clients/acme/kickoff.md` |
| `◐` | `monitoring` | `projects/pilot.md` |
| `◌` | `draft` | `reading/long-read.md` |
| `◉` (red) | `blocked` | `projects/blocked-deploy.md` |
| `×` (dimmed) | `superseded` | `archive/superseded-plan.md` |
| `×` (dimmed) | `archived` | `archive/retired-tool.md` |
| none | `waiting-legal` (unknown) | `projects/clients/acme/contract.md` |
| none | no `status:` | the other notes |

Pinned: `crates/vault/tests/testvault.rs::statuses_map_every_documented_value`.

## Frontmatter

- **Inline tags with a comment:** `alpha.md` (`tags: [lorem, ipsum] # …`).
- **Block tags:** `archive/retired-tool.md` (`archive`, `tools  # …`),
  `projects/clients/acme/kickoff.md`, `docs/guide.md`.
- **Comma tag:** `docs/guide.md` has the one tag `mocktag, weird`.
- **Tag counts:** 47 tag uses, 25 distinct tags. The largest: `lorem` 6,
  `dolor` 5, `ipsum` 4, `archive` 3, `diagram` 3. Full list pinned by
  `tags_are_counted_across_the_vault`.
- **Hub flag:** `projects/gamma.md` has `hub: true` (raw value `true`).
- **`updated:`:** `index.md` `2999-01-01`, `alpha.md` `2999-12-31`,
  `projects/gamma.md` `2999-06-01`, `projects/roadmap.md` `2999-03-15`,
  `projects/beta.md` `2000-01-01`.
- **`created:` / `archived:`:** `alpha.md` `2025-03-14`;
  `archive/retired-tool.md` `2024-06-30`.
- **CRLF:** `edge/crlf.md` (fence and lines end in `\r\n`): status
  `active`, tags `edge`, `crlf`.
- **BOM:** `edge/bom.md` (a UTF-8 BOM before the fence): status `complete`,
  tags `edge`, `bom`.
- **Unterminated fence:** `edge/unterminated.md` has no frontmatter at all:
  no status, no tags, and `status: draft` renders as body text.
- **Frontmatter links:** `projects/beta.md` (`related: "[[alpha]]"`) and
  `docs/guide.md` (block `related:` list with `[[projects/beta]]`).

## Links

Each link shows as `#` followed by its text (the alias when given), so
`[[alpha]]` shows `#alpha` and `[[projects/beta|Beta project]]` shows
`#Beta project`.

| case | where | expectation |
|---|---|---|
| resolved by bare name | `index.md`: `[[alpha]]`, `[[roadmap#Milestone Two]]` | `alpha.md`, `projects/roadmap.md` |
| path link | `index.md`: `[[projects/beta]]`, `[[archive/old-note]]` | `projects/beta.md`, `archive/old-note.md` |
| alias | `index.md`: `[[projects/beta\|Beta project]]` | target `projects/beta`, alias `Beta project` |
| heading anchor | `index.md`: `[[alpha#Deep Section]]`; `projects/beta.md`: `[[alpha#Details]]` | `alpha.md` (`### Deep Section`, `## Details`) |
| anchor into a long note | `index.md`: `[[long-read#Part 3]]` | `reading/long-read.md` (`## Part 3`, 60 paragraphs follow it, so it can open with the heading on the top row) |
| path suffix | `index.md`: `[[clients/acme/kickoff]]` | `projects/clients/acme/kickoff.md` |
| base link | `index.md`: `[[projects.base]]` | `dashboards/projects.base` |
| heading only (self link) | `gamma.md`: `[[#Local Section]]`; `reading/long-read.md`: `[[#Tail]]` | self-links in the same note |
| ambiguous | `index.md`: `[[dup]]` (root) | `docs/dup.md`, `refs/dup.md` (no proximity match) |
| ambiguous, proximity wins | `docs/guide.md`: `[[dup]]` → `docs/dup.md`; `projects/beta.md`: `[[gamma]]` → `projects/gamma.md`; `docs/guide.md`: `[[gamma]]` → root `gamma.md` | nearest folder wins |
| ambiguous `gamma` | `gamma.md` and `projects/gamma.md` | `ambiguous_basenames` lists `dup` and `gamma` |
| broken | `index.md`: `[[nowhere]]` | not found |
| embed | `docs/guide.md`: `![[alpha]]` | an embed link |
| code block | `docs/rendering.md`: `[[not-a-link]]` in a fence | not a link |
| hidden | `.trash/deleted.md` links to `[[alpha]]` | not counted |

Backlinks to `alpha.md`: `docs/guide.md`, `index.md`, `projects/beta.md`.
Pinned: `links_resolve_as_documented`, `backlinks_and_link_kinds`.

## Outline (headings)

- `alpha.md`: `# Alpha`, `## Details`, `### Deep Section`, `#### Fine print`.
- `docs/rendering.md`: `# Rendering showcase`, then `## Level two` through
  `###### Level six`.
- `projects/roadmap.md`: `## Milestone One`, `## Milestone Two`, `### Risks`.
- `reading/long-read.md`: `## Part 1` to `## Part 4`, `## Tail`.

## Checkboxes

Glyphs: `☐` unchecked, `☑` checked, `◐` in progress (`[~]`).

- `index.md`: `[ ]` Lorem…, `[x]` Consectetur…, `[ ]` Sed…, `[~]` Ut enim…
- `projects/beta.md`: `[ ]` Sunt in culpa, `[x]` Qui officia deserunt.
- `projects/gamma.md`: `[~]` Doloremque laudantium, `[ ]` Totam rem aperiam.
- `projects/roadmap.md`: four tasks (`[x]` Sed do eiusmod, then three open).
- `tasks/tasks.md`: three tasks.
- `edge/crlf.md`: `[ ]` CRLF task.

The guarded toggle (SPEC "Guarded writes") writes to a copy: its `updated:`
bump needs an existing field, and `index.md` has one (`2999-01-01`).

## Bases

- **`tasks/tasks.base`:** one view, `Notes`, filter `type == "note"`:
  7 rows: `alpha.md`, `archive/old-note.md`, `archive/superseded-plan.md`,
  `gamma.md`, `links/alpha-link.md`, `reading/long-read.md`,
  `tasks/tasks.md`.
- **`dashboards/projects.base`:** base filter `file.inFolder("projects")`
  and `status != "blocked"` (8 rows, the blocked project is dropped).
  Four views, in order:
  1. `By status`: grouped by `status`, the groups `active`, `in progress`,
     `in-progress`, `monitoring`, `pending`, `planned`, `resolved`,
     `waiting-legal`; `formula.project` (`file.asLink`) and `updated`
     columns; `properties:` shows `Status` and `Project` headers.
  2. `Hubs`: `hub == true` → `projects/gamma.md`.
  3. `Recent`: `updated >= today() - "30d"`, sorted newest first → the
     `projects/gamma.md` then `projects/roadmap.md` rows (beta is outside).
  4. `Tagged`: `file.hasTag("ops")` or `file.hasTag("legal")` → the long
     name note and `projects/clients/acme/contract.md`.
- **`dashboards/fail-open.base`:** the filter `file.size > 10` is an
  unknown expression, so it fails open with a warning and matches every
  note (30 rows). The `cards` view is dropped with a warning. The
  `formula.words` cell is an em-dash (`—`), since only `file.asLink` and
  `file.name` formulas are supported.

Pinned: `the_notes_table_base_shows_the_seven_notes`,
`the_project_dashboard_views_hold_their_rows`,
`the_fail_open_base_keeps_every_note`.

## Rendering

- **Tables:** `alpha.md` (`Column | Value`), `docs/rendering.md`
  (`Left | Center | Right`).
- **Lists:** `projects/beta.md` (nested bullets, an ordered list),
  `docs/rendering.md`.
- **Callouts:** `docs/rendering.md`, one each for `NOTE`, `TIP`,
  `IMPORTANT`, `WARNING`, `CAUTION` (the labels show as
  `Note`, `Tip`, `Important`, `Warning`, `Caution`).
- **Code, syntax highlighted:** `alpha.md` (rust), `docs/rendering.md`
  (rust, python, json, bash). The `text` fence in `docs/rendering.md`
  holds `[[not-a-link]]`.
- **Inline:** `**bold**`, `*italic*`, `~~struck~~`, `==marked==`,
  `` `inline code` ``, a footnote (`docs/rendering.md`), a blockquote, a
  rule, an external link.
- **LaTeX:** `docs/math.md`: `$E = mc^2$` inline, a `$$` display block,
  and a `latex` fence (rendered in a `latex` box).
- **Mermaid:** `diagrams/flow.md` (flowchart: `Draft`, `Review`,
  `Approved?`, `Publish`), `diagrams/sequence.md` (`Alice`, `Bob`,
  `Hello Bob`), `diagrams/pie.md` (`Lorem split`, `ipsum 40.0%`,
  `dolor 35.0%`, `sit 25.0%`). Each renders in a `mermaid` box.

## Search and scrolling

- `needle`: `index.md` (`the needle in index is this line`), `alpha.md`
  (`The needle lives here`). `deep-section-anchor-line` is in `alpha.md`.
  Every alpha string also matches `links/alpha-link.md` (the symlink is
  indexed with alpha's content), so a vault search for `needle` returns
  three notes (`index.md`, `alpha.md`, `links/alpha-link.md`), and
  `deep-section-anchor-line` returns two (`alpha.md`, `links/alpha-link.md`).
- Regular expression `TKT-\d{4}`: `projects/roadmap.md` (`TKT-1042`,
  `TKT-2077`).
- `reading/long-read.md`: `Paragraph 1:` through `Paragraph 120:`, then
  `END-OF-LONG-READ` at the bottom, past the screen.

## Layout edge cases

- **Long name:** `projects/a-note-with-a-really-long-name-that-must-truncate-in-the-sidebar.md`
  (title `A long name`). Its body is 60 filler paragraphs under `## Filler`
  and ends in `END-OF-LONG-NOTE`, past the screen.
- **Nested folders:** `projects/clients/acme/` (two levels under `projects/`).
- **Same stem:** `docs/dup.md` (`# Dup One`, `unique-one-body`) and
  `refs/dup.md` (`# Dup Two`, `unique-two-body`).
- **Hidden:** `.trash/`, `.obsidian/`.
- **Plain text:** `notes.txt`.
- **No frontmatter:** `edge/plain.md` (`# Plain note`).

## Smoke suites

The smoke suites that run on this vault copy it (`copy_testvault`) and read
these screen facts from the copy:

- `index.md`: the heading `Lorem Index` (the theme checks read its colour).
- `alpha.md`: a frontmatter table above `# Alpha`, then `## Details` and the
  table; the mouse checks open it from the tree.
- `projects/` long name note: opened from the tree (`A long name`); scrolled
  to its end (`END-OF-LONG-NOTE`).
- `reading/long-read.md`: the scroll note; the watch check appends to its copy.
- `notes.txt` and `.obsidian/`: not listed in the sidebar.
- `index.md`'s backlinks panel (`b`): three, in this order: `alpha.md`,
  `edge/crlf.md`, `links/alpha-link.md`. Pinned:
  `backlinks_and_link_kinds`.
- `edge/crlf.md` renders its heading `CRLF note`.
- Tag view (`t`): `#lorem` heads the list (the most notes, 6), and the
  list has `#rust (2)` and `#mocktag, weird (1)`. Expanding `#lorem` lists
  `alpha.md` first.
- `needle` in the results popup (`s`), in path order: `alpha.md:27`,
  `index.md:11`, `links/alpha-link.md:27`. Opened at its hit, `alpha.md`'s
  top row reads `Ut enim ad minim veniam`.
- `index.md`'s task `- [ ] Lorem ipsum dolor sit amet` (in `## Tasks`, past
  the first screen) is the checkbox toggle's target on a copy. The root
  `gamma.md` stays unmarked after `alpha.md` is edited on the copy.
