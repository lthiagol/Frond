use super::{rendered_non_empty_lines, test_assets, test_md_theme};
use crate::markdown::display_width;
use crate::markdown::{parse_markdown, parse_markdown_with_width, parse_markdown_with_width_opts};
use crate::*;

#[test]
fn h1_headings_render_double_rule_without_bottom_spacing() {
    let (ss, theme) = test_assets();
    let (lines, _, _, _) =
        parse_markdown("# 東京\n", &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    assert_eq!(rendered[0], "東京");
    assert_eq!(rendered[1], "═".repeat(display_width("東京")));
}

#[test]
fn paragraph_and_following_code_block_have_no_blank_gap() {
    let (ss, theme) = test_assets();
    let src = "Intro paragraph\n\n```rs\nfn main() {}\n```\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();
    let intro_idx = rendered
        .iter()
        .position(|line| line == "Intro paragraph")
        .unwrap();

    assert!(rendered[intro_idx + 1].starts_with("┌─ rs "));
}

#[test]
fn returning_from_nested_blockquote_inserts_outer_prefix_gap() {
    let (ss, theme) = test_assets();
    let src = "> outer\n>\n> > nested\n>\n> back\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let nested_idx = rendered
        .iter()
        .position(|l| l == "▏ ▏ nested")
        .expect("missing nested line");
    let back_idx = rendered
        .iter()
        .position(|l| l == "▏ back")
        .expect("missing back line");
    assert!(nested_idx < back_idx);
    let between = &rendered[nested_idx + 1..back_idx];
    assert!(
        between.iter().any(|l| l == "▏ "),
        "expected an outer-depth prefixed blank between nested and back, got {between:?}"
    );
}

#[test]
fn blockquote_last_paragraph_has_no_trailing_prefixed_blank_inside() {
    let (ss, theme) = test_assets();
    let src = "> a\n> b\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let last_bq_idx = rendered
        .iter()
        .rposition(|l| l.starts_with('▏'))
        .expect("expected at least one blockquote line");
    assert_eq!(
        rendered[last_bq_idx], "▏ b",
        "last blockquote line should be the content, not a prefixed blank, got {rendered:?}"
    );
}

#[test]
fn consecutive_blockquotes_preserve_blank_line_between() {
    let (ss, theme) = test_assets();
    let src = "> [!NOTE]\n> Note content.\n\n> [!TIP]\n> Tip content.\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let note_content_idx = rendered
        .iter()
        .position(|l| l == "▏ Note content.")
        .expect("missing Note content line");
    let tip_header_idx = rendered
        .iter()
        .position(|l| l.contains("Tip"))
        .expect("missing Tip header");
    assert!(note_content_idx < tip_header_idx);
    let between = &rendered[note_content_idx + 1..tip_header_idx];
    assert!(
        between.iter().any(|l| l.is_empty()),
        "expected a blank separator between consecutive blockquotes, got {between:?}"
    );
}

#[test]
fn top_level_paragraph_before_blockquote_preserves_blank_separator() {
    let (ss, theme) = test_assets();
    let src = "vs\n\n> a\n> b\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let vs_idx = rendered
        .iter()
        .position(|l| l == "vs")
        .expect("missing 'vs'");
    let bq_idx = rendered
        .iter()
        .position(|l| l == "▏ a")
        .expect("missing '▏ a'");
    assert!(vs_idx < bq_idx);
    let between = &rendered[vs_idx + 1..bq_idx];
    assert!(
        between.iter().any(|l| l.is_empty()),
        "expected blank separator between top-level paragraph and blockquote, got {between:?}"
    );
}

#[test]
fn nested_blockquote_directly_after_paragraph_has_no_prefixed_gap_between() {
    let (ss, theme) = test_assets();
    let src = "> [!WARNING]\n> Warning message.\n> > Nested inside warning.\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let msg_idx = rendered
        .iter()
        .position(|l| l == "▏ Warning message.")
        .expect("missing warning message");
    let nested_idx = rendered
        .iter()
        .position(|l| l == "▏ ▏ Nested inside warning.")
        .expect("missing nested line");
    assert_eq!(
        nested_idx,
        msg_idx + 1,
        "expected nested blockquote directly after paragraph without prefixed gap, got {rendered:?}"
    );
}

#[test]
fn explicit_bq_gap_before_nested_produces_prefixed_blank() {
    let (ss, theme) = test_assets();
    let src = "> outer paragraph\n>\n> > nested paragraph\n>\n> back to outer\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let outer_idx = rendered
        .iter()
        .position(|l| l == "▏ outer paragraph")
        .expect("missing outer");
    let nested_idx = rendered
        .iter()
        .position(|l| l == "▏ ▏ nested paragraph")
        .expect("missing nested");
    assert!(outer_idx < nested_idx);
    let between = &rendered[outer_idx + 1..nested_idx];
    assert!(
        between.iter().any(|l| l == "▏ "),
        "expected a prefixed blank between outer paragraph and nested (source has '>' empty), got {between:?}"
    );
}

#[test]
fn nested_blockquotes_keep_quote_prefix_after_inner_quote_ends() {
    let (ss, theme) = test_assets();
    let src = "> outer\n>\n> > inner\n>\n> outer again\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    assert!(rendered.iter().any(|line| line == "▏ outer"));
    assert!(rendered.iter().any(|line| line == "▏ ▏ inner"));
    assert!(rendered.iter().any(|line| line == "▏ outer again"));
}

#[test]
fn long_blockquotes_wrap_into_multiple_prefixed_lines() {
    let (ss, theme) = test_assets();
    let src = "> This is a long blockquote line that should wrap into multiple quoted lines at narrow widths.\n";
    let (lines, _, _, _) =
        parse_markdown_with_width(src, &ss, &theme, 28, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);
    let quoted: Vec<_> = rendered
        .into_iter()
        .filter(|line| line.starts_with('▏'))
        .collect();

    assert!(quoted.len() >= 2);
    assert!(quoted.iter().all(|line| line.starts_with("▏ ")));
}

#[test]
fn nested_blockquote_emits_depth_two_markers() {
    let (ss, theme) = test_assets();
    let src = "> > inner\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    assert!(rendered.iter().any(|line| line == "▏ ▏ inner"));
}

#[test]
fn triple_nested_blockquote_emits_depth_three_markers() {
    let (ss, theme) = test_assets();
    let src = "> > > deepest\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    assert!(rendered.iter().any(|line| line == "▏ ▏ ▏ deepest"));
}

#[test]
fn nested_blockquote_returns_to_single_marker_after_close() {
    let (ss, theme) = test_assets();
    let src = "> outer\n>\n> > inner\n>\n> back\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    let outer_idx = rendered.iter().position(|l| l == "▏ outer").expect("outer");
    let inner_idx = rendered
        .iter()
        .position(|l| l == "▏ ▏ inner")
        .expect("inner");
    let back_idx = rendered.iter().position(|l| l == "▏ back").expect("back");
    assert!(outer_idx < inner_idx && inner_idx < back_idx);
}

#[test]
fn blank_line_between_paragraphs_in_blockquote_is_prefixed() {
    let (ss, theme) = test_assets();
    let src = "> para 1\n>\n> para 2\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered: Vec<String> = lines.iter().map(line_plain_text).collect();

    let p1_idx = rendered
        .iter()
        .position(|l| l == "▏ para 1")
        .expect("para 1");
    let p2_idx = rendered
        .iter()
        .position(|l| l == "▏ para 2")
        .expect("para 2");
    assert!(p1_idx < p2_idx);
    let between = &rendered[p1_idx + 1..p2_idx];
    assert!(
        between.iter().any(|l| l == "▏ "),
        "expected a blockquote-prefixed blank line between paragraphs, got {between:?}"
    );
}

#[test]
fn alert_header_outer_color_only_applies_to_first_marker() {
    let (ss, theme) = test_assets();
    let src = "> [!NOTE]\n> > nested\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let nested_line = lines
        .iter()
        .find(|l| line_plain_text(l).contains("nested"))
        .expect("nested line");
    let markers: Vec<_> = nested_line
        .spans
        .iter()
        .filter(|s| s.content.as_ref().contains('▏'))
        .collect();

    assert_eq!(
        markers.len(),
        2,
        "expected 2 blockquote markers, got {markers:?}"
    );
    let alert_color = test_md_theme().alert_note;
    let default_color = test_md_theme().blockquote_marker;
    assert_eq!(markers[0].style.fg, Some(alert_color));
    assert_eq!(markers[1].style.fg, Some(default_color));
}

#[test]
fn frontmatter_is_ignored_in_preview() {
    let (ss, theme) = test_assets();
    let src = "---\ntitle: Demo\nowner: me\n---\n# Visible\nBody\n";
    let (lines, _, _, _) = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    assert!(!rendered.iter().any(|line| line.contains("title: Demo")));
    assert!(rendered.iter().any(|line| line.contains("Visible")));
}

/// `show_metadata = false` hides the rendered frontmatter block entirely
/// (vertical and compact shapes alike); the body starts at the very top
/// and the source map still points into the real file (frontmatter lines
/// included).
#[test]
fn frontmatter_metadata_block_can_be_hidden() {
    let (ss, theme) = test_assets();
    let horizontal = "---\ntitle: Demo\nowner: me\n---\n# Visible\nBody\n";
    let vertical = "---\na: 1\nb: 2\nc: 3\nd: 4\ne: 5\n---\n# Visible\nBody\n";

    for src in [horizontal, vertical] {
        let result = parse_markdown_with_width_opts(
            src,
            &ss,
            &theme,
            80,
            &test_md_theme(),
            false,
            true,
            false,
        );
        let lines = &result.lines;
        let rendered = rendered_non_empty_lines(lines);
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("Demo") || line.contains("owner")),
            "metadata block hidden: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|line| line.contains("Visible")),
            "body still rendered"
        );
        // The "# Visible" heading's rendered row maps to its true on-disk
        // line (frontmatter included), so scroll-to-source stays accurate
        // with the block hidden: horizontal → line 5, vertical → line 8.
        let expected_first = if src == horizontal { 5 } else { 8 };
        let heading_row = lines
            .iter()
            .position(|l| line_plain_text(l).contains("Visible"))
            .expect("heading rendered");
        assert_eq!(
            result.source_line_map.get(heading_row).copied(),
            Some(expected_first),
            "source map keeps the frontmatter offset"
        );
    }

    // The shown variant (the default) still renders the block.
    let result = parse_markdown_with_width_opts(
        horizontal,
        &ss,
        &theme,
        80,
        &test_md_theme(),
        false,
        true,
        true,
    );
    let rendered = rendered_non_empty_lines(&result.lines);
    assert!(
        rendered.iter().any(|line| line.contains("Demo")),
        "metadata block shown by default"
    );
}

