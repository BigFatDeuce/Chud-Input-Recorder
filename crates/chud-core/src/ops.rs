//! Editing operations. Each operation inspects a [`Timeline`] and returns a
//! [`Change`]; nothing is mutated here, which keeps every operation pure,
//! testable and undoable.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::event::{Event, EventId, EventKind, Micros};
use crate::timeline::{Change, Timeline};

/// Copied events with times relative to the first one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub items: Vec<(Micros, EventKind)>,
}

impl Clip {
    pub fn duration(&self) -> Micros {
        self.items.last().map(|(t, _)| *t).unwrap_or(0)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Adds the press/release partner of every selected press/release.
pub fn with_partners(tl: &Timeline, ids: &[EventId]) -> Vec<EventId> {
    let partners = tl.partners();
    let mut set: HashSet<EventId> = ids.iter().copied().collect();
    for &id in ids {
        if let Some(pos) = tl.position_of(id)
            && let Some(p) = partners[pos] {
                set.insert(tl.events()[p].id);
            }
    }
    let mut out: Vec<EventId> = set.into_iter().collect();
    out.sort_unstable();
    out
}

pub fn delete(tl: &Timeline, ids: &[EventId], partners: bool) -> Change {
    let ids = if partners { with_partners(tl, ids) } else { ids.to_vec() };
    Change { removed: tl.select(&ids), added: vec![] }
}

/// Inserts new events; returns the change and the new ids.
pub fn insert(tl: &Timeline, items: &[(Micros, EventKind)]) -> (Change, Vec<EventId>) {
    let base = tl.next_id();
    let added: Vec<Event> = items
        .iter()
        .enumerate()
        .map(|(i, (t, k))| Event::new(base + i as EventId, (*t).max(0), *k))
        .collect();
    let ids = added.iter().map(|e| e.id).collect();
    (Change { removed: vec![], added }, ids)
}

/// Replaces events with new versions (same ids).
pub fn update(tl: &Timeline, edits: &[(EventId, Micros, EventKind)]) -> Change {
    let mut change = Change::default();
    for &(id, t, kind) in edits {
        if let Some(old) = tl.get(id) {
            let new = Event::new(id, t.max(0), kind);
            if new != *old {
                change.removed.push(*old);
                change.added.push(new);
            }
        }
    }
    change
}

fn remap(tl: &Timeline, ids: &[EventId], f: impl Fn(&Event) -> Event) -> Change {
    let mut change = Change::default();
    for old in tl.select(ids) {
        let mut new = f(&old);
        new.t = new.t.max(0);
        if new != old {
            change.removed.push(old);
            change.added.push(new);
        }
    }
    change
}

/// Moves events in time. The block is clamped so that nothing goes below 0,
/// preserving relative timing inside the selection.
pub fn shift(tl: &Timeline, ids: &[EventId], dt: Micros) -> Change {
    let sel = tl.select(ids);
    let Some(min_t) = sel.iter().map(|e| e.t).min() else { return Change::default() };
    let dt = dt.max(-min_t);
    remap(tl, ids, |e| Event { t: e.t + dt, ..*e })
}

/// Moves pointer positions of events in space (mouse paths, clicks, wheels).
pub fn translate(tl: &Timeline, ids: &[EventId], dx: i32, dy: i32) -> Change {
    remap(tl, ids, |e| match e.kind.position() {
        Some((x, y)) => Event { kind: e.kind.with_position(x + dx, y + dy), ..*e },
        None => *e,
    })
}

/// Scales times around `anchor` (factor 2.0 = twice as slow).
pub fn scale(tl: &Timeline, ids: &[EventId], anchor: Micros, factor: f64) -> Change {
    if !(factor.is_finite() && factor > 0.0) {
        return Change::default();
    }
    remap(tl, ids, |e| {
        let t = anchor as f64 + (e.t - anchor) as f64 * factor;
        Event { t: t.round() as Micros, ..*e }
    })
}

/// Snaps times to a grid. `strength` in `0..=1` (1 = hard snap).
pub fn quantize(tl: &Timeline, ids: &[EventId], grid: Micros, strength: f64) -> Change {
    if grid <= 0 {
        return Change::default();
    }
    let s = strength.clamp(0.0, 1.0);
    remap(tl, ids, |e| {
        let snapped = ((e.t as f64 / grid as f64).round() as Micros) * grid;
        let t = e.t as f64 + (snapped - e.t) as f64 * s;
        Event { t: t.round() as Micros, ..*e }
    })
}

/// Makes every interval between consecutive selected events equal to `gap`,
/// keeping the first event in place. Simultaneous events (closer than
/// `simultaneous`) stay together.
pub fn set_gaps(tl: &Timeline, ids: &[EventId], gap: Micros, simultaneous: Micros) -> Change {
    let sel = tl.select(ids);
    let Some(first) = sel.first() else { return Change::default() };
    let mut edits = Vec::with_capacity(sel.len());
    let mut t = first.t;
    let mut prev_orig = first.t;
    for (i, e) in sel.iter().enumerate() {
        if i > 0 && e.t - prev_orig > simultaneous {
            t += gap.max(0);
        }
        prev_orig = e.t;
        edits.push((e.id, t, e.kind));
    }
    update(tl, &edits)
}

/// Changes how long a press is held by moving its release.
pub fn set_hold_duration(tl: &Timeline, press_id: EventId, duration: Micros) -> Change {
    let Some(pos) = tl.position_of(press_id) else { return Change::default() };
    let partners = tl.partners();
    let Some(up_pos) = partners[pos] else { return Change::default() };
    if up_pos < pos {
        return Change::default(); // the id given is a release
    }
    let down = tl.events()[pos];
    let up = tl.events()[up_pos];
    update(tl, &[(up.id, down.t + duration.max(0), up.kind)])
}

pub fn copy(tl: &Timeline, ids: &[EventId]) -> Clip {
    let sel = tl.select(ids);
    let Some(t0) = sel.first().map(|e| e.t) else { return Clip::default() };
    Clip { items: sel.iter().map(|e| (e.t - t0, e.kind)).collect() }
}

pub fn paste(tl: &Timeline, clip: &Clip, at: Micros) -> (Change, Vec<EventId>) {
    let items: Vec<(Micros, EventKind)> = clip.items.iter().map(|(t, k)| (at + t, *k)).collect();
    insert(tl, &items)
}

/// Duplicates events. With `offset = None` the copy starts right after the
/// selection ends (selection span + 1 ms), keeping internal timing.
pub fn duplicate(tl: &Timeline, ids: &[EventId], offset: Option<Micros>) -> (Change, Vec<EventId>) {
    let clip = copy(tl, ids);
    let sel = tl.select(ids);
    let Some(first) = sel.first() else { return (Change::default(), vec![]) };
    let at = match offset {
        Some(o) => first.t + o,
        None => first.t + clip.duration() + 1_000,
    };
    paste(tl, &clip, at)
}

/// Inserts silence at `at`: every event at or after `at` moves later by `dt`.
pub fn insert_time(tl: &Timeline, at: Micros, dt: Micros) -> Change {
    if dt <= 0 {
        return Change::default();
    }
    let ids: Vec<EventId> = tl.events()[tl.range(at, Micros::MAX)].iter().map(|e| e.id).collect();
    remap(tl, &ids, |e| Event { t: e.t + dt, ..*e })
}

/// Removes everything in `[t0, t1)`. Holds crossing the boundaries are
/// trimmed instead of left stuck. With `ripple`, later events move left to
/// close the gap.
pub fn delete_range(tl: &Timeline, t0: Micros, t1: Micros, ripple: bool) -> Change {
    if t1 <= t0 {
        return Change::default();
    }
    let events = tl.events();
    let partners = tl.partners();
    let width = t1 - t0;
    let mut removed = Vec::new();
    let mut edits: Vec<(EventId, Micros, EventKind)> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let inside = e.t >= t0 && e.t < t1;
        let partner = partners[i].map(|p| events[p]);
        if inside {
            match partner {
                // Release inside, press before: keep the release, at t0.
                Some(p) if p.t < t0 && e.kind.control().map(|c| c.1) == Some(crate::event::Edge::Up) => {
                    edits.push((e.id, t0, e.kind));
                }
                // Press inside, release after: keep the press, at t1 (or t0 when rippling).
                Some(p) if p.t >= t1 && e.kind.control().map(|c| c.1) == Some(crate::event::Edge::Down) => {
                    edits.push((e.id, if ripple { t0 } else { t1 }, e.kind));
                }
                _ => removed.push(*e),
            }
        } else if ripple && e.t >= t1 {
            edits.push((e.id, e.t - width, e.kind));
        }
    }
    let mut change = update(tl, &edits);
    change.removed.extend(removed);
    change
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, MouseButton};

