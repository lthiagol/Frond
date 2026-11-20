//! The vault index: notes, frontmatter, wikilinks, resolver, backlinks,
//! tag index, and single-file incremental refresh.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::frontmatter::{parse_frontmatter_with_raw, split_frontmatter};
use crate::walk::{is_base_path, is_indexable, strip_suffix_ignore_case, walk_vault_with_warnings};
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

/// A compiled search rule: the case-insensitive literal substring or the
/// case-insensitive regular expression, matched one line at a time.
///
/// [`VaultIndex::search`], [`VaultIndex::search_regex`] and the app's
/// in-note `n`/`N` all scan through this type, so a query finds the same
/// lines in one note as it does across the vault.
#[derive(Clone, Debug)]
pub enum SearchPattern {
    /// A blank query (a bare `re:` included) searches nothing.
    Empty,
    /// A lowercased literal needle, plus whether it is all ASCII (the
    /// per-line scan's fast path).
    Literal { needle: String, ascii: bool },
    /// A compiled case-insensitive regular expression.
    Regex(regex::Regex),
}

impl SearchPattern {
    /// The literal query's rule. An empty query matches nothing, like the
    /// search's blank-query rule (and unlike an empty regular expression,
    /// which would match every line).
    pub fn literal(query: &str) -> Self {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return Self::Empty;
        }
        let ascii = needle.is_ascii();
        Self::Literal { needle, ascii }
    }

    /// The regular-expression query's rule: compiled case-insensitively.
    /// An empty pattern matches nothing — the search's bare-`re:` rule.
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        if pattern.is_empty() {
            return Ok(Self::Empty);
        }
        Ok(Self::Regex(
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()?,
        ))
    }

    /// `true` when the pattern can never match (a blank query).
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// The 0-based indices of the lines of `text` the pattern matches, in
    /// line order. `text.lines()` is the split the vault scan uses for a
    /// note's raw text, so the indices are the note's source lines.
    pub fn matching_lines(&self, text: &str) -> Vec<usize> {
        let mut scratch = String::new();
        text.lines()
            .enumerate()
            .filter(|(_, line)| self.find_in(line, &mut scratch).is_some())
            .map(|(index, _)| index)
            .collect()
    }

    /// The first match on `line`, as byte offsets in it (snapped to char
    /// boundaries — case folding can shift them), or `None`. `scratch` is a
    /// reusable buffer for the ASCII path; the caller owns it so a
    /// vault-wide scan allocates nothing per line.
    fn find_in(&self, line: &str, scratch: &mut String) -> Option<(usize, usize)> {
        match self {
            Self::Empty => None,
            Self::Regex(regex) => regex.find(line).map(|found| (found.start(), found.end())),
            Self::Literal { needle, ascii } => {
                if *ascii && line.is_ascii() {
                    scratch.clear();
                    scratch.push_str(line);
                    scratch.make_ascii_lowercase();
                    let pos = scratch.find(needle.as_str())?;
                    Some((pos, pos + needle.len()))
                } else {
                    let folded = line.to_lowercase();
                    let pos = folded.find(needle.as_str())?;
                    Some((
                        floor_char_boundary(line, pos),
                        ceil_char_boundary(line, pos + needle.len()),
                    ))
                }
            }
        }
    }
}

/// Case-insensitive `ends_with` (ASCII folding, matching the
/// `eq_ignore_ascii_case` semantics used elsewhere here).
fn ends_with_ci(haystack: &str, needle: &str) -> bool {
    // Compare as bytes: slicing the `str` at `len - needle.len()` panics when
    // that offset lands inside a multi-byte character.
    haystack.len() >= needle.len()
        && haystack.as_bytes()[haystack.len() - needle.len()..]
            .eq_ignore_ascii_case(needle.as_bytes())
}

/// A note's cached source for the global search.
#[derive(Clone, Debug)]
struct RawText {
    text: String,
    /// ASCII-lowercased copy of `text`, kept only for all-ASCII notes (then
    /// byte offsets agree with `text`), so one substring search covers the
    /// whole note instead of one per line.
    folded: Option<String>,
}

