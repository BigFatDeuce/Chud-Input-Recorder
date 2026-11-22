//! Linux capture thread.
//!
//! Input is read straight from the kernel (evdev, `/dev/input/event*`), which
//! works the same under X11, Wayland and on the console. Reading those nodes
//! needs membership of the `input` group: Diagnostics explains it, Chud never
//! changes permissions by itself.
//!
//! * Keyboards are always open (global hotkeys). Pointer devices are opened
//!   only while something needs them (recording, live view, abort-on-input).
//! * Event times are the kernel timestamps (`CLOCK_MONOTONIC`), not the time
//!   the thread happened to wake up.
//! * Absolute pointer positions come from X11. Under Wayland there is no
//!   standard way to read them: only relative motion is recorded.
//! * Hotkeys cannot be swallowed on Linux (the focused application sees them
//!   too), but they are never recorded.

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chud_core::event::{EventKind, Key, MouseButton};
use chud_core::keys;
use crossbeam_channel::Sender;
use evdev::raw_stream::RawDevice;
use evdev::{AbsoluteAxisCode, KeyCode, RelativeAxisCode};
use parking_lot::RwLock;

use super::gamepad::{self, Layout, PadTrack};
use super::x11::{Keymap, X};
use crate::hotkey::{Hotkey, MatchResult, Matcher};
use crate::{CaptureOptions, Captured, PlatformError, Result};

/// Name prefix of Chud's own virtual devices (never recorded).
pub const OWN_DEVICE_PREFIX: &str = "Chud Input Recorder";

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const REL_HWHEEL_HI_RES: u16 = 12;
const BTN_LEFT: u16 = 0x110;

/// Fallback rescan period (hot-plug is normally seen at once via inotify).
const RESCAN_EVERY: Duration = Duration::from_secs(5);
/// Delay after a /dev/input change, letting udev apply permissions.
const RESCAN_SETTLE: Duration = Duration::from_millis(20);
const POINTER_EVERY: Duration = Duration::from_millis(2);
const FOREGROUND_EVERY: Duration = Duration::from_millis(250);

struct Global {
    sink: RwLock<Option<Sender<Captured>>>,
    opts: RwLock<CaptureOptions>,
    hotkeys: RwLock<Vec<Hotkey>>,
    started: AtomicBool,
    /// eventfd used to wake the capture thread (-1 before start).
    wake: AtomicI32,
}

static G: LazyLock<Global> = LazyLock::new(|| Global {
    sink: RwLock::new(None),
    opts: RwLock::new(CaptureOptions::default()),
    hotkeys: RwLock::new(Hotkey::defaults()),
    started: AtomicBool::new(false),
    wake: AtomicI32::new(-1),
});

/// Keys and buttons currently held on real devices, by evdev code.
pub static HELD: [AtomicBool; 0x300] = [const { AtomicBool::new(false) }; 0x300];

fn emit(c: Captured) {
    if let Some(s) = G.sink.read().as_ref() {
        let _ = s.send(c);
    }
}

fn wake() {
    let fd = G.wake.load(Ordering::Acquire);
    if fd >= 0 {
        let one: u64 = 1;
        unsafe {
            libc::write(fd, &one as *const u64 as *const _, 8);
        }
    }
}

pub fn set_options(o: CaptureOptions) {
    *G.opts.write() = o;
    wake();
}

pub fn set_hotkeys(h: Vec<Hotkey>) {
    *G.hotkeys.write() = h;
}

pub fn start(sink: Sender<Captured>, x: Option<Arc<X>>, wayland: bool) -> Result<()> {
    *G.sink.write() = Some(sink);
    if G.started.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if fd < 0 {
        G.started.store(false, Ordering::Release);
        return Err(PlatformError::System(format!("eventfd: {}", std::io::Error::last_os_error())));
    }
    G.wake.store(fd, Ordering::Release);
    std::thread::Builder::new()
        .name("chud-capture".into())
        .spawn(move || {
            let mut cap = Capture::new(x, wayland);
            cap.run(fd);
        })
        .map_err(|e| PlatformError::System(e.to_string()))?;
    Ok(())
}