/// Frontmatter values parse as inline markdown: a markdown link in a
/// value — the shape the app's wikilink preprocessing gives `related:`
/// entries (`"[alias](vault://N/target)"`) — renders as a real link
/// (marker + label), never as raw `[text](url)` syntax, and registers
/// into `link_urls`/`link_spans` so the app can make it clickable. The
/// block renders before the body, so its links take the first ids.
#[test]
fn frontmatter_metadata_values_render_links() {
    let (ss, theme) = test_assets();
    let src = "---\nstatus: active\nrelated: \"[alpha](vault://0/alpha)\", \"[beta](vault://1/beta)\"\n---\n# Body\n";
    let result =
        parse_markdown_with_width_opts(src, &ss, &theme, 80, &test_md_theme(), false, true, true);
    let rendered = rendered_non_empty_lines(&result.lines);
    let related_row = rendered
        .iter()
        .find(|line| line.contains("alpha"))
        .expect("related value row rendered");
    assert!(
        !related_row.contains("vault://"),
        "no raw link syntax in the value cell: {related_row:?}"
    );
    assert!(
        related_row.contains("alpha") && related_row.contains("beta"),
        "both link labels render: {related_row:?}"
    );
    assert_eq!(
        result.link_urls.first().map(String::as_str),
        Some("vault://0/alpha")
    );
    assert_eq!(
        result.link_urls.get(1).map(String::as_str),
        Some("vault://1/beta")
    );
    assert!(
        result.link_spans.iter().any(|s| s.link_id.0 == 0),
        "the frontmatter link has a clickable span"
    );
}

