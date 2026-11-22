//! Linux injection.
//!
//! Two backends, chosen once per injector:
//!
//! * **XTest** (X11 sessions): every action goes through the X server in
//!   one ordered stream, so a modifier sent just before a click is always
//!   applied to it, and absolute moves are exact. No special permission.
//! * **uinput** (Wayland, console, or forced with `CHUD_LINUX_INJECT=uinput`):
//!   a virtual keyboard + mouse created through `/dev/uinput`, seen by the
//!   system like real hardware. Absolute pointer moves are not available
//!   under Wayland (no standard protocol), relative moves are.
//!
//! The uinput device is created once and kept for the life of the process:
//! the desktop needs a moment to adopt a new input device, and creating one
//! per playback would lose the first events.

use std::sync::Arc;
use std::time::Duration;

use chud_core::event::MouseButton;
use chud_core::keys;
use chud_core::playback::{Action, KeyVia};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, InputEvent, InputId, KeyCode, RelativeAxisCode};
use parking_lot::Mutex;

use super::capture::OWN_DEVICE_PREFIX;
use super::gamepad::{self, SharedPads};
use super::x11::{Keymap, X};
use crate::{Injector, PlatformError, Result};

const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;

pub type SharedUinput = Arc<Mutex<Option<VirtualDevice>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    XTest,
    Uinput,
}

/// Creates Chud's virtual keyboard + mouse and waits until the system has
/// picked it up.
fn create_uinput() -> std::io::Result<VirtualDevice> {
    let mut keyset = AttributeSet::<KeyCode>::new();
    for code in 1u16..0x300 {
        let mouse = (0x110..=0x117).contains(&code);
        let other_button = (0x100..0x160).contains(&code) && !mouse;
        if !other_button {
            keyset.insert(KeyCode(code));
        }
    }
    let mut rel = AttributeSet::<RelativeAxisCode>::new();
    for a in [0u16, 1, 6, 8, 11, 12] {
        rel.insert(RelativeAxisCode(a));
    }
    let name = format!("{OWN_DEVICE_PREFIX} virtual input");
    let mut dev = VirtualDevice::builder()?
        .name(name.as_str())
        .input_id(InputId::new(BusType::BUS_VIRTUAL, 0x4348, 0x5544, 1))
        .with_keys(&keyset)?
        .with_relative_axes(&rel)?
        .build()?;
    // Wait for the device node, then give the input stack time to open it.
    for _ in 0..50 {
        if let Ok(mut nodes) = dev.enumerate_dev_nodes_blocking()
            && nodes.next().is_some_and(|n| n.is_ok_and(|p| p.exists())) {
                break;
            }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(250));
    Ok(dev)
}

pub fn ensure_uinput(shared: &SharedUinput) -> Result<()> {
    let mut g = shared.lock();
    if g.is_none() {
        let dev = create_uinput().map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound => {
                PlatformError::Permission(format!("/dev/uinput: {e}"))
            }
            _ => PlatformError::System(format!("uinput: {e}")),
        })?;
        *g = Some(dev);
    }
    Ok(())
}

fn button_code(b: MouseButton) -> u16 {
    match b {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::X1 => 0x113,
        MouseButton::X2 => 0x114,
    }
}

/// X core protocol button numbers.
fn x_button(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::X1 => 8,
        MouseButton::X2 => 9,
    }
}

/// evdev code to send for a key, honouring physical or logical mode.
pub fn key_code(key: chud_core::event::Key, via: KeyVia, keymap: Option<&Keymap>) -> Option<u16> {
    match via {
        KeyVia::Vk if key.vk != 0 => keymap
            .and_then(|m| m.evdev_for_vk(key.vk))
            .or_else(|| keys::by_vk(key.vk).map(|i| i.evdev))
            .or_else(|| keys::evdev_for(key)),
        _ => keys::evdev_for(key),
    }
}

pub struct LinuxInjector {
    backend: Backend,
    x: Option<Arc<X>>,
    uinput: SharedUinput,
    pads: SharedPads,
    keymap: Option<Keymap>,
    /// Sub-notch wheel remainders (vertical, horizontal), XTest and low-res uinput.
    wheel_acc: [i32; 2],
    warned_absolute: bool,
}

