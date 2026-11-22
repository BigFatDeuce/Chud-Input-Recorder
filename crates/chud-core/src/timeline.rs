//! Sorted event container with id lookup and reversible changes.

use std::collections::HashMap;

use crate::event::{Control, Edge, Event, EventId, EventKind, Micros};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error("event {0} does not exist")]
    Missing(EventId),
    #[error("event id {0} is already in use")]
    DuplicateId(EventId),
    #[error("negative timestamp for event {0}")]
    NegativeTime(EventId),
}

/// A reversible modification: remove some events (by id) and insert others.
/// Moving an event is expressed as removing the old version and adding the
/// new one with the same id. `inverse()` undoes the change exactly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub removed: Vec<Event>,
    pub added: Vec<Event>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }

    pub fn inverse(&self) -> Change {
        Change { removed: self.added.clone(), added: self.removed.clone() }
    }

    /// Approximate memory weight (in events) for history budgeting.
    pub fn weight(&self) -> usize {
        self.removed.len() + self.added.len()
    }
}

/// Events sorted by `(t, id)`.
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    events: Vec<Event>,
    /// `index[id] = position in events`, `u32::MAX` for unused ids.
    index: Vec<u32>,
    next_id: EventId,
}

const NO_POS: u32 = u32::MAX;

/// A press matched with its release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hold {
    pub control: Control,
    pub down: usize,
    /// `None` when the control is never released (stuck).
    pub up: Option<usize>,
}

impl Timeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a timeline from timestamped kinds. Ids are assigned in input
    /// order, and a stable sort keeps that order for simultaneous events.
    pub fn from_kinds<I: IntoIterator<Item = (Micros, EventKind)>>(items: I) -> Self {
        let mut events: Vec<Event> = items
            .into_iter()
            .enumerate()
            .map(|(i, (t, kind))| Event::new(i as EventId, t.max(0), kind))
            .collect();
        events.sort_by(Event::order);
        let next_id = events.len() as EventId;
        let mut tl = Self { events, index: Vec::new(), next_id };
        tl.renumber();
        tl
    }

    /// Reassigns ids `0..n` in timeline order (used after loading).
    pub fn renumber(&mut self) {
        for (i, e) in self.events.iter_mut().enumerate() {
            e.id = i as EventId;
        }
        self.next_id = self.events.len() as EventId;
        self.rebuild_index();
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        self.index.resize(self.next_id as usize, NO_POS);
        for (pos, e) in self.events.iter().enumerate() {
            self.index[e.id as usize] = pos as u32;
        }
    }

    #[inline]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Next id that will be handed out.
    #[inline]
    pub fn next_id(&self) -> EventId {
        self.next_id
    }

    /// Timestamp of the last event (0 when empty).
    pub fn duration(&self) -> Micros {
        self.events.last().map(|e| e.t).unwrap_or(0)
    }

    pub fn position_of(&self, id: EventId) -> Option<usize> {
        match self.index.get(id as usize) {
            Some(&p) if p != NO_POS => Some(p as usize),
            _ => None,
        }
    }

    pub fn get(&self, id: EventId) -> Option<&Event> {
        self.position_of(id).map(|p| &self.events[p])
    }

    /// Events whose ids are in `ids`, in timeline order. Unknown ids are skipped.
    pub fn select(&self, ids: &[EventId]) -> Vec<Event> {
        let mut pos: Vec<usize> = ids.iter().filter_map(|&id| self.position_of(id)).collect();
        pos.sort_unstable();
        pos.dedup();
        pos.into_iter().map(|p| self.events[p]).collect()
    }

    /// Index range of events with `t0 <= t < t1`.
    pub fn range(&self, t0: Micros, t1: Micros) -> std::ops::Range<usize> {
        let a = self.events.partition_point(|e| e.t < t0);
        let b = self.events.partition_point(|e| e.t < t1);
        a..b.max(a)
    }

    /// Applies a change. On error the timeline is left untouched.
    pub fn apply(&mut self, change: &Change) -> Result<(), TimelineError> {
        if change.is_empty() {
            return Ok(());
        }
        // Validate first so that a failing change has no effect.
        let mut removing = vec![false; self.next_id as usize];
        for e in &change.removed {
            if self.position_of(e.id).is_none() {
                return Err(TimelineError::Missing(e.id));
            }
            removing[e.id as usize] = true;
        }
        let mut seen_added = HashMap::with_capacity(change.added.len());
        for e in &change.added {
            if e.t < 0 {
                return Err(TimelineError::NegativeTime(e.id));
            }
            let in_use = self.position_of(e.id).is_some() && !removing[e.id as usize];
            if in_use || seen_added.insert(e.id, ()).is_some() {
                return Err(TimelineError::DuplicateId(e.id));
            }
        }

        if !change.removed.is_empty() {
            self.events.retain(|e| !removing[e.id as usize]);
        }
        if !change.added.is_empty() {
            let mut added = change.added.clone();
            added.sort_by(Event::order);
            self.events = merge_sorted(std::mem::take(&mut self.events), added);
            let max_id = change.added.iter().map(|e| e.id).max().unwrap_or(0);
            self.next_id = self.next_id.max(max_id + 1);
        }
        self.rebuild_index();
        Ok(())
    }

    /// Matches every press with its release. Auto-repeat presses belong to the
    /// hold they repeat. Releases without a press are ignored.
    pub fn holds(&self) -> Vec<Hold> {
        let mut open: HashMap<Control, usize> = HashMap::new();
        let mut out = Vec::new();
        for (i, e) in self.events.iter().enumerate() {
            let Some((control, edge)) = e.kind.control() else { continue };
            match edge {
                Edge::Down => {
                    if open.contains_key(&control) {
                        continue; // repeat or duplicated press: same hold
                    }
                    open.insert(control, i);
                }
                Edge::Up => {
                    if let Some(down) = open.remove(&control) {
                        out.push(Hold { control, down, up: Some(i) });
                    }
                }
            }
        }
        for (control, down) in open {
            out.push(Hold { control, down, up: None });
        }
        out.sort_by_key(|h| h.down);
        out
    }

    /// Positions of the partner of each press/release (`partner[i]`).
    pub fn partners(&self) -> Vec<Option<usize>> {
        let mut out = vec![None; self.events.len()];
        for h in self.holds() {
            if let Some(up) = h.up {
                out[h.down] = Some(up);
                out[up] = Some(h.down);
            }
        }
        out
    }

    /// Controls held at time `t` (pressed strictly before `t`, released at or after).
    pub fn held_at(&self, t: Micros) -> Vec<usize> {
        self.holds()
            .into_iter()
            .filter(|h| {
                self.events[h.down].t < t && h.up.map(|u| self.events[u].t >= t).unwrap_or(true)
            })
            .map(|h| h.down)
            .collect()
    }

    /// Debug check of internal invariants (used by tests).
    pub fn check_invariants(&self) -> Result<(), String> {
        for w in self.events.windows(2) {
            if Event::order(&w[0], &w[1]) != std::cmp::Ordering::Less {
                return Err(format!("unsorted at ids {} / {}", w[0].id, w[1].id));
            }
        }
        for (pos, e) in self.events.iter().enumerate() {
            if self.position_of(e.id) != Some(pos) {
                return Err(format!("index mismatch for id {}", e.id));
            }
            if e.id >= self.next_id {
                return Err(format!("id {} >= next_id {}", e.id, self.next_id));
            }
        }
        Ok(())
    }
}

