//! Gamepads on Linux.
//!
//! Capture reads controllers from evdev like keyboards and mice, so every
//! controller the kernel knows works: Xbox (xpad), PlayStation
//! (hid-playstation, hid-sony), Nintendo (hid-nintendo), Steam Input's
//! virtual pads and generic joysticks. Codes follow the kernel gamepad
//! specification (`Documentation/input/gamepad.rst`), with one historical
//! exception: xpad reports X with the code of `BTN_NORTH` and Y with the
//! code of `BTN_WEST` (their old names were `BTN_X` and `BTN_Y`).
//!
//! Playback creates virtual Xbox 360 controllers through `/dev/uinput`,
//! declared exactly like xpad declares a real one, so SDL, Steam and games
//! recognise them as such.

use std::time::Duration;

use chud_core::event::{EventKind, PadAxis, PadButton};
use chud_core::playback::Action;
use evdev::raw_stream::RawDevice;
use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputEvent, InputId, KeyCode, UinputAbsSetup};

use super::capture::OWN_DEVICE_PREFIX;
use crate::{PlatformError, Result};

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;

const BTN_JOYSTICK: u16 = 0x120;
const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const BTN_TRIGGER_HAPPY: u16 = 0x2c0;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_GAS: u16 = 0x09;
const ABS_BRAKE: u16 = 0x0a;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
/// Codes from here on are multitouch and misc, never controller axes.
const ABS_MISC: u16 = 0x28;

/// Whether a device is a gamepad or joystick (not a keyboard, mouse,
/// touchpad or motion sensor).
pub fn is_gamepad(has_key: impl Fn(u16) -> bool, has_abs: impl Fn(u16) -> bool) -> bool {
    has_key(BTN_SOUTH) || ((BTN_JOYSTICK..BTN_JOYSTICK + 16).any(&has_key) && has_abs(ABS_X))
}

/// How one device's codes map onto Chud's controller model.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// X and Y on the codes of `BTN_NORTH` and `BTN_WEST` (xpad).
    pub xbox_xy: bool,
    /// `ABS_Z`/`ABS_RZ` are the triggers; otherwise (generic HID pads
    /// without `ABS_RX`/`ABS_RY`) they are the right stick.
    pub z_triggers: bool,
    /// Analog triggers exist: the digital `BTN_TL2`/`BTN_TR2` are ignored.
    pub analog_lt: bool,
    pub analog_rt: bool,
    /// `(min, max)` per absolute axis code.
    pub ranges: Vec<Option<(i32, i32)>>,
}

impl Layout {
    pub fn new(xbox_xy: bool, axes: &[(u16, i32, i32)]) -> Self {
        let mut ranges = vec![None; ABS_MISC as usize];
        for &(code, min, max) in axes {
            if (code as usize) < ranges.len() && max > min {
                ranges[code as usize] = Some((min, max));
            }
        }
        let has = |c: u16| ranges[c as usize].is_some();
        let z_triggers = (has(ABS_RX) && has(ABS_RY)) || !(has(ABS_Z) && has(ABS_RZ));
        let analog_lt = (z_triggers && has(ABS_Z)) || has(ABS_BRAKE);
        let analog_rt = (z_triggers && has(ABS_RZ)) || has(ABS_GAS);
        Self { xbox_xy, z_triggers, analog_lt, analog_rt, ranges }
    }

    pub fn of(raw: &RawDevice, driver: &str, own: bool) -> Self {
        let axes: Vec<(u16, i32, i32)> = raw
            .get_absinfo()
            .map(|it| it.map(|(code, info)| (code.0, info.minimum(), info.maximum())).collect())
            .unwrap_or_default();
        Self::new(driver == "xpad" || own, &axes)
    }

