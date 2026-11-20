//! Wikilink extraction from note bodies (and frontmatter values).
//!
//! Recognizes the vault's link forms:
//! - `[[target]]` — bare filename (no extension)
//! - `[[target|alias]]` — display text (`\|` is the escaped separator used
//!   in tables; the backslash is dropped from target and alias)
//! - `[[target#heading]]` / `[[#heading]]` — heading anchor (`[[#…]]`
//!   targets the containing note)
//! - `[[dir/name]]` — path-style link (vault rule: bare filename first,
//!   path fallback)
//! - `![[target]]` — embed, tagged via [`LinkRef::is_embed`]
//!
//! Links inside fenced code blocks (``` / ~~~) and inline code spans are
//! skipped, matching Obsidian's behavior of not linking inside code.

/// One extracted `[[…]]` reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRef {
    /// Link target before any `#heading` and `|alias`, with a trailing
    /// `.md` stripped (the suffix test is case-insensitive, like the
    /// walker's). Empty for `[[#heading]]` (a link to the containing
    /// note itself).
    pub target: String,
    /// Heading anchor, if the link carried a `#…` part.
    pub heading: Option<String>,
    /// Display text, if the link carried a `|alias` part.
    pub alias: Option<String>,
    /// The raw `[[…]]` inner text as written.
    pub raw: String,
    /// True when the link was extracted from frontmatter (a value or a
    /// block-style list item) rather than the note body — Obsidian counts
    /// these as outgoing links too.
    pub from_frontmatter: bool,
    /// True for `![[…]]` embeds. Tracked (not dropped) so the renderer can
    /// tell them apart, but they are not counted as ordinary unresolved
    /// links.
    pub is_embed: bool,
}

impl LinkRef {
    /// True for `[[#heading]]`-style links, which point at the note
    /// containing them and therefore never produce backlinks.
    pub fn is_self_link(&self) -> bool {
        self.target.is_empty()
    }
}

/// One wikilink occurrence with its byte range in the source text.
///
/// The range covers the full `[[…]]` (and the leading `!` for embeds), so
/// callers can splice the source around it (the app's preprocessing hook
/// does exactly that). Offsets are byte offsets into the scanned text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WikilinkMatch {
    pub start: usize,
    pub end: usize,
    pub link: LinkRef,
}

/// Extracts every wikilink from a note body in document order.
pub fn extract_wikilinks(body: &str) -> Vec<LinkRef> {
    extract_wikilink_matches(body)
        .into_iter()
        .map(|m| m.link)
        .collect()
}

/// Extracts every wikilink with its byte range, in document order.
///
/// Line splitting mirrors `str::lines` semantics but tracks raw byte
/// offsets (each `\n` counts as one byte, a trailing `\r` stays part of
/// the line), so ranges are exact even for CRLF files.
pub fn extract_wikilink_matches(body: &str) -> Vec<WikilinkMatch> {
    let mut matches = Vec::new();
    let mut in_fence = false;
    let mut fence_marker = "";
    let mut offset = 0usize;

    for line in body.split('\n') {
        let trimmed = line.trim_start();
        if in_fence {
            // Any line starting with the opening marker closes the block
            // (``` or ~~~, possibly with trailing info/fence chars).
            if trimmed.starts_with(fence_marker) {
                in_fence = false;
                fence_marker = "";
            }
        } else if trimmed.starts_with("```") {
            in_fence = true;
            fence_marker = "```";
        } else if trimmed.starts_with("~~~") {
            in_fence = true;
            fence_marker = "~~~";
        } else {
            scan_line(line, offset, &mut matches);
        }
        offset += line.len() + 1;
    }
    matches
}

