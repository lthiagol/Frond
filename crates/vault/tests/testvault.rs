//! Pins the facts `docs/TESTVAULT.md` documents for the sample vault
//! `testvault/`: the note count, the statuses, the tags, the resolved,
//! ambiguous and broken links, and every `.base` view's row set. The vault
//! is indexed in place: indexing only reads, and the vault is never written.
//!
//! The `.base` views use a fixed `today` (`TODAY`). The date windows in the
//! vault are sentinels far in the past (`2000-01-01`) and far in the future
//! (`2999-…`), so no expectation here depends on the real clock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use testkit::testvault_root;
use vault::base::{build_table, parse_base, TableRow};
use vault::{Resolution, VaultIndex, WarningKind};

const TODAY: &str = "2026-11-26";

fn index() -> VaultIndex {
    VaultIndex::build(&testvault_root()).expect("testvault indexes")
}

/// `path` relative to the vault root, with `/` separators.
fn rel(index: &VaultIndex, path: &Path) -> String {
    path.strip_prefix(index.root())
        .expect("path is inside the vault")
        .to_string_lossy()
        .replace('\\', "/")
}

fn rels(index: &VaultIndex, paths: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = paths.iter().map(|p| rel(index, p)).collect();
    out.sort();
    out
}

/// A note's outgoing links (`VaultIndex::links_from` is test-only in the
/// crate, so integration tests read the note's own list).
fn links_of<'a>(index: &'a VaultIndex, rel_path: &str) -> &'a [vault::LinkRef] {
    &index
        .note(&index.root().join(rel_path))
        .unwrap_or_else(|| panic!("{rel_path} is indexed"))
        .links
}

fn read_base(index: &VaultIndex, rel_path: &str) -> vault::base::BaseFile {
    let source = std::fs::read_to_string(index.root().join(rel_path)).expect("base file reads");
    parse_base(&source)
}

/// The note rows of a view, as vault-relative paths in table order.
fn view_rows(index: &VaultIndex, base: &vault::base::BaseFile, view: usize) -> Vec<String> {
    let table = build_table(index, base, view, TODAY);
    table
        .rows
        .iter()
        .filter_map(|row| match row {
            TableRow::Note(note) => Some(rel(index, &note.path)),
            TableRow::Group(_) => None,
        })
        .collect()
}

#[test]
fn indexes_the_documented_notes_and_nothing_else() {
    let index = index();
    let mut notes: Vec<String> = index.notes().map(|n| rel(&index, &n.path)).collect();
    notes.sort();
    assert_eq!(notes.len(), 30, "notes: {notes:?}");
    assert!(
        notes.iter().all(|n| !n.starts_with(".trash/")),
        "hidden folders are skipped"
    );
    assert!(
        !notes.iter().any(|n| n.ends_with("notes.txt")),
        "a plain text file is not a note"
    );
    assert!(
        notes.contains(&"links/alpha-link.md".to_string()),
        "a file symlink to a note is indexed"
    );

    let mut bases: Vec<String> = index.bases().map(|p| rel(&index, p)).collect();
    bases.sort();
    assert_eq!(
        bases,
        [
            "dashboards/fail-open.base",
            "dashboards/projects.base",
            "tasks/tasks.base"
        ]
    );
}