    pub fn button(&self, code: u16) -> Option<PadButton> {
        Some(match code {
            BTN_SOUTH => PadButton::SOUTH,
            BTN_EAST => PadButton::EAST,
            BTN_NORTH if self.xbox_xy => PadButton::WEST,
            BTN_NORTH => PadButton::NORTH,
            BTN_WEST if self.xbox_xy => PadButton::NORTH,
            BTN_WEST => PadButton::WEST,
            BTN_TL => PadButton::LEFT_SHOULDER,
            BTN_TR => PadButton::RIGHT_SHOULDER,
            BTN_SELECT => PadButton::BACK,
            BTN_START => PadButton::START,
            BTN_MODE => PadButton::GUIDE,
            BTN_THUMBL => PadButton::LEFT_STICK,
            BTN_THUMBR => PadButton::RIGHT_STICK,
            0x220 => PadButton::DPAD_UP,
            0x221 => PadButton::DPAD_DOWN,
            0x222 => PadButton::DPAD_LEFT,
            0x223 => PadButton::DPAD_RIGHT,
            // Generic joystick buttons and the rarely used BTN_C / BTN_Z.
            c @ BTN_JOYSTICK..=0x135 => PadButton(PadButton::RAW_BASE + (c - BTN_JOYSTICK) as u8),
            // Extra buttons (paddles, Steam Deck back buttons…).
            c @ BTN_TRIGGER_HAPPY..=0x2e7 => PadButton(PadButton::RAW_BASE + 32 + (c - BTN_TRIGGER_HAPPY) as u8),
            _ => return None,
        })
    }

    fn axis(&self, code: u16) -> Option<(PadAxis, bool)> {
        // (axis, is_trigger)
        Some(match code {
            ABS_X => (PadAxis::LEFT_X, false),
            ABS_Y => (PadAxis::LEFT_Y, false),
            ABS_RX => (PadAxis::RIGHT_X, false),
            ABS_RY => (PadAxis::RIGHT_Y, false),
            ABS_Z if self.z_triggers => (PadAxis::LEFT_TRIGGER, true),
            ABS_RZ if self.z_triggers => (PadAxis::RIGHT_TRIGGER, true),
            ABS_Z => (PadAxis::RIGHT_X, false),
            ABS_RZ => (PadAxis::RIGHT_Y, false),
            ABS_BRAKE => (PadAxis::LEFT_TRIGGER, true),
            ABS_GAS => (PadAxis::RIGHT_TRIGGER, true),
            ABS_HAT0X | ABS_HAT0Y => return None,
            c if c < ABS_MISC => (PadAxis(PadAxis::RAW_BASE + c as u8), false),
            _ => return None,
        })
    }
}

/// Maps a raw value onto Chud's ranges: sticks `-32768..=32767`, triggers
/// `0..=32767`.
pub fn normalize(v: i32, (min, max): (i32, i32), trigger: bool) -> i16 {
    if max <= min {
        return 0;
    }
    let f = (v.clamp(min, max) - min) as f64 / (max - min) as f64;
    if trigger {
        (f * 32767.0).round() as i16
    } else {
        (f * 65535.0 - 32768.0).round().clamp(-32768.0, 32767.0) as i16
    }
}

/// Per-device state turning evdev events into Chud events.
#[derive(Debug, Default)]
pub struct PadTrack {
    hat: [i32; 2],
    held: Vec<PadButton>,
    axes: Vec<(PadAxis, i16)>,
}

impl PadTrack {
    fn press(&mut self, pad: u8, button: PadButton, down: bool, out: &mut Vec<EventKind>) {
        let was = self.held.contains(&button);
        if was == down {
            return;
        }
        if down {
            self.held.push(button);
        } else {
            self.held.retain(|b| *b != button);
        }
        out.push(EventKind::PadButton { pad, button, down });
    }

    fn set_axis(&mut self, pad: u8, axis: PadAxis, value: i16, out: &mut Vec<EventKind>) {
        let prev = self.axes.iter().find(|(a, _)| *a == axis).map(|(_, v)| *v).unwrap_or(0);
        if prev == value {
            return;
        }
        self.axes.retain(|(a, _)| *a != axis);
        if value != 0 {
            self.axes.push((axis, value));
        }
        out.push(EventKind::PadAxis { pad, axis, value });
    }

