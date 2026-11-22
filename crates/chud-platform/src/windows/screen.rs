//! Reading the screen: capture (GDI) and character recognition (the OCR
//! engine built into Windows 10 and 11, offline, in the user's languages).
//! Nothing captured leaves the process.

use chud_core::vision::{Image, Region, Screenshot, Word};
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GdiFlush, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::{PlatformError, Result};

/// The whole virtual desktop (every screen), physical pixels.
pub fn desktop() -> Region {
    unsafe {
        Region {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(0) as u32,
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(0) as u32,
        }
    }
}

/// Copies part of the desktop. Areas between screens of different sizes
/// come out black. Content protected by its application (DRM video, some
/// password fields) is black too: Windows never shows it to other programs.
pub fn capture(region: Option<Region>) -> Result<Screenshot> {
    let desk = desktop();
    let r = match region {
        Some(r) => r.intersect(&desk).ok_or_else(|| PlatformError::System("the search area is outside every screen".into()))?,
        None => desk,
    };
    let (w, h) = (r.w as i32, r.h as i32);
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(PlatformError::System("no access to the screen".into()));
        }
        let mem = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        let result = match dib {
            Ok(dib) if !bits.is_null() => {
                let old = SelectObject(mem, HGDIOBJ(dib.0));
                // No CAPTUREBLT: it makes the mouse pointer flicker on every
                // capture, and the composited desktop already includes
                // layered windows.
                let copied = BitBlt(mem, 0, 0, w, h, Some(screen), r.x, r.y, SRCCOPY);
                let _ = GdiFlush();
                let px = std::slice::from_raw_parts(bits as *const u8, w as usize * h as usize * 4).to_vec();
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(dib.0));
                copied.map(|_| px).map_err(|e| PlatformError::System(format!("screen copy failed: {e}")))
            }
            Ok(dib) => {
                let _ = DeleteObject(HGDIOBJ(dib.0));
                Err(PlatformError::System("screen buffer unavailable".into()))
            }
            Err(e) => Err(PlatformError::System(format!("screen buffer unavailable: {e}"))),
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        Ok(Screenshot { x: r.x, y: r.y, image: Image::from_bgra(r.w, r.h, result?) })
    }
}

fn ocr_err(e: windows::core::Error) -> PlatformError {
    PlatformError::System(format!("text recognition failed: {}", e.message()))
}

/// The OCR engine for the user's languages, if Windows has one.
fn engine() -> Result<OcrEngine> {
    OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
        PlatformError::Unsupported("text recognition: no OCR language is installed (Windows Settings › Time & language › Language)".into())
    })
}

pub fn ocr_available() -> bool {
    engine().is_ok()
}

/// Recognises the words of a capture. Larger than the engine accepts is
/// read in overlapping tiles; a word seen twice is kept once.
pub fn read_text(shot: &Screenshot) -> Result<Vec<Word>> {
    let engine = engine()?;
    if shot.image.is_empty() {
        return Ok(Vec::new());
    }
    let max = OcrEngine::MaxImageDimension().unwrap_or(2600).max(200);
    let img = &shot.image;
    let overlap = 120.min(max / 4);
    let step = max - overlap;
    let mut words: Vec<Word> = Vec::new();
    let mut line_base = 0u32;
    let mut ty = 0;
    loop {
        let mut tx = 0;
        loop {
            let (tw, th) = (max.min(img.w - tx), max.min(img.h - ty));
            let tile = img.crop(tx as i32, ty as i32, tw, th);
            let mut bgra = tile.rgba;
            for p in bgra.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
            }
            let buffer = CryptographicBuffer::CreateFromByteArray(&bgra).map_err(ocr_err)?;
            let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, tw as i32, th as i32).map_err(ocr_err)?;
            let result = engine.RecognizeAsync(&bitmap).map_err(ocr_err)?.join().map_err(ocr_err)?;
            let lines = result.Lines().map_err(ocr_err)?;
            let mut n = 0;
            for line in lines {
                for word in line.Words().map_err(ocr_err)? {
                    let r = word.BoundingRect().map_err(ocr_err)?;
                    let w = Word {
                        text: word.Text().map_err(ocr_err)?.to_string_lossy(),
                        x: shot.x + tx as i32 + r.X.round() as i32,
                        y: shot.y + ty as i32 + r.Y.round() as i32,
                        w: r.Width.round().max(1.0) as u32,
                        h: r.Height.round().max(1.0) as u32,
                        line: line_base + n,
                    };
                    let seen = words.iter().any(|o| {
                        o.text == w.text && (o.x - w.x).abs() <= 3 && (o.y - w.y).abs() <= 3
                    });
                    if !seen {
                        words.push(w);
                    }
                }
                n += 1;
            }
            line_base += n;
            if tx + tw >= img.w {
                break;
            }
            tx += step;
        }
        if ty + max.min(img.h - ty) >= img.h {
            break;
        }
        ty += step;
    }
    Ok(words)
}
