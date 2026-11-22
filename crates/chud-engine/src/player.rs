//! The playback thread: precise scheduling, pause/resume, safety release,
//! repeats, and the Flow host.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use chud_core::document::{CoordMap, Environment, MacroDoc};
use chud_core::event::{Control, Edge, Key, Micros, PadAxis};
use chud_core::flow::{Flow, FlowError, FlowHost, Interpreter, Target, parse_color};
use chud_core::playback::{self, Action, Plan, PlaybackSettings, Selection, TimingMode};
use chud_core::timeline::Timeline;
use chud_core::vision::{self, FindOptions, Found, Image, Region};
use chud_platform::timer::PreciseSleeper;
use chud_platform::{Injector, Platform, PlatformError};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use serde::Serialize;

/// Loads other macros for `PlayMacro` blocks.
pub trait MacroResolver: Send + Sync {
    fn load(&self, id: &str) -> Option<MacroDoc>;
}

pub struct NoMacros;
impl MacroResolver for NoMacros {
    fn load(&self, _: &str) -> Option<MacroDoc> {
        None
    }
}

/// What to play.
#[derive(Clone)]
pub struct PlayJob {
    pub macro_id: Option<String>,
    pub name: String,
    pub timeline: Arc<Timeline>,
    pub env: Environment,
    pub flow: Option<Flow>,
    pub use_flow: bool,
    pub settings: PlaybackSettings,
    pub selection: Selection,
    /// Decoded images of the macro, for "find on screen" blocks.
    pub images: Arc<HashMap<String, Image>>,
}

impl PlayJob {
    /// The job that plays a whole document (its flow when it has one).
    pub fn of_doc(doc: &MacroDoc, settings: PlaybackSettings, selection: Selection, use_flow: bool) -> PlayJob {
        PlayJob {
            macro_id: Some(doc.meta.id.clone()),
            name: doc.meta.name.clone(),
            timeline: Arc::new(doc.timeline.clone()),
            env: doc.env.clone(),
            use_flow: use_flow && doc.flow.as_ref().is_some_and(|f| !f.blocks.is_empty()),
            flow: doc.flow.clone(),
            settings,
            selection,
            images: Arc::new(doc.decoded_images()),
        }
    }
}

#[derive(Default)]
pub struct PlayerShared {
    pub stop: AtomicBool,
    pub paused: AtomicBool,
    /// Current position on the source timeline (µs).
    pub position_us: AtomicI64,
    pub duration_us: AtomicI64,
    pub iteration: AtomicU32,
    pub block: Mutex<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum PlayEnd {
    Completed,
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub level: String,
    pub message: String,
}

enum Interrupt {
    Stop,
}

struct Ctx<'a> {
    injector: Box<dyn Injector>,
    sleeper: PreciseSleeper,
    platform: &'a dyn Platform,
    shared: &'a PlayerShared,
    /// Pressed controls in press order, with the action that pressed them.
    pressed: Vec<(Control, Action)>,
    /// Gamepad axes away from rest, with their current value.
    axes: Vec<(u8, PadAxis, i16)>,
    notices: &'a Sender<Notice>,
    warned_pad: bool,
    warned_blocked: bool,
    /// Start of the run (the flow's clock).
    epoch: Instant,
}

