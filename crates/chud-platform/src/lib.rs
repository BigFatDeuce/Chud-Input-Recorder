//! # chud-platform
//!
//! Operating-system layer: global input capture, input injection, high
//! precision timing and system queries (monitors, windows, pixels).
//!
//! Everything platform specific lives behind the [`Platform`] and
//! [`Injector`] traits. Windows and Linux are implemented; other systems get
//! an "unsupported" backend so the rest of the application still builds.

use std::time::Instant;

use chud_core::document::{Environment, Monitor, Rect};
use chud_core::event::{EventKind, Key};
use chud_core::playback::Action;
use chud_core::vision::{Region, Screenshot, Word};
use serde::Serialize;

pub mod hotkey;
pub mod timer;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::activate_own_window;
#[cfg(target_os = "linux")]
mod linux;

pub use hotkey::{Hotkey, HotkeyAction, Modifiers};

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("not supported on this platform: {0}")]
    Unsupported(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("system error: {0}")]
    System(String),
    #[error("input blocked by the system (the target window may run with higher privileges)")]
    Blocked,
    /// Gamepad actions were skipped: no virtual controller driver
    /// (ViGEmBus on Windows, writable /dev/uinput on Linux).
    #[error("no virtual gamepad driver available")]
    NoGamepadOutput,
}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// Foreground window description.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct WindowInfo {
    pub handle: u64,
    pub pid: u32,
    /// Executable file name, e.g. `notepad.exe`.
    pub process: String,
    pub title: String,
    pub class: String,
    pub rect: Option<Rect>,
}

/// Something observed by the capture layer.
#[derive(Debug, Clone)]
pub enum Captured {
    /// An input event (only forwarded while forwarding is enabled).
    Input { t: Instant, kind: EventKind, injected: bool },
    /// A registered global hotkey was pressed (physical keys only).
    Hotkey { t: Instant, action: HotkeyAction },
    /// The foreground window changed.
    Foreground { t: Instant, window: WindowInfo, excluded: bool },
    /// Physical (non-injected) user activity, used for "stop when I touch
    /// the mouse/keyboard". Rate limited.
    UserActivity { t: Instant },
    /// Non fatal problem worth showing to the user.
    Warning { t: Instant, message: String },
}

/// What the capture layer forwards. Hotkeys are always detected.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureOptions {
    /// Forward input events at all (recording or live view).
    pub forward: bool,
    pub keyboard: bool,
    pub mouse_buttons: bool,
    pub mouse_move: bool,
    pub mouse_raw: bool,
    pub wheel: bool,
    /// Gamepads (XInput on Windows, evdev on Linux).
    pub gamepad: bool,
    /// Record input injected by other software (on-screen keyboards…).
    /// Input injected by Chud itself is never recorded.
    pub include_injected: bool,
    /// Report physical activity (for abort-on-input during playback).
    pub watch_activity: bool,
    /// Executable names (case-insensitive) whose input is never captured.
    pub excluded_processes: Vec<String>,
    /// Drop input while Chud's own windows are in the foreground.
    pub exclude_self: bool,
    /// Diagnostics only: also capture input injected by Chud itself
    /// (loopback measurements).
    pub include_own: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            forward: false,
            keyboard: true,
            mouse_buttons: true,
            mouse_move: true,
            mouse_raw: true,
            wheel: true,
            gamepad: true,
            include_injected: true,
            watch_activity: false,
            excluded_processes: Vec::new(),
            exclude_self: true,
            include_own: false,
        }
    }
}

impl CaptureOptions {
    pub fn needs_mouse_hook(&self) -> bool {
        (self.forward && (self.mouse_buttons || self.mouse_move || self.wheel)) || self.watch_activity
    }
    pub fn needs_raw_mouse(&self) -> bool {
        self.forward && self.mouse_raw
    }
    pub fn needs_gamepad(&self) -> bool {
        (self.forward && self.gamepad) || self.watch_activity
    }
    pub fn is_excluded(&self, process: &str) -> bool {
        self.excluded_processes.iter().any(|p| p.eq_ignore_ascii_case(process))
    }
}

/// Result of a permission / capability check, shown in Diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: String,
    pub ok: bool,
    pub title: String,
    pub detail: String,
    /// Optional command that fixes the problem (shown, never run silently).
    pub fix: Option<String>,
    /// State-specific code the interface translates (`title`/`detail` are
    /// the English fallback), with the values it interpolates.
    pub code: String,
    pub vars: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Capabilities {
    pub os: String,
    pub absolute_pointer_capture: bool,
    pub raw_mouse_capture: bool,
    pub hotkey_swallow: bool,
    pub foreground_window: bool,
    pub pixel_read: bool,
    /// Screen capture for image and colour search.
    pub screen_capture: bool,
    /// Character recognition for text search.
    pub text_recognition: bool,
    pub gamepad_output: bool,
    pub session: String,
}