    pub fn key(&mut self, l: &Layout, pad: u8, code: u16, value: i32, out: &mut Vec<EventKind>) {
        if value == 2 {
            return; // autorepeat
        }
        let down = value != 0;
        match code {
            BTN_TL2 | BTN_TR2 => {
                // Digital triggers count only on controllers without analog ones.
                let (axis, analog) = if code == BTN_TL2 { (PadAxis::LEFT_TRIGGER, l.analog_lt) } else { (PadAxis::RIGHT_TRIGGER, l.analog_rt) };
                if !analog {
                    self.set_axis(pad, axis, if down { i16::MAX } else { 0 }, out);
                }
            }
            _ => {
                if let Some(b) = l.button(code) {
                    self.press(pad, b, down, out);
                }
            }
        }
    }

    pub fn abs(&mut self, l: &Layout, pad: u8, code: u16, value: i32, out: &mut Vec<EventKind>) {
        if code == ABS_HAT0X || code == ABS_HAT0Y {
            let i = (code - ABS_HAT0X) as usize;
            let v = value.signum();
            let (neg, pos) = if i == 0 { (PadButton::DPAD_LEFT, PadButton::DPAD_RIGHT) } else { (PadButton::DPAD_UP, PadButton::DPAD_DOWN) };
            if v != self.hat[i] {
                self.hat[i] = v;
                // Releases first.
                if v != -1 {
                    self.press(pad, neg, false, out);
                }
                if v != 1 {
                    self.press(pad, pos, false, out);
                }
                if v == -1 {
                    self.press(pad, neg, true, out);
                }
                if v == 1 {
                    self.press(pad, pos, true, out);
                }
            }
            return;
        }
        let Some((axis, trigger)) = l.axis(code) else { return };
        let Some(range) = l.ranges.get(code as usize).copied().flatten() else { return };
        self.set_axis(pad, axis, normalize(value, range, trigger), out);
    }

    /// Everything back to rest (the device disappeared).
    pub fn release_all(&mut self, pad: u8, out: &mut Vec<EventKind>) {
        for b in std::mem::take(&mut self.held) {
            out.push(EventKind::PadButton { pad, button: b, down: false });
        }
        for (axis, _) in std::mem::take(&mut self.axes) {
            out.push(EventKind::PadAxis { pad, axis, value: 0 });
        }
        self.hat = [0; 2];
    }
}