/// Scans one line outside fenced blocks, skipping inline code spans and
/// escaped brackets. `base` is the line's byte offset in the full text.
fn scan_line(line: &str, base: usize, out: &mut Vec<WikilinkMatch>) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2, // escaped char: skip both
            b'`' => {
                // Inline code span: skip to the matching backtick.
                if let Some(close) = line[i + 1..].find('`') {
                    i += close + 2;
                } else {
                    return; // unterminated span: rest of line is code
                }
            }
            b'[' if i + 1 < bytes.len() && bytes[i + 1] == b'[' => {
                // An embed (`![[…]]`) is the same link syntax preceded by
                // `!`; tag it so callers can tell embeds from links. The
                // reported range includes the `!` so a splice removes the
                // whole embed.
                let is_embed = i > 0 && bytes[i - 1] == b'!';
                let range_start = if is_embed { i - 1 } else { i };
                // Find the matching `]]`; if the candidate inner text
                // itself contains `[[` (e.g. `[[oops [[fine]]`), restart
                // at the inner opener so the well-formed link still wins.
                let mut start = i + 2;
                let mut matched = false;
                while let Some(end) = line[start..].find("]]") {
                    let inner = &line[start..start + end];
                    match inner.rfind("[[") {
                        Some(nested) => start += nested + 2,
                        None => {
                            out.push(WikilinkMatch {
                                start: base + range_start,
                                end: base + start + end + 2,
                                link: parse_inner(inner, is_embed),
                            });
                            i = start + end + 2;
                            matched = true;
                            break;
                        }
                    }
                }
                if !matched {
                    i = start.max(i + 2); // no closing `]]`: not a link
                }
            }
            _ => i += 1,
        }
    }
}

