//! Physical key table: PC scan codes (Windows convention) ↔ Linux evdev
//! codes ↔ W3C `KeyboardEvent.code` names ↔ default Windows virtual keys.
//!
//! The scan code follows the values Windows reports in low-level hooks, which
//! is also the convention used by browsers: extended keys carry `0xE0` in the
//! high byte, NumLock is `0xE045` and Pause is `0x0045`.

use crate::event::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInfo {
    /// PC set-1 scan code, Windows convention.
    pub scan: u16,
    /// Linux `input-event-codes.h` key code.
    pub evdev: u16,
    /// Windows virtual key under a US layout (0 when there is none).
    pub vk: u16,
    /// W3C `KeyboardEvent.code` identifier.
    pub code: &'static str,
    /// Short human label.
    pub label: &'static str,
    pub group: KeyGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyGroup {
    Letter,
    Digit,
    Symbol,
    Whitespace,
    Modifier,
    Function,
    Navigation,
    Numpad,
    Media,
    System,
    International,
}

use KeyGroup::*;

macro_rules! k {
    ($scan:expr, $ev:expr, $vk:expr, $code:expr, $label:expr, $g:expr) => {
        KeyInfo { scan: $scan, evdev: $ev, vk: $vk, code: $code, label: $label, group: $g }
    };
}

pub static KEYS: &[KeyInfo] = &[
    k!(0x0001, 1, 0x1B, "Escape", "Esc", System),
    k!(0x0002, 2, 0x31, "Digit1", "1", Digit),
    k!(0x0003, 3, 0x32, "Digit2", "2", Digit),
    k!(0x0004, 4, 0x33, "Digit3", "3", Digit),
    k!(0x0005, 5, 0x34, "Digit4", "4", Digit),
    k!(0x0006, 6, 0x35, "Digit5", "5", Digit),
    k!(0x0007, 7, 0x36, "Digit6", "6", Digit),
    k!(0x0008, 8, 0x37, "Digit7", "7", Digit),
    k!(0x0009, 9, 0x38, "Digit8", "8", Digit),
    k!(0x000A, 10, 0x39, "Digit9", "9", Digit),
    k!(0x000B, 11, 0x30, "Digit0", "0", Digit),
    k!(0x000C, 12, 0xBD, "Minus", "-", Symbol),
    k!(0x000D, 13, 0xBB, "Equal", "=", Symbol),
    k!(0x000E, 14, 0x08, "Backspace", "Backspace", Whitespace),
    k!(0x000F, 15, 0x09, "Tab", "Tab", Whitespace),
    k!(0x0010, 16, 0x51, "KeyQ", "Q", Letter),
    k!(0x0011, 17, 0x57, "KeyW", "W", Letter),
    k!(0x0012, 18, 0x45, "KeyE", "E", Letter),
    k!(0x0013, 19, 0x52, "KeyR", "R", Letter),
    k!(0x0014, 20, 0x54, "KeyT", "T", Letter),
    k!(0x0015, 21, 0x59, "KeyY", "Y", Letter),
    k!(0x0016, 22, 0x55, "KeyU", "U", Letter),
    k!(0x0017, 23, 0x49, "KeyI", "I", Letter),
    k!(0x0018, 24, 0x4F, "KeyO", "O", Letter),
    k!(0x0019, 25, 0x50, "KeyP", "P", Letter),
    k!(0x001A, 26, 0xDB, "BracketLeft", "[", Symbol),
    k!(0x001B, 27, 0xDD, "BracketRight", "]", Symbol),
    k!(0x001C, 28, 0x0D, "Enter", "Enter", Whitespace),
    k!(0x001D, 29, 0xA2, "ControlLeft", "Ctrl", Modifier),
    k!(0x001E, 30, 0x41, "KeyA", "A", Letter),
    k!(0x001F, 31, 0x53, "KeyS", "S", Letter),
    k!(0x0020, 32, 0x44, "KeyD", "D", Letter),
    k!(0x0021, 33, 0x46, "KeyF", "F", Letter),
    k!(0x0022, 34, 0x47, "KeyG", "G", Letter),
    k!(0x0023, 35, 0x48, "KeyH", "H", Letter),
    k!(0x0024, 36, 0x4A, "KeyJ", "J", Letter),
    k!(0x0025, 37, 0x4B, "KeyK", "K", Letter),
    k!(0x0026, 38, 0x4C, "KeyL", "L", Letter),
    k!(0x0027, 39, 0xBA, "Semicolon", ";", Symbol),
    k!(0x0028, 40, 0xDE, "Quote", "'", Symbol),
    k!(0x0029, 41, 0xC0, "Backquote", "`", Symbol),
    k!(0x002A, 42, 0xA0, "ShiftLeft", "Shift", Modifier),
    k!(0x002B, 43, 0xDC, "Backslash", "\\", Symbol),
    k!(0x002C, 44, 0x5A, "KeyZ", "Z", Letter),
    k!(0x002D, 45, 0x58, "KeyX", "X", Letter),
    k!(0x002E, 46, 0x43, "KeyC", "C", Letter),
    k!(0x002F, 47, 0x56, "KeyV", "V", Letter),
    k!(0x0030, 48, 0x42, "KeyB", "B", Letter),
    k!(0x0031, 49, 0x4E, "KeyN", "N", Letter),
    k!(0x0032, 50, 0x4D, "KeyM", "M", Letter),
    k!(0x0033, 51, 0xBC, "Comma", ",", Symbol),
    k!(0x0034, 52, 0xBE, "Period", ".", Symbol),
    k!(0x0035, 53, 0xBF, "Slash", "/", Symbol),
    k!(0x0036, 54, 0xA1, "ShiftRight", "Shift R", Modifier),
    k!(0x0037, 55, 0x6A, "NumpadMultiply", "Num *", Numpad),
    k!(0x0038, 56, 0xA4, "AltLeft", "Alt", Modifier),
    k!(0x0039, 57, 0x20, "Space", "Space", Whitespace),
    k!(0x003A, 58, 0x14, "CapsLock", "Caps", Modifier),
    k!(0x003B, 59, 0x70, "F1", "F1", Function),
    k!(0x003C, 60, 0x71, "F2", "F2", Function),
    k!(0x003D, 61, 0x72, "F3", "F3", Function),
    k!(0x003E, 62, 0x73, "F4", "F4", Function),
    k!(0x003F, 63, 0x74, "F5", "F5", Function),
    k!(0x0040, 64, 0x75, "F6", "F6", Function),
    k!(0x0041, 65, 0x76, "F7", "F7", Function),
    k!(0x0042, 66, 0x77, "F8", "F8", Function),
    k!(0x0043, 67, 0x78, "F9", "F9", Function),
    k!(0x0044, 68, 0x79, "F10", "F10", Function),
    k!(0x0045, 119, 0x13, "Pause", "Pause", System),
    k!(0x0046, 70, 0x91, "ScrollLock", "ScrLk", System),
    k!(0x0047, 71, 0x67, "Numpad7", "Num 7", Numpad),
    k!(0x0048, 72, 0x68, "Numpad8", "Num 8", Numpad),
    k!(0x0049, 73, 0x69, "Numpad9", "Num 9", Numpad),
    k!(0x004A, 74, 0x6D, "NumpadSubtract", "Num -", Numpad),
    k!(0x004B, 75, 0x64, "Numpad4", "Num 4", Numpad),
    k!(0x004C, 76, 0x65, "Numpad5", "Num 5", Numpad),
    k!(0x004D, 77, 0x66, "Numpad6", "Num 6", Numpad),
    k!(0x004E, 78, 0x6B, "NumpadAdd", "Num +", Numpad),
    k!(0x004F, 79, 0x61, "Numpad1", "Num 1", Numpad),
    k!(0x0050, 80, 0x62, "Numpad2", "Num 2", Numpad),
    k!(0x0051, 81, 0x63, "Numpad3", "Num 3", Numpad),
    k!(0x0052, 82, 0x60, "Numpad0", "Num 0", Numpad),
    k!(0x0053, 83, 0x6E, "NumpadDecimal", "Num .", Numpad),
    k!(0x0056, 86, 0xE2, "IntlBackslash", "<>", International),
    k!(0x0057, 87, 0x7A, "F11", "F11", Function),
    k!(0x0058, 88, 0x7B, "F12", "F12", Function),
    k!(0x0059, 117, 0, "NumpadEqual", "Num =", Numpad),
    k!(0x0064, 183, 0x7C, "F13", "F13", Function),
    k!(0x0065, 184, 0x7D, "F14", "F14", Function),
    k!(0x0066, 185, 0x7E, "F15", "F15", Function),
    k!(0x0067, 186, 0x7F, "F16", "F16", Function),
    k!(0x0068, 187, 0x80, "F17", "F17", Function),
    k!(0x0069, 188, 0x81, "F18", "F18", Function),
    k!(0x006A, 189, 0x82, "F19", "F19", Function),
    k!(0x006B, 190, 0x83, "F20", "F20", Function),
    k!(0x006C, 191, 0x84, "F21", "F21", Function),
    k!(0x006D, 192, 0x85, "F22", "F22", Function),
    k!(0x006E, 193, 0x86, "F23", "F23", Function),
    k!(0x0070, 93, 0x15, "KanaMode", "Kana", International),
    k!(0x0073, 89, 0xC1, "IntlRo", "Ro", International),
    k!(0x0076, 194, 0x87, "F24", "F24", Function),
    k!(0x0079, 92, 0x1C, "Convert", "Henkan", International),
    k!(0x007B, 94, 0x1D, "NonConvert", "Muhenkan", International),
    k!(0x007D, 124, 0, "IntlYen", "Yen", International),
    k!(0x007E, 121, 0xC2, "NumpadComma", "Num ,", Numpad),
    k!(0xE010, 165, 0xB1, "MediaTrackPrevious", "Prev", Media),
    k!(0xE019, 163, 0xB0, "MediaTrackNext", "Next", Media),
    k!(0xE01C, 96, 0x0D, "NumpadEnter", "Num Enter", Numpad),
    k!(0xE01D, 97, 0xA3, "ControlRight", "Ctrl R", Modifier),
    k!(0xE020, 113, 0xAD, "AudioVolumeMute", "Mute", Media),
    k!(0xE021, 140, 0xB7, "LaunchApp2", "Calc", Media),
    k!(0xE022, 164, 0xB3, "MediaPlayPause", "Play/Pause", Media),
    k!(0xE024, 166, 0xB2, "MediaStop", "Stop", Media),
    k!(0xE02E, 114, 0xAE, "AudioVolumeDown", "Vol -", Media),
    k!(0xE030, 115, 0xAF, "AudioVolumeUp", "Vol +", Media),
    k!(0xE032, 172, 0xAC, "BrowserHome", "Home Page", Media),
    k!(0xE035, 98, 0x6F, "NumpadDivide", "Num /", Numpad),
    k!(0xE037, 99, 0x2C, "PrintScreen", "PrtSc", System),
    k!(0xE038, 100, 0xA5, "AltRight", "AltGr", Modifier),
    k!(0xE045, 69, 0x90, "NumLock", "NumLk", Numpad),
    k!(0xE047, 102, 0x24, "Home", "Home", Navigation),
    k!(0xE048, 103, 0x26, "ArrowUp", "↑", Navigation),
    k!(0xE049, 104, 0x21, "PageUp", "PgUp", Navigation),
    k!(0xE04B, 105, 0x25, "ArrowLeft", "←", Navigation),
    k!(0xE04D, 106, 0x27, "ArrowRight", "→", Navigation),
    k!(0xE04F, 107, 0x23, "End", "End", Navigation),
    k!(0xE050, 108, 0x28, "ArrowDown", "↓", Navigation),
    k!(0xE051, 109, 0x22, "PageDown", "PgDn", Navigation),
    k!(0xE052, 110, 0x2D, "Insert", "Ins", Navigation),
    k!(0xE053, 111, 0x2E, "Delete", "Del", Navigation),
    k!(0xE05B, 125, 0x5B, "MetaLeft", "Win", Modifier),
    k!(0xE05C, 126, 0x5C, "MetaRight", "Win R", Modifier),
    k!(0xE05D, 127, 0x5D, "ContextMenu", "Menu", System),
    k!(0xE05E, 116, 0, "Power", "Power", System),
    k!(0xE05F, 142, 0x5F, "Sleep", "Sleep", System),
    k!(0xE063, 143, 0, "WakeUp", "Wake", System),
    k!(0xE065, 217, 0xAA, "BrowserSearch", "Search", Media),
    k!(0xE066, 156, 0xAB, "BrowserFavorites", "Favorites", Media),
    k!(0xE067, 173, 0xA8, "BrowserRefresh", "Refresh", Media),
    k!(0xE068, 128, 0xA9, "BrowserStop", "Browser Stop", Media),
    k!(0xE069, 159, 0xA7, "BrowserForward", "Forward", Media),
    k!(0xE06A, 158, 0xA6, "BrowserBack", "Back", Media),
    k!(0xE06B, 157, 0xB6, "LaunchApp1", "My PC", Media),
    k!(0xE06C, 155, 0xB4, "LaunchMail", "Mail", Media),
    k!(0xE06D, 226, 0xB5, "MediaSelect", "Media", Media),
];

/// Prefix used to store Linux key codes that have no PC scan code.
pub const EVDEV_PRIVATE: u16 = 0xF000;

pub fn by_scan(scan: u16) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|k| k.scan == scan)
}

