//! Gamepads on Windows.
//!
//! Capture polls XInput: it needs no permission, works while another window
//! has the focus, and covers Xbox controllers plus everything that presents
//! itself as one (Steam Input, DS4Windows, 8BitDo in X-input mode…). Other
//! HID controllers are not seen by XInput.
//!
//! Playback drives virtual Xbox 360 controllers through the ViGEmBus driver.
//! They are created the first time a macro needs them and stay plugged in
//! until Chud exits (the driver removes them when our handle closes), because
//! many games pause or show a dialog when a controller disappears. Between
//! runs they rest in the neutral position.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use chud_core::event::{EventKind, PadAxis, PadButton};
use chud_core::playback::Action;
use parking_lot::Mutex;
use vigem_client::{Client, TargetId, XGamepad, Xbox360Wired};
use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::{capture, window_process};
use crate::{Captured, PlatformError, Result};

/// XInput supports four controllers.
pub const SLOTS: usize = 4;

/// XInput button bits and their SDL-layout equivalent. XInput reports no
/// Guide button through its public API; virtual controllers accept it.
const BUTTONS: [(u16, PadButton); 15] = [
    (0x1000, PadButton::SOUTH),
    (0x2000, PadButton::EAST),
    (0x4000, PadButton::WEST),
    (0x8000, PadButton::NORTH),
    (0x0020, PadButton::BACK),
    (0x0400, PadButton::GUIDE),
    (0x0010, PadButton::START),
    (0x0040, PadButton::LEFT_STICK),
    (0x0080, PadButton::RIGHT_STICK),
    (0x0100, PadButton::LEFT_SHOULDER),
    (0x0200, PadButton::RIGHT_SHOULDER),
    (0x0001, PadButton::DPAD_UP),
    (0x0002, PadButton::DPAD_DOWN),
    (0x0004, PadButton::DPAD_LEFT),
    (0x0008, PadButton::DPAD_RIGHT),
];

fn xbutton(b: PadButton) -> Option<u16> {
    BUTTONS.iter().find(|(_, p)| *p == b).map(|(bit, _)| *bit)
}

// ---------------------------------------------------------------------------
// Value conversions (XInput ⇄ Chud's SDL convention)
// ---------------------------------------------------------------------------

/// XInput sticks grow upward, Chud's (SDL) grow downward.
pub fn flip_y(v: i16) -> i16 {
    v.saturating_neg()
}

pub fn trigger_to_axis(v: u8) -> i16 {
    (v as i32 * 32767 / 255) as i16
}

pub fn axis_to_trigger(v: i16) -> u8 {
    ((v.max(0) as i32 * 255 + 16_383) / 32_767).min(255) as u8
}

/// One controller's state in Chud's units: buttons as XInput bits, axes in
/// `PadAxis` order (LX, LY, RX, RY, LT, RT).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub buttons: u16,
    pub axes: [i16; 6],
}

impl PadState {
    pub fn from_xinput(buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16) -> Self {
        Self { buttons, axes: [lx, flip_y(ly), rx, flip_y(ry), trigger_to_axis(lt), trigger_to_axis(rt)] }
    }

    fn report(&self) -> XGamepad {
        XGamepad {
            buttons: vigem_client::XButtons { raw: self.buttons },
            left_trigger: axis_to_trigger(self.axes[4]),
            right_trigger: axis_to_trigger(self.axes[5]),
            thumb_lx: self.axes[0],
            thumb_ly: flip_y(self.axes[1]),
            thumb_rx: self.axes[2],
            thumb_ry: flip_y(self.axes[3]),
        }
    }

    /// Applies one playback action. Returns false when it has no XInput
    /// equivalent (paddles, touchpad, raw joystick buttons…).
    pub fn apply(&mut self, a: &Action) -> bool {
        match *a {
            Action::PadButton { button, down, .. } => match xbutton(button) {
                Some(bit) => {
                    if down {
                        self.buttons |= bit;
                    } else {
                        self.buttons &= !bit;
                    }
                    true
                }
                None => false,
            },
            Action::PadAxis { axis, value, .. } if (axis.0 as usize) < 6 => {
                self.axes[axis.0 as usize] = value;
                true
            }
            _ => false,
        }
    }
}

