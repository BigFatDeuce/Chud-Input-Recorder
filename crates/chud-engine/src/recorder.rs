//! Turns the raw capture stream into a clean recording.
//!
//! Pure logic driven by timestamps, so every rule (pause, orphan releases,
//! hotkey residue, filters, limits) is unit tested without real input.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chud_core::document::{ContextSample, Environment, Marker};
use chud_core::event::{Control, Edge, EventKind, Key, Micros};
use chud_core::keys;
use chud_platform::{CaptureOptions, WindowInfo};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", content = "keys", rename_all = "snake_case")]
#[derive(Default)]
pub enum KeyFilter {
    #[default]
    All,
    /// Only these keys (scan identities).
    Only(Vec<u32>),
    Except(Vec<u32>),
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecordOptions {
    pub keyboard: bool,
    pub mouse_buttons: bool,
    pub mouse_move: bool,
    pub mouse_raw: bool,
    pub wheel: bool,
    pub gamepad: bool,
    /// Delay before recording actually starts.
    pub countdown_ms: u64,
    /// Stops automatically after this duration.
    pub max_duration_ms: Option<u64>,
    /// The timeline starts at the first input instead of at the click.
    pub start_on_first_input: bool,
    /// Replace real timing by a fixed interval ("record without intervals").
    pub uniform_gap_ms: Option<f64>,
    pub key_filter: KeyFilter,
    /// Only record while an application whose executable contains this text
    /// is in the foreground.
    pub only_process: Option<String>,
    pub include_injected: bool,
    pub excluded_processes: Vec<String>,
    /// Caps the rate of pointer motion samples (None = every report).
    pub mouse_max_hz: Option<u32>,
    /// Ignore analog changes smaller than this (gamepad noise gate).
    pub analog_threshold: i16,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            keyboard: true,
            mouse_buttons: true,
            mouse_move: true,
            mouse_raw: true,
            wheel: true,
            gamepad: true,
            countdown_ms: 0,
            max_duration_ms: None,
            start_on_first_input: true,
            uniform_gap_ms: None,
            key_filter: KeyFilter::All,
            only_process: None,
            include_injected: true,
            excluded_processes: default_exclusions(),
            mouse_max_hz: None,
            analog_threshold: 300,
        }
    }
}

