//! Updates from GitHub Releases, from inside the application.
//!
//! Nothing is sent: a check downloads the public `latest.json` of the latest
//! release and compares versions. Every download is checked against the
//! signing key built into Chud before it is used, so a tampered file is
//! refused.
//!
//! How an update is applied depends on how Chud was installed:
//! - **installed** (Windows installer, AppImage, .deb, .rpm): the updater
//!   plugin runs the new installer or replaces the AppImage;
//! - **portable** Windows executable: the new executable is downloaded next
//!   to the current one, swapped in, and started; the old one is removed by
//!   the new one once the old process has exited;
//! - anything else (a binary built by hand): the release page is opened.

use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::App;

pub const RELEASES_PAGE: &str = "https://github.com/BigFatDeuce/Chud-Input-Recorder/releases/latest";
/// `latest.json` key of the portable Windows executable.
const PORTABLE_TARGET: &str = "windows-x86_64-portable";
/// Command-line flag of a portable executable started by an update.
pub const UPDATED_FLAG: &str = "--updated";

type R<T> = Result<T, String>;

/// How this copy of Chud can be updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Installer,
    Portable,
    Manual,
}

pub fn mode() -> Mode {
    use tauri::utils::{config::BundleType, platform::bundle_type};
    match bundle_type() {
        Some(BundleType::Nsis | BundleType::Msi | BundleType::AppImage | BundleType::Deb | BundleType::Rpm) => Mode::Installer,
        None if cfg!(windows) => Mode::Portable,
        _ => Mode::Manual,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub mode: Mode,
    /// The newer version, when there is one.
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub page: String,
}

#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
struct Progress {
    done: u64,
    total: Option<u64>,
}

fn updater(app: &AppHandle) -> R<tauri_plugin_updater::Updater> {
    // Windows installer updates exit the process right after starting the
    // installer: release everything and save the open macros first.
    let mut b = app.updater_builder().timeout(Duration::from_secs(30)).on_before_exit({
        let app = app.clone();
        move || crate::shutdown(&app)
    });
    if mode() == Mode::Portable {
        b = b.target(PORTABLE_TARGET);
    }
    b.build().map_err(|e| e.to_string())
}

/// Looks for a newer version. Nothing is downloaded yet.
#[tauri::command]
pub async fn update_check(app: AppHandle, pending: State<'_, Pending>) -> R<UpdateInfo> {
    let found = updater(&app)?.check().await.map_err(|e| e.to_string())?;
    remember_check(&app);
    let info = UpdateInfo {
        current: env!("CARGO_PKG_VERSION").to_string(),
        mode: mode(),
        version: found.as_ref().map(|u| u.version.clone()),
        notes: found.as_ref().and_then(|u| u.body.clone()),
        date: found.as_ref().and_then(|u| u.raw_json.get("pub_date").and_then(|d| d.as_str()).map(str::to_string)),
        page: RELEASES_PAGE.to_string(),
    };
    log::info!("update check: current {}, available {:?} ({:?})", info.current, info.version, info.mode);
    *pending.0.lock() = found;
    Ok(info)
}

/// Downloads the update found by the last check, verifies its signature and
/// applies it. Chud closes (cleanly) and restarts as the new version.
#[tauri::command]
pub async fn update_install(app: AppHandle, pending: State<'_, Pending>) -> R<()> {
    let update = pending.0.lock().take().ok_or("no update to install: check again")?;
    let mut done = 0u64;
    let progress = {
        let app = app.clone();
        move |chunk: usize, total: Option<u64>| {
            done += chunk as u64;
            let _ = app.emit("update-progress", Progress { done, total });
        }
    };
    match mode() {
        Mode::Installer => {
            // Windows: the plugin starts the installer, then exits the process.
            // Linux: the AppImage or package is replaced, then Chud restarts.
            update.download_and_install(progress, || {}).await.map_err(|e| e.to_string())?;
            log::info!("update {} installed; restarting", update.version);
            crate::shutdown(&app);
            app.restart();
        }
        Mode::Portable => {
            let bytes = update.download(progress, || {}).await.map_err(|e| e.to_string())?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            swap_executable(&exe, &bytes)?;
            log::info!("portable executable updated to {}; restarting", update.version);
            std::process::Command::new(&exe).arg(UPDATED_FLAG).spawn().map_err(|e| format!("could not start the new version: {e}"))?;
            crate::shutdown(&app);
            app.exit(0);
            Ok(())
        }
        Mode::Manual => Err("this copy of Chud cannot update itself: download the new version from the release page".into()),
    }
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// Puts `bytes` in place of the running executable. Windows lets a running
/// executable be renamed (not deleted), so: write the new file next to it,
/// rename the current one to `*.old`, rename the new one into place.
fn swap_executable(exe: &Path, bytes: &[u8]) -> R<()> {
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(b"MZ") {
        return Err("the downloaded file is not a Windows executable".into());
    }
    let mut new_name = exe.file_name().unwrap_or_default().to_os_string();
    new_name.push(".new");
    let new = exe.with_file_name(new_name);
    let old = old_path(exe);
    std::fs::write(&new, bytes).map_err(|e| format!("cannot write next to the executable ({}): {e}", new.display()))?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("cannot move the current executable aside: {e}"));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("cannot put the new executable in place: {e}"));
    }
    Ok(())
}

/// At startup: removes the executable an update replaced. Started by an
/// update, waits (up to 15 s) for the previous process to exit first: its
/// file stays locked until then, and the single-instance check must not see it.
pub fn clean_after_update() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = old_path(&exe);
    if !old.exists() {
        return;
    }
    let wait = std::env::args().any(|a| a == UPDATED_FLAG);
    let deadline = std::time::Instant::now() + Duration::from_secs(if wait { 15 } else { 0 });
    loop {
        if std::fs::remove_file(&old).is_ok() || !old.exists() {
            log::info!("previous executable removed after an update");
            return;
        }
        if std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn remember_check(app: &AppHandle) {
    let state = app.state::<App>();
    let mut s = state.settings.write();
    s.update_last_check_ms = now_ms();
    let _ = crate::settings::save(&state.paths.settings_file, &s);
}

/// Automatic check at startup, at most once every 20 hours, only when the
/// user agreed to it. Tells the interface when a newer version exists.
pub fn check_in_background(app: &AppHandle) {
    let (enabled, last) = {
        let s = app.state::<App>().settings.read().clone();
        (s.update_check == Some(true), s.update_last_check_ms)
    };
    if !enabled || now_ms() - last < 20 * 3600 * 1000 {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(8));
        let pending = app.state::<Pending>();
        match tauri::async_runtime::block_on(update_check(app.clone(), pending)) {
            Ok(info) if info.version.is_some() => {
                let _ = app.emit("update-available", info);
            }
            Ok(_) => {}
            Err(e) => log::info!("automatic update check failed: {e}"),
        }
    });
}

#[tauri::command]
pub fn update_mode() -> Mode {
    mode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swapping_keeps_the_old_file_aside_and_refuses_non_executables() {
        let dir = std::env::temp_dir().join(format!("chud-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.exe");
        std::fs::write(&exe, b"MZ old").unwrap();
        assert!(swap_executable(&exe, b"not an exe").is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old", "a refused file changes nothing");

        let mut new = b"MZ".to_vec();
        new.resize(2 * 1024 * 1024, 7);
        swap_executable(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), new);
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"MZ old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
