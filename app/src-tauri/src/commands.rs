//! Commands invoked by the interface.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chud_core::document::{MacroDoc, MacroMeta, Marker, Monitor};
use chud_core::event::{EventId, Micros};
use chud_core::flow::Flow;
use chud_core::format;
use chud_core::playback::{PlaybackSettings, Selection};
use chud_core::stats::{self, Stats};
use chud_core::transform::{self, Repetition};
use chud_engine::{EngineStatus, PlayJob, RecordOptions, Recording};
use chud_platform::timer::JitterStats;
use chud_platform::{Capabilities, Check};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::App;
use crate::docs::{self, DocPayload, EditOp, EditResult, OpenDoc};
use crate::library::{Entry, Library, Listing, TrashItem, Version};
use crate::settings::{self, AppSettings};

type R<T> = Result<T, String>;

// ───────────────────────────── application ─────────────────────────────

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub portable: bool,
    pub data_dir: String,
    pub settings_file: String,
    pub library_dir: String,
    pub logs_dir: String,
    pub capabilities: Capabilities,
    pub monitors: Vec<Monitor>,
}

#[tauri::command]
pub fn app_info(state: State<App>) -> AppInfo {
    let p = state.engine.platform();
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        portable: state.paths.portable,
        data_dir: state.paths.data_dir.to_string_lossy().into(),
        settings_file: state.paths.settings_file.to_string_lossy().into(),
        library_dir: state.library().root().to_string_lossy().into(),
        logs_dir: state.paths.logs_dir.to_string_lossy().into(),
        capabilities: p.capabilities(),
        monitors: p.monitors(),
    }
}

#[tauri::command]
pub fn checks(state: State<App>) -> Vec<Check> {
    state.engine.platform().checks()
}

#[tauri::command]
pub fn get_settings(state: State<App>) -> AppSettings {
    state.settings.read().clone()
}

#[tauri::command]
pub fn default_settings() -> AppSettings {
    AppSettings::default()
}

#[tauri::command]
pub fn set_settings(app: tauri::AppHandle, state: State<App>, settings: AppSettings) -> R<AppSettings> {
    let mut s = settings;
    s.ui_scale = if s.ui_scale.is_finite() { s.ui_scale.clamp(crate::MIN_UI_SCALE, crate::MAX_UI_SCALE) } else { 1.0 };
    s.playback.speed = s.playback.speed.clamp(0.01, 100.0);
    s.glow = s.glow.clamp(0.0, 1.0);
    s.autosave_delay_ms = s.autosave_delay_ms.clamp(200, 60_000);
    s.automation.sanitize();
    if !["auto", "fr", "en"].contains(&s.language.as_str()) {
        s.language = "auto".into();
    }
    let old = state.settings.read().clone();
    // Written by the backend after each update check: never rolled back by an
    // interface holding older settings.
    s.update_last_check_ms = s.update_last_check_ms.max(old.update_last_check_ms);
    if old.library_dir != s.library_dir {
        let dir = state.paths.library_for(&s);
        let lib = Library::new(dir.clone()).map_err(|e| format!("{}: {e}", dir.display()))?;
        *state.library.write() = Arc::new(lib);
    }
    if old.hotkeys != s.hotkeys {
        state.engine.set_hotkeys(s.hotkeys.clone());
    }
    if old.record != s.record {
        state.engine.set_record_defaults(s.record.clone());
    }
    if old.ui_scale != s.ui_scale {
        crate::apply_ui_scale(&app, s.ui_scale);
    }
    settings::save(&state.paths.settings_file, &s).map_err(|e| e.to_string())?;
    *state.settings.write() = s.clone();
    Ok(s)
}

#[derive(Serialize)]
pub struct KeyRow {
    pub scan: u16,
    pub evdev: u16,
    pub vk: u16,
    pub code: &'static str,
    pub label: &'static str,
    pub group: String,
}

#[tauri::command]
pub fn key_table() -> Vec<KeyRow> {
    chud_core::keys::KEYS
        .iter()
        .map(|k| KeyRow {
            scan: k.scan,
            evdev: k.evdev,
            vk: k.vk,
            code: k.code,
            label: k.label,
            group: format!("{:?}", k.group).to_lowercase(),
        })
        .collect()
}