/// Events that turn `prev` into `next`: releases first, then presses, then axes.
pub fn diff(prev: &PadState, next: &PadState, pad: u8, out: &mut Vec<EventKind>) {
    let changed = prev.buttons ^ next.buttons;
    if changed != 0 {
        for down in [false, true] {
            for (bit, button) in BUTTONS {
                if changed & bit != 0 && (next.buttons & bit != 0) == down {
                    out.push(EventKind::PadButton { pad, button, down });
                }
            }
        }
    }
    for (i, (a, b)) in prev.axes.iter().zip(next.axes.iter()).enumerate() {
        if a != b {
            out.push(EventKind::PadAxis { pad, axis: PadAxis(i as u8), value: *b });
        }
    }
}

fn read(slot: usize) -> Option<PadState> {
    let mut st = XINPUT_STATE::default();
    if unsafe { XInputGetState(slot as u32, &mut st) } != 0 {
        return None;
    }
    let g = st.Gamepad;
    Some(PadState::from_xinput(g.wButtons.0, g.bLeftTrigger, g.bRightTrigger, g.sThumbLX, g.sThumbLY, g.sThumbRX, g.sThumbRY))
}

/// Bit mask of the XInput slots that currently have a controller.
pub fn connected_mask() -> u8 {
    (0..SLOTS).filter(|&s| read(s).is_some()).fold(0, |m, s| m | 1 << s)
}

/// Physical (non-Chud) controllers currently connected.
pub fn physical_count() -> usize {
    (connected_mask() & !OWNED.load(Ordering::Acquire)).count_ones() as usize
}

// ---------------------------------------------------------------------------
// Capture: XInput polling thread
// ---------------------------------------------------------------------------

/// XInput slots that belong to Chud's own virtual controllers: never
/// recorded, and never counted as user activity.
static OWNED: AtomicU8 = AtomicU8::new(0);
static WANTED: AtomicBool = AtomicBool::new(false);
/// Options changed: exclusion rules must be evaluated again.
static RECONFIGURED: AtomicBool = AtomicBool::new(false);
static POLLER: Mutex<Option<std::thread::Thread>> = Mutex::new(None);

/// Called by the capture layer whenever options change.
pub fn reconfigure(wanted: bool) {
    WANTED.store(wanted, Ordering::Release);
    RECONFIGURED.store(true, Ordering::Release);
    if !wanted {
        return; // the poller notices on its next tick and goes idle
    }
    let mut poller = POLLER.lock();
    match poller.as_ref() {
        Some(t) => t.unpark(),
        None => match std::thread::Builder::new().name("chud-gamepad".into()).spawn(poll_loop) {
            Ok(h) => *poller = Some(h.thread().clone()),
            Err(e) => log::error!("gamepad thread: {e}"),
        },
    }
}

/// Caches the foreground exclusion decision per window.
#[derive(Default)]
struct Foreground {
    hwnd: usize,
    excluded: bool,
}

impl Foreground {
    fn excluded(&mut self, own_pid: u32) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0 as usize != self.hwnd {
            self.hwnd = fg.0 as usize;
            let (pid, process) = window_process(fg);
            self.excluded = capture::with_options(|o| (o.exclude_self && pid == own_pid) || o.is_excluded(&process));
        }
        self.excluded
    }
}

/// Physical activity for abort-on-input: a button press or a clear stick
/// or trigger movement.
fn is_activity(reference: &PadState, next: &PadState) -> bool {
    next.buttons & !reference.buttons != 0
        || reference.axes.iter().zip(next.axes.iter()).any(|(a, b)| (*a as i32 - *b as i32).abs() > 8_000)
}

