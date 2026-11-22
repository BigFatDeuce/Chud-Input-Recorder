//! Finding things on screen: images (template matching) and colours.
//!
//! Template matching uses normalised cross-correlation, the measure of
//! OpenCV's `TM_CCOEFF_NORMED`: a match survives brightness and contrast
//! changes, and its score (0..1) reads as a similarity. To stay fast on
//! large desktops the search runs coarse to fine: a full scan of a reduced
//! copy of the screen (image pyramid), then each candidate is refined level
//! by level, and the final score is computed in colour at full resolution
//! (so a red button does not match the same button in green). Transparent
//! pixels of the searched image are ignored, which lets an icon with an
//! alpha channel match whatever background it sits on.
//!
//! Everything here is pure computation on pixel buffers: the screen itself
//! is captured by the platform layer.

use std::io::Cursor;

use serde::{Deserialize, Serialize};

/// An 8-bit RGBA picture.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}×{})", self.w, self.h)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VisionError {
    #[error("unreadable image: {0}")]
    Decode(String),
    #[error("image too large")]
    TooLarge,
}

/// Largest picture accepted from a file (pixels).
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

impl Image {
    pub fn new(w: u32, h: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), w as usize * h as usize * 4, "RGBA buffer size");
        Self { w, h, rgba }
    }

    /// From BGRA/BGRX rows (Windows DIBs, X11 images); made opaque.
    pub fn from_bgra(w: u32, h: u32, mut px: Vec<u8>) -> Self {
        for p in px.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            p[3] = 255;
        }
        Self::new(w, h, px)
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.w as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// True when some pixel is not fully opaque.
    pub fn has_transparency(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255)
    }

    /// The part of the image inside the rectangle (clipped to the image).
    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Image {
        let x0 = x.clamp(0, self.w as i32) as u32;
        let y0 = y.clamp(0, self.h as i32) as u32;
        let x1 = (x as i64 + w as i64).clamp(x0 as i64, self.w as i64) as u32;
        let y1 = (y as i64 + h as i64).clamp(y0 as i64, self.h as i64) as u32;
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = Vec::with_capacity(cw as usize * ch as usize * 4);
        for row in y0..y1 {
            let start = (row as usize * self.w as usize + x0 as usize) * 4;
            out.extend_from_slice(&self.rgba[start..start + cw as usize * 4]);
        }
        Image::new(cw, ch, out)
    }

    /// Bilinear resize (area-averaged when shrinking by more than half).
    pub fn resize(&self, w: u32, h: u32) -> Image {
        let (w, h) = (w.max(1), h.max(1));
        if self.is_empty() {
            return Image::new(w, h, vec![0; w as usize * h as usize * 4]);
        }
        let mut src = self.clone();
        while src.w >= w * 2 && src.h >= h * 2 && src.w > 1 && src.h > 1 {
            src = src.halve();
        }
        let mut out = vec![0u8; w as usize * h as usize * 4];
        let sx = src.w as f32 / w as f32;
        let sy = src.h as f32 / h as f32;
        for y in 0..h {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (src.h - 1) as f32);
            let (y0, ty) = (fy.floor() as u32, fy.fract());
            let y1 = (y0 + 1).min(src.h - 1);
            for x in 0..w {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (src.w - 1) as f32);
                let (x0, tx) = (fx.floor() as u32, fx.fract());
                let x1 = (x0 + 1).min(src.w - 1);
                let (a, b, c, d) = (src.pixel(x0, y0), src.pixel(x1, y0), src.pixel(x0, y1), src.pixel(x1, y1));
                let o = (y as usize * w as usize + x as usize) * 4;
                for k in 0..4 {
                    let top = a[k] as f32 * (1.0 - tx) + b[k] as f32 * tx;
                    let bottom = c[k] as f32 * (1.0 - tx) + d[k] as f32 * tx;
                    out[o + k] = (top * (1.0 - ty) + bottom * ty).round() as u8;
                }
            }
        }
        Image::new(w, h, out)
    }

    fn halve(&self) -> Image {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut out = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = ((x * 2).min(self.w - 1), (y * 2).min(self.h - 1));
                let (sx1, sy1) = ((sx + 1).min(self.w - 1), (sy + 1).min(self.h - 1));
                let p = [self.pixel(sx, sy), self.pixel(sx1, sy), self.pixel(sx, sy1), self.pixel(sx1, sy1)];
                let o = (y as usize * w as usize + x as usize) * 4;
                for k in 0..4 {
                    out[o + k] = ((p[0][k] as u32 + p[1][k] as u32 + p[2][k] as u32 + p[3][k] as u32 + 2) / 4) as u8;
                }
            }
        }
        Image::new(w, h, out)
    }

    pub fn from_png(bytes: &[u8]) -> Result<Image, VisionError> {
        let err = |e: png::DecodingError| VisionError::Decode(e.to_string());
        let mut dec = png::Decoder::new(Cursor::new(bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(err)?;
        let (w, h) = {
            let info = reader.info();
            (info.width, info.height)
        };
        if w as u64 * h as u64 > MAX_PIXELS {
            return Err(VisionError::TooLarge);
        }
        let size = reader.output_buffer_size().ok_or(VisionError::TooLarge)?;
        let mut buf = vec![0u8; size];
        let info = reader.next_frame(&mut buf).map_err(err)?;
        let ch = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err(VisionError::Decode("indexed colours".into())),
        };
        let (w, h) = (info.width, info.height);
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h as usize {
            let row = &buf[y * info.line_size..][..w as usize * ch];
            for p in row.chunks_exact(ch) {
                rgba.extend_from_slice(&match ch {
                    1 => [p[0], p[0], p[0], 255],
                    2 => [p[0], p[0], p[0], p[1]],
                    3 => [p[0], p[1], p[2], 255],
                    _ => [p[0], p[1], p[2], p[3]],
                });
            }
        }
        Ok(Image::new(w, h, rgba))
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w.max(1), self.h.max(1));
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Balanced);
            // Writing to memory cannot fail for a well-formed buffer.
            if let Ok(mut wr) = enc.write_header() {
                let _ = wr.write_image_data(&self.rgba);
            }
        }
        out
    }
}

