//! # chud-engine
//!
//! Real-time orchestration: one engine thread owns the state machine
//! (idle / countdown / recording / playing), consumes the capture stream,
//! reacts to global hotkeys and drives the playback thread. The UI talks to
//! it through [`Engine`] and receives [`EngineEvent`]s.

pub mod player;
pub mod recorder;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chud_core::event::{EventKind, Key, Micros, MouseButton};
use chud_core::keys;
use chud_platform::{CaptureOptions, Captured, Hotkey, HotkeyAction, Platform, WindowInfo};
use crossbeam_channel::{Receiver, Sender, select, unbounded};
use parking_lot::Mutex;
use serde::Serialize;

pub use player::{MacroResolver, NoMacros, Notice, PlayEnd, PlayJob, PlayerShared, find_on_screen};
pub use recorder::{KeyFilter, RecordOptions, Recording, StopReason};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Idle,
    Countdown,
    Recording,
    Playing,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EngineStatus {
    pub mode: Mode,
    pub paused: bool,
    pub countdown_ms: u64,
    /// Recording clock.
    pub elapsed_us: Micros,
    pub event_count: usize,
    pub waiting_first_input: bool,
    pub play_position_us: Micros,
    pub play_duration_us: Micros,
    pub iteration: u32,
    pub iterations: u32,
    pub macro_id: Option<String>,
    pub macro_name: String,
    pub block: String,
    pub live: bool,
    /// Id of the macro the play hotkey starts while idle (its name when it has no id).
    pub armed: Option<String>,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self {
            mode: Mode::Idle,
            paused: false,
            countdown_ms: 0,
            elapsed_us: 0,
            event_count: 0,
            waiting_first_input: false,
            play_position_us: 0,
            play_duration_us: 0,
            iteration: 0,
            iterations: 1,
            macro_id: None,
            macro_name: String::new(),
            block: String::new(),
            live: false,
            armed: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveInput {
    /// Microseconds since the live view was enabled.
    pub t: Micros,
    #[serde(flatten)]
    pub kind: EventKind,
    pub injected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    Status(EngineStatus),
    Hotkey { action: HotkeyAction },
    Live { inputs: Vec<LiveInput> },
    Foreground { window: WindowInfo },
    RecordingFinished { recording: Recording },
    PlaybackFinished { end: PlayEnd, macro_id: Option<String> },
    Notice(Notice),
}

enum Cmd {
    StartRecording(RecordOptions),
    StopRecording,
    TogglePause,
    Marker(String),
    Play(Box<PlayJob>),
    StopPlayback,
    Emergency,
    SetLive(bool),
    Arm(Option<Box<PlayJob>>),
    SetHotkeys(Vec<Hotkey>),
    SetRecordDefaults(RecordOptions),
    Shutdown,
}

/// Handle to the engine thread. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    tx: Sender<Cmd>,
    status: Arc<Mutex<EngineStatus>>,
    platform: Arc<dyn Platform>,
}

impl Engine {
    pub fn new(
        platform: Arc<dyn Platform>,
        resolver: Arc<dyn MacroResolver>,
        events: Sender<EngineEvent>,
    ) -> Result<Self, chud_platform::PlatformError> {
        let (cap_tx, cap_rx) = unbounded();
        platform.start(cap_tx)?;
        platform.set_hotkeys(Hotkey::defaults());
        platform.set_capture(CaptureOptions::default());
        let (tx, rx) = unbounded();
        let status = Arc::new(Mutex::new(EngineStatus::default()));
        let worker = Worker {
            platform: platform.clone(),
            resolver,
            events,
            status: status.clone(),
            activity: Activity::Idle,
            live: None,
            live_buf: Vec::new(),
            armed: None,
            hotkeys: Hotkey::defaults(),
            record_defaults: RecordOptions::default(),
            last_status: None,
            last_status_at: Instant::now(),
        };
        std::thread::Builder::new()
            .name("chud-engine".into())
            .spawn(move || worker.run(rx, cap_rx))
            .map_err(|e| chud_platform::PlatformError::System(e.to_string()))?;
        Ok(Self { tx, status, platform })
    }

    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }
    pub fn status(&self) -> EngineStatus {
        self.status.lock().clone()
    }
    pub fn start_recording(&self, opts: RecordOptions) {
        let _ = self.tx.send(Cmd::StartRecording(opts));
    }
    pub fn stop_recording(&self) {
        let _ = self.tx.send(Cmd::StopRecording);
    }
    pub fn toggle_pause(&self) {
        let _ = self.tx.send(Cmd::TogglePause);
    }
    pub fn add_marker(&self, label: String) {
        let _ = self.tx.send(Cmd::Marker(label));
    }
    pub fn play(&self, job: PlayJob) {
        let _ = self.tx.send(Cmd::Play(Box::new(job)));
    }
    pub fn stop_playback(&self) {
        let _ = self.tx.send(Cmd::StopPlayback);
    }
    pub fn emergency_stop(&self) {
        let _ = self.tx.send(Cmd::Emergency);
    }
    pub fn set_live(&self, on: bool) {
        let _ = self.tx.send(Cmd::SetLive(on));
    }
    /// The job the "play" hotkey starts.
    pub fn arm(&self, job: Option<PlayJob>) {
        let _ = self.tx.send(Cmd::Arm(job.map(Box::new)));
    }
    pub fn set_hotkeys(&self, hotkeys: Vec<Hotkey>) {
        let _ = self.tx.send(Cmd::SetHotkeys(hotkeys));
    }
    /// A key event received by Chud's own window (see `Platform::local_key`).
    pub fn local_key(&self, key: Key, down: bool) {
        self.platform.local_key(key, down);
    }
    pub fn set_record_defaults(&self, opts: RecordOptions) {
        let _ = self.tx.send(Cmd::SetRecordDefaults(opts));
    }
    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

enum Activity {
    Idle,
    Countdown { until: Instant, opts: RecordOptions },
    Recording(Box<recorder::Recorder>),
    Playing(PlayingState),
}

struct PlayingState {
    shared: Arc<PlayerShared>,
    handle: Option<JoinHandle<()>>,
    macro_id: Option<String>,
    name: String,
    iterations: u32,
    abort_on_input: bool,
    started: Instant,
}

struct Worker {
    platform: Arc<dyn Platform>,
    resolver: Arc<dyn MacroResolver>,
    events: Sender<EngineEvent>,
    status: Arc<Mutex<EngineStatus>>,
    activity: Activity,
    live: Option<Instant>,
    live_buf: Vec<LiveInput>,
    armed: Option<Box<PlayJob>>,
    hotkeys: Vec<Hotkey>,
    record_defaults: RecordOptions,
    last_status: Option<EngineStatus>,
    last_status_at: Instant,
}

impl Worker {
    fn run(mut self, rx: Receiver<Cmd>, cap: Receiver<Captured>) {
        let (done_tx, done_rx) = unbounded::<PlayEnd>();
        let (notice_tx, notice_rx) = unbounded::<Notice>();
        let tick = crossbeam_channel::tick(Duration::from_millis(16));
        loop {
            select! {
                recv(rx) -> cmd => match cmd {
                    Ok(Cmd::Shutdown) | Err(_) => {
                        self.stop_playback();
                        self.platform.set_capture(CaptureOptions::default());
                        return;
                    }
                    Ok(cmd) => self.command(cmd, &done_tx, &notice_tx),
                },
                recv(cap) -> c => if let Ok(c) = c { self.captured(c, &done_tx, &notice_tx) },
                recv(done_rx) -> end => if let Ok(end) = end { self.playback_done(end) },
                recv(notice_rx) -> n => if let Ok(n) = n { let _ = self.events.send(EngineEvent::Notice(n)); },
                recv(tick) -> _ => self.tick(),
            }
        }
    }

    fn apply_capture(&self) {
        let mut o = match &self.activity {
            Activity::Recording(r) => r.options().capture_options(),
            // The live view is never stored, and must show what the user
            // does while looking at it: Chud's own window is not excluded.
            _ => CaptureOptions { forward: self.live.is_some(), exclude_self: false, ..CaptureOptions::default() },
        };
        if let Activity::Playing(p) = &self.activity {
            o.watch_activity = p.abort_on_input;
        }
        if !matches!(self.activity, Activity::Recording(_)) && self.live.is_some() {
            o.excluded_processes = self.record_defaults.excluded_processes.clone();
        }
        self.platform.set_capture(o);
    }

    fn command(&mut self, cmd: Cmd, done: &Sender<PlayEnd>, notices: &Sender<Notice>) {
        match cmd {
            Cmd::StartRecording(opts) => self.start_recording(opts),
            Cmd::StopRecording => self.stop_recording(StopReason::User, &[]),
            Cmd::TogglePause => self.toggle_pause(),
            Cmd::Marker(label) => {
                if let Activity::Recording(r) = &mut self.activity {
                    r.marker(Instant::now(), label);
                }
            }
            Cmd::Play(job) => self.start_playback(*job, done, notices),
            Cmd::StopPlayback => self.stop_playback(),
            Cmd::Emergency => self.emergency(),
            Cmd::SetLive(on) => {
                self.live = if on { Some(self.live.unwrap_or_else(Instant::now)) } else { None };
                self.live_buf.clear();
                self.apply_capture();
            }
            Cmd::Arm(job) => self.armed = job,
            Cmd::SetHotkeys(h) => {
                self.platform.set_hotkeys(h.clone());
                self.hotkeys = h;
            }
            Cmd::SetRecordDefaults(o) => self.record_defaults = o,
            Cmd::Shutdown => {}
        }
        self.publish(true);
    }

    fn start_recording(&mut self, opts: RecordOptions) {
        if !matches!(self.activity, Activity::Idle) {
            return;
        }
        if opts.countdown_ms > 0 {
            let until = Instant::now() + Duration::from_millis(opts.countdown_ms);
            self.activity = Activity::Countdown { until, opts };
        } else {
            self.begin_recording(opts);
        }
    }

    fn begin_recording(&mut self, opts: RecordOptions) {
        let now = Instant::now();
        let fg = self.platform.foreground();
        let env = self.platform.environment();
        self.activity = Activity::Recording(Box::new(recorder::Recorder::new(opts, now, fg.as_ref(), env)));
        self.apply_capture();
    }

    /// Modifier keys of the hotkey bound to `action` (for residue removal).
    fn hotkey_modifier_keys(&self, action: HotkeyAction) -> Vec<Key> {
        let mut out = Vec::new();
        if let Some(hk) = self.hotkeys.iter().find(|h| h.action == action) {
            let m = hk.modifiers;
            let mut add = |codes: &[&str]| {
                for c in codes {
                    if let Some(i) = keys::by_code(c) {
                        out.push(Key::new(i.scan, i.vk));
                    }
                }
            };
            if m.ctrl {
                add(&["ControlLeft", "ControlRight"]);
            }
            if m.shift {
                add(&["ShiftLeft", "ShiftRight"]);
            }
            if m.alt {
                add(&["AltLeft", "AltRight"]);
            }
            if m.meta {
                add(&["MetaLeft", "MetaRight"]);
            }
        }
        out
    }

    fn stop_recording(&mut self, reason: StopReason, residue: &[Key]) {
        match std::mem::replace(&mut self.activity, Activity::Idle) {
            Activity::Recording(r) => {
                let rec = r.finish(Instant::now(), reason, residue);
                self.apply_capture();
                let _ = self.events.send(EngineEvent::RecordingFinished { recording: rec });
            }
            Activity::Countdown { .. } => self.apply_capture(),
            other => self.activity = other,
        }
    }

    fn toggle_pause(&mut self) {
        match &mut self.activity {
            Activity::Recording(r) => {
                let now = Instant::now();
                if r.is_paused() {
                    let p = self.platform.clone();
                    r.resume(now, |k| still_held(p.as_ref(), k));
                } else {
                    r.pause(now);
                }
            }
            Activity::Playing(p) => {
                let v = p.shared.paused.load(Ordering::Acquire);
                p.shared.paused.store(!v, Ordering::Release);
            }
            _ => {}
        }
    }

    fn start_playback(&mut self, job: PlayJob, done: &Sender<PlayEnd>, notices: &Sender<Notice>) {
        if !matches!(self.activity, Activity::Idle) {
            return;
        }
        let shared = Arc::new(PlayerShared::default());
        let s2 = shared.clone();
        let platform = self.platform.clone();
        let resolver = self.resolver.clone();
        let done = done.clone();
        let notices = notices.clone();
        let state = PlayingState {
            shared,
            handle: None,
            macro_id: job.macro_id.clone(),
            name: job.name.clone(),
            iterations: job.settings.repeat,
            abort_on_input: job.settings.abort_on_user_input,
            started: Instant::now(),
        };
        let handle = std::thread::Builder::new().name("chud-player".into()).spawn(move || {
            let end = player::run_job(&job, platform.as_ref(), resolver.as_ref(), &s2, &notices);
            let _ = done.send(end);
        });
        match handle {
            Ok(h) => {
                self.activity = Activity::Playing(PlayingState { handle: Some(h), ..state });
                self.apply_capture();
            }
            Err(e) => {
                let _ = self.events.send(EngineEvent::Notice(Notice { level: "error".into(), message: e.to_string() }));
            }
        }
    }

    fn stop_playback(&mut self) {
        if let Activity::Playing(p) = &self.activity {
            p.shared.stop.store(true, Ordering::Release);
            p.shared.paused.store(false, Ordering::Release);
        }
    }

    fn playback_done(&mut self, end: PlayEnd) {
        if let Activity::Playing(mut p) = std::mem::replace(&mut self.activity, Activity::Idle) {
            if let Some(h) = p.handle.take() {
                let _ = h.join();
            }
            self.apply_capture();
            let _ = self.events.send(EngineEvent::PlaybackFinished { end, macro_id: p.macro_id });
        }
        self.publish(true);
    }

    fn emergency(&mut self) {
        self.stop_playback();
        if matches!(self.activity, Activity::Recording(_) | Activity::Countdown { .. }) {
            self.stop_recording(StopReason::Emergency, &[]);
        }
        let _ = self.events.send(EngineEvent::Notice(Notice { level: "warning".into(), message: "emergency_stop".into() }));
    }

    fn captured(&mut self, c: Captured, done: &Sender<PlayEnd>, notices: &Sender<Notice>) {
        match c {
            Captured::Input { t, kind, injected } => {
                if let Activity::Recording(r) = &mut self.activity {
                    r.input(t, kind);
                }
                if let Some(origin) = self.live {
                    let rel = t.saturating_duration_since(origin).as_micros() as Micros;
                    self.live_buf.push(LiveInput { t: rel, kind, injected });
                    if self.live_buf.len() > 20_000 {
                        self.live_buf.drain(..10_000);
                    }
                }
            }
            Captured::Hotkey { action, .. } => self.hotkey(action, done, notices),
            Captured::Foreground { t, window, excluded } => {
                if let Activity::Recording(r) = &mut self.activity {
                    r.foreground(t, &window, excluded);
                }
                if self.live.is_some() {
                    let _ = self.events.send(EngineEvent::Foreground { window });
                }
            }
            Captured::UserActivity { t } => {
                if let Activity::Playing(p) = &self.activity {
                    // Ignore the first instants (the click that started playback).
                    if p.abort_on_input && t.saturating_duration_since(p.started) > Duration::from_millis(400) {
                        self.stop_playback();
                        let _ = self.events.send(EngineEvent::Notice(Notice {
                            level: "info".into(),
                            message: "stopped_by_user_input".into(),
                        }));
                    }
                }
            }
            Captured::Warning { message, .. } => {
                let _ = self.events.send(EngineEvent::Notice(Notice { level: "warning".into(), message }));
            }
        }
    }

    fn hotkey(&mut self, action: HotkeyAction, done: &Sender<PlayEnd>, notices: &Sender<Notice>) {
        match action {
            HotkeyAction::EmergencyStop => self.emergency(),
            HotkeyAction::ToggleRecord => match self.activity {
                Activity::Idle => self.start_recording(self.record_defaults.clone()),
                Activity::Countdown { .. } | Activity::Recording(_) => {
                    let residue = self.hotkey_modifier_keys(action);
                    self.stop_recording(StopReason::Hotkey, &residue);
                }
                Activity::Playing(_) => {}
            },
            HotkeyAction::TogglePause => self.toggle_pause(),
            HotkeyAction::TogglePlay => match &self.activity {
                Activity::Playing(_) => self.stop_playback(),
                Activity::Idle => {
                    if let Some(job) = self.armed.clone() {
                        self.start_playback(*job, done, notices);
                    } else {
                        let _ = self.events.send(EngineEvent::Hotkey { action });
                    }
                }
                _ => {}
            },
            HotkeyAction::AddMarker => {
                if let Activity::Recording(r) = &mut self.activity {
                    let n = r.event_count();
                    r.marker(Instant::now(), format!("M{}", n));
                } else {
                    let _ = self.events.send(EngineEvent::Hotkey { action });
                }
            }
            HotkeyAction::Trigger(_) => {
                let _ = self.events.send(EngineEvent::Hotkey { action });
            }
        }
        self.publish(true);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        match &self.activity {
            Activity::Countdown { until, opts } if now >= *until => {
                let opts = opts.clone();
                self.begin_recording(opts);
            }
            Activity::Recording(r) if r.reached_limit(now) => self.stop_recording(StopReason::MaxDuration, &[]),
            _ => {}
        }
        if !self.live_buf.is_empty() {
            let inputs = std::mem::take(&mut self.live_buf);
            let _ = self.events.send(EngineEvent::Live { inputs });
        }
        self.publish(false);
    }

    fn snapshot(&self) -> EngineStatus {
        let now = Instant::now();
        let mut s = EngineStatus {
            live: self.live.is_some(),
            armed: self.armed.as_ref().map(|j| j.macro_id.clone().unwrap_or_else(|| j.name.clone())),
            ..Default::default()
        };
        match &self.activity {
            Activity::Idle => {}
            Activity::Countdown { until, .. } => {
                s.mode = Mode::Countdown;
                s.countdown_ms = until.saturating_duration_since(now).as_millis() as u64;
            }
            Activity::Recording(r) => {
                s.mode = Mode::Recording;
                s.paused = r.is_paused();
                s.elapsed_us = r.elapsed(now);
                s.event_count = r.event_count();
                s.waiting_first_input = r.event_count() == 0 && r.options().start_on_first_input;
            }
            Activity::Playing(p) => {
                s.mode = Mode::Playing;
                s.paused = p.shared.paused.load(Ordering::Acquire);
                s.play_position_us = p.shared.position_us.load(Ordering::Relaxed);
                s.play_duration_us = p.shared.duration_us.load(Ordering::Relaxed);
                s.iteration = p.shared.iteration.load(Ordering::Relaxed);
                s.iterations = p.iterations;
                s.macro_id = p.macro_id.clone();
                s.macro_name = p.name.clone();
                s.block = p.shared.block.lock().clone();
            }
        }
        s
    }

    /// Publishes the status when it changed (at most every 50 ms while active).
    fn publish(&mut self, force: bool) {
        let s = self.snapshot();
        *self.status.lock() = s.clone();
        let changed = self.last_status.as_ref() != Some(&s);
        let due = self.last_status_at.elapsed() >= Duration::from_millis(50);
        if changed && (force || due || self.last_status.as_ref().is_none_or(|l| l.mode != s.mode || l.paused != s.paused)) {
            self.last_status = Some(s.clone());
            self.last_status_at = Instant::now();
            let _ = self.events.send(EngineEvent::Status(s));
        }
    }
}

fn still_held(p: &dyn Platform, k: &EventKind) -> bool {
    match *k {
        EventKind::KeyDown { key, .. } => p.key_held(key),
        EventKind::MouseDown { button, .. } => {
            let vk = match button {
                MouseButton::Left => 0x01,
                MouseButton::Right => 0x02,
                MouseButton::Middle => 0x04,
                MouseButton::X1 => 0x05,
                MouseButton::X2 => 0x06,
            };
            p.key_held(Key::new(0, vk))
        }
        _ => false,
    }
}
