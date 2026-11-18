//! YAML frontmatter parsing for vault notes.
//!
//! The vault uses a flat subset of YAML — `key: value` lines between `---`
//! fences, with tags as an inline `[a, b]` list and occasional trailing
//! `# comment`s on values. A full YAML parser is deliberately not used:
//! anything this parser does not understand is reported as a warning and
//! the note is still indexed (with empty metadata), per the read-only
//! viewer contract that a malformed note must never block the vault.

use std::collections::BTreeMap;

use crate::wikilink::{extract_wikilinks, LinkRef};

/// Frontmatter fields the viewer cares about. Dates stay raw strings —
/// the vault always writes `YYYY-MM-DD`, but tolerating whatever is there
/// beats failing the index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteFrontmatter {
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub archived: Option<String>,
    pub note_type: Option<String>,
}

impl NoteFrontmatter {
    /// True when every field is absent (e.g. no frontmatter at all).
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Splits a note body into `(frontmatter source, body)`. Returns `None`
/// for the first element when the file does not start with a `---` fence
/// or the fence is never closed (the caller then treats the whole file as
/// body and warns).
pub fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    let rest = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut lines = rest.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end_matches(['\n', '\r']) != "---" {
        return (None, rest);
    }
    let after_open = &rest[first.len()..];
    let mut offset = 0usize;
    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let fm = &after_open[..offset];
            let body = &after_open[offset + line.len()..];
            return (Some(fm), body);
        }
        offset += line.len();
    }
    (None, rest)
}

/// Parses frontmatter source into the known fields. Unknown keys are
/// ignored; lines without a `key:` shape are collected as warnings
/// instead of failing the whole block. Returns the fields, every
/// wikilink found in frontmatter values / block-style list items (each
/// tagged `from_frontmatter: true`, in source order), and warnings.
#[cfg(test)]
pub fn parse_frontmatter(source: &str) -> (NoteFrontmatter, Vec<LinkRef>, Vec<String>) {
    let (fm, _, links, warnings) = parse_frontmatter_with_raw(source);
    (fm, links, warnings)
}

/// [`parse_frontmatter`] plus the generic per-key map of raw scalar
/// values the `.base` table renderer filters and sorts against: every
/// top-level `key: value` entry (comment-stripped, unquoted, non-empty),
/// regardless of whether the viewer has a dedicated field for it. The
/// typed extracts stay the source of truth for their own fields; this
/// map is additive.
pub fn parse_frontmatter_with_raw(
    source: &str,
) -> (
    NoteFrontmatter,
    BTreeMap<String, String>,
    Vec<LinkRef>,
    Vec<String>,
) {
    let mut fm = NoteFrontmatter::default();
    let mut raw = BTreeMap::new();
    let mut warnings = Vec::new();
    // Final key/value entries in source order (duplicate keys keep their
    // first position but take the last value — warned about below).
    let mut entries: Vec<(String, String)> = Vec::new();
    // Index into `entries` whose value a block-style list item (`- item`)
    // extends.
    let mut last_idx: Option<usize> = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "-" || trimmed.starts_with("- ") {
            // Block-style list item continuing the previous key, e.g.
            // `related:` followed by `- "[[project]]"` lines. The item is
            // appended to that key's value (quotes stripped); unknown
            // keys just accumulate harmlessly.
            let item = trimmed[1..].trim();
            match last_idx.and_then(|i| entries.get_mut(i)) {
                Some((_, value)) => {
                    if !value.is_empty() {
                        value.push_str(", ");
                    }
                    value.push_str(item.trim_matches('"'));
                }
                None => warnings.push(format!(
                    "frontmatter list item without a preceding key: {trimmed:?}"
                )),
            }
            continue;
        }
        if line != trimmed {
            // An indented `key: value` is a nested YAML map, which this
            // flat parser does not model; hoisting it silently could
            // shadow a real top-level key, so warn and ignore it.
            warnings.push(format!(
                "indented frontmatter key ignored (nested maps unsupported): {line:?}"
            ));
            continue;
        }
        match line.split_once(':') {
            Some((key, value)) => {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                match entries.iter().position(|(k, _)| *k == key) {
                    Some(idx) => {
                        entries[idx].1 = value;
                        warnings.push(format!(
                            "duplicate frontmatter key `{key}` — last value wins"
                        ));
                        last_idx = Some(idx);
                    }
                    None => {
                        entries.push((key, value));
                        last_idx = Some(entries.len() - 1);
                    }
                }
            }
            None => warnings.push(format!("frontmatter line without `key:`: {line:?}")),
        }
    }

    let scalar = |key: &str| -> Option<String> {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| unquote(strip_comment(v).trim()).to_string())
            .filter(|v| !v.is_empty())
    };
    for (key, value) in &entries {
        let cleaned = unquote(strip_comment(value).trim()).to_string();
        if !cleaned.is_empty() {
            raw.insert(key.clone(), cleaned);
        }
    }

    fm.status = scalar("status");
    fm.created = scalar("created");
    fm.updated = scalar("updated");
    fm.archived = scalar("archived");
    fm.note_type = scalar("type");
    if let Some((_, raw)) = entries.iter().find(|(k, _)| k == "tags") {
        fm.tags = parse_tags(raw);
    }

    // Wikilinks inside frontmatter values count as outgoing links
    // (Obsidian indexes them for backlinks and the graph). Comments are
    // stripped first so a `# note` tail cannot hide or fabricate links.
    let mut links = Vec::new();
    for (_, value) in &entries {
        for mut link in extract_wikilinks(strip_comment(value)) {
            link.from_frontmatter = true;
            links.push(link);
        }
    }

    (fm, raw, links, warnings)
}

