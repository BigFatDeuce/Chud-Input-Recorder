//! Automatic triggers: play a macro when Chud starts, when a window becomes
//! active, or on a schedule.
//!
//! A macro that starts by itself must never surprise the user:
//! * off by default (one master switch, then one switch per rule);
//! * always announced by a countdown in the indicator, that can be cancelled;
//! * an emergency stop suspends every rule until the user resumes them;
//! * nothing starts while recording or playing, while an excluded application
//!   is in front, or on the lock screen;
//! * a window rule is dropped if its window is no longer active when the
//!   countdown ends.
//!
//! The active window is only looked at while an enabled rule needs it, and is
//! never stored or logged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{Datelike, NaiveDateTime};
use chud_core::playback::Selection;
use chud_engine::{EngineEvent, Mode};
use chud_platform::{WindowInfo, window_matches};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::App;
use crate::commands;

const POLL: Duration = Duration::from_millis(200);
/// A daily rule that could not start on time (Chud busy, computer asleep) may
/// still start this late; after that it waits for its next day.
const DAILY_GRACE_S: i64 = 120;
/// Windows reports no active window for an instant while switching, and for
/// good on the secure desktop.
const NO_WINDOW_PATIENCE: Duration = Duration::from_secs(2);
/// Windows' lock and sign-in screens.
const SECURE_SCREENS: [&str; 2] = ["LockApp.exe", "LogonUI.exe"];

// ─────────────────────────────── settings ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationSettings {
    pub enabled: bool,
    /// Countdown before a macro starts by itself.
    pub warning_s: u32,
    pub rules: Vec<Automation>,
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self { enabled: false, warning_s: 5, rules: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub macro_id: String,
    #[serde(default)]
    pub use_flow: bool,
    /// Stop playing when the user moves the mouse or presses a key.
    #[serde(default = "yes")]
    pub stop_on_input: bool,
    pub when: When,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum When {
    /// Chud has just started.
    Startup { delay_s: u32 },
    /// A window whose title and process contain these texts becomes active.
    Window {
        #[serde(default)]
        title: String,
        #[serde(default)]
        process: String,
        /// Not again before this long.
        cooldown_s: u32,
    },
    /// At a fixed time, on the chosen days (Monday first).
    Daily { hour: u32, minute: u32, days: [bool; 7] },
    Interval { minutes: u32 },
}

impl AutomationSettings {
    /// Brings every value back in range.
    pub fn sanitize(&mut self) {
        self.warning_s = self.warning_s.clamp(3, 60);
        let mut seen = std::collections::HashSet::new();
        self.rules.retain(|r| !r.id.is_empty() && seen.insert(r.id.clone()));
        for r in &mut self.rules {
            match &mut r.when {
                When::Startup { delay_s } => *delay_s = (*delay_s).min(3_600),
                When::Window { title, process, cooldown_s } => {
                    *title = title.trim().to_string();
                    *process = process.trim().to_string();
                    *cooldown_s = (*cooldown_s).clamp(3, 86_400);
                }
                When::Daily { hour, minute, .. } => {
                    *hour = (*hour).min(23);
                    *minute = (*minute).min(59);
                }
                When::Interval { minutes } => *minutes = (*minutes).clamp(1, 7 * 24 * 60),
            }
        }
    }

    fn watches_windows(&self) -> bool {
        self.rules.iter().any(|r| r.enabled && matches!(r.when, When::Window { .. }))
    }
}

impl When {
    /// Whether `w` is the window this rule waits for. A rule without any
    /// text matches nothing (rather than every window).
    fn matches(&self, w: Option<&WindowInfo>) -> bool {
        match (self, w) {
            (When::Window { title, process, .. }, Some(w)) => {
                !(title.is_empty() && process.is_empty()) && window_matches(w, title, process)
            }
            _ => false,
        }
    }
}

// ─────────────────────────────── scheduler ──────────────────────────────

/// Decides which rule starts, and when. No clock and no system access of its
/// own: everything comes from [`Tick`].
#[derive(Default)]
pub struct Scheduler {
    states: HashMap<String, RuleState>,
    /// The first look at the rules is over: startup rules seen later (added
    /// or switched on afterwards) wait for the next start.
    started: bool,
}

struct RuleState {
    when: When,
    /// Window: the window was active at the last look.
    matched: bool,
    last_fire_ms: Option<u64>,
    /// Startup and interval: when to start (monotonic).
    due_ms: Option<u64>,
    /// Daily: the last occurrence already dealt with.
    last_daily: Option<NaiveDateTime>,
}

pub struct Tick<'a> {
    /// Monotonic clock.
    pub now_ms: u64,
    pub local: NaiveDateTime,
    /// `None` also when Chud itself or an excluded application is in front.
    pub foreground: Option<&'a WindowInfo>,
    /// Nothing is recording, playing or about to.
    pub can_start: bool,
}