fn poll_loop() {
    let own_pid = std::process::id();
    let mut pads: [Option<PadState>; SLOTS] = [None; SLOTS];
    let mut reference: [PadState; SLOTS] = [PadState::default(); SLOTS];
    let mut fg = Foreground::default();
    let mut active = false;
    let mut next_scan = Instant::now();
    let mut last_activity: Option<Instant> = None;
    let mut out: Vec<EventKind> = Vec::with_capacity(24);
    loop {
        if !WANTED.load(Ordering::Acquire) {
            if active {
                active = false;
                pads = [None; SLOTS]; // a fresh baseline next time
            }
            std::thread::park_timeout(Duration::from_millis(500));
            continue;
        }
        if !active {
            active = true;
            next_scan = Instant::now();
        }
        if RECONFIGURED.swap(false, Ordering::AcqRel) {
            fg = Foreground::default();
        }
        let now = Instant::now();
        // XInputGetState on an empty slot is slow: look for new controllers
        // once per second only.
        let scan = now >= next_scan;
        if scan {
            next_scan = now + Duration::from_secs(1);
        }
        let (forward, include_own, watch) = capture::with_options(|o| (o.forward && o.gamepad, o.include_own, o.watch_activity));
        let owned = OWNED.load(Ordering::Acquire);
        let mut any = false;
        for (slot, pad) in pads.iter_mut().enumerate() {
            if pad.is_none() && !scan {
                continue;
            }
            let state = read(slot);
            let t = Instant::now();
            let ours = owned & (1 << slot) != 0 && !include_own;
            let prev = *pad;
            *pad = state;
            any |= state.is_some();
            let next = match (prev, state) {
                // Newly seen controller: its current state is the baseline.
                (None, Some(s)) => {
                    reference[slot] = s;
                    continue;
                }
                (Some(p), Some(s)) if p != s => s,
                // Unplugged: whatever it held is released.
                (Some(p), None) if p != PadState::default() => PadState::default(),
                _ => continue,
            };
            if ours {
                continue;
            }
            if watch && is_activity(&reference[slot], &next) {
                reference[slot] = next;
                if last_activity.is_none_or(|l| t.duration_since(l) > Duration::from_millis(40)) {
                    last_activity = Some(t);
                    capture::emit(Captured::UserActivity { t });
                }
            }
            if forward && !fg.excluded(own_pid) {
                out.clear();
                diff(&prev.unwrap_or_default(), &next, slot as u8, &mut out);
                for kind in out.drain(..) {
                    capture::emit(Captured::Input { t, kind, injected: false });
                }
            }
        }
        std::thread::sleep(Duration::from_millis(if any { 1 } else { 50 }));
    }
}

// ---------------------------------------------------------------------------
// Playback: ViGEmBus virtual Xbox 360 controllers
// ---------------------------------------------------------------------------

struct Virtual {
    target: Xbox360Wired<Arc<Client>>,
    state: PadState,
}

struct Bank {
    client: Option<Arc<Client>>,
    /// Last failed connection, to avoid probing the driver on every batch.
    failed_at: Option<Instant>,
    pads: [Option<Virtual>; SLOTS],
}

static BANK: Mutex<Bank> = Mutex::new(Bank { client: None, failed_at: None, pads: [const { None }; SLOTS] });

fn vigem_err(e: vigem_client::Error) -> PlatformError {
    PlatformError::System(format!("ViGEmBus: {e}"))
}

impl Bank {
    fn client(&mut self) -> Result<Arc<Client>> {
        if let Some(c) = &self.client {
            return Ok(c.clone());
        }
        if self.failed_at.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            return Err(PlatformError::NoGamepadOutput);
        }
        match Client::connect() {
            Ok(c) => {
                let c = Arc::new(c);
                self.client = Some(c.clone());
                self.failed_at = None;
                Ok(c)
            }
            Err(e) => {
                log::info!("ViGEmBus unavailable: {e}");
                self.failed_at = Some(Instant::now());
                Err(PlatformError::NoGamepadOutput)
            }
        }
    }

    fn pad(&mut self, i: usize) -> Result<&mut Virtual> {
        if self.pads[i].is_none() {
            let client = self.client()?;
            let before = connected_mask();
            let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
            target.plugin().map_err(vigem_err)?;
            target.wait_ready().map_err(vigem_err)?;
            let slot = find_slot(&mut target, before);
            match slot {
                Some(s) => {
                    OWNED.fetch_or(1 << s, Ordering::AcqRel);
                }
                None => log::warn!("virtual controller {i}: XInput slot unknown"),
            }
            log::info!("virtual controller {i} plugged in (XInput slot {slot:?})");
            self.pads[i] = Some(Virtual { target, state: PadState::default() });
        }
        Ok(self.pads[i].as_mut().expect("just created"))
    }
}

