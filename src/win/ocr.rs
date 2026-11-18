//! Windows' built-in OCR engine (Windows.Media.Ocr, offline), image preparation, and chat-area detection.

use super::capture::Frame;
use regex::Regex;
use std::sync::OnceLock;
use windows::core::{Interface, HSTRING};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;

#[derive(Clone, Debug)]
pub struct Row {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct Ocr {
    engine: OcrEngine,
}

impl Ocr {
    /// None when Windows has no English OCR language installed.
    pub fn new() -> Option<Ocr> {
        let lang = Language::CreateLanguage(&HSTRING::from("en-US")).ok()?;
        let engine = OcrEngine::TryCreateFromLanguage(&lang).or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages()).ok()?;
        Some(Ocr { engine })
    }

    pub fn max_dimension() -> usize { OcrEngine::MaxImageDimension().unwrap_or(2600) as usize }

    pub fn read_rows(&self, f: &Frame) -> windows::core::Result<Vec<Row>> {
        let bmp = SoftwareBitmap::Create(BitmapPixelFormat::Bgra8, f.w as i32, f.h as i32)?;
        {
            let buffer = bmp.LockBuffer(BitmapBufferAccessMode::Write)?;
            let plane = buffer.GetPlaneDescription(0)?;
            let reference = buffer.CreateReference()?;
            let access: IMemoryBufferByteAccess = reference.cast()?;
            let (mut ptr, mut cap) = (std::ptr::null_mut::<u8>(), 0u32);
            unsafe { access.GetBuffer(&mut ptr, &mut cap)? };
            let dst = unsafe { std::slice::from_raw_parts_mut(ptr, cap as usize) };
            let (stride, start) = (plane.Stride as usize, plane.StartIndex as usize);
            for row in 0..f.h {
                let d = start + row * stride;
                dst[d..d + f.w * 4].copy_from_slice(&f.px[row * f.w * 4..(row + 1) * f.w * 4]);
            }
            drop(access);
            let _ = reference.Close();
            let _ = buffer.Close();
        }
        let result = self.engine.RecognizeAsync(&bmp)?.get()?;
        let mut words = Vec::new();
        for line in result.Lines()? {
            for w in line.Words()? {
                let r = w.BoundingRect()?;
                words.push(Row { text: w.Text()?.to_string(), x: r.X as f64, y: r.Y as f64, w: r.Width as f64, h: r.Height as f64 });
            }
        }
        let _ = bmp.Close();
        Ok(group_rows(words))
    }
}

