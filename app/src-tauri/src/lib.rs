//! Chud Input Recorder desktop application (Tauri 2).
//!
//! The Rust side owns everything real time and every piece of data: the
//! engine (capture, recording, playback), the library on disk and the open
//! documents with their undo history. The web interface renders state and
//! sends commands.

mod commands;
mod docs;
mod library;
mod logger;
mod placement;
mod screen;
mod settings;
mod update;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chud_core::document::MacroDoc;
use chud_engine::{Engine, EngineEvent, MacroResolver, Mode};
use parking_lot::{Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::docs::Docs;
use crate::library::Library;
use crate::settings::{AppSettings, Paths};

pub struct Inner {
    pub engine: Engine,
    pub paths: Paths,
    pub settings: RwLock<AppSettings>,
    pub library: Arc<RwLock<Arc<Library>>>,
    pub docs: Arc<Mutex<Docs>>,
    /// Documents waiting to be autosaved, with their due time.
    pub autosave: Mutex<HashMap<String, Instant>>,
    /// The main window was minimized for a playback and must come back.
    pub minimized_for_play: std::sync::atomic::AtomicBool,
    /// The main window was minimized for a recording and must come back.
    pub minimized_for_record: std::sync::atomic::AtomicBool,
    /// The last capture of every screen, while the picker is open. Kept in
    /// memory only.
    pub snapshot: Mutex<Option<chud_core::vision::Screenshot>>,
}

#[derive(Clone)]
pub struct App(pub Arc<Inner>);

impl std::ops::Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Inner {
    pub fn library(&self) -> Arc<Library> {
        self.library.read().clone()
    }
}

/// Resolves nested "play macro" blocks: the open (possibly unsaved) version
/// wins over the file on disk.
struct Resolver {
    library: Arc<RwLock<Arc<Library>>>,
    docs: Arc<Mutex<Docs>>,
}

impl MacroResolver for Resolver {
    fn load(&self, id: &str) -> Option<MacroDoc> {
        if let Some(o) = self.docs.lock().open.get(id) {
            return Some(o.doc.clone());
        }
        let lib = self.library.read().clone();
        lib.load(id).ok().map(|(d, _)| d)
    }
}

fn status_tooltip(mode: Mode, paused: bool) -> String {
    let state = match (mode, paused) {
        (Mode::Recording, true) | (Mode::Playing, true) => "⏸",
        (Mode::Recording, false) => "● REC",
        (Mode::Countdown, _) => "…",
        (Mode::Playing, false) => "▶",
        (Mode::Idle, _) => "",
    };
    if state.is_empty() { "Chud Input Recorder".into() } else { format!("Chud Input Recorder — {state}") }
}

fn place_overlay(app: &AppHandle, top: bool) {
    let Some(w) = app.get_webview_window("overlay") else { return };
    let monitor = w.primary_monitor().ok().flatten().or_else(|| w.current_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let size = w.outer_size().unwrap_or(tauri::PhysicalSize::new((260.0 * scale) as u32, (52.0 * scale) as u32));
        let x = m.position().x + (m.size().width as i32 - size.width as i32) / 2;
        let margin = (14.0 * scale) as i32;
        let y = if top { m.position().y + margin } else { m.position().y + m.size().height as i32 - size.height as i32 - margin * 4 };
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// Input made in Chud's own window is never recorded (so its buttons and
/// shortcuts never end up in a take). A recording started while Chud has the
/// focus, typically from its Record button, would therefore capture nothing
/// until the user switched windows: Chud steps aside instead, and comes back
/// when the recording ends (stop, emergency stop, or a cancelled countdown).
fn keep_out_of_recording(app: &AppHandle, state: &App, mode: Mode, from_idle: bool) {
    use std::sync::atomic::Ordering;
    if from_idle && matches!(mode, Mode::Recording | Mode::Countdown) {
        if state.settings.read().minimize_on_record
            && let Some(w) = app.get_webview_window("main")
            && w.is_visible().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
            && w.minimize().is_ok()
        {
            state.minimized_for_record.store(true, Ordering::Release);
        }
    } else if mode == Mode::Idle && state.minimized_for_record.swap(false, Ordering::AcqRel) {
        show_main(app);
    }
}

/// Relays engine events to the interface and reacts to the important ones.
fn forward_events(app: AppHandle, rx: crossbeam_channel::Receiver<EngineEvent>) {
    let mut last_mode = Mode::Idle;
    for ev in rx {
        let state = app.state::<App>();
        match &ev {
            EngineEvent::Status(s) => {
                let _ = app.emit("engine-status", s);
                if s.mode != last_mode {
                    let from_idle = last_mode == Mode::Idle;
                    last_mode = s.mode;
                    keep_out_of_recording(&app, &state, s.mode, from_idle);
                    let show_play = state.settings.read().overlay.show_when_playing;
                    let top = state.settings.read().overlay.position != "bottom";
                    let visible = matches!(s.mode, Mode::Recording | Mode::Countdown) || (s.mode == Mode::Playing && show_play);
                    if let Some(w) = app.get_webview_window("overlay") {
                        if visible {
                            place_overlay(&app, top);
                            let _ = w.show();
                            let _ = w.set_ignore_cursor_events(true);
                        } else {
                            let _ = w.hide();
                        }
                    }
                }
                if let Some(tray) = app.tray_by_id("main") {
                    let _ = tray.set_tooltip(Some(status_tooltip(s.mode, s.paused)));
                }
            }
            EngineEvent::Live { inputs } => {
                let _ = app.emit("engine-live", inputs);
            }
            EngineEvent::RecordingFinished { recording } => {
                match commands::save_recording(&state, recording) {
                    Ok(Some(entry)) => {
                        let _ = app.emit("recording-saved", entry);
                        let _ = app.emit("library-changed", ());
                    }
                    Ok(None) => {
                        let _ = app.emit(
                            "engine-event",
                            serde_json::json!({"type": "notice", "level": "info", "message": "recording_empty"}),
                        );
                    }
                    Err(e) => {
                        log::error!("saving recording failed: {e}");
                        let _ = app.emit(
                            "engine-event",
                            serde_json::json!({"type": "notice", "level": "error", "message": format!("save_failed: {e}")}),
                        );
                    }
                }
            }
            EngineEvent::PlaybackFinished { .. } => {
                if state.minimized_for_play.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    show_main(&app);
                }
                let _ = app.emit("engine-event", &ev);
            }
            EngineEvent::Hotkey { action: chud_platform::HotkeyAction::Trigger(slot) } => {
                // A global hotkey bound to a library macro: play it right away.
                let trigger = state.settings.read().triggers.iter().find(|t| t.slot == *slot).cloned();
                if let Some(tr) = trigger
                    && state.engine.status().mode == Mode::Idle {
                        match commands::job_for(&state, &tr.macro_id, None, chud_core::playback::Selection::All, tr.use_flow) {
                            Ok(job) => state.engine.play(job),
                            Err(e) => {
                                let _ = app.emit(
                                    "engine-event",
                                    serde_json::json!({"type": "notice", "level": "error", "message": format!("trigger: {e}")}),
                                );
                            }
                        }
                    }
                let _ = app.emit("engine-event", &ev);
            }
            _ => {
                let _ = app.emit("engine-event", &ev);
            }
        }
    }
}

/// Saves documents whose autosave delay elapsed.
fn autosave_loop(app: AppHandle) {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let state = app.state::<App>();
        let now = Instant::now();
        let due: Vec<String> = {
            let mut a = state.autosave.lock();
            let due: Vec<String> = a.iter().filter(|(_, t)| **t <= now).map(|(id, _)| id.clone()).collect();
            for id in &due {
                a.remove(id);
            }
            due
        };
        for id in due {
            match commands::save_doc(&state, &id) {
                Ok(Some(saved)) => {
                    let _ = app.emit("doc-saved", saved);
                }
                Ok(None) => {}
                Err(e) => log::warn!("autosave of {id} failed: {e}"),
            }
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Chud Input Recorder", true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", "● Record / Stop  (F9)", true, None::<&str>)?;
    let play = MenuItem::with_id(app, "play", "▶ Play / Stop  (F8)", true, None::<&str>)?;
    let emergency = MenuItem::with_id(app, "emergency", "■ Emergency stop  (Ctrl+Alt+Esc)", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &sep, &record, &play, &emergency, &sep2, &quit_item])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../../assets/brand/chud-input-icon/png/icon-32.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Chud Input Recorder")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let state = app.state::<App>();
            match event.id().as_ref() {
                "show" => show_main(app),
                "record" => {
                    let s = state.engine.status();
                    if matches!(s.mode, Mode::Recording | Mode::Countdown) {
                        state.engine.stop_recording();
                    } else if s.mode == Mode::Idle {
                        let opts = state.settings.read().record.clone();
                        state.engine.start_recording(opts);
                    }
                }
                "play" => {
                    let s = state.engine.status();
                    if s.mode == Mode::Playing {
                        state.engine.stop_playback();
                    } else {
                        let _ = app.emit("tray-play", ());
                    }
                }
                "emergency" => state.engine.emergency_stop(),
                "quit" => quit(app),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub const MIN_UI_SCALE: f64 = 0.7;
pub const MAX_UI_SCALE: f64 = 1.6;

/// Zooms the main window's interface (the user's "Interface size").
pub fn apply_ui_scale(app: &AppHandle, scale: f64) {
    if let Some(w) = app.get_webview_window("main")
        && let Err(e) = w.set_zoom(scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE)) {
            log::warn!("interface zoom unavailable: {e}");
        }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        placement::keep_reachable(&w, placement::Need::Whole);
    }
}

pub fn quit(app: &AppHandle) {
    shutdown(app);
    app.exit(0);
}

/// Stops everything cleanly before the process ends: releases what playback
/// holds, saves the open macros (when autosave is on), stops the engine.
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<App>();
    // Never leave anything held or running behind.
    state.engine.emergency_stop();
    let ids: Vec<String> = state.docs.lock().open.keys().cloned().collect();
    if state.settings.read().autosave {
        for id in ids {
            let _ = commands::save_doc(&state, &id);
        }
    }
    std::thread::sleep(Duration::from_millis(60));
    state.engine.shutdown();
}

