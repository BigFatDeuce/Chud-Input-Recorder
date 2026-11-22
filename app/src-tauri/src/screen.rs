//! Screen tools behind "find on screen" blocks: a snapshot of every screen
//! to pick an image, an area or a colour from; images stored in macros; and
//! a live test of a search. Captures stay in memory, inside this process.

use std::time::{Duration, Instant};

use chud_core::document::Monitor;
use chud_core::flow::{Target, parse_color};
use chud_core::format::base64;
use chud_core::vision::{self, FindOptions, Found, Image, Region, Screenshot};
use serde::Serialize;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::App;
use crate::commands::with_open;

type R<T> = Result<T, String>;

/// Smallest and largest image kept for a search.
const MIN_SIDE: u32 = 4;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// An image stored in a macro, as the interface shows it.
#[derive(Debug, Clone, Serialize)]
pub struct DocImage {
    pub id: String,
    pub w: u32,
    pub h: u32,
    /// `data:image/png;base64,…`
    pub url: String,
}

fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::encode(png))
}

pub fn doc_image(id: &str, png: &[u8]) -> DocImage {
    let (w, h) = Image::from_png(png).map(|i| (i.w, i.h)).unwrap_or((0, 0));
    DocImage { id: id.to_string(), w, h, url: data_url(png) }
}