/// Kernel driver bound to an evdev node (`xpad`, `playstation`…).
pub fn driver_of(path: &std::path::Path) -> String {
    let Some(name) = path.file_name() else { return String::new() };
    let link = std::path::Path::new("/sys/class/input").join(name).join("device/device/driver");
    std::fs::read_link(link)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Playback: virtual Xbox 360 controllers
// ---------------------------------------------------------------------------

pub const SLOTS: usize = 4;

/// Xbox 360 button codes, as xpad declares them (X and Y on the codes of
/// `BTN_NORTH` / `BTN_WEST`).
fn x360_code(b: PadButton) -> Option<u16> {
    Some(match b {
        PadButton::SOUTH => BTN_SOUTH,
        PadButton::EAST => BTN_EAST,
        PadButton::WEST => BTN_NORTH,
        PadButton::NORTH => BTN_WEST,
        PadButton::LEFT_SHOULDER => BTN_TL,
        PadButton::RIGHT_SHOULDER => BTN_TR,
        PadButton::BACK => BTN_SELECT,
        PadButton::START => BTN_START,
        PadButton::GUIDE => BTN_MODE,
        PadButton::LEFT_STICK => BTN_THUMBL,
        PadButton::RIGHT_STICK => BTN_THUMBR,
        _ => return None,
    })
}

pub fn trigger_to_u8(v: i16) -> i32 {
    ((v.max(0) as i32 * 255 + 16_383) / 32_767).min(255)
}

/// One virtual controller and the state it last reported.
pub struct VirtualPad {
    dev: VirtualDevice,
    dpad: [bool; 4], // up, down, left, right
    buttons: Vec<u16>,
    axes: [i16; 6],
}

fn create_pad(index: usize) -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for c in [BTN_SOUTH, BTN_EAST, BTN_NORTH, BTN_WEST, BTN_TL, BTN_TR, BTN_SELECT, BTN_START, BTN_MODE, BTN_THUMBL, BTN_THUMBR] {
        keys.insert(KeyCode(c));
    }
    let stick = AbsInfo::new(0, -32768, 32767, 16, 128, 0);
    let trigger = AbsInfo::new(0, 0, 255, 0, 0, 0);
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let name = format!("{OWN_DEVICE_PREFIX} virtual gamepad {}", index + 1);
    let mut b = VirtualDevice::builder()?
        .name(name.as_str())
        // Microsoft Xbox 360 wired controller ids, like xpad.
        .input_id(InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0110))
        .with_keys(&keys)?;
    for (code, info) in [
        (ABS_X, stick),
        (ABS_Y, stick),
        (ABS_Z, trigger),
        (ABS_RX, stick),
        (ABS_RY, stick),
        (ABS_RZ, trigger),
        (ABS_HAT0X, hat),
        (ABS_HAT0Y, hat),
    ] {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), info))?;
    }
    let mut dev = b.build()?;
    for _ in 0..50 {
        if let Ok(mut nodes) = dev.enumerate_dev_nodes_blocking()
            && nodes.next().is_some_and(|n| n.is_ok_and(|p| p.exists()))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Let the desktop and games notice the new controller.
    std::thread::sleep(Duration::from_millis(250));
    Ok(dev)
}

pub type SharedPads = std::sync::Arc<parking_lot::Mutex<[Option<VirtualPad>; SLOTS]>>;

pub fn new_shared() -> SharedPads {
    std::sync::Arc::new(parking_lot::Mutex::new([const { None }; SLOTS]))
}

fn pad(pads: &mut [Option<VirtualPad>; SLOTS], i: usize) -> Result<&mut VirtualPad> {
    if pads[i].is_none() {
        let dev = create_pad(i).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound => PlatformError::NoGamepadOutput,
            _ => PlatformError::System(format!("uinput gamepad: {e}")),
        })?;
        pads[i] = Some(VirtualPad { dev, dpad: [false; 4], buttons: Vec::new(), axes: [0; 6] });
    }
    Ok(pads[i].as_mut().expect("just created"))
}

pub fn prepare(shared: &SharedPads, count: u8) -> Result<()> {
    let mut pads = shared.lock();
    for i in 0..(count as usize).min(SLOTS) {
        pad(&mut pads, i)?;
    }
    Ok(())
}

impl VirtualPad {
    /// Events for one action (none when it has no Xbox 360 equivalent).
    fn events(&mut self, a: &Action, out: &mut Vec<InputEvent>) {
        match *a {
            Action::PadButton { button, down, .. } => {
                let d = match button {
                    PadButton::DPAD_UP => Some(0),
                    PadButton::DPAD_DOWN => Some(1),
                    PadButton::DPAD_LEFT => Some(2),
                    PadButton::DPAD_RIGHT => Some(3),
                    _ => None,
                };
                if let Some(d) = d {
                    self.dpad[d] = down;
                    let x = self.dpad[3] as i32 - self.dpad[2] as i32;
                    let y = self.dpad[1] as i32 - self.dpad[0] as i32;
                    out.push(InputEvent::new(EV_ABS, if d < 2 { ABS_HAT0Y } else { ABS_HAT0X }, if d < 2 { y } else { x }));
                } else if let Some(code) = x360_code(button) {
                    if down {
                        self.buttons.push(code);
                    } else {
                        self.buttons.retain(|c| *c != code);
                    }
                    out.push(InputEvent::new(EV_KEY, code, down as i32));
                }
            }
            Action::PadAxis { axis, value, .. } if (axis.0 as usize) < 6 => {
                self.axes[axis.0 as usize] = value;
                let (code, v) = match axis {
                    PadAxis::LEFT_X => (ABS_X, value as i32),
                    PadAxis::LEFT_Y => (ABS_Y, value as i32),
                    PadAxis::RIGHT_X => (ABS_RX, value as i32),
                    PadAxis::RIGHT_Y => (ABS_RY, value as i32),
                    PadAxis::LEFT_TRIGGER => (ABS_Z, trigger_to_u8(value)),
                    _ => (ABS_RZ, trigger_to_u8(value)),
                };
                out.push(InputEvent::new(EV_ABS, code, v));
            }
            _ => {}
        }
    }

