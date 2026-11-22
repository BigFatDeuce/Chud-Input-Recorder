//! X11 helpers (also reachable through XWayland): pointer position, active
//! window, monitors, pixels, XTest injection and keyboard mapping.

use std::sync::Arc;

use chud_core::document::{Monitor, Rect};
use parking_lot::Mutex;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, ImageFormat, KEY_PRESS_EVENT,
    KEY_RELEASE_EVENT, Keycode, Keysym, MOTION_NOTIFY_EVENT, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::WindowInfo;

pub struct X {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    /// Serializes keymap changes used for Unicode typing.
    keymap_lock: Mutex<()>,
}

struct Atoms {
    active: Atom,
    pid: Atom,
    name: Atom,
    utf8: Atom,
    client_list: Atom,
    xkb_rules: Atom,
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

impl X {
    pub fn connect() -> Option<Arc<X>> {
        std::env::var_os("DISPLAY")?;
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atom = |n: &str| conn.intern_atom(false, n.as_bytes()).ok()?.reply().ok().map(|r| r.atom);
        let atoms = Atoms {
            active: atom("_NET_ACTIVE_WINDOW")?,
            pid: atom("_NET_WM_PID")?,
            name: atom("_NET_WM_NAME")?,
            utf8: atom("UTF8_STRING")?,
            client_list: atom("_NET_CLIENT_LIST")?,
            xkb_rules: atom("_XKB_RULES_NAMES")?,
        };
        Some(Arc::new(X { conn, root, atoms, keymap_lock: Mutex::new(()) }))
    }

    pub fn pointer(&self) -> Option<(i32, i32)> {
        let r = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((r.root_x as i32, r.root_y as i32))
    }

    pub fn warp(&self, x: i32, y: i32) -> bool {
        let ok = self.conn.warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, clamp16(x), clamp16(y)).is_ok();
        let _ = self.conn.flush();
        ok
    }

    pub fn has_xtest(&self) -> bool {
        self.conn
            .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
            .ok()
            .flatten()
            .is_some()
    }

    /// Queues an XTest motion (absolute, or relative when `relative`).
    /// Call [`X::flush`] after a batch: the server processes requests in order.
    pub fn fake_motion(&self, x: i32, y: i32, relative: bool) -> bool {
        let detail = u8::from(relative);
        self.conn
            .xtest_fake_input(MOTION_NOTIFY_EVENT, detail, 0, self.root, clamp16(x), clamp16(y), 0)
            .is_ok()
    }

    pub fn fake_button(&self, button: u8, down: bool) -> bool {
        let t = if down { BUTTON_PRESS_EVENT } else { BUTTON_RELEASE_EVENT };
        self.conn.xtest_fake_input(t, button, 0, self.root, 0, 0, 0).is_ok()
    }