/// The occurrence of a daily rule that is due now (within its grace period).
fn daily_occurrence(hour: u32, minute: u32, days: &[bool; 7], local: NaiveDateTime) -> Option<NaiveDateTime> {
    // Yesterday too: 23:59 is still within its grace period after midnight.
    [0, 1].into_iter().find_map(|back| {
        let at = (local.date() - chrono::Duration::days(back)).and_hms_opt(hour, minute, 0)?;
        let late = (local - at).num_seconds();
        (local >= at && late < DAILY_GRACE_S && days[at.weekday().num_days_from_monday() as usize]).then_some(at)
    })
}

impl Scheduler {
    /// Rules are off or suspended: forget everything, so that nothing missed
    /// meanwhile starts when they come back.
    pub fn idle(&mut self) {
        self.states.clear();
        self.started = true;
    }

    /// The rule to start now, if any.
    pub fn step(&mut self, rules: &[Automation], t: &Tick) -> Option<String> {
        self.states.retain(|id, _| rules.iter().any(|r| r.enabled && r.id == *id));
        let first = !std::mem::replace(&mut self.started, true);
        let mut fire = None;
        for r in rules.iter().filter(|r| r.enabled) {
            if self.states.get(&r.id).is_none_or(|s| s.when != r.when) {
                self.states.insert(r.id.clone(), RuleState::new(&r.when, t, first));
            }
            let Some(s) = self.states.get_mut(&r.id) else { continue };
            let free = t.can_start && fire.is_none();
            if s.due(t, free) {
                s.last_fire_ms = Some(t.now_ms);
                fire = Some(r.id.clone());
            }
        }
        fire
    }
}

impl RuleState {
    /// A rule seen for the first time never starts right away: what is
    /// already true (window active, time reached) is taken as the baseline.
    fn new(when: &When, t: &Tick, at_startup: bool) -> Self {
        Self {
            matched: when.matches(t.foreground),
            last_fire_ms: None,
            due_ms: match when {
                When::Startup { delay_s } if at_startup => Some(t.now_ms + u64::from(*delay_s) * 1_000),
                When::Interval { minutes } => Some(t.now_ms + u64::from(*minutes) * 60_000),
                _ => None,
            },
            last_daily: match when {
                When::Daily { hour, minute, days } => daily_occurrence(*hour, *minute, days, t.local),
                _ => None,
            },
            when: when.clone(),
        }
    }

    /// Whether the rule starts now. `free`: it may (otherwise window changes
    /// are missed on purpose, and timed rules wait).
    fn due(&mut self, t: &Tick, free: bool) -> bool {
        match &self.when {
            When::Window { cooldown_s, .. } => {
                let was = std::mem::replace(&mut self.matched, self.when.matches(t.foreground));
                let rested = self.last_fire_ms.is_none_or(|l| t.now_ms.saturating_sub(l) >= u64::from(*cooldown_s) * 1_000);
                self.matched && !was && free && rested
            }
            When::Daily { hour, minute, days } => match daily_occurrence(*hour, *minute, days, t.local) {
                Some(at) if free && self.last_daily != Some(at) => {
                    self.last_daily = Some(at);
                    true
                }
                _ => false,
            },
            When::Startup { .. } => free && self.due_ms.take_if(|d| t.now_ms >= *d).is_some(),
            When::Interval { minutes } => {
                let due = free && self.due_ms.is_some_and(|d| t.now_ms >= d);
                if due {
                    self.due_ms = Some(t.now_ms + u64::from(*minutes) * 60_000);
                }
                due
            }
        }
    }
}