impl RawText {
    fn new(source: &str) -> Self {
        Self {
            folded: source.is_ascii().then(|| source.to_ascii_lowercase()),
            text: source.to_string(),
        }
    }
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
    /// Lowercased root-relative note path without `.md` -> paths (several
    /// only when the vault has paths differing by case alone). Makes
    /// path-style links an O(1) lookup instead of a scan of every note.
    by_rel: BTreeMap<String, Vec<PathBuf>>,
    /// Lowercased `.base` stem (name without the suffix) -> paths sharing
    /// it, same ambiguity semantics as `by_stem`.
    by_base_stem: BTreeMap<String, Vec<PathBuf>>,
    /// Note path -> notes linking to it. Built by resolving each link;
    /// ambiguous targets contribute to every candidate.
    backlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Tag -> notes carrying it in frontmatter (frontmatter tags only —
    /// the vault has no inline tags).
    tags: BTreeMap<String, Vec<PathBuf>>,
    /// Cached raw text of every indexed note (one `String` per path,
    /// split into lines lazily by the search), kept in sync by
    /// `insert_note`/`remove_note_entry` — the global search runs against
    /// this cache, never against the disk.
    raw_lines: BTreeMap<PathBuf, RawText>,
}

impl VaultIndex {
    /// Walks `root` and indexes every `.md` note; `.base` files are
    /// collected path-only (never parsed). Fails only if `root` itself
    /// cannot be read; per-file problems and unreadable subdirectories
    /// become warnings.
    pub fn build(root: &Path) -> io::Result<Self> {
        let mut index = Self {
            root: root.to_path_buf(),
            notes: BTreeMap::new(),
            bases: BTreeSet::new(),
            warnings: Vec::new(),
            by_stem: BTreeMap::new(),
            by_rel: BTreeMap::new(),
            by_base_stem: BTreeMap::new(),
            backlinks: BTreeMap::new(),
            tags: BTreeMap::new(),
            raw_lines: BTreeMap::new(),
        };
        let walk = walk_vault_with_warnings(root)?;
        // A directory the walk could not read is reported exactly like an
        // unreadable file: its notes are skipped, but the user is told.
        index
            .warnings
            .extend(walk.warnings.into_iter().map(|w| IndexWarning {
                path: w.path,
                message: w.message,
            }));
        for entry in walk.entries {
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

    /// Diagnostic API (used by `examples/index_vault.rs`, not by the app).
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
    #[cfg(test)]
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
        let pattern = SearchPattern::literal(query);
        let SearchPattern::Literal { needle, ascii } = &pattern else {
            return SearchResults::default();
        };
        let mut hits: Vec<SearchHit> = Vec::new();
        let ascii_needle = *ascii;
        // A needle with a line break could match across lines in a whole-note
        // scan, which the per-line search never does: keep those per-line.
        let whole_note = ascii_needle && !needle.contains(['\n', '\r']);
        let mut ascii_buf = String::new();
        for (path, raw) in &self.raw_lines {
            if let (true, Some(folded)) = (whole_note, &raw.folded) {
                // One substring search over the whole (all-ASCII) note; each
                // match is mapped back to its line, resuming on the next
                // line so there is one hit per matching line.
                let mut from = 0;
                let (mut counted, mut line_idx) = (0, 0);
                while let Some(rel) = folded[from..].find(needle.as_str()) {
                    let pos = from + rel;
                    let line_start = folded[..pos].rfind('\n').map_or(0, |i| i + 1);
                    let line_end = folded[pos..].find('\n').map_or(folded.len(), |i| pos + i);
                    line_idx += folded[counted..line_start]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count();
                    counted = line_start;
                    // `str::lines` drops the `\r` of a `\r\n` terminator.
                    let content_end =
                        if line_end < folded.len() && raw.text[..line_end].ends_with('\r') {
                            line_end - 1
                        } else {
                            line_end
                        };
                    let hit = SearchHit {
                        path: path.clone(),
                        line: line_idx,
                        text: raw.text[line_start..content_end].to_string(),
                        start: pos - line_start,
                        end: pos - line_start + needle.len(),
                    };
                    if push_hit(&mut hits, hit) {
                        return SearchResults { hits, capped: true };
                    }
                    from = line_end + 1;
                    if from > folded.len() {
                        break;
                    }
                }
                continue;
            }
            for (index, line) in raw.text.lines().enumerate() {
                // The per-line path is the shared rule (`find_in`): an
                // all-ASCII line and needle fold bytewise into one reused
                // buffer (no allocation per line); any non-ASCII line takes
                // the Unicode path (it can match an ASCII needle through
                // folding, e.g. the Kelvin sign).
                let Some((start, end)) = pattern.find_in(line, &mut ascii_buf) else {
                    continue;
                };
                let hit = SearchHit {
                    path: path.clone(),
                    line: index,
                    text: line.to_string(),
                    start,
                    end,
                };
                if push_hit(&mut hits, hit) {
                    return SearchResults { hits, capped: true };
                }
            }
        }
        SearchResults {
            hits,
            capped: false,
        }
    }

    /// Case-insensitive regular-expression search over the same cached raw
    /// lines as [`Self::search`], matched line by line: one hit per
    /// matching line (the first match on it), in path order then line
    /// order, with the same cap and [`SearchHit`] offsets. An invalid
    /// pattern is returned as an error and nothing is scanned; an empty one
    /// searches nothing.
    pub fn search_regex(&self, pattern: &str) -> Result<SearchResults, regex::Error> {
        let pattern = SearchPattern::regex(pattern)?;
        let mut scratch = String::new();
        let mut hits = Vec::new();
        for (path, raw) in &self.raw_lines {
            for (index, line) in raw.text.lines().enumerate() {
                let Some((start, end)) = pattern.find_in(line, &mut scratch) else {
                    continue;
                };
                let hit = SearchHit {
                    path: path.clone(),
                    line: index,
                    text: line.to_string(),
                    start,
                    end,
                };
                if push_hit(&mut hits, hit) {
                    return Ok(SearchResults { hits, capped: true });
                }
            }
        }
        Ok(SearchResults {
            hits,
            capped: false,
        })
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
        let target = strip_suffix_ignore_case(target, ".md")
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
                .by_rel
                .get(&target.to_ascii_lowercase())
                .cloned()
                .unwrap_or_default();
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
        let target = strip_suffix_ignore_case(target, ".base")
            .unwrap_or(target)
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
                    strip_suffix_ignore_case(&rel, ".base")
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
                let rel = strip_suffix_ignore_case(&n.rel_path, ".md").unwrap_or(&n.rel_path);
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
        if self.apply_one(path) {
            self.rebuild_derived();
        }
    }

    /// [`Self::reindex_one`] for a batch: every path is applied, then the
    /// derived maps are rebuilt once (not once per path), so a `git pull`
    /// touching hundreds of notes costs one rebuild.
    pub fn reindex_many<P: AsRef<Path>>(&mut self, paths: &[P]) {
        let mut dirty = false;
        for path in paths {
            dirty |= self.apply_one(path.as_ref());
        }
        if dirty {
            self.rebuild_derived();
        }
    }

    /// Applies one path's change to `bases` / `notes` / `warnings` without
    /// rebuilding derived maps. Returns `true` when they are now stale.
    fn apply_one(&mut self, path: &Path) -> bool {
        if is_base_path(path) {
            let listed = path.exists() && path.strip_prefix(&self.root).is_ok();
            let was_listed = self.bases.contains(path);
            if listed {
                self.bases.insert(path.to_path_buf());
            } else {
                self.bases.remove(path);
            }
            return listed != was_listed;
        }
        if !is_indexable(&self.root, path) {
            return self.remove_note_entry(path);
        }
        match fs::read_to_string(path) {
            Ok(source) => {
                self.insert_note(path, &source);
                true
            }
            Err(err) => {
                let dirty = self.remove_note_entry(path);
                // Unreadable-but-present is a warning, not a silent drop.
                if path.exists() {
                    self.warnings.retain(|w| w.path != path);
                    self.warnings.push(IndexWarning {
                        path: path.to_path_buf(),
                        message: format!("unreadable: {err}"),
                    });
                }
                dirty
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
        self.raw_lines
            .insert(path.to_path_buf(), RawText::new(source));
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
    /// Drops one note's entries; `true` when anything was removed (derived
    /// maps are then stale).
    fn remove_note_entry(&mut self, path: &Path) -> bool {
        let had_lines = self.raw_lines.remove(path).is_some();
        if self.notes.remove(path).is_some()
            || had_lines
            || self.warnings.iter().any(|w| w.path == path)
        {
            self.warnings.retain(|w| w.path != path);
            return true;
        }
        false
    }

    /// Rebuilds the basename map, backlink map, and tag map from `notes`
    /// (plus the base-stem map from `bases`).
    fn rebuild_derived(&mut self) {
        self.by_rel.clear();
        for note in self.notes.values() {
            let rel = strip_suffix_ignore_case(&note.rel_path, ".md").unwrap_or(&note.rel_path);
            self.by_rel
                .entry(rel.to_ascii_lowercase())
                .or_default()
                .push(note.path.clone());
        }

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

/// Pushes one search hit, keeping at most [`SEARCH_RESULT_CAP`] and
/// reporting whether the cap was exceeded (the caller stops the scan).
/// The one home of the cap rule: both search paths use it.
fn push_hit(hits: &mut Vec<SearchHit>, hit: SearchHit) -> bool {
    hits.push(hit);
    if hits.len() > SEARCH_RESULT_CAP {
        hits.truncate(SEARCH_RESULT_CAP);
        return true;
    }
    false
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

    /// The one-note scan behind the app's `n`/`N` finds exactly the lines
    /// the vault-wide search reports for that note — the same rule, not a
    /// second copy of it.
    #[test]
    fn search_pattern_finds_the_lines_the_vault_search_reports() {
        let vault = TempDir::new("vk-search-pattern");
        let text = "---\nstatus: active\n---\nNeedle one\nplain\nneedle two\n";
        write(&vault.path().join("a.md"), text);
        let index = VaultIndex::build(vault.path()).unwrap();

        let literal = SearchPattern::literal("needle");
        let searched: Vec<usize> = index.search("needle").hits.iter().map(|h| h.line).collect();
        assert_eq!(searched, vec![3, 5]);
        assert_eq!(literal.matching_lines(text), searched);
        assert_eq!(
            SearchPattern::literal("NEEDLE").matching_lines(text),
            searched,
            "case-insensitive, like the search"
        );

        let regex = SearchPattern::regex(r"needle \w+").unwrap();
        let searched: Vec<usize> = index
            .search_regex(r"needle \w+")
            .unwrap()
            .hits
            .iter()
            .map(|h| h.line)
            .collect();
        assert_eq!(&searched, &[3, 5]);
        assert_eq!(regex.matching_lines(text), searched);

        // A blank query and a bare `re:` search nothing, like the popup.
        assert!(SearchPattern::literal("").matching_lines(text).is_empty());
        assert!(SearchPattern::regex("")
            .unwrap()
            .matching_lines(text)
            .is_empty());
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

    // --- unit 28: regex search -----------------------------------------

    #[test]
    fn regex_search_matches_a_shape_case_insensitively_in_path_order() {
        let vault = TempDir::new("vk-regex-shape");
        write(
            &vault.path().join("b.md"),
            "Invoice INV-2024 here\nnothing\n",
        );
        write(
            &vault.path().join("a.md"),
            "alpha 123\nBeta 456\na 7 only\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();

        // A shape, not a literal: three digits, case-insensitive letters.
        let hits = index.search_regex(r"inv-\d+").unwrap().hits;
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, vault.path().join("b.md"));
        assert_eq!(hits[0].line, 0);
        assert_eq!(&hits[0].text[hits[0].start..hits[0].end], "INV-2024");

        let hits = index.search_regex(r"[a-z]+ \d{3}").unwrap().hits;
        assert_eq!(hits.len(), 2, "a.md's first two lines match: {hits:?}");
        assert_eq!(hits[0].path, vault.path().join("a.md"), "path order");
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[1].line, 1);
        assert_eq!(&hits[0].text[hits[0].start..hits[0].end], "alpha 123");
    }

    #[test]
    fn regex_search_is_line_by_line_and_case_insensitive() {
        let vault = TempDir::new("vk-regex-lines");
        write(&vault.path().join("a.md"), "Alpha\nBeta\n");
        let index = VaultIndex::build(vault.path()).unwrap();

        let hits = index.search_regex("beta").unwrap().hits;
        assert_eq!(hits.len(), 1, "case-insensitive match");
        assert_eq!(&hits[0].text[hits[0].start..hits[0].end], "Beta");

        assert!(
            index.search_regex("Alpha\nBeta").unwrap().hits.is_empty(),
            "a pattern spanning a line break never matches across lines"
        );
        assert!(
            index.search_regex("gamma").unwrap().hits.is_empty(),
            "a pattern that matches nothing is an empty result, not an error"
        );
        assert!(
            index.search_regex("").unwrap().hits.is_empty(),
            "an empty pattern searches nothing, like an empty query"
        );
    }

    #[test]
    fn regex_search_rejects_an_invalid_pattern() {
        let vault = TempDir::new("vk-regex-invalid");
        write(&vault.path().join("a.md"), "anything\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        assert!(
            index.search_regex("(").is_err(),
            "an unbalanced group is reported as an error"
        );
        assert!(index.search_regex("[a-").is_err());
    }

    #[test]
    fn regex_search_caps_like_the_substring_search() {
        let vault = TempDir::new("vk-regex-cap");
        let body: String = (0..SEARCH_RESULT_CAP + 100).map(|_| "hit 1\n").collect();
        write(&vault.path().join("big.md"), &body);
        let index = VaultIndex::build(vault.path()).unwrap();
        let results = index.search_regex(r"hit \d").unwrap();
        assert_eq!(results.hits.len(), SEARCH_RESULT_CAP, "capped at 1000");
        assert!(results.capped);
        assert_eq!(results.hits[0].line, 0);
        assert_eq!(
            results.hits[SEARCH_RESULT_CAP - 1].line,
            SEARCH_RESULT_CAP - 1,
            "the FIRST hits are kept"
        );
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
    fn a_tag_containing_a_comma_is_indexed_once() {
        // Audit VAULT-4 end to end: the tag view lists the tag once and a
        // base `file.hasTag` lookup can match it (see base.rs).
        let vault = TempDir::new("vk-tag-comma");
        write(
            &vault.path().join("note.md"),
            "---\ntags:\n  - \"mocktag, weird\"\n  - gpu\n---\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();
        assert_eq!(
            note_at(&index, "note.md").tags,
            vec!["mocktag, weird", "gpu"]
        );
        let hits = index.notes_with_tag("mocktag, weird");
        assert_eq!(hits.len(), 1, "the comma tag is one tag");
        assert_eq!(hits[0].rel_path, "note.md");
        assert!(
            index.notes_with_tag("mocktag").is_empty(),
            "the comma tag is not split into a bare `mocktag`"
        );
        assert_eq!(index.tag_counts().len(), 2, "two tags, not three");
    }

    #[test]
    fn search_ascii_fast_path_agrees_with_unicode_folding() {
        let vault = TempDir::new("vk-search-fold");
        // Plain ASCII line (fast path) and a line where only Unicode
        // folding finds the ASCII needle (U+212A KELVIN SIGN lowercases
        // to `k`).
        write(&vault.path().join("a.md"), "has a Kite here\n");
        write(&vault.path().join("b.md"), "\u{212A}ite too\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let hits = index.search("kite").hits;
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(&hits[0].text[hits[0].start..hits[0].end], "Kite");
    }

    /// The whole-note fast path (all-ASCII notes) must return exactly what
    /// the per-line path does: same lines, texts and ranges — CRLF files,
    /// repeated matches in one line, first/last lines, no trailing newline.
    #[test]
    fn whole_note_search_matches_the_per_line_search() {
        let body = "Needle first\r\nnothing\r\n\r\nneedle needle twice\nlast NEEDLE no newline";
        let vault = TempDir::new("vk-search-equiv");
        write(&vault.path().join("ascii.md"), body);
        // Same text plus one non-ASCII char forces the per-line path.
        write(&vault.path().join("uni.md"), &format!("é\n{body}"));
        let index = VaultIndex::build(vault.path()).unwrap();
        let hits = index.search("needle").hits;
        let pick = |name: &str| -> Vec<(usize, String, usize, usize)> {
            hits.iter()
                .filter(|h| h.path.ends_with(name))
                .map(|h| (h.line, h.text.clone(), h.start, h.end))
                .collect()
        };
        let ascii = pick("ascii.md");
        let uni: Vec<_> = pick("uni.md")
            .into_iter()
            .map(|(line, text, s, e)| (line - 1, text, s, e)) // the extra first line
            .collect();
        assert_eq!(ascii.len(), 3, "{ascii:?}");
        assert_eq!(ascii, uni);
        assert_eq!(ascii[0], (0, "Needle first".to_string(), 0, 6));
        assert_eq!(ascii[1].0, 3);
        assert_eq!(ascii[2], (4, "last NEEDLE no newline".to_string(), 5, 11));
        // A needle spanning a line break never matches across lines.
        assert!(index.search("first\nnothing").hits.is_empty());
    }

    #[test]
    fn non_ascii_link_targets_do_not_panic() {
        let vault = TempDir::new("vk-nonascii");
        write(
            &vault.path().join("a.md"),
            "see [[ééé]] and [[sub/ééé]] and [[café]]",
        );
        write(&vault.path().join("café.md"), "x");
        let index = VaultIndex::build(vault.path()).expect("build must not panic");
        assert_eq!(
            index.resolve("café"),
            Resolution::Found(vault.path().join("café.md"))
        );
        assert_eq!(index.resolve("ééé"), Resolution::NotFound);
        assert_eq!(index.resolve("sub/ééé"), Resolution::NotFound);
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
    fn uppercase_md_suffix_resolves_bare_and_path_links() {
        // The walker indexes `X.MD` (its suffix test ignores case); the
        // resolver must strip the same suffix case-insensitively, both for
        // a bare target and for the path-style form.
        let vault = TempDir::new("vk-md-case");
        write(&vault.path().join("dir/X.MD"), "x\n");
        write(
            &vault.path().join("source.md"),
            "[[X.MD]] [[dir/X]] [[dir/X.MD]]\n",
        );
        let index = VaultIndex::build(vault.path()).unwrap();
        let note = vault.path().join("dir/X.MD");
        for target in ["X.MD", "X", "dir/X", "dir/X.MD"] {
            assert_eq!(
                index.resolve(target),
                Resolution::Found(note.clone()),
                "{target}"
            );
        }
        assert_eq!(
            index.backlinks_to(&note),
            vec![vault.path().join("source.md")],
            "both link spellings resolve to the note"
        );
    }

    #[test]
    fn uppercase_base_suffix_resolves_path_targets() {
        let vault = TempDir::new("vk-base-case");
        write(&vault.path().join("dir/Board.BASE"), "views: []\n");
        let index = VaultIndex::build(vault.path()).unwrap();
        let base = vault.path().join("dir/Board.BASE");
        for target in [
            "Board.BASE",
            "board.base",
            "dir/Board.BASE",
            "dir/board.base",
        ] {
            assert_eq!(
                index.resolve(target),
                Resolution::Found(base.clone()),
                "{target}"
            );
        }
    }

    #[test]
    fn unreadable_directory_becomes_warning_not_silent_drop() {
        let vault = TempDir::new("vk-unreadable-dir");
        write(&vault.path().join("a.md"), "ok\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = vault.path().join("locked");
            write(&locked.join("inside.md"), "---\ntags: [hidden]\n---\n");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            let index = VaultIndex::build(vault.path()).unwrap();
            assert_eq!(index.len(), 1, "notes in the locked folder are skipped");
            assert!(
                index
                    .warnings()
                    .iter()
                    .any(|w| w.path.ends_with("locked") && w.message.contains("unreadable")),
                "warning names the directory: {:?}",
                index.warnings()
            );
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
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