    fn kd(s: u16) -> EventKind {
        EventKind::KeyDown { key: Key::new(s, 0), repeat: false }
    }
    fn ku(s: u16) -> EventKind {
        EventKind::KeyUp { key: Key::new(s, 0) }
    }
    fn apply(tl: &mut Timeline, f: impl FnOnce(&Timeline) -> Change) {
        let c = f(tl);
        tl.apply(&c).unwrap();
        tl.check_invariants().unwrap();
    }

    #[test]
    fn delete_with_partners_removes_whole_hold() {
        let mut tl = Timeline::from_kinds([(0, kd(1)), (10, ku(1)), (20, kd(2))]);
        apply(&mut tl, |t| delete(t, &[1], true));
        assert_eq!(tl.len(), 1);
        assert_eq!(tl.events()[0].kind, kd(2));
    }

    #[test]
    fn shift_clamps_block_at_zero() {
        let mut tl = Timeline::from_kinds([(100, kd(1)), (300, ku(1))]);
        apply(&mut tl, |t| shift(t, &[0, 1], -500));
        assert_eq!(tl.events()[0].t, 0);
        assert_eq!(tl.events()[1].t, 200);
    }

    #[test]
    fn duplicate_keeps_internal_timing() {
        let mut tl = Timeline::from_kinds([(0, kd(1)), (50, ku(1))]);
        let (c, ids) = duplicate(&tl, &[0, 1], None);
        apply(&mut tl, |_| c);
        assert_eq!(ids.len(), 2);
        let copies = tl.select(&ids);
        assert_eq!(copies[0].t, 1_050);
        assert_eq!(copies[1].t - copies[0].t, 50);
    }

