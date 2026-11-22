//! Input injection through `SendInput`.

use chud_core::event::{Key, MouseButton};
use chud_core::playback::{Action, KeyVia};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::CHUD_SIGNATURE;
use crate::{Injector, PlatformError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualDesk {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn virtual_desk() -> VirtualDesk {
    unsafe {
        VirtualDesk {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        }
    }
}

/// Maps a pixel to the 0..=65535 normalized range used by absolute
/// `SendInput`. Windows converts back with `floor(n * size / 65536)`, so the
/// smallest `n` whose image is the target pixel is `ceil(p * 65536 / size)`.
pub fn normalize(p: i32, origin: i32, size: i32) -> i32 {
    let rel = (p - origin).clamp(0, size - 1) as i64;
    let n = (rel * 65536 + size as i64 - 1) / size as i64;
    n.clamp(0, 65535) as i32
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: CHUD_SIGNATURE },
        },
    }
}

fn keyboard(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: CHUD_SIGNATURE },
        },
    }
}

/// Keys that Windows does not reproduce reliably from a scan code.
fn needs_vk(scan: u16) -> bool {
    match scan {
        0x0045 | 0xE045 | 0xE037 => true, // Pause, NumLock, PrintScreen
        s if s >> 8 == 0xE0 => matches!(s & 0xFF, 0x10 | 0x19 | 0x20..=0x24 | 0x2E | 0x30 | 0x32 | 0x5E | 0x5F | 0x63 | 0x65..=0x6D),
        _ => false,
    }
}

fn extended_vk(vk: u16) -> bool {
    matches!(
        vk,
        0x21..=0x28 // PgUp, PgDn, End, Home, arrows
            | 0x2D | 0x2E // Insert, Delete
            | 0x5B | 0x5C | 0x5D // Win, Apps
            | 0x6F // Divide
            | 0x90 // NumLock
            | 0xA3 | 0xA5 // right Ctrl / Alt
            | 0xA6..=0xB7 // browser & media
            | 0x2C // PrintScreen
    )
}

fn key_input(key: Key, down: bool, via: KeyVia) -> Option<INPUT> {
    let up = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
    let ext_scan = key.is_extended();
    if via == KeyVia::Vk || key.scan == 0 || needs_vk(key.scan) || key.scan & 0xF000 == 0xF000 {
        let vk = if key.vk != 0 {
            key.vk
        } else {
            let sc = (key.scan & 0xFF) as u32 | if ext_scan { 0xE000 } else { 0 };
            unsafe { MapVirtualKeyW(sc, MAPVK_VSC_TO_VK_EX) as u16 }
        };
        if vk == 0 {
            return None;
        }
        let scan = if key.scan != 0 && key.scan & 0xF000 != 0xF000 {
            key.scan & 0xFF
        } else {
            unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 }
        };
        let ext = if ext_scan || extended_vk(vk) { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
        Some(keyboard(vk, scan, up | ext))
    } else {
        let ext = if ext_scan { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
        Some(keyboard(0, key.scan & 0xFF, KEYEVENTF_SCANCODE | up | ext))
    }
}

fn button_flags(b: MouseButton, down: bool) -> (MOUSE_EVENT_FLAGS, u32) {
    match (b, down) {
        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, d) => (if d { MOUSEEVENTF_XDOWN } else { MOUSEEVENTF_XUP }, 1),
        (MouseButton::X2, d) => (if d { MOUSEEVENTF_XDOWN } else { MOUSEEVENTF_XUP }, 2),
    }
}

