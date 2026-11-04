//! A colour picker: saturation across and value down a square of the hue,
//! and the hue in a bar beside it, both to drag. A colour goes out as it's
//! dragged, and once more when let go, for what's slow to follow.

use iced::{
    Color, Point, Rectangle, Renderer, Size, Theme, mouse,
    widget::canvas::{self, Frame, Geometry, Path, Stroke, gradient},
};

use crate::theme;

/// How wide the hue's bar is, and how far from the square.
const BAR: f32 = 22.0;
const GAP: f32 = 12.0;
const RADIUS: f32 = 6.0;

pub struct Picker<'a, Message> {
    pub color: [u8; 3],
    /// The hue the square shows, 0 to 360: kept by its owner, as a grey
    /// has none of its own.
    pub hue: f32,
    /// A colour picked, and the hue it was picked in.
    pub on_pick: Box<dyn Fn([u8; 3], f32) -> Message + 'a>,
    pub on_release: Message,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Dragging {
    #[default]
    Nothing,
    Square,
    Hue,
}

/// The square, and the hue's bar right of it.
fn areas(bounds: Size) -> (Rectangle, Rectangle) {
    let square = Rectangle::new(
        Point::ORIGIN,
        Size::new((bounds.width - BAR - GAP).max(1.0), bounds.height),
    );
    let bar = Rectangle::new(
        Point::new(square.width + GAP, 0.0),
        Size::new(BAR, bounds.height),
    );
    (square, bar)
}

impl<Message> Picker<'_, Message> {
    /// What a point picks, dragging `what`: off the edge, the edge.
    fn pick(&self, what: Dragging, size: Size, at: Point) -> Option<([u8; 3], f32)> {
        let (square, bar) = areas(size);
        let along = |value: f32, from: f32, length: f32| ((value - from) / length).clamp(0.0, 1.0);
        match what {
            Dragging::Nothing => None,
            Dragging::Square => {
                let saturation = along(at.x, square.x, square.width);
                let value = 1.0 - along(at.y, square.y, square.height);
                Some((rgb(self.hue, saturation, value), self.hue))
            }
            Dragging::Hue => {
                let hue = along(at.y, bar.y, bar.height) * 360.0;
                let (_, saturation, value) = hsv(self.color);
                Some((rgb(hue, saturation, value), hue))
            }
        }
    }
}