/// Drops a trailing YAML comment (` # ...`) from a scalar value. Only a
/// space-prefixed `#` counts, so values containing `#` without whitespace
/// before it (headings, hex colors) survive. Quoted values are scanned
/// only *after* their closing quote, so ` #` inside `"a # b"` is kept.
fn strip_comment(value: &str) -> &str {
    let quote = match value.chars().next() {
        Some(q @ ('"' | '\'')) => q,
        _ => return plain_comment_strip(value),
    };
    let Some(close) = value[1..].find(quote) else {
        return value; // unterminated quote: leave the value untouched
    };
    let after = close + 2; // value[after..] starts past the closing quote
    match value[after..].find(" #") {
        Some(idx) => &value[..after + idx],
        None => value,
    }
}

/// Comment stripping for unquoted values (see [`strip_comment`]).
fn plain_comment_strip(value: &str) -> &str {
    match value.find(" #") {
        Some(idx) => &value[..idx],
        None => value,
    }
}

/// Inner text of a value wrapped in one pair of matching quotes, `None`
/// when it is not so wrapped. The single quote-pair rule shared by the
/// frontmatter parser and the `.base` parser.
pub(crate) fn strip_quote_pair(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let quoted = value.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[value.len() - 1] == bytes[0];
    quoted.then(|| &value[1..value.len() - 1])
}

/// Strips one pair of matching surrounding quotes from a scalar value,
/// so a quoted known-key value (`type: "rca # draft"`) reads the same as
/// its unquoted form. Unquoted values pass through unchanged.
fn unquote(value: &str) -> &str {
    strip_quote_pair(value).unwrap_or(value)
}

