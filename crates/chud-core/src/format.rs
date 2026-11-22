//! The `.chud` file format (see `docs/FORMAT.md`).
//!
//! A `.chud` file is a ZIP archive:
//!
//! | entry | content |
//! |---|---|
//! | `manifest.json` | format version, metadata, settings, environment, markers, summary |
//! | `events.bin` | current take, compact binary codec below |
//! | `original.bin` | untouched first take (optional) |
//! | `contexts.json` | window focus changes (optional) |
//! | `flow.json` | automation blocks (optional) |
//!
//! The binary event codec ("CHEV" v1) stores, per event, the time delta from
//! the previous event (LEB128), a tag byte and a tag specific payload where
//! absolute pointer positions are delta coded against the last known
//! position (zig-zag LEB128). Decoding is exact.

use std::io::{Read, Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{ContextSample, Environment, MacroDoc, MacroMeta, Marker};
use crate::event::{EventKind, Key, Micros, MouseButton, PadAxis, PadButton};
use crate::flow::Flow;
use crate::playback::PlaybackSettings;
use crate::timeline::Timeline;

pub const FORMAT_NAME: &str = "chud-input-recorder";
pub const FORMAT_VERSION: u32 = 1;
pub const EXTENSION: &str = "chud";
const MAGIC: &[u8; 4] = b"CHEV";
const CODEC_VERSION: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Chud file: {0}")]
    NotChud(String),
    #[error("file made by a newer version (format {0}); please update Chud Input Recorder")]
    TooNew(u32),
    #[error("corrupted event stream: {0}")]
    Corrupt(String),
}

/// Summary stored in the manifest so the library can display a macro
/// without decoding its events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Summary {
    pub event_count: usize,
    pub duration_us: Micros,
    pub keyboard: usize,
    pub mouse_buttons: usize,
    pub mouse_moves: usize,
    pub wheel: usize,
    pub gamepad: usize,
    pub has_flow: bool,
    /// Normalized event density over 48 buckets (0..=255) for previews.
    pub density: Vec<u8>,
}

