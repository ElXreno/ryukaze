//! The firmware's boot logo, from ACPI's BGRT table as Linux gives it
//! (`/sys/firmware/acpi/bgrt/image`, a BMP anyone may read): on MSI
//! laptops the shield on the lid, for the picture's logo, in its light's
//! colour.

use std::{
    collections::HashMap,
    fs,
    sync::{Mutex, OnceLock},
};

use iced::widget::image::Handle;

const BGRT: &str = "/sys/firmware/acpi/bgrt/image";

/// The longest side the logo is kept at.
const SIDE: usize = 160;

/// The logo cut to its shape: how much of each pixel is the logo, and how
/// bright it is there, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Logo {
    pub width: usize,
    pub height: usize,
    /// (coverage, brightness), each 0 to 1.
    pixels: Vec<(f32, f32)>,
}

/// This laptop's; none without a BGRT or one that doesn't read.
pub fn read() -> Option<Logo> {
    Logo::from_bmp(&fs::read(BGRT).ok()?)
}

impl Logo {
    /// An uncompressed 24 or 32 bit BMP, cut to what isn't black around it
    /// and made no bigger than `SIDE`.
    fn from_bmp(bmp: &[u8]) -> Option<Logo> {
        let u16_at = |at: usize| Some(u16::from_le_bytes(bmp.get(at..at + 2)?.try_into().ok()?));
        let u32_at = |at: usize| Some(u32::from_le_bytes(bmp.get(at..at + 4)?.try_into().ok()?));
        if bmp.get(..2)? != b"BM" {
            return None;
        }
        let offset = u32_at(10)? as usize;
        let width = i32::try_from(u32_at(18)?).ok()?;
        let height = u32_at(22)? as i32;
        let bits = u16_at(28)?;
        let compression = u32_at(30)?;
        let bytes = match (bits, compression) {
            (24, 0) => 3,
            (32, 0 | 3) => 4,
            _ => return None,
        };
        let (width, rows) = (
            width.unsigned_abs() as usize,
            height.unsigned_abs() as usize,
        );
        if width == 0 || rows == 0 {
            return None;
        }
        let stride = (width * bytes).div_ceil(4) * 4;
        // Bottom up unless the height says otherwise.
        let pixel = |x: usize, y: usize| -> Option<[u8; 3]> {
            let row = if height > 0 { rows - 1 - y } else { y };
            let at = offset + row * stride + x * bytes;
            let [b, g, r] = bmp.get(at..at + 3)?.try_into().ok()?;
            Some([r, g, b])
        };

        // What isn't black: the logo, on the firmware's black screen.
        const DARK: u8 = 24;
        let lit = |[r, g, b]: [u8; 3]| r.max(g).max(b) > DARK;
        let (mut left, mut top, mut right, mut bottom) = (width, rows, 0, 0);
        for y in 0..rows {
            for x in 0..width {
                if lit(pixel(x, y)?) {
                    (left, top) = (left.min(x), top.min(y));
                    (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
                }
            }
        }
        if right <= left || bottom <= top {
            return None;
        }

        let (cut_width, cut_height) = (right - left, bottom - top);
        let step = cut_width.max(cut_height).div_ceil(SIDE).max(1);
        let (out_width, out_height) = (cut_width.div_ceil(step), cut_height.div_ceil(step));
        let mut pixels = Vec::with_capacity(out_width * out_height);
        for oy in 0..out_height {
            for ox in 0..out_width {
                // A box of `step` by `step`, averaged.
                let (mut coverage, mut brightness, mut count) = (0.0, 0.0, 0.0);
                for y in top + oy * step..(top + (oy + 1) * step).min(bottom) {
                    for x in left + ox * step..(left + (ox + 1) * step).min(right) {
                        let [r, g, b] = pixel(x, y)?;
                        let max = f32::from(r.max(g).max(b));
                        coverage += ((max - f32::from(DARK)) / 48.0).clamp(0.0, 1.0);
                        brightness +=
                            (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b))
                                / 255.0;
                        count += 1.0;
                    }
                }
                pixels.push((coverage / count, brightness / count));
            }
        }
        Some(Logo {
            width: out_width,
            height: out_height,
            pixels,
        })
    }

    /// The logo lit in `color`: its brighter parts brighter, as a lit logo
    /// shows its drawing; cached by colour.
    pub fn tinted(&self, color: [u8; 3]) -> Handle {
        static CACHE: OnceLock<Mutex<HashMap<[u8; 3], Handle>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Mutex::default);
        if let Ok(cache) = cache.lock()
            && let Some(handle) = cache.get(&color)
        {
            return handle.clone();
        }
        let handle = Handle::from_rgba(self.width as u32, self.height as u32, self.rgba(color));
        if let Ok(mut cache) = cache.lock() {
            // The picture shows few colours at once; past that, start over.
            if cache.len() > 64 {
                cache.clear();
            }
            cache.insert(color, handle.clone());
        }
        handle
    }

    fn rgba(&self, color: [u8; 3]) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&(coverage, brightness)| {
                let shade = 0.45 + 0.55 * brightness;
                let [r, g, b] = color.map(|channel| (f32::from(channel) * shade).round() as u8);
                [r, g, b, (coverage * 255.0).round() as u8]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BMP of `width` by `height`, bottom up, 24 bits, `lit` white on
    /// black.
    fn bmp(width: usize, height: usize, lit: impl Fn(usize, usize) -> bool) -> Vec<u8> {
        let stride = (width * 3).div_ceil(4) * 4;
        let mut data = vec![0u8; 54 + stride * height];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes());
        data[18..22].copy_from_slice(&(width as u32).to_le_bytes());
        data[22..26].copy_from_slice(&(height as u32).to_le_bytes());
        data[28..30].copy_from_slice(&24u16.to_le_bytes());
        for y in 0..height {
            for x in 0..width {
                if lit(x, y) {
                    let row = height - 1 - y;
                    let at = 54 + row * stride + x * 3;
                    data[at..at + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
        }
        data
    }

    #[test]
    fn cuts_the_logo_out_of_the_black() {
        // A 4 by 3 block at (5, 2) on a 20 by 10 screen.
        let logo = Logo::from_bmp(&bmp(20, 10, |x, y| {
            (5..9).contains(&x) && (2..5).contains(&y)
        }))
        .unwrap();
        assert_eq!((logo.width, logo.height), (4, 3));
        assert!(logo.pixels.iter().all(|&(coverage, _)| coverage == 1.0));
        let rgba = logo.rgba([255, 0, 0]);
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn keeps_a_big_logo_small() {
        let logo = Logo::from_bmp(&bmp(1000, 500, |x, y| x > 100 && x < 900 && y > 50)).unwrap();
        assert!(logo.width <= SIDE && logo.height <= SIDE);
        assert!(logo.width > logo.height);
    }

    #[test]
    fn refuses_what_isnt_a_plain_bmp() {
        assert_eq!(Logo::from_bmp(b"PNG"), None);
        assert_eq!(Logo::from_bmp(&bmp(10, 10, |_, _| false)), None);
        let mut rle = bmp(10, 10, |_, _| true);
        rle[30..34].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(Logo::from_bmp(&rle), None);
    }
}
