//! Undo/redo on top of [`Change`]. Memory cost is proportional to what each
//! edit touched, never to the size of the document.

use serde::Serialize;

use crate::timeline::{Change, Timeline, TimelineError};

#[derive(Debug, Clone)]
pub struct Transaction {
    pub label: String,
    pub change: Change,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HistoryInfo {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub revision: u64,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Maximum number of events kept across all undo entries.
    budget: usize,
    revision: u64,
    /// Monotonic counter: every commit gets a never-seen revision number.
    counter: u64,
    saved_revision: u64,
    /// Revision reached after each undo entry; used to know if we are back at the saved state.
    undo_revisions: Vec<u64>,
    redo_revisions: Vec<u64>,
}

impl Default for History {
    fn default() -> Self {
        Self::with_budget(4_000_000)
    }
}

impl History {
    pub fn with_budget(budget: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            budget,
            revision: 0,
            counter: 0,
            saved_revision: 0,
            undo_revisions: Vec::new(),
            redo_revisions: Vec::new(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Applies `change` to `tl` and records it. Empty changes are ignored.
    pub fn commit(
        &mut self,
        tl: &mut Timeline,
        label: impl Into<String>,
        change: Change,
    ) -> Result<bool, TimelineError> {
        if change.is_empty() {
            return Ok(false);
        }
        tl.apply(&change)?;
        self.redo.clear();
        self.redo_revisions.clear();
        self.undo_revisions.push(self.revision);
        self.revision = self.next_revision();
        self.undo.push(Transaction { label: label.into(), change });
        self.trim();
        Ok(true)
    }

    fn next_revision(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn trim(&mut self) {
        let mut total: usize = self.undo.iter().map(|t| t.change.weight()).sum();
        while total > self.budget && self.undo.len() > 1 {
            let dropped = self.undo.remove(0);
            self.undo_revisions.remove(0);
            total -= dropped.change.weight();
        }
    }

    pub fn undo(&mut self, tl: &mut Timeline) -> Result<Option<String>, TimelineError> {
        let Some(tx) = self.undo.pop() else { return Ok(None) };
        let prev_rev = self.undo_revisions.pop().unwrap_or(0);
        if let Err(e) = tl.apply(&tx.change.inverse()) {
            self.undo.push(tx);
            self.undo_revisions.push(prev_rev);
            return Err(e);
        }
        let label = tx.label.clone();
        self.redo_revisions.push(self.revision);
        self.redo.push(tx);
        self.revision = prev_rev;
        Ok(Some(label))
    }

    pub fn redo(&mut self, tl: &mut Timeline) -> Result<Option<String>, TimelineError> {
        let Some(tx) = self.redo.pop() else { return Ok(None) };
        let next_rev = self.redo_revisions.pop().unwrap_or(self.revision + 1);
        if let Err(e) = tl.apply(&tx.change) {
            self.redo.push(tx);
            self.redo_revisions.push(next_rev);
            return Err(e);
        }
        let label = tx.label.clone();
        self.undo_revisions.push(self.revision);
        self.undo.push(tx);
        self.revision = next_rev;
        Ok(Some(label))
    }

    /// The change [`History::undo`] would apply next (for mirroring it).
    pub fn undo_change(&self) -> Option<Change> {
        self.undo.last().map(|t| t.change.inverse())
    }

    /// The change [`History::redo`] would apply next.
    pub fn redo_change(&self) -> Option<Change> {
        self.redo.last().map(|t| t.change.clone())
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_revisions.clear();
        self.redo_revisions.clear();
    }

    pub fn info(&self) -> HistoryInfo {
        HistoryInfo {
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            undo_label: self.undo.last().map(|t| t.label.clone()),
            redo_label: self.redo.last().map(|t| t.label.clone()),
            revision: self.revision,
            dirty: self.is_dirty(),
        }
    }

    /// Labels of the undo stack, most recent last.
    pub fn labels(&self) -> Vec<String> {
        self.undo.iter().map(|t| t.label.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventKind};

    fn mv(x: i32) -> EventKind {
        EventKind::MouseMove { x, y: 0 }
    }

    #[test]
    fn undo_redo_restores_exact_state() {
        let mut tl = Timeline::from_kinds([(0, mv(0)), (10, mv(1)), (20, mv(2))]);
        let original = tl.events().to_vec();
        let mut h = History::default();
        assert!(!h.is_dirty());

        let e1 = *tl.get(1).unwrap();
        h.commit(&mut tl, "move", Change { removed: vec![e1], added: vec![Event { t: 25, ..e1 }] }).unwrap();
        let after_first = tl.events().to_vec();
        let e0 = *tl.get(0).unwrap();
        h.commit(&mut tl, "delete", Change { removed: vec![e0], added: vec![] }).unwrap();
        assert!(h.is_dirty());
        assert_eq!(tl.len(), 2);

        assert_eq!(h.undo(&mut tl).unwrap().as_deref(), Some("delete"));
        assert_eq!(tl.events(), &after_first[..]);
        assert_eq!(h.undo(&mut tl).unwrap().as_deref(), Some("move"));
        assert_eq!(tl.events(), &original[..]);
        assert!(!h.is_dirty());
        assert_eq!(h.undo(&mut tl).unwrap(), None);

        assert_eq!(h.redo(&mut tl).unwrap().as_deref(), Some("move"));
        assert_eq!(tl.events(), &after_first[..]);
        assert!(h.is_dirty());
        tl.check_invariants().unwrap();
    }

    #[test]
    fn saved_marker_tracks_position() {
        let mut tl = Timeline::from_kinds([(0, mv(0))]);
        let mut h = History::default();
        let e = *tl.get(0).unwrap();
        h.commit(&mut tl, "a", Change { removed: vec![e], added: vec![Event { t: 5, ..e }] }).unwrap();
        h.mark_saved();
        assert!(!h.is_dirty());
        h.undo(&mut tl).unwrap();
        assert!(h.is_dirty());
        h.redo(&mut tl).unwrap();
        assert!(!h.is_dirty());
        // A new edit after undo never collides with the saved revision.
        h.undo(&mut tl).unwrap();
        let e = *tl.get(0).unwrap();
        h.commit(&mut tl, "b", Change { removed: vec![e], added: vec![Event { t: 7, ..e }] }).unwrap();
        assert!(h.is_dirty());
    }

    #[test]
    fn budget_drops_oldest_entries() {
        let mut tl = Timeline::from_kinds((0..10).map(|i| (i * 10, mv(i as i32))));
        let mut h = History::with_budget(4);
        for _ in 0..5 {
            let e = *tl.get(0).unwrap();
            h.commit(&mut tl, "m", Change { removed: vec![e], added: vec![Event { t: e.t + 1, ..e }] }).unwrap();
        }
        assert_eq!(h.labels().len(), 2);
    }
}