impl Summary {
    pub fn of(doc: &MacroDoc) -> Self {
        use crate::event::Category::*;
        let tl = &doc.timeline;
        let mut s = Summary {
            event_count: tl.len(),
            duration_us: tl.duration(),
            has_flow: doc.flow.as_ref().is_some_and(|f| !f.blocks.is_empty()),
            ..Default::default()
        };
        let buckets = 48usize;
        let mut d = vec![0u32; buckets];
        let dur = tl.duration().max(1);
        for e in tl.events() {
            match e.kind.category() {
                Keyboard => s.keyboard += 1,
                MouseButton => s.mouse_buttons += 1,
                MouseMove | MouseRaw => s.mouse_moves += 1,
                Wheel => s.wheel += 1,
                PadButton | PadAxis => s.gamepad += 1,
            }
            // Discrete actions weigh more than motion samples.
            let w = if matches!(e.kind.category(), MouseMove | MouseRaw | PadAxis) { 1 } else { 12 };
            let b = ((e.t as f64 / dur as f64) * (buckets - 1) as f64) as usize;
            d[b.min(buckets - 1)] += w;
        }
        let max = d.iter().copied().max().unwrap_or(0).max(1) as f64;
        s.density = d.iter().map(|&v| ((v as f64 / max).sqrt() * 255.0).round() as u8).collect();
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub meta: MacroMeta,
    #[serde(default)]
    pub settings: PlaybackSettings,
    #[serde(default)]
    pub env: Environment,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub summary: Summary,
    #[serde(default)]
    pub has_original: bool,
}

// ------------------------------------------------------------------ codec

fn put_uvar(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_ivar(out: &mut Vec<u8>, v: i64) {
    put_uvar(out, ((v << 1) ^ (v >> 63)) as u64);
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, FormatError> {
        let b = *self.buf.get(self.pos).ok_or_else(|| FormatError::Corrupt("unexpected end".into()))?;
        self.pos += 1;
        Ok(b)
    }
    fn uvar(&mut self) -> Result<u64, FormatError> {
        let mut v: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            if shift >= 64 {
                return Err(FormatError::Corrupt("varint overflow".into()));
            }
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }
    fn ivar(&mut self) -> Result<i64, FormatError> {
        let u = self.uvar()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }
    fn i32v(&mut self) -> Result<i32, FormatError> {
        i32::try_from(self.ivar()?).map_err(|_| FormatError::Corrupt("value out of range".into()))
    }
}

mod tag {
    pub const MOVE: u8 = 1;
    pub const RAW: u8 = 2;
    pub const DOWN: u8 = 3;
    pub const UP: u8 = 4;
    pub const WHEEL: u8 = 5;
    pub const KEY_DOWN: u8 = 6;
    pub const KEY_UP: u8 = 7;
    pub const PAD_BUTTON: u8 = 8;
    pub const PAD_AXIS: u8 = 9;
    pub const KEY_REPEAT: u8 = 10;
}

/// Encodes events (must be sorted by time).
pub fn encode_events(items: &[(Micros, EventKind)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + items.len() * 5);
    out.extend_from_slice(MAGIC);
    out.push(CODEC_VERSION);
    put_uvar(&mut out, items.len() as u64);
    let mut prev_t: Micros = 0;
    let (mut px, mut py) = (0i64, 0i64);
    let pos = |out: &mut Vec<u8>, px: &mut i64, py: &mut i64, x: i32, y: i32| {
        put_ivar(out, x as i64 - *px);
        put_ivar(out, y as i64 - *py);
        *px = x as i64;
        *py = y as i64;
    };
    for &(t, k) in items {
        put_uvar(&mut out, (t - prev_t).max(0) as u64);
        prev_t = t.max(prev_t);
        match k {
            EventKind::MouseMove { x, y } => {
                out.push(tag::MOVE);
                pos(&mut out, &mut px, &mut py, x, y);
            }
            EventKind::MouseRaw { dx, dy } => {
                out.push(tag::RAW);
                put_ivar(&mut out, dx as i64);
                put_ivar(&mut out, dy as i64);
            }
            EventKind::MouseDown { button, x, y } | EventKind::MouseUp { button, x, y } => {
                out.push(if matches!(k, EventKind::MouseDown { .. }) { tag::DOWN } else { tag::UP });
                out.push(button as u8);
                pos(&mut out, &mut px, &mut py, x, y);
            }
            EventKind::Wheel { delta, horizontal, x, y } => {
                out.push(tag::WHEEL);
                out.push(horizontal as u8);
                put_ivar(&mut out, delta as i64);
                pos(&mut out, &mut px, &mut py, x, y);
            }
            EventKind::KeyDown { key, repeat } => {
                out.push(if repeat { tag::KEY_REPEAT } else { tag::KEY_DOWN });
                put_uvar(&mut out, key.scan as u64);
                put_uvar(&mut out, key.vk as u64);
            }
            EventKind::KeyUp { key } => {
                out.push(tag::KEY_UP);
                put_uvar(&mut out, key.scan as u64);
                put_uvar(&mut out, key.vk as u64);
            }
            EventKind::PadButton { pad, button, down } => {
                out.push(tag::PAD_BUTTON);
                out.push(pad);
                out.push(button.0);
                out.push(down as u8);
            }
            EventKind::PadAxis { pad, axis, value } => {
                out.push(tag::PAD_AXIS);
                out.push(pad);
                out.push(axis.0);
                put_ivar(&mut out, value as i64);
            }
        }
    }
    out
}

pub fn decode_events(buf: &[u8]) -> Result<Vec<(Micros, EventKind)>, FormatError> {
    if buf.len() < 5 || &buf[..4] != MAGIC {
        return Err(FormatError::Corrupt("bad magic".into()));
    }
    if buf[4] > CODEC_VERSION {
        return Err(FormatError::Corrupt(format!("unknown codec version {}", buf[4])));
    }
    let mut r = Reader { buf, pos: 5 };
    let n = r.uvar()? as usize;
    if n > buf.len() {
        return Err(FormatError::Corrupt("event count larger than data".into()));
    }
    let mut out = Vec::with_capacity(n);
    let mut t: Micros = 0;
    let (mut px, mut py) = (0i64, 0i64);
    for _ in 0..n {
        t = t.checked_add(r.uvar()? as i64).ok_or_else(|| FormatError::Corrupt("time overflow".into()))?;
        let tg = r.byte()?;
        let mut pos = |r: &mut Reader| -> Result<(i32, i32), FormatError> {
            px += r.ivar()?;
            py += r.ivar()?;
            let x = i32::try_from(px).map_err(|_| FormatError::Corrupt("x out of range".into()))?;
            let y = i32::try_from(py).map_err(|_| FormatError::Corrupt("y out of range".into()))?;
            Ok((x, y))
        };
        let kind = match tg {
            tag::MOVE => {
                let (x, y) = pos(&mut r)?;
                EventKind::MouseMove { x, y }
            }
            tag::RAW => EventKind::MouseRaw { dx: r.i32v()?, dy: r.i32v()? },
            tag::DOWN | tag::UP => {
                let button = MouseButton::from_u8(r.byte()?).ok_or_else(|| FormatError::Corrupt("bad button".into()))?;
                let (x, y) = pos(&mut r)?;
                if tg == tag::DOWN {
                    EventKind::MouseDown { button, x, y }
                } else {
                    EventKind::MouseUp { button, x, y }
                }
            }
            tag::WHEEL => {
                let horizontal = r.byte()? != 0;
                let delta = r.i32v()?;
                let (x, y) = pos(&mut r)?;
                EventKind::Wheel { delta, horizontal, x, y }
            }
            tag::KEY_DOWN | tag::KEY_REPEAT | tag::KEY_UP => {
                let scan = u16::try_from(r.uvar()?).map_err(|_| FormatError::Corrupt("scan".into()))?;
                let vk = u16::try_from(r.uvar()?).map_err(|_| FormatError::Corrupt("vk".into()))?;
                let key = Key::new(scan, vk);
                match tg {
                    tag::KEY_UP => EventKind::KeyUp { key },
                    _ => EventKind::KeyDown { key, repeat: tg == tag::KEY_REPEAT },
                }
            }
            tag::PAD_BUTTON => {
                let pad = r.byte()?;
                let button = PadButton(r.byte()?);
                let down = r.byte()? != 0;
                EventKind::PadButton { pad, button, down }
            }
            tag::PAD_AXIS => {
                let pad = r.byte()?;
                let axis = PadAxis(r.byte()?);
                let v = r.ivar()?;
                let value = i16::try_from(v).map_err(|_| FormatError::Corrupt("axis value".into()))?;
                EventKind::PadAxis { pad, axis, value }
            }
            other => return Err(FormatError::Corrupt(format!("unknown tag {other}"))),
        };
        out.push((t, kind));
    }
    Ok(out)
}

// ------------------------------------------------------------------ archive

pub fn manifest_of(doc: &MacroDoc) -> Manifest {
    Manifest {
        format: FORMAT_NAME.into(),
        version: FORMAT_VERSION,
        meta: doc.meta.clone(),
        settings: doc.settings.clone(),
        env: doc.env.clone(),
        markers: doc.markers.clone(),
        summary: Summary::of(doc),
        has_original: doc.original.is_some(),
    }
}

pub fn write_doc<W: Write + Seek>(doc: &MacroDoc, w: W) -> Result<(), FormatError> {
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(w);
    let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("manifest.json", deflate)?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest_of(doc))?)?;

