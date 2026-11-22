//! Windows capture thread: low-level keyboard/mouse hooks, Raw Input for
//! sensor deltas, and a WinEvent hook for foreground changes.
//!
//! Hook callbacks must be fast (Windows silently removes hooks that exceed
//! `LowLevelHooksTimeout`): they timestamp the event, filter it and push it
//! into a lock-free channel. Nothing else.

use std::cell::RefCell;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chud_core::event::{EventKind, Key, MouseButton};
use crossbeam_channel::Sender;
use parking_lot::RwLock;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK,
    RIDEV_REMOVE, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::{CHUD_SIGNATURE, window_info, window_process};
use crate::hotkey::{Hotkey, MatchResult, Matcher};
use crate::{CaptureOptions, Captured, PlatformError, Result};

const WM_APP_RECONFIGURE: u32 = WM_APP + 1;
/// A key event received by Chud's own window (see [`local_key`]):
/// `wparam` = scan | vk << 16, `lparam` = 1 for down.
const WM_APP_LOCAL_KEY: u32 = WM_APP + 2;

struct Global {
    sink: RwLock<Option<Sender<Captured>>>,
    opts: RwLock<CaptureOptions>,
    hotkeys: RwLock<Vec<Hotkey>>,
    thread_id: AtomicU32,
    started: AtomicBool,
    forward: AtomicBool,
    include_own: AtomicBool,
    /// Bumped on every option change. The capture thread applies new options
    /// a moment later (posted message); a per-window decision cached under
    /// the old ones in between (e.g. "Chud's own window is excluded" while
    /// recording) must not leak into the next mode.
    opts_gen: AtomicU64,
}

static G: LazyLock<Global> = LazyLock::new(|| Global {
    sink: RwLock::new(None),
    opts: RwLock::new(CaptureOptions::default()),
    hotkeys: RwLock::new(Hotkey::defaults()),
    thread_id: AtomicU32::new(0),
    started: AtomicBool::new(false),
    forward: AtomicBool::new(false),
    include_own: AtomicBool::new(false),
    opts_gen: AtomicU64::new(0),
});

