//! "Smart" editing tools. Like `ops`, each tool returns a [`Change`] so the
//! result is previewable and undoable.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::event::{Category, Control, Edge, Event, EventId, EventKind, Micros};
use crate::ops::update;
use crate::timeline::{Change, Timeline};

/// Restricts a tool to part of the timeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    /// Only events with these ids (all events when `None`).
    pub ids: Option<Vec<EventId>>,
}

impl Scope {
    pub fn all() -> Self {
        Self { ids: None }
    }
    fn mask(&self, tl: &Timeline) -> Vec<bool> {
        match &self.ids {
            None => vec![true; tl.len()],
            Some(ids) => {
                let mut m = vec![false; tl.len()];
                for &id in ids {
                    if let Some(p) = tl.position_of(id) {
                        m[p] = true;
                    }
                }
                m
            }
        }
    }
}

/// Compresses every pause longer than `threshold` down to `keep`.
/// With `respect_holds`, pauses during which a key/button is held are left
/// alone (holding "W" for five seconds is meaningful, not dead time).
pub fn remove_dead_time(tl: &Timeline, threshold: Micros, keep: Micros, respect_holds: bool) -> Change {
    let keep = keep.clamp(0, threshold.max(0));
    let mut held: HashSet<Control> = HashSet::new();
    let mut shift: Micros = 0;
    let mut prev_t: Micros = 0;
    let mut edits = Vec::new();
    for e in tl.events() {
        let gap = e.t - prev_t;
        if gap > threshold && !(respect_holds && !held.is_empty()) {
            shift += gap - keep;
        }
        prev_t = e.t;
        if shift > 0 {
            edits.push((e.id, e.t - shift, e.kind));
        }
        if let Some((c, edge)) = e.kind.control() {
            match edge {
                Edge::Down => {
                    held.insert(c);
                }
                Edge::Up => {
                    held.remove(&c);
                }
            }
        }
    }
    update(tl, &edits)
}

/// Ramer–Douglas–Peucker on pointer paths. Every run of consecutive absolute
/// moves (runs are broken by clicks and wheel events, which anchor the path)
/// is reduced to the points needed to stay within `epsilon` pixels of the
/// original trajectory. A point is also kept at least every `max_interval`
/// so that motion stays smooth during playback.
pub fn simplify_mouse(tl: &Timeline, scope: &Scope, epsilon: f64, max_interval: Micros) -> Change {
    let mask = scope.mask(tl);
    let events = tl.events();
    let mut removed = Vec::new();
    let mut run: Vec<usize> = Vec::new();

    let flush = |run: &mut Vec<usize>, removed: &mut Vec<Event>| {
        if run.len() > 2 {
            let pts: Vec<(f64, f64)> = run
                .iter()
                .map(|&i| {
                    let (x, y) = events[i].kind.position().unwrap();
                    (x as f64, y as f64)
                })
                .collect();
            let mut keep = vec![false; pts.len()];
            rdp(&pts, 0, pts.len() - 1, epsilon.max(0.0), &mut keep);
            // Enforce a maximum interval between kept points.
            if max_interval > 0 {
                let mut last_kept_t = events[run[0]].t;
                for k in 1..run.len() {
                    let t = events[run[k]].t;
                    if keep[k] {
                        last_kept_t = t;
                    } else if t - last_kept_t >= max_interval {
                        keep[k] = true;
                        last_kept_t = t;
                    }
                }
            }
            for (k, &i) in run.iter().enumerate() {
                if !keep[k] {
                    removed.push(events[i]);
                }
            }
        }
        run.clear();
    };

    for (i, e) in events.iter().enumerate() {
        match e.kind.category() {
            Category::MouseMove if mask[i] => run.push(i),
            Category::MouseButton | Category::Wheel => flush(&mut run, &mut removed),
            Category::MouseMove => flush(&mut run, &mut removed), // out of scope breaks the run
            _ => {}
        }
    }
    flush(&mut run, &mut removed);
    Change { removed, added: vec![] }
}