    zip.start_file("events.bin", deflate)?;
    zip.write_all(&encode_events(&doc.current_items()))?;

    if let Some(orig) = &doc.original {
        zip.start_file("original.bin", deflate)?;
        zip.write_all(&encode_events(orig))?;
    }
    if !doc.contexts.is_empty() {
        zip.start_file("contexts.json", deflate)?;
        zip.write_all(&serde_json::to_vec(&doc.contexts)?)?;
    }
    if let Some(flow) = &doc.flow {
        zip.start_file("flow.json", deflate)?;
        zip.write_all(&serde_json::to_vec_pretty(flow)?)?;
    }
    // PNG is already compressed: stored as is.
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (id, png) in &doc.images {
        if crate::document::valid_image_id(id) {
            zip.start_file(format!("images/{id}.png"), stored)?;
            zip.write_all(png)?;
        }
    }
    zip.finish()?;
    Ok(())
}

fn read_entry<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<Option<Vec<u8>>, FormatError> {
    match zip.by_name(name) {
        Ok(mut f) => {
            let mut v = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut v)?;
            Ok(Some(v))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn check_manifest(m: &Manifest) -> Result<(), FormatError> {
    if m.format != FORMAT_NAME {
        return Err(FormatError::NotChud(format!("format `{}`", m.format)));
    }
    if m.version > FORMAT_VERSION {
        return Err(FormatError::TooNew(m.version));
    }
    Ok(())
}

/// Reads only the manifest (fast: used by the library index).
pub fn read_manifest<R: Read + Seek>(r: R) -> Result<Manifest, FormatError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let bytes = read_entry(&mut zip, "manifest.json")?.ok_or_else(|| FormatError::NotChud("no manifest".into()))?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    check_manifest(&m)?;
    Ok(m)
}

pub fn read_doc<R: Read + Seek>(r: R) -> Result<MacroDoc, FormatError> {
    let mut zip = zip::ZipArchive::new(r)?;
    let bytes = read_entry(&mut zip, "manifest.json")?.ok_or_else(|| FormatError::NotChud("no manifest".into()))?;
    let m: Manifest = serde_json::from_slice(&bytes)?;
    check_manifest(&m)?;
    let events = match read_entry(&mut zip, "events.bin")? {
        Some(b) => decode_events(&b)?,
        None => Vec::new(),
    };
    let original = match read_entry(&mut zip, "original.bin")? {
        Some(b) => Some(decode_events(&b)?),
        None => None,
    };
    let contexts: Vec<ContextSample> = match read_entry(&mut zip, "contexts.json")? {
        Some(b) => serde_json::from_slice(&b)?,
        None => Vec::new(),
    };
    let flow: Option<Flow> = match read_entry(&mut zip, "flow.json")? {
        Some(b) => Some(serde_json::from_slice(&b)?),
        None => None,
    };
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.to_string())).collect();
    let mut images = std::collections::BTreeMap::new();
    for name in names {
        let Some(id) = name.strip_prefix("images/").and_then(|n| n.strip_suffix(".png")) else { continue };
        if crate::document::valid_image_id(id)
            && let Some(png) = read_entry(&mut zip, &name)?
        {
            images.insert(id.to_string(), png);
        }
    }
    Ok(MacroDoc {
        meta: m.meta,
        settings: m.settings,
        env: m.env,
        timeline: Timeline::from_kinds(events),
        original,
        markers: m.markers,
        contexts,
        flow,
        images,
    })
}