impl LinuxInjector {
    pub fn new(backend: Backend, x: Option<Arc<X>>, uinput: SharedUinput, pads: SharedPads) -> Self {
        let keymap = x.as_ref().and_then(|x| x.keymap());
        Self { backend, x, uinput, pads, keymap, wheel_acc: [0; 2], warned_absolute: false }
    }

    fn notches(&mut self, delta: i32, horizontal: bool) -> i32 {
        let acc = &mut self.wheel_acc[horizontal as usize];
        *acc += delta;
        let n = *acc / 120;
        *acc -= n * 120;
        n
    }

    fn send_xtest(&mut self, actions: &[Action]) -> Result<()> {
        let Some(x) = self.x.clone() else { return Err(PlatformError::Unsupported("X11".into())) };
        let mut ok = true;
        for a in actions {
            match *a {
                Action::MoveTo { x: px, y: py } => ok &= x.fake_motion(px, py, false),
                Action::MoveBy { dx, dy } => ok &= x.fake_motion(dx, dy, true),
                Action::Button { button, down, at } => {
                    if let Some((px, py)) = at {
                        ok &= x.fake_motion(px, py, false);
                    }
                    ok &= x.fake_button(x_button(button), down);
                }
                Action::Wheel { delta, horizontal, at } => {
                    if let Some((px, py)) = at {
                        ok &= x.fake_motion(px, py, false);
                    }
                    let n = self.notches(delta, horizontal);
                    let b = match (horizontal, n > 0) {
                        (false, true) => 4,
                        (false, false) => 5,
                        (true, true) => 7,
                        (true, false) => 6,
                    };
                    for _ in 0..n.unsigned_abs().min(64) {
                        ok &= x.fake_button(b, true);
                        ok &= x.fake_button(b, false);
                    }
                }
                Action::Key { key, down, via } => {
                    if let Some(code) = key_code(key, via, self.keymap.as_ref())
                        && let Ok(kc) = u8::try_from(code + 8) {
                            ok &= x.fake_key(kc, down);
                        }
                }
                Action::PadButton { .. } | Action::PadAxis { .. } => {}
            }
        }
        ok &= x.flush();
        if ok { Ok(()) } else { Err(PlatformError::System("X server connection lost".into())) }
    }

    fn send_uinput(&mut self, actions: &[Action]) -> Result<()> {
        ensure_uinput(&self.uinput)?;
        let mut unsupported_abs = false;
        let mut frames: Vec<Vec<InputEvent>> = Vec::new();
        for a in actions {
            match *a {
                Action::MoveTo { .. } => unsupported_abs = true,
                Action::MoveBy { dx, dy } => {
                    frames.push(vec![InputEvent::new(EV_REL, 0, dx), InputEvent::new(EV_REL, 1, dy)]);
                }
                Action::Button { button, down, at } => {
                    unsupported_abs |= at.is_some();
                    frames.push(vec![InputEvent::new(EV_KEY, button_code(button), down as i32)]);
                }
                Action::Wheel { delta, horizontal, at } => {
                    unsupported_abs |= at.is_some();
                    let n = self.notches(delta, horizontal);
                    let (lo, hi) = if horizontal { (6, 12) } else { (8, 11) };
                    let mut f = vec![InputEvent::new(EV_REL, hi, delta)];
                    if n != 0 {
                        f.push(InputEvent::new(EV_REL, lo, n));
                    }
                    frames.push(f);
                }
                Action::Key { key, down, via } => {
                    if let Some(code) = key_code(key, via, self.keymap.as_ref()) {
                        frames.push(vec![InputEvent::new(EV_KEY, code, down as i32)]);
                    }
                }
                Action::PadButton { .. } | Action::PadAxis { .. } => {}
            }
        }
        let shared = self.uinput.clone();
        let mut guard = shared.lock();
        let Some(dev) = guard.as_mut() else { return Err(PlatformError::Unsupported("uinput".into())) };
        for f in &frames {
            dev.emit(f).map_err(|e| PlatformError::System(format!("uinput write: {e}")))?;
        }
        drop(guard);
        if unsupported_abs && !self.warned_absolute {
            self.warned_absolute = true;
            return Err(PlatformError::Unsupported(
                "absolute pointer positioning under Wayland (use the relative mouse mode)".into(),
            ));
        }
        Ok(())
    }
}

