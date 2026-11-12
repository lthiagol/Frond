//! `.base` file support: a hand-rolled parser for the YAML subset the
//! vault's base files actually use, filter evaluation against the index,
//! and the table model the app renders.
//!
//! Scope (post-v1 loop 6, orchestrator-approved):
//! - **filters** — `and`/`or` combinators (nestable) of
//!   `file.inFolder(path)`, `file.hasTag(tag)`, property equality
//!   `prop == value` (quoted strings and bare booleans), inequality
//!   `prop != value` (deviation: needed by the real GPU-matrix base),
//!   and the date comparison `prop >= today() - "Nd"` where `prop`
//!   values are `YYYY-MM-DD`.
//! - **properties** — `displayName` mapping for column headers.
//! - **formulas** — ONLY `file.asLink(title)` and `file.name`; any other
//!   formula's column shows an em-dash.
//! - **views** — `type: table` with `name`, `order` (columns), `sort`
//!   (property + direction), `groupBy` (property + direction).
//!
//! Fail-open contract: anything unknown (filter expression, view type,
//! unparsable file) produces a warning and *broadens* — an unknown
//! filter matches everything, an unknown view type is dropped, a wholly
//! unparsable file renders one default all-notes view. The result can
//! only ever be wider than the author intended, never silently narrower.
//! Malformed quoting in `==`/`!=` values (an unterminated or stray
//! quote) and over-deep nesting (> [`MAX_PARSE_DEPTH`] levels) fail
//! open the same way: warning + match-everything, never a silent
//! match-nothing literal.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::index::{NoteMeta, VaultIndex};

/// A parsed base file. `warnings` carries everything that was dropped or
/// fail-opened during parsing (surfaced in the status bar by the app).
pub struct BaseFile {
    /// Base-level filters; combined (AND) with each view's own filters.
    pub filters: Filter,
    /// `properties:` entries: column key → `displayName`.
    pub properties: BTreeMap<String, String>,
    /// `formulas:` entries: formula name → expression source.
    pub formulas: BTreeMap<String, String>,
    /// Table views, in file order (non-table views dropped with a
    /// warning; an empty list becomes one synthesized default view).
    pub views: Vec<BaseView>,
    pub warnings: Vec<String>,
}

/// One table view.
pub struct BaseView {
    pub name: String,
    /// View-level filters (combined with the base-level ones).
    pub filters: Filter,
    /// Column keys in display order.
    pub order: Vec<String>,
    /// Sort entries: (property key, descending?). Primary first.
    pub sort: Vec<(String, bool)>,
    /// Group-by: (property key, descending label order?).
    pub group_by: Option<(String, bool)>,
}

/// A filter expression. [`Filter::MatchAll`] is the fail-open value: it
/// matches every note, so an unknown expression inside `and` is neutral
/// and inside `or` broadens to "everything" — the result set can only
/// grow, never silently narrow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    MatchAll,
    /// `file.inFolder("path")` — the note's vault-relative path is inside
    /// (or equal to) the folder.
    InFolder(String),
    /// `file.hasTag("tag")`.
    HasTag(String),
    /// `prop == value` (raw frontmatter string comparison).
    Eq {
        prop: String,
        value: String,
    },
    /// `prop != value` (deviation, see module docs).
    Ne {
        prop: String,
        value: String,
    },
    /// `prop >= today() - "Nd"` — the note's `YYYY-MM-DD` property is at
    /// or after `today - N days`.
    SinceDays {
        prop: String,
        days: i64,
    },
    And(Vec<Filter>),
    Or(Vec<Filter>),
}

impl Filter {
    /// Does `note` match under this filter, given `today` as
    /// `YYYY-MM-DD` (supplied by the caller so the crate stays
    /// dependency-free and tests stay deterministic)?
    pub fn matches(&self, note: &NoteMeta, today: &str) -> bool {
        match self {
            Filter::MatchAll => true,
            Filter::InFolder(folder) => {
                let folder = folder.trim_matches('/');
                folder.is_empty() || note.rel_path.starts_with(&format!("{folder}/"))
            }
            Filter::HasTag(tag) => note.tags.iter().any(|t| t == tag),
            Filter::Eq { prop, value } => note.raw.get(prop).is_some_and(|v| v == value),
            Filter::Ne { prop, value } => !note.raw.get(prop).is_some_and(|v| v == value),
            Filter::SinceDays { prop, days } => match note.raw.get(prop) {
                Some(value) => match (parse_date(value), parse_date(today)) {
                    (Some(d), Some(today_d)) => d >= today_d - days,
                    _ => false, // malformed/unparseable date: not "recent"
                },
                None => false,
            },
            Filter::And(parts) => parts.iter().all(|f| f.matches(note, today)),
            Filter::Or(parts) => parts.iter().any(|f| f.matches(note, today)),
        }
    }
}

/// One rendered column.
pub struct Column {
    /// The view-order key (`file.name`, `formula.x`, or a property name).
    pub key: String,
    /// Header text (displayName mapping applied).
    pub display: String,
}

/// One data cell.
pub struct Cell {
    pub text: String,
    /// Clickable: opens `path` (file.asLink/file.name columns).
    pub link: bool,
}

pub struct BaseNoteRow {
    /// Absolute note path (the click target of link cells).
    pub path: PathBuf,
    /// One cell per column of [`BaseTable::columns`].
    pub cells: Vec<Cell>,
    /// Group label when the view groups (empty when it does not).
    pub group: String,
}

/// A rendered table row: a group header or a note row.
pub enum TableRow {
    Group(String),
    Note(BaseNoteRow),
}

pub struct BaseTable {
    pub columns: Vec<Column>,
    pub rows: Vec<TableRow>,
}

impl BaseTable {
    /// Number of note rows (group headers excluded).
    pub fn note_rows(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r, TableRow::Note(_)))
            .count()
    }
}

/// Parses a `.base` file's source. Never fails outright: a structural
/// parse error degrades to the fail-open default (no filters, one
/// all-notes default view) with a warning.
pub fn parse_base(source: &str) -> BaseFile {
    let mut warnings = Vec::new();
    match parse_yaml_block(source, &mut warnings) {
        Ok(Yaml::Map(entries)) => base_from_entries(entries, &mut warnings),
        Ok(_) => {
            warnings.push("base: top level is not a mapping; showing all notes".to_string());
            default_base(warnings)
        }
        Err(err) => {
            warnings.push(format!("base: {err}; showing all notes (fail-open)"));
            default_base(warnings)
        }
    }
}

/// The fail-open fallback document: match-all filter and one default
/// table view over every note.
fn default_base(warnings: Vec<String>) -> BaseFile {
    BaseFile {
        filters: Filter::MatchAll,
        properties: BTreeMap::new(),
        formulas: BTreeMap::new(),
        views: vec![BaseView {
            name: "All notes".to_string(),
            filters: Filter::MatchAll,
            order: vec!["file.name".to_string(), "status".to_string()],
            sort: Vec::new(),
            group_by: None,
        }],
        warnings,
    }
}