impl<Message: Clone> canvas::Program<Message> for Picker<'_, Message> {
    type State = Dragging;

    fn update(
        &self,
        state: &mut Dragging,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        let local = |at: Point| Point::new(at.x - bounds.x, at.y - bounds.y);
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let at = cursor.position_in(bounds)?;
                let (square, bar) = areas(bounds.size());
                *state = if square.contains(at) {
                    Dragging::Square
                } else if bar.contains(at) {
                    Dragging::Hue
                } else {
                    return None;
                };
                let (color, hue) = self.pick(*state, bounds.size(), at)?;
                Some(canvas::Action::publish((self.on_pick)(color, hue)).and_capture())
            }
            // Past the edge too: dragged out, it stays at the edge.
            mouse::Event::CursorMoved { .. } => {
                let (color, hue) = self.pick(*state, bounds.size(), local(cursor.position()?))?;
                Some(canvas::Action::publish((self.on_pick)(color, hue)).and_capture())
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if *state != Dragging::Nothing => {
                *state = Dragging::Nothing;
                Some(canvas::Action::publish(self.on_release.clone()).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Dragging,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let colors = theme::colors(theme);
        let mut frame = Frame::new(renderer, bounds.size());
        let (square, bar) = areas(bounds.size());
        let rounded =
            |area: Rectangle| Path::rounded_rectangle(area.position(), area.size(), RADIUS.into());

        // The hue at full, white over it to the left, black over both down.
        let full = rgb(self.hue, 1.0, 1.0);
        frame.fill(&rounded(square), led(full));
        frame.fill(
            &rounded(square),
            gradient::Linear::new(
                Point::new(square.x, 0.0),
                Point::new(square.x + square.width, 0.0),
            )
            .add_stop(0.0, Color::WHITE)
            .add_stop(1.0, Color::TRANSPARENT),
        );
        frame.fill(
            &rounded(square),
            gradient::Linear::new(
                Point::new(0.0, square.y),
                Point::new(0.0, square.y + square.height),
            )
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::BLACK),
        );

        let mut hues =
            gradient::Linear::new(Point::new(0.0, bar.y), Point::new(0.0, bar.y + bar.height));
        for i in 0..=6 {
            let at = i as f32 / 6.0;
            hues = hues.add_stop(at, led(rgb(at * 360.0, 1.0, 1.0)));
        }
        frame.fill(&rounded(bar), hues);
        for area in [square, bar] {
            frame.stroke(
                &rounded(area),
                Stroke::default().with_color(colors.border).with_width(1.0),
            );
        }

        // Where the colour is: a ring in the square, a band across the bar.
        let (_, saturation, value) = hsv(self.color);
        let ring = Point::new(
            square.x + saturation * square.width,
            square.y + (1.0 - value) * square.height,
        );
        let band = Rectangle::new(
            Point::new(bar.x - 2.0, bar.y + self.hue / 360.0 * bar.height - 3.0),
            Size::new(bar.width + 4.0, 6.0),
        );
        for (width, color) in [(3.0, Color::BLACK), (1.5, Color::WHITE)] {
            let stroke = Stroke::default().with_color(color).with_width(width);
            frame.stroke(&Path::circle(ring, 7.0), stroke);
            frame.stroke(
                &Path::rounded_rectangle(band.position(), band.size(), 3.0.into()),
                stroke,
            );
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Dragging,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let (square, bar) = areas(bounds.size());
        match cursor.position_in(bounds) {
            _ if *state != Dragging::Nothing => mouse::Interaction::Grabbing,
            Some(at) if square.contains(at) || bar.contains(at) => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}

fn led([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgb8(r, g, b)
}

/// Hue 0 to 360, saturation and value 0 to 1.
pub fn hsv([r, g, b]: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = [r, g, b].map(|channel| f32::from(channel) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

pub fn rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let hue = hue.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = value - chroma;
    [r, g, b].map(|channel| ((channel + m) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goes_round_between_rgb_and_hsv() {
        for color in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 200, 0],
            [250, 179, 135],
            [18, 52, 86],
            [128, 128, 128],
            [0, 0, 0],
            [255, 255, 255],
        ] {
            let (hue, saturation, value) = hsv(color);
            assert_eq!(rgb(hue, saturation, value), color, "{color:?}");
        }
        assert_eq!(hsv([255, 0, 0]), (0.0, 1.0, 1.0));
        let (hue, ..) = hsv([0, 0, 255]);
        assert!((hue - 240.0).abs() < 1e-3);
        assert_eq!(rgb(360.0, 1.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn picks_where_it_is_dragged() {
        let picker = Picker {
            color: [255, 0, 0],
            hue: 0.0,
            on_pick: Box::new(|color, hue| (color, hue)),
            on_release: ([0; 3], 0.0),
        };
        let size = Size::new(200.0 + BAR + GAP, 100.0);
        // Top right of the square: the hue at full; bottom: black.
        assert_eq!(
            picker.pick(Dragging::Square, size, Point::new(200.0, 0.0)),
            Some(([255, 0, 0], 0.0))
        );
        assert_eq!(
            picker.pick(Dragging::Square, size, Point::new(50.0, 100.0)),
            Some(([0, 0, 0], 0.0))
        );
        // Top left: white; off the edge, the edge.
        assert_eq!(
            picker.pick(Dragging::Square, size, Point::new(-30.0, -30.0)),
            Some(([255, 255, 255], 0.0))
        );
        // Half way down the bar: cyan, at the colour's own saturation.
        let (color, hue) = picker
            .pick(Dragging::Hue, size, Point::new(220.0, 50.0))
            .unwrap();
        assert_eq!((color, hue), ([0, 255, 255], 180.0));
        assert_eq!(picker.pick(Dragging::Nothing, size, Point::ORIGIN), None);
    }
}