/// A rectangle in screen pixels (virtual desktop coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    pub fn intersect(&self, o: &Region) -> Option<Region> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x as i64 + self.w as i64).min(o.x as i64 + o.w as i64);
        let y1 = (self.y as i64 + self.h as i64).min(o.y as i64 + o.h as i64);
        (x1 > x0 as i64 && y1 > y0 as i64).then(|| Region { x: x0, y: y0, w: (x1 - x0 as i64) as u32, h: (y1 - y0 as i64) as u32 })
    }
}

/// A capture of part of the desktop; `x, y` is the screen position of its
/// top-left pixel.
#[derive(Debug, Clone, Default)]
pub struct Screenshot {
    pub x: i32,
    pub y: i32,
    pub image: Image,
}

/// Something found on screen (coordinates of the capture it was found in).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Similarity, 0..=1.
    pub score: f64,
}

impl Found {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w as i32 / 2, self.y + self.h as i32 / 2)
    }
    pub fn offset(self, dx: i32, dy: i32) -> Found {
        Found { x: self.x + dx, y: self.y + dy, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FindOptions {
    /// Minimum similarity, 0..=1 (0.9 is a good default).
    pub threshold: f64,
    /// Also try the image smaller and larger (screens with another scaling).
    pub any_scale: bool,
    /// Matches returned at most (best first).
    pub max_results: usize,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self { threshold: 0.9, any_scale: false, max_results: 1 }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search {
    /// Matches at or above the threshold, best first, not overlapping.
    pub matches: Vec<Found>,
    /// The most similar place, even below the threshold (to tune it).
    pub best: Option<Found>,
}

/// Size factors tried by `any_scale`, closest to 1 first. They cover the
/// usual display scalings (100, 125, 150, 175, 200 %) against each other.
const SCALES: [f64; 9] = [1.0, 0.8, 1.25, 0.667, 1.5, 0.571, 1.75, 0.5, 2.0];

/// Finds `needle` in `hay`.
pub fn find_image(hay: &Image, needle: &Image, o: &FindOptions) -> Search {
    let mut out = Search::default();
    let scales: &[f64] = if o.any_scale { &SCALES } else { &SCALES[..1] };
    for &s in scales {
        let scaled;
        let n = if s == 1.0 {
            needle
        } else {
            let (w, h) = ((needle.w as f64 * s).round() as u32, (needle.h as f64 * s).round() as u32);
            if w < 4 || h < 4 {
                continue;
            }
            scaled = needle.resize(w, h);
            &scaled
        };
        let r = find_at_scale(hay, n, o);
        if let Some(b) = r.best
            && out.best.is_none_or(|ob| b.score > ob.score)
        {
            out.best = Some(b);
        }
        if !r.matches.is_empty() {
            out.matches = r.matches;
            break;
        }
    }
    out
}

/// Luma plane, integer values 0..=255 stored as u8.
struct Plane {
    w: usize,
    h: usize,
    v: Vec<u8>,
}

impl Plane {
    fn luma(img: &Image) -> Plane {
        let v = img
            .rgba
            .as_chunks::<4>().0.iter()
            .map(|p| ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128) >> 8) as u8)
            .collect();
        Plane { w: img.w as usize, h: img.h as usize, v }
    }

    /// 3×3 binomial blur: makes the coarse scan tolerant of sub-pixel
    /// shifts (an image rarely sits on the pyramid's pixel grid).
    fn blur(&self) -> Plane {
        let (w, h) = (self.w, self.h);
        let at = |x: isize, y: isize| self.v[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize] as u32;
        let mut v = vec![0u8; w * h];
        for y in 0..h as isize {
            for x in 0..w as isize {
                let s = at(x - 1, y - 1) + 2 * at(x, y - 1) + at(x + 1, y - 1)
                    + 2 * at(x - 1, y) + 4 * at(x, y) + 2 * at(x + 1, y)
                    + at(x - 1, y + 1) + 2 * at(x, y + 1) + at(x + 1, y + 1);
                v[y as usize * w + x as usize] = ((s + 8) / 16) as u8;
            }
        }
        Plane { w, h, v }
    }

    /// The plane without a border of `b` pixels.
    fn inner(&self, b: usize) -> Plane {
        let (w, h) = (self.w - 2 * b, self.h - 2 * b);
        let mut v = Vec::with_capacity(w * h);
        for y in b..b + h {
            v.extend_from_slice(&self.v[y * self.w + b..][..w]);
        }
        Plane { w, h, v }
    }

    fn half(&self) -> Plane {
        let (w, h) = (self.w / 2, self.h / 2);
        let mut v = vec![0u8; w * h];
        for y in 0..h {
            let (r0, r1) = (&self.v[2 * y * self.w..][..self.w], &self.v[(2 * y + 1) * self.w..][..self.w]);
            for x in 0..w {
                v[y * w + x] = ((r0[2 * x] as u16 + r0[2 * x + 1] as u16 + r1[2 * x] as u16 + r1[2 * x + 1] as u16 + 2) / 4) as u8;
            }
        }
        Plane { w, h, v }
    }
}

/// The searched image at one pyramid level, ready for correlation.
struct Tpl {
    w: usize,
    h: usize,
    /// `(t - mean) · m` per pixel.
    z: Vec<f32>,
    /// Pixel weights (1 or 0); `None` when every pixel counts.
    m: Option<Vec<f32>>,
    /// Number of counted pixels.
    n: f64,
    mean: f64,
    /// `sqrt(Σ z²)`; ~0 for a uniform image.
    norm: f64,
}

impl Tpl {
    fn new(p: &Plane, mask: Option<&Vec<f32>>) -> Tpl {
        let weight = |i: usize| mask.map_or(1.0, |m| m[i]);
        let n: f64 = (0..p.v.len()).map(|i| weight(i) as f64).sum::<f64>().max(1.0);
        let mean = (0..p.v.len()).map(|i| p.v[i] as f64 * weight(i) as f64).sum::<f64>() / n;
        let z: Vec<f32> = (0..p.v.len()).map(|i| ((p.v[i] as f64 - mean) * weight(i) as f64) as f32).collect();
        let norm = z.iter().map(|&v| v as f64 * v as f64).sum::<f64>().sqrt();
        Tpl { w: p.w, h: p.h, z, m: mask.cloned(), n, mean, norm }
    }

    fn flat(&self) -> bool {
        self.norm < 0.5
    }

    /// Similarity of the window of `hay` whose top-left corner is (x, y).
    fn score(&self, hay: &Plane, x: usize, y: usize) -> f32 {
        let (mut s1, mut s2, mut cross) = (0f64, 0f64, 0f64);
        for ty in 0..self.h {
            let row = &hay.v[(y + ty) * hay.w + x..][..self.w];
            let zrow = &self.z[ty * self.w..][..self.w];
            let (mut a, mut b, mut c) = (0f32, 0f32, 0f32);
            match &self.m {
                None => {
                    for (&i, &z) in row.iter().zip(zrow) {
                        let i = i as f32;
                        a += i;
                        b += i * i;
                        c += z * i;
                    }
                }
                Some(m) => {
                    for ((&i, &z), &w) in row.iter().zip(zrow).zip(&m[ty * self.w..][..self.w]) {
                        let i = i as f32;
                        a += w * i;
                        b += w * i * i;
                        c += z * i;
                    }
                }
            }
            s1 += a as f64;
            s2 += b as f64;
            cross += c as f64;
        }
        let n = self.n;
        if self.flat() {
            // A uniform image: compare levels instead of shapes.
            let sse = (s2 - 2.0 * self.mean * s1 + n * self.mean * self.mean).max(0.0);
            let rms = (sse / n).sqrt();
            return (1.0 - rms / 255.0 * 4.0).max(0.0) as f32;
        }
        let var = s2 - s1 * s1 / n;
        if var <= n * 0.25 {
            return 0.0; // uniform window, textured image: no match
        }
        (cross / (self.norm * var.sqrt())) as f32
    }
}

fn level_mask(img: &Image, levels: usize) -> Vec<Option<Vec<f32>>> {
    if !img.has_transparency() {
        return vec![None; levels + 1];
    }
    let (mut w, mut h) = (img.w as usize, img.h as usize);
    let mut cur: Vec<f32> = img.rgba.as_chunks::<4>().0.iter().map(|p| if p[3] >= 128 { 1.0 } else { 0.0 }).collect();
    let mut out = vec![Some(cur.clone())];
    for _ in 0..levels {
        let (nw, nh) = (w / 2, h / 2);
        let mut next = vec![0f32; nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let s = cur[2 * y * w + 2 * x] + cur[2 * y * w + 2 * x + 1] + cur[(2 * y + 1) * w + 2 * x] + cur[(2 * y + 1) * w + 2 * x + 1];
                next[y * nw + x] = if s >= 2.0 { 1.0 } else { 0.0 };
            }
        }
        cur = next;
        (w, h) = (nw, nh);
        out.push(Some(cur.clone()));
    }
    out
}