#[test]
fn statuses_map_every_documented_value() {
    let index = index();
    let statuses: BTreeMap<String, Option<String>> = index
        .notes()
        .map(|n| (rel(&index, &n.path), n.status.clone()))
        .collect();
    let expected: &[(&str, Option<&str>)] = &[
        ("alpha.md", Some("complete")),
        ("archive/old-note.md", Some("complete")),
        ("archive/retired-tool.md", Some("archived")),
        ("archive/superseded-plan.md", Some("superseded")),
        ("docs/dup.md", None),
        ("docs/guide.md", None),
        ("docs/math.md", None),
        ("docs/rendering.md", None),
        ("diagrams/flow.md", None),
        ("diagrams/pie.md", None),
        ("diagrams/sequence.md", None),
        ("edge/bom.md", Some("complete")),
        ("edge/crlf.md", Some("active")),
        ("edge/plain.md", None),
        ("edge/unterminated.md", None),
        ("gamma.md", Some("active")),
        ("index.md", Some("active")),
        ("links/alpha-link.md", Some("complete")),
        (
            "projects/a-note-with-a-really-long-name-that-must-truncate-in-the-sidebar.md",
            Some("pending"),
        ),
        ("projects/beta.md", Some("active")),
        ("projects/blocked-deploy.md", Some("blocked")),
        ("projects/clients/acme/contract.md", Some("waiting-legal")),
        ("projects/clients/acme/kickoff.md", Some("in progress")),
        ("projects/gamma.md", Some("in-progress")),
        ("projects/launch.md", Some("resolved")),
        ("projects/pilot.md", Some("monitoring")),
        ("projects/roadmap.md", Some("planned")),
        ("reading/long-read.md", Some("draft")),
        ("refs/dup.md", None),
        ("tasks/tasks.md", Some("active")),
    ];
    assert_eq!(statuses.len(), expected.len());
    for (path, status) in expected {
        assert_eq!(
            statuses.get(*path),
            Some(&status.map(str::to_string)),
            "status of {path}"
        );
    }
}

#[test]
fn frontmatter_fields_and_edge_cases() {
    let index = index();
    let note = |rel_path: &str| {
        index
            .note(&index.root().join(rel_path))
            .unwrap_or_else(|| panic!("{rel_path} is indexed"))
    };
    assert_eq!(
        note("archive/retired-tool.md").archived.as_deref(),
        Some("2024-06-30")
    );
    assert_eq!(note("alpha.md").updated.as_deref(), Some("2999-12-31"));
    assert_eq!(note("alpha.md").created.as_deref(), Some("2025-03-14"));
    assert_eq!(
        note("projects/beta.md").updated.as_deref(),
        Some("2000-01-01")
    );
    assert_eq!(
        note("projects/gamma.md").raw.get("hub").map(String::as_str),
        Some("true")
    );
    assert_eq!(note("edge/crlf.md").tags, ["edge", "crlf"]);
    assert_eq!(note("edge/bom.md").tags, ["edge", "bom"]);
    assert!(note("edge/unterminated.md").tags.is_empty());
    assert!(note("edge/unterminated.md").status.is_none());
    assert_eq!(note("alpha.md").note_type.as_deref(), Some("note"));

    // Only the notes without a usable frontmatter block are diagnosed.
    let mut warned: Vec<String> = index
        .warnings()
        .iter()
        .inspect(|w| assert_eq!(w.kind, WarningKind::Frontmatter))
        .map(|w| rel(&index, &w.path))
        .collect();
    warned.sort();
    assert_eq!(
        warned,
        [
            "docs/dup.md",
            "edge/plain.md",
            "edge/unterminated.md",
            "refs/dup.md"
        ]
    );
}

#[test]
fn tags_are_counted_across_the_vault() {
    let index = index();
    let counts: Vec<(String, usize)> = index
        .tag_counts()
        .into_iter()
        .map(|(tag, n)| (tag.to_string(), n))
        .collect();
    let expected: Vec<(String, usize)> = [
        ("acme", 1),
        ("amet", 1),
        ("archive", 3),
        ("bom", 1),
        ("clients", 2),
        ("crlf", 1),
        ("diagram", 3),
        ("dolor", 5),
        ("edge", 2),
        ("index", 1),
        ("ipsum", 4),
        ("launch", 1),
        ("legal", 1),
        ("lorem", 6),
        ("markdown", 1),
        ("math", 1),
        ("mocktag, weird", 1),
        ("ops", 2),
        ("plan", 2),
        ("reading", 1),
        ("rust", 2),
        ("sit", 2),
        ("stats", 1),
        ("tasks", 1),
        ("tools", 1),
    ]
    .iter()
    .map(|(tag, n)| (tag.to_string(), *n))
    .collect();
    assert_eq!(counts, expected);
    assert_eq!(counts.iter().map(|(_, n)| n).sum::<usize>(), 47);

    let tagged = |tag: &str| {
        let mut out: Vec<String> = index
            .notes_with_tag(tag)
            .iter()
            .map(|n| rel(&index, &n.path))
            .collect();
        out.sort();
        out
    };
    assert_eq!(tagged("rust"), ["docs/guide.md", "docs/rendering.md"]);
    assert_eq!(tagged("mocktag, weird"), ["docs/guide.md"]);
    assert_eq!(
        tagged("clients"),
        [
            "projects/clients/acme/contract.md",
            "projects/clients/acme/kickoff.md"
        ]
    );
    assert!(
        tagged("ghost").is_empty(),
        "a hidden folder's note is not indexed"
    );
}