/// Parses a tags value: an inline `[a, b, c]` list (the vault convention).
/// A bare scalar is tolerated as a single tag; empty yields no tags.
fn parse_tags(raw: &str) -> Vec<String> {
    let trimmed = strip_comment(raw).trim();
    let inner = match (trimmed.starts_with('['), trimmed.ends_with(']')) {
        (true, true) => &trimmed[1..trimmed.len() - 1],
        (false, false) if !trimmed.is_empty() => trimmed,
        _ => return Vec::new(),
    };
    inner
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_standard_frontmatter() {
        let src = "---\nstatus: active\ntags: [a, b]\n---\n\n# Body\n";
        let (fm, body) = split_frontmatter(src);
        assert_eq!(fm, Some("status: active\ntags: [a, b]\n"));
        assert_eq!(body, "\n# Body\n");
    }

    #[test]
    fn no_frontmatter_when_file_does_not_open_with_fence() {
        assert_eq!(split_frontmatter("# Body\n"), (None, "# Body\n"));
    }

    #[test]
    fn unterminated_fence_treats_everything_as_body() {
        let src = "---\nstatus: active\nno close ever";
        assert_eq!(split_frontmatter(src), (None, src));
    }

    #[test]
    fn tolerates_crlf_and_bom() {
        let src = "\u{feff}---\r\nstatus: active\r\n---\r\nbody";
        let (fm, body) = split_frontmatter(src);
        assert!(fm.is_some());
        assert_eq!(body, "body");
    }

    #[test]
    fn parses_all_known_fields() {
        let src = "status: complete\ntype: rca\ncreated: 2026-06-03\nupdated: 2026-10-23\narchived: 2026-10-21\ntags: [analysis, mocktag, disk]";
        let (fm, _, warnings) = parse_frontmatter(src);
        assert!(warnings.is_empty());
        assert_eq!(fm.status.as_deref(), Some("complete"));
        assert_eq!(fm.note_type.as_deref(), Some("rca"));
        assert_eq!(fm.created.as_deref(), Some("2026-06-03"));
        assert_eq!(fm.updated.as_deref(), Some("2026-10-23"));
        assert_eq!(fm.archived.as_deref(), Some("2026-10-21"));
        assert_eq!(fm.tags, vec!["analysis", "mocktag", "disk"]);
    }

    #[test]
    fn strips_yaml_comments_after_values() {
        let (fm, _, w) = parse_frontmatter("status: planned # planned → active\n");
        assert!(w.is_empty());
        assert_eq!(fm.status.as_deref(), Some("planned"));
    }

    #[test]
    fn comments_inside_quoted_values_are_kept() {
        // ` #` inside a quoted value must not be treated as a comment;
        // only a comment after the closing quote is stripped.
        let (fm, _, w) = parse_frontmatter("type: \"rca # draft\" # real comment\n");
        assert!(w.is_empty());
        assert_eq!(fm.note_type.as_deref(), Some("rca # draft"));
    }

    #[test]
    fn unterminated_quote_is_left_untouched() {
        let (fm, _, w) = parse_frontmatter("type: \"rca # draft\n");
        assert!(w.is_empty());
        assert_eq!(fm.note_type.as_deref(), Some("\"rca # draft"));
    }

    #[test]
    fn empty_values_and_placeholders_stay_raw() {
        let (fm, _, _) = parse_frontmatter("status:\narchived: YYYY-MM-DD # ONLY when archived\n");
        assert_eq!(fm.status, None);
        // Placeholder values are kept verbatim; interpreting them is the
        // caller's job (the vault has one such file).
        assert_eq!(fm.archived.as_deref(), Some("YYYY-MM-DD"));
    }

    #[test]
    fn malformed_line_warns_but_rest_parses() {
        let (fm, _, warnings) = parse_frontmatter("status: active\nthis is not yaml\n");
        assert_eq!(fm.status.as_deref(), Some("active"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("this is not yaml"));
    }

    #[test]
    fn block_style_lists_attach_to_their_key_without_warnings() {
        // Real-vault pattern: `related:` with `- "[[...]]"` items (473
        // files). Must not warn per item; `tags` in block form works too.
        let src = "status: active\nrelated:\n  - \"[[project]]\"\n  - \"[[gpu-monitoring-todo]]\"\ntags:\n  - mocktag\n  - gpu\n";
        let (fm, links, warnings) = parse_frontmatter(src);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(fm.status.as_deref(), Some("active"));
        assert_eq!(fm.tags, vec!["mocktag", "gpu"]);
        // Frontmatter wikilinks are extracted and tagged (F1).
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target, "project");
        assert_eq!(links[1].target, "gpu-monitoring-todo");
        assert!(links.iter().all(|l| l.from_frontmatter));
    }

    #[test]
    fn list_item_without_preceding_key_warns() {
        let (_, _, warnings) = parse_frontmatter("- \"[[orphan]]\"\nstatus: active\n");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("without a preceding key"));
    }

    #[test]
    fn indented_key_lines_warn_and_are_ignored() {
        // A nested-map line must not be hoisted into the top-level map
        // (it could shadow a real `status:`); warn instead.
        let src = "status: real\nparent:\n  status: shadowed-in-yaml\n";
        let (fm, _, warnings) = parse_frontmatter(src);
        assert_eq!(fm.status.as_deref(), Some("real"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("indented frontmatter key"));
    }

    #[test]
    fn duplicate_keys_last_win_with_a_warning() {
        let (fm, _, warnings) = parse_frontmatter("status: first\nstatus: second\n");
        assert_eq!(fm.status.as_deref(), Some("second"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("duplicate frontmatter key `status`"));
    }

    #[test]
    fn frontmatter_link_extraction_skips_comments_and_code_spans() {
        let src = "related: \"[[real]]\" # see [[commented]]\nnote: `[[in-code]]`\n";
        let (fm, links, warnings) = parse_frontmatter(src);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
        assert!(links[0].from_frontmatter);
        assert!(fm.is_empty());
    }

    #[test]
    fn unknown_keys_ignored() {
        let (fm, _, w) = parse_frontmatter("status: active\ncustom: whatever\n");
        assert!(w.is_empty());
        assert_eq!(fm.status.as_deref(), Some("active"));
        assert!(!fm.is_empty());
    }

    #[test]
    fn tags_bare_scalar_and_empty_list() {
        assert_eq!(parse_tags("[a, b]"), vec!["a", "b"]);
        assert_eq!(parse_tags("solo"), vec!["solo"]);
        assert!(parse_tags("[]").is_empty());
        assert!(parse_tags("").is_empty());
        let (fm, _, _) = parse_frontmatter("tags: [x, y] # comment\n");
        assert_eq!(fm.tags, vec!["x", "y"]);
    }
}