/// Monotonic clock, the one evdev timestamps use after `EVIOCSCLOCKID`.
fn mono_now() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// `EVIOCSCLOCKID`: `_IOW('E', 0xa0, int)`.
const EVIOCSCLOCKID: u64 = 0x4004_45a0;

fn set_monotonic(fd: i32) -> bool {
    let clk: libc::c_int = libc::CLOCK_MONOTONIC;
    unsafe { libc::ioctl(fd, EVIOCSCLOCKID as _, &clk as *const libc::c_int) == 0 }
}

/// Converts a kernel timestamp into an [`Instant`].
fn to_instant(ts: SystemTime, mono: bool, now_i: Instant, now_m: Duration) -> Instant {
    if !mono {
        return now_i;
    }
    let Ok(ev) = ts.duration_since(UNIX_EPOCH) else { return now_i };
    match now_m.checked_sub(ev) {
        Some(age) if age < Duration::from_secs(10) => now_i.checked_sub(age).unwrap_or(now_i),
        _ => now_i,
    }
}

/// inotify watch on `/dev/input` (or on `/dev` until `/dev/input` exists).
struct DevWatch {
    fd: i32,
    input_wd: i32,
}

impl DevWatch {
    fn new() -> Self {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        let mut w = Self { fd, input_wd: -1 };
        if fd >= 0 && !w.watch_input() {
            unsafe {
                libc::inotify_add_watch(fd, c"/dev".as_ptr(), libc::IN_CREATE);
            }
        }
        w
    }

    fn watch_input(&mut self) -> bool {
        if self.input_wd < 0 {
            self.input_wd = unsafe {
                libc::inotify_add_watch(self.fd, c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB)
            };
        }
        self.input_wd >= 0
    }