fn base_from_entries(entries: Vec<(String, Yaml)>, warnings: &mut Vec<String>) -> BaseFile {
    let mut base = BaseFile {
        filters: Filter::MatchAll,
        properties: BTreeMap::new(),
        formulas: BTreeMap::new(),
        views: Vec::new(),
        warnings: Vec::new(),
    };
    for (key, value) in entries {
        match (key.as_str(), &value) {
            ("filters", Yaml::Map(_) | Yaml::List(_)) => {
                base.filters = parse_filter(&value, warnings);
            }
            ("properties", Yaml::Map(props)) => {
                for (prop, spec) in props {
                    if let Yaml::Map(fields) = spec {
                        for (field, display) in fields {
                            if field == "displayName" {
                                base.properties.insert(prop.clone(), display.scalar_text());
                            }
                        }
                    }
                }
            }
            ("formulas", Yaml::Map(map)) => {
                for (name, expr) in map {
                    base.formulas.insert(name.clone(), expr.scalar_text());
                }
            }
            ("views", Yaml::List(items)) => {
                for item in items {
                    match item {
                        Yaml::Map(fields) => parse_view(fields.clone(), warnings).map_or_else(
                            || (), // non-table view already warned
                            |view| base.views.push(view),
                        ),
                        _ => {
                            warnings.push("base: view entry is not a mapping; skipped".to_string())
                        }
                    }
                }
            }
            (known @ ("filters" | "properties" | "formulas" | "views"), _) => {
                warnings.push(format!(
                    "base: `{known}` has an unexpected shape; ignored (fail-open)"
                ));
            }
            (other, _) => {
                warnings.push(format!("base: unknown top-level key `{other}`; ignored"));
            }
        }
    }
    if base.views.is_empty() {
        warnings.push("base: no table views found; showing all notes".to_string());
        let mut fallback = default_base(Vec::new());
        fallback.warnings = Vec::new();
        base.views = fallback.views;
    }
    base.warnings = std::mem::take(warnings);
    base
}

/// Parses one view mapping. `None` = dropped (non-table or broken), the
/// warning is already recorded.
fn parse_view(fields: Vec<(String, Yaml)>, warnings: &mut Vec<String>) -> Option<BaseView> {
    let mut view_type: Option<String> = None;
    let mut name = String::new();
    let mut filters = Filter::MatchAll;
    let mut order = Vec::new();
    let mut sort = Vec::new();
    let mut group_by = None;
    for (key, value) in fields {
        match key.as_str() {
            "type" => view_type = Some(value.scalar_text()),
            "name" => name = value.scalar_text(),
            "filters" => filters = parse_filter(&value, warnings),
            "order" => match value {
                Yaml::List(items) => {
                    order.extend(items.into_iter().map(|i| i.scalar_text()));
                }
                Yaml::Scalar(s) if is_inline_flow(&s) => warnings.push(
                    "base: inline flow sequences are not supported (`order: []`); ignored (fail-open)"
                        .to_string(),
                ),
                _ => warnings.push("base: view `order` is not a list; ignored".to_string()),
            },
            "sort" => match value {
                Yaml::List(items) => {
                    for item in items {
                        match sort_entry(&item, warnings) {
                            Some(entry) => sort.push(entry),
                            None => warnings.push(
                                "base: sort entry without property/direction; skipped".to_string(),
                            ),
                        }
                    }
                }
                Yaml::Scalar(s) if is_inline_flow(&s) => warnings.push(
                    "base: inline flow sequences are not supported (`sort: []`); ignored (fail-open)"
                        .to_string(),
                ),
                _ => warnings.push("base: view `sort` is not a list; ignored".to_string()),
            },
            "groupBy" => match value {
                Yaml::Map(fields) => match sort_entry(&Yaml::Map(fields), warnings) {
                    Some(entry) => group_by = Some(entry),
                    None => warnings
                        .push("base: groupBy without property/direction; ignored".to_string()),
                },
                _ => warnings.push("base: view `groupBy` is not a mapping; ignored".to_string()),
            },
            other => warnings.push(format!(
                "base: unknown view key `{other}` ignored (view `{}`)",
                if name.is_empty() { "?" } else { &name }
            )),
        }
    }
    match view_type.as_deref() {
        Some("table") => Some(BaseView {
            name: if name.is_empty() {
                "table".to_string()
            } else {
                name
            },
            filters,
            order: if order.is_empty() {
                vec!["file.name".to_string()]
            } else {
                order
            },
            sort,
            group_by,
        }),
        Some(other) => {
            warnings.push(format!(
                "base: unsupported view type `{other}` dropped (fail-open)"
            ));
            None
        }
        None => {
            warnings.push("base: view without a type dropped (fail-open)".to_string());
            None
        }
    }
}

/// `sort:` list entries and `groupBy` mappings share the
/// `property`/`direction` shape.
fn sort_entry(item: &Yaml, warnings: &mut Vec<String>) -> Option<(String, bool)> {
    let Yaml::Map(fields) = item else {
        warnings.push("base: sort/group entry is not a mapping; skipped".to_string());
        return None;
    };
    let mut prop: Option<String> = None;
    let mut desc = false;
    for (key, value) in fields {
        match key.as_str() {
            "property" => prop = Some(value.scalar_text()),
            "direction" => desc = value.scalar_text().eq_ignore_ascii_case("desc"),
            other => warnings.push(format!("base: unknown sort/group key `{other}` ignored")),
        }
    }
    prop.map(|p| (p, desc))
}

/// Parses a filter node (scalar expression or `and`/`or` mapping).
fn parse_filter(value: &Yaml, warnings: &mut Vec<String>) -> Filter {
    match value {
        Yaml::Scalar(expr) => parse_expr(expr, warnings),
        Yaml::Map(fields) => {
            if fields.len() != 1 {
                warnings.push(
                    "base: filter mapping must have exactly one `and`/`or` key; fail-open"
                        .to_string(),
                );
                return Filter::MatchAll;
            }
            let (key, inner) = fields.iter().next().expect("checked len");
            let combinator = match key.as_str() {
                "and" => 0,
                "or" => 1,
                _ => {
                    warnings.push(format!(
                        "base: unknown filter combinator `{key}`; fail-open"
                    ));
                    return Filter::MatchAll;
                }
            };
            let parts = match inner {
                Yaml::List(items) => items
                    .iter()
                    .map(|item| parse_filter(item, warnings))
                    .collect::<Vec<_>>(),
                Yaml::Scalar(s) if is_inline_flow(s) => {
                    warnings.push(format!(
                        "base: inline flow sequences are not supported (`{key}: []`); fail-open"
                    ));
                    return Filter::MatchAll;
                }
                _ => {
                    warnings.push(format!(
                        "base: `{key}` filter operand is not a list; fail-open"
                    ));
                    return Filter::MatchAll;
                }
            };
            if combinator == 0 {
                Filter::And(parts)
            } else {
                Filter::Or(parts)
            }
        }
        Yaml::List(_) => {
            warnings.push("base: bare list where a filter was expected; fail-open".to_string());
            Filter::MatchAll
        }
        Yaml::Empty => Filter::MatchAll,
    }
}

