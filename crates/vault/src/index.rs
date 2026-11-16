//! The vault index: notes, frontmatter, wikilinks, resolver, backlinks,
//! tag index, and single-file incremental refresh.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::frontmatter::{parse_frontmatter_with_raw, split_frontmatter};
use crate::walk::{is_base_path, is_indexable, walk_vault};
use crate::wikilink::{extract_wikilinks, LinkRef};

/// One raw-text match of a global vault search ([`VaultIndex::search`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// Absolute path of the note containing the match.
    pub path: PathBuf,
    /// 0-based raw line index in the file (display +1 for grep-style
    /// line numbers).
    pub line: usize,
    /// The raw line text, without its line terminator.
    pub text: String,
    /// Byte range of the (first) match within `text`, adjusted to char
    /// boundaries (case folding can shift offsets for exotic inputs).
    pub start: usize,
    pub end: usize,
}

/// Maximum number of hits [`VaultIndex::search`] returns. The scan stops
/// early once the cap is exceeded — a vault-wide one-letter query can
/// otherwise allocate tens of thousands of hits (~10 MB measured on a
/// 478-note vault) that the results popup can never show at once.
pub const SEARCH_RESULT_CAP: usize = 1000;

/// The outcome of a global vault search: the (capped) hits plus whether
/// more matches exist than were returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    /// Up to [`SEARCH_RESULT_CAP`] hits, in path order then line order.
    pub hits: Vec<SearchHit>,
    /// `true` when the scan found more than [`SEARCH_RESULT_CAP`]
    /// matches — the returned `hits` are only the first [`SEARCH_RESULT_CAP`].
    pub capped: bool,
}

/// Case-insensitive `ends_with` (ASCII folding, matching the
/// `eq_ignore_ascii_case` semantics used elsewhere here).
fn ends_with_ci(haystack: &str, needle: &str) -> bool {
    haystack.len() >= needle.len()
        && haystack[haystack.len() - needle.len()..].eq_ignore_ascii_case(needle)
}

/// A note as indexed: its path, frontmatter fields, and outgoing links.
/// Missing or invalid frontmatter leaves every field empty — the note is
/// still indexed and the reason is reported as an [`IndexWarning`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMeta {
    /// Absolute path under the vault root (as walked).
    pub path: PathBuf,
    /// Root-relative path with `/` separators (what path-style links
    /// `[[dir/name]]` match against).
    pub rel_path: String,
    /// File name without the `.md` suffix (what bare links match).
    pub stem: String,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub archived: Option<String>,
    /// Frontmatter `type:` field (vault convention: analysis, rca, todo…).
    pub note_type: Option<String>,
    /// Generic map of raw scalar frontmatter values (comment-stripped,
    /// unquoted, non-empty), keyed by frontmatter key — the `.base`
    /// table renderer filters/sorts/groups against these. Additive to
    /// the typed extracts above, which remain the source of truth for
    /// their own fields.
    pub raw: BTreeMap<String, String>,
    /// Outgoing wikilinks in document order.
    pub links: Vec<LinkRef>,
}

/// Something worth surfacing about a note that could not be indexed
/// cleanly (no frontmatter, unterminated fence, malformed line,
/// unreadable file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexWarning {
    pub path: PathBuf,
    pub message: String,
}

/// Result of resolving a wikilink target against the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one note matches.
    Found(PathBuf),
    /// Several notes share this basename — the collision is returned, not
    /// guessed at (the app renders a warning marker / popup in phase 2).
    Ambiguous(Vec<PathBuf>),
    /// No note matches.
    NotFound,
}

/// The whole vault: per-note metadata plus the derived maps.
///
/// Derived state (basename map, backlinks, tags) is rebuilt after every
/// mutation, so it is always consistent with `notes`. All maps are
/// `BTreeMap`s for deterministic iteration.
pub struct VaultIndex {
    root: PathBuf,
    notes: BTreeMap<PathBuf, NoteMeta>,
    /// `.base` table files, path-only (never parsed here; the base view
    /// reads them on demand). Exists so wikilink targets ending in
    /// `.base` can resolve — see [`VaultIndex::resolve`].
    bases: BTreeSet<PathBuf>,
    warnings: Vec<IndexWarning>,
    /// Lowercased note stem -> paths sharing it (len > 1 means
    /// ambiguous; resolution is case-insensitive).
    by_stem: BTreeMap<String, Vec<PathBuf>>,
    /// Lowercased `.base` stem (name without the suffix) -> paths sharing
    /// it, same ambiguity semantics as `by_stem`.
    by_base_stem: BTreeMap<String, Vec<PathBuf>>,
    /// Note path -> notes linking to it. Built by resolving each link;
    /// ambiguous targets contribute to every candidate.
    backlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Tag -> notes carrying it in frontmatter (frontmatter tags only —
    /// the vault has no inline tags).
    tags: BTreeMap<String, Vec<PathBuf>>,
    /// Cached raw text of every indexed note (one `Vec` of lines per
    /// path), kept in sync by `insert_note`/`remove_note` — the global
    /// search runs against this cache, never against the disk.
    raw_lines: BTreeMap<PathBuf, Vec<String>>,
}

impl VaultIndex {
    /// Walks `root` and indexes every `.md` note; `.base` files are
    /// collected path-only (never parsed). Fails only if `root` itself
    /// cannot be read; per-file problems become warnings.
    pub fn build(root: &Path) -> io::Result<Self> {
        let mut index = Self {
            root: root.to_path_buf(),
            notes: BTreeMap::new(),
            bases: BTreeSet::new(),
            warnings: Vec::new(),
            by_stem: BTreeMap::new(),
            by_base_stem: BTreeMap::new(),
            backlinks: BTreeMap::new(),
            tags: BTreeMap::new(),
            raw_lines: BTreeMap::new(),
        };
        for entry in walk_vault(root)? {
            if is_base_path(&entry.path) {
                index.bases.insert(entry.path);
                continue;
            }
            match fs::read_to_string(&entry.path) {
                Ok(source) => index.insert_note(&entry.path, &source),
                Err(err) => index.warnings.push(IndexWarning {
                    path: entry.path,
                    message: format!("unreadable: {err}"),
                }),
            }
        }
        index.rebuild_derived();
        Ok(index)
    }