pub fn by_evdev(code: u16) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|k| k.evdev == code)
}

pub fn by_code(code: &str) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|k| k.code.eq_ignore_ascii_case(code))
}

/// First key producing `vk` under a US layout. Ambiguous VKs (Enter, which
/// exists on the main block and the numpad) resolve to the main block.
pub fn by_vk(vk: u16) -> Option<&'static KeyInfo> {
    if vk == 0 {
        return None;
    }
    // Generic modifier VKs map to their left variant.
    let vk = match vk {
        0x10 => 0xA0, // VK_SHIFT
        0x11 => 0xA2, // VK_CONTROL
        0x12 => 0xA4, // VK_MENU
        v => v,
    };
    KEYS.iter().find(|k| k.vk == vk)
}

/// Converts a Linux evdev key code to a [`Key`].
pub fn key_from_evdev(code: u16) -> Key {
    match by_evdev(code) {
        Some(info) => Key::new(info.scan, info.vk),
        None => Key::new(EVDEV_PRIVATE | (code & 0x0FFF), 0),
    }
}

/// Linux evdev key code for a [`Key`], if one is known.
pub fn evdev_for(key: Key) -> Option<u16> {
    if key.scan & 0xF000 == EVDEV_PRIVATE {
        return Some(key.scan & 0x0FFF);
    }
    if key.scan != 0
        && let Some(info) = by_scan(key.scan) {
            return Some(info.evdev);
        }
    by_vk(key.vk).map(|i| i.evdev)
}