// ──────────────────────────────── runtime ───────────────────────────────

/// What the interface shows about the rules.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AutomationStatus {
    pub enabled: bool,
    /// Stopped by an emergency stop (or by the user) until resumed.
    pub suspended: bool,
    pub pending: Option<PendingStatus>,
    /// Last outcome of each rule since Chud started.
    pub last: HashMap<String, Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PendingStatus {
    pub rule: String,
    pub macro_name: String,
    pub remaining_s: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub at_ms: i64,
    /// "started" or why it did not: "cancelled", "busy", "blocked", "left", "failed".
    pub what: &'static str,
}

#[derive(Default)]
pub struct Shared {
    suspended: AtomicBool,
    cancel: AtomicBool,
    /// A macro is being announced: the indicator must stay visible.
    announcing: AtomicBool,
    status: Mutex<AutomationStatus>,
}

impl Shared {
    pub fn suspend(&self, on: bool) {
        self.suspended.store(on, Ordering::Release);
    }
    pub fn announcing(&self) -> bool {
        self.announcing.load(Ordering::Acquire)
    }
    pub fn toggle(&self) {
        self.suspended.fetch_xor(true, Ordering::AcqRel);
    }
}

struct Pending {
    rule: Automation,
    name: String,
    due: Instant,
    /// Since when no window has been active.
    no_window: Option<Instant>,
}

fn notice(app: &AppHandle, level: &str, message: String) {
    let _ = app.emit("engine-event", serde_json::json!({"type": "notice", "level": level, "message": message}));
}

/// Nothing may start by itself after an emergency stop, until the user says so.
pub fn watch(app: &AppHandle, ev: &EngineEvent) {
    let EngineEvent::Notice(n) = ev else { return };
    let state = app.state::<App>();
    if n.message == "emergency_stop" && state.settings.read().automation.enabled {
        state.automation.suspend(true);
        state.engine.stop_playback(); // see `start`
        log::info!("automation suspended by an emergency stop");
    }
}

/// Windows Chud must leave alone: excluded applications, secure screens.
fn off_limits(state: &App, w: &WindowInfo) -> bool {
    SECURE_SCREENS.iter().any(|p| p.eq_ignore_ascii_case(&w.process))
        || state.settings.read().record.excluded_processes.iter().any(|p| p.eq_ignore_ascii_case(&w.process))
}