/// Parses one scalar filter expression. Unknown shapes warn and return
/// [`Filter::MatchAll`] (fail-open).
fn parse_expr(expr: &str, warnings: &mut Vec<String>) -> Filter {
    let expr = expr.trim();
    let fail_open = |warnings: &mut Vec<String>| -> Filter {
        warnings.push(format!(
            "base: unknown filter expression {expr:?}; matches everything (fail-open)"
        ));
        Filter::MatchAll
    };
    if is_inline_flow(expr) {
        warnings.push(
            "base: inline flow sequences are not supported in filters; matches everything (fail-open)"
                .to_string(),
        );
        return Filter::MatchAll;
    }
    if let Some(rest) = expr.strip_prefix("file.inFolder(") {
        return match quoted_arg(rest.strip_suffix(')').unwrap_or(rest)) {
            Some(arg) => Filter::InFolder(arg),
            None => fail_open(warnings),
        };
    }
    if let Some(rest) = expr.strip_prefix("file.hasTag(") {
        return match quoted_arg(rest.strip_suffix(')').unwrap_or(rest)) {
            Some(arg) => Filter::HasTag(arg),
            None => fail_open(warnings),
        };
    }
    // Date comparison: `prop >= today() - "Nd"` (the minus term is
    // optional in principle; the vault only writes the full form).
    if let Some((prop, rhs)) = expr.split_once(">=") {
        let prop = prop.trim();
        if prop.contains(['=', '<', '>', '!']) {
            return fail_open(warnings);
        }
        return match parse_today_rhs(rhs.trim()) {
            Some(days) => Filter::SinceDays {
                prop: prop.to_string(),
                days,
            },
            None => fail_open(warnings),
        };
    }
    for (op, eq) in [("==", true), ("!=", false)] {
        if let Some((prop, value)) = expr.split_once(op) {
            let prop = prop.trim();
            let value = value.trim();
            if prop.is_empty()
                || prop.contains(['=', '<', '>', '!'])
                || value.is_empty()
                || value.contains(['=', '<', '>'])
            {
                return fail_open(warnings);
            }
            let value = match clean_quote_value(value) {
                Some(v) => v,
                // Malformed quoting (unterminated or stray quote) must
                // fail open: a quoted-literal value would silently match
                // nothing, narrowing the view without any warning.
                None => return fail_open(warnings),
            };
            return if eq {
                Filter::Eq {
                    prop: prop.to_string(),
                    value,
                }
            } else {
                Filter::Ne {
                    prop: prop.to_string(),
                    value,
                }
            };
        }
    }
    fail_open(warnings)
}

/// `"7d"` (or `7d`) → `7`; the whole `today() - "Nd"` right-hand side of
/// a `>=` comparison.
fn parse_today_rhs(rhs: &str) -> Option<i64> {
    let rest = rhs.strip_prefix("today()")?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Some(0);
    }
    let rest = rest.strip_prefix('-')?.trim();
    let inner = quoted_arg(rest).unwrap_or_else(|| rest.to_string());
    let digits = inner.strip_suffix('d')?;
    digits.parse::<i64>().ok()
}

/// Strips one pair of matching quotes and returns the inner text.
fn quoted_arg(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if value.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[value.len() - 1] == bytes[0]
    {
        Some(value[1..value.len() - 1].to_string())
    } else {
        None
    }
}

/// Strips surrounding quotes when present; otherwise the value verbatim.
fn unquote_yaml(value: &str) -> String {
    quoted_arg(value).unwrap_or_else(|| value.to_string())
}

/// Validates and unquotes an `==`/`!=` value. `None` = malformed quoting:
/// an unterminated quoted value (`"active`), a stray quote inside the
/// value (`act"ive`, `"a"b"`), or a quote in a bare value. Such values
/// must fail open (match everything + warning) — treating them as
/// literal values would silently match nothing.
fn clean_quote_value(raw: &str) -> Option<String> {
    if raw.starts_with('"') || raw.starts_with('\'') {
        let quote = raw.as_bytes()[0] as char;
        let inner = quoted_arg(raw)?;
        // A matched pair is only trusted when the quote character does
        // not reappear inside (`"a"b"` is not a clean scalar).
        if inner.contains(quote) {
            return None;
        }
        Some(inner)
    } else if raw.contains('"') || raw.contains('\'') {
        None
    } else {
        Some(raw.to_string())
    }
}

// --- table building ----------------------------------------------------