#[test]
fn h2_headings_are_underlined_and_compact() {
    let (ss, theme) = test_assets();
    let (lines, _, _, _) = parse_markdown_with_width(
        "Intro\n\n## Section\nBody\n",
        &ss,
        &theme,
        40,
        &test_md_theme(),
        false,
        true,
    )
    .into();
    let rendered = rendered_non_empty_lines(&lines);

    assert!(rendered.iter().any(|line| line.contains("Section")));
    assert!(rendered.iter().any(|line| line.contains("────")));
}

#[test]
fn rules_use_render_width_without_extra_blank_after() {
    let (ss, theme) = test_assets();
    let (lines, _, _, _) = parse_markdown_with_width(
        "Alpha\n\n---\nBeta\n",
        &ss,
        &theme,
        24,
        &test_md_theme(),
        false,
        true,
    )
    .into();
    let rendered = rendered_non_empty_lines(&lines);
    let rule = rendered
        .iter()
        .find(|line| line.trim_start().starts_with('─'))
        .unwrap();

    assert_eq!(display_width(rule.trim_start()), 24);
    let rule_idx = rendered.iter().position(|line| line == rule).unwrap();
    assert_eq!(rendered[rule_idx + 1], "Beta");
}

#[test]
fn body_dashes_are_thematic_breaks_not_metadata_block() {
    let (ss, theme) = test_assets();
    let src = "# Title\n\n---\n### SSL\n\n- item 1\n- item 2\n\n---\n### TLS\nend\n";
    let (lines, _, _, _) =
        parse_markdown_with_width(src, &ss, &theme, 40, &test_md_theme(), false, true).into();
    let rendered = rendered_non_empty_lines(&lines);

    let rule_count = rendered
        .iter()
        .filter(|line| line.trim_start().starts_with('─'))
        .count();
    assert!(
        rule_count >= 2,
        "expected two thematic breaks, got {rule_count}"
    );
    assert!(rendered.iter().any(|line| line.contains("SSL")));
    assert!(rendered.iter().any(|line| line.contains("item 1")));
    assert!(rendered.iter().any(|line| line.contains("item 2")));
    assert!(rendered.iter().any(|line| line.contains("TLS")));
}

