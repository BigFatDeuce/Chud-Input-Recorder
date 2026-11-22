//! Flow: automation blocks (loops, conditions, variables, waits…) and their
//! interpreter. The interpreter talks to the outside world only through
//! [`FlowHost`], so it is fully testable with a simulated host.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::event::{Key, Micros, MouseButton, WHEEL_NOTCH};
use crate::expr::{self, Env, Value};
use crate::playback::{Action, KeyVia};
use crate::vision::{Found, Region};

fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
fn one_u32() -> u32 {
    1
}
fn click_hold() -> f64 {
    25.0
}
fn poll() -> f64 {
    100.0
}
fn similarity() -> f64 {
    0.9
}
fn find_poll() -> f64 {
    250.0
}
fn found_var() -> String {
    "found".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Flow {
    pub blocks: Vec<Node>,
}

impl Flow {
    /// Ids of the macro images this flow searches for (`Target::Image`).
    pub fn image_ids(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        if let Ok(v) = serde_json::to_value(self) {
            walk_image_refs(&v, &mut |id| {
                out.insert(id.to_string());
            });
        }
        out
    }

    /// Points image targets at new ids (`old → new`); others are untouched.
    pub fn rename_images(&mut self, map: &HashMap<String, String>) {
        if map.is_empty() {
            return;
        }
        let Ok(mut v) = serde_json::to_value(&*self) else { return };
        rename_image_refs(&mut v, map);
        if let Ok(f) = serde_json::from_value(v) {
            *self = f;
        }
    }
}

/// An image target, whatever block or condition holds it, serialises as
/// `{"kind": "image", "image": "<id>", …}`.
fn image_ref(m: &serde_json::Map<String, serde_json::Value>) -> Option<&str> {
    if m.get("kind").and_then(|k| k.as_str()) == Some("image") { m.get("image").and_then(|i| i.as_str()) } else { None }
}

fn walk_image_refs(v: &serde_json::Value, f: &mut dyn FnMut(&str)) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(id) = image_ref(m) {
                f(id);
            }
            m.values().for_each(|c| walk_image_refs(c, f));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|c| walk_image_refs(c, f)),
        _ => {}
    }
}