/// Builds the table for view `view_index` (caller clamps) by evaluating
/// the combined filters against every indexed note.
///
/// `today` is `YYYY-MM-DD` (the caller's clock; tests pass fixed dates).
pub fn build_table(
    index: &VaultIndex,
    base: &BaseFile,
    view_index: usize,
    today: &str,
) -> BaseTable {
    let Some(view) = base.views.get(view_index) else {
        return BaseTable {
            columns: Vec::new(),
            rows: Vec::new(),
        };
    };
    let columns: Vec<Column> = view
        .order
        .iter()
        .map(|key| Column {
            key: key.clone(),
            display: display_name(base, key),
        })
        .collect();

    let mut matches: Vec<&NoteMeta> = index
        .notes()
        .filter(|note| base.filters.matches(note, today) && view.filters.matches(note, today))
        .collect();
    sort_notes(&mut matches, &view.sort);

    let mut rows: Vec<TableRow> = Vec::new();
    match &view.group_by {
        None => {
            rows.extend(
                matches
                    .into_iter()
                    .map(|note| TableRow::Note(note_row(note, &columns, base, ""))),
            );
        }
        Some((prop, desc)) => {
            // Collect groups (label → rows in the already-sorted order),
            // then emit groups in label order; the missing-property
            // group sorts last in both directions.
            let mut groups: Vec<(String, bool, Vec<BaseNoteRow>)> = Vec::new();
            for note in matches {
                let missing = !note.raw.contains_key(prop);
                let label = note
                    .raw
                    .get(prop)
                    .cloned()
                    .unwrap_or_else(|| "(none)".to_string());
                match groups.iter_mut().find(|(g, _, _)| *g == label) {
                    Some((_, _, rows)) => rows.push(note_row(note, &columns, base, &label)),
                    None => groups.push((
                        label.clone(),
                        missing,
                        vec![note_row(note, &columns, base, "")],
                    )),
                }
            }
            groups.sort_by(|(a, a_missing, _), (b, b_missing, _)| {
                match (*a_missing, *b_missing) {
                    (true, false) => return std::cmp::Ordering::Greater,
                    (false, true) => return std::cmp::Ordering::Less,
                    _ => {}
                }
                let ord = a.to_lowercase().cmp(&b.to_lowercase());
                if *desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
            for (label, _, group_rows) in groups {
                rows.push(TableRow::Group(label.clone()));
                rows.extend(group_rows.into_iter().map(TableRow::Note));
            }
        }
    }
    BaseTable { columns, rows }
}

/// Header text for a column key: the `properties:` displayName when
/// present, else a friendly default (`formula.x` → `x`,
/// `file.name` → `Name`), else the key itself.
fn display_name(base: &BaseFile, key: &str) -> String {
    if let Some(display) = base.properties.get(key) {
        if !display.is_empty() {
            return display.clone();
        }
    }
    if let Some(name) = key.strip_prefix("formula.") {
        return name.to_string();
    }
    match key {
        "file.name" => "Name".to_string(),
        other => other.to_string(),
    }
}

/// Builds one note row's cells for `columns`.
fn note_row(note: &NoteMeta, columns: &[Column], base: &BaseFile, group: &str) -> BaseNoteRow {
    let cells = columns
        .iter()
        .map(|column| cell_for(note, &column.key, base))
        .collect();
    BaseNoteRow {
        path: note.path.clone(),
        cells,
        group: group.to_string(),
    }
}

/// The value of one column for one note.
fn cell_for(note: &NoteMeta, key: &str, base: &BaseFile) -> Cell {
    if key == "file.name" {
        return Cell {
            text: note.stem.clone(),
            link: true,
        };
    }
    if let Some(name) = key.strip_prefix("formula.") {
        // The only supported formulas: file.asLink(title) and file.name.
        let expr = base.formulas.get(name).map(|e| e.trim()).unwrap_or("");
        return if expr == "file.asLink(title)" {
            // Link text: the note's `title` property, falling back to the
            // file stem (the vault always writes `title:`, but tolerance
            // beats an empty cell).
            Cell {
                text: note
                    .raw
                    .get("title")
                    .cloned()
                    .unwrap_or_else(|| note.stem.clone()),
                link: true,
            }
        } else if expr == "file.name" {
            Cell {
                text: note.stem.clone(),
                link: true,
            }
        } else {
            // Unsupported/unknown formula: em-dash, not clickable.
            Cell {
                text: "—".to_string(),
                link: false,
            }
        };
    }
    Cell {
        text: note.raw.get(key).cloned().unwrap_or_default(),
        link: false,
    }
}

/// Sorts `notes` in place by the view's sort entries. Missing property
/// values sort after present ones regardless of direction; string
/// comparisons are lowercase-insensitive for stability. Without sort
/// entries the walk (path) order stands.
fn sort_notes(notes: &mut [&NoteMeta], sort: &[(String, bool)]) {
    if sort.is_empty() {
        return;
    }
    let directions: Vec<bool> = sort.iter().map(|(_, desc)| *desc).collect();
    type SortKeyed<'a> = Vec<(Vec<(Option<String>, bool)>, &'a NoteMeta)>;
    let mut keyed: SortKeyed<'_> = notes
        .iter()
        .map(|note| {
            let keys = sort
                .iter()
                .map(|(prop, _)| {
                    let value = sort_value(note, prop);
                    (value.clone(), value.is_some())
                })
                .collect();
            (keys, *note)
        })
        .collect();
    keyed.sort_by(|(a_keys, _), (b_keys, _)| {
        for (i, ((a, a_present), (b, b_present))) in a_keys.iter().zip(b_keys.iter()).enumerate() {
            // Missing always sorts after present, whichever direction.
            match (a_present, b_present) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }
            let ord = a
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b.as_deref().unwrap_or("").to_lowercase());
            if ord != std::cmp::Ordering::Equal {
                return if directions[i] { ord.reverse() } else { ord };
            }
        }
        std::cmp::Ordering::Equal
    });
    for (slot, (_, note)) in notes.iter_mut().zip(keyed) {
        *slot = note;
    }
}

/// The sort key of one note for one property (`file.name` sorts by stem).
fn sort_value(note: &NoteMeta, prop: &str) -> Option<String> {
    if prop == "file.name" {
        Some(note.stem.clone())
    } else {
        note.raw.get(prop).cloned()
    }
}

// --- minimal YAML subset parser -----------------------------------------

/// A parsed YAML node (the subset the base files use).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Yaml {
    /// A scalar (quotes already stripped for quoted values).
    Scalar(String),
    /// An ordered mapping.
    Map(Vec<(String, Yaml)>),
    /// A sequence.
    List(Vec<Yaml>),
    /// `key:` with no value and nothing nested beneath it.
    Empty,
}

impl Yaml {
    /// Scalar text (mapping/list stringify to their last scalar; only
    /// used where scalar shapes are expected).
    fn scalar_text(&self) -> String {
        match self {
            Yaml::Scalar(s) => s.clone(),
            Yaml::Map(entries) => entries
                .last()
                .map(|(_, v)| v.scalar_text())
                .unwrap_or_default(),
            Yaml::List(items) => items.last().map(|v| v.scalar_text()).unwrap_or_default(),
            Yaml::Empty => String::new(),
        }
    }
}

/// One significant line: its indentation and trimmed content.
struct Line {
    indent: usize,
    text: String,
}

/// Maximum block-nesting depth the parser recurses into. Real base
/// files nest a handful of levels; beyond this the file fails open
/// (same cap convention as the sidebar/engine's recursive descent).
const MAX_PARSE_DEPTH: usize = 32;

/// Collects significant lines (blank and `#`-comment lines dropped,
/// indentation measured in spaces; tabs *in indentation* are a parse
/// error — tabs inside values are tolerated, leaf-style).
fn significant_lines(source: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (number, raw) in source.lines().enumerate() {
        let trimmed_end = raw.trim_end();
        let indent = trimmed_end.len() - trimmed_end.trim_start_matches(' ').len();
        let text = trimmed_end.trim_start().to_string();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let leading_ws = raw.len() - raw.trim_start_matches([' ', '\t']).len();
        if raw[..leading_ws].contains('\t') {
            return Err(format!(
                "line {}: tabs in indentation are not supported",
                number + 1
            ));
        }
        out.push(Line { indent, text });
    }
    Ok(out)
}