    /// Consumes pending notifications; true when something changed.
    fn drain(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            any = true;
        }
        if any {
            self.watch_input();
        }
        any
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Class {
    keyboard: bool,
    pointer: bool,
    gamepad: bool,
    own: bool,
    /// Created by software (uinput): treated as injected input.
    virtual_bus: bool,
}

fn classify(raw: &RawDevice) -> Class {
    let name = raw.name().unwrap_or("");
    let keys = raw.supported_keys();
    let has = |c: u16| keys.is_some_and(|k| k.contains(KeyCode(c)));
    let rel = raw.supported_relative_axes();
    let has_rel = |c: u16| rel.is_some_and(|r| r.contains(RelativeAxisCode(c)));
    let joystick = (0x120..0x140).any(has);
    let any_key = keys.is_some_and(|k| k.iter().any(|c| c.0 > 0 && c.0 < 0x100));
    let abs = raw.supported_absolute_axes();
    let has_abs = |c: u16| abs.is_some_and(|a| a.contains(AbsoluteAxisCode(c)));
    let gamepad = gamepad::is_gamepad(has, has_abs);
    Class {
        keyboard: any_key && !(joystick && !has(30)),
        pointer: !gamepad && ((has_rel(REL_X) && has_rel(REL_Y)) || has(BTN_LEFT) || has_rel(REL_WHEEL)),
        gamepad,
        own: name.starts_with(OWN_DEVICE_PREFIX),
        virtual_bus: raw.input_id().bus_type().0 == 0x06,
    }
}

fn button_for(code: u16) -> Option<MouseButton> {
    Some(match code {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        0x113 | 0x116 => MouseButton::X1, // BTN_SIDE / BTN_BACK
        0x114 | 0x115 => MouseButton::X2, // BTN_EXTRA / BTN_FORWARD
        _ => return None,
    })
}

#[derive(Default)]
struct Frame {
    dx: i32,
    dy: i32,
    wheel: i32,
    hwheel: i32,
    wheel_hi: Option<i32>,
    hwheel_hi: Option<i32>,
    abs_motion: bool,
    keys: Vec<(u16, i32)>,
    /// Absolute axes of gamepads.
    abs: Vec<(u16, i32)>,
}

impl Frame {
    fn clear(&mut self) {
        self.dx = 0;
        self.dy = 0;
        self.wheel = 0;
        self.hwheel = 0;
        self.wheel_hi = None;
        self.hwheel_hi = None;
        self.abs_motion = false;
        self.keys.clear();
        self.abs.clear();
    }
}

struct Dev {
    path: PathBuf,
    raw: RawDevice,
    class: Class,
    mono: bool,
    frame: Frame,
    dropping: bool,
    pad: Option<PadDev>,
}

/// A controller: its number while open, how its codes map, what it holds.
struct PadDev {
    index: u8,
    layout: Layout,
    track: PadTrack,
}

struct Capture {
    x: Option<Arc<X>>,
    /// X pointer positions are meaningful (X11 session, not XWayland).
    x_pointer: bool,
    wayland: bool,
    devs: Vec<Dev>,
    /// Classification cache by `(path, inode)`, also remembering failures.
    known: HashMap<(PathBuf, u64), Option<Class>>,
    opts: CaptureOptions,
    matcher: Matcher,
    keymap: Option<Keymap>,
    own_pid: u32,
    last_activity: Option<Instant>,
    // Pointer tracking.
    pos: (i32, i32),
    pointer_due: Option<Instant>,
    last_query: Option<Instant>,
    // Foreground tracking.
    fg_window: Option<u32>,
    fg_excluded: bool,
    next_fg: Instant,
    next_rescan: Instant,
    warned_access: bool,
    warned_wayland: bool,
    was_forwarding: bool,
}

impl Capture {
    fn new(x: Option<Arc<X>>, wayland: bool) -> Self {
        let now = Instant::now();
        Self {
            x_pointer: x.is_some() && !wayland,
            x,
            wayland,
            devs: Vec::new(),
            known: HashMap::new(),
            opts: G.opts.read().clone(),
            matcher: Matcher::default(),
            keymap: None,
            own_pid: std::process::id(),
            last_activity: None,
            pos: (0, 0),
            pointer_due: None,
            last_query: None,
            fg_window: None,
            fg_excluded: false,
            next_fg: now,
            next_rescan: now,
            warned_access: false,
            warned_wayland: false,
            was_forwarding: false,
        }
    }

    fn need_pointer(&self) -> bool {
        self.opts.needs_mouse_hook() || self.opts.needs_raw_mouse()
    }

    fn wanted(&self, c: &Class) -> bool {
        if c.own && !self.opts.include_own {
            return false;
        }
        c.keyboard || (c.pointer && self.need_pointer()) || (c.gamepad && self.opts.needs_gamepad())
    }

    /// Lowest controller number not used by an open controller.
    fn free_pad_index(&self) -> u8 {
        (0u8..).find(|i| !self.devs.iter().any(|d| d.pad.as_ref().is_some_and(|p| p.index == *i))).unwrap_or(0)
    }