/// Parses the inner text of a `[[…]]` into a [`LinkRef`].
fn parse_inner(inner: &str, is_embed: bool) -> LinkRef {
    let raw = inner.to_string();
    // The first `|` separates target from alias. The vault's index-table
    // convention escapes it as `\|` so the table parser keeps the cell
    // intact; within the wikilink that backslash belongs to the
    // separator and is dropped from the target.
    let (target_part, alias) = match inner.split_once('|') {
        Some((t, a)) => (
            t.strip_suffix('\\').unwrap_or(t),
            Some(a.trim().to_string()),
        ),
        None => (inner, None),
    };
    let (target_part, heading) = match target_part.split_once('#') {
        Some((t, h)) => (t, Some(h.trim().to_string())),
        None => (target_part, None),
    };
    let mut target = target_part.trim().to_string();
    if let Some(stripped) = crate::walk::strip_suffix_ignore_case(&target, ".md") {
        target = stripped.to_string();
    }
    LinkRef {
        target,
        heading,
        alias,
        raw,
        from_frontmatter: false,
        is_embed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_and_aliased_links() {
        let body = "See [[analysis]] and [[analysis|the deep dive]].";
        let links = extract_wikilinks(body);
        assert_eq!(
            links,
            vec![
                LinkRef {
                    target: "analysis".into(),
                    heading: None,
                    alias: None,
                    raw: "analysis".into(),
                    from_frontmatter: false,
                    is_embed: false,
                },
                LinkRef {
                    target: "analysis".into(),
                    heading: None,
                    alias: Some("the deep dive".into()),
                    raw: "analysis|the deep dive".into(),
                    from_frontmatter: false,
                    is_embed: false,
                },
            ]
        );
    }

    #[test]
    fn heading_and_self_links() {
        let links = extract_wikilinks("[[note#Summary]] [[#Changes since 2026-06-23]]");
        assert_eq!(links[0].target, "note");
        assert_eq!(links[0].heading.as_deref(), Some("Summary"));
        assert!(links[0].raw == "note#Summary");
        assert!(links[1].is_self_link());
        assert_eq!(
            links[1].heading.as_deref(),
            Some("Changes since 2026-06-23")
        );
    }

    #[test]
    fn path_style_links() {
        let links = extract_wikilinks("[[execution-log/index]] [[archive/index|Archive]]");
        assert_eq!(links[0].target, "execution-log/index");
        assert_eq!(links[1].target, "archive/index");
        assert_eq!(links[1].alias.as_deref(), Some("Archive"));
    }

    #[test]
    fn md_suffix_stripped_case_insensitively() {
        // The walker indexes `X.MD`, so the link parser must treat its
        // suffix the same way and leave the rest of the stem intact.
        let links = extract_wikilinks("[[analysis.md]] [[analysis.MD]] [[Dir/Note.Md]]");
        assert_eq!(links[0].target, "analysis");
        assert_eq!(links[1].target, "analysis");
        assert_eq!(links[2].target, "Dir/Note");
    }

    #[test]
    fn code_blocks_and_spans_are_skipped() {
        let body = "\
Inline `[[not-a-link]]` stays code.

```text
[[fenced-link]]
```

Real: [[real-link]]

~~~
[[tilded-link]]
~~~
";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real-link");
    }

    #[test]
    fn unterminated_brackets_are_not_links() {
        let links = extract_wikilinks("oops [[broken and [[fine]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "fine");
    }

    #[test]
    fn escaped_brackets_skipped() {
        let links = extract_wikilinks("\\[[escaped]] [[real]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "real");
    }

    #[test]
    fn document_order_preserved() {
        let links = extract_wikilinks("[[b]] mid [[a]] end [[c]]");
        let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, vec!["b", "a", "c"]);
    }

    #[test]
    fn escaped_alias_separator_inside_table_row() {
        // The vault's index-table convention (`[[stem\|Label]]`): the
        // backslash belongs to the separator, not to target or alias.
        let body = "| Task | Link |\n|------|------|\n| todo | [[20260424-monstrao-todo\\|pending-tasks]] |\n";
        let links = extract_wikilinks(body);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "20260424-monstrao-todo");
        assert_eq!(links[0].alias.as_deref(), Some("pending-tasks"));
        assert_eq!(links[0].raw, "20260424-monstrao-todo\\|pending-tasks");
        assert!(!links[0].is_embed);
    }

    #[test]
    fn unescaped_pipe_still_splits() {
        let links = extract_wikilinks("[[note|Label]]");
        assert_eq!(links[0].target, "note");
        assert_eq!(links[0].alias.as_deref(), Some("Label"));
    }

    #[test]
    fn embeds_are_tagged_not_dropped() {
        let links = extract_wikilinks("Dashboard:\n\n![[tasks.base]]\n\nSee [[plain]].");
        assert_eq!(links.len(), 2);
        assert!(links[0].is_embed);
        assert_eq!(links[0].target, "tasks.base");
        assert!(!links[1].is_embed);
        assert_eq!(links[1].target, "plain");
    }

    #[test]
    fn bang_alone_is_not_an_embed_marker_for_later_links() {
        // `!` far from the `[[` must not tag the link as an embed.
        let links = extract_wikilinks("Wow! A link [[real]] here.");
        assert_eq!(links.len(), 1);
        assert!(!links[0].is_embed);
    }

    #[test]
    fn ranges_cover_the_full_bracket_span() {
        let body = "See [[alpha]] then [[beta|B]] end.";
        let matches = extract_wikilink_matches(body);
        assert_eq!(matches.len(), 2);
        assert_eq!(&body[matches[0].start..matches[0].end], "[[alpha]]");
        assert_eq!(&body[matches[1].start..matches[1].end], "[[beta|B]]");
        assert_eq!(matches[0].link.target, "alpha");
        assert_eq!(matches[1].link.alias.as_deref(), Some("B"));
    }

    #[test]
    fn ranges_span_multiple_lines_with_absolute_offsets() {
        let body = "first [[one]]\nsecond [[two]]\n";
        let matches = extract_wikilink_matches(body);
        assert_eq!(matches.len(), 2);
        assert_eq!(&body[matches[0].start..matches[0].end], "[[one]]");
        assert_eq!(&body[matches[1].start..matches[1].end], "[[two]]");
        assert!(matches[1].start > matches[0].end, "second link is later");
    }

    #[test]
    fn embed_ranges_include_the_bang() {
        let body = "![[board.base]] and [[plain]]";
        let matches = extract_wikilink_matches(body);
        assert_eq!(matches.len(), 2);
        assert_eq!(&body[matches[0].start..matches[0].end], "![[board.base]]");
        assert!(matches[0].link.is_embed);
        assert_eq!(&body[matches[1].start..matches[1].end], "[[plain]]");
    }

    #[test]
    fn ranges_skip_code_and_fences_like_extract() {
        let body = "`[[inline]]`\n```\n[[fenced]]\n```\n[[real]]";
        let matches = extract_wikilink_matches(body);
        assert_eq!(matches.len(), 1);
        assert_eq!(&body[matches[0].start..matches[0].end], "[[real]]");
    }

    #[test]
    fn crlf_offsets_stay_exact() {
        let body = "a [[one]]\r\nb [[two]]\r\n";
        let matches = extract_wikilink_matches(body);
        assert_eq!(matches.len(), 2);
        assert_eq!(&body[matches[0].start..matches[0].end], "[[one]]");
        assert_eq!(&body[matches[1].start..matches[1].end], "[[two]]");
    }
}