    pub fn fake_key(&self, keycode: Keycode, down: bool) -> bool {
        let t = if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT };
        self.conn.xtest_fake_input(t, keycode, 0, self.root, 0, 0, 0).is_ok()
    }

    pub fn flush(&self) -> bool {
        self.conn.flush().is_ok()
    }

    /// Round trip: everything sent before has been processed by the server.
    pub fn sync(&self) {
        if let Ok(c) = self.conn.get_input_focus() {
            let _ = c.reply();
        }
    }

    pub fn monitors(&self) -> Vec<Monitor> {
        let Ok(c) = self.conn.randr_get_monitors(self.root, true) else { return vec![] };
        let Ok(r) = c.reply() else { return vec![] };
        r.monitors
            .iter()
            .map(|m| Monitor {
                name: self.atom_name(m.name).unwrap_or_default(),
                rect: Rect { x: m.x as i32, y: m.y as i32, width: m.width as i32, height: m.height as i32 },
                scale: 1.0,
                primary: m.primary,
                refresh_hz: None,
            })
            .collect()
    }

    fn atom_name(&self, a: Atom) -> Option<String> {
        let r = self.conn.get_atom_name(a).ok()?.reply().ok()?;
        Some(String::from_utf8_lossy(&r.name).to_string())
    }

    fn prop_u32(&self, w: Window, prop: Atom, ty: impl Into<Atom>) -> Vec<u32> {
        self.conn
            .get_property(false, w, prop, ty, 0, 4096)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    pub fn window(&self, w: Window) -> Option<WindowInfo> {
        if w == 0 {
            return None;
        }
        let pid = self.prop_u32(w, self.atoms.pid, AtomEnum::CARDINAL).first().copied().unwrap_or(0);
        let title = self
            .conn
            .get_property(false, w, self.atoms.name, self.atoms.utf8, 0, 1024)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).to_string())
            .unwrap_or_default();
        let class = self
            .conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).split('\0').nth(1).unwrap_or("").to_string())
            .unwrap_or_default();
        let rect = self.conn.get_geometry(w).ok().and_then(|c| c.reply().ok()).and_then(|g| {
            let t = self.conn.translate_coordinates(w, self.root, 0, 0).ok()?.reply().ok()?;
            Some(Rect { x: t.dst_x as i32, y: t.dst_y as i32, width: g.width as i32, height: g.height as i32 })
        });
        Some(WindowInfo { handle: w as u64, pid, process: super::process_name(pid), title, class, rect })
    }

    pub fn active_window_id(&self) -> Option<u32> {
        self.prop_u32(self.root, self.atoms.active, AtomEnum::WINDOW).first().copied().filter(|&w| w != 0)
    }

    pub fn active_window(&self) -> Option<WindowInfo> {
        self.window(self.active_window_id()?)
    }

    pub fn client_windows(&self) -> Vec<WindowInfo> {
        self.prop_u32(self.root, self.atoms.client_list, AtomEnum::WINDOW)
            .into_iter()
            .filter_map(|w| self.window(w))
            .collect()
    }

    /// Size of the whole X screen (every monitor).
    pub fn screen_size(&self) -> Option<(u32, u32)> {
        let g = self.conn.get_geometry(self.root).ok()?.reply().ok()?;
        Some((g.width as u32, g.height as u32))
    }

    /// Copies part of the screen, in strips (bounded reply sizes).
    pub fn capture(&self, x: i32, y: i32, w: u32, h: u32) -> Result<chud_core::vision::Image, String> {
        let mut px = Vec::with_capacity(w as usize * h as usize * 4);
        let strip = ((4 << 20) / (w.max(1) as usize * 4)).clamp(1, h.max(1) as usize) as u32;
        let mut row = 0;
        while row < h {
            let sh = strip.min(h - row);
            let r = self
                .conn
                .get_image(ImageFormat::Z_PIXMAP, self.root, clamp16(x), clamp16(y + row as i32), w as u16, sh as u16, !0)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            let need = w as usize * sh as usize * 4;
            if r.depth < 24 || r.data.len() < need {
                return Err(format!("unsupported screen format (depth {})", r.depth));
            }
            px.extend_from_slice(&r.data[..need]);
            row += sh;
        }
        Ok(chud_core::vision::Image::from_bgra(w, h, px))
    }

    pub fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        let r = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, clamp16(x), clamp16(y), 1, 1, !0)
            .ok()?
            .reply()
            .ok()?;
        // 24/32-bit TrueColor visuals: BGRX in memory.
        let d = r.data;
        (d.len() >= 3).then(|| (d[2], d[1], d[0]))
    }

    /// Keyboard layout from the XKB rules property, e.g. `fr` or `us(intl)`.
    pub fn xkb_layout(&self) -> Option<String> {
        let r = self
            .conn
            .get_property(false, self.root, self.atoms.xkb_rules, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        let parts: Vec<String> = r.value.split(|&b| b == 0).map(|p| String::from_utf8_lossy(p).to_string()).collect();
        let layout = parts.get(2).filter(|l| !l.is_empty())?;
        Some(match parts.get(3).filter(|v| !v.is_empty()) {
            Some(v) => format!("{layout}({v})"),
            None => layout.clone(),
        })
    }

    /// Snapshot of the current keyboard mapping.
    pub fn keymap(&self) -> Option<Keymap> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
        Some(Keymap { min, per: map.keysyms_per_keycode as usize, syms: map.keysyms })
    }

    /// Types one character, remapping a spare keycode when the layout has no
    /// key for it (the technique used by xdotool).
    pub fn type_char(&self, c: char) -> bool {
        let sym = keysym_for_char(c);
        let Some(map) = self.keymap() else { return false };
        if let Some((kc, shift)) = map.find(sym) {
            let shift_kc = map.find(0xffe1).map(|(k, _)| k).unwrap_or(50);
            if shift {
                self.fake_key(shift_kc, true);
            }
            self.fake_key(kc, true);
            self.fake_key(kc, false);
            if shift {
                self.fake_key(shift_kc, false);
            }
            self.sync();
            return true;
        }
        let _guard = self.keymap_lock.lock();
        let Some(spare) = map.spare() else { return false };
        let per = map.per as u8;
        if self.conn.change_keyboard_mapping(1, spare, per, &vec![sym; map.per]).is_err() {
            return false;
        }
        self.sync();
        self.fake_key(spare, true);
        self.fake_key(spare, false);
        self.sync();
        let _ = self.conn.change_keyboard_mapping(1, spare, per, &vec![0; map.per]);
        self.sync();
        true
    }
}