/// Parses a whole document as one block node.
fn parse_yaml_block(source: &str, warnings: &mut Vec<String>) -> Result<Yaml, String> {
    let lines = significant_lines(source)?;
    if lines.is_empty() {
        return Ok(Yaml::Empty);
    }
    let mut pos = 0usize;
    let node = parse_node(&lines, &mut pos, lines[0].indent, warnings, 0)?;
    if pos < lines.len() {
        return Err(format!(
            "line {}: unexpected content after the top-level block ({:?})",
            pos + 1,
            lines[pos].text
        ));
    }
    Ok(node)
}

/// Parses one block node (mapping or list) starting at `lines[pos]` with
/// minimum indent `indent`.
fn parse_node(
    lines: &[Line],
    pos: &mut usize,
    indent: usize,
    warnings: &mut Vec<String>,
    depth: usize,
) -> Result<Yaml, String> {
    if depth > MAX_PARSE_DEPTH {
        return Err(format!(
            "nesting deeper than {MAX_PARSE_DEPTH} levels is not supported"
        ));
    }
    if *pos >= lines.len() {
        return Ok(Yaml::Empty);
    }
    if lines[*pos].text.starts_with("- ") || lines[*pos].text == "-" {
        parse_list(lines, pos, indent, warnings, depth)
    } else {
        parse_map(lines, pos, indent, warnings, depth)
    }
}

fn parse_list(
    lines: &[Line],
    pos: &mut usize,
    indent: usize,
    warnings: &mut Vec<String>,
    depth: usize,
) -> Result<Yaml, String> {
    let mut items = Vec::new();
    while *pos < lines.len() && lines[*pos].indent == indent {
        let line = &lines[*pos];
        if !(line.text.starts_with("- ") || line.text == "-") {
            break;
        }
        if line.text == "-" {
            // Bare dash: the item is the nested block beneath.
            *pos += 1;
            if *pos < lines.len() && lines[*pos].indent > indent {
                let child_indent = lines[*pos].indent;
                items.push(parse_node(lines, pos, child_indent, warnings, depth + 1)?);
            } else {
                items.push(Yaml::Empty);
            }
            continue;
        }
        let rest = line.text[2..].trim_start().to_string();
        let item_inline_indent =
            indent + 2 + (line.text[2..].len() - line.text[2..].trim_start().len());
        *pos += 1;
        // An inline `key: value` starts a mapping that continues on the
        // following lines at the key's column (classic YAML block-seq
        // item shape).
        let is_map_item = !starts_quoted(&rest) && has_top_level_colon(&rest);
        if is_map_item {
            let mut entries = Vec::new();
            let (key, value) = split_key_value(&rest);
            match value {
                Some(v) => entries.push((key, Yaml::Scalar(v))),
                None => {
                    // Nested block beneath this key.
                    if *pos < lines.len() && lines[*pos].indent > item_inline_indent {
                        let child_indent = lines[*pos].indent;
                        entries.push((
                            key,
                            parse_node(lines, pos, child_indent, warnings, depth + 1)?,
                        ));
                    } else {
                        entries.push((key, Yaml::Empty));
                    }
                }
            }
            // Continuation keys of the same item map.
            while *pos < lines.len() && lines[*pos].indent == item_inline_indent {
                let cont = &lines[*pos];
                if cont.text.starts_with("- ") || cont.text == "-" {
                    break;
                }
                let (ckey, cvalue) = split_key_value(&cont.text);
                *pos += 1;
                match cvalue {
                    Some(v) => entries.push((ckey, Yaml::Scalar(v))),
                    None => {
                        if *pos < lines.len() && lines[*pos].indent > item_inline_indent {
                            let child_indent = lines[*pos].indent;
                            entries.push((
                                ckey,
                                parse_node(lines, pos, child_indent, warnings, depth + 1)?,
                            ));
                        } else {
                            entries.push((ckey, Yaml::Empty));
                        }
                    }
                }
            }
            items.push(Yaml::Map(entries));
        } else {
            // A comment-only item (`- # note`) is an empty scalar.
            let stripped = strip_comment(&rest).trim();
            if stripped.is_empty() {
                items.push(Yaml::Empty);
            } else {
                items.push(Yaml::Scalar(unquote_yaml(stripped)));
            }
        }
    }
    if *pos < lines.len() && lines[*pos].indent > indent {
        return Err(format!(
            "line {}: unexpected deeper indentation in a list",
            *pos + 1
        ));
    }
    Ok(Yaml::List(items))
}

fn parse_map(
    lines: &[Line],
    pos: &mut usize,
    indent: usize,
    warnings: &mut Vec<String>,
    depth: usize,
) -> Result<Yaml, String> {
    let mut entries: Vec<(String, Yaml)> = Vec::new();
    while *pos < lines.len() && lines[*pos].indent == indent {
        let line = &lines[*pos];
        if line.text.starts_with("- ") || line.text == "-" {
            break;
        }
        if !has_top_level_colon(&line.text) {
            return Err(format!("line {}: expected `key:` shape", *pos + 1));
        }
        let (key, value) = split_key_value(&line.text);
        *pos += 1;
        match value {
            Some(v) => entries.push((key, Yaml::Scalar(v))),
            None => {
                if *pos < lines.len() && lines[*pos].indent > indent {
                    let child_indent = lines[*pos].indent;
                    let child = parse_node(lines, pos, child_indent, warnings, depth + 1)?;
                    entries.push((key, child));
                } else {
                    entries.push((key, Yaml::Empty));
                }
            }
        }
    }
    if *pos < lines.len() && lines[*pos].indent > indent {
        return Err(format!(
            "line {}: unexpected deeper indentation in a mapping",
            *pos + 1
        ));
    }
    Ok(Yaml::Map(entries))
}

/// True when a scalar is an inline flow sequence (`[]`, `[a, b]`),
/// which the parser does not support (block lists only).
fn is_inline_flow(text: &str) -> bool {
    let text = text.trim();
    text.len() >= 2 && text.starts_with('[') && text.ends_with(']')
}

/// True when the text begins with a quote character (a list item that is
/// a quoted string, never a mapping key).
fn starts_quoted(text: &str) -> bool {
    text.starts_with('"') || text.starts_with('\'')
}

/// True when `key:` (colon followed by space/end, outside quotes)
/// appears — i.e. the line can start a mapping entry.
fn has_top_level_colon(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b':' => {
                let at_end = i + 1 == bytes.len();
                let followed_by_space =
                    i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t');
                if at_end || followed_by_space {
                    return true;
                }
            }
            None => {}
        }
    }
    false
}

/// Splits `key: value` at the first top-level colon; `None` value = the
/// colon ended the line (nested block or empty).
fn split_key_value(text: &str) -> (String, Option<String>) {
    let bytes = text.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b':' => {
                let at_end = i + 1 == bytes.len();
                let followed_by_space =
                    i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t');
                if at_end || followed_by_space {
                    let key = text[..i].trim().to_string();
                    let rest = text[i + 1..].trim();
                    // A comment-only remainder (`key: # note`) is an
                    // empty value with a trailing comment, never a
                    // literal `# …` value.
                    let stripped = strip_comment(rest).trim();
                    let value = if stripped.is_empty() {
                        None
                    } else {
                        Some(unquote_yaml(stripped))
                    };
                    return (key, value);
                }
            }
            None => {}
        }
    }
    (text.trim().to_string(), None)
}

