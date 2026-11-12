//! Vault model for the frond: read-only indexing of an
//! Obsidian-style markdown vault.
//!
//! Layers:
//! - [`walk`] — vault-root discovery of `.md` notes (hidden entries and
//!   directory symlinks skipped, depth-capped).
//! - [`frontmatter`] — tolerant YAML-frontmatter parsing (`status`,
//!   `tags`, `created`, `updated`, `archived`, `type`); wikilinks inside
//!   frontmatter values/list items are extracted as outgoing links too
//!   (`LinkRef::from_frontmatter`).
//! - [`wikilink`] — `[[target]]` / `[[target|alias]]` / `[[#heading]]` /
//!   `[[dir/name]]` extraction (code blocks skipped; `\|` escaped
//!   separators handled; `![[…]]` embeds tagged via
//!   `LinkRef::is_embed`).
//! - [`index`] — [`VaultIndex`]: per-note metadata, basename resolver with
//!   ambiguity detection, backlink map, tag map, cached raw-line text with
//!   a case-insensitive substring [`VaultIndex::search`] (capped at
//!   [`index::SEARCH_RESULT_CAP`] hits), and watch-driven
//!   single-file refresh via [`VaultIndex::reindex_one`].
//! - [`base`] — `.base` table files: a hand-rolled parser for the YAML
//!   subset the vault's base files use (filters, properties, formulas,
//!   table views), fail-open filter evaluation against the index, and
//!   the sorted/grouped table model the app renders (the app re-reads
//!   the open base file on its own stamp — base files are not index
//!   entries).
//! - [`git`] — read-only sync status (`status --porcelain -uall`,
//!   `rev-list` ahead/behind vs the upstream ref — `@{upstream}` with an
//!   `origin/main` fallback) behind a strict read-only
//!   subcommand allowlist; every failure degrades to `None`.
//! - [`tasks`] — task-marker (checkbox) scanning and the one guarded
//!   write feature: a toggle replaces only the marker state byte on the
//!   exact source line (everything else byte-preserved), bumps the
//!   frontmatter `updated:` field when it exists, and replaces the file
//!   atomically (temp + rename). Precondition guards (clean/synced git
//!   tree, un-raced mtime) live in the app layer.
//!
//! The crate is a pure library with zero dependencies; rendering and
//! interaction live in `crates/app`. Reads are unconditional; the only
//! write path is the guarded checkbox toggle above.

pub mod base;
pub mod frontmatter;
pub mod git;
pub mod index;
pub mod tasks;
pub mod walk;
pub mod wikilink;

#[cfg(test)]
mod testutil;

pub use git::{read_status as read_git_status, repo_toplevel, GitStatus};
pub use index::SEARCH_RESULT_CAP;
pub use index::{IndexWarning, NoteMeta, Resolution, SearchHit, SearchResults, VaultIndex};
pub use tasks::{TaskHit, TaskState};
pub use wikilink::LinkRef;
