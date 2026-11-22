//! A macro document: metadata, the recording, its untouched original take,
//! playback settings, markers, window context, the optional Flow and the
//! images its "find on screen" blocks look for.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::event::{EventKind, Micros};
use crate::flow::Flow;
use crate::playback::PlaybackSettings;
use crate::timeline::Timeline;
use crate::vision::Image;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MacroMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// Icon identifier from the app's icon set.
    pub icon: String,
    /// Accent color, `#rrggbb`.
    pub color: String,
    pub favorite: bool,
    pub created_ms: i64,
    pub modified_ms: i64,
    pub app_version: String,
    pub source_os: String,
}

impl Default for MacroMeta {
    fn default() -> Self {
        let now = now_ms();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            description: String::new(),
            tags: Vec::new(),
            icon: "orbit".into(),
            color: "#f9c83a".into(),
            favorite: false,
            created_ms: now,
            modified_ms: now,
            app_version: env!("CARGO_PKG_VERSION").into(),
            source_os: std::env::consts::OS.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
    pub fn distance2(&self, x: i32, y: i32) -> i64 {
        let cx = x.clamp(self.x, self.x + self.width - 1);
        let cy = y.clamp(self.y, self.y + self.height - 1);
        let (dx, dy) = ((x - cx) as i64, (y - cy) as i64);
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Monitor {
    pub name: String,
    /// Bounds in physical pixels of the virtual desktop.
    pub rect: Rect,
    /// Scale factor (1.0 = 96 DPI).
    pub scale: f32,
    pub primary: bool,
    pub refresh_hz: Option<u32>,
}

/// The machine a recording was made on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Environment {
    pub os: String,
    pub monitors: Vec<Monitor>,
    pub keyboard_layout: String,
    /// Pointer speed setting (Windows 1..=20).
    pub mouse_speed: Option<u32>,
    pub mouse_acceleration: Option<bool>,
    pub hostname_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Marker {
    pub t: Micros,
    /// End of a region marker; `None` for a point marker.
    pub end: Option<Micros>,
    pub label: String,
    pub color: String,
}

impl Default for Marker {
    fn default() -> Self {
        Self { t: 0, end: None, label: String::new(), color: "#38d6ff".into() }
    }
}

/// Foreground window change observed while recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct ContextSample {
    pub t: Micros,
    pub process: String,
    pub title: String,
    pub class: String,
    pub rect: Option<Rect>,
    /// Recording was suspended because this application is excluded.
    pub redacted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MacroDoc {
    pub meta: MacroMeta,
    pub settings: PlaybackSettings,
    pub env: Environment,
    pub timeline: Timeline,
    /// First take, never modified by editing.
    pub original: Option<Vec<(Micros, EventKind)>>,
    pub markers: Vec<Marker>,
    pub contexts: Vec<ContextSample>,
    pub flow: Option<Flow>,
    /// Images searched on screen by the flow, PNG encoded, by id.
    pub images: BTreeMap<String, Vec<u8>>,
}

/// Image ids are short and file-name safe (`images/<id>.png` in archives).
pub fn valid_image_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl MacroDoc {
    /// Stores an image and returns its new id.
    pub fn add_image(&mut self, png: Vec<u8>) -> String {
        let id = loop {
            let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
            if !self.images.contains_key(&id) {
                break id;
            }
        };
        self.images.insert(id.clone(), png);
        id
    }

    /// The stored images, decoded (undecodable ones are left out).
    pub fn decoded_images(&self) -> HashMap<String, Image> {
        self.images.iter().filter_map(|(id, png)| Image::from_png(png).ok().map(|i| (id.clone(), i))).collect()
    }

    /// Creates a document from a fresh recording; the take is kept as original.
    pub fn from_recording(name: impl Into<String>, items: Vec<(Micros, EventKind)>, env: Environment) -> Self {
        let timeline = Timeline::from_kinds(items.iter().copied());
        let original = Some(timeline.events().iter().map(|e| (e.t, e.kind)).collect());
        Self {
            meta: MacroMeta { name: name.into(), ..Default::default() },
            env,
            timeline,
            original,
            ..Default::default()
        }
    }

    pub fn current_items(&self) -> Vec<(Micros, EventKind)> {
        self.timeline.events().iter().map(|e| (e.t, e.kind)).collect()
    }

    pub fn touch(&mut self) {
        self.meta.modified_ms = now_ms();
    }
}

/// Maps coordinates recorded on one monitor layout onto another: each point
/// keeps its relative position inside the corresponding monitor.
#[derive(Debug, Clone)]
pub struct CoordMap {
    pairs: Vec<(Rect, Rect)>,
    identity: bool,
}

impl CoordMap {
    /// Monitors are matched primary-first, then left to right, top to bottom.
    pub fn new(recorded: &[Monitor], current: &[Monitor]) -> Self {
        let order = |ms: &[Monitor]| {
            let mut v: Vec<Rect> = Vec::new();
            let mut sorted: Vec<&Monitor> = ms.iter().collect();
            sorted.sort_by_key(|m| (!m.primary, m.rect.x, m.rect.y));
            v.extend(sorted.iter().map(|m| m.rect));
            v
        };
        let (a, b) = (order(recorded), order(current));
        let pairs: Vec<(Rect, Rect)> = a.iter().copied().zip(b.iter().copied()).collect();
        let identity = pairs.is_empty() || (a.len() == b.len() && pairs.iter().all(|(r, c)| r == c));
        Self { pairs, identity }
    }

    pub fn is_identity(&self) -> bool {
        self.identity
    }

    pub fn map(&self, x: i32, y: i32) -> (i32, i32) {
        if self.identity {
            return (x, y);
        }
        let (src, dst) = self
            .pairs
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .or_else(|| self.pairs.iter().min_by_key(|(r, _)| r.distance2(x, y)))
            .copied()
            .unwrap();
        let fx = (x - src.x) as f64 / src.width.max(1) as f64;
        let fy = (y - src.y) as f64 / src.height.max(1) as f64;
        (
            dst.x + (fx * dst.width as f64).round() as i32,
            dst.y + (fy * dst.height as f64).round() as i32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, y: i32, w: i32, h: i32, primary: bool) -> Monitor {
        Monitor { rect: Rect { x, y, width: w, height: h }, primary, scale: 1.0, ..Default::default() }
    }

    #[test]
    fn identical_layouts_are_identity() {
        let a = vec![mon(0, 0, 2560, 1440, true), mon(-1080, -200, 1080, 1920, false)];
        let m = CoordMap::new(&a, &a);
        assert!(m.is_identity());
        assert_eq!(m.map(-500, 100), (-500, 100));
    }

    #[test]
    fn scales_inside_matching_monitor() {
        let rec = vec![mon(0, 0, 1920, 1080, true), mon(1920, 0, 1920, 1080, false)];
        let cur = vec![mon(0, 0, 2560, 1440, true), mon(-1280, 0, 1280, 720, false)];
        let m = CoordMap::new(&rec, &cur);
        assert_eq!(m.map(960, 540), (1280, 720));
        assert_eq!(m.map(1920 + 960, 540), (-640, 360));
        // Points outside every monitor use the nearest one.
        assert_eq!(m.map(-10, 0), (-13, 0));
    }
}