fn rename_image_refs(v: &mut serde_json::Value, map: &HashMap<String, String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(new) = image_ref(m).and_then(|id| map.get(id)).cloned() {
                m.insert("image".into(), serde_json::Value::String(new));
            }
            m.values_mut().for_each(|c| rename_image_refs(c, map));
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(|c| rename_image_refs(c, map)),
        _ => {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    #[serde(default)]
    pub id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub block: Block,
}

impl Node {
    pub fn new(block: Block) -> Self {
        Self { id: uuid::Uuid::new_v4().simple().to_string()[..8].to_string(), enabled: true, block }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    Instant,
    Linear,
    #[default]
    Smooth,
    /// Eased, slightly curved path with tiny jitter.
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    /// Plays this macro's own recording (or a time range of it).
    PlayRecording {
        #[serde(default)]
        start: Option<Micros>,
        #[serde(default)]
        end: Option<Micros>,
        #[serde(default = "one")]
        speed: f64,
    },
    /// Plays another macro of the library.
    PlayMacro {
        macro_id: String,
        #[serde(default)]
        name: String,
    },
    /// Waits for an expression in milliseconds (`"500"`, `"random(200, 400)"`).
    Wait { ms: String },
    /// Presses keys in order, holds, releases in reverse order (shortcuts).
    Keys {
        keys: Vec<Key>,
        #[serde(default)]
        hold_ms: f64,
    },
    KeyDown { key: Key },
    KeyUp { key: Key },
    TypeText {
        text: String,
        #[serde(default)]
        interval_ms: f64,
    },
    MoveMouse {
        x: String,
        y: String,
        #[serde(default)]
        relative: bool,
        #[serde(default)]
        duration_ms: f64,
        #[serde(default)]
        motion: Motion,
    },
    Click {
        button: MouseButton,
        #[serde(default = "one_u32")]
        count: u32,
        #[serde(default)]
        interval_ms: f64,
        #[serde(default = "click_hold")]
        hold_ms: f64,
    },
    MouseDown { button: MouseButton },
    MouseUp { button: MouseButton },
    /// Wheel notches (positive = up / right).
    Scroll {
        notches: i32,
        #[serde(default)]
        horizontal: bool,
    },
    /// `count = None` repeats until stopped. The 1-based iteration number is
    /// stored in `counter` when set.
    Repeat {
        #[serde(default)]
        count: Option<String>,
        #[serde(default)]
        counter: String,
        body: Vec<Node>,
    },
    While {
        cond: Condition,
        body: Vec<Node>,
        #[serde(default)]
        max_iterations: Option<u64>,
    },
    If {
        cond: Condition,
        then: Vec<Node>,
        #[serde(default)]
        otherwise: Vec<Node>,
    },
    WaitUntil {
        cond: Condition,
        #[serde(default)]
        timeout_ms: Option<f64>,
        #[serde(default = "poll")]
        poll_ms: f64,
    },
    /// Looks for an image, a colour or text on screen, then optionally
    /// points at it or clicks it.
    Find {
        target: Target,
        /// Search area (screen pixels); the whole desktop when absent.
        #[serde(default)]
        region: Option<Region>,
        /// Keeps looking this long, in ms (0: a single look).
        #[serde(default)]
        timeout_ms: f64,
        #[serde(default = "find_poll")]
        poll_ms: f64,
        #[serde(default)]
        action: FoundAction,
        /// The point acted on, relative to the centre of what was found.
        #[serde(default)]
        offset_x: i32,
        #[serde(default)]
        offset_y: i32,
        /// Pointer travel before acting (0: jump).
        #[serde(default)]
        duration_ms: f64,
        #[serde(default)]
        motion: Motion,
        /// Variables set: `<var>` (found or not), `<var>_x`, `<var>_y` (the
        /// point), `<var>_w`, `<var>_h`, `<var>_score` (similarity 0..1).
        #[serde(default = "found_var")]
        var: String,
        #[serde(default)]
        if_missing: IfMissing,
    },
    SetVar { name: String, value: String },
    Notify { message: String },
    Break,
    Continue,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    Expr { expr: String },
    /// Foreground window title and/or process contains the given text
    /// (case-insensitive; empty fields match anything).
    WindowActive {
        #[serde(default)]
        title: String,
        #[serde(default)]
        process: String,
    },
    WindowExists {
        #[serde(default)]
        title: String,
        #[serde(default)]
        process: String,
    },
    PixelColor {
        x: i32,
        y: i32,
        /// `#rrggbb`
        color: String,
        #[serde(default)]
        tolerance: u8,
    },
    KeyHeld { key: Key },
    /// An image, colour or text is visible (in `region`, or anywhere).
    OnScreen {
        target: Target,
        #[serde(default)]
        region: Option<Region>,
    },
    Chance { percent: f64 },
    Not { cond: Box<Condition> },
    All { conds: Vec<Condition> },
    Any { conds: Vec<Condition> },
}

/// What "find on screen" looks for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// An image stored in the macro (`MacroDoc::images`).
    Image {
        image: String,
        /// Minimum similarity, 0..=1.
        #[serde(default = "similarity")]
        threshold: f64,
        /// Also match it displayed smaller or larger (another scaling).
        #[serde(default)]
        any_scale: bool,
    },
    /// A pixel of this colour (`#rrggbb`): the first one from the top left.
    Color {
        color: String,
        #[serde(default)]
        tolerance: u8,
    },
    /// Text read on screen (character recognition).
    Text {
        text: String,
        #[serde(default)]
        match_case: bool,
    },
}

impl Target {
    /// Short description for messages.
    pub fn describe(&self) -> String {
        match self {
            Target::Image { image, .. } => format!("image {image}"),
            Target::Color { color, .. } => format!("colour {color}"),
            Target::Text { text, .. } => format!("text “{text}”"),
        }
    }
}

/// What to do with what was found.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FoundAction {
    /// Only set the variables.
    #[default]
    None,
    Move,
    Click,
    DoubleClick,
    RightClick,
    MiddleClick,
}

/// What happens when nothing was found in time.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IfMissing {
    /// Go on: the variable says it was not found.
    #[default]
    Continue,
    /// End the flow quietly.
    Stop,
    /// End the flow with an error.
    Fail,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FlowError {
    #[error("stopped")]
    Cancelled,
    #[error("expression error in block {block}: {source}")]
    Expr { block: String, source: expr::ExprError },
    #[error("{0}")]
    Host(String),
    #[error("macro nesting too deep")]
    TooDeep,
}