/// Drops a trailing ` # ...` comment outside quotes (same rule as the
/// frontmatter parser). A value that *starts* with `#` is also a comment
/// (in YAML a comment needs whitespace before the `#`, which the `key: `
/// split guarantees).
fn strip_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut quote: Option<u8> = None;
    for i in 0..bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'#' && (i == 0 || bytes[i - 1] == b' ') => return value[..i].trim_end(),
            None => {}
        }
    }
    value
}

// --- date helpers --------------------------------------------------------

/// Parses `YYYY-MM-DD` into days since the civil epoch (Howard Hinnant's
/// `days_from_civil`), `None` for anything else.
pub(crate) fn parse_date(value: &str) -> Option<i64> {
    let value = value.trim();
    let mut parts = value.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    days_from_civil(year, month, day)
}

/// Howard Hinnant's `days_from_civil` (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=9999).contains(&y) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;
    use std::path::Path;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn note(status: &str, raw: &[(&str, &str)], tags: &[&str]) -> NoteMeta {
        let mut raw_map = BTreeMap::new();
        for (k, v) in raw {
            raw_map.insert(k.to_string(), v.to_string());
        }
        NoteMeta {
            path: PathBuf::from("/v/n.md"),
            rel_path: "n.md".to_string(),
            stem: "n".to_string(),
            status: Some(status.to_string()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created: None,
            updated: None,
            archived: None,
            note_type: None,
            raw: raw_map,
            links: Vec::new(),
        }
    }

    // --- YAML subset parsing --------------------------------------------

    #[test]
    fn parses_nested_or_inside_and() {
        let base = parse_base(
            "filters:\n  and:\n    - 'hub == true'\n    - or:\n        - 'status == \"active\"'\n        - 'status == \"draft\"'\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n",
        );
        assert!(base.warnings.is_empty(), "{:?}", base.warnings);
        assert_eq!(
            base.filters,
            Filter::And(vec![
                Filter::Eq {
                    prop: "hub".into(),
                    value: "true".into()
                },
                Filter::Or(vec![
                    Filter::Eq {
                        prop: "status".into(),
                        value: "active".into()
                    },
                    Filter::Eq {
                        prop: "status".into(),
                        value: "draft".into()
                    },
                ])
            ])
        );
    }

    #[test]
    fn parses_booleans_displaynames_and_groupby() {
        let base = parse_base(
            "filters:\n  and:\n    - file.inFolder(\"projects\")\n    - hub == true\nformulas:\n  project: file.asLink(title)\nproperties:\n  formula.project:\n    displayName: Project\nviews:\n  - type: table\n    name: Projects\n    groupBy:\n      property: area\n      direction: ASC\n    order:\n      - formula.project\n      - status\n    sort:\n      - property: updated\n        direction: DESC\n",
        );
        assert!(base.warnings.is_empty(), "{:?}", base.warnings);
        assert_eq!(base.views.len(), 1);
        let view = &base.views[0];
        assert_eq!(view.name, "Projects");
        assert_eq!(view.group_by, Some(("area".to_string(), false)));
        assert_eq!(view.order, vec!["formula.project", "status"]);
        assert_eq!(view.sort, vec![("updated".to_string(), true)]);
        assert_eq!(
            base.properties.get("formula.project").map(String::as_str),
            Some("Project")
        );
    }

    #[test]
    fn unparsable_file_fails_open_with_warning() {
        let base = parse_base("this is not yaml at all\n  - broken\n");
        assert!(!base.warnings.is_empty(), "a warning is recorded");
        assert_eq!(base.filters, Filter::MatchAll);
        assert_eq!(base.views.len(), 1, "the default view exists");
        // Tabs are rejected as a structural error.
        let base = parse_base("filters:\n\tand: []\n");
        assert!(base.warnings.iter().any(|w| w.contains("tabs")));
    }

    #[test]
    fn unknown_filter_expressions_fail_open_with_warning() {
        let base = parse_base(
            "filters:\n  and:\n    - file.inFolder(\"tasks\")\n    - file.size > 100\nviews:\n  - type: table\n    name: X\n    order:\n      - file.name\n",
        );
        assert!(base
            .warnings
            .iter()
            .any(|w| w.contains("unknown filter expression")));
        // The unknown expression is match-all, the known one still narrows.
        let filter = base.filters;
        // And(InFolder, MatchAll): a note outside tasks/ must not match.
        let mut outside = note("active", &[], &[]);
        outside.rel_path = "elsewhere/n.md".into();
        let mut inside = note("active", &[], &[]);
        inside.rel_path = "tasks/n.md".into();
        assert!(!filter.matches(&outside, "2026-01-01"));
        assert!(filter.matches(&inside, "2026-01-01"));

        // Unknown expression inside `or` broadens to everything.
        let base = parse_base(
            "filters:\n  or:\n    - 'status == \"active\"'\n    - file.mtime > yesterday()\nviews:\n  - type: table\n    name: X\n    order:\n      - file.name\n",
        );
        let or_filter = base.filters;
        let other = note("draft", &[], &[]);
        assert!(
            or_filter.matches(&other, "2026-01-01"),
            "fail-open: or widens"
        );
    }

    #[test]
    fn unknown_view_types_are_dropped_with_warning() {
        let base = parse_base(
            "views:\n  - type: cards\n    name: Cards\n    order:\n      - file.name\n  - type: table\n    name: Table\n    order:\n      - file.name\n  - name: NoType\n    order:\n      - file.name\n",
        );
        assert_eq!(base.views.len(), 1);
        assert_eq!(base.views[0].name, "Table");
        assert!(base
            .warnings
            .iter()
            .any(|w| w.contains("unsupported view type `cards`")));
        assert!(base.warnings.iter().any(|w| w.contains("without a type")));
    }

    #[test]
    fn unsupported_formulas_render_em_dashes() {
        let base = parse_base(
            "formulas:\n  good: 'file.asLink(title)'\n  bad: 'file.size + \"b\"'\nviews:\n  - type: table\n    name: V\n    order:\n      - formula.good\n      - formula.bad\n      - formula.missing\n",
        );
        let index = index_with(&[("a.md", "---\nstatus: active\ntitle: Alpha\n---\n")]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        assert_eq!(table.rows.len(), 1);
        let TableRow::Note(row) = &table.rows[0] else {
            panic!("note row expected");
        };
        assert_eq!(row.cells[0].text, "Alpha");
        assert!(row.cells[0].link);
        assert_eq!(row.cells[1].text, "—");
        assert!(!row.cells[1].link);
        assert_eq!(row.cells[2].text, "—");
    }

    // --- filter evaluation truth table -----------------------------------

    #[test]
    fn filter_truth_table() {
        let today = "2026-11-11";
        let task = note(
            "active",
            &[
                ("type", "task"),
                ("hub", "true"),
                ("last_change", "2026-11-10"),
            ],
            &["gpu"],
        );
        let project = note(
            "archived",
            &[
                ("type", "project"),
                ("hub", "false"),
                ("area", "lorem-area"),
            ],
            &[],
        );
        let eq = |prop: &str, value: &str| Filter::Eq {
            prop: prop.into(),
            value: value.into(),
        };
        assert!(
            Filter::InFolder("".into()).matches(&task, today),
            "empty folder = all"
        );
        assert!(Filter::InFolder("tasks".into()).matches(
            &{
                let mut n = task.clone();
                n.rel_path = "tasks/ipsum/x.md".into();
                n
            },
            today
        ));
        assert!(!Filter::InFolder("tasks".into()).matches(&project, today));
        assert!(!Filter::InFolder("tasks".into()).matches(
            &{
                let mut n = task.clone();
                n.rel_path = "tasksheets/x.md".into();
                n
            },
            today,
        ));
        assert!(Filter::HasTag("gpu".into()).matches(&task, today));
        assert!(!Filter::HasTag("gpu".into()).matches(&project, today));
        assert!(eq("hub", "true").matches(&task, today));
        assert!(!eq("hub", "true").matches(&project, today));
        assert!(
            !eq("missing", "x").matches(&task, today),
            "missing property never equals"
        );
        assert!(Filter::Ne {
            prop: "hub".into(),
            value: "true".into()
        }
        .matches(&project, today));
        assert!(!Filter::Ne {
            prop: "hub".into(),
            value: "true".into()
        }
        .matches(&task, today));
        assert!(
            Filter::SinceDays {
                prop: "last_change".into(),
                days: 7
            }
            .matches(&task, today),
            "2026-11-10 >= 2026-11-04"
        );
        assert!(
            !Filter::SinceDays {
                prop: "last_change".into(),
                days: 7
            }
            .matches(
                &{
                    let mut n = task.clone();
                    n.raw.insert("last_change".into(), "2026-11-03".into());
                    n
                },
                today
            ),
            "2026-11-03 < 2026-11-04"
        );
        // Boundary: exactly N days ago is included (>=).
        assert!(Filter::SinceDays {
            prop: "last_change".into(),
            days: 7
        }
        .matches(
            &{
                let mut n = task.clone();
                n.raw.insert("last_change".into(), "2026-11-04".into());
                n
            },
            today
        ));
        // Malformed property date: not recent.
        assert!(!Filter::SinceDays {
            prop: "last_change".into(),
            days: 7
        }
        .matches(
            &{
                let mut n = task.clone();
                n.raw.insert("last_change".into(), "YYYY-MM-DD".into());
                n
            },
            today
        ));
        // Combinators.
        assert!(Filter::And(vec![Filter::MatchAll, Filter::MatchAll]).matches(&task, today));
        assert!(Filter::Or(vec![eq("type", "project"), eq("type", "task")]).matches(&task, today));
        assert!(
            !Filter::And(vec![eq("type", "task"), eq("status", "draft")]).matches(&task, today)
        );
        assert!(
            Filter::Or(vec![eq("type", "project"), Filter::MatchAll]).matches(&project, today),
            "unknown inside or broadens"
        );
    }

    // --- sorting and grouping ---------------------------------------------

    fn index_with(notes: &[(&str, &str)]) -> VaultIndex {
        let dir = TempDir::new("vk-base-index");
        for (name, content) in notes {
            write(&dir.path().join(name), content);
        }
        VaultIndex::build(dir.path()).unwrap()
    }

    #[test]
    fn sort_applies_direction_and_missing_goes_last() {
        let base = parse_base(
            "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    sort:\n      - property: updated\n        direction: DESC\n",
        );
        let index = index_with(&[
            ("a.md", "---\nupdated: 2026-01-01\n---\n"),
            ("b.md", "---\nupdated: 2026-03-01\n---\n"),
            ("c.md", "---\nupdated: 2026-02-01\n---\n"),
            ("d.md", "no frontmatter\n"),
        ]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        let names: Vec<String> = table
            .rows
            .iter()
            .filter_map(|r| match r {
                TableRow::Note(n) => {
                    Some(n.path.file_name().unwrap().to_string_lossy().into_owned())
                }
                TableRow::Group(_) => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["b.md", "c.md", "a.md", "d.md"],
            "DESC by updated, missing last"
        );
    }

    #[test]
    fn group_by_emits_headers_in_label_order() {
        let base = parse_base(
            "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    groupBy:\n      property: area\n      direction: ASC\n",
        );
        let index = index_with(&[
            ("a.md", "---\narea: zeta\n---\n"),
            ("b.md", "---\narea: alpha\n---\n"),
            ("c.md", "---\narea: zeta\n---\n"),
            ("d.md", "no frontmatter\n"),
        ]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        let mut labels = Vec::new();
        let mut counts = Vec::new();
        for row in &table.rows {
            match row {
                TableRow::Group(label) => {
                    labels.push(label.clone());
                    counts.push(0usize);
                }
                TableRow::Note(_) => {
                    *counts.last_mut().unwrap() += 1;
                }
            }
        }
        assert_eq!(
            labels,
            vec!["alpha", "zeta", "(none)"],
            "ASC label order, missing last-ish"
        );
        assert_eq!(counts, vec![1, 2, 1]);
        assert_eq!(table.note_rows(), 4);
    }

    #[test]
    fn view_filters_combine_with_base_filters() {
        let base = parse_base(
            "filters:\n  and:\n    - 'type == \"task\"'\nviews:\n  - type: table\n    name: Open\n    filters:\n      and:\n        - 'status == \"active\"'\n    order:\n      - file.name\n  - type: table\n    name: All\n    order:\n      - file.name\n",
        );
        let index = index_with(&[
            ("a.md", "---\ntype: task\nstatus: active\n---\n"),
            ("b.md", "---\ntype: task\nstatus: done\n---\n"),
            ("c.md", "---\ntype: project\nstatus: active\n---\n"),
        ]);
        let open = build_table(&index, &base, 0, "2026-01-01");
        assert_eq!(open.note_rows(), 1, "type AND status");
        let all = build_table(&index, &base, 1, "2026-01-01");
        assert_eq!(all.note_rows(), 2, "base filter alone");
    }

    #[test]
    fn empty_views_or_missing_order_fail_open() {
        // A views block with no tables: synthesized default view + warning.
        let base = parse_base("views: []\n");
        assert_eq!(base.views.len(), 1);
        assert!(base.warnings.iter().any(|w| w.contains("no table views")));
        // A table view without order defaults to a Name column.
        let base = parse_base("views:\n  - type: table\n    name: Bare\n");
        assert_eq!(base.views[0].order, vec!["file.name"]);
        let index = index_with(&[("x.md", "---\nstatus: active\n---\n")]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        assert_eq!(table.columns[0].display, "Name");
        assert_eq!(table.note_rows(), 1);
    }

    #[test]
    fn status_column_carries_the_status_value_for_glyph_styling() {
        let base = parse_base(
            "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n      - status\n",
        );
        let index = index_with(&[("a.md", "---\nstatus: active\n---\n")]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        let TableRow::Note(row) = &table.rows[0] else {
            panic!("note row");
        };
        assert_eq!(row.cells[1].text, "active");
        assert!(!row.cells[1].link);
    }

    #[test]
    fn file_name_column_is_clickable_and_uses_the_stem() {
        let base =
            parse_base("views:\n  - type: table\n    name: V\n    order:\n      - file.name\n");
        let index = index_with(&[("deep/note.md", "---\nstatus: active\n---\n")]);
        let table = build_table(&index, &base, 0, "2026-01-01");
        let TableRow::Note(row) = &table.rows[0] else {
            panic!("note row");
        };
        assert_eq!(row.cells[0].text, "note");
        assert!(row.cells[0].link);
        assert_eq!(row.path, index.root().join("deep/note.md"));
    }

    #[test]
    fn date_compare_uses_days_since_epoch_math() {
        assert_eq!(parse_date("2026-11-11"), parse_date("2026-11-11"));
        assert!(parse_date("2026-11-11") > parse_date("2026-11-10"));
        assert!(
            parse_date("2027-01-01") > parse_date("2026-12-31"),
            "year rollover"
        );
        assert!(
            parse_date("2026-03-01") > parse_date("2026-02-28"),
            "month rollover"
        );
        assert_eq!(parse_date("not-a-date"), None);
        assert_eq!(parse_date("2026-13-01"), None);
    }

    // --- fail-open hardening (loop-6 review fixes) ------------------------

    #[test]
    fn unterminated_or_stray_quotes_in_eq_values_fail_open() {
        // The exact reviewer case: one lost quote must NOT become a
        // quoted-literal value that silently matches nothing.
        for expr in [
            "status == \"active",
            "status == act\"ive",
            "status == \"active\" extra",
            "status == \"a\"b\"",
            "status == 'active",
            "status != \"active",
        ] {
            let src = format!(
                "filters:\n  and:\n    - '{expr}'\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n"
            );
            let base = parse_base(&src);
            // Fail-open probe: a note lacking the property must still
            // match (MatchAll nested in the combinator), never a
            // quoted-literal match-nothing.
            let probe = note("active", &[], &[]);
            assert!(
                base.filters.matches(&probe, "2026-01-01"),
                "{expr} must fail open (match everything), got {:?}",
                base.filters
            );
            assert!(
                base.warnings.iter().any(|w| w.contains("fail-open")),
                "{expr} must warn: {:?}",
                base.warnings
            );
        }
        // Clean quoting still parses, including a `#` inside quotes.
        let base = parse_base(
            "filters:\n  and:\n    - 'title == \"a # b\"'\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n",
        );
        assert!(base.warnings.is_empty(), "{:?}", base.warnings);
        assert_eq!(
            base.filters,
            Filter::And(vec![Filter::Eq {
                prop: "title".into(),
                value: "a # b".into()
            }])
        );
    }

    #[test]
    fn deeply_nested_filters_fail_open_without_crashing() {
        // A 200-level indentation pyramid: beyond the parser's depth cap
        // this must fail open (default view + warning), not overflow.
        let mut src = String::from("filters:\n");
        for level in 0..200 {
            src.push_str(&format!("{}or:\n", " ".repeat(2 + 2 * level)));
        }
        src.push_str(&format!("{}- 'a == b'\n", " ".repeat(2 + 2 * 200)));
        let base = parse_base(&src);
        assert_eq!(base.filters, Filter::MatchAll, "fail-open past the cap");
        assert_eq!(base.views.len(), 1);
        assert!(
            base.warnings.iter().any(|w| w.contains("nesting")),
            "a warning is recorded: {:?}",
            base.warnings
        );
        // Modest nesting (well under the cap) still parses.
        let ok = parse_base(
            "filters:\n  and:\n    - or:\n        - 'a == b'\n        - 'c == d'\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n",
        );
        assert!(ok.warnings.is_empty(), "{:?}", ok.warnings);
    }

    #[test]
    fn comment_only_value_is_empty_not_a_literal_hash_value() {
        // `key: # note` with a nested block beneath: previously the
        // comment became the literal value and the nested block then
        // degraded the whole file to the default view.
        let base = parse_base(
            "filters: # note\n  and:\n    - 'status == \"active\"'\nviews: # another\n  - type: table\n    name: V\n    order:\n      - file.name\n",
        );
        assert!(base.warnings.is_empty(), "{:?}", base.warnings);
        assert_eq!(
            base.filters,
            Filter::And(vec![Filter::Eq {
                prop: "status".into(),
                value: "active".into()
            }])
        );
        assert_eq!(base.views.len(), 1);
    }

    #[test]
    fn inline_flow_sequences_warn_accurately() {
        // `order: []` must not claim the value "is not a list".
        let base =
            parse_base("views:\n  - type: table\n    name: V\n    order: []\n    sort: []\n");
        assert!(
            base.warnings
                .iter()
                .any(|w| w.contains("inline flow sequences")),
            "{:?}",
            base.warnings
        );
        assert!(!base.warnings.iter().any(|w| w.contains("not a list")));
        // `and: []` / a bare `[]` filter expression likewise.
        let base = parse_base(
            "filters:\n  and: []\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n",
        );
        assert!(
            base.warnings
                .iter()
                .any(|w| w.contains("inline flow sequences")),
            "{:?}",
            base.warnings
        );
    }

    #[test]
    fn tabs_inside_values_are_tolerated_indentation_tabs_are_not() {
        // A tab inside a quoted scalar (legal YAML) no longer rejects
        // the file.
        let base = parse_base(
            "views:\n  - type: table\n    name: \"a\tb\"\n    order:\n      - file.name\n",
        );
        assert!(base.warnings.is_empty(), "{:?}", base.warnings);
        assert_eq!(base.views[0].name, "a\tb");
        // Tabs in indentation are still a structural error (fail-open).
        let base = parse_base("views:\n\t- type: table\n");
        assert!(
            base.warnings
                .iter()
                .any(|w| w.contains("tabs in indentation")),
            "{:?}",
            base.warnings
        );
    }
}