/// Saves atomically: writes a temporary sibling then renames it over the
/// destination, so a crash never leaves a half-written macro.
pub fn save(doc: &MacroDoc, path: &Path) -> Result<(), FormatError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut bw = std::io::BufWriter::new(f);
        write_doc(doc, &mut bw)?;
        let f = bw.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(path: &Path) -> Result<MacroDoc, FormatError> {
    let f = std::fs::File::open(path)?;
    read_doc(std::io::BufReader::new(f))
}

pub fn load_manifest(path: &Path) -> Result<Manifest, FormatError> {
    let f = std::fs::File::open(path)?;
    read_manifest(std::io::BufReader::new(f))
}

// ------------------------------------------------------------------ JSON

/// Complete, human-readable JSON representation (export / import).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonDoc {
    pub format: String,
    pub version: u32,
    pub meta: MacroMeta,
    #[serde(default)]
    pub settings: PlaybackSettings,
    #[serde(default)]
    pub env: Environment,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub contexts: Vec<ContextSample>,
    #[serde(default)]
    pub flow: Option<Flow>,
    /// Images of the flow: PNG files in base64, by id.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub images: std::collections::BTreeMap<String, String>,
    pub events: Vec<JsonEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEvent {
    pub t: Micros,
    #[serde(flatten)]
    pub kind: EventKind,
}

