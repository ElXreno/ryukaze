//! The sensors over the last minutes, a graph under each gauge: the
//! temperature, and the fan's duty fainter, both from 0 to 100. The
//! readings are the window's polls, or the tray's, sparser, while it's
//! closed.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use iced::{
    Point, Rectangle, Renderer, Size, Theme, mouse,
    widget::{
        canvas::{self, Frame, Geometry, Path, Stroke, Text},
        text,
    },
};
use ryukaze_core::{Status, fans::Fan};

use crate::{app::Message, theme};

/// How far back the graphs go.
pub const SPAN: Duration = Duration::from_secs(10 * 60);

/// Readings further apart than this aren't joined: the daemon was away in
/// between.
const GAP: Duration = Duration::from_secs(30);

/// The strip over the lines for the time they span, or the readout under
/// the pointer.
const BAND: f32 = 16.0;

const PAD: f32 = 2.0;

#[derive(Debug, Clone, Copy)]
struct Reading {
    at: Instant,
    /// °C; none off a sleeping GPU.
    temp: Option<u8>,
    /// Percent.
    fan: Option<u8>,
}

#[derive(Debug, Default)]
pub struct History {
    cpu: VecDeque<Reading>,
    gpu: VecDeque<Reading>,
}

/// A stretch of readings to draw as one line: (seconds before the newest
/// reading, value), oldest first.
type Run = Vec<(f32, u8)>;

/// Which of a reading a line follows.
type Pick = fn(&Reading) -> Option<u8>;

impl History {
    pub fn push(&mut self, at: Instant, status: &Status) {
        for (readings, temp, fan) in [
            (&mut self.cpu, status.cpu_temp, status.cpu_fan),
            (&mut self.gpu, status.gpu_temp, status.gpu_fan),
        ] {
            if temp.is_none() && fan.is_none() {
                continue;
            }
            readings.push_back(Reading {
                at,
                temp: temp.filter(|&temp| temp > 0),
                fan,
            });
            while readings
                .front()
                .is_some_and(|first| at.duration_since(first.at) > SPAN)
            {
                readings.pop_front();
            }
        }
    }

    fn readings(&self, fan: Fan) -> &VecDeque<Reading> {
        match fan {
            Fan::Cpu => &self.cpu,
            Fan::Gpu => &self.gpu,
        }
    }

    /// `fan`'s temperatures or duties, by `value`, broken where one is
    /// missing or comes too late after the last.
    fn runs(&self, fan: Fan, value: Pick) -> Vec<Run> {
        let readings = self.readings(fan);
        let Some(newest) = readings.back() else {
            return Vec::new();
        };
        let mut runs = Vec::new();
        let mut run = Run::new();
        let mut last: Option<Instant> = None;
        for reading in readings {
            let late = last.is_some_and(|last| reading.at.duration_since(last) > GAP);
            last = Some(reading.at);
            if late && !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            match value(reading) {
                Some(value) => {
                    let ago = newest.at.duration_since(reading.at).as_secs_f32();
                    run.push((ago, value));
                }
                None if !run.is_empty() => runs.push(std::mem::take(&mut run)),
                None => {}
            }
        }
        if !run.is_empty() {
            runs.push(run);
        }
        runs
    }

    /// The reading nearest `ago` seconds before the newest, and how long
    /// before it was.
    fn near(&self, fan: Fan, ago: f32) -> Option<(f32, Reading)> {
        let readings = self.readings(fan);
        let newest = readings.back()?.at;
        readings
            .iter()
            .map(|reading| (newest.duration_since(reading.at).as_secs_f32(), *reading))
            .min_by(|a, b| (a.0 - ago).abs().total_cmp(&(b.0 - ago).abs()))
    }
}

pub struct Graph<'a> {
    pub history: &'a History,
    pub fan: Fan,
}

/// The pointer was over the graph at the last move.
#[derive(Default)]
pub struct Hovered(bool);

impl Graph<'_> {
    fn to_screen(bounds: Rectangle, ago: f32, value: u8) -> Point {
        let x = 1.0 - (ago / SPAN.as_secs_f32()).clamp(0.0, 1.0);
        let y = 1.0 - (f32::from(value) / 100.0).clamp(0.0, 1.0);
        let height = (bounds.height - BAND - 2.0 * PAD).max(1.0);
        Point::new(x * bounds.width, BAND + PAD + y * height)
    }

    fn ago_at(bounds: Rectangle, x: f32) -> f32 {
        (1.0 - (x / bounds.width).clamp(0.0, 1.0)) * SPAN.as_secs_f32()
    }
}

