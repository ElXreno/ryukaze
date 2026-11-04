//! The keys on a device's picture: GG draws a key's outline, and its
//! legend, clear or half clear through an opaque picture, so a key is an
//! opaque patch ringed by clearer pixels.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use png::{ColorType, Transformations};

/// A picture's pixels, by how opaque and how bright each is.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    alpha: Vec<u8>,
    light: Vec<u8>,
}

/// How much darker than the most of a region a key drawn on it is.
const DARKER: u8 = 16;

/// How opaque a pixel must be to be the picture rather than an outline:
/// some pictures draw them clear, some half clear.
pub const SOLID: [u8; 2] = [128, 255];

/// A key's patch, in the picture's pixels, its right and bottom edges
/// outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patch {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Patch {
    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }

    pub fn centre(&self) -> (f32, f32) {
        (
            (self.x0 + self.x1) as f32 / 2.0,
            (self.y0 + self.y1) as f32 / 2.0,
        )
    }

    fn encloses(&self, other: &Patch) -> bool {
        self != other
            && self.x0 <= other.x0
            && self.y0 <= other.y0
            && other.x1 <= self.x1
            && other.y1 <= self.y1
    }
}

/// A PNG or WebP picture.
pub fn decode(bytes: &[u8]) -> Result<Picture> {
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return webp(bytes);
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("not a PNG")?;
    let size = reader
        .output_buffer_size()
        .context("a PNG too large to read")?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).context("a broken PNG")?;
    let (width, height) = (info.width as usize, info.height as usize);
    let (channels, alpha) = match info.color_type {
        ColorType::Rgba => (4, Some(3)),
        ColorType::GrayscaleAlpha => (2, Some(1)),
        ColorType::Rgb => (3, None),
        ColorType::Grayscale => (1, None),
        ColorType::Indexed => bail!("a PNG left indexed"),
    };
    let pixel = |at: usize| {
        let (row, column) = (at / width, at % width);
        &buffer[row * info.line_size + column * channels..][..channels]
    };
    let alpha = (0..width * height)
        .map(|at| alpha.map_or(u8::MAX, |alpha| pixel(at)[alpha]))
        .collect();
    let light = (0..width * height)
        .map(|at| light(&pixel(at)[..if channels >= 3 { 3 } else { 1 }]))
        .collect();
    Ok(Picture {
        width,
        height,
        alpha,
        light,
    })
}

fn webp(bytes: &[u8]) -> Result<Picture> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).context("not a WebP")?;
    let (width, height) = decoder.dimensions();
    let (width, height) = (width as usize, height as usize);
    let size = decoder
        .output_buffer_size()
        .context("a WebP too large to read")?;
    let mut buffer = vec![0; size];
    decoder.read_image(&mut buffer).context("a broken WebP")?;
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let pixels = buffer.chunks(channels);
    let alpha = pixels
        .clone()
        .map(|pixel| pixel.get(3).copied().unwrap_or(u8::MAX))
        .collect();
    let light = pixels.map(|pixel| light(&pixel[..3])).collect();
    Ok(Picture {
        width,
        height,
        alpha,
        light,
    })
}

/// A pixel's brightness, its channels' mean.
fn light(channels: &[u8]) -> u8 {
    (channels.iter().map(|&c| u32::from(c)).sum::<u32>() / channels.len() as u32) as u8
}

impl Picture {
    #[cfg(test)]
    pub fn new(width: usize, height: usize, solid: Vec<bool>) -> Self {
        assert_eq!(solid.len(), width * height);
        Self {
            width,
            height,
            alpha: solid
                .into_iter()
                .map(|solid| if solid { 255 } else { 0 })
                .collect(),
            light: vec![100; width * height],
        }
    }

    /// The opaque patches of the picture, at least `solid` opaque.
    fn patches(&self, solid: u8) -> Vec<Patch> {
        let opaque: Vec<bool> = self.alpha.iter().map(|&alpha| alpha >= solid).collect();
        self.runs(&opaque)
            .into_iter()
            .map(|(patch, _)| patch)
            .collect()
    }