    #[test]
    fn scale_and_quantize() {
        let mut tl = Timeline::from_kinds([(1_000, kd(1)), (3_000, ku(1))]);
        apply(&mut tl, |t| scale(t, &[0, 1], 1_000, 2.0));
        assert_eq!(tl.events()[1].t, 5_000);
        apply(&mut tl, |t| quantize(t, &[0, 1], 4_000, 1.0));
        assert_eq!(tl.events()[0].t, 0);
        assert_eq!(tl.events()[1].t, 4_000);
    }

    #[test]
    fn set_gaps_keeps_simultaneous_groups() {
        let mut tl = Timeline::from_kinds([(0, kd(1)), (0, kd(2)), (900, ku(1)), (5_000, ku(2))]);
        apply(&mut tl, |t| set_gaps(t, &[0, 1, 2, 3], 100, 10));
        let ts: Vec<_> = tl.events().iter().map(|e| e.t).collect();
        assert_eq!(ts, vec![0, 0, 100, 200]);
    }

    #[test]
    fn hold_duration_moves_release() {
        let mut tl = Timeline::from_kinds([(0, kd(1)), (50, ku(1))]);
        apply(&mut tl, |t| set_hold_duration(t, 0, 2_000));
        assert_eq!(tl.get(1).unwrap().t, 2_000);
        assert!(set_hold_duration(&tl, 1, 10).is_empty());
    }

    #[test]
    fn delete_range_trims_crossing_holds() {
        // A held across [100, 200): press at 50, release at 150.
        // B pressed at 120, released at 300.
        let mut tl = Timeline::from_kinds([
            (50, kd(1)),
            (120, kd(2)),
            (130, EventKind::MouseMove { x: 1, y: 1 }),
            (150, ku(1)),
            (300, ku(2)),
        ]);
        apply(&mut tl, |t| delete_range(t, 100, 200, true));
        let got: Vec<_> = tl.events().iter().map(|e| (e.t, e.kind)).collect();
        assert_eq!(got, vec![(50, kd(1)), (100, kd(2)), (100, ku(1)), (200, ku(2))]);
        assert!(tl.holds().iter().all(|h| h.up.is_some()));
    }

    #[test]
    fn translate_moves_positions_only() {
        let mut tl = Timeline::from_kinds([
            (0, EventKind::MouseDown { button: MouseButton::Left, x: 10, y: 10 }),
            (5, kd(1)),
        ]);
        apply(&mut tl, |t| translate(t, &[0, 1], 5, -3));
        assert_eq!(tl.events()[0].kind.position(), Some((15, 7)));
        assert_eq!(tl.events()[1].kind, kd(1));
    }

    #[test]
    fn insert_time_shifts_tail() {
        let mut tl = Timeline::from_kinds([(0, kd(1)), (100, ku(1))]);
        apply(&mut tl, |t| insert_time(t, 50, 1_000));
        assert_eq!(tl.events()[1].t, 1_100);
    }
}