pub fn build(actions: &[Action], vd: VirtualDesk, out: &mut Vec<INPUT>) {
    let abs = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let at = |x: i32, y: i32| (normalize(x, vd.x, vd.w), normalize(y, vd.y, vd.h));
    for a in actions {
        match *a {
            Action::MoveTo { x, y } => {
                let (nx, ny) = at(x, y);
                out.push(mouse(nx, ny, 0, abs));
            }
            Action::MoveBy { dx, dy } => out.push(mouse(dx, dy, 0, MOUSEEVENTF_MOVE)),
            Action::Button { button, down, at: pos } => {
                let (f, data) = button_flags(button, down);
                match pos {
                    Some((x, y)) => {
                        let (nx, ny) = at(x, y);
                        out.push(mouse(nx, ny, data, f | abs));
                    }
                    None => out.push(mouse(0, 0, data, f)),
                }
            }
            Action::Wheel { delta, horizontal, at: pos } => {
                let f = if horizontal { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL };
                match pos {
                    Some((x, y)) => {
                        let (nx, ny) = at(x, y);
                        out.push(mouse(nx, ny, delta as u32, f | abs));
                    }
                    None => out.push(mouse(0, 0, delta as u32, f)),
                }
            }
            Action::Key { key, down, via } => {
                if let Some(i) = key_input(key, down, via) {
                    out.push(i);
                }
            }
            // Gamepad output goes through the virtual controller driver.
            Action::PadButton { .. } | Action::PadAxis { .. } => {}
        }
    }
}

pub struct WinInjector {
    buf: Vec<INPUT>,
}

impl WinInjector {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(64) }
    }

    fn flush(&mut self) -> Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let sent = unsafe { SendInput(&self.buf, std::mem::size_of::<INPUT>() as i32) };
        let expected = self.buf.len();
        self.buf.clear();
        if sent as usize != expected {
            return Err(PlatformError::Blocked);
        }
        Ok(())
    }
}

impl Injector for WinInjector {
    fn send(&mut self, actions: &[Action]) -> Result<()> {
        self.buf.clear();
        build(actions, virtual_desk(), &mut self.buf);
        let input = self.flush();
        let pads = super::gamepad::send(actions);
        input.and(pads)
    }

    fn prepare_gamepads(&mut self, count: u8) -> Result<()> {
        super::gamepad::prepare(count)
    }

    fn type_char(&mut self, c: char) -> Result<()> {
        self.buf.clear();
        let mut units = [0u16; 2];
        for &u in c.encode_utf16(&mut units).iter() {
            self.buf.push(keyboard(0, u, KEYEVENTF_UNICODE));
            self.buf.push(keyboard(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        self.flush()
    }
}

impl Drop for WinInjector {
    /// Safety net: whatever happened during playback, virtual controllers
    /// are left at rest.
    fn drop(&mut self) {
        super::gamepad::neutral();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Windows does with a normalized coordinate.
    fn back(n: i32, origin: i32, size: i32) -> i32 {
        origin + ((n as i64 * size as i64) >> 16) as i32
    }

    #[test]
    fn normalization_hits_every_pixel() {
        for &(origin, size) in &[(0, 1920), (-864, 5248), (0, 2560), (-1920, 1920), (0, 3), (0, 65536)] {
            for p in (origin..origin + size).step_by(((size / 2000) as usize).max(1)) {
                assert_eq!(back(normalize(p, origin, size), origin, size), p, "origin {origin} size {size} p {p}");
            }
            let last = origin + size - 1;
            assert_eq!(back(normalize(last, origin, size), origin, size), last);
        }
    }

    #[test]
    fn keys_prefer_scan_codes() {
        let a = key_input(Key::new(0x1E, 0x41), true, KeyVia::Scan).unwrap();
        let ki = unsafe { a.Anonymous.ki };
        assert_eq!(ki.wScan, 0x1E);
        assert!(ki.dwFlags.contains(KEYEVENTF_SCANCODE));
        let up = key_input(Key::new(0xE048, 0x26), false, KeyVia::Scan).unwrap();
        let ki = unsafe { up.Anonymous.ki };
        assert!(ki.dwFlags.contains(KEYEVENTF_EXTENDEDKEY) && ki.dwFlags.contains(KEYEVENTF_KEYUP));
        // Pause always goes through its virtual key.
        let p = key_input(Key::new(0x0045, 0x13), true, KeyVia::Scan).unwrap();
        assert_eq!(unsafe { p.Anonymous.ki }.wVk, VIRTUAL_KEY(0x13));
    }
}