#[derive(Serialize)]
pub struct TimerReport {
    pub fine: JitterStats,
    pub coarse: JitterStats,
}

/// Measures how precisely this machine wakes up on schedule (~2.5 s).
#[tauri::command]
pub async fn diag_timer(state: State<'_, App>) -> R<TimerReport> {
    let platform = state.engine.platform().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _boost = platform.boost_thread();
        let fine = chud_platform::timer::measure_sleep_jitter(1500, Duration::from_millis(1));
        let coarse = chud_platform::timer::measure_sleep_jitter(120, Duration::from_micros(7_300));
        TimerReport { fine, coarse }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn engine_status(state: State<App>) -> EngineStatus {
    state.engine.status()
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::quit(&app);
}

// ─────────────────────────────── library ───────────────────────────────

#[tauri::command]
pub fn library_list(state: State<App>) -> Listing {
    state.library().list()
}

#[tauri::command]
pub fn library_create_folder(state: State<App>, parent: String, name: String) -> R<String> {
    state.library().create_folder(&parent, &name)
}

#[tauri::command]
pub fn library_rename_folder(state: State<App>, folder: String, name: String) -> R<String> {
    let r = state.library().rename_folder(&folder, &name);
    refresh_open_paths(&state);
    r
}

#[tauri::command]
pub fn library_move_folder(state: State<App>, folder: String, into: String) -> R<String> {
    let r = state.library().move_folder(&folder, &into);
    refresh_open_paths(&state);
    r
}

#[tauri::command]
pub fn library_trash_folder(state: State<App>, folder: String) -> R<()> {
    let lib = state.library();
    // Open documents inside are closed first (their unsaved edits are saved).
    let prefix = lib.folder(&folder)?;
    let inside: Vec<String> =
        state.docs.lock().open.iter().filter(|(_, o)| o.path.starts_with(&prefix)).map(|(id, _)| id.clone()).collect();
    for id in inside {
        let _ = save_doc(&state, &id);
        state.docs.lock().open.remove(&id);
    }
    lib.trash_folder(&folder)
}

/// After folders move, open documents must follow their files.
fn refresh_open_paths(state: &App) {
    let lib = state.library();
    lib.list();
    for (id, o) in state.docs.lock().open.iter_mut() {
        if let Some(p) = lib.path_of(id) {
            o.path = p;
        }
    }
}

#[tauri::command]
pub fn library_move(state: State<App>, ids: Vec<String>, folder: String) -> R<()> {
    let lib = state.library();
    for id in ids {
        let p = lib.move_entry(&id, &folder)?;
        if let Some(o) = state.docs.lock().open.get_mut(&id) {
            o.path = p;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn library_trash(state: State<App>, ids: Vec<String>) -> R<()> {
    let lib = state.library();
    for id in ids {
        state.docs.lock().open.remove(&id);
        state.autosave.lock().remove(&id);
        lib.trash_entry(&id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn library_trash_list(state: State<App>) -> Vec<TrashItem> {
    state.library().trash()
}

#[tauri::command]
pub fn library_restore(state: State<App>, deleted_ms: i64, name: String) -> R<()> {
    state.library().restore(deleted_ms, &name)
}

/// Permanently deletes the trash content (the interface confirms first).
#[tauri::command]
pub fn library_empty_trash(state: State<App>) -> R<usize> {
    state.library().empty_trash()
}

fn current_doc(state: &App, id: &str) -> R<MacroDoc> {
    if let Some(o) = state.docs.lock().open.get(id) {
        return Ok(o.doc.clone());
    }
    state.library().load(id).map(|(d, _)| d)
}

#[tauri::command]
pub fn library_duplicate(state: State<App>, id: String, copy_suffix: String) -> R<String> {
    let lib = state.library();
    let mut doc = current_doc(&state, &id)?;
    let folder = lib
        .path_of(&id)
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| d.strip_prefix(lib.root()).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default())
        .unwrap_or_default();
    let fresh = MacroMeta::default();
    doc.meta.id = fresh.id;
    doc.meta.created_ms = fresh.created_ms;
    doc.meta.modified_ms = fresh.modified_ms;
    doc.meta.favorite = false;
    doc.meta.name = format!("{} {}", doc.meta.name, copy_suffix).trim().to_string();
    lib.save_new(&doc, &folder)?;
    Ok(doc.meta.id)
}

#[derive(Serialize)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub failed: Vec<(String, String)>,
}

fn read_any(path: &str) -> R<MacroDoc> {
    let p = PathBuf::from(path);
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "json" {
        let s = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        format::from_json(&s).map_err(|e| e.to_string())
    } else {
        format::load(&p).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn library_import(state: State<App>, paths: Vec<String>, folder: String) -> ImportReport {
    let lib = state.library();
    lib.list();
    let mut report = ImportReport { imported: Vec::new(), failed: Vec::new() };
    for path in paths {
        match read_any(&path) {
            Ok(mut doc) => {
                if doc.meta.id.is_empty() || lib.path_of(&doc.meta.id).is_some() {
                    doc.meta.id = MacroMeta::default().id;
                }
                if doc.meta.name.trim().is_empty() {
                    doc.meta.name = PathBuf::from(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                }
                match lib.save_new(&doc, &folder) {
                    Ok(_) => report.imported.push(doc.meta.id),
                    Err(e) => report.failed.push((path, e)),
                }
            }
            Err(e) => report.failed.push((path, e)),
        }
    }
    report
}

/// Exports the current state of a macro as `.chud` or `.json`.
#[tauri::command]
pub fn library_export(state: State<App>, id: String, dest: String) -> R<()> {
    let doc = current_doc(&state, &id)?;
    let p = PathBuf::from(&dest);
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        let s = format::to_json(&doc).map_err(|e| e.to_string())?;
        std::fs::write(&p, s).map_err(|e| e.to_string())
    } else {
        format::save(&doc, &p).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MetaPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub favorite: Option<bool>,
}

fn apply_patch(m: &mut MacroMeta, p: &MetaPatch) {
    if let Some(v) = &p.name {
        let v = v.trim();
        if !v.is_empty() {
            m.name = v.chars().take(200).collect();
        }
    }
    if let Some(v) = &p.description {
        m.description = v.clone();
    }
    if let Some(v) = &p.tags {
        let mut tags: Vec<String> = v.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        tags.dedup();
        m.tags = tags;
    }
    if let Some(v) = &p.icon {
        m.icon = v.clone();
    }
    if let Some(v) = &p.color
        && v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            m.color = v.clone();
        }
    if let Some(v) = p.favorite {
        m.favorite = v;
    }
}

/// Metadata changes are written at once, independently of timeline edits.
#[tauri::command]
pub fn library_set_meta(state: State<App>, id: String, patch: MetaPatch) -> R<MacroMeta> {
    let lib = state.library();
    let (mut on_disk, path) = lib.load(&id)?;
    apply_patch(&mut on_disk.meta, &patch);
    on_disk.touch();
    format::save(&on_disk, &path).map_err(|e| e.to_string())?;
    let mut final_path = path;
    if patch.name.is_some() {
        final_path = lib.rename_file(&id, &on_disk.meta.name)?;
    }
    if let Some(o) = state.docs.lock().open.get_mut(&id) {
        apply_patch(&mut o.doc.meta, &patch);
        o.doc.meta.modified_ms = on_disk.meta.modified_ms;
        o.path = final_path;
    }
    Ok(on_disk.meta)
}

#[tauri::command]
pub fn library_versions(state: State<App>, id: String) -> Vec<Version> {
    state.library().versions(&id)
}

/// Makes an older version current (the current one becomes a version).
#[tauri::command]
pub fn library_restore_version(state: State<App>, id: String, ms: i64) -> R<Option<DocPayload>> {
    let lib = state.library();
    let mut old = lib.load_version(&id, ms)?;
    old.meta.id = id.clone();
    old.touch();
    let path = lib.path_of(&id).ok_or("not found")?;
    let keep = state.settings.read().versions_kept.max(1);
    lib.save_over(&old, &path, keep)?;
    let mut docs = state.docs.lock();
    if let std::collections::hash_map::Entry::Occupied(mut e) = docs.open.entry(id) {
        let o = OpenDoc::new(old, path);
        let payload = DocPayload::of(&o);
        e.insert(o);
        return Ok(Some(payload));
    }
    Ok(None)
}

#[tauri::command]
pub fn library_absolute_path(state: State<App>, id: Option<String>) -> R<String> {
    let lib = state.library();
    match id {
        Some(id) => lib.path_of(&id).map(|p| p.to_string_lossy().to_string()).ok_or_else(|| "not found".into()),
        None => Ok(lib.root().to_string_lossy().to_string()),
    }
}

// ────────────────────────────── documents ──────────────────────────────

#[tauri::command]
pub fn doc_new(state: State<App>, name: String, folder: String) -> R<String> {
    let mut doc = MacroDoc::default();
    doc.meta.name = name;
    doc.env = state.engine.platform().environment();
    doc.settings = state.settings.read().playback.clone();
    state.library().save_new(&doc, &folder)?;
    Ok(doc.meta.id)
}

#[tauri::command]
pub fn doc_open(state: State<App>, id: String) -> R<DocPayload> {
    if let Some(o) = state.docs.lock().open.get(&id) {
        return Ok(DocPayload::of(o));
    }
    let (doc, path) = state.library().load(&id)?;
    let o = OpenDoc::new(doc, path);
    let payload = DocPayload::of(&o);
    state.docs.lock().open.insert(id, o);
    Ok(payload)
}

#[tauri::command]
pub fn doc_close(state: State<App>, id: String, save: bool) -> R<()> {
    if save {
        save_doc(&state, &id)?;
    }
    state.docs.lock().open.remove(&id);
    state.autosave.lock().remove(&id);
    Ok(())
}

fn schedule_autosave(state: &App, id: &str) {
    let s = state.settings.read();
    if s.autosave {
        let due = Instant::now() + Duration::from_millis(s.autosave_delay_ms);
        state.autosave.lock().insert(id.to_string(), due);
    }
}

#[tauri::command]
pub fn doc_edit(state: State<App>, id: String, op: EditOp) -> R<EditResult> {
    let mut docs = state.docs.lock();
    let o = docs.open.get_mut(&id).ok_or("document not open")?;
    let (change, created, report) = docs::compute(&o.doc.timeline, o.doc.original.as_ref(), &op)?;
    o.history.commit(&mut o.doc.timeline, op.label(), change.clone()).map_err(|e| e.to_string())?;
    let r = docs::result_of(o, &change, created, report);
    drop(docs);
    if !change.is_empty() {
        schedule_autosave(&state, &id);
    }
    Ok(r)
}

#[tauri::command]
pub fn doc_undo(state: State<App>, id: String) -> R<Option<EditResult>> {
    let mut docs = state.docs.lock();
    let o = docs.open.get_mut(&id).ok_or("document not open")?;
    let Some(change) = o.history.undo_change() else { return Ok(None) };
    o.history.undo(&mut o.doc.timeline).map_err(|e| e.to_string())?;
    let r = docs::result_of(o, &change, Vec::new(), None);
    drop(docs);
    schedule_autosave(&state, &id);
    Ok(Some(r))
}

#[tauri::command]
pub fn doc_redo(state: State<App>, id: String) -> R<Option<EditResult>> {
    let mut docs = state.docs.lock();
    let o = docs.open.get_mut(&id).ok_or("document not open")?;
    let Some(change) = o.history.redo_change() else { return Ok(None) };
    o.history.redo(&mut o.doc.timeline).map_err(|e| e.to_string())?;
    let r = docs::result_of(o, &change, Vec::new(), None);
    drop(docs);
    schedule_autosave(&state, &id);
    Ok(Some(r))
}

#[derive(Debug, Clone, Serialize)]
pub struct Saved {
    pub id: String,
    pub modified_ms: i64,
    pub history: chud_core::history::HistoryInfo,
}

/// Saves an open document if it has changes. `None` when nothing to do.
pub fn save_doc(state: &App, id: &str) -> R<Option<Saved>> {
    let lib = state.library();
    let keep = state.settings.read().versions_kept;
    let mut docs = state.docs.lock();
    let Some(o) = docs.open.get_mut(id) else { return Ok(None) };
    if !o.dirty() {
        return Ok(None);
    }
    o.doc.touch();
    lib.save_over(&o.doc, &o.path, keep)?;
    o.mark_saved();
    Ok(Some(Saved { id: id.to_string(), modified_ms: o.doc.meta.modified_ms, history: o.history.info() }))
}

#[tauri::command]
pub fn doc_save(state: State<App>, id: String) -> R<Option<Saved>> {
    state.autosave.lock().remove(&id);
    save_doc(&state, &id)
}

pub(crate) fn with_open<T>(state: &App, id: &str, f: impl FnOnce(&mut OpenDoc) -> T) -> R<T> {
    let mut docs = state.docs.lock();
    let o = docs.open.get_mut(id).ok_or("document not open")?;
    let r = f(o);
    o.extra_dirty = true;
    drop(docs);
    schedule_autosave(state, id);
    Ok(r)
}

#[tauri::command]
pub fn doc_set_settings(state: State<App>, id: String, settings: PlaybackSettings) -> R<()> {
    with_open(&state, &id, |o| o.doc.settings = settings)
}

#[tauri::command]
pub fn doc_set_markers(state: State<App>, id: String, markers: Vec<Marker>) -> R<()> {
    with_open(&state, &id, |o| o.doc.markers = markers)
}

#[tauri::command]
pub fn doc_set_flow(state: State<App>, id: String, flow: Option<Flow>) -> R<()> {
    with_open(&state, &id, |o| o.doc.flow = flow)
}

/// Copies the flow of macro `from` (with the images it searches for) into the
/// open macro `id`, replacing its flow. Returns the updated document.
#[tauri::command]
pub fn doc_import_flow(state: State<App>, id: String, from: String) -> R<DocPayload> {
    if id == from {
        return Err("a macro cannot copy its own flow".into());
    }
    let open_src = state.docs.lock().open.get(&from).map(|o| (o.doc.flow.clone(), o.doc.images.clone()));
    let (flow, images) = match open_src {
        Some(s) => s,
        None => {
            let (doc, _) = state.library().load(&from)?;
            (doc.flow, doc.images)
        }
    };
    let flow = flow.ok_or("this macro has no flow")?;
    with_open(&state, &id, |o| {
        o.doc.import_flow(&flow, &images);
        DocPayload::of(o)
    })
}

#[tauri::command]
pub fn doc_stats(state: State<App>, id: String) -> R<Stats> {
    let docs = state.docs.lock();
    let o = docs.open.get(&id).ok_or("document not open")?;
    Ok(stats::compute(&o.doc.timeline))
}

#[tauri::command]
pub fn doc_repetitions(state: State<App>, id: String, min_len: usize, max_results: usize) -> R<Vec<Repetition>> {
    let docs = state.docs.lock();
    let o = docs.open.get(&id).ok_or("document not open")?;
    Ok(transform::detect_repetitions(&o.doc.timeline, min_len, max_results.clamp(1, 50)))
}

/// Creates a new macro from some events of a document (sub-sequence).
#[tauri::command]
pub fn doc_extract(state: State<App>, id: String, ids: Vec<EventId>, name: String) -> R<String> {
    let (mut doc, folder) = {
        let docs = state.docs.lock();
        let o = docs.open.get(&id).ok_or("document not open")?;
        let mut picked: Vec<_> = o.doc.timeline.select(&chud_core::ops::with_partners(&o.doc.timeline, &ids));
        picked.sort_by(chud_core::event::Event::order);
        let t0 = picked.first().map(|e| e.t).unwrap_or(0);
        let items: Vec<(Micros, _)> = picked.iter().map(|e| (e.t - t0, e.kind)).collect();
        let mut doc = MacroDoc::from_recording(name, items, o.doc.env.clone());
        doc.settings = o.doc.settings.clone();
        doc.meta.color = o.doc.meta.color.clone();
        doc.meta.icon = o.doc.meta.icon.clone();
        let lib = state.library();
        let folder = o
            .path
            .parent()
            .and_then(|d| d.strip_prefix(lib.root()).ok())
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        (doc, folder)
    };
    doc.original = None;
    state.library().save_new(&doc, &folder)?;
    Ok(doc.meta.id)
}

/// Opens one of the app folders in the file manager (paths decided here,
/// so the interface needs no file-system scope).
#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<App>, which: String) -> R<()> {
    use tauri_plugin_opener::OpenerExt;
    let path = match which.as_str() {
        "data" => state.paths.data_dir.clone(),
        "logs" => state.paths.logs_dir.clone(),
        "library" => state.library().root().to_path_buf(),
        _ => return Err(format!("unknown folder: {which}")),
    };
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// Validates a flow expression; returns the error text, or `None` when valid.
#[tauri::command]
pub fn expr_check(src: String) -> Option<String> {
    chud_core::expr::parse(&src).err().map(|e| e.to_string())
}

// ─────────────────────────── record & playback ──────────────────────────

/// Turns a finished recording into a library macro.
pub fn save_recording(state: &App, rec: &Recording) -> R<Option<Entry>> {
    if rec.items.is_empty() {
        return Ok(None);
    }
    let now = chrono_like_now();
    let mut doc = MacroDoc::from_recording(format!("REC {now}"), rec.items.clone(), rec.env.clone());
    doc.contexts = rec.contexts.clone();
    doc.markers = rec.markers.clone();
    doc.settings = state.settings.read().playback.clone();
    let lib = state.library();
    let path = lib.save_new(&doc, "")?;
    let listing = lib.list();
    Ok(listing.entries.into_iter().find(|e| lib.root().join(&e.path) == path || e.id == doc.meta.id))
}

/// Local date and time as `YYYY-MM-DD HH-MM-SS` (file-name safe).
fn chrono_like_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string()
}

#[tauri::command]
pub fn record_start(state: State<App>, options: Option<RecordOptions>) {
    let opts = options.unwrap_or_else(|| state.settings.read().record.clone());
    state.engine.start_recording(opts);
}

#[tauri::command]
pub fn record_stop(state: State<App>) {
    state.engine.stop_recording();
}

#[tauri::command]
pub fn record_pause(state: State<App>) {
    state.engine.toggle_pause();
}

#[tauri::command]
pub fn record_marker(state: State<App>, label: String) {
    state.engine.add_marker(label);
}

pub(crate) fn job_for(state: &App, id: &str, settings: Option<PlaybackSettings>, selection: Selection, use_flow: bool) -> R<PlayJob> {
    let doc = current_doc(state, id)?;
    let settings = settings.unwrap_or_else(|| doc.settings.clone());
    Ok(PlayJob::of_doc(&doc, settings, selection, use_flow))
}

#[tauri::command]
pub fn play(
    app: AppHandle,
    state: State<App>,
    id: String,
    settings: Option<PlaybackSettings>,
    selection: Option<Selection>,
    use_flow: bool,
) -> R<()> {
    let job = job_for(&state, &id, settings, selection.unwrap_or_default(), use_flow)?;
    step_aside(&app, &state, false);
    state.engine.play(job);
    Ok(())
}

/// Minimizes Chud for a playback (restored when it ends). `if_focused`: only
/// when its window would receive the input.
pub(crate) fn step_aside(app: &AppHandle, state: &App, if_focused: bool) {
    if state.settings.read().minimize_on_play
        && let Some(w) = tauri::Manager::get_webview_window(app, "main")
        && (!if_focused || w.is_focused().unwrap_or(false))
        && w.minimize().is_ok()
    {
        state.minimized_for_play.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn macro_name(state: &App, id: &str) -> R<String> {
    current_doc(state, id).map(|d| d.meta.name)
}

#[tauri::command]
pub fn play_stop(state: State<App>) {
    state.engine.stop_playback();
}

#[tauri::command]
pub fn play_pause(state: State<App>) {
    state.engine.toggle_pause();
}

#[tauri::command]
pub fn emergency_stop(state: State<App>) {
    state.engine.emergency_stop();
}

/// Chooses what the "play" hotkey starts.
#[tauri::command]
pub fn arm(state: State<App>, id: Option<String>, use_flow: bool) -> R<()> {
    match id {
        Some(id) => {
            let job = job_for(&state, &id, None, Selection::All, use_flow)?;
            state.engine.arm(Some(job));
        }
        None => state.engine.arm(None),
    }
    Ok(())
}

#[tauri::command]
pub fn live_set(state: State<App>, on: bool) {
    state.engine.set_live(on);
}

/// A key pressed or released in Chud's own window (the system does not show
/// those to the global keyboard hook): drives global hotkeys and the live
/// view. Never recorded: recordings exclude Chud's window.
#[tauri::command]
pub fn local_key(state: State<App>, key: chud_core::event::Key, down: bool) {
    state.engine.local_key(key, down);
}

#[cfg(test)]
mod tests {
    #[test]
    fn date_format_shape() {
        let s = super::chrono_like_now();
        assert_eq!(s.len(), 19, "{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[13..14], "-");
    }
}