/// The XInput slot Windows gave a new virtual controller. The driver knows
/// it once XInput has enumerated the device; failing that, the one slot
/// that appeared since `before` is ours.
fn find_slot(target: &mut Xbox360Wired<Arc<Client>>, before: u8) -> Option<u8> {
    for _ in 0..50 {
        if let Ok(i) = target.get_user_index()
            && (i as usize) < SLOTS
        {
            return Some(i as u8);
        }
        let new = connected_mask() & !before & !OWNED.load(Ordering::Acquire);
        if new.count_ones() == 1 {
            return Some(new.trailing_zeros() as u8);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Whether gamepad playback is possible (the driver answers).
pub fn output_available() -> bool {
    BANK.lock().client().is_ok()
}

pub fn prepare(count: u8) -> Result<()> {
    let mut bank = BANK.lock();
    for i in 0..(count as usize).min(SLOTS) {
        bank.pad(i)?;
    }
    Ok(())
}

/// Sends the gamepad actions of a batch (other actions are ignored).
pub fn send(actions: &[Action]) -> Result<()> {
    if !actions.iter().any(|a| matches!(a, Action::PadButton { .. } | Action::PadAxis { .. })) {
        return Ok(());
    }
    let mut bank = BANK.lock();
    let mut dirty = 0u8;
    for a in actions {
        let pad = match *a {
            Action::PadButton { pad, .. } | Action::PadAxis { pad, .. } => pad as usize,
            _ => continue,
        };
        if pad >= SLOTS {
            continue;
        }
        let v = bank.pad(pad)?;
        if v.state.apply(a) {
            dirty |= 1 << pad;
        }
    }
    for (i, slot) in bank.pads.iter_mut().enumerate() {
        if dirty & (1 << i) != 0
            && let Some(v) = slot
        {
            let report = v.state.report();
            v.target.update(&report).map_err(vigem_err)?;
        }
    }
    Ok(())
}

/// Returns every virtual controller to rest (nothing pressed, sticks
/// centred, triggers released).
pub fn neutral() {
    let mut bank = BANK.lock();
    for v in bank.pads.iter_mut().flatten() {
        if v.state != PadState::default() {
            v.state = PadState::default();
            let _ = v.target.update(&v.state.report());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(axis_to_trigger(trigger_to_axis(v)), v);
        }
        assert_eq!(trigger_to_axis(255), 32767);
        assert_eq!(axis_to_trigger(-5), 0);
    }

    #[test]
    fn stick_y_is_flipped_without_overflow() {
        assert_eq!(flip_y(0), 0);
        assert_eq!(flip_y(32767), -32767);
        assert_eq!(flip_y(-32768), 32767);
        let s = PadState::from_xinput(0, 0, 0, 100, 20_000, -5, -32768);
        assert_eq!(s.axes[1], -20_000); // pushed up = negative in SDL
        assert_eq!(s.report().thumb_ly, 20_000);
        assert_eq!(s.report().thumb_ry, -32767);
    }

    #[test]
    fn diff_orders_releases_before_presses() {
        let a = PadState { buttons: 0x1000 | 0x0100, axes: [0; 6] };
        let b = PadState { buttons: 0x2000 | 0x0100, axes: [0, 0, 0, 0, 32767, 0] };
        let mut out = Vec::new();
        diff(&a, &b, 2, &mut out);
        assert_eq!(
            out,
            vec![
                EventKind::PadButton { pad: 2, button: PadButton::SOUTH, down: false },
                EventKind::PadButton { pad: 2, button: PadButton::EAST, down: true },
                EventKind::PadAxis { pad: 2, axis: PadAxis::LEFT_TRIGGER, value: 32767 },
            ]
        );
    }

    #[test]
    fn actions_rebuild_the_report() {
        let mut s = PadState::default();
        assert!(s.apply(&Action::PadButton { pad: 0, button: PadButton::NORTH, down: true }));
        assert!(s.apply(&Action::PadButton { pad: 0, button: PadButton::DPAD_LEFT, down: true }));
        assert!(s.apply(&Action::PadAxis { pad: 0, axis: PadAxis::LEFT_Y, value: -32768 }));
        assert!(s.apply(&Action::PadAxis { pad: 0, axis: PadAxis::RIGHT_TRIGGER, value: 32767 }));
        assert!(!s.apply(&Action::PadButton { pad: 0, button: PadButton::TOUCHPAD, down: true }));
        let r = s.report();
        assert_eq!(r.buttons.raw, 0x8000 | 0x0004);
        assert_eq!(r.thumb_ly, 32767);
        assert_eq!(r.right_trigger, 255);
        assert!(s.apply(&Action::PadButton { pad: 0, button: PadButton::NORTH, down: false }));
        assert_eq!(s.report().buttons.raw, 0x0004);
    }

    #[test]
    fn every_mapped_button_round_trips() {
        for (bit, button) in BUTTONS {
            let mut s = PadState::default();
            s.apply(&Action::PadButton { pad: 0, button, down: true });
            assert_eq!(s.buttons, bit);
            let mut out = Vec::new();
            diff(&PadState::default(), &s, 0, &mut out);
            assert_eq!(out, vec![EventKind::PadButton { pad: 0, button, down: true }]);
        }
    }
}