fn rdp(pts: &[(f64, f64)], a: usize, b: usize, eps: f64, keep: &mut [bool]) {
    keep[a] = true;
    keep[b] = true;
    if b <= a + 1 {
        return;
    }
    let (ax, ay) = pts[a];
    let (bx, by) = pts[b];
    let (dx, dy) = (bx - ax, by - ay);
    let len = (dx * dx + dy * dy).sqrt();
    let mut best = (0.0, a);
    for (i, &(px, py)) in pts.iter().enumerate().take(b).skip(a + 1) {
        let d = if len < 1e-9 {
            ((px - ax).powi(2) + (py - ay).powi(2)).sqrt()
        } else {
            ((px - ax) * dy - (py - ay) * dx).abs() / len
        };
        if d > best.0 {
            best = (d, i);
        }
    }
    if best.0 > eps {
        rdp(pts, a, best.1, eps, keep);
        rdp(pts, best.1, b, eps, keep);
    }
}

/// Merges raw mouse deltas into buckets of `interval`, summing the deltas so
/// that the total displacement is preserved exactly.
pub fn merge_raw_motion(tl: &Timeline, scope: &Scope, interval: Micros) -> Change {
    let mask = scope.mask(tl);
    let mut removed = Vec::new();
    let mut edits = Vec::new();
    let mut bucket: Option<(Event, i32, i32)> = None;
    let close = |bucket: &mut Option<(Event, i32, i32)>, edits: &mut Vec<(EventId, Micros, EventKind)>| {
        if let Some((first, dx, dy)) = bucket.take() {
            let k = EventKind::MouseRaw { dx, dy };
            if k != first.kind {
                edits.push((first.id, first.t, k));
            }
        }
    };
    for (i, e) in tl.events().iter().enumerate() {
        let EventKind::MouseRaw { dx, dy } = e.kind else {
            if matches!(e.kind.category(), Category::MouseButton | Category::Wheel) {
                close(&mut bucket, &mut edits);
            }
            continue;
        };
        if !mask[i] {
            close(&mut bucket, &mut edits);
            continue;
        }
        match &mut bucket {
            Some((first, sx, sy)) if e.t - first.t < interval => {
                *sx += dx;
                *sy += dy;
                removed.push(*e);
            }
            _ => {
                close(&mut bucket, &mut edits);
                bucket = Some((*e, dx, dy));
            }
        }
    }
    close(&mut bucket, &mut edits);
    let mut change = update(tl, &edits);
    change.removed.extend(removed);
    change
}

