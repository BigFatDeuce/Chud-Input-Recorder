//! Linux backend: evdev capture, XTest / uinput injection, X11 queries.

mod capture;
pub mod gamepad;
mod inject;
mod x11;

use std::sync::Arc;

use chud_core::document::{Environment, Monitor};
use chud_core::event::Key;
use chud_core::keys;

use crate::{Capabilities, CaptureOptions, Captured, Check, Hotkey, Injector, Platform, PlatformError, Result, ThreadBoost, WindowInfo};

pub struct LinuxPlatform {
    x: Option<Arc<x11::X>>,
    session: String,
    wayland: bool,
    uinput: inject::SharedUinput,
    pads: gamepad::SharedPads,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default().to_lowercase();
        let wayland = session == "wayland" || (session.is_empty() && std::env::var_os("WAYLAND_DISPLAY").is_some());
        let session = if session.is_empty() {
            if wayland {
                "wayland".into()
            } else if std::env::var_os("DISPLAY").is_some() {
                "x11".into()
            } else {
                "tty".into()
            }
        } else {
            session
        };
        Self { x: x11::X::connect(), session, wayland, uinput: Arc::default(), pads: gamepad::new_shared() }
    }

    fn x11_usable(&self) -> bool {
        self.x.is_some() && !self.wayland
    }

    fn backend(&self) -> inject::Backend {
        match std::env::var("CHUD_LINUX_INJECT").ok().as_deref() {
            Some("uinput") => inject::Backend::Uinput,
            Some("xtest") if self.x.is_some() => inject::Backend::XTest,
            _ if self.x11_usable() && self.x.as_ref().is_some_and(|x| x.has_xtest()) => inject::Backend::XTest,
            _ => inject::Backend::Uinput,
        }
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

/// Executable name of a process (`/proc/<pid>/exe`, else `comm`).
pub(crate) fn process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    if let Ok(p) = std::fs::read_link(format!("/proc/{pid}/exe"))
        && let Some(n) = p.file_name() {
            return n.to_string_lossy().trim_end_matches(" (deleted)").to_string();
        }
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// evdev code tracked in the held table for a key (mouse buttons use the
/// Windows virtual key numbers 1, 2, 4, 5, 6).
fn held_code(key: Key) -> Option<u16> {
    if key.scan == 0 {
        match key.vk {
            0x01 => return Some(0x110),
            0x02 => return Some(0x111),
            0x04 => return Some(0x112),
            0x05 => return Some(0x113),
            0x06 => return Some(0x114),
            _ => {}
        }
    }
    keys::evdev_for(key)
}

fn uinput_writable() -> std::result::Result<(), std::io::Error> {
    std::fs::OpenOptions::new().write(true).open("/dev/uinput").map(|_| ())
}

impl Platform for LinuxPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            os: "linux".into(),
            absolute_pointer_capture: self.x11_usable(),
            raw_mouse_capture: true,
            hotkey_swallow: false,
            foreground_window: self.x11_usable(),
            pixel_read: self.x11_usable(),
            screen_capture: self.x11_usable(),
            text_recognition: false,
            gamepad_output: uinput_writable().is_ok(),
            session: self.session.clone(),
        }
    }

    fn start(&self, sink: crossbeam_channel::Sender<Captured>) -> Result<()> {
        capture::start(sink, self.x.clone(), self.wayland)
    }

    fn set_capture(&self, options: CaptureOptions) {
        capture::set_options(options);
    }

    fn set_hotkeys(&self, hotkeys: Vec<Hotkey>) {
        capture::set_hotkeys(hotkeys);
    }

    fn injector(&self) -> Result<Box<dyn Injector>> {
        let backend = self.backend();
        if backend == inject::Backend::Uinput {
            inject::ensure_uinput(&self.uinput)?;
        }
        Ok(Box::new(inject::LinuxInjector::new(backend, self.x.clone(), self.uinput.clone(), self.pads.clone())))
    }

    fn monitors(&self) -> Vec<Monitor> {
        self.x.as_ref().map(|x| x.monitors()).unwrap_or_default()
    }

    fn capture(&self, region: Option<chud_core::vision::Region>) -> Result<chud_core::vision::Screenshot> {
        use chud_core::vision::{Region, Screenshot};
        let x = match &self.x {
            Some(x) if !self.wayland => x,
            _ if self.wayland => {
                return Err(PlatformError::Unsupported(
                    "screen capture in a Wayland session (only the compositor may read the screen; log in with an X11 session to search the screen)".into(),
                ));
            }
            _ => return Err(PlatformError::Unsupported("screen capture without a display (no X11 server could be reached)".into())),
        };
        let (w, h) = x.screen_size().ok_or_else(|| PlatformError::System("screen size unavailable".into()))?;
        let desk = Region { x: 0, y: 0, w, h };
        let r = match region {
            Some(r) => r.intersect(&desk).ok_or_else(|| PlatformError::System("the search area is outside every screen".into()))?,
            None => desk,
        };
        let image = x.capture(r.x, r.y, r.w, r.h).map_err(PlatformError::System)?;
        Ok(Screenshot { x: r.x, y: r.y, image })
    }

    fn environment(&self) -> Environment {
        Environment {
            os: "linux".into(),
            monitors: self.monitors(),
            keyboard_layout: self.x.as_ref().and_then(|x| x.xkb_layout()).unwrap_or_default(),
            mouse_speed: None,
            mouse_acceleration: None,
            hostname_hash: None,
        }
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        if !self.x11_usable() {
            return None;
        }
        self.x.as_ref()?.pointer()
    }

    fn set_cursor(&self, x: i32, y: i32) -> Result<()> {
        match &self.x {
            Some(conn) if self.x11_usable() && conn.warp(x, y) => Ok(()),
            _ => Err(PlatformError::Unsupported("moving the pointer outside X11".into())),
        }
    }

    fn foreground(&self) -> Option<WindowInfo> {
        if !self.x11_usable() {
            return None;
        }
        self.x.as_ref()?.active_window()
    }

    fn window_exists(&self, title: &str, process: &str) -> bool {
        self.x
            .as_ref()
            .is_some_and(|x| x.client_windows().iter().any(|w| crate::window_matches(w, title, process)))
    }

    fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        if !self.x11_usable() {
            return None;
        }
        self.x.as_ref()?.pixel(x, y)
    }

    fn key_held(&self, key: Key) -> bool {
        held_code(key)
            .and_then(|c| capture::HELD.get(c as usize))
            .is_some_and(|h| h.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn checks(&self) -> Vec<Check> {
        let mut out = Vec::new();

        // 1. Reading input devices.
        let (mut total, mut readable, mut keyboards, mut pads) = (0, 0, 0, 0);
        if let Ok(dir) = std::fs::read_dir("/dev/input") {
            for e in dir.flatten() {
                if !e.file_name().to_string_lossy().starts_with("event") {
                    continue;
                }
                total += 1;
                if let Ok(dev) = evdev::raw_stream::RawDevice::open(e.path()) {
                    readable += 1;
                    let keys = dev.supported_keys();
                    let has = |c: u16| keys.is_some_and(|k| k.contains(evdev::KeyCode(c)));
                    if has(30) {
                        keyboards += 1;
                    }
                    let abs = dev.supported_absolute_axes();
                    let own = dev.name().is_some_and(|n| n.starts_with(capture::OWN_DEVICE_PREFIX));
                    if !own && gamepad::is_gamepad(has, |c| abs.is_some_and(|a| a.contains(evdev::AbsoluteAxisCode(c)))) {
                        pads += 1;
                    }
                }
            }
        }
        out.push(Check {
            id: "evdev".into(),
            ok: keyboards > 0,
            title: "Reading keyboards and mice".into(),
            detail: if total == 0 {
                "No input device found in /dev/input.".into()
            } else {
                format!(
                    "{readable} of {total} input devices readable ({keyboards} keyboard(s)). Chud reads input directly from the kernel, which requires membership of the 'input' group. Note that any program you run can then read the keyboard too."
                )
            },
            fix: (keyboards == 0 && total > 0).then(|| "sudo usermod -aG input $USER   # then log out and back in".into()),
            code: if total == 0 { "evdev_none".into() } else { "evdev".into() },
            vars: vec![
                ("readable".into(), readable.to_string()),
                ("total".into(), total.to_string()),
                ("keyboards".into(), keyboards.to_string()),
            ],
        });

        out.push(Check {
            id: "evdev_pads".into(),
            ok: true,
            title: "Gamepad capture".into(),
            detail: format!(
                "{pads} readable controller(s). Every controller the kernel supports is captured (Xbox, PlayStation, Nintendo, generic joysticks)."
            ),
            fix: None,
            code: "evdev_pads".into(),
            vars: vec![("count".into(), pads.to_string())],
        });

        // 2. Injection.
        let backend = self.backend();
        let uinput = uinput_writable();
        let (ok, code, detail, fix) = match (backend, &uinput) {
            (inject::Backend::XTest, _) => (
                true,
                "inject_xtest",
                "Playback goes through the X server (XTest): ordered, exact absolute positioning, no special permission.".to_string(),
                None,
            ),
            (inject::Backend::Uinput, Ok(())) => (
                true,
                "inject_uinput",
                "Playback uses a virtual input device (/dev/uinput).".to_string(),
                None,
            ),
            (inject::Backend::Uinput, Err(e)) => (
                false,
                "inject_denied",
                format!("/dev/uinput is not writable ({e}): keyboard and mouse playback is unavailable."),
                Some(
                    "sudo modprobe uinput && echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' | sudo tee /etc/udev/rules.d/60-chud-uinput.rules && sudo udevadm control --reload && sudo udevadm trigger".into(),
                ),
            ),
        };
        let vars = match &uinput {
            Err(e) => vec![("error".to_string(), e.to_string())],
            Ok(()) => Vec::new(),
        };
        out.push(Check { id: "inject".into(), ok, title: "Keyboard and mouse playback".into(), detail, fix, code: code.into(), vars });

        // 3. Display server.
        let (ok, code, title, detail) = if self.x11_usable() {
            (
                true,
                "display_x11",
                "X11 session".to_string(),
                "Pointer positions, active window, application exclusions and pixel conditions are available.".to_string(),
            )
        } else if self.wayland {
            (
                false,
                "display_wayland",
                "Wayland session (limited)".to_string(),
                "Wayland does not let applications read the pointer position or the active window: Chud records relative mouse motion only, cannot apply application exclusions, and plays mouse moves in relative mode. Keyboard, buttons and wheel work normally. Log in with an X11 session for full features.".to_string(),
            )
        } else {
            (
                false,
                "display_none",
                "No graphical session".to_string(),
                "Running without X11 or Wayland: only keyboard, buttons, wheel and relative motion are available.".to_string(),
            )
        };
        out.push(Check { id: "display".into(), ok, title, detail, fix: None, code: code.into(), vars: Vec::new() });
        out
    }

    fn boost_thread(&self) -> ThreadBoost {
        // Minimal timer slack makes sleeps wake up on time; a higher priority
        // is attempted but needs CAP_SYS_NICE (or rtkit), so may be refused.
        unsafe {
            let slack = libc::prctl(libc::PR_GET_TIMERSLACK, 0, 0, 0, 0);
            libc::prctl(libc::PR_SET_TIMERSLACK, 1 as libc::c_ulong, 0, 0, 0);
            let tid = libc::gettid() as libc::id_t;
            *libc::__errno_location() = 0;
            let nice = libc::getpriority(libc::PRIO_PROCESS, tid);
            let boosted = libc::setpriority(libc::PRIO_PROCESS, tid, -10) == 0;
            ThreadBoost::new(move || {
                if slack > 0 {
                    libc::prctl(libc::PR_SET_TIMERSLACK, slack as libc::c_ulong, 0, 0, 0);
                }
                if boosted {
                    libc::setpriority(libc::PRIO_PROCESS, tid, nice);
                }
            })
        }
    }
}