    /// Opens new devices, closes the ones no longer needed or gone.
    fn rescan(&mut self) {
        let mut present = Vec::new();
        if let Ok(dir) = std::fs::read_dir("/dev/input") {
            for e in dir.flatten() {
                let path = e.path();
                if !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event")) {
                    continue;
                }
                let ino = e.metadata().map(|m| m.ino()).unwrap_or(0);
                present.push((path, ino));
            }
        }
        self.known.retain(|k, _| present.contains(k));
        let open: Vec<PathBuf> = self.devs.iter().map(|d| d.path.clone()).collect();
        let mut readable = 0;
        let mut denied = 0;
        for key in &present {
            if let Some(Some(c)) = self.known.get(key) {
                readable += 1;
                if open.contains(&key.0) || !self.wanted(c) {
                    continue;
                }
            } else if let Some(None) = self.known.get(key) {
                denied += 1;
                continue;
            }
            match RawDevice::open(&key.0) {
                Ok(raw) => {
                    let class = classify(&raw);
                    if !self.known.contains_key(key) {
                        readable += 1;
                    }
                    self.known.insert(key.clone(), Some(class));
                    if self.wanted(&class) && !open.contains(&key.0) {
                        let mono = set_monotonic(raw.as_raw_fd());
                        let pad = class.gamepad.then(|| PadDev {
                            index: self.free_pad_index(),
                            layout: Layout::of(&raw, &gamepad::driver_of(&key.0), class.own),
                            track: PadTrack::default(),
                        });
                        self.devs.push(Dev { path: key.0.clone(), raw, class, mono, frame: Frame::default(), dropping: false, pad });
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        denied += 1;
                        self.known.insert(key.clone(), None);
                    }
                }
            }
        }
        let paths: Vec<&PathBuf> = present.iter().map(|(p, _)| p).collect();
        let need_pointer = self.need_pointer();
        let need_pads = self.opts.needs_gamepad();
        let include_own = self.opts.include_own;
        self.devs.retain(|d| {
            paths.contains(&&d.path)
                && !(d.class.own && !include_own)
                && (d.class.keyboard || (d.class.pointer && need_pointer) || (d.class.gamepad && need_pads))
        });
        if readable == 0 && denied > 0 && !self.warned_access {
            self.warned_access = true;
            emit(Captured::Warning { t: Instant::now(), message: "linux_input_permission".into() });
        }
        self.next_rescan = Instant::now() + RESCAN_EVERY;
    }

    fn reconfigure(&mut self) {
        self.opts = G.opts.read().clone();
        self.fg_window = None;
        if self.opts.forward && !self.was_forwarding {
            // A new recording / live session: refresh the layout.
            self.keymap = self.x.as_ref().and_then(|x| x.keymap());
            if let Some(p) = self.query_pointer() {
                self.pos = p;
            }
            if self.wayland && !self.warned_wayland {
                self.warned_wayland = true;
                emit(Captured::Warning { t: Instant::now(), message: "wayland_limited_capture".into() });
            }
        }
        self.was_forwarding = self.opts.forward;
        self.rescan();
    }