/// Colour similarity at full resolution (per-channel normalised
/// correlation; level difference for uniform images).
fn color_score(hay: &Image, needle: &Image, x: u32, y: u32) -> f64 {
    let mut sum = [[0f64; 2]; 3]; // per channel: Σt, Σi
    let mut n = 0f64;
    for ty in 0..needle.h {
        for tx in 0..needle.w {
            let t = needle.pixel(tx, ty);
            if t[3] < 128 {
                continue;
            }
            let i = hay.pixel(x + tx, y + ty);
            for c in 0..3 {
                sum[c][0] += t[c] as f64;
                sum[c][1] += i[c] as f64;
            }
            n += 1.0;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    let mt = [sum[0][0] / n, sum[1][0] / n, sum[2][0] / n];
    let mi = [sum[0][1] / n, sum[1][1] / n, sum[2][1] / n];
    let (mut tt, mut ii, mut ti, mut sq) = (0f64, 0f64, 0f64, 0f64);
    for ty in 0..needle.h {
        for tx in 0..needle.w {
            let t = needle.pixel(tx, ty);
            if t[3] < 128 {
                continue;
            }
            let i = hay.pixel(x + tx, y + ty);
            for c in 0..3 {
                let (dt, di) = (t[c] as f64 - mt[c], i[c] as f64 - mi[c]);
                tt += dt * dt;
                ii += di * di;
                ti += dt * di;
                let d = t[c] as f64 - i[c] as f64;
                sq += d * d;
            }
        }
    }
    if tt.sqrt() < 0.5 * (3.0 * n).sqrt() {
        // Uniform colour: how close the colours are.
        let rms = (sq / (3.0 * n)).sqrt();
        return (1.0 - rms / 255.0 * 4.0).max(0.0);
    }
    if ii <= 0.25 * 3.0 * n {
        return 0.0;
    }
    ti / (tt.sqrt() * ii.sqrt())
}

fn find_at_scale(hay: &Image, needle: &Image, o: &FindOptions) -> Search {
    let mut out = Search::default();
    if needle.is_empty() || needle.w > hay.w || needle.h > hay.h {
        return out;
    }
    // Pyramid depth: the coarse needle keeps at least 8 px on its short side.
    let short = needle.w.min(needle.h) as usize;
    let mut levels = 0;
    while levels < 4 && short >> (levels + 1) >= 8 {
        levels += 1;
    }
    let mut hp = vec![Plane::luma(hay)];
    let mut np = vec![Plane::luma(needle)];
    for l in 0..levels {
        hp.push(hp[l].half());
        np.push(np[l].half());
    }
    let masks = level_mask(needle, levels);
    let tpls: Vec<Tpl> = (0..=levels).map(|l| Tpl::new(&np[l], masks[l].as_ref())).collect();

    // 1. Full scan of the coarsest level, in parallel bands of rows. The
    //    image rarely sits on the pyramid's pixel grid: at the coarse level
    //    its edge pixels then mix with whatever surrounds it. So the coarse
    //    scan is softened (blur) and ignores a 1-pixel border of the image.
    let (nw, nh) = (np[levels].w, np[levels].h);
    if nw > hp[levels].w || nh > hp[levels].h {
        return out;
    }
    let blurred;
    let (top, tpl, b): (&Plane, Tpl, usize) = if levels > 0 {
        blurred = hp[levels].blur();
        let mask = masks[levels].as_ref().map(|m| {
            let p = Plane { w: nw, h: nh, v: m.iter().map(|&v| v as u8).collect() }.inner(1);
            p.v.iter().map(|&v| v as f32).collect::<Vec<f32>>()
        });
        (&blurred, Tpl::new(&np[levels].blur().inner(1), mask.as_ref()), 1)
    } else {
        (&hp[0], Tpl::new(&np[0], masks[0].as_ref()), 0)
    };
    let tpl = &tpl;
    let (pw, ph) = (top.w + 1 - nw, top.h + 1 - nh);
    let mut map = vec![0f32; pw * ph];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
    let band = ph.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (bi, chunk) in map.chunks_mut(band * pw).enumerate() {
            s.spawn(move || {
                for (r, row) in chunk.chunks_mut(pw).enumerate() {
                    let y = bi * band + r;
                    for (x, v) in row.iter_mut().enumerate() {
                        *v = tpl.score(top, x + b, y + b);
                    }
                }
            });
        }
    });

    // 2. Candidates: local maxima above a looser threshold.
    let coarse = if levels == 0 { o.threshold - 0.15 } else { o.threshold - 0.3 }.max(0.25) as f32;
    let mut cands: Vec<(f32, usize, usize)> = Vec::new();
    for y in 0..ph {
        for x in 0..pw {
            let v = map[y * pw + x];
            if v < coarse {
                continue;
            }
            let mut peak = true;
            'n: for ny in y.saturating_sub(1)..=(y + 1).min(ph - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(pw - 1) {
                    let u = map[ny * pw + nx];
                    if u > v || (u == v && (ny, nx) < (y, x)) {
                        peak = false;
                        break 'n;
                    }
                }
            }
            if peak {
                cands.push((v, x, y));
            }
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    cands.truncate(64 + o.max_results * 4);

    // 3. Refine each candidate down to full resolution, then score in colour.
    let mut found: Vec<Found> = Vec::new();
    for &(_, cx, cy) in &cands {
        let (mut x, mut y) = (cx, cy);
        let mut gray = 0f32;
        for l in (0..levels).rev() {
            let (p, t) = (&hp[l], &tpls[l]);
            let (maxx, maxy) = (p.w - t.w, p.h - t.h);
            let (bx, by) = ((x * 2).min(maxx), (y * 2).min(maxy));
            let mut best = (f32::MIN, bx, by);
            for ny in by.saturating_sub(2)..=(by + 2).min(maxy) {
                for nx in bx.saturating_sub(2)..=(bx + 2).min(maxx) {
                    let v = t.score(p, nx, ny);
                    if v > best.0 {
                        best = (v, nx, ny);
                    }
                }
            }
            (gray, x, y) = best;
        }
        if levels == 0 {
            gray = tpls[0].score(&hp[0], x, y);
        }
        let color = color_score(hay, needle, x as u32, y as u32);
        let score = (gray as f64).min(color).clamp(0.0, 1.0);
        found.push(Found { x: x as i32, y: y as i32, w: needle.w, h: needle.h, score });
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.best = found.first().copied();

    // 4. Keep the best of overlapping matches.
    for f in found.into_iter().filter(|f| f.score >= o.threshold) {
        if out.matches.len() >= o.max_results.max(1) {
            break;
        }
        let overlaps = out.matches.iter().any(|m| {
            let ix = (m.x + m.w as i32).min(f.x + f.w as i32) - m.x.max(f.x);
            let iy = (m.y + m.h as i32).min(f.y + f.h as i32) - m.y.max(f.y);
            ix > 0 && iy > 0 && (ix as i64 * iy as i64) * 2 > f.w as i64 * f.h as i64
        });
        if !overlaps {
            out.matches.push(f);
        }
    }
    out
}

/// First pixel (row by row, from the top left) within `tolerance` of the
/// colour on every channel.
pub fn find_color(hay: &Image, color: (u8, u8, u8), tolerance: u8) -> Option<Found> {
    let t = tolerance as i16;
    let (r, g, b) = (color.0 as i16, color.1 as i16, color.2 as i16);
    hay.rgba.as_chunks::<4>().0.iter().position(|p| (p[0] as i16 - r).abs() <= t && (p[1] as i16 - g).abs() <= t && (p[2] as i16 - b).abs() <= t).map(|i| {
        let p = &hay.rgba[i * 4..i * 4 + 3];
        let d = (p[0] as i16 - r).abs().max((p[1] as i16 - g).abs()).max((p[2] as i16 - b).abs());
        Found { x: (i % hay.w as usize) as i32, y: (i / hay.w as usize) as i32, w: 1, h: 1, score: 1.0 - d as f64 / 255.0 }
    })
}

/// A word read on screen by character recognition (screen pixels).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Index of its line, in reading order.
    pub line: u32,
}