pub fn keysym_for_char(c: char) -> Keysym {
    let cp = c as u32;
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        _ if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) => cp,
        _ => 0x0100_0000 | cp,
    }
}

/// Keyboard mapping snapshot (keysyms per keycode).
#[derive(Debug, Clone)]
pub struct Keymap {
    pub min: u8,
    pub per: usize,
    pub syms: Vec<Keysym>,
}

fn letter_vk(sym: Keysym) -> Option<u16> {
    match sym {
        0x61..=0x7a => Some((sym - 0x20) as u16),
        0x41..=0x5a => Some(sym as u16),
        _ => None,
    }
}

fn digit_vk(sym: Keysym) -> Option<u16> {
    (0x30..=0x39).contains(&sym).then_some(sym as u16)
}

/// Windows `VK_OEM_*` code for punctuation keysyms.
fn oem_vk(sym: Keysym) -> Option<u16> {
    Some(match char::from_u32(sym)? {
        ';' => 0xBA,
        '=' | '+' => 0xBB,
        ',' => 0xBC,
        '-' => 0xBD,
        '.' => 0xBE,
        '/' => 0xBF,
        '`' => 0xC0,
        '[' => 0xDB,
        '\\' => 0xDC,
        ']' => 0xDD,
        '\'' => 0xDE,
        '<' => 0xE2,
        _ => return None,
    })
}

impl Keymap {
    fn levels(&self, keycode: u8) -> (Keysym, Keysym) {
        let i = keycode.wrapping_sub(self.min) as usize * self.per;
        let get = |j: usize| self.syms.get(i + j).copied().unwrap_or(0);
        (get(0), if self.per > 1 { get(1) } else { 0 })
    }

    /// `(keycode, needs_shift)` producing `sym` on the first two levels.
    pub fn find(&self, sym: Keysym) -> Option<(Keycode, bool)> {
        if self.per == 0 {
            return None;
        }
        for (i, syms) in self.syms.chunks(self.per).enumerate() {
            for (level, &s) in syms.iter().enumerate().take(2) {
                if s == sym {
                    return Some((self.min + i as u8, level == 1));
                }
            }
        }
        None
    }