/// Merges consecutive wheel events on the same axis and in the same
/// direction occurring within `window`.
pub fn merge_wheel(tl: &Timeline, window: Micros) -> Change {
    let mut removed = Vec::new();
    let mut edits = Vec::new();
    let mut acc: Option<(Event, i32)> = None;
    let flush = |acc: &mut Option<(Event, i32)>, edits: &mut Vec<(EventId, Micros, EventKind)>| {
        if let Some((first, sum)) = acc.take()
            && let EventKind::Wheel { horizontal, x, y, delta } = first.kind
                && delta != sum {
                    edits.push((first.id, first.t, EventKind::Wheel { delta: sum, horizontal, x, y }));
                }
    };
    let mut last_t = 0;
    for e in tl.events() {
        match e.kind {
            EventKind::Wheel { delta, horizontal, .. } => {
                let same = match &acc {
                    Some((first, sum)) => {
                        matches!(first.kind, EventKind::Wheel { horizontal: h, .. } if h == horizontal)
                            && sum.signum() == delta.signum()
                            && e.t - last_t <= window
                    }
                    None => false,
                };
                if same {
                    if let Some((_, sum)) = &mut acc {
                        *sum += delta;
                    }
                    removed.push(*e);
                } else {
                    flush(&mut acc, &mut edits);
                    acc = Some((*e, delta));
                }
                last_t = e.t;
            }
            EventKind::MouseMove { .. } | EventKind::MouseRaw { .. } => {}
            _ => flush(&mut acc, &mut edits),
        }
    }
    flush(&mut acc, &mut edits);
    let mut change = update(tl, &edits);
    change.removed.extend(removed);
    change
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CleanupOptions {
    /// Remove releases that have no matching press.
    pub orphan_releases: bool,
    /// Add a release at the end for every press that is never released.
    pub close_stuck: bool,
    /// Remove keyboard auto-repeat presses.
    pub strip_repeats: bool,
    /// Remove pointer moves that do not change the position, and zero raw moves.
    pub redundant_moves: bool,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self { orphan_releases: true, close_stuck: true, strip_repeats: false, redundant_moves: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanupReport {
    pub orphan_releases: usize,
    pub closed_stuck: usize,
    pub repeats: usize,
    pub redundant_moves: usize,
}

impl CleanupReport {
    pub fn total(&self) -> usize {
        self.orphan_releases + self.closed_stuck + self.repeats + self.redundant_moves
    }
}

pub fn cleanup(tl: &Timeline, opts: CleanupOptions) -> (Change, CleanupReport) {
    let mut report = CleanupReport::default();
    let mut change = Change::default();
    let events = tl.events();
    let mut paired = vec![false; events.len()];
    let holds = tl.holds();
    for h in &holds {
        paired[h.down] = true;
        if let Some(u) = h.up {
            paired[u] = true;
        }
    }
    let mut held: HashMap<Control, ()> = HashMap::new();
    let mut last_pos: Option<(i32, i32)> = None;
    for (i, e) in events.iter().enumerate() {
        if let Some((c, edge)) = e.kind.control() {
            match edge {
                Edge::Up if !paired[i] && opts.orphan_releases => {
                    change.removed.push(*e);
                    report.orphan_releases += 1;
                    continue;
                }
                Edge::Down if e.kind.is_repeat() && opts.strip_repeats => {
                    change.removed.push(*e);
                    report.repeats += 1;
                    continue;
                }
                Edge::Down => {
                    held.insert(c, ());
                }
                Edge::Up => {
                    held.remove(&c);
                }
            }
        }
        match e.kind {
            EventKind::MouseMove { x, y } => {
                if opts.redundant_moves && last_pos == Some((x, y)) {
                    change.removed.push(*e);
                    report.redundant_moves += 1;
                    continue;
                }
                last_pos = Some((x, y));
            }
            EventKind::MouseRaw { dx: 0, dy: 0 } if opts.redundant_moves => {
                change.removed.push(*e);
                report.redundant_moves += 1;
                continue;
            }
            ref k => {
                if let Some(p) = k.position() {
                    last_pos = Some(p);
                }
            }
        }
    }
    if opts.close_stuck {
        let end = tl.duration() + 1_000;
        let stuck: Vec<(Micros, EventKind)> = holds
            .iter()
            .filter(|h| h.up.is_none())
            .filter_map(|h| events[h.down].kind.release_for().map(|k| (end, k)))
            .collect();
        report.closed_stuck = stuck.len();
        let (c, _) = crate::ops::insert(tl, &stuck);
        change.added.extend(c.added);
    }
    (change, report)
}

/// Replaces real timing by a uniform `gap` between consecutive events.
/// Used for "record without intervals".
pub fn uniform_timing(tl: &Timeline, gap: Micros, simultaneous: Micros) -> Change {
    let ids: Vec<EventId> = tl.events().iter().map(|e| e.id).collect();
    crate::ops::set_gaps(tl, &ids, gap, simultaneous)
}

/// A pattern of discrete actions that occurs several times.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Repetition {
    /// Number of discrete actions in one occurrence.
    pub length: usize,
    /// `(start, end)` time of each non-overlapping occurrence.
    pub occurrences: Vec<(Micros, Micros)>,
    /// Ids of the events (all kinds) inside the first occurrence.
    pub first_ids: Vec<EventId>,
    /// Short description such as "Ctrl C Ctrl V".
    pub summary: String,
}

/// Finds repeated sequences of discrete actions (presses, releases, wheel).
/// Pointer motion and timing are ignored when comparing, so a loop recorded
/// by hand is detected even if it was not performed at a constant pace.
pub fn detect_repetitions(tl: &Timeline, min_len: usize, max_results: usize) -> Vec<Repetition> {
    let events = tl.events();
    let tokens: Vec<(usize, u64)> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| token(&e.kind).map(|t| (i, t)))
        .collect();
    let n = tokens.len();
    let min_len = min_len.max(2);
    if n < min_len * 2 {
        return vec![];
    }
    let max_len = (n / 2).min(96);
    let mut out = Vec::new();

    // Polynomial prefix hashes (mod 2^64) give O(1) window hashes; equality is
    // always verified token by token, so collisions cannot cause false results.
    const B: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut prefix = vec![0u64; n + 1];
    let mut pow = vec![1u64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i].wrapping_mul(B).wrapping_add(tokens[i].1);
        pow[i + 1] = pow[i].wrapping_mul(B);
    }
    let window = |s: usize, len: usize| prefix[s + len].wrapping_sub(prefix[s].wrapping_mul(pow[len]));

    // Phase 1: candidate patterns of every length.
    let mut candidates: Vec<(usize, Vec<usize>)> = Vec::new();
    for len in min_len..=max_len {
        let mut groups: HashMap<u64, Vec<usize>> = HashMap::new();
        for start in 0..=n - len {
            groups.entry(window(start, len)).or_default().push(start);
        }
        let mut found: Vec<Vec<usize>> = groups
            .into_values()
            .filter(|v| v.len() >= 2)
            .map(|starts| {
                // Greedy non-overlapping occurrences, verified token by token.
                let mut occ: Vec<usize> = Vec::new();
                for s in starts {
                    let overlaps = occ.last().is_some_and(|&p| s < p + len);
                    let same = occ.first().is_none_or(|&f| (0..len).all(|k| tokens[f + k].1 == tokens[s + k].1));
                    if !overlaps && same {
                        occ.push(s);
                    }
                }
                occ
            })
            .filter(|occ| occ.len() >= 2)
            // A pattern made of a shorter repeated unit (ABAB) is reported as
            // that unit instead.
            .filter(|occ| !is_periodic(&tokens[occ[0]..occ[0] + len]))
            .collect();
        found.sort_by_key(|occ| std::cmp::Reverse(occ.len()));
        found.truncate(64);
        candidates.extend(found.into_iter().map(|occ| (len, occ)));
    }

    // Phase 2: keep the patterns explaining the most actions, without overlap.
    candidates.sort_by_key(|(len, occ)| (std::cmp::Reverse(len * occ.len()), std::cmp::Reverse(occ.len())));
    let mut covered = vec![false; n];
    for (len, occ) in candidates {
        let free: Vec<usize> = occ.into_iter().filter(|&s| !(s..s + len).any(|k| covered[k])).collect();
        if free.len() < 2 {
            continue;
        }
        for &s in &free {
            for c in covered.iter_mut().skip(s).take(len) {
                *c = true;
            }
        }
        let occurrences: Vec<(Micros, Micros)> =
            free.iter().map(|&s| (events[tokens[s].0].t, events[tokens[s + len - 1].0].t)).collect();
        let (a, b) = (tokens[free[0]].0, tokens[free[0] + len - 1].0);
        let first_ids = events[a..=b].iter().map(|e| e.id).collect();
        let summary = summarize(&events[a..=b]);
        out.push(Repetition { length: len, occurrences, first_ids, summary });
        if out.len() >= max_results {
            break;
        }
    }
    out
}

