//! data/ryukaze.svg, drawn at the sizes the tray and the window ask for.

use resvg::{tiny_skia, usvg};

const SVG: &[u8] = include_bytes!("../../../data/ryukaze.svg");

/// Straight (not premultiplied) RGBA rows, `size` pixels square.
pub fn rgba(size: u32) -> Vec<u8> {
    let tree =
        usvg::Tree::from_data(SVG, &usvg::Options::default()).expect("data/ryukaze.svg parses");
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("a non-zero size");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}

/// For StatusNotifierItem: ARGB32 in network byte order, a few sizes for
/// the host to pick from.
pub fn tray() -> Vec<ksni::Icon> {
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|size| ksni::Icon {
            width: size as i32,
            height: size as i32,
            data: rgba(size)
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|&[r, g, b, a]| [a, r, g, b])
                .collect(),
        })
        .collect()
}

pub fn window() -> Option<iced::window::Icon> {
    iced::window::icon::from_rgba(rgba(64), 64, 64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_the_fan() {
        let pixels = rgba(32);
        assert_eq!(pixels.len(), 32 * 32 * 4);
        // The middle is the white hub on the black housing; the corners
        // are outside the round housing, so empty.
        let at = |x: usize, y: usize| &pixels[(y * 32 + x) * 4..][..4];
        assert_eq!(at(16, 16), [0xfa, 0xfa, 0xfa, 0xff]);
        assert_eq!(at(16, 4)[..3], [0x09, 0x09, 0x0b]);
        assert_eq!(at(0, 0)[3], 0);
    }
}