#[test]
fn source_line_map_plain_document_is_aligned_with_first_event() {
    let (ss, theme) = test_assets();
    let src = "# Title\n\nfirst paragraph\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    let first = parsed.source_line_map[0];
    let paragraph_idx = parsed
        .lines
        .iter()
        .position(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .contains("first paragraph")
        })
        .expect("paragraph rendered");
    assert_eq!(first, 1);
    assert_eq!(parsed.source_line_map[paragraph_idx], 3);
}

/// Nested list items flush after the inner list ends, so the flush-time
/// event cursor sits past the outer item's own line. The item's start
/// line must be snapshotted instead (VENDORED item 3a): the outer item's
/// rendered lines map to raw line 1, the inner item's to raw line 2.
#[test]
fn source_line_map_nested_list_items_point_to_their_own_lines() {
    let (ss, theme) = test_assets();
    let src = "- outer\n  - inner\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    let rendered: Vec<String> = parsed.lines.iter().map(line_plain_text).collect();
    let outer = rendered
        .iter()
        .position(|l| l.contains("outer"))
        .expect("outer item rendered");
    let inner = rendered
        .iter()
        .position(|l| l.contains("inner"))
        .expect("inner item rendered");
    assert_eq!(parsed.source_line_map[outer], 1, "outer item's own line");
    assert_eq!(parsed.source_line_map[inner], 2, "inner item's own line");
}

#[test]
fn source_line_map_skips_code_block_fence_drift() {
    let (ss, theme) = test_assets();
    let src = "intro\n\n```rs\nfn main() {}\n```\nend\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    let rendered: Vec<String> = parsed.lines.iter().map(line_plain_text).collect();
    let code_row = rendered
        .iter()
        .position(|l| l.contains("fn main()"))
        .expect("code rendered");
    assert!(parsed.source_line_map[code_row] >= 3);
    let end_row = rendered
        .iter()
        .position(|l| l == "end")
        .expect("end rendered");
    assert_eq!(parsed.source_line_map[end_row], 6);
}