/// True when the sequence is a whole repetition of a shorter unit.
fn is_periodic(seq: &[(usize, u64)]) -> bool {
    let n = seq.len();
    let mut fail = vec![0usize; n];
    let mut k = 0;
    for i in 1..n {
        while k > 0 && seq[i].1 != seq[k].1 {
            k = fail[k - 1];
        }
        if seq[i].1 == seq[k].1 {
            k += 1;
        }
        fail[i] = k;
    }
    let period = n - fail[n - 1];
    period < n && n.is_multiple_of(period)
}

fn token(k: &EventKind) -> Option<u64> {
    let (c, edge) = match k {
        EventKind::KeyDown { repeat: true, .. } => return None,
        EventKind::Wheel { delta, horizontal, .. } => {
            return Some(0x5700_0000 | ((*horizontal as u64) << 8) | (delta.signum() + 1) as u64);
        }
        _ => k.control()?,
    };
    let base = match c {
        Control::Key(id) => 0x1000_0000_0000 | id as u64,
        Control::Mouse(b) => 0x2000_0000_0000 | b as u64,
        Control::Pad(p, b) => 0x3000_0000_0000 | ((p as u64) << 8) | b.0 as u64,
    };
    Some(base | ((edge == Edge::Down) as u64) << 40)
}