    fn run(&mut self, wake_fd: i32) {
        self.keymap = self.x.as_ref().and_then(|x| x.keymap());
        let mut watch = DevWatch::new();
        self.rescan();
        let mut fds: Vec<libc::pollfd> = Vec::new();
        const BASE: usize = 2;
        loop {
            fds.clear();
            fds.push(libc::pollfd { fd: wake_fd, events: libc::POLLIN, revents: 0 });
            fds.push(libc::pollfd { fd: watch.fd, events: libc::POLLIN, revents: 0 });
            for d in &self.devs {
                fds.push(libc::pollfd { fd: d.raw.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            }
            let now = Instant::now();
            let mut deadline = self.next_rescan;
            if let Some(p) = self.pointer_due {
                deadline = deadline.min(p);
            }
            if self.opts.forward && self.x.is_some() {
                deadline = deadline.min(self.next_fg);
            }
            let timeout = deadline.saturating_duration_since(now).as_millis().min(i32::MAX as u128) as i32;
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    std::thread::sleep(Duration::from_millis(50));
                }
                continue;
            }
            if fds[0].revents & libc::POLLIN != 0 {
                let mut buf = 0u64;
                unsafe {
                    libc::read(wake_fd, &mut buf as *mut u64 as *mut _, 8);
                }
                self.reconfigure();
                continue; // device list may have changed
            }
            if fds[1].revents & libc::POLLIN != 0 && watch.drain() {
                // A node appeared, vanished or changed permissions.
                self.known.retain(|_, c| c.is_some());
                self.next_rescan = self.next_rescan.min(Instant::now() + RESCAN_SETTLE);
            }
            let mut gone = Vec::new();
            for (i, pfd) in fds.iter().enumerate().skip(BASE) {
                if pfd.revents == 0 {
                    continue;
                }
                let idx = i - BASE;
                if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    gone.push(idx);
                    continue;
                }
                if pfd.revents & libc::POLLIN != 0 && self.read_device(idx).is_err() {
                    gone.push(idx);
                }
            }
            for idx in gone.into_iter().rev() {
                let mut d = self.devs.remove(idx);
                self.known.retain(|k, _| k.0 != d.path);
                // An unplugged controller releases whatever it held.
                if let Some(p) = d.pad.as_mut() {
                    let mut out = Vec::new();
                    p.track.release_all(p.index, &mut out);
                    self.emit_pad(d.class, out, Instant::now());
                }
            }
            let now = Instant::now();
            if self.pointer_due.is_some_and(|p| p <= now) {
                self.pointer_tick(now);
            }
            if self.opts.forward && self.x.is_some() && self.next_fg <= now {
                self.next_fg = now + FOREGROUND_EVERY;
                self.refresh_foreground();
            }
            if self.next_rescan <= now {
                self.rescan();
            }
        }
    }

    fn read_device(&mut self, idx: usize) -> std::io::Result<()> {
        let now_i = Instant::now();
        let now_m = mono_now();
        let events: Vec<(u16, u16, i32, SystemTime)> = self.devs[idx]
            .raw
            .fetch_events()?
            .map(|e| (e.event_type().0, e.code(), e.value(), e.timestamp()))
            .collect();
        for (ty, code, value, ts) in events {
            let d = &mut self.devs[idx];
            match ty {
                EV_SYN if code == SYN_DROPPED => {
                    d.dropping = true;
                    d.frame.clear();
                }
                EV_SYN if code == SYN_REPORT => {
                    if d.dropping {
                        d.dropping = false;
                        d.frame.clear();
                        continue;
                    }
                    let t = to_instant(ts, d.mono, now_i, now_m);
                    let frame = std::mem::take(&mut d.frame);
                    let class = d.class;
                    if class.gamepad {
                        self.process_pad_frame(idx, &frame, t);
                    } else {
                        self.process_frame(class, &frame, t);
                    }
                    let mut frame = frame;
                    frame.clear();
                    self.devs[idx].frame = frame;
                }
                _ if d.dropping => {}
                EV_KEY => d.frame.keys.push((code, value)),
                EV_ABS if d.class.gamepad => d.frame.abs.push((code, value)),
                EV_REL => match code {
                    REL_X => d.frame.dx += value,
                    REL_Y => d.frame.dy += value,
                    REL_WHEEL => d.frame.wheel += value,
                    REL_HWHEEL => d.frame.hwheel += value,
                    REL_WHEEL_HI_RES => *d.frame.wheel_hi.get_or_insert(0) += value,
                    REL_HWHEEL_HI_RES => *d.frame.hwheel_hi.get_or_insert(0) += value,
                    _ => {}
                },
                EV_ABS if code <= 1 => d.frame.abs_motion = true,
                _ => {}
            }
        }
        Ok(())
    }

    fn note_activity(&mut self, t: Instant) {
        if self.last_activity.is_none_or(|l| t.duration_since(l) > Duration::from_millis(40)) {
            self.last_activity = Some(t);
            if self.opts.watch_activity {
                emit(Captured::UserActivity { t });
            }
        }
    }

    fn query_pointer(&mut self) -> Option<(i32, i32)> {
        if !self.x_pointer {
            return None;
        }
        self.last_query = Some(Instant::now());
        self.x.as_ref()?.pointer()
    }

    /// Reads the pointer now and records a move if it changed. Returns true
    /// when the position changed.
    fn sample_pointer(&mut self, t: Instant) -> bool {
        let Some(p) = self.query_pointer() else { return false };
        if p == self.pos {
            return false;
        }
        self.pos = p;
        if self.opts.forward && self.opts.mouse_move && !self.fg_excluded {
            emit(Captured::Input { t, kind: EventKind::MouseMove { x: p.0, y: p.1 }, injected: false });
        }
        true
    }

    fn pointer_tick(&mut self, now: Instant) {
        self.pointer_due = None;
        if self.sample_pointer(now) {
            // The server may still be applying motion: look again shortly.
            self.pointer_due = Some(now + POINTER_EVERY);
        }
    }

    fn schedule_pointer(&mut self, now: Instant) {
        if !self.x_pointer {
            return;
        }
        let due = match self.last_query {
            Some(l) if now.duration_since(l) < POINTER_EVERY => l + POINTER_EVERY,
            _ => now,
        };
        if due <= now {
            self.pointer_tick(now);
        } else if self.pointer_due.is_none() {
            self.pointer_due = Some(due);
        }
    }

    /// Re-evaluates the foreground window (one round trip when unchanged).
    fn refresh_foreground(&mut self) {
        let Some(x) = self.x.clone() else { return };
        let id = x.active_window_id();
        if id == self.fg_window {
            return;
        }
        self.fg_window = id;
        match id.and_then(|w| x.window(w)) {
            Some(window) => {
                let excluded = (self.opts.exclude_self && window.pid == self.own_pid) || self.opts.is_excluded(&window.process);
                self.fg_excluded = excluded;
                if self.opts.forward {
                    emit(Captured::Foreground { t: Instant::now(), window, excluded });
                }
            }
            None => self.fg_excluded = false,
        }
    }

    /// Forwards controller events, applying the same rules as keys.
    fn emit_pad(&mut self, class: Class, events: Vec<EventKind>, t: Instant) {
        if events.is_empty() {
            return;
        }
        let injected = class.virtual_bus && !class.own;
        let press = events.iter().any(|e| matches!(e, EventKind::PadButton { down: true, .. }));
        if press && !class.virtual_bus && !class.own {
            self.note_activity(t);
        }
        if !(self.opts.forward && self.opts.gamepad && (!injected || self.opts.include_injected)) {
            return;
        }
        if press {
            self.refresh_foreground();
        }
        if self.fg_excluded {
            return;
        }
        for kind in events {
            emit(Captured::Input { t, kind, injected });
        }
    }

    fn process_pad_frame(&mut self, idx: usize, f: &Frame, t: Instant) {
        let d = &mut self.devs[idx];
        let class = d.class;
        let Some(p) = d.pad.as_mut() else { return };
        let mut out = Vec::new();
        for &(code, value) in &f.keys {
            p.track.key(&p.layout, p.index, code, value, &mut out);
        }
        for &(code, value) in &f.abs {
            p.track.abs(&p.layout, p.index, code, value, &mut out);
        }
        self.emit_pad(class, out, t);
    }

    fn process_frame(&mut self, class: Class, f: &Frame, t: Instant) {
        let physical = !class.virtual_bus && !class.own;
        let injected = class.virtual_bus && !class.own;
        let forward = self.opts.forward;
        let source_ok = !injected || self.opts.include_injected;

        // 1. Motion.
        if f.dx != 0 || f.dy != 0 || f.abs_motion {
            if physical {
                self.note_activity(t);
            }
            if f.dx != 0 || f.dy != 0 {
                if !self.x_pointer {
                    self.pos = (self.pos.0 + f.dx, self.pos.1 + f.dy);
                }
                if forward && self.opts.mouse_raw && source_ok && !self.fg_excluded {
                    emit(Captured::Input { t, kind: EventKind::MouseRaw { dx: f.dx, dy: f.dy }, injected });
                }
            }
            self.schedule_pointer(t);
        }

        // 2. Wheel (high resolution when the device provides it).
        let wheels = [
            (f.wheel_hi.unwrap_or(f.wheel * 120), false),
            (f.hwheel_hi.unwrap_or(f.hwheel * 120), true),
        ];
        for (delta, horizontal) in wheels {
            if delta == 0 {
                continue;
            }
            if physical {
                self.note_activity(t);
            }
            if forward && self.opts.wheel && source_ok {
                self.sample_pointer(t);
                if !self.fg_excluded {
                    let (x, y) = self.pos;
                    emit(Captured::Input { t, kind: EventKind::Wheel { delta, horizontal, x, y }, injected });
                }
            }
        }

        // 3. Keys and buttons, in device order.
        for &(code, value) in &f.keys {
            if let Some(button) = button_for(code) {
                if value == 2 {
                    continue;
                }
                let down = value == 1;
                if !class.own
                    && let Some(h) = HELD.get(code as usize) {
                        h.store(down, Ordering::Relaxed);
                    }
                if physical {
                    self.note_activity(t);
                }
                if forward && self.opts.mouse_buttons && source_ok {
                    if down {
                        self.refresh_foreground();
                    }
                    self.sample_pointer(t);
                    if !self.fg_excluded {
                        let (x, y) = self.pos;
                        let kind = if down {
                            EventKind::MouseDown { button, x, y }
                        } else {
                            EventKind::MouseUp { button, x, y }
                        };
                        emit(Captured::Input { t, kind, injected });
                    }
                }
                continue;
            }
            if !((code > 0 && code < 0x100) || (0x160..0x300).contains(&code)) {
                continue; // touch, tool and gamepad buttons
            }
            let table = keys::key_from_evdev(code);
            let vk = match &self.keymap {
                Some(m) if table.scan & 0xF000 != keys::EVDEV_PRIVATE => m.logical_vk(code, table.vk),
                _ => table.vk,
            };
            let key = Key::new(table.scan, vk);
            let down = value != 0;
            let repeat = value == 2;
            if physical {
                self.note_activity(t);
                let hotkeys = G.hotkeys.read();
                match self.matcher.feed(key, down, &hotkeys) {
                    MatchResult::Trigger(action) => {
                        drop(hotkeys);
                        if let Some(h) = HELD.get(code as usize) {
                            h.store(true, Ordering::Relaxed);
                        }
                        emit(Captured::Hotkey { t, action });
                        continue;
                    }
                    MatchResult::Swallow => {
                        if let Some(h) = HELD.get(code as usize) {
                            h.store(down, Ordering::Relaxed);
                        }
                        continue;
                    }
                    MatchResult::Pass => {}
                }
            }
            if !class.own
                && let Some(h) = HELD.get(code as usize) {
                    h.store(down, Ordering::Relaxed);
                }
            if forward && self.opts.keyboard && source_ok {
                if down && !repeat {
                    self.refresh_foreground();
                }
                if !self.fg_excluded {
                    let kind = if down { EventKind::KeyDown { key, repeat } } else { EventKind::KeyUp { key } };
                    emit(Captured::Input { t, kind, injected });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_timestamps_map_to_instants() {
        let now_i = Instant::now();
        let now_m = mono_now();
        let ev = UNIX_EPOCH + (now_m - Duration::from_millis(3));
        let t = to_instant(ev, true, now_i, now_m);
        let age = now_i.duration_since(t);
        assert!(age >= Duration::from_millis(3) && age < Duration::from_millis(4), "{age:?}");
        // Realtime stamps (clock switch failed) fall back to "now".
        assert_eq!(to_instant(SystemTime::now(), false, now_i, now_m), now_i);
        // Nonsense stamps too.
        assert_eq!(to_instant(UNIX_EPOCH, true, now_i, now_m), now_i);
    }

    #[test]
    fn mouse_buttons_map() {
        assert_eq!(button_for(0x110), Some(MouseButton::Left));
        assert_eq!(button_for(0x113), Some(MouseButton::X1));
        assert_eq!(button_for(0x115), Some(MouseButton::X2));
        assert_eq!(button_for(0x14a), None);
    }
}