impl Ctx<'_> {
    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    fn track(&mut self, a: &Action) {
        if let Action::PadAxis { pad, axis, value } = *a {
            self.axes.retain(|(p, x, _)| (*p, *x) != (pad, axis));
            if value != 0 {
                self.axes.push((pad, axis, value));
            }
        }
        if let Some((c, edge)) = a.control() {
            match edge {
                Edge::Down => {
                    if !self.pressed.iter().any(|(p, _)| *p == c) {
                        self.pressed.push((c, *a));
                    }
                }
                Edge::Up => self.pressed.retain(|(p, _)| *p != c),
            }
        }
    }

    fn warn_pad(&mut self) {
        if !self.warned_pad {
            self.warned_pad = true;
            let _ = self.notices.send(Notice { level: "warning".into(), message: "gamepad_output_unavailable".into() });
        }
    }

    fn send(&mut self, batch: &[Action]) {
        match self.injector.send(batch) {
            Ok(()) => {}
            Err(PlatformError::NoGamepadOutput) => self.warn_pad(),
            Err(PlatformError::Blocked) => {
                if !self.warned_blocked {
                    self.warned_blocked = true;
                    let _ = self.notices.send(Notice { level: "warning".into(), message: "input_blocked".into() });
                }
            }
            Err(e) => {
                let _ = self.notices.send(Notice { level: "error".into(), message: e.to_string() });
            }
        }
        for a in batch {
            self.track(a);
        }
    }

    /// Releases everything currently held (reverse press order) and returns
    /// gamepad axes to rest. Returns the actions that restore that state.
    fn release_all(&mut self) -> Vec<Action> {
        let held = std::mem::take(&mut self.pressed);
        let axes = std::mem::take(&mut self.axes);
        let mut releases: Vec<Action> = held.iter().rev().filter_map(|(_, a)| a.release()).collect();
        releases.extend(axes.iter().map(|&(pad, axis, _)| Action::PadAxis { pad, axis, value: 0 }));
        if !releases.is_empty() {
            let _ = self.injector.send(&releases);
        }
        let mut restore: Vec<Action> = held.into_iter().map(|(_, a)| a).collect();
        restore.extend(axes.into_iter().map(|(pad, axis, value)| Action::PadAxis { pad, axis, value }));
        restore
    }

    /// Waits until `deadline`, handling pause (release + re-press) and stop.
    /// Returns how long playback was paused, to shift the schedule.
    fn wait(&mut self, deadline: Instant) -> Result<Duration, Interrupt> {
        let mut paused_total = Duration::ZERO;
        let mut deadline = deadline;
        loop {
            let shared = self.shared;
            let finished = self
                .sleeper
                .sleep_until(deadline, || !shared.stop.load(Ordering::Acquire) && !shared.paused.load(Ordering::Acquire));
            if self.stopped() {
                return Err(Interrupt::Stop);
            }
            if finished {
                return Ok(paused_total);
            }
            // Paused: give the user their keyboard and mouse back.
            let began = Instant::now();
            let held = self.release_all();
            while self.shared.paused.load(Ordering::Acquire) {
                if self.stopped() {
                    return Err(Interrupt::Stop);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            if !held.is_empty() {
                self.send(&held);
            }
            let d = began.elapsed();
            paused_total += d;
            deadline += d;
        }
    }

    fn run_plan(&mut self, plan: &Plan, timing: TimingMode, max_lag: Duration) -> Result<(), Interrupt> {
        let mut anchor = Instant::now();
        let mut last_dispatch = anchor;
        let mut prev_at: Micros = 0;
        let steps = &plan.steps;
        let mut i = 0;
        let mut batch: Vec<Action> = Vec::with_capacity(16);
        self.shared.duration_us.store(plan.source_time(plan.duration), Ordering::Relaxed);
        while i < steps.len() {
            let at = steps[i].at;
            let due = match timing {
                TimingMode::Relative => last_dispatch + Duration::from_micros((at - prev_at).max(0) as u64),
                _ => anchor + Duration::from_micros(at.max(0) as u64),
            };
            let paused = self.wait(due)?;
            anchor += paused;
            if timing == TimingMode::Adaptive {
                let now = Instant::now();
                let target = anchor + Duration::from_micros(at.max(0) as u64);
                if now > target + max_lag {
                    // Too late: shift the rest of the timeline instead of bursting.
                    anchor += now - target;
                }
            }
            batch.clear();
            while i < steps.len() && steps[i].at == at && batch.len() < 64 {
                batch.push(steps[i].action);
                i += 1;
            }
            self.send(&batch.clone());
            last_dispatch = Instant::now();
            prev_at = at;
            self.shared.position_us.store(plan.source_time(at), Ordering::Relaxed);
        }
        Ok(())
    }
}

struct Host<'a, 'b> {
    ctx: &'a mut Ctx<'b>,
    job: &'a PlayJob,
    resolver: &'a dyn MacroResolver,
    depth: u32,
    rng: u64,
}

impl Host<'_, '_> {
    fn interrupt(_: Interrupt) -> FlowError {
        FlowError::Cancelled
    }

    fn plan_for(&self, tl: &Timeline, env: &Environment, settings: &PlaybackSettings, sel: &Selection) -> Plan {
        let current = self.ctx.platform.monitors();
        let map = CoordMap::new(&env.monitors, &current);
        playback::compile(tl, settings, sel, Some(&map))
    }
}

impl FlowHost for Host<'_, '_> {
    fn perform(&mut self, action: Action) -> Result<(), FlowError> {
        if self.ctx.stopped() {
            return Err(FlowError::Cancelled);
        }
        self.ctx.send(&[action]);
        Ok(())
    }

    fn sleep(&mut self, us: Micros) -> Result<(), FlowError> {
        let due = Instant::now() + Duration::from_micros(us.max(0) as u64);
        self.ctx.wait(due).map(|_| ()).map_err(Self::interrupt)
    }

    fn check(&mut self) -> Result<(), FlowError> {
        self.ctx.wait(Instant::now()).map(|_| ()).map_err(Self::interrupt)
    }

    fn play_recording(&mut self, start: Option<Micros>, end: Option<Micros>, speed: f64) -> Result<(), FlowError> {
        let sel = match (start, end) {
            (None, None) => Selection::All,
            (Some(s), None) => Selection::From { t: s },
            (s, Some(e)) => Selection::Range { start: s.unwrap_or(0), end: e },
        };
        let mut settings = self.job.settings.clone();
        settings.speed = (self.job.settings.speed * speed).clamp(0.01, 100.0);
        let plan = self.plan_for(&self.job.timeline, &self.job.env, &settings, &sel);
        let max_lag = Duration::from_micros((settings.adaptive_max_lag_ms * 1_000.0) as u64);
        self.ctx.run_plan(&plan, settings.timing, max_lag).map_err(Self::interrupt)
    }

    fn play_macro(&mut self, id: &str) -> Result<(), FlowError> {
        if self.depth >= 8 {
            return Err(FlowError::TooDeep);
        }
        let Some(doc) = self.resolver.load(id) else {
            return Err(FlowError::Host(format!("macro {id} not found")));
        };
        if let Some(flow) = doc.flow.clone().filter(|f| !f.blocks.is_empty()) {
            let job = PlayJob::of_doc(&doc, doc.settings.clone(), Selection::All, true);
            let mut nested = Host { ctx: &mut *self.ctx, job: &job, resolver: self.resolver, depth: self.depth + 1, rng: self.rng };
            Interpreter::new(&mut nested).run(&flow)
        } else {
            let plan = self.plan_for(&doc.timeline, &doc.env, &doc.settings, &Selection::All);
            let max_lag = Duration::from_micros((doc.settings.adaptive_max_lag_ms * 1_000.0) as u64);
            self.ctx.run_plan(&plan, doc.settings.timing, max_lag).map_err(Self::interrupt)
        }
    }

    fn type_text(&mut self, text: &str, interval_us: Micros) -> Result<(), FlowError> {
        for (i, c) in text.chars().enumerate() {
            if i > 0 && interval_us > 0 {
                self.sleep(interval_us)?;
            } else {
                self.check()?;
            }
            if let Err(e) = self.ctx.injector.type_char(c) {
                return Err(FlowError::Host(e.to_string()));
            }
        }
        Ok(())
    }

    fn cursor(&mut self) -> (i32, i32) {
        self.ctx.platform.cursor().unwrap_or((0, 0))
    }

    fn window_active(&mut self, title: &str, process: &str) -> bool {
        self.ctx.platform.foreground().is_some_and(|w| chud_platform::window_matches(&w, title, process))
    }

    fn window_exists(&mut self, title: &str, process: &str) -> bool {
        self.ctx.platform.window_exists(title, process)
    }

    fn pixel(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        self.ctx.platform.pixel(x, y)
    }

    fn find(&mut self, target: &Target, region: Option<Region>) -> Result<Option<Found>, FlowError> {
        find_on_screen(self.ctx.platform, &self.job.images, target, region).map_err(FlowError::Host)
    }

    fn now(&mut self) -> Micros {
        self.ctx.epoch.elapsed().as_micros() as Micros
    }

    fn key_held(&mut self, key: Key) -> bool {
        self.ctx.platform.key_held(key)
    }

    fn random(&mut self) -> f64 {
        // xorshift64*: deterministic per run, good enough for jitter.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn notify(&mut self, message: &str) {
        let _ = self.ctx.notices.send(Notice { level: "info".into(), message: message.to_string() });
    }

    fn on_block(&mut self, id: &str) {
        *self.ctx.shared.block.lock() = id.to_string();
    }
}

/// Looks once for `target` on screen; positions in screen pixels. Errors
/// are messages for the user (capture unavailable, image missing…).
pub fn find_on_screen(
    platform: &dyn Platform,
    images: &HashMap<String, Image>,
    target: &Target,
    region: Option<Region>,
) -> Result<Option<Found>, String> {
    // Checked before capturing: a missing image is a mistake in the macro.
    let needle = match target {
        Target::Image { image, .. } => Some(images.get(image).ok_or_else(|| format!("the image {image} is missing from this macro"))?),
        _ => None,
    };
    let shot = platform.capture(region).map_err(|e| e.to_string())?;
    let found = match target {
        Target::Image { threshold, any_scale, .. } => {
            let opts = FindOptions { threshold: *threshold, any_scale: *any_scale, max_results: 1 };
            vision::find_image(&shot.image, needle.expect("checked above"), &opts).matches.first().copied()
        }
        Target::Color { color, tolerance } => {
            let c = parse_color(color).ok_or_else(|| format!("invalid colour {color}"))?;
            vision::find_color(&shot.image, c, *tolerance)
        }
        Target::Text { text, match_case } => {
            let words = platform.read_text(&shot).map_err(|e| e.to_string())?;
            // Words are already in screen pixels.
            return Ok(vision::find_text(&words, text, *match_case));
        }
    };
    Ok(found.map(|f| f.offset(shot.x, shot.y)))
}

/// Runs a job to completion on the calling thread (the engine spawns a
/// dedicated thread for it).
pub fn run_job(
    job: &PlayJob,
    platform: &dyn Platform,
    resolver: &dyn MacroResolver,
    shared: &PlayerShared,
    notices: &Sender<Notice>,
) -> PlayEnd {
    let injector = match platform.injector() {
        Ok(i) => i,
        Err(e) => return PlayEnd::Failed(e.to_string()),
    };
    let _boost = platform.boost_thread();
    let start_cursor = platform.cursor();
    let mut ctx = Ctx {
        injector,
        sleeper: PreciseSleeper::new(),
        platform,
        shared,
        pressed: Vec::new(),
        axes: Vec::new(),
        notices,
        warned_pad: false,
        warned_blocked: false,
        epoch: Instant::now(),
    };
    let s = &job.settings;
    let max_lag = Duration::from_micros((s.adaptive_max_lag_ms.max(1.0) * 1_000.0) as u64);
    let current = platform.monitors();
    let map = CoordMap::new(&job.env.monitors, &current);
    let plan = playback::compile(&job.timeline, s, &job.selection, Some(&map));
    // Virtual controllers are plugged in before the first event, so that
    // neither the timing nor the game's controller detection suffers.
    let pads = plan
        .steps
        .iter()
        .filter_map(|st| match st.action {
            Action::PadButton { pad, .. } | Action::PadAxis { pad, .. } => Some(pad as u16 + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if pads > 0 {
        match ctx.injector.prepare_gamepads(pads.min(u8::MAX as u16) as u8) {
            Ok(()) => {}
            Err(PlatformError::NoGamepadOutput) => ctx.warn_pad(),
            Err(e) => {
                let _ = notices.send(Notice { level: "error".into(), message: e.to_string() });
            }
        }
    }
    let flow = job.flow.as_ref().filter(|f| job.use_flow && !f.blocks.is_empty());
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ 0x9E37_79B9_7F4A_7C15;

    let mut result = PlayEnd::Completed;
    let mut iteration: u32 = 0;
    loop {
        iteration += 1;
        shared.iteration.store(iteration, Ordering::Relaxed);
        let outcome = match flow {
            Some(f) => {
                let mut host = Host { ctx: &mut ctx, job, resolver, depth: 0, rng: seed ^ iteration as u64 };
                match Interpreter::new(&mut host).run(f) {
                    Ok(()) => Ok(()),
                    Err(FlowError::Cancelled) => Err(PlayEnd::Stopped),
                    Err(e) => Err(PlayEnd::Failed(e.to_string())),
                }
            }
            None => ctx.run_plan(&plan, s.timing, max_lag).map_err(|_| PlayEnd::Stopped),
        };
        // Nothing may stay pressed between runs.
        ctx.release_all();
        if let Err(end) = outcome {
            result = end;
            break;
        }
        if s.repeat != 0 && iteration >= s.repeat {
            break;
        }
        if s.repeat_delay_ms > 0.0 {
            let due = Instant::now() + Duration::from_micros((s.repeat_delay_ms * 1_000.0) as u64);
            if ctx.wait(due).is_err() {
                result = PlayEnd::Stopped;
                break;
            }
        } else if ctx.stopped() {
            result = PlayEnd::Stopped;
            break;
        }
    }
    ctx.release_all();
    if s.restore_cursor
        && let Some((x, y)) = start_cursor {
            let _ = platform.set_cursor(x, y);
        }
    result
}