/// Everything the interpreter needs from the outside world.
pub trait FlowHost {
    /// Sends one output action immediately.
    fn perform(&mut self, action: Action) -> Result<(), FlowError>;
    /// Sleeps for `us` microseconds, returning `Cancelled` if stopped meanwhile.
    fn sleep(&mut self, us: Micros) -> Result<(), FlowError>;
    /// Cooperative cancellation point (also handles pause).
    fn check(&mut self) -> Result<(), FlowError>;
    fn play_recording(&mut self, start: Option<Micros>, end: Option<Micros>, speed: f64) -> Result<(), FlowError>;
    fn play_macro(&mut self, id: &str) -> Result<(), FlowError>;
    fn type_text(&mut self, text: &str, interval_us: Micros) -> Result<(), FlowError>;
    fn cursor(&mut self) -> (i32, i32);
    fn window_active(&mut self, title: &str, process: &str) -> bool;
    fn window_exists(&mut self, title: &str, process: &str) -> bool;
    fn pixel(&mut self, x: i32, y: i32) -> Option<(u8, u8, u8)>;
    /// Looks once for `target` (in `region`, or on the whole desktop).
    /// Positions are screen pixels.
    fn find(&mut self, target: &Target, region: Option<Region>) -> Result<Option<Found>, FlowError>;
    /// Monotonic clock in µs: time spent searching counts toward timeouts.
    fn now(&mut self) -> Micros;
    fn key_held(&mut self, key: Key) -> bool;
    fn random(&mut self) -> f64;
    fn notify(&mut self, message: &str);
    /// Called before each block runs (progress display).
    fn on_block(&mut self, _id: &str) {}
}

enum Signal {
    Normal,
    Break,
    Continue,
    Stop,
}

pub struct Interpreter<'h> {
    host: &'h mut dyn FlowHost,
    pub vars: HashMap<String, Value>,
}

struct VarEnv<'a, 'h> {
    vars: &'a HashMap<String, Value>,
    host: &'a mut (dyn FlowHost + 'h),
}

impl Env for VarEnv<'_, '_> {
    fn var(&self, name: &str) -> Option<Value> {
        self.vars.get(name).cloned()
    }
    fn random(&mut self) -> f64 {
        self.host.random()
    }
}

impl<'h> Interpreter<'h> {
    pub fn new(host: &'h mut dyn FlowHost) -> Self {
        Self { host, vars: HashMap::new() }
    }

    /// Runs a flow to completion (or until `Stop`).
    pub fn run(&mut self, flow: &Flow) -> Result<(), FlowError> {
        self.block_list(&flow.blocks).map(|_| ())
    }

    fn eval(&mut self, id: &str, src: &str) -> Result<Value, FlowError> {
        let mut env = VarEnv { vars: &self.vars, host: &mut *self.host };
        expr::eval_str(src, &mut env).map_err(|source| FlowError::Expr { block: id.to_string(), source })
    }

    fn num(&mut self, id: &str, src: &str) -> Result<f64, FlowError> {
        self.eval(id, src)?
            .as_num()
            .map_err(|source| FlowError::Expr { block: id.to_string(), source })
    }

