//! The input event model shared by every part of Chud.
//!
//! Events are small `Copy` values (no heap allocation) so that timelines with
//! millions of entries stay compact and cache friendly.

use serde::{Deserialize, Serialize};

/// Microseconds since the start of a recording.
pub type Micros = i64;
/// Session-local identifier of an event. Never persisted: ids are reassigned
/// in timeline order when a document is loaded.
pub type EventId = u32;

pub const MICROS_PER_MS: Micros = 1_000;
pub const MICROS_PER_SEC: Micros = 1_000_000;

/// Standard wheel notch, as defined by Windows (`WHEEL_DELTA`). Linux wheel
/// clicks are converted to this unit (one click = 120) and high resolution
/// wheels report fractions of it.
pub const WHEEL_NOTCH: i32 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
    X1 = 3,
    X2 = 4,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] = [Self::Left, Self::Right, Self::Middle, Self::X1, Self::X2];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Middle => "Middle",
            Self::X1 => "X1",
            Self::X2 => "X2",
        }
    }
}

/// A physical keyboard key.
///
/// * `scan` is the PC/AT **set 1** make code, the de-facto platform neutral
///   identity of a physical key. Extended keys carry `0xE0` in the high byte
///   (right Ctrl = `0xE01D`); NumLock is `0xE045` and Pause `0x0045`, the
///   convention Windows hooks and browsers use. Linux evdev codes without a
///   PC equivalent are stored as `0xF000 | evdev_code`. `0` means unknown.
/// * `vk` is the Windows virtual-key code observed at record time (the
///   *logical* key under the active layout). `0` when unknown (for example a
///   recording made on Linux).
///
/// Physical identity (pairing presses with releases, tracking held keys) uses
/// `scan`, falling back to `vk` when the scan code is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Key {
    pub scan: u16,
    pub vk: u16,
}

impl Key {
    pub const fn new(scan: u16, vk: u16) -> Self {
        Self { scan, vk }
    }

    /// Stable identity of the physical key.
    pub fn ident(self) -> u32 {
        if self.scan != 0 {
            self.scan as u32
        } else {
            0x1_0000 | self.vk as u32
        }
    }

    pub fn is_extended(self) -> bool {
        self.scan & 0xFF00 == 0xE000
    }
}

/// Gamepad button, using the normalized SDL/Xbox layout for standard
/// controls and raw joystick indices (`RAW_BASE + n`) for devices such as
/// wheels, flight sticks or arcade panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PadButton(pub u8);

impl PadButton {
    pub const SOUTH: Self = Self(0); // A / Cross
    pub const EAST: Self = Self(1); // B / Circle
    pub const WEST: Self = Self(2); // X / Square
    pub const NORTH: Self = Self(3); // Y / Triangle
    pub const BACK: Self = Self(4);
    pub const GUIDE: Self = Self(5);
    pub const START: Self = Self(6);
    pub const LEFT_STICK: Self = Self(7);
    pub const RIGHT_STICK: Self = Self(8);
    pub const LEFT_SHOULDER: Self = Self(9);
    pub const RIGHT_SHOULDER: Self = Self(10);
    pub const DPAD_UP: Self = Self(11);
    pub const DPAD_DOWN: Self = Self(12);
    pub const DPAD_LEFT: Self = Self(13);
    pub const DPAD_RIGHT: Self = Self(14);
    pub const MISC1: Self = Self(15);
    pub const RIGHT_PADDLE1: Self = Self(16);
    pub const LEFT_PADDLE1: Self = Self(17);
    pub const RIGHT_PADDLE2: Self = Self(18);
    pub const LEFT_PADDLE2: Self = Self(19);
    pub const TOUCHPAD: Self = Self(20);
    pub const RAW_BASE: u8 = 64;

    pub fn name(self) -> String {
        const NAMES: [&str; 21] = [
            "A", "B", "X", "Y", "Back", "Guide", "Start", "LS", "RS", "LB", "RB", "Up", "Down",
            "Left", "Right", "Misc", "P1", "P2", "P3", "P4", "Touchpad",
        ];
        match NAMES.get(self.0 as usize) {
            Some(n) => (*n).to_string(),
            None if self.0 >= Self::RAW_BASE => format!("Btn{}", self.0 - Self::RAW_BASE + 1),
            None => format!("Btn#{}", self.0),
        }
    }
}

/// Gamepad axis. Sticks are `-32768..=32767` (Y grows downward like SDL),
/// triggers `0..=32767`. Raw joystick axes start at `RAW_BASE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PadAxis(pub u8);

impl PadAxis {
    pub const LEFT_X: Self = Self(0);
    pub const LEFT_Y: Self = Self(1);
    pub const RIGHT_X: Self = Self(2);
    pub const RIGHT_Y: Self = Self(3);
    pub const LEFT_TRIGGER: Self = Self(4);
    pub const RIGHT_TRIGGER: Self = Self(5);
    pub const RAW_BASE: u8 = 64;

    pub fn name(self) -> String {
        const NAMES: [&str; 6] = ["LX", "LY", "RX", "RY", "LT", "RT"];
        match NAMES.get(self.0 as usize) {
            Some(n) => (*n).to_string(),
            None if self.0 >= Self::RAW_BASE => format!("Axis{}", self.0 - Self::RAW_BASE + 1),
            None => format!("Axis#{}", self.0),
        }
    }

