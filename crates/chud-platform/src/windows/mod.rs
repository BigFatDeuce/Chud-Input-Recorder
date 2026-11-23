//! Windows backend.

mod capture;
pub mod gamepad;
mod inject;
mod screen;

use chud_core::document::{Environment, Monitor, Rect};
use chud_core::event::Key;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, GetPixel, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, SetThreadPriority, THREAD_PRIORITY_NORMAL,
    THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyboardLayoutNameW, MAPVK_VSC_TO_VK_EX, MapVirtualKeyW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, SPI_GETMOUSE, SPI_GETMOUSESPEED, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetCursorPos, SystemParametersInfoW,
};
use windows::core::{BOOL, PWSTR};

use crate::{Capabilities, CaptureOptions, Captured, Check, Hotkey, Injector, Platform, PlatformError, Result, ThreadBoost, WindowInfo};

/// Marker placed in `dwExtraInfo` of every input Chud injects, so that the
/// capture layer can recognize (and ignore) its own playback.
pub const CHUD_SIGNATURE: usize = 0x4348_5544;

pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn new() -> Self {
        unsafe {
            // Physical pixel coordinates everywhere. Fails harmlessly when the
            // host (e.g. Tauri's manifest) already set the awareness.
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        Self
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string()
    }
}

/// `(pid, executable name)` of a window, without sending it any message.
pub(crate) fn window_process(hwnd: HWND) -> (u32, String) {
    if hwnd.is_invalid() {
        return (0, String::new());
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    (pid, process_name(pid))
}

/// Gives the focus back to one of this application's windows, while the
/// application is already in front (a plain request to the system).
pub fn activate_own_window(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    let _ = unsafe { SetForegroundWindow(HWND(hwnd as *mut _)) };
}

pub(crate) fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    if hwnd.is_invalid() {
        return None;
    }
    let (pid, process) = window_process(hwnd);
    unsafe {
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        let mut class = [0u16; 256];
        let c = GetClassNameW(hwnd, &mut class);
        let mut r = RECT::default();
        let rect = GetWindowRect(hwnd, &mut r)
            .ok()
            .map(|_| Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top });
        Some(WindowInfo {
            handle: hwnd.0 as u64,
            pid,
            process,
            title: String::from_utf16_lossy(&title[..n.max(0) as usize]),
            class: String::from_utf16_lossy(&class[..c.max(0) as usize]),
            rect,
        })
    }
}

fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elev = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elev.TokenIsElevated != 0
    }
}

unsafe extern "system" fn monitor_enum(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
        let r = info.monitorInfo.rcMonitor;
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
        out.push(Monitor {
            name: wide_to_string(&info.szDevice),
            rect: Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top },
            scale: dx as f32 / 96.0,
            primary: info.monitorInfo.dwFlags & 1 != 0,
            refresh_hz: None,
        });
    }
    BOOL(1)
}

struct FindCtx<'a> {
    title: &'a str,
    process: &'a str,
    found: bool,
}

unsafe extern "system" fn find_enum(hwnd: HWND, data: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(data.0 as *mut FindCtx) };
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }
    if let Some(w) = window_info(hwnd)
        && !w.title.is_empty() && crate::window_matches(&w, ctx.title, ctx.process) {
            ctx.found = true;
            return BOOL(0);
        }
    BOOL(1)
}