/// Rebuilds visual rows from word positions. Windows OCR splits "00:36 Player (Talon):" and
/// "Galio used Flash" into separate lines once chat has several lines, losing the timestamp.
pub fn group_rows(mut words: Vec<Row>) -> Vec<Row> {
    if words.is_empty() { return words; }
    let mut heights: Vec<f64> = words.iter().map(|w| w.h).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let tol = (heights[heights.len() / 2] * 0.5).max(2.0);
    words.sort_by(|a, b| (a.y + a.h / 2.0).partial_cmp(&(b.y + b.h / 2.0)).unwrap());
    let mut rows: Vec<(f64, Vec<Row>)> = Vec::new();
    for w in words {
        let cy = w.y + w.h / 2.0;
        match rows.last_mut() {
            Some((rcy, ws)) if (*rcy - cy).abs() <= tol => {
                ws.push(w);
                *rcy = ws.iter().map(|x| x.y + x.h / 2.0).sum::<f64>() / ws.len() as f64;
            }
            _ => rows.push((cy, vec![w])),
        }
    }
    rows.into_iter().map(|(_, mut ws)| {
        ws.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        let x0 = ws.iter().map(|w| w.x).fold(f64::MAX, f64::min);
        let y0 = ws.iter().map(|w| w.y).fold(f64::MAX, f64::min);
        let x1 = ws.iter().map(|w| w.x + w.w).fold(f64::MIN, f64::max);
        let y1 = ws.iter().map(|w| w.y + w.h).fold(f64::MIN, f64::max);
        Row { text: ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "), x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }).collect()
}

/// Enemy names are red, which OCR reads far worse than white text: strongly red pixels become white.
pub fn whiten_red(f: &Frame) -> Frame {
    let mut px = f.px.clone();
    for p in px.chunks_exact_mut(4) {
        let (b, g, r) = (p[0] as i32, p[1] as i32, p[2] as i32);
        let m = g.max(b);
        if r >= 120 && r - m >= 45 {
            let v = (r + (r - m)).min(255) as u8;
            p[0] = v; p[1] = v; p[2] = v;
        }
    }
    Frame { px, w: f.w, h: f.h }
}

/// Nearest-neighbour upscale; small game text reads far better at 2x.
pub fn upscale(f: &Frame, k: usize) -> Frame {
    if k <= 1 { return f.clone(); }
    let (w, h) = (f.w * k, f.h * k);
    let mut px = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = y / k;
        for x in 0..w {
            let s = (sy * f.w + x / k) * 4;
            let d = (y * w + x) * 4;
            px[d..d + 4].copy_from_slice(&f.px[s..s + 4]);
        }
    }
    Frame { px, w, h }
}

/// Fingerprint of the text-like pixels only (bright white/blue/gold glyphs and the red enemy names),
/// sampled on a 2x2 grid. The game map behind the chat changes every frame, but the chat text only
/// changes when a new line appears or lines fade, so OCR runs a handful of times a minute instead
/// of every scan.
pub fn text_fingerprint(f: &Frame) -> u64 {
    let mut hash: u64 = 1469598103934665603;
    let mut bits: u64 = 0;
    let mut n = 0;
    for y in (0..f.h).step_by(2) {
        for x in (0..f.w).step_by(2) {
            let i = (y * f.w + x) * 4;
            let (b, g, r) = (f.px[i] as i32, f.px[i + 1] as i32, f.px[i + 2] as i32);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let text = (max >= 185 && max - min < 70) || (r >= 150 && r - g.max(b) >= 60) || (max >= 200 && b >= 200);
            bits = (bits << 1) | text as u64;
            n += 1;
            if n == 64 {
                hash ^= bits;
                hash = hash.wrapping_mul(1099511628211);
                bits = 0;
                n = 0;
            }
        }
    }
    hash ^ bits
}


fn crop(f: &Frame, x: usize, y: usize, w: usize, h: usize) -> Frame {
    let mut px = vec![0u8; w * h * 4];
    for r in 0..h {
        let s = ((y + r) * f.w + x) * 4;
        px[r * w * 4..(r + 1) * w * 4].copy_from_slice(&f.px[s..s + w * 4]);
    }
    Frame { px, w, h }
}

/// Finds the chat box in a full frame of the game window from rows that start with a chat timestamp,
/// and returns a region around them with room for older and longer lines. Needs chat text on screen.
pub fn detect_chat_area(ocr: &Ocr, full: &Frame) -> Result<([f64; 4], usize), String> {
    static TS: OnceLock<Regex> = OnceLock::new();
    let ts = TS.get_or_init(|| Regex::new(r"^\W{0,3}\d{1,2}\s?[:;.,/]\s?\d\s?\d(?:\D|$)").unwrap());
    let scale = 2;
    let max = Ocr::max_dimension() / scale;
    let (tw, th, overlap) = (640.min(max), 480.min(max), 60);
    let prepared = whiten_red(full);
    let mut rows = Vec::new();
    let mut ty = 0;
    while ty < full.h {
        let mut tx = 0;
        while tx < full.w {
            let (w, h) = (tw.min(full.w - tx), th.min(full.h - ty));
            if w >= 40 && h >= 20 {
                let tile = upscale(&crop(&prepared, tx, ty, w, h), scale);
                for r in ocr.read_rows(&tile).map_err(|e| e.to_string())? {
                    if ts.is_match(&r.text) {
                        rows.push(Row { x: tx as f64 + r.x / scale as f64, y: ty as f64 + r.y / scale as f64, w: r.w / scale as f64, h: r.h / scale as f64, ..r });
                    }
                }
            }
            tx += tw - overlap;
        }
        ty += th - overlap;
    }
    // tiles overlap, so the same row may appear twice
    rows.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap());
    let mut unique: Vec<Row> = Vec::new();
    for r in rows {
        if !unique.iter().any(|u| (u.y - r.y).abs() < 6.0 && (u.x - r.x).abs() < 40.0) { unique.push(r); }
    }
    if unique.len() < 2 {
        return Err("No chat lines found. Open the chat history in game (press Enter) and try again.".into());
    }
    // chat rows share a left edge; keep the biggest group
    let mut best: Vec<&Row> = Vec::new();
    for r in &unique {
        let group: Vec<&Row> = unique.iter().filter(|u| (u.x - r.x).abs() < 30.0).collect();
        if group.len() > best.len() { best = group; }
    }
    let mut hs: Vec<f64> = best.iter().map(|r| r.h).collect();
    hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let line_h = hs[hs.len() / 2];
    let left = best.iter().map(|r| r.x).fold(f64::MAX, f64::min) - line_h;
    let right = (best.iter().map(|r| r.x + r.w).fold(f64::MIN, f64::max) + 3.0 * line_h).max(left + full.w as f64 * 0.22);
    let top = best.iter().map(|r| r.y).fold(f64::MAX, f64::min) - 4.0 * line_h;
    let bottom = best.iter().map(|r| r.y + r.h).fold(f64::MIN, f64::max) + 2.0 * line_h;
    let (fw, fh) = (full.w as f64, full.h as f64);
    let (fx, fy) = ((left / fw).clamp(0.0, 1.0), (top / fh).clamp(0.0, 1.0));
    let (fr, fb) = ((right / fw).clamp(0.0, 1.0), (bottom / fh).clamp(0.0, 1.0));
    let round = |v: f64| (v * 10000.0).round() / 10000.0;
    Ok(([round(fx), round(fy), round(fr - fx), round(fb - fy)], best.len()))
}

/// Loads a PNG/JPEG as BGRA (used by the --ocr-test diagnostic).
pub fn load_image(path: &std::path::Path) -> windows::core::Result<Frame> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder};
    use windows::Storage::{FileAccessMode, StorageFile};
    // canonicalize() adds a \\?\ prefix that StorageFile rejects
    let full = path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(full))?.get()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.get()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let bmp = decoder.GetSoftwareBitmapConvertedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)?.get()?;
    let buffer = bmp.LockBuffer(BitmapBufferAccessMode::Read)?;
    let plane = buffer.GetPlaneDescription(0)?;
    let reference = buffer.CreateReference()?;
    let access: IMemoryBufferByteAccess = reference.cast()?;
    let (mut ptr, mut cap) = (std::ptr::null_mut::<u8>(), 0u32);
    unsafe { access.GetBuffer(&mut ptr, &mut cap)? };
    let src = unsafe { std::slice::from_raw_parts(ptr, cap as usize) };
    let (w, h, stride, start) = (plane.Width as usize, plane.Height as usize, plane.Stride as usize, plane.StartIndex as usize);
    let mut px = vec![0u8; w * h * 4];
    for r in 0..h { px[r * w * 4..(r + 1) * w * 4].copy_from_slice(&src[start + r * stride..start + r * stride + w * 4]); }
    Ok(Frame { px, w, h })
}