/// Completes a key recorded with only one of its two codes.
pub fn normalize(key: Key) -> Key {
    match (key.scan, key.vk) {
        (0, vk) => by_vk(vk).map(|i| Key::new(i.scan, vk)).unwrap_or(key),
        (scan, 0) => by_scan(scan).map(|i| Key::new(scan, i.vk)).unwrap_or(key),
        _ => key,
    }
}

pub fn is_modifier(key: Key) -> bool {
    matches!(
        key.scan,
        0x001D | 0xE01D | 0x002A | 0x0036 | 0x0038 | 0xE038 | 0xE05B | 0xE05C
    ) || matches!(key.vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

/// Human label for a key. Letters and digits use the *logical* character
/// recorded (so an AZERTY `A` shows as `A`, not `Q`).
pub fn label(key: Key) -> String {
    let vk = key.vk;
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from(vk as u8).to_string();
    }
    if let Some(info) = by_scan(key.scan).or_else(|| by_vk(vk)) {
        return info.label.to_string();
    }
    if key.scan & 0xF000 == EVDEV_PRIVATE {
        return format!("Key {}", key.scan & 0x0FFF);
    }
    if vk != 0 {
        return format!("VK {vk:#04X}");
    }
    format!("SC {:#06X}", key.scan)
}

/// W3C code identifier for a key (`"KeyA"`, `"ShiftLeft"`…), if known.
pub fn code_name(key: Key) -> Option<&'static str> {
    by_scan(key.scan).or_else(|| by_vk(key.vk)).map(|i| i.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_has_unique_identities() {
        let mut scans = HashSet::new();
        let mut evs = HashSet::new();
        let mut codes = HashSet::new();
        for k in KEYS {
            assert!(scans.insert(k.scan), "duplicate scan {:#06X}", k.scan);
            assert!(evs.insert(k.evdev), "duplicate evdev {}", k.evdev);
            assert!(codes.insert(k.code), "duplicate code {}", k.code);
        }
    }

    #[test]
    fn evdev_roundtrip() {
        for k in KEYS {
            let key = key_from_evdev(k.evdev);
            assert_eq!(key.scan, k.scan);
            assert_eq!(evdev_for(key), Some(k.evdev));
        }
        let exotic = key_from_evdev(0x2FF);
        assert_eq!(exotic.scan, 0xF2FF);
        assert_eq!(evdev_for(exotic), Some(0x2FF));
    }

    #[test]
    fn labels_follow_logical_layout() {
        // AZERTY: the physical Q position (0x10) produces 'A'.
        assert_eq!(label(Key::new(0x10, 0x41)), "A");
        assert_eq!(label(Key::new(0xE048, 0x26)), "↑");
        assert_eq!(label(Key::new(0x1D, 0)), "Ctrl");
        assert_eq!(label(Key::new(0, 0x10)), "Shift");
    }

    #[test]
    fn normalize_fills_missing_codes() {
        assert_eq!(normalize(Key::new(0, 0x41)), Key::new(0x1E, 0x41));
        assert_eq!(normalize(Key::new(0xE048, 0)), Key::new(0xE048, 0x26));
        assert!(is_modifier(Key::new(0xE038, 0xA5)));
        assert!(!is_modifier(Key::new(0x1E, 0x41)));
    }
}