pub fn run(app: AppHandle) {
    let origin = Instant::now();
    let mut scheduler = Scheduler::default();
    let mut pending: Option<Pending> = None;
    let mut last: HashMap<String, Outcome> = HashMap::new();
    loop {
        std::thread::sleep(POLL);
        let state = app.state::<App>();
        let cfg = state.settings.read().automation.clone();
        let shared = &state.automation;
        if !cfg.enabled {
            shared.suspend(false);
        }
        let active = cfg.enabled && !shared.suspended.load(Ordering::Acquire);
        let cancelled = shared.cancel.swap(false, Ordering::AcqRel);
        let mode = state.engine.status().mode;
        let foreground =
            if active && (pending.is_some() || cfg.watches_windows()) { state.engine.platform().foreground() } else { None };

        let mut can_start = pending.is_none() && mode == Mode::Idle && state.snapshot.lock().is_none();
        if let Some(p) = &mut pending {
            let verdict = if cancelled || !active || !cfg.rules.contains(&p.rule) {
                Some("cancelled")
            } else if mode != Mode::Idle || state.snapshot.lock().is_some() {
                Some("busy")
            } else {
                judge(&state, p, foreground.as_ref())
            };
            if let Some(what) = verdict {
                let p = pending.take().expect("pending");
                can_start = false;
                shared.announcing.store(false, Ordering::Release);
                let what = if what == "start" { start(&app, &state, &p) } else { what };
                if what != "started" {
                    hide_indicator(&app, &state);
                }
                if matches!(what, "busy" | "blocked" | "left") {
                    notice(&app, "info", format!("automation_{what}: {}", p.name));
                }
                log::info!("automation {}: {what}", p.rule.id);
                last.insert(p.rule.id.clone(), Outcome { at_ms: chrono::Utc::now().timestamp_millis(), what });
            }
        }

        if active {
            let own = std::process::id();
            let seen = foreground.as_ref().filter(|w| w.pid != own && !off_limits(&state, w));
            let tick = Tick {
                now_ms: origin.elapsed().as_millis() as u64,
                local: chrono::Local::now().naive_local(),
                foreground: seen,
                can_start,
            };
            if let Some(rule) = scheduler.step(&cfg.rules, &tick).and_then(|id| cfg.rules.iter().find(|r| r.id == id)) {
                match commands::macro_name(&state, &rule.macro_id) {
                    Ok(name) => {
                        pending = Some(Pending {
                            rule: rule.clone(),
                            name,
                            due: Instant::now() + Duration::from_secs(cfg.warning_s.into()),
                            no_window: None,
                        });
                        shared.cancel.store(false, Ordering::Release);
                        shared.announcing.store(true, Ordering::Release);
                        crate::show_indicator(&app);
                    }
                    Err(e) => {
                        log::warn!("automation {}: {e}", rule.id);
                        notice(&app, "error", format!("automation_failed: {e}"));
                        last.insert(rule.id.clone(), Outcome { at_ms: chrono::Utc::now().timestamp_millis(), what: "failed" });
                    }
                }
            }
        } else {
            scheduler.idle();
        }

        last.retain(|id, _| cfg.rules.iter().any(|r| r.id == *id));
        let status = AutomationStatus {
            enabled: cfg.enabled,
            suspended: cfg.enabled && !active,
            pending: pending.as_ref().map(|p| PendingStatus {
                rule: p.rule.id.clone(),
                macro_name: p.name.clone(),
                remaining_s: p.due.saturating_duration_since(Instant::now()).as_millis().div_ceil(1_000) as u64,
            }),
            last: last.clone(),
        };
        let mut shown = shared.status.lock();
        if *shown != status {
            *shown = status.clone();
            drop(shown);
            let _ = app.emit("automation-status", status);
        }
    }
}

/// What becomes of an announced macro: `None` while the countdown runs.
fn judge(state: &App, p: &mut Pending, foreground: Option<&WindowInfo>) -> Option<&'static str> {
    let now = Instant::now();
    match foreground {
        Some(w) if off_limits(state, w) => return Some("blocked"),
        // The system has switched to a screen no program may drive.
        None if cfg!(windows) => {
            if now.duration_since(*p.no_window.get_or_insert(now)) >= NO_WINDOW_PATIENCE {
                return Some("blocked");
            }
        }
        _ => p.no_window = None,
    }
    if now < p.due || p.no_window.is_some() {
        return None;
    }
    if matches!(p.rule.when, When::Window { .. }) && !p.rule.when.matches(foreground) {
        return Some("left");
    }
    Some("start")
}