    pub fn is_trigger(self) -> bool {
        self == Self::LEFT_TRIGGER || self == Self::RIGHT_TRIGGER
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// Absolute cursor position in physical pixels of the virtual desktop.
    MouseMove { x: i32, y: i32 },
    /// Relative motion as reported by the sensor (before pointer ballistics).
    MouseRaw { dx: i32, dy: i32 },
    MouseDown { button: MouseButton, x: i32, y: i32 },
    MouseUp { button: MouseButton, x: i32, y: i32 },
    /// Wheel rotation in `WHEEL_NOTCH` units (positive = away from user / right).
    Wheel { delta: i32, horizontal: bool, x: i32, y: i32 },
    KeyDown {
        key: Key,
        /// True for typematic auto-repeat while the key is held.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        repeat: bool,
    },
    KeyUp { key: Key },
    PadButton { pad: u8, button: PadButton, down: bool },
    PadAxis { pad: u8, axis: PadAxis, value: i16 },
}

/// Coarse family of an event, used for lanes, filters and statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Keyboard,
    MouseButton,
    MouseMove,
    MouseRaw,
    Wheel,
    PadButton,
    PadAxis,
}

/// Identity of a "pressable" control, used to pair presses with releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    Key(u32),
    Mouse(MouseButton),
    Pad(u8, PadButton),
}

/// Whether an event presses or releases a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Down,
    Up,
}

impl EventKind {
    pub fn category(&self) -> Category {
        match self {
            Self::MouseMove { .. } => Category::MouseMove,
            Self::MouseRaw { .. } => Category::MouseRaw,
            Self::MouseDown { .. } | Self::MouseUp { .. } => Category::MouseButton,
            Self::Wheel { .. } => Category::Wheel,
            Self::KeyDown { .. } | Self::KeyUp { .. } => Category::Keyboard,
            Self::PadButton { .. } => Category::PadButton,
            Self::PadAxis { .. } => Category::PadAxis,
        }
    }

    /// The control this event presses or releases, if any.
    pub fn control(&self) -> Option<(Control, Edge)> {
        match *self {
            Self::KeyDown { key, .. } => Some((Control::Key(key.ident()), Edge::Down)),
            Self::KeyUp { key } => Some((Control::Key(key.ident()), Edge::Up)),
            Self::MouseDown { button, .. } => Some((Control::Mouse(button), Edge::Down)),
            Self::MouseUp { button, .. } => Some((Control::Mouse(button), Edge::Up)),
            Self::PadButton { pad, button, down } => {
                Some((Control::Pad(pad, button), if down { Edge::Down } else { Edge::Up }))
            }
            _ => None,
        }
    }

    pub fn is_repeat(&self) -> bool {
        matches!(self, Self::KeyDown { repeat: true, .. })
    }

    /// Absolute pointer position carried by the event, if any.
    pub fn position(&self) -> Option<(i32, i32)> {
        match *self {
            Self::MouseMove { x, y }
            | Self::MouseDown { x, y, .. }
            | Self::MouseUp { x, y, .. }
            | Self::Wheel { x, y, .. } => Some((x, y)),
            _ => None,
        }
    }

    /// Returns a copy with the absolute position replaced (no-op for events
    /// without a position).
    pub fn with_position(mut self, nx: i32, ny: i32) -> Self {
        match &mut self {
            Self::MouseMove { x, y }
            | Self::MouseDown { x, y, .. }
            | Self::MouseUp { x, y, .. }
            | Self::Wheel { x, y, .. } => {
                *x = nx;
                *y = ny;
            }
            _ => {}
        }
        self
    }

    /// The release event matching a press (used to close stuck controls).
    pub fn release_for(&self) -> Option<EventKind> {
        match *self {
            Self::KeyDown { key, .. } => Some(Self::KeyUp { key }),
            Self::MouseDown { button, x, y } => Some(Self::MouseUp { button, x, y }),
            Self::PadButton { pad, button, down: true } => {
                Some(Self::PadButton { pad, button, down: false })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub t: Micros,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    pub const fn new(id: EventId, t: Micros, kind: EventKind) -> Self {
        Self { id, t, kind }
    }

    /// Canonical timeline order: time, then id (capture order for ties).
    #[inline]
    pub fn order(a: &Event, b: &Event) -> std::cmp::Ordering {
        a.t.cmp(&b.t).then(a.id.cmp(&b.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_identity_prefers_scan() {
        assert_eq!(Key::new(0x1E, 0x41).ident(), 0x1E);
        assert_eq!(Key::new(0, 0x41).ident(), 0x1_0041);
        assert!(Key::new(0xE01D, 0xA3).is_extended());
    }

    #[test]
    fn control_pairs() {
        let k = Key::new(0x1E, 0x41);
        let d = EventKind::KeyDown { key: k, repeat: false };
        assert_eq!(d.control(), Some((Control::Key(0x1E), Edge::Down)));
        assert_eq!(d.release_for(), Some(EventKind::KeyUp { key: k }));
        assert!(EventKind::MouseMove { x: 1, y: 2 }.control().is_none());
    }

    #[test]
    fn json_shape_is_readable() {
        let e = Event::new(3, 1500, EventKind::MouseDown { button: MouseButton::Left, x: 10, y: -4 });
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, r#"{"id":3,"t":1500,"type":"mouse_down","button":"left","x":10,"y":-4}"#);
        let back: Event = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }
}