fn summarize(events: &[Event]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for e in events {
        let s = match e.kind {
            EventKind::KeyDown { key, repeat: false } => crate::keys::label(key),
            EventKind::MouseDown { button, .. } => format!("Click {}", button.name()),
            EventKind::Wheel { delta, .. } => if delta > 0 { "Wheel ↑".into() } else { "Wheel ↓".into() },
            EventKind::PadButton { button, down: true, .. } => button.name(),
            _ => continue,
        };
        parts.push(s);
        if parts.len() >= 8 {
            parts.push("…".into());
            break;
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Key, MouseButton};

    fn kd(s: u16) -> EventKind {
        EventKind::KeyDown { key: Key::new(s, 0x41), repeat: false }
    }
    fn ku(s: u16) -> EventKind {
        EventKind::KeyUp { key: Key::new(s, 0x41) }
    }
    fn mv(x: i32, y: i32) -> EventKind {
        EventKind::MouseMove { x, y }
    }
    fn applied(tl: &Timeline, c: Change) -> Timeline {
        let mut t = tl.clone();
        t.apply(&c).unwrap();
        t.check_invariants().unwrap();
        t
    }

    #[test]
    fn dead_time_compressed_but_holds_respected() {
        let tl = Timeline::from_kinds([
            (5_000_000, kd(1)),
            (5_100_000, ku(1)),
            (9_000_000, kd(2)),       // 3.9 s pause before: compressed
            (15_000_000, ku(2)),      // 6 s while held: kept
        ]);
        let out = applied(&tl, remove_dead_time(&tl, 1_000_000, 200_000, true));
        let ts: Vec<_> = out.events().iter().map(|e| e.t).collect();
        assert_eq!(ts, vec![200_000, 300_000, 500_000, 6_500_000]);
        let out2 = applied(&tl, remove_dead_time(&tl, 1_000_000, 200_000, false));
        assert_eq!(out2.events()[3].t, 700_000);
    }

    #[test]
    fn simplify_keeps_corners_and_endpoints() {
        // L-shaped path: straight right then straight down.
        let mut items: Vec<(Micros, EventKind)> = Vec::new();
        for i in 0..=10 {
            items.push((i * 1_000, mv(i as i32 * 10, 0)));
        }
        for i in 1..=10 {
            items.push(((10 + i) * 1_000, mv(100, i as i32 * 10)));
        }
        let tl = Timeline::from_kinds(items);
        let out = applied(&tl, simplify_mouse(&tl, &Scope::all(), 1.0, 0));
        let pts: Vec<_> = out.events().iter().map(|e| e.kind.position().unwrap()).collect();
        assert_eq!(pts, vec![(0, 0), (100, 0), (100, 100)]);
        // A max interval forces intermediate points.
        let out = applied(&tl, simplify_mouse(&tl, &Scope::all(), 1.0, 5_000));
        assert!(out.len() > 3);
    }

    #[test]
    fn clicks_anchor_simplified_paths() {
        let tl = Timeline::from_kinds([
            (0, mv(0, 0)),
            (1, mv(1, 0)),
            (2, mv(2, 0)),
            (3, EventKind::MouseDown { button: MouseButton::Left, x: 2, y: 0 }),
            (4, mv(3, 0)),
            (5, mv(4, 0)),
            (6, mv(5, 0)),
        ]);
        let out = applied(&tl, simplify_mouse(&tl, &Scope::all(), 1.0, 0));
        assert_eq!(out.len(), 5); // (0,0) (2,0) click (3,0) (5,0)
    }

    #[test]
    fn raw_merge_preserves_total_displacement() {
        let tl = Timeline::from_kinds((0..100).map(|i| (i * 125, EventKind::MouseRaw { dx: 1, dy: -2 })));
        let out = applied(&tl, merge_raw_motion(&tl, &Scope::all(), 1_000));
        let (sx, sy) = out.events().iter().fold((0, 0), |(sx, sy), e| match e.kind {
            EventKind::MouseRaw { dx, dy } => (sx + dx, sy + dy),
            _ => (sx, sy),
        });
        assert_eq!((sx, sy), (100, -200));
        assert_eq!(out.len(), 13);
    }

    #[test]
    fn wheel_merge_respects_direction() {
        let w = |d| EventKind::Wheel { delta: d, horizontal: false, x: 0, y: 0 };
        let tl = Timeline::from_kinds([(0, w(120)), (10_000, w(120)), (20_000, w(-120)), (30_000, w(-120))]);
        let out = applied(&tl, merge_wheel(&tl, 50_000));
        let ds: Vec<_> = out.events().iter().map(|e| match e.kind { EventKind::Wheel { delta, .. } => delta, _ => 0 }).collect();
        assert_eq!(ds, vec![240, -240]);
    }

    #[test]
    fn cleanup_fixes_common_problems() {
        let tl = Timeline::from_kinds([
            (0, ku(9)),                    // orphan
            (10, kd(1)),
            (20, EventKind::KeyDown { key: Key::new(1, 0x41), repeat: true }),
            (30, mv(5, 5)),
            (40, mv(5, 5)),               // redundant
            (50, EventKind::MouseRaw { dx: 0, dy: 0 }), // redundant
            (60, kd(2)),                   // never released
            (70, ku(1)),
        ]);
        let (c, report) = cleanup(&tl, CleanupOptions { strip_repeats: true, ..Default::default() });
        assert_eq!(report, CleanupReport { orphan_releases: 1, closed_stuck: 1, repeats: 1, redundant_moves: 2 });
        let out = applied(&tl, c);
        assert!(out.holds().iter().all(|h| h.up.is_some()));
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn repetitions_are_found_regardless_of_pace() {
        let mut items = Vec::new();
        let mut t = 0;
        for rep in 0..4 {
            for s in [0x1Du16, 0x2E] {
                items.push((t, kd(s)));
                t += 10_000 + rep * 3_000;
            }
            for s in [0x2Eu16, 0x1D] {
                items.push((t, ku(s)));
                t += 10_000;
            }
            items.push((t, mv(rep as i32, 0)));
            t += 500_000;
        }
        let tl = Timeline::from_kinds(items);
        let reps = detect_repetitions(&tl, 3, 5);
        assert!(!reps.is_empty());
        let best = &reps[0];
        assert_eq!(best.length, 4);
        assert_eq!(best.occurrences.len(), 4);
    }
}