#[test]
fn links_resolve_as_documented() {
    let index = index();
    let from = |rel_path: &str| index.root().join(rel_path);

    // From the vault root: a bare name is ambiguous when two notes share it.
    let root = from("index.md");
    let Resolution::Ambiguous(mut candidates) = index.resolve_with_hint(&root, "dup") else {
        panic!("a bare name shared by two notes is ambiguous from the root");
    };
    candidates.sort();
    assert_eq!(candidates, [from("docs/dup.md"), from("refs/dup.md")]);
    assert_eq!(
        index.resolve_with_hint(&root, "gamma"),
        Resolution::Found(from("gamma.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "alpha"),
        Resolution::Found(from("alpha.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "roadmap"),
        Resolution::Found(from("projects/roadmap.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "long-read"),
        Resolution::Found(from("reading/long-read.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "projects/beta"),
        Resolution::Found(from("projects/beta.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "archive/old-note"),
        Resolution::Found(from("archive/old-note.md"))
    );
    // A path that is not a full vault path matches by suffix.
    assert_eq!(
        index.resolve_with_hint(&root, "clients/acme/kickoff"),
        Resolution::Found(from("projects/clients/acme/kickoff.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&root, "nowhere"),
        Resolution::NotFound
    );
    assert_eq!(
        index.resolve("projects.base"),
        Resolution::Found(from("dashboards/projects.base"))
    );

    // A note's own folder wins a same-name tie: the proximity rule.
    assert_eq!(
        index.resolve_with_hint(&from("docs/guide.md"), "dup"),
        Resolution::Found(from("docs/dup.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&from("projects/beta.md"), "gamma"),
        Resolution::Found(from("projects/gamma.md"))
    );
    assert_eq!(
        index.resolve_with_hint(&from("docs/guide.md"), "gamma"),
        Resolution::Found(from("gamma.md"))
    );

    let mut ambiguous: Vec<String> = index
        .ambiguous_basenames()
        .iter()
        .map(|(stem, _)| stem.to_string())
        .collect();
    ambiguous.sort();
    assert_eq!(ambiguous, ["dup", "gamma"]);
}

#[test]
fn backlinks_and_link_kinds() {
    let index = index();
    let backlinks = |rel_path: &str| rels(&index, index.backlinks_to(&index.root().join(rel_path)));
    assert_eq!(
        backlinks("alpha.md"),
        ["docs/guide.md", "index.md", "projects/beta.md"],
        "the trashed note links to alpha but is not indexed"
    );
    assert_eq!(
        backlinks("projects/roadmap.md"),
        [
            "archive/superseded-plan.md",
            "index.md",
            "projects/blocked-deploy.md"
        ]
    );
    // The self-link `[[#Tail]]` is no backlink; index's anchor into Part 3 is.
    assert_eq!(backlinks("reading/long-read.md"), ["index.md"]);
    assert_eq!(
        backlinks("index.md"),
        ["alpha.md", "edge/crlf.md", "links/alpha-link.md"]
    );

    let links = |rel_path: &str| links_of(&index, rel_path);

    // Frontmatter wikilinks are links too, flagged as such.
    let beta = links("projects/beta.md");
    assert!(beta
        .iter()
        .any(|l| l.target == "alpha" && l.from_frontmatter));
    // A heading link to the same note is a self-link.
    let gamma = links("gamma.md");
    assert!(gamma
        .iter()
        .any(|l| l.is_self_link() && l.heading.as_deref() == Some("Local Section")));
    // An embed is an embed.
    let guide = links("docs/guide.md");
    assert!(guide.iter().any(|l| l.target == "alpha" && l.is_embed));
    // A wikilink inside a fenced code block is not a link.
    assert!(links("docs/rendering.md").is_empty());
    // An alias keeps its target.
    assert!(links_of(&index, "index.md")
        .iter()
        .any(|l| l.target == "projects/beta" && l.alias.as_deref() == Some("Beta project")));
    // A cross-note anchor keeps its heading and points at another note.
    assert!(links_of(&index, "index.md").iter().any(|l| {
        l.target == "long-read" && l.heading.as_deref() == Some("Part 3") && !l.is_self_link()
    }));
}

#[test]
fn the_notes_table_base_shows_the_seven_notes() {
    let index = index();
    let base = read_base(&index, "tasks/tasks.base");
    assert_eq!(
        view_rows(&index, &base, 0),
        [
            "alpha.md",
            "archive/old-note.md",
            "archive/superseded-plan.md",
            "gamma.md",
            "links/alpha-link.md",
            "reading/long-read.md",
            "tasks/tasks.md",
        ]
    );
}

#[test]
fn the_project_dashboard_views_hold_their_rows() {
    let index = index();
    let base = read_base(&index, "dashboards/projects.base");
    assert!(base.warnings.is_empty(), "{:?}", base.warnings);
    assert_eq!(base.views.len(), 4);
    assert_eq!(
        base.properties.get("status").map(String::as_str),
        Some("Status")
    );

    // The base-level filter drops the blocked project; the rest of
    // projects/ stays, nested folders included.
    let by_status = build_table(&index, &base, 0, TODAY);
    let groups: Vec<String> = by_status
        .rows
        .iter()
        .filter_map(|row| match row {
            TableRow::Group(label) => Some(label.clone()),
            TableRow::Note(_) => None,
        })
        .collect();
    assert_eq!(
        groups,
        [
            "active",
            "in progress",
            "in-progress",
            "monitoring",
            "pending",
            "planned",
            "resolved",
            "waiting-legal",
        ]
    );
    assert_eq!(by_status.note_rows(), 8);
    assert_eq!(
        by_status
            .columns
            .iter()
            .map(|c| c.display.as_str())
            .collect::<Vec<_>>(),
        ["Project", "Status", "updated"]
    );

    assert_eq!(
        view_rows(&index, &base, 1),
        ["projects/gamma.md"],
        "hub == true"
    );
    // The sentinel dates sit on either side of any 30-day window.
    assert_eq!(
        view_rows(&index, &base, 2),
        ["projects/gamma.md", "projects/roadmap.md"],
        "updated >= today - 30d, newest first"
    );
    assert_eq!(
        view_rows(&index, &base, 3),
        [
            "projects/a-note-with-a-really-long-name-that-must-truncate-in-the-sidebar.md",
            "projects/clients/acme/contract.md",
        ],
        "ops or legal; the blocked ops note is dropped by the base filter"
    );
}

#[test]
fn the_fail_open_base_keeps_every_note() {
    let index = index();
    let base = read_base(&index, "dashboards/fail-open.base");
    assert!(
        base.warnings.iter().any(|w| w.contains("file.size > 10")),
        "unknown filter warns: {:?}",
        base.warnings
    );
    assert!(
        base.warnings.iter().any(|w| w.contains("`cards`")),
        "non-table view warns: {:?}",
        base.warnings
    );
    assert_eq!(base.views.len(), 1, "the cards view is dropped");
    let table = build_table(&index, &base, 0, TODAY);
    assert_eq!(
        table.note_rows(),
        30,
        "an unknown filter matches everything"
    );
    let TableRow::Note(first) = &table.rows[0] else {
        panic!("no group rows in this view");
    };
    assert_eq!(
        first.cells[1].text, "—",
        "an unsupported formula renders an em-dash"
    );
}