#[test]
fn source_line_map_frontmatter_lines_point_to_line_one() {
    let (ss, theme) = test_assets();
    let src = "---\ntitle: Hello\nauthor: Me\n---\nbody\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.source_line_map[0], 1);
    let rendered: Vec<String> = parsed.lines.iter().map(line_plain_text).collect();
    let body_idx = rendered
        .iter()
        .position(|l| l == "body")
        .expect("body rendered");
    assert_eq!(parsed.source_line_map[body_idx], 5);
}

#[test]
fn source_line_map_file_mode_first_content_line_is_one() {
    let (ss, theme) = test_assets();
    let wrapped = "````rs\nfn main() {}\n````";
    let parsed = parse_markdown(wrapped, &ss, &theme, &test_md_theme(), true, true);
    let rendered: Vec<String> = parsed.lines.iter().map(line_plain_text).collect();
    let code_row = rendered
        .iter()
        .position(|l| l.contains("fn main()"))
        .expect("code rendered");
    assert_eq!(parsed.source_line_map[code_row], 1);
}

#[test]
fn source_line_map_padding_repeats_last_event_source_line() {
    let (ss, theme) = test_assets();
    let src = "first\n\nmiddle\n\nlast\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    let total = parsed.lines.len();
    let last = parsed.source_line_map[total - 1];
    assert_eq!(last, 5);
}

#[test]
fn code_blocks_capture_raw_content_with_trailing_newline() {
    let (ss, theme) = test_assets();
    let src = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 1);
    let block = &parsed.code_blocks[0];
    assert_eq!(block.raw_content, "fn main() {\n    println!(\"hi\");\n}\n");
    assert!(block.rendered_start < block.rendered_end);
    assert!(block.rendered_end < parsed.lines.len());
}

#[test]
fn code_blocks_capture_mermaid_raw_content() {
    let (ss, theme) = test_assets();
    let src = "```mermaid\ngraph TD;\nA-->B;\n```\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 1);
    let block = &parsed.code_blocks[0];
    assert_eq!(block.raw_content, "graph TD;\nA-->B;\n");
    assert!(block.rendered_end < parsed.lines.len());
}

#[test]
fn code_blocks_capture_latex_raw_content() {
    let (ss, theme) = test_assets();
    let src = "```latex\nE = mc^2\n```\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 1);
    let block = &parsed.code_blocks[0];
    assert_eq!(block.raw_content, "E = mc^2\n");
    assert!(block.rendered_end < parsed.lines.len());
}

#[test]
fn code_blocks_multiple_blocks_are_captured_in_order() {
    let (ss, theme) = test_assets();
    let src = "```rust\nfirst\n```\n\nsome text\n\n```python\nsecond\n```\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 2);
    assert_eq!(parsed.code_blocks[0].raw_content, "first\n");
    assert_eq!(parsed.code_blocks[1].raw_content, "second\n");
    assert!(parsed.code_blocks[0].rendered_end < parsed.code_blocks[1].rendered_start);
}

#[test]
fn code_blocks_raw_content_excludes_fences() {
    let (ss, theme) = test_assets();
    let src = "```\nplain code\n```\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 1);
    let block = &parsed.code_blocks[0];
    assert!(!block.raw_content.contains("```"));
    assert_eq!(block.raw_content, "plain code\n");
}

#[test]
fn code_blocks_capture_display_math_dollar_syntax() {
    let (ss, theme) = test_assets();
    let src = "$$ E = mc^2 $$\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 1);
    let block = &parsed.code_blocks[0];
    assert_eq!(block.raw_content, "E = mc^2");
    assert!(block.rendered_start <= block.rendered_end);
    assert!(block.rendered_end < parsed.lines.len());
}

#[test]
fn code_blocks_ignores_inline_math_dollar_syntax() {
    let (ss, theme) = test_assets();
    let src = "Voici une formule inline $a + b$ dans un paragraphe.\n";
    let parsed = parse_markdown(src, &ss, &theme, &test_md_theme(), false, true);
    assert_eq!(parsed.code_blocks.len(), 0);
}
