//! Recording statistics (shown in the inspector and exported in reports).

use std::collections::HashMap;

use serde::Serialize;

use crate::event::{EventKind, Micros, MouseButton};
use crate::keys;
use crate::timeline::Timeline;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Stats {
    pub events: usize,
    pub duration_us: Micros,
    pub key_presses: usize,
    pub clicks: usize,
    pub wheel_notches: f64,
    /// Total pointer travel in pixels.
    pub mouse_distance_px: f64,
    pub pad_presses: usize,
    /// Discrete actions per minute.
    pub apm: f64,
    /// Most used keys: (label, presses).
    pub top_keys: Vec<(String, usize)>,
    /// Longest pause between two events.
    pub longest_pause_us: Micros,
    /// Average / max press duration.
    pub avg_hold_us: Micros,
    pub max_hold_us: Micros,
    /// Estimated pointer report rate (Hz) from raw or absolute moves.
    pub mouse_rate_hz: Option<f64>,
    pub clicks_by_button: Vec<(String, usize)>,
}

pub fn compute(tl: &Timeline) -> Stats {
    let ev = tl.events();
    let mut s = Stats { events: ev.len(), duration_us: tl.duration(), ..Default::default() };
    let mut key_counts: HashMap<String, usize> = HashMap::new();
    let mut buttons: HashMap<MouseButton, usize> = HashMap::new();
    let mut last_pos: Option<(i32, i32)> = None;
    let mut prev_t: Option<Micros> = None;
    let mut move_gaps: Vec<Micros> = Vec::new();
    let mut last_move_t: Option<Micros> = None;
    for e in ev {
        if let Some(p) = prev_t {
            s.longest_pause_us = s.longest_pause_us.max(e.t - p);
        }
        prev_t = Some(e.t);
        match e.kind {
            EventKind::KeyDown { key, repeat: false } => {
                s.key_presses += 1;
                *key_counts.entry(keys::label(key)).or_default() += 1;
            }
            EventKind::MouseDown { button, .. } => {
                s.clicks += 1;
                *buttons.entry(button).or_default() += 1;
            }
            EventKind::Wheel { delta, .. } => s.wheel_notches += (delta as f64 / 120.0).abs(),
            EventKind::PadButton { down: true, .. } => s.pad_presses += 1,
            EventKind::MouseMove { .. } | EventKind::MouseRaw { .. } => {
                if let Some(lt) = last_move_t {
                    let g = e.t - lt;
                    if g > 0 && g < 50_000 {
                        move_gaps.push(g);
                    }
                }
                last_move_t = Some(e.t);
            }
            _ => {}
        }
        if let Some((x, y)) = e.kind.position() {
            if let Some((lx, ly)) = last_pos {
                s.mouse_distance_px += (((x - lx) as f64).powi(2) + ((y - ly) as f64).powi(2)).sqrt();
            }
            last_pos = Some((x, y));
        }
    }
    let minutes = (s.duration_us as f64 / 60_000_000.0).max(1.0 / 60.0);
    s.apm = (s.key_presses + s.clicks + s.pad_presses) as f64 / minutes;

    let mut top: Vec<(String, usize)> = key_counts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(10);
    s.top_keys = top;

    let mut by_button: Vec<(String, usize)> = buttons.into_iter().map(|(b, n)| (b.name().to_string(), n)).collect();
    by_button.sort_by_key(|b| std::cmp::Reverse(b.1));
    s.clicks_by_button = by_button;

    let holds = tl.holds();
    let durations: Vec<Micros> = holds.iter().filter_map(|h| h.up.map(|u| ev[u].t - ev[h.down].t)).collect();
    if !durations.is_empty() {
        s.avg_hold_us = durations.iter().sum::<Micros>() / durations.len() as Micros;
        s.max_hold_us = *durations.iter().max().unwrap();
    }
    if move_gaps.len() >= 20 {
        move_gaps.sort_unstable();
        let median = move_gaps[move_gaps.len() / 2];
        if median > 0 {
            s.mouse_rate_hz = Some((1_000_000.0 / median as f64).round());
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Key;

    #[test]
    fn computes_basic_numbers() {
        let mut items = vec![
            (0, EventKind::KeyDown { key: Key::new(0x1E, 0x41), repeat: false }),
            (100_000, EventKind::KeyUp { key: Key::new(0x1E, 0x41) }),
            (200_000, EventKind::MouseDown { button: MouseButton::Left, x: 0, y: 0 }),
            (250_000, EventKind::MouseUp { button: MouseButton::Left, x: 3, y: 4 }),
        ];
        for i in 0..40 {
            items.push((300_000 + i * 1_000, EventKind::MouseRaw { dx: 1, dy: 0 }));
        }
        let s = compute(&Timeline::from_kinds(items));
        assert_eq!(s.key_presses, 1);
        assert_eq!(s.clicks, 1);
        assert_eq!(s.mouse_distance_px, 5.0);
        assert_eq!(s.top_keys[0], ("A".to_string(), 1));
        assert_eq!(s.avg_hold_us, 75_000);
        assert_eq!(s.mouse_rate_hz, Some(1000.0));
    }
}