    fn block_list(&mut self, nodes: &[Node]) -> Result<Signal, FlowError> {
        for n in nodes {
            if !n.enabled {
                continue;
            }
            self.host.check()?;
            self.host.on_block(&n.id);
            match self.node(n)? {
                Signal::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Signal::Normal)
    }

    fn ms(v: f64) -> Micros {
        (v.max(0.0) * 1_000.0).round() as Micros
    }

    fn node(&mut self, n: &Node) -> Result<Signal, FlowError> {
        let id = n.id.as_str();
        match &n.block {
            Block::PlayRecording { start, end, speed } => self.host.play_recording(*start, *end, *speed)?,
            Block::PlayMacro { macro_id, .. } => self.host.play_macro(macro_id)?,
            Block::Wait { ms } => {
                let v = self.num(id, ms)?;
                self.host.sleep(Self::ms(v))?;
            }
            Block::Keys { keys, hold_ms } => {
                for &key in keys {
                    self.host.perform(Action::Key { key, down: true, via: via(key) })?;
                }
                if *hold_ms > 0.0 {
                    self.host.sleep(Self::ms(*hold_ms))?;
                }
                for &key in keys.iter().rev() {
                    self.host.perform(Action::Key { key, down: false, via: via(key) })?;
                }
            }
            Block::KeyDown { key } => self.host.perform(Action::Key { key: *key, down: true, via: via(*key) })?,
            Block::KeyUp { key } => self.host.perform(Action::Key { key: *key, down: false, via: via(*key) })?,
            Block::TypeText { text, interval_ms } => {
                let text = interpolate(text, &self.vars);
                self.host.type_text(&text, Self::ms(*interval_ms))?
            }
            Block::MoveMouse { x, y, relative, duration_ms, motion } => {
                let (tx, ty) = (self.num(id, x)?.round() as i32, self.num(id, y)?.round() as i32);
                let (cx, cy) = self.host.cursor();
                let (tx, ty) = if *relative { (cx + tx, cy + ty) } else { (tx, ty) };
                self.move_mouse((cx, cy), (tx, ty), *duration_ms, *motion)?;
            }
            Block::Click { button, count, interval_ms, hold_ms } => {
                for i in 0..(*count).max(1) {
                    if i > 0 {
                        self.host.sleep(Self::ms(*interval_ms))?;
                    }
                    self.host.perform(Action::Button { button: *button, down: true, at: None })?;
                    if *hold_ms > 0.0 {
                        self.host.sleep(Self::ms(*hold_ms))?;
                    }
                    self.host.perform(Action::Button { button: *button, down: false, at: None })?;
                }
            }
            Block::MouseDown { button } => self.host.perform(Action::Button { button: *button, down: true, at: None })?,
            Block::MouseUp { button } => self.host.perform(Action::Button { button: *button, down: false, at: None })?,
            Block::Scroll { notches, horizontal } => self.host.perform(Action::Wheel {
                delta: notches * WHEEL_NOTCH,
                horizontal: *horizontal,
                at: None,
            })?,
            Block::Repeat { count, counter, body } => {
                let limit = match count {
                    Some(c) if !c.trim().is_empty() => Some(self.num(id, c)?.max(0.0) as u64),
                    _ => None,
                };
                let mut i: u64 = 0;
                while limit.is_none_or(|l| i < l) {
                    i += 1;
                    if !counter.is_empty() {
                        self.vars.insert(counter.clone(), Value::Num(i as f64));
                    }
                    match self.block_list(body)? {
                        Signal::Break => break,
                        Signal::Stop => return Ok(Signal::Stop),
                        Signal::Normal | Signal::Continue => {}
                    }
                    // Unbounded loops without any wait still yield to cancellation.
                    self.host.check()?;
                }
            }
            Block::While { cond, body, max_iterations } => {
                let mut i = 0u64;
                while self.cond(id, cond)? {
                    if max_iterations.is_some_and(|m| i >= m) {
                        break;
                    }
                    i += 1;
                    match self.block_list(body)? {
                        Signal::Break => break,
                        Signal::Stop => return Ok(Signal::Stop),
                        _ => {}
                    }
                    self.host.check()?;
                }
            }
            Block::If { cond, then, otherwise } => {
                let branch = if self.cond(id, cond)? { then } else { otherwise };
                return self.block_list(branch);
            }
            Block::WaitUntil { cond, timeout_ms, poll_ms } => {
                let step = Self::ms(poll_ms.max(1.0));
                let mut waited: Micros = 0;
                while !self.cond(id, cond)? {
                    if let Some(t) = timeout_ms
                        && waited >= Self::ms(*t) {
                            break;
                        }
                    self.host.sleep(step)?;
                    waited += step;
                }
            }
            Block::Find { target, region, timeout_ms, poll_ms, action, offset_x, offset_y, duration_ms, motion, var, if_missing } => {
                let start = self.host.now();
                let limit = Self::ms(*timeout_ms);
                let step = Self::ms(poll_ms.max(10.0));
                let found = loop {
                    if let Some(f) = self.host.find(target, *region)? {
                        break Some(f);
                    }
                    let elapsed = self.host.now() - start;
                    if elapsed >= limit {
                        break None;
                    }
                    self.host.sleep(step.min(limit - elapsed))?;
                };
                let name = if var.trim().is_empty() { "found" } else { var.trim() };
                self.vars.insert(name.to_string(), Value::Bool(found.is_some()));
                let field = |f: &str| format!("{name}_{f}");
                match found {
                    Some(f) => {
                        let (cx, cy) = f.center();
                        let (px, py) = (cx + offset_x, cy + offset_y);
                        let score = (f.score * 1000.0).round() / 1000.0;
                        for (k, v) in [("x", px as f64), ("y", py as f64), ("w", f.w as f64), ("h", f.h as f64), ("score", score)] {
                            self.vars.insert(field(k), Value::Num(v));
                        }
                        if *action != FoundAction::None {
                            let from = self.host.cursor();
                            self.move_mouse(from, (px, py), *duration_ms, *motion)?;
                            let (button, count) = match action {
                                FoundAction::Click => (MouseButton::Left, 1),
                                FoundAction::DoubleClick => (MouseButton::Left, 2),
                                FoundAction::RightClick => (MouseButton::Right, 1),
                                FoundAction::MiddleClick => (MouseButton::Middle, 1),
                                FoundAction::Move | FoundAction::None => (MouseButton::Left, 0),
                            };
                            for i in 0..count {
                                if i > 0 {
                                    self.host.sleep(Self::ms(80.0))?;
                                }
                                self.host.perform(Action::Button { button, down: true, at: None })?;
                                self.host.sleep(Self::ms(click_hold()))?;
                                self.host.perform(Action::Button { button, down: false, at: None })?;
                            }
                        }
                    }
                    None => {
                        // No stale position left from an earlier search.
                        for k in ["x", "y", "w", "h", "score"] {
                            self.vars.remove(&field(k));
                        }
                        match if_missing {
                            IfMissing::Continue => {}
                            IfMissing::Stop => return Ok(Signal::Stop),
                            IfMissing::Fail => return Err(FlowError::Host(format!("not found on screen: {}", target.describe()))),
                        }
                    }
                }
            }
            Block::SetVar { name, value } => {
                let v = self.eval(id, value)?;
                self.vars.insert(name.clone(), v);
            }
            Block::Notify { message } => {
                let msg = interpolate(message, &self.vars);
                self.host.notify(&msg);
            }
            Block::Break => return Ok(Signal::Break),
            Block::Continue => return Ok(Signal::Continue),
            Block::Stop => return Ok(Signal::Stop),
        }
        Ok(Signal::Normal)
    }

    fn cond(&mut self, id: &str, c: &Condition) -> Result<bool, FlowError> {
        Ok(match c {
            Condition::Expr { expr } => self.eval(id, expr)?.truthy(),
            Condition::WindowActive { title, process } => self.host.window_active(title, process),
            Condition::WindowExists { title, process } => self.host.window_exists(title, process),
            Condition::PixelColor { x, y, color, tolerance } => match (self.host.pixel(*x, *y), parse_color(color)) {
                (Some((r, g, b)), Some((er, eg, eb))) => {
                    let t = *tolerance as i32;
                    (r as i32 - er as i32).abs() <= t
                        && (g as i32 - eg as i32).abs() <= t
                        && (b as i32 - eb as i32).abs() <= t
                }
                _ => false,
            },
            Condition::KeyHeld { key } => self.host.key_held(*key),
            Condition::OnScreen { target, region } => self.host.find(target, *region)?.is_some(),
            Condition::Chance { percent } => self.host.random() * 100.0 < *percent,
            Condition::Not { cond } => !self.cond(id, cond)?,
            Condition::All { conds } => {
                for c in conds {
                    if !self.cond(id, c)? {
                        return Ok(false);
                    }
                }
                true
            }
            Condition::Any { conds } => {
                for c in conds {
                    if self.cond(id, c)? {
                        return Ok(true);
                    }
                }
                false
            }
        })
    }

    fn move_mouse(&mut self, from: (i32, i32), to: (i32, i32), duration_ms: f64, motion: Motion) -> Result<(), FlowError> {
        if motion == Motion::Instant || duration_ms <= 0.0 {
            return self.host.perform(Action::MoveTo { x: to.0, y: to.1 });
        }
        // ~125 Hz updates.
        let steps = ((duration_ms / 8.0).ceil() as usize).max(1);
        let dt = Self::ms(duration_ms) / steps as Micros;
        let (fx, fy) = (from.0 as f64, from.1 as f64);
        let (tx, ty) = (to.0 as f64, to.1 as f64);
        // Control point for a gentle arc (human motion).
        let (mx, my) = ((fx + tx) / 2.0, (fy + ty) / 2.0);
        let (nx, ny) = (-(ty - fy), tx - fx);
        let bend = if motion == Motion::Human { (self.host.random() - 0.5) * 0.25 } else { 0.0 };
        let (cx, cy) = (mx + nx * bend, my + ny * bend);
        for i in 1..=steps {
            let u = i as f64 / steps as f64;
            let e = match motion {
                Motion::Linear => u,
                _ => u * u * (3.0 - 2.0 * u), // smoothstep
            };
            let (mut x, mut y) = if motion == Motion::Human {
                let a = (1.0 - e) * (1.0 - e);
                let b = 2.0 * (1.0 - e) * e;
                let c = e * e;
                (a * fx + b * cx + c * tx, a * fy + b * cy + c * ty)
            } else {
                (fx + (tx - fx) * e, fy + (ty - fy) * e)
            };
            if motion == Motion::Human && i < steps {
                x += (self.host.random() - 0.5) * 1.5;
                y += (self.host.random() - 0.5) * 1.5;
            }
            self.host.sleep(dt)?;
            self.host.perform(Action::MoveTo { x: x.round() as i32, y: y.round() as i32 })?;
        }
        Ok(())
    }
}

fn via(key: Key) -> KeyVia {
    if key.scan != 0 { KeyVia::Scan } else { KeyVia::Vk }
}

/// Replaces `{name}` with variable values.
pub fn interpolate(text: &str, vars: &HashMap<String, Value>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => {
                let name = &rest[open + 1..open + close];
                match vars.get(name.trim()) {
                    Some(v) => out.push_str(&v.to_string()),
                    None => out.push_str(&rest[open..=open + close]),
                }
                rest = &rest[open + close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Sim {
        now: Micros,
        log: Vec<(Micros, String)>,
        cursor: (i32, i32),
        active: String,
        stop_after: Option<usize>,
        /// Images on screen: id, where, and after how many looks it shows.
        screen: Vec<(String, Found, usize)>,
        looks: usize,
    }

    impl FlowHost for Sim {
        fn perform(&mut self, a: Action) -> Result<(), FlowError> {
            if let Action::MoveTo { x, y } = a {
                self.cursor = (x, y);
            }
            self.log.push((self.now, format!("{a:?}")));
            Ok(())
        }
        fn sleep(&mut self, us: Micros) -> Result<(), FlowError> {
            self.now += us;
            Ok(())
        }
        fn check(&mut self) -> Result<(), FlowError> {
            match self.stop_after {
                Some(n) if self.log.len() >= n => Err(FlowError::Cancelled),
                _ => Ok(()),
            }
        }
        fn play_recording(&mut self, _: Option<Micros>, _: Option<Micros>, _: f64) -> Result<(), FlowError> {
            self.log.push((self.now, "rec".into()));
            self.now += 1_000_000;
            Ok(())
        }
        fn play_macro(&mut self, id: &str) -> Result<(), FlowError> {
            self.log.push((self.now, format!("macro {id}")));
            Ok(())
        }
        fn type_text(&mut self, t: &str, _: Micros) -> Result<(), FlowError> {
            self.log.push((self.now, format!("type {t}")));
            Ok(())
        }
        fn cursor(&mut self) -> (i32, i32) {
            self.cursor
        }
        fn window_active(&mut self, title: &str, _: &str) -> bool {
            self.active.contains(title)
        }
        fn window_exists(&mut self, _: &str, _: &str) -> bool {
            false
        }
        fn pixel(&mut self, _: i32, _: i32) -> Option<(u8, u8, u8)> {
            Some((250, 200, 60))
        }
        fn find(&mut self, target: &Target, region: Option<Region>) -> Result<Option<Found>, FlowError> {
            self.looks += 1;
            self.now += 40_000; // a search takes time
            let Target::Image { image, .. } = target else { return Ok(None) };
            let looks = self.looks;
            Ok(self
                .screen
                .iter()
                .find(|(id, f, from)| {
                    id == image && looks > *from && region.is_none_or(|r| r.intersect(&Region { x: f.x, y: f.y, w: f.w, h: f.h }).is_some())
                })
                .map(|(_, f, _)| *f))
        }
        fn now(&mut self) -> Micros {
            self.now
        }
        fn key_held(&mut self, _: Key) -> bool {
            false
        }
        fn random(&mut self) -> f64 {
            0.5
        }
        fn notify(&mut self, m: &str) {
            self.log.push((self.now, format!("notify {m}")));
        }
    }

    fn n(b: Block) -> Node {
        Node::new(b)
    }

    fn find_block(image: &str, action: FoundAction, timeout_ms: f64, if_missing: IfMissing) -> Block {
        Block::Find {
            target: Target::Image { image: image.into(), threshold: 0.9, any_scale: false },
            region: None,
            timeout_ms,
            poll_ms: 100.0,
            action,
            offset_x: 0,
            offset_y: 0,
            duration_ms: 0.0,
            motion: Motion::Instant,
            var: "found".into(),
            if_missing,
        }
    }

    fn button() -> Found {
        Found { x: 100, y: 50, w: 21, h: 11, score: 0.97 }
    }

    #[test]
    fn find_clicks_the_centre_and_sets_variables() {
        let mut sim = Sim { screen: vec![("ok".into(), button(), 0)], ..Default::default() };
        let flow = Flow { blocks: vec![n(find_block("ok", FoundAction::DoubleClick, 0.0, IfMissing::Continue))] };
        let mut it = Interpreter::new(&mut sim);
        it.run(&flow).unwrap();
        assert_eq!(it.vars.get("found"), Some(&Value::Bool(true)));
        assert_eq!(it.vars.get("found_x"), Some(&Value::Num(110.0)));
        assert_eq!(it.vars.get("found_y"), Some(&Value::Num(55.0)));
        assert_eq!(it.vars.get("found_score"), Some(&Value::Num(0.97)));
        assert_eq!(sim.cursor, (110, 55));
        let downs = sim.log.iter().filter(|(_, l)| l.contains("down: true")).count();
        assert_eq!(downs, 2, "{:?}", sim.log);
    }

    #[test]
    fn find_keeps_looking_until_it_appears() {
        // Shows up on the 4th look; polling every 100 ms within 2 s.
        let mut sim = Sim { screen: vec![("ok".into(), button(), 3)], ..Default::default() };
        let flow = Flow { blocks: vec![n(find_block("ok", FoundAction::Move, 2_000.0, IfMissing::Fail))] };
        Interpreter::new(&mut sim).run(&flow).unwrap();
        assert_eq!(sim.looks, 4);
        assert_eq!(sim.cursor, (110, 55));
        // 4 searches of 40 ms + 3 pauses of 100 ms.
        assert_eq!(sim.now, 4 * 40_000 + 3 * 100_000);
    }

    #[test]
    fn missing_target_continues_stops_or_fails() {
        let after = || n(Block::Notify { message: "after".into() });
        // Continue: variable false, no stale position, next block runs.
        let mut sim = Sim::default();
        let mut it = Interpreter::new(&mut sim);
        it.vars.insert("found_x".into(), Value::Num(1.0));
        it.run(&Flow { blocks: vec![n(find_block("ok", FoundAction::Click, 300.0, IfMissing::Continue)), after()] }).unwrap();
        assert_eq!(it.vars.get("found"), Some(&Value::Bool(false)));
        assert_eq!(it.vars.get("found_x"), None);
        assert!(sim.log.iter().any(|(_, l)| l == "notify after"));
        assert!(!sim.log.iter().any(|(_, l)| l.contains("Button")), "nothing clicked");
        // The timeout bounds the search time.
        assert!(sim.now >= 300_000 && sim.now < 450_000, "{}", sim.now);
        // Stop: the flow ends quietly.
        let mut sim = Sim::default();
        Interpreter::new(&mut sim).run(&Flow { blocks: vec![n(find_block("ok", FoundAction::None, 0.0, IfMissing::Stop)), after()] }).unwrap();
        assert!(sim.log.is_empty());
        // Fail: an error names what was missing.
        let mut sim = Sim::default();
        let err = Interpreter::new(&mut sim).run(&Flow { blocks: vec![n(find_block("ok", FoundAction::None, 0.0, IfMissing::Fail))] });
        assert_eq!(err, Err(FlowError::Host("not found on screen: image ok".into())));
    }

    #[test]
    fn on_screen_condition_branches_and_respects_the_region() {
        let flow = |region| Flow {
            blocks: vec![n(Block::If {
                cond: Condition::OnScreen { target: Target::Image { image: "ok".into(), threshold: 0.9, any_scale: false }, region },
                then: vec![n(Block::Notify { message: "seen".into() })],
                otherwise: vec![n(Block::Notify { message: "not seen".into() })],
            })],
        };
        let screen = vec![("ok".to_string(), button(), 0)];
        let mut sim = Sim { screen: screen.clone(), ..Default::default() };
        Interpreter::new(&mut sim).run(&flow(None)).unwrap();
        assert_eq!(sim.log.last().unwrap().1, "notify seen");
        let mut sim = Sim { screen, ..Default::default() };
        Interpreter::new(&mut sim).run(&flow(Some(Region { x: 500, y: 500, w: 100, h: 100 }))).unwrap();
        assert_eq!(sim.log.last().unwrap().1, "notify not seen");
    }

    #[test]
    fn find_block_json_uses_defaults() {
        let json = r#"{"type":"find","target":{"kind":"image","image":"a1b2"},"action":"click"}"#;
        let b: Block = serde_json::from_str(json).unwrap();
        let Block::Find { target, timeout_ms, poll_ms, var, if_missing, region, .. } = b else { panic!() };
        assert_eq!(target, Target::Image { image: "a1b2".into(), threshold: 0.9, any_scale: false });
        assert_eq!((timeout_ms, poll_ms, var.as_str(), if_missing, region), (0.0, 250.0, "found", IfMissing::Continue, None));
        let c: Condition = serde_json::from_str(r##"{"type":"on_screen","target":{"kind":"color","color":"#ff0000","tolerance":8},"region":{"x":0,"y":0,"w":10,"h":10}}"##).unwrap();
        assert!(matches!(c, Condition::OnScreen { target: Target::Color { tolerance: 8, .. }, region: Some(_) }));
    }

    #[test]
    fn spec_example_runs_with_exact_timing() {
        // Wait 2 s, Ctrl+C, wait, move, double click, all repeated 10 times.
        let ctrl = Key::new(0x1D, 0xA2);
        let c = Key::new(0x2E, 0x43);
        let flow = Flow {
            blocks: vec![n(Block::Repeat {
                count: Some("10".into()),
                counter: "i".into(),
                body: vec![
                    n(Block::Wait { ms: "2000".into() }),
                    n(Block::Keys { keys: vec![ctrl, c], hold_ms: 0.0 }),
                    n(Block::Wait { ms: "500".into() }),
                    n(Block::MoveMouse { x: "100".into(), y: "200".into(), relative: false, duration_ms: 0.0, motion: Motion::Instant }),
                    n(Block::Click { button: MouseButton::Left, count: 2, interval_ms: 80.0, hold_ms: 20.0 }),
                ],
            })],
        };
        let mut sim = Sim::default();
        let mut it = Interpreter::new(&mut sim);
        it.run(&flow).unwrap();
        assert_eq!(it.vars.get("i"), Some(&Value::Num(10.0)));
        // Per iteration: 4 key actions + 1 move + 4 button actions.
        assert_eq!(sim.log.len(), 90);
        // One iteration lasts 2000 + 500 + 20 + 80 + 20 ms.
        assert_eq!(sim.now, 10 * 2_620_000);
        assert_eq!(sim.cursor, (100, 200));
    }

    #[test]
    fn conditions_variables_and_break() {
        let flow = Flow {
            blocks: vec![
                n(Block::SetVar { name: "n".into(), value: "0".into() }),
                n(Block::Repeat {
                    count: None,
                    counter: String::new(),
                    body: vec![
                        n(Block::SetVar { name: "n".into(), value: "n + 1".into() }),
                        n(Block::If {
                            cond: Condition::Expr { expr: "n >= 5".into() },
                            then: vec![n(Block::Break)],
                            otherwise: vec![],
                        }),
                    ],
                }),
                n(Block::If {
                    cond: Condition::All {
                        conds: vec![
                            Condition::WindowActive { title: "Editor".into(), process: String::new() },
                            Condition::PixelColor { x: 0, y: 0, color: "#f9c83a".into(), tolerance: 10 },
                        ],
                    },
                    then: vec![n(Block::Notify { message: "n={n}".into() })],
                    otherwise: vec![],
                }),
            ],
        };
        let mut sim = Sim { active: "My Editor".into(), ..Default::default() };
        let mut it = Interpreter::new(&mut sim);
        it.run(&flow).unwrap();
        assert_eq!(sim.log.last().unwrap().1, "notify n=5");
    }

    #[test]
    fn infinite_loop_is_cancellable_and_wait_until_times_out() {
        let flow = Flow {
            blocks: vec![
                n(Block::WaitUntil { cond: Condition::WindowExists { title: "x".into(), process: String::new() }, timeout_ms: Some(1_000.0), poll_ms: 100.0 }),
                n(Block::Repeat { count: None, counter: String::new(), body: vec![n(Block::Scroll { notches: 1, horizontal: false })] }),
            ],
        };
        let mut sim = Sim { stop_after: Some(50), ..Default::default() };
        let res = Interpreter::new(&mut sim).run(&flow);
        assert_eq!(res, Err(FlowError::Cancelled));
        assert_eq!(sim.log.len(), 50);
        assert_eq!(sim.log[0].0, 1_000_000);
    }

    #[test]
    fn smooth_motion_reaches_target() {
        let flow = Flow {
            blocks: vec![n(Block::MoveMouse { x: "300".into(), y: "-40".into(), relative: false, duration_ms: 100.0, motion: Motion::Human })],
        };
        let mut sim = Sim::default();
        Interpreter::new(&mut sim).run(&flow).unwrap();
        assert_eq!(sim.cursor, (300, -40));
        assert!(sim.log.len() >= 12);
        assert!(sim.now >= 96_000 && sim.now <= 100_000);
    }

    #[test]
    fn json_roundtrip_and_disabled_blocks() {
        let mut node = n(Block::Stop);
        node.enabled = false;
        let flow = Flow { blocks: vec![node, n(Block::Notify { message: "ok".into() })] };
        let json = serde_json::to_string(&flow).unwrap();
        let back: Flow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, flow);
        let mut sim = Sim::default();
        Interpreter::new(&mut sim).run(&back).unwrap();
        assert_eq!(sim.log.len(), 1);
    }

    #[test]
    fn interpolation() {
        let mut vars = HashMap::new();
        vars.insert("i".to_string(), Value::Num(4.0));
        assert_eq!(interpolate("run {i} of {total}", &vars), "run 4 of {total}");
    }
}