/// Finds `query` (one or several words) in recognised text. A phrase must
/// sit on one line; spaces are normalised, case is ignored unless
/// `match_case`. The result covers the words of the first occurrence.
pub fn find_text(words: &[Word], query: &str, match_case: bool) -> Option<Found> {
    let norm = |s: &str| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if match_case { s } else { s.to_lowercase() }
    };
    let q = norm(query);
    if q.is_empty() {
        return None;
    }
    let mut lines: Vec<(u32, Vec<&Word>)> = Vec::new();
    for w in words {
        match lines.iter_mut().find(|(l, _)| *l == w.line) {
            Some((_, v)) => v.push(w),
            None => lines.push((w.line, vec![w])),
        }
    }
    for (_, ws) in &lines {
        let mut text = String::new();
        let mut spans = Vec::new();
        for w in ws {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(&norm(&w.text));
            spans.push((start, text.len(), *w));
        }
        let Some(at) = text.find(&q) else { continue };
        let end = at + q.len();
        let hit: Vec<&Word> = spans.iter().filter(|(s, e, _)| *s < end && *e > at).map(|(_, _, w)| *w).collect();
        let x0 = hit.iter().map(|w| w.x).min()?;
        let y0 = hit.iter().map(|w| w.y).min()?;
        let x1 = hit.iter().map(|w| w.x + w.w as i32).max()?;
        let y1 = hit.iter().map(|w| w.y + w.h as i32).max()?;
        return Some(Found { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32, score: 1.0 });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic "desktop": gradients, noise and a few shapes.
    fn desktop(w: u32, h: u32, seed: u32) -> Image {
        let mut s = seed.wrapping_mul(2_654_435_761).max(1);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let noise = (s % 24) as u8;
                let r = ((x * 255 / w) as u8).wrapping_add(noise);
                let g = ((y * 255 / h) as u8).wrapping_add(noise / 2);
                let b = (((x / 7 + y / 5) % 64) * 3) as u8;
                px.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Image::new(w, h, px)
    }

    /// An "icon": a ring with a cross, in the given colour.
    fn icon(size: u32, c: [u8; 3]) -> Image {
        let mut px = Vec::new();
        let m = size as f32 / 2.0;
        for y in 0..size {
            for x in 0..size {
                let d = ((x as f32 - m).powi(2) + (y as f32 - m).powi(2)).sqrt();
                let ring = (d - m * 0.7).abs() < m * 0.15;
                let cross = (x as f32 - m).abs() < 2.0 || (y as f32 - m).abs() < 2.0;
                px.extend_from_slice(&if ring {
                    [c[0], c[1], c[2], 255]
                } else if cross && d < m * 0.5 {
                    [c[2], c[0], c[1], 255]
                } else {
                    [20, 24, 32, 255]
                });
            }
        }
        Image::new(size, size, px)
    }

    fn paste(dst: &mut Image, src: &Image, x: u32, y: u32) {
        for ty in 0..src.h {
            for tx in 0..src.w {
                let p = src.pixel(tx, ty);
                if p[3] < 128 {
                    continue;
                }
                let i = (((y + ty) * dst.w + x + tx) * 4) as usize;
                dst.rgba[i..i + 4].copy_from_slice(&p);
            }
        }
    }

    #[test]
    fn finds_an_icon_where_it_is() {
        let mut d = desktop(800, 500, 7);
        let ic = icon(40, [250, 200, 60]);
        paste(&mut d, &ic, 523, 311);
        let r = find_image(&d, &ic, &FindOptions::default());
        let m = r.matches.first().expect("found");
        assert_eq!((m.x, m.y), (523, 311));
        assert!(m.score > 0.99, "{}", m.score);
    }

    #[test]
    fn found_at_every_offset_of_the_coarse_grid() {
        // The coarse scan works on 4×4 blocks: every sub-block position must
        // still be found (the bug this guards against missed odd offsets).
        let ic = icon(40, [220, 40, 40]);
        for dy in 0..4 {
            for dx in 0..4 {
                let mut d = desktop(400, 260, 31 + dx + 4 * dy);
                paste(&mut d, &ic, 200 + dx, 120 + dy);
                let r = find_image(&d, &ic, &FindOptions::default());
                assert_eq!(r.matches.first().map(|m| (m.x, m.y)), Some((200 + dx as i32, 120 + dy as i32)), "offset ({dx}, {dy}): best {:?}", r.best);
            }
        }
    }

    #[test]
    fn finds_a_crop_of_the_screen_itself() {
        // The usual case: the image was snipped from a capture of the screen.
        let d = desktop(640, 400, 3);
        let snip = d.crop(201, 97, 57, 31);
        let r = find_image(&d, &snip, &FindOptions::default());
        let m = r.matches.first().expect("found");
        assert_eq!((m.x, m.y, m.w, m.h), (201, 97, 57, 31));
    }

    #[test]
    fn absent_image_is_not_found_but_best_is_reported() {
        let d = desktop(600, 400, 11);
        let ic = icon(32, [250, 200, 60]);
        let r = find_image(&d, &ic, &FindOptions::default());
        assert!(r.matches.is_empty());
        assert!(r.best.is_none_or(|b| b.score < 0.9));
    }

    #[test]
    fn same_shape_in_another_colour_does_not_match() {
        let mut d = desktop(600, 400, 5);
        paste(&mut d, &icon(40, [40, 200, 90]), 100, 100); // green on screen
        let wanted = icon(40, [220, 40, 40]); // red wanted
        let r = find_image(&d, &wanted, &FindOptions::default());
        assert!(r.matches.is_empty(), "{:?}", r.matches);
        // …but it is found when the right colour appears.
        paste(&mut d, &wanted, 400, 250);
        let r = find_image(&d, &wanted, &FindOptions::default());
        assert_eq!(r.matches.first().map(|m| (m.x, m.y)), Some((400, 250)), "best: {:?}", r.best);
    }

    #[test]
    fn tolerates_brightness_changes() {
        let mut d = desktop(500, 300, 9);
        let ic = icon(36, [200, 160, 40]);
        let mut dim = ic.clone();
        for p in dim.rgba.as_chunks_mut::<4>().0 {
            for c in &mut p[..3] {
                *c = (*c as f32 * 0.8) as u8;
            }
        }
        paste(&mut d, &dim, 50, 60);
        let r = find_image(&d, &ic, &FindOptions { threshold: 0.9, ..Default::default() });
        assert_eq!(r.matches.first().map(|m| (m.x, m.y)), Some((50, 60)));
    }

    #[test]
    fn several_copies_are_listed_best_first_without_overlap() {
        let mut d = desktop(700, 400, 13);
        let ic = icon(30, [240, 240, 240]);
        for (x, y) in [(40, 40), (300, 200), (600, 330)] {
            paste(&mut d, &ic, x, y);
        }
        let r = find_image(&d, &ic, &FindOptions { max_results: 10, ..Default::default() });
        let mut at: Vec<_> = r.matches.iter().map(|m| (m.x, m.y)).collect();
        at.sort();
        assert_eq!(at, vec![(40, 40), (300, 200), (600, 330)]);
    }

    #[test]
    fn transparent_pixels_are_ignored() {
        // A ring with a transparent background, pasted on two backgrounds.
        let mut ring = icon(40, [250, 200, 60]);
        for p in ring.rgba.as_chunks_mut::<4>().0 {
            if p[0] == 20 {
                p[3] = 0;
            }
        }
        let mut d = desktop(600, 300, 17);
        paste(&mut d, &ring, 420, 130);
        let r = find_image(&d, &ring, &FindOptions { threshold: 0.85, ..Default::default() });
        assert_eq!(r.matches.first().map(|m| (m.x, m.y)), Some((420, 130)));
    }

    #[test]
    fn other_display_scaling_is_found_with_any_scale() {
        let mut d = desktop(800, 500, 21);
        let ic = icon(40, [250, 200, 60]);
        paste(&mut d, &ic.resize(50, 50), 300, 200); // shown at 125 %
        let strict = find_image(&d, &ic, &FindOptions::default());
        assert!(strict.matches.is_empty());
        let r = find_image(&d, &ic, &FindOptions { any_scale: true, threshold: 0.8, ..Default::default() });
        let m = r.matches.first().expect("found at another scale");
        assert!((m.x - 300).abs() <= 2 && (m.y - 200).abs() <= 2, "{m:?}");
        assert_eq!((m.w, m.h), (50, 50));
    }

    #[test]
    fn small_and_uniform_images_work() {
        let mut d = desktop(300, 200, 23);
        // A solid 6×6 patch of a colour absent from the desktop.
        let patch = Image::new(6, 6, [10, 250, 10, 255].repeat(36));
        paste(&mut d, &patch, 120, 80);
        let r = find_image(&d, &patch, &FindOptions::default());
        assert_eq!(r.matches.first().map(|m| (m.x, m.y)), Some((120, 80)));
    }

    #[test]
    fn needle_larger_than_screen_is_not_found() {
        let d = desktop(50, 50, 1);
        let r = find_image(&d, &desktop(60, 40, 2), &FindOptions::default());
        assert_eq!(r, Search::default());
    }

    #[test]
    fn colour_search_returns_the_first_matching_pixel() {
        let mut d = Image::new(10, 5, [0, 0, 0, 255].repeat(50));
        for (x, y) in [(7, 1), (2, 3)] {
            let i = (y * 10 + x) * 4;
            d.rgba[i..i + 3].copy_from_slice(&[200, 100, 50]);
        }
        let f = find_color(&d, (205, 98, 50), 6).unwrap();
        assert_eq!((f.x, f.y), (7, 1));
        assert!(find_color(&d, (205, 98, 50), 2).is_none());
    }

    #[test]
    fn text_is_found_by_phrase_on_one_line() {
        let w = |text: &str, x: i32, line: u32| Word { text: text.into(), x, y: 10 + line as i32 * 30, w: text.len() as u32 * 8, h: 14, line };
        let words = vec![w("Save", 10, 0), w("as…", 50, 0), w("Cancel", 10, 1), w("Save", 200, 2), w("changes", 240, 2)];
        // Case-insensitive, extra spaces ignored: the second "save" line matches.
        let f = find_text(&words, "  save   CHANGES ", false).unwrap();
        assert_eq!((f.x, f.y, f.w, f.h), (200, 70, 96, 14));
        // A single word: the first occurrence in reading order.
        assert_eq!(find_text(&words, "save", false).map(|f| (f.x, f.y)), Some((10, 10)));
        // Part of a word counts (\"ancel\" in \"Cancel\").
        assert!(find_text(&words, "ancel", false).is_some());
        assert!(find_text(&words, "cancel", true).is_none());
        // A phrase never spans two lines.
        assert!(find_text(&words, "as… cancel", false).is_none());
        assert!(find_text(&words, "   ", false).is_none());
    }

    #[test]
    fn png_round_trip_keeps_pixels_and_alpha() {
        let mut ic = icon(17, [1, 2, 3]);
        ic.rgba[3] = 0;
        let back = Image::from_png(&ic.to_png()).unwrap();
        assert_eq!(back, ic);
        assert!(Image::from_png(b"not a png").is_err());
    }

    #[test]
    fn crop_and_resize_stay_in_bounds() {
        let d = desktop(20, 10, 4);
        assert_eq!(d.crop(15, 5, 10, 10).w, 5);
        assert_eq!(d.crop(-5, -5, 10, 10).w, 5);
        assert!(d.crop(30, 0, 5, 5).is_empty());
        let r = d.resize(7, 3);
        assert_eq!((r.w, r.h, r.rgba.len()), (7, 3, 84));
    }
}

