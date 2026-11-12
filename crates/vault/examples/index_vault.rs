//! Acceptance check for the vault crate: indexes a *copy* of a real vault
//! (e.g. `cp -r testvault /tmp/vault-copy` — never touch the original)
//! and prints per-note stats plus an ambiguity report.
//!
//! Usage: `cargo run -p vault --example index_vault [-- <vault-dir>]`
//! (defaults to `/tmp/vault-copy`).

use std::path::PathBuf;
use std::process::ExitCode;

use vault::{Resolution, VaultIndex};

fn main() -> ExitCode {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/vault-copy"));

    if !root.is_dir() {
        eprintln!("not a directory: {}", root.display());
        eprintln!("hint: cp -r testvault /tmp/vault-copy  (read-only on the original)");
        return ExitCode::from(2);
    }

    let index = match VaultIndex::build(&root) {
        Ok(index) => index,
        Err(err) => {
            eprintln!("index failed: {err}");
            return ExitCode::from(1);
        }
    };

    let mut with_frontmatter = 0usize;
    let mut with_status = 0usize;
    let mut total_tags = 0usize;
    let mut total_links = 0usize;
    let mut frontmatter_links = 0usize;
    let mut embeds = 0usize;
    let mut resolved = 0usize;
    let mut ambiguous_links = 0usize;
    let mut unresolved_links = 0usize;
    let mut by_type: Vec<(String, usize)> = Vec::new();

    for note in index.notes() {
        let has_frontmatter = note.status.is_some()
            || note.created.is_some()
            || note.updated.is_some()
            || note.archived.is_some()
            || note.note_type.is_some()
            || !note.tags.is_empty();
        if has_frontmatter {
            with_frontmatter += 1;
        }
        if note.status.is_some() {
            with_status += 1;
        }
        total_tags += note.tags.len();
        total_links += note.links.len();
        frontmatter_links += note.links.iter().filter(|l| l.from_frontmatter).count();
        embeds += note.links.iter().filter(|l| l.is_embed).count();
        if let Some(t) = &note.note_type {
            match by_type.iter_mut().find(|(k, _)| k == t) {
                Some(entry) => entry.1 += 1,
                None => by_type.push((t.clone(), 1)),
            }
        }
        println!(
            "{}\tstatus={}\ttype={}\ttags={}\tlinks={}",
            note.rel_path,
            note.status.as_deref().unwrap_or("-"),
            note.note_type.as_deref().unwrap_or("-"),
            note.tags.len(),
            note.links.len()
        );
        for link in &note.links {
            if link.is_embed {
                continue; // embeds are tracked separately, not link stats
            }
            match index.resolve(&link.target) {
                Resolution::Found(_) => resolved += 1,
                Resolution::Ambiguous(_) => ambiguous_links += 1,
                Resolution::NotFound if link.is_self_link() => {} // heading-only: fine
                Resolution::NotFound => unresolved_links += 1,
            }
        }
    }

    println!("\n=== summary ===");
    println!("root: {}", index.root().display());
    println!("notes indexed:        {}", index.len());
    println!("with frontmatter:     {with_frontmatter}");
    println!("with status field:    {with_status}");
    println!("total tags:           {total_tags}");
    println!("total links out:      {total_links}");
    println!("  from frontmatter:   {frontmatter_links}");
    println!("  embeds (![[…]]):    {embeds}");
    println!("link resolution:      {resolved} resolved, {ambiguous_links} ambiguous, {unresolved_links} unresolved (self-links and embeds excluded)");
    println!("warnings:             {}", index.warnings().len());
    for warning in index.warnings().iter().take(10) {
        println!("  warn {}:{}", warning.path.display(), warning.message);
    }
    if index.warnings().len() > 10 {
        println!("  … and {} more", index.warnings().len() - 10);
    }

    println!("\n=== note types ===");
    by_type.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (kind, count) in &by_type {
        println!("{kind}: {count}");
    }

    println!("\n=== ambiguous basenames ===");
    let ambiguous = index.ambiguous_basenames();
    if ambiguous.is_empty() {
        println!("(none)");
    }
    for (stem, paths) in &ambiguous {
        println!("{stem} ({} candidates):", paths.len());
        for path in *paths {
            println!("  {}", path.display());
        }
    }

    println!("\n=== top tags ===");
    let mut tags = index.tag_counts();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (tag, count) in tags.iter().take(10) {
        println!("{tag}: {count}");
    }

    ExitCode::SUCCESS
}