    /// The patches of `opaque` pixels, each as its own: four-way connected
    /// runs of them, with how many pixels each has.
    fn runs(&self, opaque: &[bool]) -> Vec<(Patch, usize)> {
        let mut seen = vec![false; opaque.len()];
        let mut patches = Vec::new();
        let mut stack = Vec::new();
        for start in 0..opaque.len() {
            if seen[start] || !opaque[start] {
                continue;
            }
            seen[start] = true;
            stack.push(start);
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            let mut area = 0;
            while let Some(at) = stack.pop() {
                area += 1;
                let (x, y) = (at % self.width, at / self.width);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                let mut visit = |next: usize| {
                    if !seen[next] && opaque[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                };
                if x > 0 {
                    visit(at - 1);
                }
                if x + 1 < self.width {
                    visit(at + 1);
                }
                if y > 0 {
                    visit(at - self.width);
                }
                if y + 1 < self.height {
                    visit(at + self.width);
                }
            }
            patches.push((
                Patch {
                    x0: x0 as u32,
                    y0: y0 as u32,
                    x1: x1 as u32,
                    y1: y1 as u32,
                },
                area,
            ));
        }
        patches
    }

    /// The parts of a picture of several, such as a laptop's lid and its
    /// deck side by side: opaque patches of a twentieth of it at least,
    /// the largest first.
    pub fn parts(&self) -> Vec<Patch> {
        let opaque: Vec<bool> = self.alpha.iter().map(|&alpha| alpha >= SOLID[0]).collect();
        let mut parts = self.runs(&opaque);
        parts.retain(|(_, area)| area * 20 >= self.alpha.len());
        parts.sort_by_key(|(_, area)| std::cmp::Reverse(*area));
        parts.into_iter().map(|(patch, _)| patch).collect()
    }

    /// The keys drawn on a part of the picture, darker than most of it:
    /// patches of such pixels about as tall as the most of them.
    pub fn dark_keys(&self, part: &Patch) -> Vec<Patch> {
        let inside = |at: usize| {
            let (x, y) = ((at % self.width) as u32, (at / self.width) as u32);
            part.x0 <= x && x < part.x1 && part.y0 <= y && y < part.y1 && self.alpha[at] >= SOLID[0]
        };
        let mut counts = [0usize; 256];
        for at in (0..self.light.len()).filter(|&at| inside(at)) {
            counts[usize::from(self.light[at])] += 1;
        }
        let Some(most) = (0..=u8::MAX).max_by_key(|&light| counts[usize::from(light)]) else {
            return Vec::new();
        };
        let dark: Vec<bool> = (0..self.light.len())
            .map(|at| inside(at) && self.light[at].saturating_add(DARKER) < most)
            .collect();
        let least = (part.width() / 80).max(3);
        let mut keys: Vec<Patch> = self
            .runs(&dark)
            .into_iter()
            .map(|(patch, _)| patch)
            .filter(|patch| patch.width() >= least && patch.height() >= least)
            .collect();
        let mut heights: Vec<u32> = keys.iter().map(Patch::height).collect();
        heights.sort_unstable();
        let Some(&usual) = heights.get(heights.len() / 2) else {
            return keys;
        };
        keys.retain(|patch| patch.height() * 2 >= usual && patch.height() <= usual * 3);
        keys
    }

    /// The keys, ringed by pixels less than `solid` opaque: patches the
    /// size of a key at least, leaving out the picture around them, which
    /// holds many, and what's within a key, such as a letter's inside.
    pub fn keys(&self, solid: u8) -> Vec<Patch> {
        let least = (self.width.max(self.height) / 60).max(4) as u32;
        let big: Vec<Patch> = self
            .patches(solid)
            .into_iter()
            .filter(|patch| patch.width() >= least && patch.height() >= least)
            .collect();
        let keys: Vec<Patch> = big
            .iter()
            .filter(|patch| big.iter().filter(|other| patch.encloses(other)).count() < 3)
            .copied()
            .collect();
        let mut keys: Vec<Patch> = keys
            .iter()
            .filter(|patch| !keys.iter().any(|other| other.encloses(patch)))
            .copied()
            .collect();
        keys.sort_by_key(|patch| (patch.y0, patch.x0));
        keys
    }
}

/// A picture of keys as GG draws them, for tests: each key's outline
/// clear, a pixel thick, and a clear pixel within for a legend.
#[cfg(test)]
pub fn draw(width: usize, height: usize, keys: &[Patch]) -> Picture {
    let mut solid = vec![true; width * height];
    let mut clear = |x: u32, y: u32| solid[y as usize * width + x as usize] = false;
    for key in keys {
        for x in key.x0 - 1..=key.x1 {
            clear(x, key.y0 - 1);
            clear(x, key.y1);
        }
        for y in key.y0 - 1..=key.y1 {
            clear(key.x0 - 1, y);
            clear(key.x1, y);
        }
        let (x, y) = key.centre();
        clear(x as u32, y as u32);
    }
    Picture::new(width, height, solid)
}

/// The same picture as a PNG.
#[cfg(test)]
pub fn encode(picture: &Picture) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, picture.width as u32, picture.height as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let pixels: Vec<u8> = picture
        .alpha
        .iter()
        .flat_map(|&alpha| [40, 40, 40, alpha])
        .collect();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(x0: u32, y0: u32, x1: u32, y1: u32) -> Patch {
        Patch { x0, y0, x1, y1 }
    }