    fn neutral(&mut self) {
        let mut out = Vec::new();
        for code in std::mem::take(&mut self.buttons) {
            out.push(InputEvent::new(EV_KEY, code, 0));
        }
        if self.dpad.iter().any(|d| *d) {
            self.dpad = [false; 4];
            out.push(InputEvent::new(EV_ABS, ABS_HAT0X, 0));
            out.push(InputEvent::new(EV_ABS, ABS_HAT0Y, 0));
        }
        // Same order as PadAxis: LX, LY, RX, RY, LT, RT.
        for (i, code) in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ].into_iter().enumerate() {
            if self.axes[i] != 0 {
                self.axes[i] = 0;
                out.push(InputEvent::new(EV_ABS, code, 0));
            }
        }
        if !out.is_empty() {
            let _ = self.dev.emit(&out);
        }
    }
}

/// Sends the gamepad actions of a batch, one report per controller.
pub fn send(shared: &SharedPads, actions: &[Action]) -> Result<()> {
    if !actions.iter().any(|a| matches!(a, Action::PadButton { .. } | Action::PadAxis { .. })) {
        return Ok(());
    }
    let mut pads = shared.lock();
    let mut frames: [Vec<InputEvent>; SLOTS] = Default::default();
    for a in actions {
        let i = match *a {
            Action::PadButton { pad, .. } | Action::PadAxis { pad, .. } => pad as usize,
            _ => continue,
        };
        if i >= SLOTS {
            continue;
        }
        pad(&mut pads, i)?.events(a, &mut frames[i]);
    }
    for (i, f) in frames.iter().enumerate() {
        if !f.is_empty()
            && let Some(p) = pads[i].as_mut()
        {
            p.dev.emit(f).map_err(|e| PlatformError::System(format!("uinput gamepad write: {e}")))?;
        }
    }
    Ok(())
}