/// Hides Chud's window while `f` looks at the screen (so it does not
/// capture itself), then brings it back.
fn with_window_hidden<T>(app: &AppHandle, f: impl FnOnce() -> T) -> T {
    let main = app.get_webview_window("main").filter(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if let Some(w) = &main {
        let _ = w.hide();
        // Let the windows behind repaint.
        std::thread::sleep(Duration::from_millis(280));
    }
    let r = f();
    if let Some(w) = &main {
        let _ = w.show();
        let _ = w.set_focus();
    }
    r
}

#[derive(Serialize)]
pub struct SnapshotInfo {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub monitors: Vec<Monitor>,
    pub cursor: Option<(i32, i32)>,
}

/// Captures every screen (Chud hidden) and keeps the picture for the picker.
#[tauri::command]
pub async fn screen_snapshot(app: AppHandle, state: State<'_, App>) -> R<SnapshotInfo> {
    let platform = state.engine.platform().clone();
    let cursor = platform.cursor();
    let p = platform.clone();
    let shot = tauri::async_runtime::spawn_blocking(move || with_window_hidden(&app, || p.capture(None)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let info = SnapshotInfo { x: shot.x, y: shot.y, w: shot.image.w, h: shot.image.h, monitors: platform.monitors(), cursor };
    *state.snapshot.lock() = Some(shot);
    Ok(info)
}

fn snapshot_crop(state: &App, region: Region) -> R<Image> {
    let snap = state.snapshot.lock();
    let s = snap.as_ref().ok_or("no screen capture: take one first")?;
    Ok(s.image.crop(region.x - s.x, region.y - s.y, region.w, region.h))
}

/// Raw pixels of part of the snapshot: width and height (u32, little
/// endian) then RGBA rows. Binary, so a 4K screen is not sent as JSON.
#[tauri::command]
pub fn screen_snapshot_pixels(state: State<App>, region: Region) -> R<Response> {
    let img = snapshot_crop(&state, region)?;
    let mut out = Vec::with_capacity(8 + img.rgba.len());
    out.extend_from_slice(&img.w.to_le_bytes());
    out.extend_from_slice(&img.h.to_le_bytes());
    out.extend_from_slice(&img.rgba);
    Ok(Response::new(out))
}

/// Frees the snapshot (the picker closed).
#[tauri::command]
pub fn screen_snapshot_clear(state: State<App>) {
    *state.snapshot.lock() = None;
}

fn check_size(w: u32, h: u32) -> R<()> {
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(format!("the image must be at least {MIN_SIDE}×{MIN_SIDE} pixels"));
    }
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err("the image is too large".into());
    }
    Ok(())
}

/// Stores part of the snapshot in the macro as a searchable image.
#[tauri::command]
pub fn doc_add_image(state: State<App>, id: String, region: Region) -> R<DocImage> {
    let img = snapshot_crop(&state, region)?;
    check_size(img.w, img.h)?;
    let png = img.to_png();
    let image_id = with_open(&state, &id, |o| o.doc.add_image(png.clone()))?;
    Ok(doc_image(&image_id, &png))
}

/// Stores a picture file (PNG, JPEG, BMP, GIF, WebP) in the macro.
#[tauri::command]
pub fn doc_import_image(state: State<App>, id: String, path: String) -> R<DocImage> {
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err("the file is too large".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("unreadable picture: {e}"))?.to_rgba8();
    let (w, h) = decoded.dimensions();
    check_size(w, h)?;
    let png = Image::new(w, h, decoded.into_raw()).to_png();
    let image_id = with_open(&state, &id, |o| o.doc.add_image(png.clone()))?;
    Ok(doc_image(&image_id, &png))
}

#[tauri::command]
pub fn doc_remove_image(state: State<App>, id: String, image_id: String) -> R<()> {
    with_open(&state, &id, |o| {
        o.doc.images.remove(&image_id);
    })
}

#[derive(Serialize)]
pub struct TestResult {
    /// Where it was found (screen pixels), if it was.
    pub found: Option<Found>,
    /// The closest thing on screen, even below the threshold.
    pub best: Option<Found>,
    /// Search time (capture included).
    pub ms: f64,
    /// Picture of the found (or closest) place, with a margin.
    pub preview: Option<String>,
    /// Words read, for a text search.
    pub words: Option<usize>,
}

/// Runs a search now, Chud hidden, exactly as a flow would.
#[tauri::command]
pub async fn screen_test(app: AppHandle, state: State<'_, App>, id: String, target: Target, region: Option<Region>) -> R<TestResult> {
    let needle = match &target {
        Target::Image { image, .. } => {
            let docs = state.docs.lock();
            let o = docs.open.get(&id).ok_or("document not open")?;
            let png = o.doc.images.get(image).ok_or("this image is no longer in the macro")?;
            Some(Image::from_png(png).map_err(|e| e.to_string())?)
        }
        _ => None,
    };
    if let Target::Color { color, .. } = &target {
        parse_color(color).ok_or_else(|| format!("invalid colour {color}"))?;
    }
    let platform = state.engine.platform().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_window_hidden(&app, || -> R<TestResult> {
            let t0 = Instant::now();
            let shot: Screenshot = platform.capture(region).map_err(|e| e.to_string())?;
            let to_screen = |f: Found| f.offset(shot.x, shot.y);
            let (found, best, words) = match &target {
                Target::Image { threshold, any_scale, .. } => {
                    let opts = FindOptions { threshold: *threshold, any_scale: *any_scale, max_results: 1 };
                    let s = vision::find_image(&shot.image, needle.as_ref().expect("image loaded"), &opts);
                    (s.matches.first().copied().map(to_screen), s.best.map(to_screen), None)
                }
                Target::Color { color, tolerance } => {
                    let f = vision::find_color(&shot.image, parse_color(color).expect("checked"), *tolerance).map(to_screen);
                    (f, f, None)
                }
                Target::Text { text, match_case } => {
                    let ws = platform.read_text(&shot).map_err(|e| e.to_string())?;
                    let f = vision::find_text(&ws, text, *match_case);
                    (f, f, Some(ws.len()))
                }
            };
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let preview = found.or(best).map(|f| {
                let pad = 12;
                let crop = shot.image.crop(f.x - shot.x - pad, f.y - shot.y - pad, f.w + 2 * pad as u32, f.h + 2 * pad as u32);
                data_url(&crop.to_png())
            });
            Ok(TestResult { found, best, ms, preview, words })
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