/// State owned by the capture thread (hooks run on that thread).
struct HookState {
    matcher: Matcher,
    /// Physically held keys, indexed by `(scan & 0xFF) | ext << 8`.
    held: [bool; 512],
    fg_hwnd: usize,
    /// Option generation `fg_excluded` was computed under.
    fg_gen: u64,
    fg_excluded: bool,
    last_activity: Option<Instant>,
    kb_hook: Option<HHOOK>,
    mouse_hook: Option<HHOOK>,
    raw_registered: bool,
    msg_window: HWND,
    own_pid: u32,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

pub(super) fn emit(c: Captured) {
    if let Some(s) = G.sink.read().as_ref() {
        let _ = s.send(c);
    }
}

/// Reads the current capture options without cloning them.
pub(super) fn with_options<R>(f: impl FnOnce(&CaptureOptions) -> R) -> R {
    f(&G.opts.read())
}

pub fn set_options(o: CaptureOptions) {
    G.forward.store(o.forward, Ordering::Release);
    G.include_own.store(o.include_own, Ordering::Release);
    let pads = o.needs_gamepad() && G.started.load(Ordering::Acquire);
    *G.opts.write() = o;
    G.opts_gen.fetch_add(1, Ordering::AcqRel);
    super::gamepad::reconfigure(pads);
    let tid = G.thread_id.load(Ordering::Acquire);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_APP_RECONFIGURE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn set_hotkeys(h: Vec<Hotkey>) {
    *G.hotkeys.write() = h;
}

/// Hands a key event of Chud's own window to the capture thread.
///
/// While Chud's window has the focus, Windows does not call our low-level
/// keyboard hook (measured: the mouse hook and other processes' keyboard
/// hooks still run; the keystrokes go to the WebView2 child window). Without
/// this, global hotkeys (record, play, emergency stop) did nothing when
/// pressed in Chud and the live view showed no keys.
pub fn local_key(key: Key, down: bool) {
    let tid = G.thread_id.load(Ordering::Acquire);
    if tid != 0 {
        let packed = key.scan as usize | (key.vk as usize) << 16;
        unsafe {
            let _ = PostThreadMessageW(tid, WM_APP_LOCAL_KEY, WPARAM(packed), LPARAM(down as isize));
        }
    }
}

/// Runs on the capture thread: same treatment as a physical key in
/// [`kb_proc`], except that it cannot be swallowed (the window already has
/// it) and a transition the hook already saw is dropped.
fn handle_local_key(key: Key, down: bool) {
    let t = Instant::now();
    if key.scan == 0 {
        return;
    }
    STATE.with(|s| {
        let Ok(mut guard) = s.try_borrow_mut() else { return };
        let Some(st) = guard.as_mut() else { return };
        let ext = key.scan & 0xFF00 == 0xE000;
        let idx = ((key.scan & 0xFF) | if ext { 0x100 } else { 0 }) as usize;
        if st.held[idx] == down {
            return; // the hook saw it, or an auto-repeat
        }
        st.held[idx] = down;
        note_activity(st, t);
        let hotkeys = G.hotkeys.read();
        match st.matcher.feed(key, down, &hotkeys) {
            MatchResult::Trigger(action) => {
                drop(hotkeys);
                emit(Captured::Hotkey { t, action });
                return;
            }
            MatchResult::Swallow => return,
            MatchResult::Pass => {}
        }
        drop(hotkeys);
        if G.forward.load(Ordering::Acquire) && G.opts.read().keyboard && !foreground_excluded(st) {
            let kind = if down { EventKind::KeyDown { key, repeat: false } } else { EventKind::KeyUp { key } };
            emit(Captured::Input { t, kind, injected: false });
        }
    });
}

pub fn start(sink: Sender<Captured>) -> Result<()> {
    *G.sink.write() = Some(sink);
    if G.started.swap(true, Ordering::AcqRel) {
        return Ok(()); // already running: only the sink changed
    }
    let (tx, rx) = crossbeam_channel::bounded::<std::result::Result<(), String>>(1);
    std::thread::Builder::new()
        .name("chud-capture".into())
        .spawn(move || {
            if let Err(e) = run(&tx) {
                let _ = tx.send(Err(e));
            }
            G.started.store(false, Ordering::Release);
            G.thread_id.store(0, Ordering::Release);
        })
        .map_err(|e| PlatformError::System(e.to_string()))?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {
            super::gamepad::reconfigure(G.opts.read().needs_gamepad());
            Ok(())
        }
        Ok(Err(e)) => Err(PlatformError::System(e)),
        Err(_) => Err(PlatformError::System("capture thread did not start".into())),
    }
}

fn run(ready: &crossbeam_channel::Sender<std::result::Result<(), String>>) -> std::result::Result<(), String> {
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("ChudCaptureWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Chud capture"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| format!("CreateWindowEx: {e}"))?;

        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), Some(hinst.into()), 0)
            .map_err(|e| format!("keyboard hook: {e}"))?;
        let fg_hook: HWINEVENTHOOK = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        STATE.with(|s| {
            *s.borrow_mut() = Some(HookState {
                matcher: Matcher::default(),
                held: [false; 512],
                fg_hwnd: 0,
                fg_gen: u64::MAX,
                fg_excluded: false,
                last_activity: None,
                kb_hook: Some(kb),
                mouse_hook: None,
                raw_registered: false,
                msg_window: hwnd,
                own_pid: std::process::id(),
            })
        });
        G.thread_id.store(GetCurrentThreadId(), Ordering::Release);
        reconfigure();
        let _ = ready.send(Ok(()));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_APP_RECONFIGURE {
                reconfigure();
                continue;
            }
            if msg.hwnd.is_invalid() && msg.message == WM_APP_LOCAL_KEY {
                let key = Key::new(msg.wParam.0 as u16, (msg.wParam.0 >> 16) as u16);
                handle_local_key(key, msg.lParam.0 != 0);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !fg_hook.is_invalid() {
            let _ = UnhookWinEvent(fg_hook);
        }
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().take() {
                for h in [st.kb_hook, st.mouse_hook].into_iter().flatten() {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
        });
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

/// Installs or removes the mouse hook and raw input registration so that
/// nothing more than necessary observes the user's input.
fn reconfigure() {
    let opts = G.opts.read().clone();
    STATE.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(st) = guard.as_mut() else { return };
        st.fg_hwnd = 0; // exclusion rules may have changed
        unsafe {
            let want_mouse = opts.needs_mouse_hook();
            match (want_mouse, st.mouse_hook) {
                (true, None) => {
                    let hinst = GetModuleHandleW(None).ok();
                    match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hinst.map(Into::into), 0) {
                        Ok(h) => st.mouse_hook = Some(h),
                        Err(e) => emit(Captured::Warning { t: Instant::now(), message: format!("mouse hook failed: {e}") }),
                    }
                }
                (false, Some(h)) => {
                    let _ = UnhookWindowsHookEx(h);
                    st.mouse_hook = None;
                }
                _ => {}
            }
            let want_raw = opts.needs_raw_mouse();
            if want_raw != st.raw_registered {
                let dev = RAWINPUTDEVICE {
                    usUsagePage: 0x01,
                    usUsage: 0x02,
                    dwFlags: if want_raw { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
                    hwndTarget: if want_raw { st.msg_window } else { HWND::default() },
                };
                match RegisterRawInputDevices(&[dev], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
                    Ok(()) => st.raw_registered = want_raw,
                    Err(e) => emit(Captured::Warning { t: Instant::now(), message: format!("raw input: {e}") }),
                }
            }
        }
    });
}