impl Platform for WindowsPlatform {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            os: "windows".into(),
            absolute_pointer_capture: true,
            raw_mouse_capture: true,
            hotkey_swallow: true,
            foreground_window: true,
            pixel_read: true,
            screen_capture: true,
            text_recognition: screen::ocr_available(),
            gamepad_output: gamepad::output_available(),
            session: if is_elevated() { "administrator".into() } else { "user".into() },
        }
    }

    fn start(&self, sink: crossbeam_channel::Sender<Captured>) -> Result<()> {
        capture::start(sink)
    }

    fn set_capture(&self, options: CaptureOptions) {
        capture::set_options(options);
    }

    fn set_hotkeys(&self, hotkeys: Vec<Hotkey>) {
        capture::set_hotkeys(hotkeys);
    }

    fn local_key(&self, key: Key, down: bool) {
        capture::local_key(key, down);
    }

    fn injector(&self) -> Result<Box<dyn Injector>> {
        Ok(Box::new(inject::WinInjector::new()))
    }

    fn monitors(&self) -> Vec<Monitor> {
        let mut out: Vec<Monitor> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(None, None, Some(monitor_enum), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }

    fn environment(&self) -> Environment {
        let mut layout = [0u16; 9];
        let keyboard_layout = unsafe { GetKeyboardLayoutNameW(&mut layout) }
            .map(|_| wide_to_string(&layout))
            .unwrap_or_default();
        let mut speed: u32 = 0;
        let mut accel = [0i32; 3];
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETMOUSESPEED,
                0,
                Some(&mut speed as *mut _ as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let _ = SystemParametersInfoW(SPI_GETMOUSE, 0, Some(accel.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        }
        Environment {
            os: "windows".into(),
            monitors: self.monitors(),
            keyboard_layout,
            mouse_speed: (speed > 0).then_some(speed),
            mouse_acceleration: Some(accel[2] != 0),
            hostname_hash: None,
        }
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
    }

    fn set_cursor(&self, x: i32, y: i32) -> Result<()> {
        unsafe { SetCursorPos(x, y) }.map_err(|e| PlatformError::System(e.to_string()))
    }

    fn foreground(&self) -> Option<WindowInfo> {
        window_info(unsafe { GetForegroundWindow() })
    }

    fn window_exists(&self, title: &str, process: &str) -> bool {
        let mut ctx = FindCtx { title, process, found: false };
        unsafe {
            let _ = EnumWindows(Some(find_enum), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.found
    }

    fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        unsafe {
            let dc = GetDC(None);
            if dc.is_invalid() {
                return None;
            }
            let c = GetPixel(dc, x, y);
            ReleaseDC(None, dc);
            if c.0 == 0xFFFF_FFFF {
                return None; // CLR_INVALID: outside the desktop
            }
            Some(((c.0 & 0xFF) as u8, ((c.0 >> 8) & 0xFF) as u8, ((c.0 >> 16) & 0xFF) as u8))
        }
    }

    fn capture(&self, region: Option<chud_core::vision::Region>) -> Result<chud_core::vision::Screenshot> {
        screen::capture(region)
    }

    fn read_text(&self, shot: &chud_core::vision::Screenshot) -> Result<Vec<chud_core::vision::Word>> {
        screen::read_text(shot)
    }

    fn key_held(&self, key: Key) -> bool {
        let vk = if key.vk != 0 {
            key.vk as u32
        } else {
            let sc = (key.scan & 0xFF) as u32 | if key.is_extended() { 0xE000 } else { 0 };
            unsafe { MapVirtualKeyW(sc, MAPVK_VSC_TO_VK_EX) }
        };
        vk != 0 && unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0
    }

    fn checks(&self) -> Vec<Check> {
        let elevated = is_elevated();
        vec![
            Check {
                id: "hooks".into(),
                ok: true,
                title: "Global keyboard and mouse capture".into(),
                detail: "Low-level hooks and Raw Input need no special permission on Windows.".into(),
                fix: None,
                code: "hooks".into(),
                vars: Vec::new(),
            },
            Check {
                id: "uipi".into(),
                // Informational: running without elevation is the normal case.
                ok: true,
                title: if elevated { "Running as administrator".into() } else { "Running without administrator rights".into() },
                detail: if elevated {
                    "Windows lets Chud record and replay input in every window, including elevated ones.".into()
                } else {
                    "Windows (UIPI) prevents a normal program from observing or controlling windows that run as administrator (Task Manager, installers…). Restart Chud as administrator only if you need to automate those windows.".into()
                },
                fix: None,
                code: if elevated { "uipi_admin".into() } else { "uipi_user".into() },
                vars: Vec::new(),
            },
            {
                let pads = gamepad::physical_count();
                Check {
                    id: "xinput".into(),
                    ok: true,
                    title: "Gamepad capture (XInput)".into(),
                    detail: format!("{pads} controller(s) connected. XInput sees Xbox controllers and those presented as one (Steam Input, DS4Windows, 8BitDo X-input mode)."),
                    fix: None,
                    code: "xinput".into(),
                    vars: vec![("count".into(), pads.to_string())],
                }
            },
            {
                let ok = gamepad::output_available();
                Check {
                    id: "vigem".into(),
                    ok,
                    title: "Gamepad playback (ViGEmBus)".into(),
                    detail: if ok {
                        "The ViGEmBus driver is installed: gamepad events are replayed through virtual Xbox 360 controllers.".into()
                    } else {
                        "Install the ViGEmBus driver (github.com/nefarius/ViGEmBus/releases) to replay gamepad events. Keyboard and mouse playback do not need it.".into()
                    },
                    fix: None,
                    code: if ok { "vigem".into() } else { "vigem_missing".into() },
                    vars: Vec::new(),
                }
            },
        ]
    }

    fn boost_thread(&self) -> ThreadBoost {
        use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
        use windows::Win32::System::Threading::{AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW};
        unsafe {
            timeBeginPeriod(1);
            let mut task = 0u32;
            let mm = AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut task).ok();
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            let mm_raw = mm.map(|h| h.0 as usize);
            ThreadBoost::new(move || {
                if let Some(h) = mm_raw {
                    let _ = AvRevertMmThreadCharacteristics(HANDLE(h as *mut _));
                }
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL);
                timeEndPeriod(1);
            })
        }
    }
}
