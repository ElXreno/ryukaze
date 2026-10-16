//! The advanced fan mode's curve as a graph to drag: one dot where each
//! step starts, sideways for its temperature, up or down for its speed. The
//! line is drawn as the fans will take it: steps when the EC goes through
//! the curve itself, straight between the dots when ryukazed adapts.

use iced::{
    Point, Rectangle, Renderer, Theme, mouse,
    widget::canvas::{self, Frame, Geometry, Path, Stroke, Text},
};
use ryukaze_core::fans::{Curve, Fan};

use crate::{app::Message, theme};

const TEMP_MIN: f32 = 20.0;
const TEMP_MAX: f32 = 100.0;
const SPEED_MAX: f32 = 100.0;

const PAD_LEFT: f32 = 36.0;
const PAD_RIGHT: f32 = 10.0;
const PAD_TOP: f32 = 10.0;
const PAD_BOTTOM: f32 = 22.0;

/// How close the pointer has to be to grab a dot.
const GRAB_RADIUS: f32 = 12.0;

pub struct Editor {
    pub curve: Curve,
    /// The temperature the EC reads and the speed it runs at now.
    pub now: Option<(u8, u8)>,
    /// The daemon adapts: the fans go straight between the dots.
    pub smooth: bool,
}

#[derive(Default)]
pub struct Grabbed {
    handle: Option<usize>,
}

/// Where dot `handle` sits: step 0 starts at the left edge, step `i` at
/// `temps[i - 1]`.
fn handle_at(curve: &Curve, handle: usize) -> (f32, f32) {
    let temp = match handle {
        0 => TEMP_MIN,
        i => f32::from(curve.temps[i - 1]),
    };
    (temp, f32::from(curve.speeds[handle]))
}

/// `curve` with dot `handle` dragged to `temp` and `speed`, kept in
/// order: a step can't start before the one below it or after the one
/// above, nor run slower than the one below or faster than the one above.
pub fn drag(curve: Curve, handle: usize, temp: f32, speed: f32) -> Curve {
    let mut curve = curve;
    let speeds = curve.speeds;
    let below = if handle == 0 { 0 } else { speeds[handle - 1] };
    let above = match speeds.get(handle + 1) {
        Some(&above) => above,
        // The top step may sit above the graph if the config put it there.
        None => speeds[6].max(SPEED_MAX as u8),
    };
    curve.speeds[handle] = within(speed.round().clamp(0.0, 255.0) as u8, below, above);

    if handle > 0 {
        let step = handle - 1;
        let temps = curve.temps;
        let after = if step == 0 {
            TEMP_MIN as u8
        } else {
            temps[step - 1]
        };
        let before = temps.get(step + 1).copied().unwrap_or(TEMP_MAX as u8 + 1);
        let temp = temp.round().clamp(0.0, 255.0) as u8;
        curve.temps[step] = within(temp, after.saturating_add(1), before.saturating_sub(1));
    }
    curve
}

/// `value` between `low` and `high`, `high` winning should they cross.
fn within(value: u8, low: u8, high: u8) -> u8 {
    value.max(low).min(high)
}