/// Whether the current foreground window must not be captured.
fn foreground_excluded(st: &mut HookState) -> bool {
    let fg = unsafe { GetForegroundWindow() };
    let key = fg.0 as usize;
    let generation = G.opts_gen.load(Ordering::Acquire);
    if key != st.fg_hwnd || generation != st.fg_gen {
        st.fg_hwnd = key;
        st.fg_gen = generation;
        // Process lookup only: no window text, which could send messages and
        // re-enter the hook.
        let (pid, process) = window_process(fg);
        let opts = G.opts.read();
        st.fg_excluded = (opts.exclude_self && pid == st.own_pid) || opts.is_excluded(&process);
    }
    st.fg_excluded
}

fn note_activity(st: &mut HookState, t: Instant) {
    if st.last_activity.is_none_or(|l| t.duration_since(l) > Duration::from_millis(40)) {
        st.last_activity = Some(t);
        if G.opts.read().watch_activity {
            emit(Captured::UserActivity { t });
        }
    }
}

unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let t = Instant::now();
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let ours = k.dwExtraInfo == CHUD_SIGNATURE && !G.include_own.load(Ordering::Relaxed);
        let injected = (k.flags & LLKHF_INJECTED).0 != 0;
        let up = (k.flags & LLKHF_UP).0 != 0;
        let ext = (k.flags & LLKHF_EXTENDED).0 != 0;
        let vk = k.vkCode as u16;
        let mut scan = (k.scanCode as u16 & 0xFF) | if ext { 0xE000 } else { 0 };
        if vk == 0xA1 {
            scan = 0x0036; // right Shift is sometimes flagged extended
        }
        if k.scanCode == 0 {
            scan = 0;
        }
        let key = Key::new(scan, vk);
        let _ = wparam;
        // VK_GAMEPAD_* (0xC3–0xDA): keystrokes Windows synthesizes from
        // controllers for gamepad-aware windows. Gamepads are captured as
        // such; recording these too would replay every press twice.
        if (0xC3..=0xDA).contains(&vk) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }

        let swallow = STATE.with(|s| {
            // A hook can be re-entered while Windows waits on a sent message;
            // in that rare case the nested event is simply passed through.
            let Ok(mut guard) = s.try_borrow_mut() else { return false };
            let Some(st) = guard.as_mut() else { return false };
            if ours {
                return false;
            }
            let idx = ((scan & 0xFF) | if ext { 0x100 } else { 0 }) as usize;
            let mut repeat = false;
            if !injected {
                note_activity(st, t);
                let hotkeys = G.hotkeys.read();
                match st.matcher.feed(key, !up, &hotkeys) {
                    MatchResult::Trigger(action) => {
                        drop(hotkeys);
                        emit(Captured::Hotkey { t, action });
                        return true;
                    }
                    MatchResult::Swallow => return true,
                    MatchResult::Pass => {}
                }
            }
            if scan != 0 {
                repeat = !up && st.held[idx];
                st.held[idx] = !up;
            }
            if G.forward.load(Ordering::Acquire) {
                let opts = G.opts.read();
                let allowed = opts.keyboard && (!injected || opts.include_injected);
                drop(opts);
                if allowed && !foreground_excluded(st) {
                    let kind = if up { EventKind::KeyUp { key } } else { EventKind::KeyDown { key, repeat } };
                    emit(Captured::Input { t, kind, injected });
                }
            }
            false
        });
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let t = Instant::now();
        let m = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let ours = m.dwExtraInfo == CHUD_SIGNATURE && !G.include_own.load(Ordering::Relaxed);
        let injected = m.flags & LLMHF_INJECTED != 0;
        if !ours {
            let (x, y) = (m.pt.x, m.pt.y);
            let hi = (m.mouseData >> 16) as u16;
            let kind = match wparam.0 as u32 {
                WM_MOUSEMOVE => Some(EventKind::MouseMove { x, y }),
                WM_LBUTTONDOWN => Some(EventKind::MouseDown { button: MouseButton::Left, x, y }),
                WM_LBUTTONUP => Some(EventKind::MouseUp { button: MouseButton::Left, x, y }),
                WM_RBUTTONDOWN => Some(EventKind::MouseDown { button: MouseButton::Right, x, y }),
                WM_RBUTTONUP => Some(EventKind::MouseUp { button: MouseButton::Right, x, y }),
                WM_MBUTTONDOWN => Some(EventKind::MouseDown { button: MouseButton::Middle, x, y }),
                WM_MBUTTONUP => Some(EventKind::MouseUp { button: MouseButton::Middle, x, y }),
                WM_XBUTTONDOWN | WM_XBUTTONUP => {
                    let button = if hi == 2 { MouseButton::X2 } else { MouseButton::X1 };
                    Some(if wparam.0 as u32 == WM_XBUTTONDOWN {
                        EventKind::MouseDown { button, x, y }
                    } else {
                        EventKind::MouseUp { button, x, y }
                    })
                }
                WM_MOUSEWHEEL => Some(EventKind::Wheel { delta: hi as i16 as i32, horizontal: false, x, y }),
                WM_MOUSEHWHEEL => Some(EventKind::Wheel { delta: hi as i16 as i32, horizontal: true, x, y }),
                _ => None,
            };
            STATE.with(|s| {
                let Ok(mut guard) = s.try_borrow_mut() else { return };
                let Some(st) = guard.as_mut() else { return };
                if !injected {
                    note_activity(st, t);
                }
                let Some(kind) = kind else { return };
                if !G.forward.load(Ordering::Acquire) {
                    return;
                }
                let opts = G.opts.read();
                let wanted = match kind {
                    EventKind::MouseMove { .. } => opts.mouse_move,
                    EventKind::Wheel { .. } => opts.wheel,
                    _ => opts.mouse_buttons,
                } && (!injected || opts.include_injected);
                drop(opts);
                if wanted && !foreground_excluded(st) {
                    emit(Captured::Input { t, kind, injected });
                }
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_INPUT {
        let t = Instant::now();
        let mut raw = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let n = unsafe {
            GetRawInputData(
                HRAWINPUT(lparam.0 as *mut _),
                RID_INPUT,
                Some(&mut raw as *mut _ as *mut _),
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if n != u32::MAX && raw.header.dwType == RIM_TYPEMOUSE.0 && !raw.header.hDevice.is_invalid() {
            let mouse = unsafe { raw.data.mouse };
            // Absolute devices (tablets, remote desktop) are covered by the hook.
            let absolute = mouse.usFlags.0 & 0x01 != 0;
            if !absolute && (mouse.lLastX != 0 || mouse.lLastY != 0) && G.forward.load(Ordering::Acquire) {
                let excluded = STATE.with(|s| match s.try_borrow_mut() {
                    Ok(mut g) => g.as_mut().map(foreground_excluded).unwrap_or(true),
                    Err(_) => true,
                });
                if !excluded && G.opts.read().mouse_raw {
                    emit(Captured::Input {
                        t,
                        kind: EventKind::MouseRaw { dx: mouse.lLastX, dy: mouse.lLastY },
                        injected: false,
                    });
                }
            }
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || id_object != OBJID_WINDOW.0 || !G.forward.load(Ordering::Acquire) {
        return;
    }
    let t = Instant::now();
    if let Some(window) = window_info(hwnd) {
        let excluded = STATE.with(|s| match s.try_borrow_mut() {
            Ok(mut g) => g.as_mut().map(foreground_excluded).unwrap_or(false),
            Err(_) => false,
        });
        emit(Captured::Foreground { t, window, excluded });
    }
}
