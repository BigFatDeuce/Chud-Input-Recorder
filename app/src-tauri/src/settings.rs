//! Application settings and data locations (normal or portable mode).

use std::path::{Path, PathBuf};

use chud_core::playback::PlaybackSettings;
use chud_engine::RecordOptions;
use chud_platform::Hotkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OverlaySettings {
    /// Show the floating indicator while recording (always on: the setting
    /// only controls the indicator during playback).
    pub show_when_playing: bool,
    /// "top" | "bottom"
    pub position: String,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self { show_when_playing: true, position: "top".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    /// "auto", "fr" or "en".
    pub language: String,
    pub hotkeys: Vec<Hotkey>,
    pub record: RecordOptions,
    pub playback: PlaybackSettings,
    pub overlay: OverlaySettings,
    pub autosave: bool,
    pub autosave_delay_ms: u64,
    /// Previous versions kept for each macro.
    pub versions_kept: usize,
    /// Custom library folder (default: Documents/Chud Input Recorder/Library).
    pub library_dir: Option<String>,
    pub reduce_motion: bool,
    /// Interface zoom (1 = 100 %), on top of the system display scaling.
    pub ui_scale: f64,
    /// Glow intensity, 0..=1.
    pub glow: f32,
    pub starfield: bool,
    pub close_to_tray: bool,
    /// Ask before moving items to the trash.
    pub confirm_delete: bool,
    /// Minimize Chud while a macro started from its window plays, so the
    /// input reaches the application behind it; restored afterwards.
    pub minimize_on_play: bool,
    /// Minimize Chud when a recording starts while its window has the focus.
    /// Input made in Chud's own window is never recorded, so without this a
    /// recording started from the Record button would see nothing until the
    /// user switches to another window. Restored when the recording stops.
    pub minimize_on_record: bool,
    /// Soft beeps during the recording countdown (last seconds, then "go").
    pub countdown_sound: bool,
    /// Look for a new version at startup (at most once a day). `None`: the
    /// user has not been asked yet; nothing is checked until they answer.
    pub update_check: Option<bool>,
    /// Last update check (Unix ms).
    pub update_last_check_ms: i64,
    /// Macros started by `Trigger(slot)` hotkeys.
    pub triggers: Vec<MacroTrigger>,
}

/// Global hotkey slot bound to a library macro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroTrigger {
    pub slot: u32,
    pub macro_id: String,
    #[serde(default)]
    pub use_flow: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            hotkeys: Hotkey::defaults(),
            record: RecordOptions::default(),
            playback: PlaybackSettings::default(),
            overlay: OverlaySettings::default(),
            autosave: true,
            autosave_delay_ms: 1500,
            versions_kept: 20,
            library_dir: None,
            reduce_motion: false,
            ui_scale: 1.0,
            glow: 0.7,
            starfield: true,
            close_to_tray: false,
            confirm_delete: true,
            minimize_on_play: true,
            minimize_on_record: true,
            countdown_sound: true,
            update_check: None,
            update_last_check_ms: 0,
            triggers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Paths {
    pub portable: bool,
    pub data_dir: PathBuf,
    pub settings_file: PathBuf,
    pub library_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl Paths {
    /// Portable mode is enabled by a `portable.flag` file next to the
    /// executable: everything then lives in `./data`.
    pub fn resolve() -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        if let Some(dir) = exe_dir.filter(|d| d.join("portable.flag").exists()) {
            let data = dir.join("data");
            return Self {
                portable: true,
                settings_file: data.join("settings.json"),
                library_dir: data.join("Library"),
                logs_dir: data.join("logs"),
                data_dir: data,
            };
        }
        let data = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("Chud Input Recorder");
        let docs = dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| data.clone());
        Self {
            portable: false,
            settings_file: data.join("settings.json"),
            library_dir: docs.join("Chud Input Recorder").join("Library"),
            logs_dir: data.join("logs"),
            data_dir: data,
        }
    }

    pub fn library_for(&self, s: &AppSettings) -> PathBuf {
        match &s.library_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.library_dir.clone(),
        }
    }
}

pub fn load(path: &Path) -> AppSettings {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            log::warn!("settings file unreadable ({e}); using defaults");
            // Keep the broken file for the user instead of overwriting it.
            let _ = std::fs::copy(path, path.with_extension("json.broken"));
            AppSettings::default()
        }),
        Err(_) => AppSettings::default(),
    }
}

pub fn save(path: &Path, s: &AppSettings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = std::env::temp_dir().join(format!("chud-settings-{}", std::process::id()));
        let f = dir.join("settings.json");
        let mut s = AppSettings { language: "fr".into(), ..Default::default() };
        s.playback.speed = 2.0;
        save(&f, &s).unwrap();
        assert_eq!(load(&f), s);
        // Unknown and missing fields are tolerated.
        std::fs::write(&f, r#"{"language":"en","future_option":1}"#).unwrap();
        let l = load(&f);
        assert_eq!(l.language, "en");
        assert_eq!(l.hotkeys, Hotkey::defaults());
        let _ = std::fs::remove_dir_all(dir);
    }
}