fn start(app: &AppHandle, state: &App, p: &Pending) -> &'static str {
    match commands::job_for(state, &p.rule.macro_id, None, Selection::All, p.rule.use_flow) {
        Ok(mut job) => {
            // Loading may have taken a while: an emergency stop or a
            // cancellation made meanwhile still counts.
            let stopped = || state.automation.suspended.load(Ordering::Acquire);
            if stopped() || state.automation.cancel.load(Ordering::Acquire) {
                return "cancelled";
            }
            job.settings.abort_on_user_input |= p.rule.stop_on_input;
            commands::step_aside(app, state, true);
            state.engine.play(job);
            // An emergency stop handled just before `play` had nothing to
            // stop. Either it is seen here, or `watch`, which also stops
            // playback after suspending, comes after `play`.
            if stopped() {
                state.engine.stop_playback();
            }
            "started"
        }
        Err(e) => {
            notice(app, "error", format!("automation_failed: {e}"));
            "failed"
        }
    }
}

/// Hides the indicator after an announcement. A recording that starts at the
/// same instant must keep it: checked again once hidden.
fn hide_indicator(app: &AppHandle, state: &App) {
    if state.engine.status().mode != Mode::Idle {
        return;
    }
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
    if matches!(state.engine.status().mode, Mode::Recording | Mode::Countdown) {
        crate::show_indicator(app);
    }
}

// ─────────────────────────────── commands ───────────────────────────────

#[tauri::command]
pub fn automation_status(state: State<App>) -> AutomationStatus {
    state.automation.status.lock().clone()
}

/// Cancels the announced macro.
#[tauri::command]
pub fn automation_cancel(state: State<App>) {
    state.automation.cancel.store(true, Ordering::Release);
}

#[tauri::command]
pub fn automation_suspend(state: State<App>, on: bool) {
    state.automation.suspend(on);
}