    /// A keycode with no keysym at all, usable for temporary remapping.
    fn spare(&self) -> Option<Keycode> {
        if self.per == 0 {
            return None;
        }
        // Skip the lowest codes, which some servers reserve.
        self.syms
            .chunks(self.per)
            .enumerate()
            .skip(8)
            .find(|(_, s)| s.iter().all(|&k| k == 0))
            .map(|(i, _)| self.min + i as u8)
    }

    /// Logical (layout dependent) Windows-style virtual key for an evdev
    /// code, so that letters follow the layout as they do on Windows.
    /// `table_vk` is the US-layout code of that physical key.
    pub fn logical_vk(&self, evdev: u16, table_vk: u16) -> u16 {
        let Ok(kc) = u8::try_from(evdev + 8) else { return table_vk };
        let (l0, l1) = self.levels(kc);
        if l0 == 0 && l1 == 0 {
            return table_vk;
        }
        if let Some(vk) = letter_vk(l0).or_else(|| letter_vk(l1)) {
            return vk;
        }
        if let Some(vk) = digit_vk(l0).or_else(|| digit_vk(l1)) {
            return vk;
        }
        let alnum = (0x30..=0x39).contains(&table_vk) || (0x41..=0x5A).contains(&table_vk);
        if alnum || (0xBA..=0xE2).contains(&table_vk) {
            return oem_vk(l0).or_else(|| oem_vk(l1)).unwrap_or(if alnum { 0 } else { table_vk });
        }
        table_vk
    }

    /// evdev code of the key producing a letter or digit virtual key.
    pub fn evdev_for_vk(&self, vk: u16) -> Option<u16> {
        let wanted: &[Keysym] = match vk {
            0x41..=0x5A => &[vk as Keysym + 0x20, vk as Keysym],
            0x30..=0x39 => &[vk as Keysym],
            _ => return None,
        };
        let mut fallback = None;
        for (i, syms) in self.syms.chunks(self.per.max(1)).enumerate() {
            let kc = self.min as usize + i;
            if kc < 8 {
                continue;
            }
            if syms.first().is_some_and(|s| wanted.contains(s)) {
                return Some((kc - 8) as u16);
            }
            if fallback.is_none() && syms.get(1).is_some_and(|s| wanted.contains(s)) {
                fallback = Some((kc - 8) as u16);
            }
        }
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal AZERTY-like map.
    fn azerty() -> Keymap {
        let per = 2;
        let mut syms = vec![0; (255 - 8 + 1) * per];
        let mut set = |kc: usize, a: Keysym, b: Keysym| {
            syms[(kc - 8) * per] = a;
            syms[(kc - 8) * per + 1] = b;
        };
        set(24, 0x61, 0x41); // evdev 16 (US "Q") gives a
        set(10, 0x26, 0x31); // evdev 2 (US "1") gives & / 1
        set(58, 0x2c, 0x3f); // evdev 50 (US "M") gives ,
        set(47, 0x6d, 0x4d); // evdev 39 (US ";") gives m
        Keymap { min: 8, per, syms }
    }

    #[test]
    fn logical_vks_follow_the_layout() {
        let m = azerty();
        assert_eq!(m.logical_vk(16, 0x51), 0x41);
        assert_eq!(m.logical_vk(2, 0x31), 0x31);
        assert_eq!(m.logical_vk(50, 0x4D), 0xBC);
        assert_eq!(m.logical_vk(39, 0xBA), 0x4D);
        assert_eq!(m.logical_vk(1, 0x1B), 0x1B);
        assert_eq!(m.evdev_for_vk(0x41), Some(16));
        assert_eq!(m.evdev_for_vk(0x31), Some(2));
        assert_eq!(m.evdev_for_vk(0x4D), Some(39));
    }

    #[test]
    fn keysyms_for_chars() {
        assert_eq!(keysym_for_char('a'), 0x61);
        assert_eq!(keysym_for_char('é'), 0xe9);
        assert_eq!(keysym_for_char('€'), 0x0100_20ac);
        assert_eq!(keysym_for_char('\n'), 0xff0d);
    }
}
