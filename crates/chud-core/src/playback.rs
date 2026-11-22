//! Playback settings and the *plan*: the exact list of output actions with
//! their due times. Compiling is a pure function so every timing rule can be
//! unit tested without touching the operating system.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::document::CoordMap;
use crate::event::{Control, Edge, EventId, EventKind, Key, Micros, MouseButton, PadAxis, PadButton};
use crate::timeline::Timeline;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimingMode {
    /// Due times anchored to the start: no cumulative drift.
    #[default]
    Exact,
    /// Each interval measured from the previous actual dispatch.
    Relative,
    /// Exact, but a lag larger than `adaptive_max_lag_ms` re-anchors the
    /// timeline instead of replaying the late events in a burst.
    Adaptive,
    /// Intervals removed (`fast_gap_us` between events).
    Fast,
    /// Intervals clamped to `[custom_min_gap_ms, custom_max_gap_ms]`, or
    /// replaced by `custom_fixed_gap_ms` when set.
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseMode {
    /// Absolute positions (desktop automation).
    #[default]
    Absolute,
    /// Raw relative deltas (games with a captured cursor).
    Relative,
    /// Only clicks and wheel, at the current pointer position.
    Off,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyMode {
    /// Scan codes: the same physical keys (layout independent, DirectInput friendly).
    #[default]
    Physical,
    /// Virtual keys: the same logical keys under the current layout.
    Logical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordMode {
    /// Use coordinates exactly as recorded.
    #[default]
    AsRecorded,
    /// Remap to the current monitor layout when it differs from the recording.
    AdaptToScreens,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlaybackSettings {
    pub speed: f64,
    /// Number of runs; `0` means until stopped.
    pub repeat: u32,
    pub repeat_delay_ms: f64,
    pub timing: TimingMode,
    pub adaptive_max_lag_ms: f64,
    pub fast_gap_us: i64,
    pub custom_min_gap_ms: f64,
    pub custom_max_gap_ms: f64,
    pub custom_fixed_gap_ms: Option<f64>,
    pub mouse: MouseMode,
    pub keys: KeyMode,
    pub coords: CoordMode,
    pub gamepad: bool,
    /// Put the pointer back where it was once playback ends.
    pub restore_cursor: bool,
    /// Stop when the user moves the mouse or presses a key.
    pub abort_on_user_input: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            repeat: 1,
            repeat_delay_ms: 0.0,
            timing: TimingMode::Exact,
            adaptive_max_lag_ms: 15.0,
            fast_gap_us: 1_000,
            custom_min_gap_ms: 0.0,
            custom_max_gap_ms: 1_000.0,
            custom_fixed_gap_ms: None,
            mouse: MouseMode::Absolute,
            keys: KeyMode::Physical,
            coords: CoordMode::AsRecorded,
            gamepad: true,
            restore_cursor: false,
            abort_on_user_input: false,
        }
    }
}

/// Which part of the timeline to play.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    #[default]
    All,
    /// From a time to the end.
    From { t: Micros },
    /// `[start, end)`.
    Range { start: Micros, end: Micros },
    /// Specific events.
    Ids { ids: Vec<EventId> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyVia {
    Scan,
    Vk,
}

/// Output-level action, independent of the operating system.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    MoveTo { x: i32, y: i32 },
    MoveBy { dx: i32, dy: i32 },
    Button { button: MouseButton, down: bool, at: Option<(i32, i32)> },
    Wheel { delta: i32, horizontal: bool, at: Option<(i32, i32)> },
    Key { key: Key, down: bool, via: KeyVia },
    PadButton { pad: u8, button: PadButton, down: bool },
    PadAxis { pad: u8, axis: PadAxis, value: i16 },
}

impl Action {
    /// The control this action presses or releases.
    pub fn control(&self) -> Option<(Control, Edge)> {
        let edge = |down: bool| if down { Edge::Down } else { Edge::Up };
        match *self {
            Action::Key { key, down, .. } => Some((Control::Key(key.ident()), edge(down))),
            Action::Button { button, down, .. } => Some((Control::Mouse(button), edge(down))),
            Action::PadButton { pad, button, down } => Some((Control::Pad(pad, button), edge(down))),
            _ => None,
        }
    }

    /// The action releasing a pressed control.
    pub fn release(&self) -> Option<Action> {
        match *self {
            Action::Key { key, down: true, via } => Some(Action::Key { key, down: false, via }),
            Action::Button { button, down: true, .. } => Some(Action::Button { button, down: false, at: None }),
            Action::PadButton { pad, button, down: true } => Some(Action::PadButton { pad, button, down: false }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Step {
    /// Due time from the start of this run, already scaled by speed.
    pub at: Micros,
    /// Source event (`u32::MAX` for synthesized steps).
    pub id: EventId,
    pub action: Action,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// Time of the last step.
    pub duration: Micros,
    /// Source time of the first step (to map progress back onto the timeline).
    pub source_start: Micros,
    pub speed: f64,
}

impl Plan {
    /// Timeline time corresponding to a plan time (for the playhead).
    pub fn source_time(&self, at: Micros) -> Micros {
        self.source_start + (at as f64 * self.speed) as Micros
    }
}

pub const SYNTHETIC: EventId = EventId::MAX;

/// Builds the playback plan.
pub fn compile(tl: &Timeline, settings: &PlaybackSettings, selection: &Selection, coords: Option<&CoordMap>) -> Plan {
    let events = tl.events();
    let picked: Vec<usize> = match selection {
        Selection::All => (0..events.len()).collect(),
        Selection::From { t } => tl.range(*t, Micros::MAX).collect(),
        Selection::Range { start, end } => tl.range(*start, *end).collect(),
        Selection::Ids { ids } => {
            let mut v: Vec<usize> = ids.iter().filter_map(|&id| tl.position_of(id)).collect();
            v.sort_unstable();
            v.dedup();
            v
        }
    };

    let map = |x: i32, y: i32| match (settings.coords, coords) {
        (CoordMode::AdaptToScreens, Some(m)) => m.map(x, y),
        _ => (x, y),
    };
    let via = match settings.keys {
        KeyMode::Physical => KeyVia::Scan,
        KeyMode::Logical => KeyVia::Vk,
    };
    let key_via = |key: Key| match via {
        KeyVia::Scan if key.scan == 0 || key.scan & 0xF000 == crate::keys::EVDEV_PRIVATE && key.vk != 0 => KeyVia::Vk,
        KeyVia::Vk if key.vk == 0 => KeyVia::Scan,
        v => v,
    };

    // 1. Translate events into actions, skipping releases whose press is not
    //    part of the selection (playing from the middle of a hold).
    let mut pressed: HashSet<Control> = HashSet::new();
    let mut raw: Vec<(Micros, EventId, Action)> = Vec::with_capacity(picked.len());
    for &i in &picked {
        let e = events[i];
        if let Some((c, edge)) = e.kind.control() {
            match edge {
                Edge::Down => {
                    pressed.insert(c);
                }
                Edge::Up => {
                    if !pressed.remove(&c) {
                        continue;
                    }
                }
            }
        }
        let action = match e.kind {
            EventKind::MouseMove { x, y } => match settings.mouse {
                MouseMode::Absolute => {
                    let (x, y) = map(x, y);
                    Action::MoveTo { x, y }
                }
                _ => continue,
            },
            EventKind::MouseRaw { dx, dy } => match settings.mouse {
                MouseMode::Relative => Action::MoveBy { dx, dy },
                _ => continue,
            },
            EventKind::MouseDown { button, x, y } | EventKind::MouseUp { button, x, y } => Action::Button {
                button,
                down: matches!(e.kind, EventKind::MouseDown { .. }),
                at: (settings.mouse == MouseMode::Absolute).then(|| map(x, y)),
            },
            EventKind::Wheel { delta, horizontal, x, y } => Action::Wheel {
                delta,
                horizontal,
                at: (settings.mouse == MouseMode::Absolute).then(|| map(x, y)),
            },
            EventKind::KeyDown { key, .. } => Action::Key { key, down: true, via: key_via(key) },
            EventKind::KeyUp { key } => Action::Key { key, down: false, via: key_via(key) },
            EventKind::PadButton { pad, button, down } => {
                if !settings.gamepad {
                    continue;
                }
                Action::PadButton { pad, button, down }
            }
            EventKind::PadAxis { pad, axis, value } => {
                if !settings.gamepad {
                    continue;
                }
                Action::PadAxis { pad, axis, value }
            }
        };
        raw.push((e.t, e.id, action));
    }

    let Some(&(first_t, _, _)) = raw.first() else {
        return Plan { speed: settings.speed, ..Default::default() };
    };
    // A full playback keeps the leading silence; partial playback starts now.
    let origin = if matches!(selection, Selection::All) { 0 } else { first_t };

    // 2. Re-time according to the timing mode.
    let gap_rule = |gap: Micros| -> Micros {
        if gap <= 0 {
            return 0;
        }
        match settings.timing {
            TimingMode::Fast => settings.fast_gap_us.max(0),
            TimingMode::Custom => match settings.custom_fixed_gap_ms {
                Some(f) => (f * 1_000.0).round().max(0.0) as Micros,
                None => {
                    let lo = (settings.custom_min_gap_ms * 1_000.0) as Micros;
                    let hi = (settings.custom_max_gap_ms * 1_000.0) as Micros;
                    gap.clamp(lo.max(0), hi.max(lo.max(0)))
                }
            },
            _ => gap,
        }
    };
    let speed = if settings.speed.is_finite() && settings.speed > 0.0 { settings.speed } else { 1.0 };
    let mut steps = Vec::with_capacity(raw.len() + pressed.len());
    let mut src_prev = origin;
    let mut cursor: Micros = 0;
    for (k, (t, id, action)) in raw.iter().enumerate() {
        let gap = t - src_prev;
        let gap = if k == 0 && !matches!(selection, Selection::All) { 0 } else { gap_rule(gap) };
        cursor += gap;
        src_prev = *t;
        steps.push(Step { at: (cursor as f64 / speed).round() as Micros, id: *id, action: *action });
    }

    // 3. Never leave anything pressed: close holds that are not released
    //    inside the selection.
    let end = steps.last().map(|s| s.at).unwrap_or(0);
    let mut open: Vec<Action> = Vec::new();
    let mut state: HashSet<Control> = HashSet::new();
    for s in &steps {
        if let Some((c, edge)) = s.action.control() {
            match edge {
                Edge::Down => {
                    if state.insert(c) {
                        open.push(s.action);
                    }
                }
                Edge::Up => {
                    state.remove(&c);
                    open.retain(|a| a.control().map(|x| x.0) != Some(c));
                }
            }
        }
    }
    for a in open.into_iter().rev() {
        if let Some(r) = a.release() {
            steps.push(Step { at: end, id: SYNTHETIC, action: r });
        }
    }

    Plan { duration: steps.last().map(|s| s.at).unwrap_or(0), steps, source_start: origin, speed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kd(s: u16) -> EventKind {
        EventKind::KeyDown { key: Key::new(s, 0x41), repeat: false }
    }
    fn ku(s: u16) -> EventKind {
        EventKind::KeyUp { key: Key::new(s, 0x41) }
    }

    fn tl() -> Timeline {
        Timeline::from_kinds([
            (1_000, EventKind::MouseMove { x: 10, y: 10 }),
            (1_000, EventKind::MouseRaw { dx: 3, dy: 4 }),
            (2_000, kd(0x1E)),
            (5_000, EventKind::MouseDown { button: MouseButton::Left, x: 10, y: 10 }),
            (6_000, EventKind::MouseUp { button: MouseButton::Left, x: 12, y: 10 }),
            (10_000, ku(0x1E)),
        ])
    }

    #[test]
    fn exact_keeps_times_and_leading_silence() {
        let p = compile(&tl(), &PlaybackSettings::default(), &Selection::All, None);
        let ats: Vec<_> = p.steps.iter().map(|s| s.at).collect();
        assert_eq!(ats, vec![1_000, 2_000, 5_000, 6_000, 10_000]);
        assert!(matches!(p.steps[0].action, Action::MoveTo { x: 10, y: 10 }));
        assert!(matches!(p.steps[2].action, Action::Button { at: Some((10, 10)), down: true, .. }));
    }

    #[test]
    fn speed_and_relative_mouse() {
        let s = PlaybackSettings { speed: 2.0, mouse: MouseMode::Relative, ..Default::default() };
        let p = compile(&tl(), &s, &Selection::All, None);
        assert!(matches!(p.steps[0].action, Action::MoveBy { dx: 3, dy: 4 }));
        assert!(matches!(p.steps[2].action, Action::Button { at: None, .. }));
        assert_eq!(p.steps.last().unwrap().at, 5_000);
    }

    #[test]
    fn fast_and_custom_timing() {
        let s = PlaybackSettings { timing: TimingMode::Fast, fast_gap_us: 100, ..Default::default() };
        let p = compile(&tl(), &s, &Selection::All, None);
        let ats: Vec<_> = p.steps.iter().map(|s| s.at).collect();
        assert_eq!(ats, vec![100, 200, 300, 400, 500]);

        let s = PlaybackSettings {
            timing: TimingMode::Custom,
            custom_min_gap_ms: 2.0,
            custom_max_gap_ms: 3.0,
            ..Default::default()
        };
        let p = compile(&tl(), &s, &Selection::All, None);
        let ats: Vec<_> = p.steps.iter().map(|s| s.at).collect();
        assert_eq!(ats, vec![2_000, 4_000, 7_000, 9_000, 12_000]);
    }

    #[test]
    fn partial_selection_starts_immediately_and_closes_holds() {
        // From 3 ms: the key press at 2 ms is outside, so its release is skipped.
        let p = compile(&tl(), &PlaybackSettings::default(), &Selection::From { t: 3_000 }, None);
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[0].at, 0);
        assert_eq!(p.source_time(1_000), 6_000);

        // Range ending before the release: a synthetic release is appended.
        let p = compile(&tl(), &PlaybackSettings::default(), &Selection::Range { start: 0, end: 5_500 }, None);
        // Released in reverse press order: the button first, then the key.
        let tail: Vec<_> = p.steps.iter().rev().take(2).collect();
        assert!(tail.iter().all(|s| s.id == SYNTHETIC && s.at == 4_000));
        assert!(matches!(tail[0].action, Action::Key { down: false, .. }));
        assert!(matches!(tail[1].action, Action::Button { down: false, .. }));
    }

    #[test]
    fn logical_mode_and_scan_fallback() {
        let t = Timeline::from_kinds([(0, EventKind::KeyDown { key: Key::new(0, 0x41), repeat: false })]);
        let p = compile(&t, &PlaybackSettings::default(), &Selection::All, None);
        assert!(matches!(p.steps[0].action, Action::Key { via: KeyVia::Vk, .. }));
        let s = PlaybackSettings { keys: KeyMode::Logical, ..Default::default() };
        let p = compile(&tl(), &s, &Selection::All, None);
        assert!(matches!(p.steps[1].action, Action::Key { via: KeyVia::Vk, .. }));
    }
}