/// Applications whose keystrokes are never recorded by default.
pub fn default_exclusions() -> Vec<String> {
    [
        "KeePass.exe",
        "KeePassXC.exe",
        "1Password.exe",
        "Bitwarden.exe",
        "Dashlane.exe",
        "LastPass.exe",
        "NordPass.exe",
        "ProtonPass.exe",
        "CredentialUIBroker.exe",
        "LogonUI.exe",
        "consent.exe",
        "keepassxc",
        "1password",
        "bitwarden",
        "seahorse",
        "polkit-gnome-authentication-agent-1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl RecordOptions {
    pub fn capture_options(&self) -> CaptureOptions {
        CaptureOptions {
            forward: true,
            keyboard: self.keyboard,
            mouse_buttons: self.mouse_buttons,
            mouse_move: self.mouse_move,
            mouse_raw: self.mouse_raw,
            wheel: self.wheel,
            gamepad: self.gamepad,
            include_injected: self.include_injected,
            watch_activity: false,
            excluded_processes: self.excluded_processes.clone(),
            exclude_self: true,
            include_own: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    User,
    Hotkey,
    MaxDuration,
    Emergency,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub items: Vec<(Micros, EventKind)>,
    pub contexts: Vec<ContextSample>,
    pub markers: Vec<Marker>,
    pub env: Environment,
    pub duration_us: Micros,
    pub stopped_by: StopReason,
    /// Events dropped because an excluded application was focused.
    pub redacted_events: usize,
}

pub struct Recorder {
    opts: RecordOptions,
    /// Origin of the recording clock (None until the first input when
    /// `start_on_first_input`).
    t0: Option<Instant>,
    armed_at: Instant,
    paused_since: Option<Instant>,
    paused_total: Duration,
    items: Vec<(Micros, EventKind)>,
    contexts: Vec<ContextSample>,
    markers: Vec<Marker>,
    /// Controls pressed during the recording (for orphan filtering), with
    /// the index of their press event.
    held: HashMap<Control, usize>,
    /// Keys physically held at pause time.
    held_at_pause: Vec<(Control, EventKind)>,
    last_move: Option<Instant>,
    last_raw_index: Option<usize>,
    last_axis: HashMap<(u8, u8), i16>,
    /// System controller index → index in the recording, in order of first
    /// use: whatever slot the system gave the controller, the first one used
    /// is controller 0 (playback then needs a single virtual controller).
    pad_map: Vec<u8>,
    foreground_ok: bool,
    redacted: usize,
    env: Environment,
}

impl Recorder {
    pub fn new(opts: RecordOptions, now: Instant, foreground: Option<&WindowInfo>, env: Environment) -> Self {
        let mut r = Self {
            t0: if opts.start_on_first_input { None } else { Some(now) },
            armed_at: now,
            paused_since: None,
            paused_total: Duration::ZERO,
            items: Vec::new(),
            contexts: Vec::new(),
            markers: Vec::new(),
            held: HashMap::new(),
            held_at_pause: Vec::new(),
            last_move: None,
            last_raw_index: None,
            last_axis: HashMap::new(),
            pad_map: Vec::new(),
            foreground_ok: true,
            redacted: 0,
            env,
            opts,
        };
        if let Some(w) = foreground {
            r.foreground_ok = r.process_allowed(&w.process);
            r.contexts.push(context(0, w, false));
        }
        r
    }

    fn process_allowed(&self, process: &str) -> bool {
        match &self.opts.only_process {
            Some(p) if !p.trim().is_empty() => process.to_lowercase().contains(&p.trim().to_lowercase()),
            _ => true,
        }
    }

    pub fn options(&self) -> &RecordOptions {
        &self.opts
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    pub fn event_count(&self) -> usize {
        self.items.len()
    }

    /// Recording clock at `t` (pauses excluded).
    pub fn clock(&self, t: Instant) -> Micros {
        let Some(t0) = self.t0 else { return 0 };
        let paused = self.paused_total + self.paused_since.map(|p| t.saturating_duration_since(p)).unwrap_or_default();
        t.saturating_duration_since(t0).saturating_sub(paused).as_micros() as Micros
    }

    /// Elapsed time shown to the user (counts from arming when waiting for
    /// the first input).
    pub fn elapsed(&self, now: Instant) -> Micros {
        if self.t0.is_some() { self.clock(now) } else { 0 }
    }

    pub fn reached_limit(&self, now: Instant) -> bool {
        match self.opts.max_duration_ms {
            Some(ms) => {
                let ref_t = if self.t0.is_some() { self.clock(now) } else { now.duration_since(self.armed_at).as_micros() as Micros };
                ref_t >= ms as Micros * 1_000
            }
            None => false,
        }
    }

    fn key_allowed(&self, key: Key) -> bool {
        match &self.opts.key_filter {
            KeyFilter::All => true,
            KeyFilter::Only(list) => list.contains(&key.ident()),
            KeyFilter::Except(list) => !list.contains(&key.ident()),
        }
    }

    /// Renumbers controllers in order of first use. A release from a
    /// controller never seen before is an orphan (pressed before the
    /// recording started) and is dropped.
    fn remap_pad(&mut self, kind: EventKind) -> Option<EventKind> {
        let pad = match kind {
            EventKind::PadButton { pad, .. } | EventKind::PadAxis { pad, .. } => pad,
            _ => return Some(kind),
        };
        let index = match self.pad_map.iter().position(|&p| p == pad) {
            Some(i) => i as u8,
            None if matches!(kind, EventKind::PadButton { down: false, .. }) => return None,
            None => {
                self.pad_map.push(pad);
                (self.pad_map.len() - 1) as u8
            }
        };
        Some(match kind {
            EventKind::PadButton { button, down, .. } => EventKind::PadButton { pad: index, button, down },
            EventKind::PadAxis { axis, value, .. } => EventKind::PadAxis { pad: index, axis, value },
            other => other,
        })
    }

    /// Feeds one input event. Returns true if it was kept.
    pub fn input(&mut self, t: Instant, kind: EventKind) -> bool {
        if self.paused_since.is_some() {
            return false;
        }
        if !self.foreground_ok {
            self.redacted += 1;
            return false;
        }
        // Category filters (the capture layer filters too, but options can
        // change while events are in flight).
        let wanted = match kind {
            EventKind::KeyDown { key, .. } | EventKind::KeyUp { key } => self.opts.keyboard && self.key_allowed(key),
            EventKind::MouseDown { .. } | EventKind::MouseUp { .. } => self.opts.mouse_buttons,
            EventKind::MouseMove { .. } => self.opts.mouse_move,
            EventKind::MouseRaw { .. } => self.opts.mouse_raw,
            EventKind::Wheel { .. } => self.opts.wheel,
            EventKind::PadButton { .. } | EventKind::PadAxis { .. } => self.opts.gamepad,
        };
        if !wanted {
            return false;
        }
        let kind = match self.remap_pad(kind) {
            Some(k) => k,
            None => return false,
        };

        // Orphan releases: a release for something pressed before the
        // recording started (typically the start hotkey or a click on the
        // record button) is dropped.
        if let Some((c, edge)) = kind.control() {
            match edge {
                Edge::Up => {
                    if self.held.remove(&c).is_none() {
                        return false;
                    }
                }
                Edge::Down => {
                    if !kind.is_repeat() {
                        self.held.entry(c).or_insert(self.items.len());
                    }
                }
            }
        }

        // Analog noise gate.
        if let EventKind::PadAxis { pad, axis, value } = kind {
            let prev = self.last_axis.get(&(pad, axis.0)).copied();
            let extreme = value == i16::MAX || value == i16::MIN || value == 0;
            if prev.is_some_and(|p| (p as i32 - value as i32).abs() < self.opts.analog_threshold as i32) && !extreme {
                return false;
            }
            self.last_axis.insert((pad, axis.0), value);
        }

        if self.t0.is_none() {
            self.t0 = Some(t);
        }
        let ts = self.clock(t);

        // Pointer sampling cap: coalesce instead of dropping, so absolute
        // moves keep the latest position and raw moves keep total motion.
        if let Some(hz) = self.opts.mouse_max_hz.filter(|h| *h > 0) {
            let min_gap = Duration::from_micros(1_000_000 / hz as u64);
            match kind {
                EventKind::MouseMove { .. } => {
                    if let Some(lm) = self.last_move
                        && t.saturating_duration_since(lm) < min_gap {
                            if let Some(last) = self.items.iter_mut().rev().find(|(_, k)| matches!(k, EventKind::MouseMove { .. })) {
                                last.1 = kind;
                            }
                            return true;
                        }
                    self.last_move = Some(t);
                }
                EventKind::MouseRaw { dx, dy } => {
                    if let Some(i) = self.last_raw_index
                        && let (Some((rt, EventKind::MouseRaw { dx: ax, dy: ay })), true) =
                            (self.items.get(i).copied(), true)
                            && ts - rt < min_gap.as_micros() as Micros {
                                self.items[i].1 = EventKind::MouseRaw { dx: ax + dx, dy: ay + dy };
                                return true;
                            }
                    self.last_raw_index = Some(self.items.len());
                }
                _ => {}
            }
        }

        // Keep the stream sorted even if two sources deliver slightly out of order.
        let pos = self.items.partition_point(|(it, _)| *it <= ts);
        if pos == self.items.len() {
            self.items.push((ts, kind));
        } else {
            self.items.insert(pos, (ts, kind));
            for v in self.held.values_mut() {
                if *v >= pos {
                    *v += 1;
                }
            }
            if let Some(i) = self.last_raw_index.as_mut()
                && *i >= pos {
                    *i += 1;
                }
        }
        true
    }

    pub fn foreground(&mut self, t: Instant, w: &WindowInfo, excluded: bool) {
        let was_ok = self.foreground_ok;
        self.foreground_ok = !excluded && self.process_allowed(&w.process);
        let ts = self.clock(t);
        // Leaving an allowed window while keys are held: close the holds so
        // that nothing stays pressed during playback.
        if was_ok && !self.foreground_ok {
            self.release_all(ts);
        }
        self.contexts.push(context(ts, w, excluded));
    }

    fn release_all(&mut self, ts: Micros) {
        let mut open: Vec<(Control, usize)> = self.held.drain().collect();
        open.sort_by_key(|(_, i)| std::cmp::Reverse(*i));
        for (_, idx) in open {
            if let Some(rel) = self.items.get(idx).and_then(|(_, k)| k.release_for()) {
                self.items.push((ts.max(self.items.last().map(|i| i.0).unwrap_or(0)), rel));
            }
        }
    }

    pub fn marker(&mut self, t: Instant, label: String) {
        let ts = self.clock(t);
        self.markers.push(Marker { t: ts, label, ..Default::default() });
    }

    pub fn pause(&mut self, t: Instant) {
        if self.paused_since.is_some() {
            return;
        }
        let ts = self.clock(t);
        // Remember what was held so that it can be pressed again on resume.
        self.held_at_pause = self
            .held
            .iter()
            .filter_map(|(c, &i)| self.items.get(i).map(|(_, k)| (*c, *k)))
            .collect();
        self.release_all(ts);
        self.paused_since = Some(t);
    }

    /// `still_held` tells which controls are physically down at resume time.
    pub fn resume(&mut self, t: Instant, still_held: impl Fn(&EventKind) -> bool) {
        let Some(p) = self.paused_since.take() else { return };
        self.paused_total += t.saturating_duration_since(p);
        let ts = self.clock(t);
        for (c, k) in std::mem::take(&mut self.held_at_pause) {
            if still_held(&k) {
                self.held.insert(c, self.items.len());
                self.items.push((ts, k));
            }
        }
    }

    /// Finishes the recording. `hotkey_mods` lists modifier keys of the stop
    /// hotkey: if they are still held, their presses are residue of the
    /// hotkey and are removed instead of being released.
    pub fn finish(mut self, t: Instant, reason: StopReason, hotkey_mods: &[Key]) -> Recording {
        let end = self.clock(t);
        let residue: HashSet<u32> = hotkey_mods.iter().map(|k| k.ident()).collect();
        let mut remove: Vec<usize> = Vec::new();
        let recent = 1_500_000;
        for (c, &idx) in &self.held {
            if let (Control::Key(id), Some((pt, k))) = (c, self.items.get(idx)) {
                let is_mod = matches!(k, EventKind::KeyDown { key, .. } if keys::is_modifier(*key));
                if residue.contains(id) && is_mod && end - pt < recent {
                    remove.push(idx);
                }
            }
        }
        // Also drop auto-repeats of residue modifiers.
        remove.sort_unstable();
        for idx in remove.iter().rev() {
            let (_, k) = self.items.remove(*idx);
            if let Some((c, _)) = k.control() {
                self.held.remove(&c);
                self.items.retain(|(_, e)| !(e.is_repeat() && e.control().map(|x| x.0) == Some(c)));
            }
        }
        // Rebuild positions after removals, then release what is still held.
        let mut open: HashMap<Control, usize> = HashMap::new();
        for (i, (_, k)) in self.items.iter().enumerate() {
            if let Some((c, e)) = k.control() {
                match e {
                    Edge::Down => {
                        open.entry(c).or_insert(i);
                    }
                    Edge::Up => {
                        open.remove(&c);
                    }
                }
            }
        }
        self.held = open;
        let last = self.items.last().map(|i| i.0).unwrap_or(0);
        self.release_all(end.max(last));

        let mut items = self.items;
        if let Some(gap) = self.opts.uniform_gap_ms {
            let tl = chud_core::Timeline::from_kinds(items.clone());
            let mut tl2 = tl.clone();
            let change = chud_core::transform::uniform_timing(&tl, (gap * 1_000.0) as Micros, 0);
            if tl2.apply(&change).is_ok() {
                items = tl2.events().iter().map(|e| (e.t, e.kind)).collect();
            }
        }
        let duration_us = items.last().map(|i| i.0).unwrap_or(0);
        Recording {
            items,
            contexts: self.contexts,
            markers: self.markers,
            env: self.env,
            duration_us,
            stopped_by: reason,
            redacted_events: self.redacted,
        }
    }
}

fn context(t: Micros, w: &WindowInfo, redacted: bool) -> ContextSample {
    ContextSample {
        t,
        process: w.process.clone(),
        title: if redacted { String::new() } else { w.title.clone() },
        class: w.class.clone(),
        rect: w.rect,
        redacted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chud_core::event::MouseButton;

    fn kd(code: &str) -> EventKind {
        let i = keys::by_code(code).unwrap();
        EventKind::KeyDown { key: Key::new(i.scan, i.vk), repeat: false }
    }
    fn ku(code: &str) -> EventKind {
        let i = keys::by_code(code).unwrap();
        EventKind::KeyUp { key: Key::new(i.scan, i.vk) }
    }
    fn ms(base: Instant, v: u64) -> Instant {
        base + Duration::from_millis(v)
    }
    fn rec(opts: RecordOptions) -> (Recorder, Instant) {
        let now = Instant::now();
        (Recorder::new(opts, now, None, Environment::default()), now)
    }

    #[test]
    fn controllers_are_numbered_in_order_of_use() {
        use chud_core::event::{PadAxis, PadButton};
        let (mut r, b) = rec(RecordOptions::default());
        // Released before it was ever pressed: an orphan.
        assert!(!r.input(ms(b, 10), EventKind::PadButton { pad: 3, button: PadButton::SOUTH, down: false }));
        assert!(r.input(ms(b, 20), EventKind::PadButton { pad: 2, button: PadButton::EAST, down: true }));
        assert!(r.input(ms(b, 30), EventKind::PadAxis { pad: 0, axis: PadAxis::LEFT_X, value: 20_000 }));
        assert!(r.input(ms(b, 40), EventKind::PadButton { pad: 2, button: PadButton::EAST, down: false }));
        let out = r.finish(ms(b, 50), StopReason::User, &[]);
        assert_eq!(
            out.items,
            vec![
                (0, EventKind::PadButton { pad: 0, button: PadButton::EAST, down: true }),
                (10_000, EventKind::PadAxis { pad: 1, axis: PadAxis::LEFT_X, value: 20_000 }),
                (20_000, EventKind::PadButton { pad: 0, button: PadButton::EAST, down: false }),
            ]
        );
    }

    #[test]
    fn starts_at_first_input_and_drops_orphans() {
        let (mut r, b) = rec(RecordOptions::default());
        assert!(!r.input(ms(b, 100), ku("F9"))); // release of the start hotkey
        assert!(r.input(ms(b, 500), kd("KeyA")));
        assert!(r.input(ms(b, 600), ku("KeyA")));
        let out = r.finish(ms(b, 700), StopReason::User, &[]);
        assert_eq!(out.items, vec![(0, kd("KeyA")), (100_000, ku("KeyA"))]);
    }

    #[test]
    fn pause_excludes_time_and_rebalances_holds() {
        let opts = RecordOptions { start_on_first_input: false, ..Default::default() };
        let (mut r, b) = rec(opts);
        r.input(ms(b, 100), kd("KeyW"));
        r.pause(ms(b, 200));
        assert!(!r.input(ms(b, 300), kd("KeyX")));
        r.resume(ms(b, 1_200), |_| true); // W still held
        r.input(ms(b, 1_300), ku("KeyW"));
        let out = r.finish(ms(b, 1_400), StopReason::User, &[]);
        let ts: Vec<_> = out.items.iter().map(|i| (i.0, i.1)).collect();
        assert_eq!(
            ts,
            vec![(100_000, kd("KeyW")), (200_000, ku("KeyW")), (200_000, kd("KeyW")), (300_000, ku("KeyW"))]
        );
    }

    #[test]
    fn stop_hotkey_modifier_residue_is_removed() {
        let (mut r, b) = rec(RecordOptions::default());
        r.input(ms(b, 0), kd("KeyA"));
        r.input(ms(b, 50), ku("KeyA"));
        r.input(ms(b, 900), kd("ControlLeft"));
        r.input(ms(b, 950), EventKind::KeyDown { key: kd_key("ControlLeft"), repeat: true });
        r.input(ms(b, 1_000), kd("KeyS")); // a normal key still held at stop: released
        let ctrl = kd_key("ControlLeft");
        let out = r.finish(ms(b, 1_100), StopReason::Hotkey, &[ctrl]);
        let kinds: Vec<_> = out.items.iter().map(|i| i.1).collect();
        assert_eq!(kinds, vec![kd("KeyA"), ku("KeyA"), kd("KeyS"), ku("KeyS")]);
        assert_eq!(out.items.last().unwrap().0, 1_100_000);
    }

    fn kd_key(code: &str) -> Key {
        let i = keys::by_code(code).unwrap();
        Key::new(i.scan, i.vk)
    }

    #[test]
    fn excluded_foreground_redacts_and_closes_holds() {
        let (mut r, b) = rec(RecordOptions::default());
        r.input(ms(b, 0), EventKind::MouseDown { button: MouseButton::Left, x: 1, y: 1 });
        let vault = WindowInfo { process: "KeePass.exe".into(), title: "secret".into(), ..Default::default() };
        r.foreground(ms(b, 100), &vault, true);
        assert!(!r.input(ms(b, 150), kd("KeyP")));
        let editor = WindowInfo { process: "code.exe".into(), title: "x".into(), ..Default::default() };
        r.foreground(ms(b, 200), &editor, false);
        let out = r.finish(ms(b, 300), StopReason::User, &[]);
        assert_eq!(out.redacted_events, 1);
        assert!(matches!(out.items[1].1, EventKind::MouseUp { .. }));
        assert_eq!(out.contexts[0].title, "");
        assert!(out.contexts[0].redacted);
    }

    #[test]
    fn filters_limits_and_sampling() {
        let opts = RecordOptions {
            key_filter: KeyFilter::Only(vec![kd_key("KeyA").ident()]),
            mouse_max_hz: Some(100),
            max_duration_ms: Some(1_000),
            ..Default::default()
        };
        let (mut r, b) = rec(opts);
        assert!(r.input(ms(b, 0), kd("KeyA")));
        assert!(!r.input(ms(b, 1), kd("KeyB")));
        for i in 0..10 {
            r.input(ms(b, 2 + i), EventKind::MouseMove { x: i as i32, y: 0 });
            r.input(ms(b, 2 + i), EventKind::MouseRaw { dx: 1, dy: 2 });
        }
        assert!(!r.reached_limit(ms(b, 500)));
        assert!(r.reached_limit(ms(b, 1_000)));
        let out = r.finish(ms(b, 20), StopReason::MaxDuration, &[]);
        let moves: Vec<_> = out.items.iter().filter(|i| matches!(i.1, EventKind::MouseMove { .. })).collect();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].1, EventKind::MouseMove { x: 9, y: 0 });
        let raw_total: i32 = out.items.iter().map(|i| match i.1 { EventKind::MouseRaw { dx, .. } => dx, _ => 0 }).sum();
        assert_eq!(raw_total, 10);
    }

    #[test]
    fn only_process_filter() {
        let opts = RecordOptions { only_process: Some("game".into()), ..Default::default() };
        let now = Instant::now();
        let desk = WindowInfo { process: "explorer.exe".into(), ..Default::default() };
        let mut r = Recorder::new(opts, now, Some(&desk), Environment::default());
        assert!(!r.input(ms(now, 10), kd("KeyA")));
        let game = WindowInfo { process: "MyGame.exe".into(), ..Default::default() };
        r.foreground(ms(now, 20), &game, false);
        assert!(r.input(ms(now, 30), kd("KeyA")));
    }

    #[test]
    fn uniform_timing_option() {
        let opts = RecordOptions { uniform_gap_ms: Some(50.0), ..Default::default() };
        let (mut r, b) = rec(opts);
        r.input(ms(b, 0), kd("KeyA"));
        r.input(ms(b, 700), ku("KeyA"));
        r.input(ms(b, 3_000), kd("KeyB"));
        r.input(ms(b, 3_001), ku("KeyB"));
        let out = r.finish(ms(b, 3_100), StopReason::User, &[]);
        let ts: Vec<_> = out.items.iter().map(|i| i.0).collect();
        assert_eq!(ts, vec![0, 50_000, 100_000, 150_000]);
    }
}