pub fn to_json(doc: &MacroDoc) -> Result<String, FormatError> {
    let j = JsonDoc {
        format: FORMAT_NAME.into(),
        version: FORMAT_VERSION,
        meta: doc.meta.clone(),
        settings: doc.settings.clone(),
        env: doc.env.clone(),
        markers: doc.markers.clone(),
        contexts: doc.contexts.clone(),
        flow: doc.flow.clone(),
        images: doc.images.iter().map(|(id, png)| (id.clone(), base64::encode(png))).collect(),
        events: doc.timeline.events().iter().map(|e| JsonEvent { t: e.t, kind: e.kind }).collect(),
    };
    Ok(serde_json::to_string_pretty(&j)?)
}

pub fn from_json(s: &str) -> Result<MacroDoc, FormatError> {
    let j: JsonDoc = serde_json::from_str(s)?;
    if j.format != FORMAT_NAME {
        return Err(FormatError::NotChud(format!("format `{}`", j.format)));
    }
    if j.version > FORMAT_VERSION {
        return Err(FormatError::TooNew(j.version));
    }
    let mut items: Vec<(Micros, EventKind)> = j.events.into_iter().map(|e| (e.t.max(0), e.kind)).collect();
    items.sort_by_key(|(t, _)| *t);
    Ok(MacroDoc {
        meta: j.meta,
        settings: j.settings,
        env: j.env,
        timeline: Timeline::from_kinds(items),
        original: None,
        markers: j.markers,
        contexts: j.contexts,
        flow: j.flow,
        images: j
            .images
            .into_iter()
            .filter(|(id, _)| crate::document::valid_image_id(id))
            .map(|(id, b64)| base64::decode(&b64).map(|png| (id, png)).ok_or_else(|| FormatError::NotChud("bad image data".into())))
            .collect::<Result<_, _>>()?,
    })
}