    #[test]
    fn finds_the_keys() {
        let keys = [
            patch(10, 10, 40, 40),
            patch(45, 10, 75, 40),
            patch(80, 10, 140, 40),
            patch(10, 45, 75, 75),
            // Touching another's outline, as GG's Enter and backslash do.
            patch(77, 45, 140, 75),
        ];
        let picture = decode(&encode(&draw(160, 90, &keys))).unwrap();
        assert_eq!((picture.width, picture.height), (160, 90));
        assert_eq!(picture.keys(SOLID[0]), keys);
    }

    #[test]
    fn leaves_out_specks_and_letters() {
        let keys = [
            patch(10, 10, 60, 60),
            patch(65, 10, 115, 60),
            patch(120, 10, 170, 60),
        ];
        let mut picture = draw(180, 80, &keys);
        // A letter's ring within the first key, its inside opaque.
        for x in 20..=40 {
            picture.alpha[20 * 180 + x] = 0;
            picture.alpha[40 * 180 + x] = 0;
        }
        for y in 20..=40 {
            picture.alpha[y * 180 + 20] = 0;
            picture.alpha[y * 180 + 40] = 0;
        }
        // A speck outside.
        picture.alpha[70 * 180 + 70] = 0;
        picture.alpha[70 * 180 + 72] = 0;
        assert_eq!(picture.keys(SOLID[0]), keys);
    }

    #[test]
    fn finds_keys_ringed_half_clear() {
        let keys = [
            patch(10, 10, 40, 40),
            patch(45, 10, 75, 40),
            patch(80, 10, 110, 40),
        ];
        let mut picture = draw(120, 50, &keys);
        for alpha in &mut picture.alpha {
            if *alpha == 0 {
                *alpha = 200;
            }
        }
        assert_ne!(picture.keys(SOLID[0]), keys);
        assert_eq!(picture.keys(SOLID[1]), keys);
    }

    #[test]
    fn reads_a_webp() {
        let keys = [
            patch(10, 10, 40, 40),
            patch(45, 10, 75, 40),
            patch(80, 10, 110, 40),
        ];
        let picture = draw(120, 50, &keys);
        let pixels: Vec<u8> = picture
            .alpha
            .iter()
            .flat_map(|&alpha| [40, 40, 40, alpha])
            .collect();
        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes)
            .encode(&pixels, 120, 50, image_webp::ColorType::Rgba8)
            .unwrap();
        let picture = decode(&bytes).unwrap();
        assert_eq!(picture.keys(SOLID[0]), keys);
    }

    #[test]
    fn refuses_what_isnt_a_picture() {
        assert!(decode(b"GIF89a").is_err());
        assert!(decode(b"RIFF\0\0\0\0WEBPVP8 ").is_err());
    }
}