/// Sends actions to the system.
pub trait Injector: Send {
    /// Sends a batch of actions as atomically as the OS allows.
    fn send(&mut self, actions: &[Action]) -> Result<()>;
    /// Types Unicode text, one character per call to `send`.
    fn type_char(&mut self, c: char) -> Result<()>;
    /// Creates the virtual controllers `0..count` ahead of time, so that the
    /// first gamepad action is not delayed by the driver (and games have
    /// seen the controller before it is used).
    fn prepare_gamepads(&mut self, count: u8) -> Result<()> {
        let _ = count;
        Ok(())
    }
}

pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    /// Starts the capture thread. Captured items are sent to `sink`.
    fn start(&self, sink: crossbeam_channel::Sender<Captured>) -> Result<()>;
    fn set_capture(&self, options: CaptureOptions);
    fn set_hotkeys(&self, hotkeys: Vec<Hotkey>);
    /// A key event that Chud's own window received. Windows does not run our
    /// low-level keyboard hook while Chud's (WebView2) window has the focus,
    /// so the interface hands its key events over: they then drive global
    /// hotkeys and the live view like any other key. A transition the hook
    /// already saw is ignored.
    fn local_key(&self, key: Key, down: bool) {
        let _ = (key, down);
    }
    fn injector(&self) -> Result<Box<dyn Injector>>;
    fn monitors(&self) -> Vec<Monitor>;
    fn environment(&self) -> Environment;
    fn cursor(&self) -> Option<(i32, i32)>;
    fn set_cursor(&self, x: i32, y: i32) -> Result<()>;
    fn foreground(&self) -> Option<WindowInfo>;
    /// True when a visible top-level window matches (case-insensitive contains).
    fn window_exists(&self, title: &str, process: &str) -> bool;
    fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8)>;
    /// Copies part of the desktop (`None`: every screen). Physical pixels;
    /// the position is in virtual desktop coordinates.
    fn capture(&self, region: Option<Region>) -> Result<Screenshot> {
        let _ = region;
        Err(PlatformError::Unsupported("screen capture".into()))
    }
    /// Reads the words of a capture (positions in screen pixels).
    fn read_text(&self, shot: &Screenshot) -> Result<Vec<Word>> {
        let _ = shot;
        Err(PlatformError::Unsupported("text recognition".into()))
    }
    fn key_held(&self, key: Key) -> bool;
    fn checks(&self) -> Vec<Check>;
    /// Prepares the current thread for time critical work (priority, timer resolution).
    fn boost_thread(&self) -> ThreadBoost;
}

/// Guard restoring thread scheduling settings when dropped.
pub struct ThreadBoost {
    restore: Option<Box<dyn FnOnce() + Send>>,
}

impl ThreadBoost {
    pub fn new(restore: impl FnOnce() + Send + 'static) -> Self {
        Self { restore: Some(Box::new(restore)) }
    }
    pub fn none() -> Self {
        Self { restore: None }
    }
}

impl Drop for ThreadBoost {
    fn drop(&mut self) {
        if let Some(f) = self.restore.take() {
            f();
        }
    }
}

/// Matches a window against "contains" filters (empty filter = any).
pub fn window_matches(w: &WindowInfo, title: &str, process: &str) -> bool {
    let contains = |hay: &str, needle: &str| needle.is_empty() || hay.to_lowercase().contains(&needle.to_lowercase());
    contains(&w.title, title) && contains(&w.process, process)
}

/// The platform backend for this system.
pub fn native() -> Box<dyn Platform> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(unsupported::Unsupported)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported {
    use super::*;
    pub struct Unsupported;
    impl Platform for Unsupported {
        fn capabilities(&self) -> Capabilities {
            Capabilities { os: std::env::consts::OS.into(), ..Default::default() }
        }
        fn start(&self, _: crossbeam_channel::Sender<Captured>) -> Result<()> {
            Err(PlatformError::Unsupported("input capture".into()))
        }
        fn set_capture(&self, _: CaptureOptions) {}
        fn set_hotkeys(&self, _: Vec<Hotkey>) {}
        fn injector(&self) -> Result<Box<dyn Injector>> {
            Err(PlatformError::Unsupported("input injection".into()))
        }
        fn monitors(&self) -> Vec<Monitor> {
            vec![]
        }
        fn environment(&self) -> Environment {
            Environment { os: std::env::consts::OS.into(), ..Default::default() }
        }
        fn cursor(&self) -> Option<(i32, i32)> {
            None
        }
        fn set_cursor(&self, _: i32, _: i32) -> Result<()> {
            Err(PlatformError::Unsupported("cursor".into()))
        }
        fn foreground(&self) -> Option<WindowInfo> {
            None
        }
        fn window_exists(&self, _: &str, _: &str) -> bool {
            false
        }
        fn pixel(&self, _: i32, _: i32) -> Option<(u8, u8, u8)> {
            None
        }
        fn key_held(&self, _: Key) -> bool {
            false
        }
        fn checks(&self) -> Vec<Check> {
            vec![]
        }
        fn boost_thread(&self) -> ThreadBoost {
            ThreadBoost::none()
        }
    }
}