impl canvas::Program<Message> for Graph<'_> {
    type State = Hovered;

    // The readout follows the pointer, and goes once it leaves.
    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) = event else {
            return None;
        };
        let over = cursor.is_over(bounds);
        let was = std::mem::replace(&mut state.0, over);
        (over || was).then(canvas::Action::request_redraw)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let colors = theme::colors(theme);
        let mut frame = Frame::new(renderer, bounds.size());
        let label = |frame: &mut Frame, content: String, color, left: bool| {
            frame.fill_text(Text {
                content,
                position: Point::new(if left { 0.0 } else { bounds.width }, 0.0),
                color,
                size: 11.0.into(),
                font: theme::font(),
                align_x: if left {
                    text::Alignment::Left
                } else {
                    text::Alignment::Right
                },
                ..Text::default()
            });
        };

        for value in [0, 50, 100] {
            let from = Self::to_screen(bounds, SPAN.as_secs_f32(), value);
            frame.stroke(
                &Path::line(from, Point::new(bounds.width, from.y)),
                Stroke::default().with_color(colors.border).with_width(1.0),
            );
        }

        let lines: [(Pick, _, _); 2] = [
            (|reading| reading.fan, colors.muted, 1.5),
            (|reading| reading.temp, colors.strong, 2.0),
        ];
        for (value, color, width) in lines {
            for run in self.history.runs(self.fan, value) {
                let points: Vec<Point> = run
                    .iter()
                    .map(|&(ago, value)| Self::to_screen(bounds, ago, value))
                    .collect();
                if let [point] = points[..] {
                    frame.fill(&Path::circle(point, width), color);
                    continue;
                }
                let line = Path::new(|path| {
                    path.move_to(points[0]);
                    for &point in &points[1..] {
                        path.line_to(point);
                    }
                });
                frame.stroke(&line, Stroke::default().with_color(color).with_width(width));
            }
        }

        let hovered = cursor
            .position_in(bounds)
            .and_then(|at| self.history.near(self.fan, Self::ago_at(bounds, at.x)));
        let Some((ago, reading)) = hovered else {
            let minutes = SPAN.as_secs() / 60;
            label(&mut frame, format!("{minutes} min ago"), colors.muted, true);
            label(&mut frame, "now".to_owned(), colors.muted, false);
            return vec![frame.into_geometry()];
        };

        // A filled sliver: tiny-skia leaves a stroked vertical line behind
        // when it moves.
        let top = Self::to_screen(bounds, ago, 100);
        let bottom = Self::to_screen(bounds, ago, 0);
        frame.fill(
            &Path::rectangle(
                Point::new(top.x - 0.5, top.y),
                Size::new(1.0, bottom.y - top.y),
            ),
            colors.muted,
        );
        let mut readout = match reading.temp {
            Some(temp) => format!("{temp}°C"),
            None => "Asleep".to_owned(),
        };
        if let Some(fan) = reading.fan {
            readout += &format!(" · fan {fan}%");
        }
        let secs = ago.round() as u32;
        if secs > 0 {
            readout += &format!(" · {}:{:02} ago", secs / 60, secs % 60);
        }
        // On the side away from the pointer, clear of it.
        label(&mut frame, readout, colors.text, top.x > bounds.width / 2.0);

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(cpu: (u8, u8), gpu: (u8, u8)) -> Status {
        Status {
            cpu_temp: Some(cpu.0),
            cpu_fan: Some(cpu.1),
            gpu_temp: Some(gpu.0),
            gpu_fan: Some(gpu.1),
            ..Status::default()
        }
    }

    #[test]
    fn keeps_the_last_ten_minutes() {
        let start = Instant::now();
        let mut history = History::default();
        for (secs, temp) in [(0, 50), (300, 60), (660, 70)] {
            history.push(
                start + Duration::from_secs(secs),
                &status((temp, 40), (0, 0)),
            );
        }
        let runs = history.runs(Fan::Cpu, |reading| reading.temp);
        // The first is over ten minutes before the last; the second is
        // too far before the last to join it.
        assert_eq!(runs, [vec![(360.0, 60)], vec![(0.0, 70)]]);
    }

    #[test]
    fn breaks_where_the_gpu_sleeps() {
        let start = Instant::now();
        let mut history = History::default();
        for (secs, gpu) in [(0, (55, 30)), (2, (0, 0)), (4, (0, 0)), (6, (58, 35))] {
            history.push(start + Duration::from_secs(secs), &status((60, 40), gpu));
        }
        assert_eq!(
            history.runs(Fan::Gpu, |reading| reading.temp),
            [vec![(6.0, 55)], vec![(0.0, 58)]]
        );
        // The fan reads 0, not nothing, while it sleeps.
        assert_eq!(history.runs(Fan::Gpu, |reading| reading.fan).len(), 1);

        let (ago, reading) = history.near(Fan::Gpu, 3.5).unwrap();
        assert_eq!((ago, reading.temp, reading.fan), (4.0, None, Some(0)));
    }

    #[test]
    fn nothing_for_what_the_ec_lacks() {
        let mut history = History::default();
        history.push(
            Instant::now(),
            &Status {
                cpu_temp: Some(50),
                ..Status::default()
            },
        );
        assert_eq!(history.runs(Fan::Cpu, |reading| reading.temp).len(), 1);
        assert!(history.runs(Fan::Cpu, |reading| reading.fan).is_empty());
        assert!(history.near(Fan::Gpu, 0.0).is_none());
    }
}
