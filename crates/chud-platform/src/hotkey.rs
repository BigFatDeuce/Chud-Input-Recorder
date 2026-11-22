//! Global hotkeys, matched inside the capture layer on physical key presses.

use chud_core::event::Key;
use chud_core::keys;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    ToggleRecord,
    TogglePause,
    TogglePlay,
    EmergencyStop,
    AddMarker,
    /// Plays a library macro (index into the engine's trigger table).
    Trigger(u32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn bits(self) -> u8 {
        (self.ctrl as u8) | (self.shift as u8) << 1 | (self.alt as u8) << 2 | (self.meta as u8) << 3
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub action: HotkeyAction,
    pub modifiers: Modifiers,
    /// Main key (identified by scan code, so the layout does not matter).
    pub key: Key,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Hotkey {
    pub fn new(action: HotkeyAction, modifiers: Modifiers, code: &str) -> Self {
        let info = keys::by_code(code).expect("known key code");
        Self { action, modifiers, key: Key::new(info.scan, info.vk), enabled: true }
    }

    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl".into());
        }
        if self.modifiers.shift {
            parts.push("Shift".into());
        }
        if self.modifiers.alt {
            parts.push("Alt".into());
        }
        if self.modifiers.meta {
            parts.push("Win".into());
        }
        parts.push(keys::label(self.key));
        parts.join("+")
    }

    pub fn defaults() -> Vec<Hotkey> {
        let none = Modifiers::default();
        let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Default::default() };
        vec![
            Hotkey::new(HotkeyAction::ToggleRecord, none, "F9"),
            Hotkey::new(HotkeyAction::TogglePause, none, "F10"),
            Hotkey::new(HotkeyAction::TogglePlay, none, "F8"),
            Hotkey::new(HotkeyAction::AddMarker, none, "F7"),
            Hotkey::new(HotkeyAction::EmergencyStop, ctrl_alt, "Escape"),
        ]
    }
}

/// Tracks physical modifier state and matches hotkeys. Shared by all
/// backends; fed with physical (non-injected) key transitions only.
#[derive(Debug, Default)]
pub struct Matcher {
    held: [bool; 8],
    /// Keys whose press was swallowed: their release must be swallowed too.
    swallowed: Vec<u32>,
}

const MOD_SCANS: [(u16, usize); 8] = [
    (0x001D, 0),
    (0xE01D, 1),
    (0x002A, 2),
    (0x0036, 3),
    (0x0038, 4),
    (0xE038, 5),
    (0xE05B, 6),
    (0xE05C, 7),
];

#[derive(Debug, PartialEq, Eq)]
pub enum MatchResult {
    /// Not a hotkey: deliver normally.
    Pass,
    /// Hotkey press: swallow and trigger.
    Trigger(HotkeyAction),
    /// Release of a swallowed hotkey: swallow silently.
    Swallow,
}

impl Matcher {
    pub fn modifiers(&self) -> Modifiers {
        Modifiers {
            ctrl: self.held[0] || self.held[1],
            shift: self.held[2] || self.held[3],
            alt: self.held[4] || self.held[5],
            meta: self.held[6] || self.held[7],
        }
    }

    /// Feeds one physical key transition. `repeat` presses of a swallowed
    /// key are swallowed too.
    pub fn feed(&mut self, key: Key, down: bool, hotkeys: &[Hotkey]) -> MatchResult {
        if let Some(&(_, idx)) = MOD_SCANS.iter().find(|(s, _)| *s == key.scan) {
            self.held[idx] = down;
        }
        let id = key.ident();
        if !down {
            if let Some(p) = self.swallowed.iter().position(|&s| s == id) {
                self.swallowed.swap_remove(p);
                return MatchResult::Swallow;
            }
            return MatchResult::Pass;
        }
        if self.swallowed.contains(&id) {
            return MatchResult::Swallow; // auto-repeat of a hotkey
        }
        let mods = self.modifiers();
        for hk in hotkeys.iter().filter(|h| h.enabled) {
            if hk.key.ident() == id && hk.modifiers == mods {
                self.swallowed.push(id);
                return MatchResult::Trigger(hk.action);
            }
        }
        MatchResult::Pass
    }

    /// Forget all state (after the capture thread restarts).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str) -> Key {
        let i = keys::by_code(code).unwrap();
        Key::new(i.scan, i.vk)
    }

    #[test]
    fn matches_with_exact_modifiers_and_swallows_release() {
        let hks = Hotkey::defaults();
        let mut m = Matcher::default();
        assert_eq!(m.feed(key("F9"), true, &hks), MatchResult::Trigger(HotkeyAction::ToggleRecord));
        assert_eq!(m.feed(key("F9"), true, &hks), MatchResult::Swallow); // repeat
        assert_eq!(m.feed(key("F9"), false, &hks), MatchResult::Swallow);
        // Ctrl+F9 is not F9.
        assert_eq!(m.feed(key("ControlLeft"), true, &hks), MatchResult::Pass);
        assert_eq!(m.feed(key("F9"), true, &hks), MatchResult::Pass);
        assert_eq!(m.feed(key("F9"), false, &hks), MatchResult::Pass);
        // Ctrl+Alt+Esc with right Alt.
        assert_eq!(m.feed(key("AltRight"), true, &hks), MatchResult::Pass);
        assert_eq!(m.feed(key("Escape"), true, &hks), MatchResult::Trigger(HotkeyAction::EmergencyStop));
        assert_eq!(m.feed(key("ControlLeft"), false, &hks), MatchResult::Pass);
        assert_eq!(m.feed(key("Escape"), false, &hks), MatchResult::Swallow);
    }

    #[test]
    fn labels() {
        assert_eq!(Hotkey::defaults()[4].label(), "Ctrl+Alt+Esc");
    }
}