/// Name and title of the window that is active after `delay_ms` (to fill in a
/// window rule). Asked for by the user, never kept.
#[tauri::command]
pub async fn foreground_after(state: State<'_, App>, delay_ms: u64) -> Result<Option<WindowInfo>, String> {
    let platform = state.engine.platform().clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(delay_ms.min(10_000)));
        platform.foreground().filter(|w| w.pid != std::process::id())
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, when: When) -> Automation {
        Automation { id: id.into(), enabled: true, macro_id: "m".into(), use_flow: false, stop_on_input: true, when }
    }
    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn win(process: &str, title: &str) -> WindowInfo {
        WindowInfo { process: process.into(), title: title.into(), ..Default::default() }
    }
    fn tick(now_s: u64, fg: Option<&WindowInfo>, can_start: bool) -> Tick<'_> {
        Tick { now_ms: now_s * 1_000, local: at("2026-11-23 12:00:00"), foreground: fg, can_start }
    }
    fn clock(s: &str, can_start: bool) -> Tick<'static> {
        Tick { now_ms: 0, local: at(s), foreground: None, can_start }
    }
    fn editor(cooldown_s: u32) -> When {
        When::Window { title: String::new(), process: "notepad".into(), cooldown_s }
    }

    #[test]
    fn startup_fires_once_and_only_for_rules_present_at_start() {
        let mut s = Scheduler::default();
        let mut rules = vec![rule("a", When::Startup { delay_s: 5 })];
        assert_eq!(s.step(&rules, &tick(0, None, true)), None);
        assert_eq!(s.step(&rules, &tick(4, None, true)), None);
        assert_eq!(s.step(&rules, &tick(5, None, false)), None, "busy: waits");
        assert_eq!(s.step(&rules, &tick(6, None, true)).as_deref(), Some("a"));
        rules.push(rule("b", When::Startup { delay_s: 0 }));
        for n in 7..60 {
            assert_eq!(s.step(&rules, &tick(n, None, true)), None);
        }
    }

    #[test]
    fn startup_does_not_fire_when_automation_is_switched_on_later() {
        let mut s = Scheduler::default();
        s.idle();
        let rules = [rule("a", When::Startup { delay_s: 0 })];
        assert_eq!(s.step(&rules, &tick(1, None, true)), None);
        assert_eq!(s.step(&rules, &tick(60, None, true)), None);
    }

    #[test]
    fn window_fires_on_activation_only() {
        let mut s = Scheduler::default();
        let rules = [rule("a", editor(10))];
        let note = win("Notepad.exe", "x");
        let other = win("game.exe", "y");
        // Already active when first seen: baseline, nothing starts.
        assert_eq!(s.step(&rules, &tick(0, Some(&note), true)), None);
        assert_eq!(s.step(&rules, &tick(1, Some(&other), true)), None);
        assert_eq!(s.step(&rules, &tick(2, Some(&note), true)).as_deref(), Some("a"));
        assert_eq!(s.step(&rules, &tick(3, Some(&note), true)), None);
        // Back within the cooldown: missed, and not made up for later.
        assert_eq!(s.step(&rules, &tick(4, Some(&other), true)), None);
        assert_eq!(s.step(&rules, &tick(5, Some(&note), true)), None);
        assert_eq!(s.step(&rules, &tick(30, Some(&note), true)), None);
        assert_eq!(s.step(&rules, &tick(31, None, true)), None);
        assert_eq!(s.step(&rules, &tick(32, Some(&note), true)).as_deref(), Some("a"));
    }

    #[test]
    fn window_changes_made_while_playing_are_ignored() {
        let mut s = Scheduler::default();
        let rules = [rule("a", editor(3))];
        let note = win("notepad.exe", "");
        s.step(&rules, &tick(0, None, true));
        assert_eq!(s.step(&rules, &tick(5, Some(&note), false)), None);
        assert_eq!(s.step(&rules, &tick(6, Some(&note), true)), None);
    }

    #[test]
    fn empty_window_rule_matches_nothing() {
        let mut s = Scheduler::default();
        let rules = [rule("a", When::Window { title: String::new(), process: String::new(), cooldown_s: 3 })];
        s.step(&rules, &tick(0, None, true));
        assert_eq!(s.step(&rules, &tick(5, Some(&win("x.exe", "x")), true)), None);
    }

    #[test]
    fn daily_respects_days_grace_and_fires_once() {
        // 2026-11-23 is a Monday.
        let mut days = [false; 7];
        days[0] = true;
        let rules = [rule("a", When::Daily { hour: 8, minute: 30, days })];
        let mut s = Scheduler::default();
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:29:59", true)), None);
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:30:00", false)), None, "busy: waits");
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:30:40", true)).as_deref(), Some("a"));
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:31:00", true)), None);
        assert_eq!(s.step(&rules, &clock("2026-11-24 08:30:00", true)), None, "Tuesday");
        // Next Monday, but the computer was asleep: too late.
        assert_eq!(s.step(&rules, &clock("2026-11-30 08:33:00", true)), None);
        assert_eq!(s.step(&rules, &clock("2026-12-07 08:30:01", true)).as_deref(), Some("a"));
    }

    #[test]
    fn daily_survives_midnight_and_clock_going_back() {
        let rules = [rule("a", When::Daily { hour: 23, minute: 59, days: [true; 7] })];
        let mut s = Scheduler::default();
        s.step(&rules, &clock("2026-11-23 23:58:00", true));
        assert_eq!(s.step(&rules, &clock("2026-11-24 00:00:10", true)).as_deref(), Some("a"));
        // The clock goes back over the same time: not a second time.
        assert_eq!(s.step(&rules, &clock("2026-11-23 23:59:30", true)), None);
    }

    #[test]
    fn daily_created_just_after_its_time_waits_for_tomorrow() {
        let rules = [rule("a", When::Daily { hour: 8, minute: 0, days: [true; 7] })];
        let mut s = Scheduler::default();
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:00:30", true)), None);
        assert_eq!(s.step(&rules, &clock("2026-11-23 08:00:31", true)), None);
        assert_eq!(s.step(&rules, &clock("2026-11-24 08:00:00", true)).as_deref(), Some("a"));
    }

    #[test]
    fn interval_counts_from_the_last_start() {
        let rules = [rule("a", When::Interval { minutes: 1 })];
        let mut s = Scheduler::default();
        assert_eq!(s.step(&rules, &tick(0, None, true)), None);
        assert_eq!(s.step(&rules, &tick(60, None, false)), None, "busy: waits");
        assert_eq!(s.step(&rules, &tick(65, None, true)).as_deref(), Some("a"));
        assert_eq!(s.step(&rules, &tick(124, None, true)), None);
        assert_eq!(s.step(&rules, &tick(125, None, true)).as_deref(), Some("a"));
    }

    #[test]
    fn one_rule_at_a_time_the_other_waits() {
        let rules = [rule("a", When::Interval { minutes: 1 }), rule("b", When::Interval { minutes: 1 })];
        let mut s = Scheduler::default();
        s.step(&rules, &tick(0, None, true));
        assert_eq!(s.step(&rules, &tick(60, None, true)).as_deref(), Some("a"));
        assert_eq!(s.step(&rules, &tick(61, None, false)), None);
        assert_eq!(s.step(&rules, &tick(62, None, true)).as_deref(), Some("b"));
    }

    #[test]
    fn suspension_forgets_what_was_due() {
        let rules = [rule("a", When::Interval { minutes: 1 }), rule("b", editor(3))];
        let mut s = Scheduler::default();
        s.step(&rules, &tick(0, None, true));
        s.idle();
        let note = win("notepad.exe", "");
        // Back: the interval starts again, the active window is the baseline.
        assert_eq!(s.step(&rules, &tick(300, Some(&note), true)), None);
        assert_eq!(s.step(&rules, &tick(359, Some(&note), true)), None);
        assert_eq!(s.step(&rules, &tick(360, Some(&note), true)).as_deref(), Some("a"));
    }

    #[test]
    fn disabled_and_edited_rules() {
        let mut rules = vec![rule("a", When::Interval { minutes: 1 })];
        let mut s = Scheduler::default();
        s.step(&rules, &tick(0, None, true));
        rules[0].enabled = false;
        assert_eq!(s.step(&rules, &tick(61, None, true)), None);
        // Back on, then edited: each time the wait starts over.
        rules[0].enabled = true;
        assert_eq!(s.step(&rules, &tick(62, None, true)), None);
        rules[0].when = When::Interval { minutes: 2 };
        assert_eq!(s.step(&rules, &tick(130, None, true)), None);
        assert_eq!(s.step(&rules, &tick(250, None, true)).as_deref(), Some("a"));
    }

    #[test]
    fn settings_are_brought_back_in_range() {
        let mut s = AutomationSettings {
            enabled: true,
            warning_s: 0,
            rules: vec![
                rule("a", When::Interval { minutes: 0 }),
                rule("a", When::Interval { minutes: 5 }),
                rule("", When::Interval { minutes: 5 }),
                rule("d", When::Daily { hour: 99, minute: 99, days: [true; 7] }),
                rule("w", When::Window { title: " x ".into(), process: " ".into(), cooldown_s: 0 }),
            ],
        };
        s.sanitize();
        assert_eq!(s.warning_s, 3, "the countdown cannot be removed");
        assert_eq!(s.rules.len(), 3);
        assert_eq!(s.rules[0].when, When::Interval { minutes: 1 });
        assert_eq!(s.rules[1].when, When::Daily { hour: 23, minute: 59, days: [true; 7] });
        assert_eq!(s.rules[2].when, When::Window { title: "x".into(), process: String::new(), cooldown_s: 3 });
    }

    #[test]
    fn off_by_default_and_json_roundtrip() {
        let d: AutomationSettings = serde_json::from_str("{}").unwrap();
        assert!(!d.enabled && d.warning_s == 5 && d.rules.is_empty());
        let s = AutomationSettings { enabled: true, warning_s: 5, rules: vec![rule("a", editor(10))] };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains(r#""type":"window""#), "{j}");
        assert_eq!(serde_json::from_str::<AutomationSettings>(&j).unwrap(), s);
    }
}