fn merge_sorted(a: Vec<Event>, b: Vec<Event>) -> Vec<Event> {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if Event::order(&a[i], &b[j]) != std::cmp::Ordering::Greater {
            out.push(a[i]);
            i += 1;
        } else {
            out.push(b[j]);
            j += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, MouseButton};

    fn kd(scan: u16) -> EventKind {
        EventKind::KeyDown { key: Key::new(scan, 0), repeat: false }
    }
    fn ku(scan: u16) -> EventKind {
        EventKind::KeyUp { key: Key::new(scan, 0) }
    }

    fn sample() -> Timeline {
        Timeline::from_kinds([
            (0, kd(0x1E)),
            (50, EventKind::MouseMove { x: 1, y: 1 }),
            (100, ku(0x1E)),
            (100, kd(0x1F)),
            (300, ku(0x1F)),
        ])
    }

    #[test]
    fn build_and_lookup() {
        let tl = sample();
        tl.check_invariants().unwrap();
        assert_eq!(tl.len(), 5);
        assert_eq!(tl.duration(), 300);
        assert_eq!(tl.get(3).unwrap().kind, kd(0x1F));
        assert_eq!(tl.range(50, 101), 1..4);
    }

    #[test]
    fn apply_and_inverse_roundtrip() {
        let mut tl = sample();
        let before: Vec<Event> = tl.events().to_vec();
        let moved = Event { t: 400, ..*tl.get(0).unwrap() };
        let change = Change {
            removed: vec![*tl.get(0).unwrap(), *tl.get(1).unwrap()],
            added: vec![moved, Event::new(tl.next_id(), 10, EventKind::Wheel { delta: 120, horizontal: false, x: 0, y: 0 })],
        };
        tl.apply(&change).unwrap();
        tl.check_invariants().unwrap();
        assert_eq!(tl.events().last().unwrap().id, 0);
        assert_eq!(tl.events()[0].t, 10);
        tl.apply(&change.inverse()).unwrap();
        tl.check_invariants().unwrap();
        assert_eq!(tl.events(), &before[..]);
    }

    #[test]
    fn failing_change_has_no_effect() {
        let mut tl = sample();
        let before = tl.events().to_vec();
        let bad = Change { removed: vec![Event::new(99, 0, kd(1))], added: vec![] };
        assert_eq!(tl.apply(&bad), Err(TimelineError::Missing(99)));
        let dup = Change { removed: vec![], added: vec![Event::new(2, 5, kd(1))] };
        assert_eq!(tl.apply(&dup), Err(TimelineError::DuplicateId(2)));
        assert_eq!(tl.events(), &before[..]);
    }

    #[test]
    fn holds_pair_presses_and_skip_repeats() {
        let tl = Timeline::from_kinds([
            (0, kd(0x1E)),
            (30, EventKind::KeyDown { key: Key::new(0x1E, 0), repeat: true }),
            (60, ku(0x1E)),
            (70, ku(0x30)), // orphan release
            (80, EventKind::MouseDown { button: MouseButton::Left, x: 0, y: 0 }),
        ]);
        let holds = tl.holds();
        assert_eq!(holds.len(), 2);
        assert_eq!(holds[0].down, 0);
        assert_eq!(holds[0].up, Some(2));
        assert_eq!(holds[1].up, None);
        assert_eq!(tl.held_at(65), Vec::<usize>::new());
        assert_eq!(tl.held_at(90), vec![4]);
    }
}