pub fn neutral(shared: &SharedPads) {
    for p in shared.lock().iter_mut().flatten() {
        p.neutral();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xpad() -> Layout {
        Layout::new(
            true,
            &[(ABS_X, -32768, 32767), (ABS_Y, -32768, 32767), (ABS_Z, 0, 255), (ABS_RX, -32768, 32767), (ABS_RY, -32768, 32767), (ABS_RZ, 0, 255), (ABS_HAT0X, -1, 1), (ABS_HAT0Y, -1, 1)],
        )
    }

    #[test]
    fn xpad_and_spec_layouts_differ_only_on_x_y() {
        let x = xpad();
        assert_eq!(x.button(BTN_NORTH), Some(PadButton::WEST));
        assert_eq!(x.button(BTN_WEST), Some(PadButton::NORTH));
        let sony = Layout::new(false, &[(ABS_X, 0, 255), (ABS_Y, 0, 255), (ABS_RX, 0, 255), (ABS_RY, 0, 255), (ABS_Z, 0, 255), (ABS_RZ, 0, 255)]);
        assert_eq!(sony.button(BTN_NORTH), Some(PadButton::NORTH));
        assert_eq!(sony.button(BTN_WEST), Some(PadButton::WEST));
        assert!(sony.z_triggers && sony.analog_lt && sony.analog_rt);
        assert_eq!(x.button(0x2c1), Some(PadButton(PadButton::RAW_BASE + 33)));
    }

    #[test]
    fn generic_pads_use_z_rz_as_right_stick() {
        let g = Layout::new(false, &[(ABS_X, 0, 255), (ABS_Y, 0, 255), (ABS_Z, 0, 255), (ABS_RZ, 0, 255)]);
        assert!(!g.z_triggers && !g.analog_lt);
        assert_eq!(g.axis(ABS_Z), Some((PadAxis::RIGHT_X, false)));
        // Digital triggers then become full-range trigger axes.
        let mut t = PadTrack::default();
        let mut out = Vec::new();
        t.key(&g, 0, BTN_TL2, 1, &mut out);
        assert_eq!(out, vec![EventKind::PadAxis { pad: 0, axis: PadAxis::LEFT_TRIGGER, value: 32767 }]);
    }

    #[test]
    fn values_are_normalized() {
        assert_eq!(normalize(0, (-32768, 32767), false), 0);
        assert_eq!(normalize(-32768, (-32768, 32767), false), -32768);
        assert_eq!(normalize(32767, (-32768, 32767), false), 32767);
        assert_eq!(normalize(0, (0, 255), false), -32768);
        assert_eq!(normalize(255, (0, 255), false), 32767);
        assert_eq!(normalize(255, (0, 255), true), 32767);
        assert_eq!(normalize(0, (0, 1023), true), 0);
        assert_eq!(normalize(9999, (0, 255), true), 32767);
    }

    #[test]
    fn hat_becomes_dpad_buttons() {
        let l = xpad();
        let mut t = PadTrack::default();
        let mut out = Vec::new();
        t.abs(&l, 1, ABS_HAT0X, -1, &mut out);
        t.abs(&l, 1, ABS_HAT0X, 1, &mut out);
        t.abs(&l, 1, ABS_HAT0X, 0, &mut out);
        assert_eq!(
            out,
            vec![
                EventKind::PadButton { pad: 1, button: PadButton::DPAD_LEFT, down: true },
                EventKind::PadButton { pad: 1, button: PadButton::DPAD_LEFT, down: false },
                EventKind::PadButton { pad: 1, button: PadButton::DPAD_RIGHT, down: true },
                EventKind::PadButton { pad: 1, button: PadButton::DPAD_RIGHT, down: false },
            ]
        );
    }

    #[test]
    fn unplugging_releases_everything() {
        let l = xpad();
        let mut t = PadTrack::default();
        let mut out = Vec::new();
        t.key(&l, 0, BTN_SOUTH, 1, &mut out);
        t.abs(&l, 0, ABS_X, 1000, &mut out);
        out.clear();
        t.release_all(0, &mut out);
        assert_eq!(
            out,
            vec![
                EventKind::PadButton { pad: 0, button: PadButton::SOUTH, down: false },
                EventKind::PadAxis { pad: 0, axis: PadAxis::LEFT_X, value: 0 },
            ]
        );
    }

    #[test]
    fn virtual_pad_codes_round_trip_through_the_xpad_layout() {
        let l = xpad();
        for b in [PadButton::SOUTH, PadButton::EAST, PadButton::WEST, PadButton::NORTH, PadButton::BACK, PadButton::START, PadButton::GUIDE, PadButton::LEFT_STICK, PadButton::RIGHT_STICK, PadButton::LEFT_SHOULDER, PadButton::RIGHT_SHOULDER] {
            assert_eq!(l.button(x360_code(b).unwrap()), Some(b), "{}", b.name());
        }
        assert_eq!(trigger_to_u8(32767), 255);
        assert_eq!(normalize(trigger_to_u8(16384), (0, 255), true), 16448);
    }
}