/// The curves as they'd go into `services.ryukaze.settings`.
pub fn as_nix(curves: &[(Fan, Curve)]) -> String {
    let list = |values: &[u8]| {
        values
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut nix = String::from("fans = {\n");
    for (fan, curve) in curves {
        nix += &format!(
            "  {fan} = {{\n    temps = [ {} ];\n    speeds = [ {} ];\n  }};\n",
            list(&curve.temps),
            list(&curve.speeds)
        );
    }
    nix + "};\n"
}

impl Editor {
    fn plot(bounds: Rectangle) -> Rectangle {
        Rectangle {
            x: PAD_LEFT,
            y: PAD_TOP,
            width: (bounds.width - PAD_LEFT - PAD_RIGHT).max(1.0),
            height: (bounds.height - PAD_TOP - PAD_BOTTOM).max(1.0),
        }
    }

    fn to_screen(bounds: Rectangle, temp: f32, speed: f32) -> Point {
        let plot = Self::plot(bounds);
        let x = ((temp - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)).clamp(0.0, 1.0);
        let y = (speed / SPEED_MAX).clamp(0.0, 1.0);
        Point::new(plot.x + x * plot.width, plot.y + (1.0 - y) * plot.height)
    }

    fn to_values(bounds: Rectangle, at: Point) -> (f32, f32) {
        let plot = Self::plot(bounds);
        let x = ((at.x - plot.x) / plot.width).clamp(0.0, 1.0);
        let y = 1.0 - ((at.y - plot.y) / plot.height).clamp(0.0, 1.0);
        (TEMP_MIN + x * (TEMP_MAX - TEMP_MIN), y * SPEED_MAX)
    }

    fn nearest(&self, bounds: Rectangle, at: Point) -> Option<usize> {
        (0..7)
            .map(|handle| {
                let (temp, speed) = handle_at(&self.curve, handle);
                (handle, Self::to_screen(bounds, temp, speed).distance(at))
            })
            .filter(|&(_, distance)| distance <= GRAB_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(handle, _)| handle)
    }
}

impl canvas::Program<Message> for Editor {
    type State = Grabbed;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else {
            return None;
        };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                state.handle = Some(self.nearest(bounds, cursor.position_in(bounds)?)?);
                Some(canvas::Action::request_redraw().and_capture())
            }
            // Past the edge too: a dot flicked to the top should get there.
            mouse::Event::CursorMoved { .. } => {
                let handle = state.handle?;
                let at = cursor.position()?;
                let local = Point::new(at.x - bounds.x, at.y - bounds.y);
                let (temp, speed) = Self::to_values(bounds, local);
                let curve = drag(self.curve, handle, temp, speed);
                Some(canvas::Action::publish(Message::CurveDragged(curve)).and_capture())
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                state.handle.take()?;
                Some(canvas::Action::publish(Message::CurveReleased).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let colors = theme::colors(theme);
        let mut frame = Frame::new(renderer, bounds.size());
        let plot = Self::plot(bounds);
        let label = |frame: &mut Frame, content: String, position: Point| {
            frame.fill_text(Text {
                content,
                position,
                color: colors.muted,
                size: 11.0.into(),
                font: theme::font(),
                ..Text::default()
            });
        };

        for speed in (0..=100).step_by(25) {
            let at = Self::to_screen(bounds, TEMP_MIN, speed as f32);
            frame.stroke(
                &Path::line(at, Point::new(plot.x + plot.width, at.y)),
                Stroke::default().with_color(colors.border).with_width(1.0),
            );
            label(&mut frame, format!("{speed}%"), Point::new(2.0, at.y - 7.0));
        }
        for temp in (TEMP_MIN as u8..=TEMP_MAX as u8).step_by(10) {
            let at = Self::to_screen(bounds, f32::from(temp), 0.0);
            frame.stroke(
                &Path::line(Point::new(at.x, plot.y), at),
                Stroke::default().with_color(colors.border).with_width(1.0),
            );
            label(
                &mut frame,
                format!("{temp}°"),
                Point::new(at.x - 8.0, at.y + 6.0),
            );
        }

        // Where the fan is now, to aim the curve at. A filled sliver, not a
        // stroked line: tiny-skia's damage tracking takes a vertical line's
        // bounds as zero wide and leaves bits of it behind when it moves.
        if let Some((temp, speed)) = self.now {
            let at = Self::to_screen(bounds, f32::from(temp), f32::from(speed));
            frame.fill(
                &Path::rectangle(
                    Point::new(at.x - 0.5, plot.y),
                    iced::Size::new(1.0, plot.height),
                ),
                colors.muted,
            );
            frame.fill(&Path::circle(at, 4.0), colors.muted);
        }

        let curve = &self.curve;
        // As the fans will take it: steps when the EC goes through the
        // curve itself, straight between the dots when the daemon adapts.
        let line = Path::new(|path| {
            path.move_to(Self::to_screen(
                bounds,
                TEMP_MIN,
                f32::from(curve.speeds[0]),
            ));
            for (step, &temp) in curve.temps.iter().enumerate() {
                let temp = f32::from(temp);
                if !self.smooth {
                    path.line_to(Self::to_screen(bounds, temp, f32::from(curve.speeds[step])));
                }
                path.line_to(Self::to_screen(
                    bounds,
                    temp,
                    f32::from(curve.speeds[step + 1]),
                ));
            }
            path.line_to(Self::to_screen(
                bounds,
                TEMP_MAX,
                f32::from(curve.speeds[6]),
            ));
        });
        frame.stroke(
            &line,
            Stroke::default().with_color(colors.strong).with_width(2.0),
        );

        let hovered = cursor
            .position_in(bounds)
            .and_then(|at| self.nearest(bounds, at));
        for handle in 0..7 {
            let (temp, speed) = handle_at(curve, handle);
            let at = Self::to_screen(bounds, temp, speed);
            let active = state.handle == Some(handle) || hovered == Some(handle);
            frame.fill(
                &Path::circle(at, if active { 6.0 } else { 4.0 }),
                colors.strong,
            );
            if active {
                let text = match handle {
                    0 => format!("below {}°C: {speed}%", curve.temps[0]),
                    _ => format!("from {temp}°C: {speed}%"),
                };
                // Keep the label inside the graph at the right edge.
                let x = if at.x > plot.x + plot.width - 110.0 {
                    at.x - 120.0
                } else {
                    at.x + 10.0
                };
                frame.fill_text(Text {
                    content: text,
                    position: Point::new(x, (at.y - 20.0).max(0.0)),
                    color: colors.text,
                    size: 12.0.into(),
                    font: theme::font(),
                    ..Text::default()
                });
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.handle.is_some() {
            return mouse::Interaction::Grabbing;
        }
        match cursor
            .position_in(bounds)
            .and_then(|at| self.nearest(bounds, at))
        {
            Some(_) => mouse::Interaction::Grab,
            None => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn factory() -> Curve {
        Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 80]).unwrap()
    }

    #[test]
    fn drags_a_step_within_its_neighbours() {
        // Step 3 starts at 64 °C at 50%.
        let curve = drag(factory(), 3, 60.4, 47.6);
        assert_eq!(curve.temps[2], 60);
        assert_eq!(curve.speeds[3], 48);

        // It can't pass the step below or above it.
        let curve = drag(factory(), 3, 20.0, 0.0);
        assert_eq!((curve.temps[2], curve.speeds[3]), (58, 45));
        let curve = drag(factory(), 3, 99.0, 100.0);
        assert_eq!((curve.temps[2], curve.speeds[3]), (70, 57));
    }

    #[test]
    fn the_first_step_only_moves_up_and_down() {
        let curve = drag(factory(), 0, 90.0, 30.0);
        assert_eq!(curve.temps, factory().temps);
        assert_eq!(curve.speeds[0], 30);
    }

    #[test]
    fn the_last_step_reaches_full_speed_and_the_top_temperature() {
        let curve = drag(factory(), 6, 120.0, 130.0);
        assert_eq!((curve.temps[5], curve.speeds[6]), (100, 100));
    }

    #[test]
    fn dragged_curves_stay_valid() {
        let mut curve = factory();
        for (handle, temp, speed) in [
            (1, 10.0, 90.0),
            (6, 51.0, 0.0),
            (4, 52.0, 100.0),
            (2, 100.0, 0.0),
        ] {
            curve = drag(curve, handle, temp, speed);
            assert!(Curve::new(&curve.temps, &curve.speeds).is_ok(), "{curve:?}");
        }
    }

    #[test]
    fn writes_nix() {
        let nix = as_nix(&[(Fan::Cpu, factory())]);
        assert_eq!(
            nix,
            "fans = {\n  cpu = {\n    temps = [ 50 57 64 71 78 85 ];\n    speeds = [ 0 40 45 50 57 65 80 ];\n  };\n};\n"
        );
    }
}
