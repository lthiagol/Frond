//! Navigation history: back/forward stacks of `(path, scroll)` entries.
//!
//! Semantics (documented in the help popup):
//! - every file open pushes the previously-current note (with its scroll
//!   position) onto the back stack and clears the forward stack;
//! - `Backspace` pops the back stack, restoring both the file and its
//!   scroll position, and pushes the displaced current note forward;
//! - `Ctrl+O` does the mirror image onto the back stack.

use std::path::PathBuf;

/// One history entry: which note was open and where the viewport was.
pub type Entry = (PathBuf, u32);

#[derive(Default)]
pub struct Nav {
    back: Vec<Entry>,
    forward: Vec<Entry>,
}

impl Nav {
    /// Called *before* opening the new file: records the outgoing note.
    pub fn push(&mut self, entry: Entry) {
        self.back.push(entry);
        self.forward.clear();
    }

    /// True if `Backspace` has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// True if forward navigation has somewhere to go.
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The entry `Backspace` would restore, without mutating the stacks.
    /// Callers open the target first and only commit via [`Self::pop_back`]
    /// once the open succeeded, so a failed open cannot corrupt history.
    pub fn peek_back(&self) -> Option<&Entry> {
        self.back.last()
    }

    /// Mirror of [`Self::peek_back`] for `Ctrl+O`.
    pub fn peek_forward(&self) -> Option<&Entry> {
        self.forward.last()
    }

    /// Pops the most recent back entry (the caller restores it) and records
    /// `current` for a possible forward jump.
    pub fn pop_back(&mut self, current: Entry) -> Option<Entry> {
        let entry = self.back.pop()?;
        self.forward.push(current);
        Some(entry)
    }

    /// Mirror of `pop_back` for `Ctrl+O`.
    pub fn pop_forward(&mut self, current: Entry) -> Option<Entry> {
        let entry = self.forward.pop()?;
        self.back.push(current);
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(format!("/v/{name}.md"))
    }

    #[test]
    fn open_clears_forward_stack() {
        let mut nav = Nav::default();
        nav.push((p("a"), 5)); // left a@5 → now at b
        nav.push((p("b"), 0)); // left b@0 → now at c
        let back = nav.pop_back((p("c"), 7)).unwrap(); // back → b@0
        assert_eq!(back, (p("b"), 0));
        assert!(nav.can_go_forward(), "c@7 lies ahead");
        // Opening a new file from here must invalidate forward.
        nav.push((p("b"), 1)); // left b@1 → now at d
        assert!(!nav.can_go_forward());
        assert_eq!(nav.pop_back((p("d"), 0)).unwrap(), (p("b"), 1));
        assert_eq!(nav.pop_back((p("d"), 0)).unwrap(), (p("a"), 5));
        assert!(!nav.can_go_back());
    }

    #[test]
    fn back_and_forward_roundtrip() {
        let mut nav = Nav::default();
        nav.push((p("a"), 12)); // left a@12 → now at b
        let back = nav.pop_back((p("b"), 3)).unwrap(); // back → a@12
        assert_eq!(back, (p("a"), 12));
        let fwd = nav.pop_forward((p("a"), 12)).unwrap(); // forward → b@3
        assert_eq!(fwd, (p("b"), 3), "forward restores the saved scroll");
        // The displaced a@12 is behind again.
        assert_eq!(nav.pop_back((p("b"), 0)).unwrap(), (p("a"), 12));
    }

    #[test]
    fn empty_stacks_are_noops() {
        let mut nav = Nav::default();
        assert!(nav.pop_back((p("x"), 0)).is_none());
        assert!(nav.pop_forward((p("x"), 0)).is_none());
        assert!(!nav.can_go_back() && !nav.can_go_forward());
    }
}
