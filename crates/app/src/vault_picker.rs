//! Vault picker: chooses one of the config's `[vaults]` entries. The
//! behaviour (listing, filter, keys, preselection, the not-a-directory
//! error) is SPEC.md's **Vault picker** bullet.
//!
//! Terminal-independent state, like [`crate::quick`]: filtering reuses the
//! quick switcher's subsequence matcher ([`crate::quick::fuzzy_match`]) and
//! the drawing lives in `ui.rs` ([`crate::ui::draw_vault_picker`]). At
//! startup `main.rs` drives it in a small event loop of its own, before any
//! vault is indexed.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::quick::fuzzy_match;

/// One `[vaults]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultEntry {
    pub name: String,
    /// The entry's path, `~` already expanded.
    pub path: PathBuf,
    /// The path as listed: `$HOME` shown as `~`.
    pub display: String,
}

impl VaultEntry {
    pub fn new(name: String, path: PathBuf, home: Option<&Path>) -> Self {
        let display = home_relative(&path, home);
        Self {
            name,
            path,
            display,
        }
    }
}

/// `path` with a leading `home` written as `~`.
fn home_relative(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// A listed row: the entry and the matched characters of its name and of
/// its displayed path (for the highlight).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerItem {
    pub entry: usize,
    pub name_positions: Vec<usize>,
    pub path_positions: Vec<usize>,
    score: i32,
}

/// What a key did.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerFlow {
    Continue,
    /// Open this vault (an existing directory).
    Open(PathBuf),
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct PickerState {
    pub entries: Vec<VaultEntry>,
    pub query: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
    /// The inline error (`not a directory: <path>`), cleared by the next key.
    pub error: Option<String>,
}

impl PickerState {
    /// `entries` in name order; `preselect` indexes into them.
    pub fn new(entries: Vec<VaultEntry>, preselect: usize) -> Self {
        let mut state = Self {
            entries,
            ..Self::default()
        };
        state.refresh();
        state.selected = state
            .items
            .iter()
            .position(|item| item.entry == preselect)
            .unwrap_or(0);
        state
    }

    /// Re-filters for the current query; the selection goes to the best
    /// match, as in the quick switcher.
    fn refresh(&mut self) {
        let query = self.query.as_str();
        let mut items: Vec<PickerItem> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(entry, vault)| {
                let name = fuzzy_match(query, &vault.name);
                let path = fuzzy_match(query, &vault.display);
                let (score, name_positions, path_positions) = match (name, path) {
                    (None, None) => return None,
                    (Some(n), Some(p)) if p.score > n.score => (p.score, Vec::new(), p.positions),
                    (Some(n), _) => (n.score, n.positions, Vec::new()),
                    (None, Some(p)) => (p.score, Vec::new(), p.positions),
                };
                Some(PickerItem {
                    entry,
                    name_positions,
                    path_positions,
                    score,
                })
            })
            .collect();
        // Best first; ties (every row, with no query) keep name order.
        items.sort_by(|a, b| b.score.cmp(&a.score).then(a.entry.cmp(&b.entry)));
        self.items = items;
        self.selected = 0;
    }

    pub fn selected_entry(&self) -> Option<&VaultEntry> {
        self.items
            .get(self.selected)
            .map(|item| &self.entries[item.entry])
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerFlow {
        self.error = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return PickerFlow::Quit,
            KeyCode::Char('c') if ctrl => return PickerFlow::Quit,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    if entry.path.is_dir() {
                        return PickerFlow::Open(entry.path.clone());
                    }
                    self.error = Some(format!("not a directory: {}", entry.path.display()));
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh();
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.refresh();
            }
            _ => {}
        }
        PickerFlow::Continue
    }
}