    /// The vault root this index was built from.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All indexed notes, in path order.
    pub fn notes(&self) -> impl Iterator<Item = &NoteMeta> {
        self.notes.values()
    }

    /// All known `.base` files, in path order (path-only entries; the
    /// base view reads their content on demand).
    pub fn bases(&self) -> impl Iterator<Item = &Path> {
        self.bases.iter().map(|p| p.as_path())
    }

    /// Number of indexed notes.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Metadata for one note, by absolute path.
    pub fn note(&self, path: &Path) -> Option<&NoteMeta> {
        self.notes.get(path)
    }

    /// All warnings, in walk order.
    pub fn warnings(&self) -> &[IndexWarning] {
        &self.warnings
    }

    /// Basenames shared by more than one note: `(stem, candidate paths)`.
    /// Resolution is case-insensitive, so the reported stem is the
    /// (original-case) stem of the first candidate path.
    pub fn ambiguous_basenames(&self) -> Vec<(&str, &[PathBuf])> {
        self.by_stem
            .iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(lower, paths)| {
                let stem = self
                    .notes
                    .get(&paths[0])
                    .map(|n| n.stem.as_str())
                    .unwrap_or(lower.as_str());
                (stem, paths.as_slice())
            })
            .collect()
    }

    /// Notes linking *to* `path` (source order is path order).
    pub fn backlinks_to(&self, path: &Path) -> &[PathBuf] {
        self.backlinks
            .get(path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Outgoing links of one note.
    pub fn links_from(&self, path: &Path) -> &[LinkRef] {
        static EMPTY: [LinkRef; 0] = [];
        self.notes
            .get(path)
            .map(|n| n.links.as_slice())
            .unwrap_or(&EMPTY)
    }

    /// Notes carrying `tag` in their frontmatter tags list.
    pub fn notes_with_tag(&self, tag: &str) -> Vec<&NoteMeta> {
        self.tags
            .get(tag)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|p| self.notes.get(p))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// All tags in use, sorted, with their note counts.
    pub fn tag_counts(&self) -> Vec<(&str, usize)> {
        self.tags
            .iter()
            .map(|(tag, paths)| (tag.as_str(), paths.len()))
            .collect()
    }

    /// Case-insensitive substring search over every indexed note's raw
    /// text. One hit per matching line, in path order then line order.
    /// Runs against the cached raw lines kept in sync on every (re)index
    /// of a note — no disk reads and no subprocess at search time (484
    /// small notes are trivial to scan in memory).
    ///
    /// The result is capped at [`SEARCH_RESULT_CAP`] hits: the scan stops
    /// as soon as the cap is passed (one extra match is detected to set
    /// [`SearchResults::capped`] exactly — a vault with exactly the cap
    /// many matches is *not* reported as capped), and
    /// [`SearchResults::capped`] tells the caller to display "N+".
    pub fn search(&self, query: &str) -> SearchResults {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return SearchResults::default();
        }
        let mut hits: Vec<SearchHit> = Vec::new();
        for (path, lines) in &self.raw_lines {
            for (index, line) in lines.iter().enumerate() {
                // Offsets come from the folded string; case folding can
                // change byte lengths (e.g. `İ`), so the range is snapped
                // to char boundaries of the original line afterwards.
                let folded = line.to_lowercase();
                let Some(pos) = folded.find(&needle) else {
                    continue;
                };
                let start = floor_char_boundary(line, pos);
                let end = ceil_char_boundary(line, pos + needle.len());
                hits.push(SearchHit {
                    path: path.clone(),
                    line: index,
                    text: line.clone(),
                    start,
                    end,
                });
                if hits.len() > SEARCH_RESULT_CAP {
                    // One match past the cap: truncate to exactly the cap
                    // and mark the result as truncated.
                    hits.truncate(SEARCH_RESULT_CAP);
                    return SearchResults { hits, capped: true };
                }
            }
        }
        SearchResults {
            hits,
            capped: false,
        }
    }

    /// Resolves a wikilink target per the vault rule: bare filename
    /// first, path fallback. `target` may carry a trailing `.md` (it is
    /// stripped). A target ending in `.base` (case-insensitive) resolves
    /// only against the vault's `.base` files — a bare stem without the
    /// suffix never matches a base, matching Obsidian, where a plain
    /// `[[name]]` cannot land on a base. Path-style targets
    /// (`dir/name`) match against root-relative note paths; matching is
    /// case-insensitive for both
    /// stems and paths, and a path target that no longer matches fully
    /// (e.g. after the note moved) falls back to Obsidian-style suffix
    /// matching: a unique suffix match resolves, several are ambiguous,
    /// and nothing is ever guessed.
    pub fn resolve(&self, target: &str) -> Resolution {
        if ends_with_ci(target, ".base") {
            return self.resolve_base(target);
        }
        let target = target
            .strip_suffix(".md")
            .unwrap_or(target)
            .trim()
            .to_string();
        if target.is_empty() {
            return Resolution::NotFound;
        }
        if target.contains('/') {
            // Path-style: match against relative note paths, tolerating a
            // missing `.md` on either side and case differences.
            let matches: Vec<PathBuf> = self
                .notes
                .values()
                .filter(|n| {
                    n.rel_path
                        .strip_suffix(".md")
                        .unwrap_or(&n.rel_path)
                        .eq_ignore_ascii_case(&target)
                })
                .map(|n| n.path.clone())
                .collect();
            return match matches.len() {
                0 => self.resolve_by_suffix(&target),
                1 => Resolution::Found(matches[0].clone()),
                _ => Resolution::Ambiguous(matches),
            };
        }
        match self.by_stem.get(&target.to_ascii_lowercase()) {
            None => Resolution::NotFound,
            Some(paths) if paths.len() == 1 => Resolution::Found(paths[0].clone()),
            Some(paths) => Resolution::Ambiguous(paths.clone()),
        }
    }

    /// Resolves an explicit `.base` target against the vault's base
    /// files only: bare stem (`inventories.base`) via the basename map,
    /// path-style (`dir/inventories.base`) against root-relative base
    /// paths — both case-insensitive, both suffix-stripped on either
    /// side. No suffix fallback (unlike notes): a moved base is simply
    /// not found rather than guessed at.
    fn resolve_base(&self, target: &str) -> Resolution {
        let target = if ends_with_ci(target, ".base") {
            &target[..target.len() - ".base".len()]
        } else {
            target
        }
        .trim()
        .to_string();
        if target.is_empty() {
            return Resolution::NotFound;
        }
        if target.contains('/') {
            let matches: Vec<PathBuf> = self
                .bases
                .iter()
                .filter(|p| {
                    let rel = p
                        .strip_prefix(&self.root)
                        .unwrap_or(p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    rel.strip_suffix(".base")
                        .unwrap_or(&rel)
                        .eq_ignore_ascii_case(&target)
                })
                .cloned()
                .collect();
            return match matches.len() {
                0 => Resolution::NotFound,
                1 => Resolution::Found(matches[0].clone()),
                _ => Resolution::Ambiguous(matches),
            };
        }
        match self.by_base_stem.get(&target.to_ascii_lowercase()) {
            None => Resolution::NotFound,
            Some(paths) if paths.len() == 1 => Resolution::Found(paths[0].clone()),
            Some(paths) => Resolution::Ambiguous(paths.clone()),
        }
    }

    /// Resolves `target` with a proximity hint, matching Obsidian's
    /// behavior when a bare link is ambiguous: plain [`Self::resolve`]
    /// first; if that is [`Resolution::Ambiguous`], prefer the candidate
    /// in the same folder as `source`, then the candidate in the nearest
    /// ancestor folder of `source`. Only when no candidate lives in the
    /// source's folder or any of its ancestors does this return
    /// [`Resolution::Ambiguous`] (the app then shows a disambiguation
    /// popup instead of guessing). [`Resolution::Found`] and
    /// [`Resolution::NotFound`] pass through untouched.
    ///
    /// Pure: reads nothing from disk, only the in-memory index.
    pub fn resolve_with_hint(&self, source: &Path, target: &str) -> Resolution {
        match self.resolve(target) {
            Resolution::Ambiguous(candidates) => disambiguate_by_proximity(source, candidates),
            resolved => resolved,
        }
    }

    /// Obsidian-style suffix-match fallback for path targets: collects
    /// notes whose vault-relative path ends with `/target` (so the match
    /// is always on a directory boundary). Unique → Found, several →
    /// Ambiguous, none → NotFound — never a guess.
    fn resolve_by_suffix(&self, target: &str) -> Resolution {
        let needle = format!("/{}", target);
        let matches: Vec<PathBuf> = self
            .notes
            .values()
            .filter(|n| {
                let rel = n.rel_path.strip_suffix(".md").unwrap_or(&n.rel_path);
                ends_with_ci(rel, &needle)
            })
            .map(|n| n.path.clone())
            .collect();
        match matches.len() {
            0 => Resolution::NotFound,
            1 => Resolution::Found(matches[0].clone()),
            _ => Resolution::Ambiguous(matches),
        }
    }

    /// Watch-driven refresh for one file: add, modify, or remove.
    ///
    /// `.base` paths are tracked path-only: a present, in-vault base file
    /// is listed, anything else (deleted, moved out) is dropped — the
    /// index never parses base content. A missing/unreadable/non-markdown
    /// note path removes the note (if indexed) or does nothing; only an
    /// existing readable `.md` under the root is (re)parsed. Derived maps
    /// are rebuilt afterwards, so the index is never observed
    /// half-updated.
    pub fn reindex_one(&mut self, path: &Path) {
        if is_base_path(path) {
            let listed = path.exists() && path.strip_prefix(&self.root).is_ok();
            let was_listed = self.bases.contains(path);
            if listed {
                self.bases.insert(path.to_path_buf());
            } else {
                self.bases.remove(path);
            }
            if listed != was_listed {
                self.rebuild_derived();
            }
            return;
        }
        if !is_indexable(&self.root, path) {
            self.remove_note(path);
            return;
        }
        match fs::read_to_string(path) {
            Ok(source) => {
                self.insert_note(path, &source);
                self.rebuild_derived();
            }
            Err(err) => {
                self.remove_note(path);
                // Unreadable-but-present is a warning, not a silent drop.
                if path.exists() {
                    self.warnings.retain(|w| w.path != path);
                    self.warnings.push(IndexWarning {
                        path: path.to_path_buf(),
                        message: format!("unreadable: {err}"),
                    });
                }
            }
        }
    }

    /// Inserts or replaces one note's metadata from its source. Does not
    /// touch derived maps (callers rebuild them).
    fn insert_note(&mut self, path: &Path, source: &str) {
        self.warnings.retain(|w| w.path != path);
        let (fm_source, body) = split_frontmatter(source);
        let (frontmatter, raw, mut fm_links, warnings) = match fm_source {
            Some(fm) => parse_frontmatter_with_raw(fm),
            None => {
                let mut warnings = Vec::new();
                if source.trim_start().starts_with("---") {
                    warnings.push("frontmatter fence never closed".to_string());
                } else {
                    warnings.push("no frontmatter".to_string());
                }
                (Default::default(), BTreeMap::new(), Vec::new(), warnings)
            }
        };
        for message in warnings {
            self.warnings.push(IndexWarning {
                path: path.to_path_buf(),
                message,
            });
        }
        let rel_path = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        // Document order: frontmatter links come first (they appear
        // first in the file), then body links.
        fm_links.extend(extract_wikilinks(body));
        self.raw_lines.insert(
            path.to_path_buf(),
            source.lines().map(|l| l.to_string()).collect(),
        );
        self.notes.insert(
            path.to_path_buf(),
            NoteMeta {
                path: path.to_path_buf(),
                rel_path,
                stem,
                status: frontmatter.status,
                tags: frontmatter.tags,
                created: frontmatter.created,
                updated: frontmatter.updated,
                archived: frontmatter.archived,
                note_type: frontmatter.note_type,
                raw,
                links: fm_links,
            },
        );
    }

    /// Removes a note and its warnings (used by `reindex_one` on delete).
    fn remove_note(&mut self, path: &Path) {
        let had_lines = self.raw_lines.remove(path).is_some();
        if self.notes.remove(path).is_some()
            || had_lines
            || self.warnings.iter().any(|w| w.path == path)
        {
            self.warnings.retain(|w| w.path != path);
            self.rebuild_derived();
        }
    }

    /// Rebuilds the basename map, backlink map, and tag map from `notes`
    /// (plus the base-stem map from `bases`).
    fn rebuild_derived(&mut self) {
        self.by_stem.clear();
        for note in self.notes.values() {
            self.by_stem
                .entry(note.stem.to_ascii_lowercase())
                .or_default()
                .push(note.path.clone());
        }

        self.by_base_stem.clear();
        for base in &self.bases {
            let stem = base
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            self.by_base_stem
                .entry(stem)
                .or_default()
                .push(base.clone());
        }

        self.backlinks.clear();
        for note in self.notes.values() {
            for link in &note.links {
                if link.is_self_link() {
                    continue; // [[#heading]] never produces backlinks
                }
                match self.resolve(&link.target) {
                    Resolution::Found(target) => self
                        .backlinks
                        .entry(target)
                        .or_default()
                        .push(note.path.clone()),
                    // Ambiguous basename: every candidate plausibly
                    // refers here; phase 2's popup picks the real one.
                    Resolution::Ambiguous(candidates) => {
                        for target in candidates {
                            self.backlinks
                                .entry(target)
                                .or_default()
                                .push(note.path.clone());
                        }
                    }
                    Resolution::NotFound => {}
                }
            }
        }
        for sources in self.backlinks.values_mut() {
            sources.sort();
            sources.dedup();
        }

        self.tags.clear();
        for note in self.notes.values() {
            for tag in &note.tags {
                self.tags
                    .entry(tag.clone())
                    .or_default()
                    .push(note.path.clone());
            }
        }
    }
}

/// Picks the ambiguous candidate closest to `source`: same folder first,
/// then the nearest ancestor folder of `source`. Candidates that are not
/// located inside the source's directory chain don't constrain the hint;
/// if none qualifies (or two qualify equally — which cannot happen for
/// real ambiguities, but is guarded anyway), the ambiguity is returned
/// rather than guessed at.
fn disambiguate_by_proximity(source: &Path, candidates: Vec<PathBuf>) -> Resolution {
    let Some(source_dir) = source.parent() else {
        return Resolution::Ambiguous(candidates);
    };
    let source_components: Vec<_> = source_dir.components().collect();
    let mut best: Option<(usize, &PathBuf)> = None;
    for candidate in &candidates {
        let Some(candidate_dir) = candidate.parent() else {
            continue;
        };
        let candidate_components: Vec<_> = candidate_dir.components().collect();
        if candidate_components.len() > source_components.len()
            || candidate_components[..] != source_components[..candidate_components.len()]
        {
            continue; // not an ancestor of the source note's folder
        }
        let distance = source_components.len() - candidate_components.len();
        match best {
            None => best = Some((distance, candidate)),
            Some((best_distance, _)) if distance < best_distance => {
                best = Some((distance, candidate));
            }
            Some((best_distance, _)) if distance == best_distance => {
                // Two candidates equally near: refuse to guess.
                return Resolution::Ambiguous(candidates);
            }
            _ => {}
        }
    }
    match best {
        Some((_, candidate)) => Resolution::Found(candidate.clone()),
        None => Resolution::Ambiguous(candidates),
    }
}

/// Largest index `<= i` that is a char boundary of `s`.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest index `>= i` that is a char boundary of `s` (clamped to len).
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    let len = s.len();
    while i < len && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn note_at<'a>(index: &'a VaultIndex, rel: &str) -> &'a NoteMeta {
        let path = index.root().join(rel);
        index
            .note(&path)
            .unwrap_or_else(|| panic!("note not indexed: {rel}"))
    }

    // --- M4: global search --------------------------------------------

    #[test]
    fn search_finds_substrings_case_insensitively_in_path_order() {
        let vault = TempDir::new("vk-search");
        write(&vault.path().join("b.md"), "first\nhas Needle here\n");
        write(&vault.path().join("a.md"), "needle at top\nplain\n");
        write(&vault.path().join("sub/c.md"), "nothing here\n");
        let index = VaultIndex::build(vault.path()).unwrap();

        let hits = index.search("needle").hits;
        assert_eq!(hits.len(), 2, "one hit per matching line: {hits:?}");
        // Path order: a.md before b.md.
        assert_eq!(hits[0].path, vault.path().join("a.md"));
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[0].text, "needle at top");
        assert_eq!(hits[1].path, vault.path().join("b.md"));
        assert_eq!(hits[1].line, 1);
        assert_eq!(hits[1].text, "has Needle here");
        // Byte range covers the match inside the raw line.
        assert_eq!(&hits[1].text[hits[1].start..hits[1].end], "Needle");
    }

    #[test]
    fn search_covers_frontmatter_and_updates_on_reindex() {
        let vault = TempDir::new("vk-search-fm");
        write(
            &vault.path().join("a.md"),
            "---\nstatus: active\ntags: [mocktag]\n---\nbody text\n",
        );
        let mut index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            index.search("mocktag").hits.len(),
            1,
            "frontmatter is searchable"
        );

        // Reindex with the needle removed: the cached lines are refreshed.
        write(
            &vault.path().join("a.md"),
            "---\nstatus: active\n---\nbody\n",
        );
        index.reindex_one(&vault.path().join("a.md"));
        assert!(
            index.search("mocktag").hits.is_empty(),
            "reindex refreshed the cached raw lines"
        );

        // Removal drops the note from the search space entirely.
        std::fs::remove_file(vault.path().join("a.md")).unwrap();
        index.reindex_one(&vault.path().join("a.md"));
        assert!(index.search("body").hits.is_empty());
    }

    #[test]
    fn search_empty_query_and_no_match() {
        let vault = TempDir::new("vk-search-empty");
        write(&vault.path().join("a.md"), "alpha\nbeta\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        assert!(index.search("").hits.is_empty());
        assert!(index.search("gamma").hits.is_empty());
    }

    #[test]
    fn search_caps_results_and_reports_capping() {
        let vault = TempDir::new("vk-search-cap");
        let body: String = (0..SEARCH_RESULT_CAP + 100).map(|_| "hit line\n").collect();
        write(&vault.path().join("big.md"), &body);
        let index = VaultIndex::build(vault.path()).unwrap();
        let results = index.search("hit");
        assert_eq!(results.hits.len(), SEARCH_RESULT_CAP, "capped at 1000");
        assert!(results.capped, "more hits existed than the cap");
        assert_eq!(results.hits[0].line, 0);
        assert_eq!(
            results.hits[SEARCH_RESULT_CAP - 1].line,
            SEARCH_RESULT_CAP - 1,
            "the FIRST hits are kept (path/line order, earliest wins)"
        );
    }

    #[test]
    fn search_at_exactly_the_cap_is_not_reported_capped() {
        let vault = TempDir::new("vk-search-cap-exact");
        let body: String = (0..SEARCH_RESULT_CAP).map(|_| "hit line\n").collect();
        write(&vault.path().join("big.md"), &body);
        let index = VaultIndex::build(vault.path()).unwrap();
        let results = index.search("hit");
        assert_eq!(results.hits.len(), SEARCH_RESULT_CAP);
        assert!(!results.capped, "exactly the cap: nothing truncated");
    }

    #[test]
    fn search_unicode_folding_hits_char_boundaries() {
        let vault = TempDir::new("vk-search-uni");
        write(&vault.path().join("u.md"), "Grüße aus München\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let hits = index.search("MÜNCHEN").hits;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start, 12, "byte offset of the match");
        assert_eq!(&hits[0].text[hits[0].start..hits[0].end], "München");
        // Slicing stays panic-free even for exotic folding shifts.
        let _ = &hits[0].text[..hits[0].end];
    }

    #[test]
    fn indexes_nested_dirs_with_frontmatter_and_links() {
        let vault = TempDir::new("vk-index");
        write(
            &vault.path().join("projects/analysis.md"),
            "---\nstatus: complete\ntype: analysis\ntags: [mocktag, disk]\ncreated: 2026-06-03\nupdated: 2026-10-23\n---\nBody [[todo]] and [[archive/index|Archive]].\n",
        );
        write(
            &vault.path().join("projects/todo.md"),
            "no frontmatter here\n",
        );
        write(
            &vault.path().join("archive/index.md"),
            "---\nstatus: archived\n---\n",
        );

        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(index.len(), 3);

        let analysis = note_at(&index, "projects/analysis.md");
        assert_eq!(analysis.status.as_deref(), Some("complete"));
        assert_eq!(analysis.note_type.as_deref(), Some("analysis"));
        assert_eq!(analysis.tags, vec!["mocktag", "disk"]);
        assert_eq!(analysis.created.as_deref(), Some("2026-06-03"));
        assert_eq!(analysis.links.len(), 2);
        assert_eq!(analysis.links[0].target, "todo");
        assert_eq!(analysis.links[1].target, "archive/index");
        assert_eq!(analysis.links[1].alias.as_deref(), Some("Archive"));

        // The frontmatter-less note is indexed with empty metadata + a warning.
        let todo = note_at(&index, "projects/todo.md");
        assert!(todo.status.is_none() && todo.tags.is_empty());
        assert!(index
            .warnings()
            .iter()
            .any(|w| w.path.ends_with("projects/todo.md") && w.message == "no frontmatter"));

        // Backlinks: todo ← analysis; archive/index ← analysis (path link).
        let todo_path = vault.path().join("projects/todo.md");
        assert_eq!(
            index.backlinks_to(&todo_path),
            vec![vault.path().join("projects/analysis.md")]
        );
        let archive_path = vault.path().join("archive/index.md");
        assert_eq!(
            index.backlinks_to(&archive_path),
            vec![vault.path().join("projects/analysis.md")]
        );

        // Tag index: frontmatter tags only.
        let mocktag: Vec<&NoteMeta> = index.notes_with_tag("mocktag");
        assert_eq!(mocktag.len(), 1);
        assert_eq!(mocktag[0].rel_path, "projects/analysis.md");
        assert!(index.notes_with_tag("missing").is_empty());
    }

    #[test]
    fn resolve_bare_ambiguous_and_path_forms() {
        let vault = TempDir::new("vk-resolve");
        write(&vault.path().join("a/dup.md"), "x");
        write(&vault.path().join("b/dup.md"), "x");
        write(&vault.path().join("unique.md"), "x");

        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            index.resolve("unique"),
            Resolution::Found(vault.path().join("unique.md"))
        );
        assert_eq!(
            index.resolve("unique.md"),
            index.resolve("unique"),
            ".md suffix tolerated"
        );
        let mut candidates = match index.resolve("dup") {
            Resolution::Ambiguous(c) => c,
            other => panic!("expected ambiguity, got {other:?}"),
        };
        candidates.sort();
        assert_eq!(
            candidates,
            vec![vault.path().join("a/dup.md"), vault.path().join("b/dup.md")]
        );
        assert_eq!(
            index.resolve("b/dup"),
            Resolution::Found(vault.path().join("b/dup.md")),
            "path-style link disambiguates"
        );
        assert_eq!(index.resolve("nonexistent"), Resolution::NotFound);
        assert_eq!(index.resolve(""), Resolution::NotFound);

        let ambiguous = index.ambiguous_basenames();
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].0, "dup");
        assert_eq!(ambiguous[0].1.len(), 2);
    }

    #[test]
    fn resolve_base_targets_explicit_suffix_only() {
        let vault = TempDir::new("vk-resolve-base");
        write(&vault.path().join("inventories.base"), "filters: []\n");
        write(&vault.path().join("a/board.base"), "filters: []\n");
        // A note whose stem collides with a base's stem: bare `[[board]]`
        // must still resolve to the note, never the base.
        write(&vault.path().join("board.md"), "x\n");

        let index = VaultIndex::build(vault.path()).unwrap();
        // Bases are path-only: not counted as notes, never parsed.
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.bases().collect::<Vec<_>>(),
            vec![
                vault.path().join("a/board.base"),
                vault.path().join("inventories.base")
            ]
        );

        // Bare stem with the explicit `.base` suffix (case-insensitive).
        assert_eq!(
            index.resolve("inventories.base"),
            Resolution::Found(vault.path().join("inventories.base"))
        );
        assert_eq!(
            index.resolve("Inventories.BASE"),
            Resolution::Found(vault.path().join("inventories.base"))
        );
        // Path-style base target.
        assert_eq!(
            index.resolve("a/board.base"),
            Resolution::Found(vault.path().join("a/board.base"))
        );
        // Bare stem without the suffix never lands on a base.
        assert_eq!(
            index.resolve("board"),
            Resolution::Found(vault.path().join("board.md"))
        );
        assert_eq!(index.resolve("inventories"), Resolution::NotFound);
        assert_eq!(index.resolve("no/such.base"), Resolution::NotFound);

        // Ambiguous bare base stems surface as ambiguous, not guessed.
        write(&vault.path().join("b/dup.base"), "");
        write(&vault.path().join("c/dup.base"), "");
        let index = VaultIndex::build(vault.path()).unwrap();
        let mut candidates = match index.resolve("dup.base") {
            Resolution::Ambiguous(c) => c,
            other => panic!("expected ambiguity, got {other:?}"),
        };
        candidates.sort();
        assert_eq!(
            candidates,
            vec![
                vault.path().join("b/dup.base"),
                vault.path().join("c/dup.base")
            ]
        );
    }

    #[test]
    fn reindex_one_tracks_base_add_and_remove() {
        let vault = TempDir::new("vk-reindex-base");
        write(&vault.path().join("a.md"), "---\nstatus: active\n---\na\n");
        let mut index = VaultIndex::build(vault.path()).unwrap();
        assert!(index.bases().next().is_none());

        // ADD: a new base file appears (watch path calls reindex_one).
        let base = vault.path().join("tasks.base");
        write(&base, "filters: []\n");
        index.reindex_one(&base);
        assert_eq!(index.bases().collect::<Vec<_>>(), vec![base.clone()]);
        assert_eq!(index.len(), 1, "a base is not a note");

        // MODIFY: content change keeps the listing, adds no warnings.
        write(&base, "filters:\n  and: []\n");
        index.reindex_one(&base);
        assert_eq!(index.bases().count(), 1);
        assert!(index.warnings().is_empty());

        // REMOVE: the base drops out of the resolver.
        std::fs::remove_file(&base).unwrap();
        index.reindex_one(&base);
        assert!(index.bases().next().is_none());

        // A renamed-away base (path now `.md`) leaves the base set.
        write(&base, "filters: []\n");
        index.reindex_one(&base);
        assert_eq!(index.bases().count(), 1);
        index.reindex_one(&vault.path().join("tasks.md")); // not present on disk
        assert_eq!(index.bases().count(), 1, "md path never touches bases");
    }

    #[test]
    fn base_wikilinks_register_backlinks() {
        let vault = TempDir::new("vk-base-backlinks");
        write(&vault.path().join("tasks.base"), "filters: []\n");
        write(
            &vault.path().join("dash.md"),
            "Open [[tasks.base]] for the table.\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            index.backlinks_to(&vault.path().join("tasks.base")),
            vec![vault.path().join("dash.md")]
        );
    }

    #[test]
    fn self_heading_links_do_not_create_backlinks() {
        let vault = TempDir::new("vk-self");
        write(
            &vault.path().join("note.md"),
            "See [[#Summary]] and [[note#Detail]].\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();
        let path = vault.path().join("note.md");
        // [[#Summary]] is a self link (no backlink); [[note#Detail]] resolves
        // to the same note, so the note does link to itself once.
        assert_eq!(index.backlinks_to(&path), vec![path.clone()]);
    }

    #[test]
    fn reindex_add_modify_remove() {
        let vault = TempDir::new("vk-reindex");
        write(
            &vault.path().join("a.md"),
            "---\nstatus: active\n---\n[[b]]\n",
        );
        write(&vault.path().join("b.md"), "target\n");
        let mut index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(index.len(), 2);

        // ADD: a new note linking to a.md creates a backlink on a.md.
        let c = vault.path().join("c.md");
        write(&c, "---\nstatus: draft\n---\n[[a]]\n");
        index.reindex_one(&c);
        assert_eq!(index.len(), 3);
        assert_eq!(
            index.backlinks_to(&vault.path().join("a.md")),
            vec![c.clone()]
        );

        // MODIFY: rewriting b.md changes its links (and backlinks).
        let b = vault.path().join("b.md");
        write(&b, "[[a]] now\n");
        index.reindex_one(&b);
        assert_eq!(
            index.backlinks_to(&vault.path().join("a.md")),
            vec![b.clone(), c.clone()],
            "backlinks updated after modify, path order"
        );
        assert_eq!(
            index.note(&b).unwrap().links[0].target,
            "a",
            "note metadata refreshed"
        );

        // REMOVE: deleting c.md drops it and its backlinks.
        fs::remove_file(&c).unwrap();
        index.reindex_one(&c);
        assert_eq!(index.len(), 2);
        assert!(index.note(&c).is_none());
        assert_eq!(
            index.backlinks_to(&vault.path().join("a.md")),
            vec![b.clone()]
        );

        // Remove of an already-absent path is a harmless no-op.
        index.reindex_one(&c);
        assert_eq!(index.len(), 2);

        // Non-md and outside-vault paths are ignored.
        let txt = vault.path().join("notes.txt");
        write(&txt, "x");
        index.reindex_one(&txt);
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn reindex_updates_warnings_for_the_touched_file() {
        let vault = TempDir::new("vk-reindex-warn");
        let bad = vault.path().join("bad.md");
        write(&bad, "no frontmatter\n");
        let mut index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(index.warnings().len(), 1);

        // Fix the file: its warning disappears, no others appear.
        write(&bad, "---\nstatus: active\n---\nbody\n");
        index.reindex_one(&bad);
        assert!(index.warnings().is_empty());

        // Break it again in a different way: warning replaced, not stacked.
        write(&bad, "---\nstatus: active\n"); // unterminated fence
        index.reindex_one(&bad);
        assert_eq!(index.warnings().len(), 1);
        assert_eq!(
            index.warnings()[0].message,
            "frontmatter fence never closed"
        );
    }

    #[test]
    fn unreadable_file_becomes_warning_not_crash() {
        let vault = TempDir::new("vk-unreadable");
        write(&vault.path().join("a.md"), "ok\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = vault.path().join("locked.md");
            write(&locked, "secret\n");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            let index = VaultIndex::build(vault.path()).unwrap();
            assert_eq!(index.len(), 1);
            assert!(index
                .warnings()
                .iter()
                .any(|w| w.path.ends_with("locked.md") && w.message.contains("unreadable")));
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        }
    }

    #[test]
    fn code_blocks_do_not_generate_links() {
        let vault = TempDir::new("vk-code");
        write(
            &vault.path().join("a.md"),
            "```\n[[fenced]]\n```\n`[[inline]]`\n[[real]]\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(index.links_from(&vault.path().join("a.md")).len(), 1);
    }

    #[test]
    fn frontmatter_links_are_indexed_and_make_backlinks() {
        let vault = TempDir::new("vk-fm-links");
        write(
            &vault.path().join("source.md"),
            "---\nstatus: active\nrelated:\n  - \"[[target]]\"\n  - \"[[target#Section]]\"\n---\nBody [[body-only]].\n",
        );
        write(&vault.path().join("target.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();

        let links = index.links_from(&vault.path().join("source.md"));
        assert_eq!(links.len(), 3, "2 frontmatter + 1 body link");
        assert!(links[0].from_frontmatter && links[1].from_frontmatter);
        assert!(!links[2].from_frontmatter);
        assert_eq!(links[0].target, "target");
        assert_eq!(links[1].heading.as_deref(), Some("Section"));

        // The backlink map includes frontmatter links.
        assert_eq!(
            index.backlinks_to(&vault.path().join("target.md")),
            vec![vault.path().join("source.md")]
        );
    }

    #[test]
    fn path_targets_fall_back_to_unique_suffix_match() {
        let vault = TempDir::new("vk-suffix");
        write(
            &vault
                .path()
                .join("archive/projects/x/execution-log/index.md"),
            "x\n",
        );
        write(
            &vault.path().join("linker.md"),
            "See [[execution-log/index]].\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();

        assert_eq!(
            index.resolve("execution-log/index"),
            Resolution::Found(
                vault
                    .path()
                    .join("archive/projects/x/execution-log/index.md")
            ),
            "stale path link resolves via unique suffix"
        );
        assert_eq!(
            index.backlinks_to(
                &vault
                    .path()
                    .join("archive/projects/x/execution-log/index.md")
            ),
            vec![vault.path().join("linker.md")]
        );
        assert_eq!(index.resolve("no/such/path"), Resolution::NotFound);
    }

    #[test]
    fn ambiguous_suffix_matches_are_reported_not_guessed() {
        let vault = TempDir::new("vk-suffix-amb");
        write(&vault.path().join("a/sub/log.md"), "x\n");
        write(&vault.path().join("b/sub/log.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let mut candidates = match index.resolve("sub/log") {
            Resolution::Ambiguous(c) => c,
            other => panic!("expected ambiguity, got {other:?}"),
        };
        candidates.sort();
        assert_eq!(
            candidates,
            vec![
                vault.path().join("a/sub/log.md"),
                vault.path().join("b/sub/log.md")
            ]
        );
    }

    #[test]
    fn full_path_match_beats_suffix_fallback() {
        let vault = TempDir::new("vk-full-beats-suffix");
        write(&vault.path().join("log.md"), "x\n");
        write(&vault.path().join("deep/log.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            index.resolve("deep/log"),
            Resolution::Found(vault.path().join("deep/log.md")),
            "exact path match preferred over suffix candidates"
        );
    }

    #[test]
    fn bare_stem_matching_is_case_insensitive() {
        let vault = TempDir::new("vk-stem-case");
        write(&vault.path().join("Mixed-Case-Note.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            index.resolve("mixed-case-note"),
            Resolution::Found(vault.path().join("Mixed-Case-Note.md"))
        );
        assert_eq!(
            index.resolve("MIXED-CASE-NOTE"),
            Resolution::Found(vault.path().join("Mixed-Case-Note.md"))
        );
    }

    #[test]
    fn embeds_stay_in_links_but_are_tagged() {
        let vault = TempDir::new("vk-embed");
        write(&vault.path().join("dash.md"), "![[tasks.base]]\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let links = index.links_from(&vault.path().join("dash.md"));
        assert_eq!(links.len(), 1);
        assert!(links[0].is_embed);
        assert!(!links[0].from_frontmatter);
    }

    fn amb(index: &VaultIndex, target: &str) -> Vec<PathBuf> {
        match index.resolve(target) {
            Resolution::Ambiguous(c) => c,
            other => panic!("expected ambiguity for {target}, got {other:?}"),
        }
    }

    #[test]
    fn hint_prefers_same_folder_then_nearest_ancestor() {
        let vault = TempDir::new("vk-hint");
        write(&vault.path().join("dup.md"), "x\n");
        write(&vault.path().join("a/dup.md"), "x\n");
        write(&vault.path().join("a/sub/dup.md"), "x\n");
        write(&vault.path().join("b/dup.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();

        let source = vault.path().join("a/sub/source.md");
        // Same folder as the source wins outright.
        assert_eq!(
            index.resolve_with_hint(&source, "dup"),
            Resolution::Found(vault.path().join("a/sub/dup.md"))
        );
        // No same-folder candidate: nearest ancestor (a/) beats the root.
        let source = vault.path().join("a/source.md");
        assert_eq!(
            index.resolve_with_hint(&source, "dup"),
            Resolution::Found(vault.path().join("a/dup.md"))
        );
        // Path-style ambiguity gets the same hint treatment.
        assert_eq!(
            index.resolve_with_hint(&source, "sub/dup"),
            Resolution::Found(vault.path().join("a/sub/dup.md"))
        );
        // Unambiguous and missing targets pass through untouched.
        write(&vault.path().join("only.md"), "x\n");
        let index = {
            let mut idx = index;
            idx.reindex_one(&vault.path().join("only.md"));
            idx
        };
        assert_eq!(
            index.resolve_with_hint(&source, "only"),
            Resolution::Found(vault.path().join("only.md"))
        );
        assert_eq!(
            index.resolve_with_hint(&source, "nope"),
            Resolution::NotFound
        );
    }

    #[test]
    fn hint_without_related_candidates_returns_ambiguity() {
        let vault = TempDir::new("vk-hint-none");
        write(&vault.path().join("a/dup.md"), "x\n");
        write(&vault.path().join("b/dup.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let candidates = amb(&index, "dup");

        // Source elsewhere: no candidate is in its folder chain.
        let source = vault.path().join("c/source.md");
        assert_eq!(
            index.resolve_with_hint(&source, "dup"),
            Resolution::Ambiguous(candidates.clone())
        );
        // Source at the vault root: no candidate folder is an ancestor of
        // the root itself.
        let root_source = vault.path().join("source.md");
        assert_eq!(
            index.resolve_with_hint(&root_source, "dup"),
            Resolution::Ambiguous(candidates)
        );
        // A source outside the vault gets no hint at all.
        let outside = Path::new("/elsewhere/source.md");
        assert!(matches!(
            index.resolve_with_hint(outside, "dup"),
            Resolution::Ambiguous(_)
        ));
    }

    #[test]
    fn hint_deeper_ancestor_beats_shallow_shared_root() {
        // All candidates share the vault root as an ancestor: the deepest
        // (nearest) ancestor wins, a tie refuses to guess.
        let vault = TempDir::new("vk-hint-depth");
        write(&vault.path().join("log.md"), "x\n");
        write(&vault.path().join("a/log.md"), "x\n");
        let index = VaultIndex::build(vault.path()).unwrap();

        let source = vault.path().join("a/other.md");
        assert_eq!(
            index.resolve_with_hint(&source, "log"),
            Resolution::Found(vault.path().join("a/log.md")),
            "a/ is a nearer ancestor than the vault root"
        );
        // Equal distance (both candidates in the same folder — the root):
        // refuse to guess. Constructed via suffix ambiguity instead, where
        // two candidates can share a folder chain.
        let vault2 = TempDir::new("vk-hint-tie");
        write(&vault2.path().join("x/sub/target.md"), "x\n");
        write(&vault2.path().join("y/sub/target.md"), "x\n");
        let index2 = VaultIndex::build(vault2.path()).unwrap();
        let source = vault2.path().join("z/other.md");
        assert!(matches!(
            index2.resolve_with_hint(&source, "sub/target"),
            Resolution::Ambiguous(_)
        ));
    }
}