/// Minimal standard base64 (RFC 4648) for images in JSON exports.
pub mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let (mut acc, mut bits) = (0u32, 0u32);
        for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
            if c == b'=' {
                break;
            }
            let v = ALPHABET.iter().position(|&a| a == c)? as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Block, Node};

    fn sample_items() -> Vec<(Micros, EventKind)> {
        vec![
            (0, EventKind::MouseMove { x: -1200, y: 300 }),
            (125, EventKind::MouseRaw { dx: -3, dy: 2 }),
            (1_000, EventKind::MouseDown { button: MouseButton::X2, x: -1199, y: 301 }),
            (1_000, EventKind::KeyDown { key: Key::new(0xE01D, 0xA3), repeat: false }),
            (31_000, EventKind::KeyDown { key: Key::new(0xE01D, 0xA3), repeat: true }),
            (40_000, EventKind::KeyUp { key: Key::new(0xE01D, 0xA3) }),
            (41_000, EventKind::Wheel { delta: -360, horizontal: true, x: 5000, y: -20 }),
            (42_000, EventKind::MouseUp { button: MouseButton::X2, x: i32::MAX, y: i32::MIN }),
            (50_000, EventKind::PadButton { pad: 1, button: PadButton::NORTH, down: true }),
            (50_500, EventKind::PadAxis { pad: 1, axis: PadAxis::LEFT_TRIGGER, value: i16::MIN }),
            (9_000_000_000, EventKind::KeyUp { key: Key::new(0xF2FF, 0) }),
        ]
    }

    #[test]
    fn images_survive_archive_and_json() {
        let mut doc = MacroDoc::from_recording("Img", sample_items(), Environment::default());
        let png = crate::vision::Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).to_png();
        let id = doc.add_image(png.clone());
        // An id that could escape the images folder is never written.
        doc.images.insert("../evil".into(), vec![1, 2, 3]);
        let mut buf = std::io::Cursor::new(Vec::new());
        write_doc(&doc, &mut buf).unwrap();
        let back = read_doc(std::io::Cursor::new(buf.into_inner())).unwrap();
        assert_eq!(back.images.len(), 1);
        assert_eq!(back.images.get(&id), Some(&png));
        assert_eq!(back.decoded_images()[&id].pixel(1, 0), [0, 0, 255, 128]);
        let json = to_json(&back).unwrap();
        let again = from_json(&json).unwrap();
        assert_eq!(again.images, back.images);
    }

    #[test]
    fn base64_matches_the_rfc_examples() {
        for (raw, enc) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64::encode(raw.as_bytes()), enc);
            assert_eq!(base64::decode(enc).unwrap(), raw.as_bytes());
        }
        assert_eq!(base64::decode("Zm9v!"), None);
    }

    #[test]
    fn codec_roundtrip_is_exact() {
        let items = sample_items();
        let bytes = encode_events(&items);
        assert_eq!(decode_events(&bytes).unwrap(), items);
    }

    #[test]
    fn codec_is_compact_for_mouse_paths() {
        let items: Vec<_> = (0..10_000).map(|i| (i as i64 * 1_000, EventKind::MouseMove { x: 500 + i % 7, y: 300 - i % 5 })).collect();
        let bytes = encode_events(&items);
        assert!(bytes.len() < items.len() * 6, "{} bytes", bytes.len());
    }

    #[test]
    fn corrupt_streams_are_rejected_not_panicking() {
        let bytes = encode_events(&sample_items());
        for cut in 0..bytes.len() {
            let _ = decode_events(&bytes[..cut]);
        }
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(decode_events(&bad).is_err());
        // Random garbage after a valid header.
        let mut junk = b"CHEV\x01\x05".to_vec();
        junk.extend((0..40).map(|i| (i * 37 % 251) as u8));
        let _ = decode_events(&junk);
    }

    #[test]
    fn archive_roundtrip() {
        let mut doc = MacroDoc::from_recording("Test", sample_items(), Environment { os: "windows".into(), ..Default::default() });
        doc.meta.tags = vec!["demo".into()];
        doc.markers.push(Marker { t: 1_000, label: "start".into(), ..Default::default() });
        doc.contexts.push(ContextSample { t: 0, process: "notepad.exe".into(), title: "x".into(), ..Default::default() });
        doc.flow = Some(Flow { blocks: vec![Node::new(Block::Notify { message: "hi".into() })] });
        // Edit the current take: the original must stay untouched.
        let first = *doc.timeline.events().first().unwrap();
        doc.timeline.apply(&crate::ops::delete(&doc.timeline, &[first.id], false)).unwrap();

        let mut buf = std::io::Cursor::new(Vec::new());
        write_doc(&doc, &mut buf).unwrap();
        buf.set_position(0);
        let m = read_manifest(&mut buf).unwrap();
        assert_eq!(m.meta.name, "Test");
        assert_eq!(m.summary.event_count, sample_items().len() - 1);
        assert_eq!(m.summary.density.len(), 48);
        buf.set_position(0);
        let back = read_doc(buf).unwrap();
        assert_eq!(back.current_items(), doc.current_items());
        assert_eq!(back.original.as_ref().unwrap().len(), sample_items().len());
        assert_eq!(back.meta, doc.meta);
        assert_eq!(back.markers, doc.markers);
        assert_eq!(back.contexts, doc.contexts);
        assert_eq!(back.flow, doc.flow);
    }

    #[test]
    fn json_roundtrip() {
        let doc = MacroDoc::from_recording("J", sample_items(), Environment::default());
        let s = to_json(&doc).unwrap();
        assert!(s.contains("\"type\": \"mouse_down\""));
        let back = from_json(&s).unwrap();
        assert_eq!(back.current_items(), doc.current_items());
    }

    #[test]
    fn save_is_atomic_and_loadable() {
        let dir = std::env::temp_dir().join(format!("chud-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("m.chud");
        let doc = MacroDoc::from_recording("Disk", sample_items(), Environment::default());
        save(&doc, &path).unwrap();
        assert!(!path.with_extension("chud.tmp").exists());
        assert_eq!(load(&path).unwrap().current_items(), doc.current_items());
        assert_eq!(load_manifest(&path).unwrap().meta.name, "Disk");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