/// The preselected entry: the last vault opened when it is still an
/// entry, else the entry the `vault` key names (by name, or by path), else
/// the first. Paths compare canonically.
pub fn preselect(entries: &[VaultEntry], last: Option<&Path>, vault_key: Option<&Path>) -> usize {
    let canon = |path: &Path| path.canonicalize().ok();
    let by_path = |target: &Path| {
        let target = canon(target)?;
        entries
            .iter()
            .position(|entry| canon(&entry.path).as_deref() == Some(target.as_path()))
    };
    last.and_then(by_path)
        .or_else(|| {
            let key = vault_key?;
            entries
                .iter()
                .position(|entry| Path::new(&entry.name) == key)
                .or_else(|| by_path(key))
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(state: &mut PickerState, text: &str) {
        for c in text.chars() {
            state.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn entries(dir: &Path) -> Vec<VaultEntry> {
        ["archive", "notes", "work"]
            .iter()
            .map(|name| {
                let path = dir.join(name);
                std::fs::create_dir_all(&path).unwrap();
                VaultEntry::new(name.to_string(), path, Some(dir))
            })
            .collect()
    }

    fn selected_name(state: &PickerState) -> &str {
        &state.selected_entry().unwrap().name
    }

    #[test]
    fn paths_under_home_display_with_a_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(
            home_relative(Path::new("/home/me/notes"), Some(home)),
            "~/notes"
        );
        assert_eq!(home_relative(home, Some(home)), "~");
        assert_eq!(home_relative(Path::new("/srv/x"), Some(home)), "/srv/x");
        assert_eq!(
            home_relative(Path::new("/home/me2"), Some(home)),
            "/home/me2"
        );
    }

    #[test]
    fn filter_and_arrows_end_on_the_expected_entry() {
        let dir = TempDir::new("picker-filter");
        let mut state = PickerState::new(entries(dir.path()), 0);
        let names: Vec<&str> = state
            .items
            .iter()
            .map(|item| state.entries[item.entry].name.as_str())
            .collect();
        assert_eq!(names, ["archive", "notes", "work"], "name order");
        state.handle_key(key(KeyCode::Down));
        state.handle_key(key(KeyCode::Down));
        state.handle_key(key(KeyCode::Down));
        assert_eq!(selected_name(&state), "work", "Down stops at the last");
        state.handle_key(key(KeyCode::Up));
        assert_eq!(selected_name(&state), "notes");

        type_str(&mut state, "wk");
        assert_eq!(state.items.len(), 1, "subsequence over the name");
        assert_eq!(selected_name(&state), "work");
        assert_eq!(state.items[0].name_positions, vec![0, 3]);

        state.handle_key(key(KeyCode::Backspace));
        state.handle_key(key(KeyCode::Backspace));
        type_str(&mut state, "NOTES");
        assert_eq!(selected_name(&state), "notes", "case-insensitive");

        state.query.clear();
        state.refresh();
        type_str(&mut state, "zzz");
        assert!(state.items.is_empty());
        assert_eq!(state.handle_key(key(KeyCode::Enter)), PickerFlow::Continue);
    }

    #[test]
    fn filter_matches_the_displayed_path_too() {
        let dir = TempDir::new("picker-path");
        let mut list = entries(dir.path());
        list[0].display = "/srv/qqq".to_string();
        let mut state = PickerState::new(list, 2);
        type_str(&mut state, "qqq");
        assert_eq!(selected_name(&state), "archive");
        assert!(state.items[0].name_positions.is_empty());
        assert_eq!(state.items[0].path_positions, vec![5, 6, 7]);
    }

    #[test]
    fn enter_opens_a_directory_and_reports_anything_else() {
        let dir = TempDir::new("picker-enter");
        let mut list = entries(dir.path());
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        list[1].path = file.clone();
        let mut state = PickerState::new(list, 1);
        assert_eq!(state.handle_key(key(KeyCode::Enter)), PickerFlow::Continue);
        assert_eq!(
            state.error.as_deref(),
            Some(format!("not a directory: {}", file.display()).as_str())
        );
        assert_eq!(selected_name(&state), "notes", "the picker stays open");
        state.handle_key(key(KeyCode::Down));
        assert_eq!(state.error, None, "the next key clears the error");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            PickerFlow::Open(dir.path().join("work"))
        );
    }

    #[test]
    fn esc_and_ctrl_c_quit() {
        let dir = TempDir::new("picker-quit");
        let mut state = PickerState::new(entries(dir.path()), 0);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), PickerFlow::Quit);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(state.handle_key(ctrl_c), PickerFlow::Quit);
        assert_eq!(state.query, "", "Ctrl+C is not typed into the filter");
    }

    #[test]
    fn preselection_prefers_last_then_vault_key_then_first() {
        let dir = TempDir::new("picker-pre");
        let list = entries(dir.path());
        let work = dir.path().join("work");
        let notes = dir.path().join("notes");
        assert_eq!(preselect(&list, None, None), 0, "first");
        assert_eq!(
            preselect(&list, None, Some(Path::new("notes"))),
            1,
            "key by name"
        );
        assert_eq!(preselect(&list, None, Some(&work)), 2, "key by path");
        assert_eq!(
            preselect(&list, Some(&work), Some(Path::new("notes"))),
            2,
            "last wins"
        );
        let gone = dir.path().join("gone");
        assert_eq!(
            preselect(&list, Some(&gone), Some(&notes)),
            1,
            "a last vault no longer listed falls through to the key"
        );
        let state = PickerState::new(list, 2);
        assert_eq!(selected_name(&state), "work");
    }
}