impl Injector for LinuxInjector {
    fn send(&mut self, actions: &[Action]) -> Result<()> {
        // Gamepad-only batches never need the keyboard/mouse device.
        let input = if actions.iter().all(|a| matches!(a, Action::PadButton { .. } | Action::PadAxis { .. })) {
            Ok(())
        } else {
            match self.backend {
                Backend::XTest => self.send_xtest(actions),
                Backend::Uinput => self.send_uinput(actions),
            }
        };
        let pads = gamepad::send(&self.pads, actions);
        input.and(pads)
    }

    fn prepare_gamepads(&mut self, count: u8) -> Result<()> {
        gamepad::prepare(&self.pads, count)
    }

    fn type_char(&mut self, c: char) -> Result<()> {
        if let Some(x) = &self.x
            && x.type_char(c) {
                return Ok(());
            }
        // Without X: US layout fallback for plain ASCII.
        let (key, shift) = ascii_key(c).ok_or_else(|| PlatformError::Unsupported(format!("typing {c:?} without X11")))?;
        let k = |code: u16, down: bool| Action::Key {
            key: keys::key_from_evdev(code),
            down,
            via: KeyVia::Scan,
        };
        let mut seq = Vec::new();
        if shift {
            seq.push(k(42, true));
        }
        seq.push(k(key, true));
        seq.push(k(key, false));
        if shift {
            seq.push(k(42, false));
        }
        self.send_uinput(&seq)
    }
}

impl Drop for LinuxInjector {
    /// Safety net: virtual controllers are left at rest whatever happened.
    fn drop(&mut self) {
        gamepad::neutral(&self.pads);
    }
}

/// evdev code and shift state for an ASCII character on a US layout.
fn ascii_key(c: char) -> Option<(u16, bool)> {
    const ROW: &[(char, char, u16)] = &[
        ('1', '!', 2), ('2', '@', 3), ('3', '#', 4), ('4', '$', 5), ('5', '%', 6), ('6', '^', 7), ('7', '&', 8),
        ('8', '*', 9), ('9', '(', 10), ('0', ')', 11), ('-', '_', 12), ('=', '+', 13), ('[', '{', 26), (']', '}', 27),
        (';', ':', 39), ('\'', '"', 40), ('`', '~', 41), ('\\', '|', 43), (',', '<', 51), ('.', '>', 52), ('/', '?', 53),
        (' ', ' ', 57), ('\n', '\n', 28), ('\t', '\t', 15),
    ];
    const LETTERS: &[u16] = &[
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,
    ];
    if c.is_ascii_lowercase() {
        return Some((LETTERS[(c as u8 - b'a') as usize], false));
    }
    if c.is_ascii_uppercase() {
        return Some((LETTERS[(c as u8 - b'A') as usize], true));
    }
    ROW.iter().find_map(|&(a, b, code)| {
        if c == a {
            Some((code, false))
        } else if c == b {
            Some((code, true))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chud_core::event::Key;

    #[test]
    fn ascii_fallback() {
        assert_eq!(ascii_key('a'), Some((30, false)));
        assert_eq!(ascii_key('Z'), Some((44, true)));
        assert_eq!(ascii_key('?'), Some((53, true)));
        assert_eq!(ascii_key('é'), None);
    }

    #[test]
    fn key_codes_by_mode() {
        // Physical A (scan 0x1E) is evdev 30 whatever the layout.
        let a = Key::new(0x1E, 0x41);
        assert_eq!(key_code(a, KeyVia::Scan, None), Some(30));
        // Logical without keymap: US position.
        assert_eq!(key_code(Key::new(0, 0x41), KeyVia::Vk, None), Some(30));
        // F13 has a scan code and an evdev code.
        let f13 = keys::by_code("F13").unwrap();
        assert_eq!(key_code(Key::new(f13.scan, f13.vk), KeyVia::Scan, None), Some(f13.evdev));
    }
}