pub fn run() {
    let paths = Paths::resolve();
    logger::init(&paths.logs_dir);
    update::clean_after_update();
    log::info!("Chud Input Recorder {} starting (portable: {})", env!("CARGO_PKG_VERSION"), paths.portable);
    let settings = settings::load(&paths.settings_file);

    let lib_dir = paths.library_for(&settings);
    let library = match Library::new(lib_dir.clone()) {
        Ok(l) => l,
        Err(e) => {
            log::error!("library folder {} unusable ({e}); falling back to the default", lib_dir.display());
            Library::new(paths.library_dir.clone()).expect("default library folder")
        }
    };
    let library = Arc::new(RwLock::new(Arc::new(library)));
    let docs: Arc<Mutex<Docs>> = Arc::default();

    let platform: Arc<dyn chud_platform::Platform> = Arc::from(chud_platform::native());
    let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
    let resolver = Arc::new(Resolver { library: library.clone(), docs: docs.clone() });
    let engine = match Engine::new(platform, resolver, ev_tx) {
        Ok(e) => e,
        Err(e) => {
            log::error!("engine failed to start: {e}");
            panic!("engine failed to start: {e}");
        }
    };
    engine.set_hotkeys(settings.hotkeys.clone());
    engine.set_record_defaults(settings.record.clone());

    let state = App(Arc::new(Inner {
        engine,
        paths,
        settings: RwLock::new(settings),
        library,
        docs,
        autosave: Mutex::default(),
        minimized_for_play: Default::default(),
        minimized_for_record: Default::default(),
        snapshot: Mutex::default(),
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .manage(update::Pending::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::Builder::new().name("chud-events".into()).spawn({
                let h = handle.clone();
                move || forward_events(h, ev_rx)
            })?;
            std::thread::Builder::new().name("chud-autosave".into()).spawn({
                let h = handle.clone();
                move || autosave_loop(h)
            })?;
            // The window starts hidden: it is sized to the screen first.
            if let Some(main) = app.get_webview_window("main") {
                placement::place_initial(&main);
                let _ = main.show();
            }
            let scale = handle.state::<App>().settings.read().ui_scale;
            if scale != 1.0 {
                apply_ui_scale(&handle, scale);
            }
            if let Err(e) = build_tray(&handle) {
                log::warn!("tray icon unavailable: {e}");
            }
            // Floating, click-through activity indicator.
            let overlay = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("index.html?view=overlay".into()))
                .title("Chud Input Recorder — indicator")
                .inner_size(260.0, 52.0)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .focused(false)
                .focusable(false)
                // Left out of screen captures: searches never see it.
                .content_protected(true)
                .visible(false)
                .build();
            if let Err(e) = overlay {
                log::warn!("indicator window unavailable: {e}");
            }
            update::check_in_background(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            // Frameless window: never let its title bar end up off screen.
            let need = match event {
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => Some(placement::Need::Whole),
                tauri::WindowEvent::Moved(_) => Some(placement::Need::Part),
                _ => None,
            };
            if let Some(need) = need {
                let app = window.app_handle().clone();
                let _ = window.run_on_main_thread(move || {
                    if let Some(w) = app.get_webview_window("main") {
                        placement::keep_reachable(&w, need);
                    }
                });
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let to_tray = app.state::<App>().settings.read().close_to_tray;
                api.prevent_close();
                if to_tray {
                    let _ = window.hide();
                } else {
                    quit(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::checks,
            commands::get_settings,
            commands::set_settings,
            commands::default_settings,
            commands::key_table,
            commands::diag_timer,
            commands::engine_status,
            commands::library_list,
            commands::library_create_folder,
            commands::library_rename_folder,
            commands::library_move_folder,
            commands::library_trash_folder,
            commands::library_move,
            commands::library_trash,
            commands::library_trash_list,
            commands::library_restore,
            commands::library_empty_trash,
            commands::library_duplicate,
            commands::library_import,
            commands::library_export,
            commands::library_set_meta,
            commands::library_versions,
            commands::library_restore_version,
            commands::library_absolute_path,
            commands::doc_new,
            commands::doc_open,
            commands::doc_close,
            commands::doc_edit,
            commands::doc_undo,
            commands::doc_redo,
            commands::doc_save,
            commands::doc_set_settings,
            commands::doc_set_markers,
            commands::doc_set_flow,
            commands::doc_import_flow,
            commands::doc_stats,
            commands::doc_repetitions,
            commands::doc_extract,
            commands::expr_check,
            commands::open_folder,
            commands::record_start,
            commands::record_stop,
            commands::record_pause,
            commands::record_marker,
            commands::local_key,
            update::update_check,
            update::update_install,
            update::update_mode,
            commands::play,
            commands::play_stop,
            commands::play_pause,
            commands::emergency_stop,
            commands::arm,
            commands::live_set,
            commands::quit_app,
            screen::screen_snapshot,
            screen::screen_snapshot_pixels,
            screen::screen_snapshot_clear,
            screen::doc_add_image,
            screen::doc_import_image,
            screen::doc_remove_image,
            screen::screen_test,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Chud Input Recorder");
}