pub fn crop_region(f: &Frame, r: [f64; 4]) -> Frame {
    let x = ((f.w as f64 * r[0]) as usize).min(f.w);
    let y = ((f.h as f64 * r[1]) as usize).min(f.h);
    let w = ((f.w as f64 * r[2]) as usize).min(f.w - x);
    let h = ((f.h as f64 * r[3]) as usize).min(f.h - y);
    crop(f, x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Frame {
        let mut px = vec![255u8; w * h * 4];
        for y in 0..h { for x in 0..w { let [r, g, b] = f(x, y); let i = (y * w + x) * 4; px[i] = b; px[i + 1] = g; px[i + 2] = r; } }
        Frame { px, w, h }
    }

    #[test]
    fn text_fingerprint_ignores_the_map_but_sees_new_text() {
        // dark greenish "map" that changes, with a white text block that stays
        let text = |x: usize, y: usize| (10..40).contains(&x) && (4..10).contains(&y);
        let a = frame(64, 32, |x, y| if text(x, y) { [235, 235, 235] } else { [40, 70 + (x % 7) as u8, 30] });
        let b = frame(64, 32, |x, y| if text(x, y) { [235, 235, 235] } else { [60, 90 + (y % 5) as u8 * 3, 45] });
        assert_eq!(text_fingerprint(&a), text_fingerprint(&b)); // camera moved: no OCR
        let c = frame(64, 32, |x, y| if text(x, y) || ((10..30).contains(&x) && (20..26).contains(&y)) { [235, 235, 235] } else { [40, 70, 30] });
        assert_ne!(text_fingerprint(&a), text_fingerprint(&c)); // a new chat line: OCR
        let red = frame(64, 32, |x, y| if text(x, y) { [235, 235, 235] } else if (40..50).contains(&x) && (4..10).contains(&y) { [220, 60, 60] } else { [40, 70, 30] });
        assert_ne!(text_fingerprint(&a), text_fingerprint(&red)); // red enemy name counts as text
    }
}
