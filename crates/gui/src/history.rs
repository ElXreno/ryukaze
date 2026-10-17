//! The last minutes as graphs: under each gauge the temperature, and the
//! fan's duty fainter, both from 0 to 100; under the battery the watts out
//! of it, and into it fainter. The readings are the window's polls, or the
//! tray's, sparser, while it's closed.

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

/// What a graph follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Series {
    /// The temperature, and the fan's duty.
    Sensor(Fan),
    /// The watts out of the battery, and into it.
    Power,
}

impl Series {
    fn index(self) -> usize {
        match self {
            Self::Sensor(Fan::Cpu) => 0,
            Self::Sensor(Fan::Gpu) => 1,
            Self::Power => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Reading {
    at: Instant,
    /// The strong line and the faint one: °C (none off a sleeping GPU) and
    /// fan percent, or watts out and watts in.
    lines: [Option<f32>; 2],
}

#[derive(Debug, Default)]
pub struct History {
    series: [VecDeque<Reading>; 3],
}

/// A stretch of readings to draw as one line: (seconds before the newest
/// reading, value), oldest first.
type Run = Vec<(f32, f32)>;

impl History {
    pub fn push(&mut self, at: Instant, status: &Status) {
        let sensor = |temp: Option<u8>, fan: Option<u8>| {
            (temp.is_some() || fan.is_some()).then(|| {
                [
                    temp.filter(|&temp| temp > 0).map(f32::from),
                    fan.map(f32::from),
                ]
            })
        };
        // Out while it discharges, in otherwise: none at all while it's held
        // at the limit. Nothing where the battery doesn't say.
        let power = status.battery_state.as_deref().and_then(|state| {
            let watts = status.battery_watts? as f32;
            Some(match state {
                "Discharging" => [Some(watts), None],
                _ => [None, Some(watts)],
            })
        });
        for (readings, lines) in self.series.iter_mut().zip([
            sensor(status.cpu_temp, status.cpu_fan),
            sensor(status.gpu_temp, status.gpu_fan),
            power,
        ]) {
            let Some(lines) = lines else {
                continue;
            };
            readings.push_back(Reading { at, lines });
            while readings
                .front()
                .is_some_and(|first| at.duration_since(first.at) > SPAN)
            {
                readings.pop_front();
            }
        }
    }

    fn readings(&self, series: Series) -> &VecDeque<Reading> {
        &self.series[series.index()]
    }

    /// Line `line` of `series`, broken where a value is missing or comes too
    /// late after the last.
    fn runs(&self, series: Series, line: usize) -> Vec<Run> {
        let readings = self.readings(series);
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
            match reading.lines[line] {
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

    /// Where a sensor read nothing (a sleeping GPU), as (from, to) seconds
    /// before the newest reading, out to the readings either side.
    fn asleep(&self, series: Series) -> Vec<(f32, f32)> {
        if series == Series::Power {
            return Vec::new();
        }
        let readings = self.readings(series);
        let Some(newest) = readings.back() else {
            return Vec::new();
        };
        let ago = |i: usize| newest.at.duration_since(readings[i].at).as_secs_f32();
        let joined = |i: usize| readings[i + 1].at.duration_since(readings[i].at) <= GAP;
        let mut stretches = Vec::new();
        let mut start = None;
        for (i, reading) in readings.iter().enumerate() {
            if reading.lines[0].is_some() {
                continue;
            }
            let first = *start.get_or_insert(i);
            let last = i + 1 == readings.len();
            if last || readings[i + 1].lines[0].is_some() || !joined(i) {
                start = None;
                let from = if first > 0 && joined(first - 1) {
                    ago(first - 1)
                } else {
                    ago(first)
                };
                let to = if !last && joined(i) {
                    ago(i + 1)
                } else {
                    ago(i)
                };
                stretches.push((from, to));
            }
        }
        stretches
    }

    /// The value at the top of the graph: 100 for the sensors; for the
    /// battery the most it drew or took in the span, up to the next 10 W,
    /// and no less than 20 W.
    pub fn top(&self, series: Series) -> f32 {
        match series {
            Series::Sensor(_) => 100.0,
            Series::Power => {
                let most = self
                    .readings(series)
                    .iter()
                    .flat_map(|reading| reading.lines.into_iter().flatten())
                    .fold(0.0, f32::max);
                ((most / 10.0).ceil() * 10.0).max(20.0)
            }
        }
    }

    /// The reading nearest `ago` seconds before the newest, and how long
    /// before it was.
    fn near(&self, series: Series, ago: f32) -> Option<(f32, Reading)> {
        let readings = self.readings(series);
        let newest = readings.back()?.at;
        readings
            .iter()
            .map(|reading| (newest.duration_since(reading.at).as_secs_f32(), *reading))
            .min_by(|a, b| (a.0 - ago).abs().total_cmp(&(b.0 - ago).abs()))
    }
}

pub struct Graph<'a> {
    pub history: &'a History,
    pub series: Series,
}

/// The pointer was over the graph at the last move.
#[derive(Default)]
pub struct Hovered(bool);

impl Graph<'_> {
    fn to_screen(&self, bounds: Rectangle, ago: f32, value: f32) -> Point {
        let x = 1.0 - (ago / SPAN.as_secs_f32()).clamp(0.0, 1.0);
        let y = 1.0 - (value / self.history.top(self.series)).clamp(0.0, 1.0);
        let height = (bounds.height - BAND - 2.0 * PAD).max(1.0);
        Point::new(x * bounds.width, BAND + PAD + y * height)
    }

    fn ago_at(bounds: Rectangle, x: f32) -> f32 {
        (1.0 - (x / bounds.width).clamp(0.0, 1.0)) * SPAN.as_secs_f32()
    }

    /// What the reading under the pointer says.
    fn readout(&self, reading: &Reading) -> String {
        match (self.series, reading.lines) {
            (Series::Sensor(_), [temp, fan]) => {
                let mut readout = match temp {
                    Some(temp) => format!("{temp:.0}°C"),
                    None => "Asleep".to_owned(),
                };
                if let Some(fan) = fan {
                    readout += &format!(" · fan {fan:.0}%");
                }
                readout
            }
            (Series::Power, [Some(out), _]) => format!("{out:.0} W out"),
            (Series::Power, [None, Some(into)]) if into >= 0.5 => format!("{into:.0} W in"),
            (Series::Power, _) => "0 W".to_owned(),
        }
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
        let top = self.history.top(self.series);
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

        // Shaded, so a sleeping GPU isn't a line along the bottom.
        for (from, to) in self.history.asleep(self.series) {
            let corner = self.to_screen(bounds, from, top);
            let bottom = self.to_screen(bounds, to, 0.0);
            let size = Size::new(bottom.x - corner.x, bottom.y - corner.y);
            if size.width < 1.0 {
                continue;
            }
            frame.fill(&Path::rectangle(corner, size), colors.idle);
            if size.width >= 48.0 {
                frame.fill_text(Text {
                    content: "Asleep".to_owned(),
                    position: Point::new(corner.x + size.width / 2.0, corner.y + size.height / 2.0),
                    color: colors.muted,
                    size: 11.0.into(),
                    font: theme::font(),
                    align_x: text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..Text::default()
                });
            }
        }

        for value in [0.0, top / 2.0, top] {
            let from = self.to_screen(bounds, SPAN.as_secs_f32(), value);
            frame.stroke(
                &Path::line(from, Point::new(bounds.width, from.y)),
                Stroke::default().with_color(colors.border).with_width(1.0),
            );
        }

        // The faint line first, the strong one over it.
        for (line, color, width) in [(1, colors.muted, 1.5), (0, colors.strong, 2.0)] {
            for run in self.history.runs(self.series, line) {
                let points: Vec<Point> = run
                    .iter()
                    .map(|&(ago, value)| self.to_screen(bounds, ago, value))
                    .collect();
                if let [point] = points[..] {
                    frame.fill(&Path::circle(point, width), color);
                    continue;
                }
                let path = Path::new(|path| {
                    path.move_to(points[0]);
                    for &point in &points[1..] {
                        path.line_to(point);
                    }
                });
                frame.stroke(&path, Stroke::default().with_color(color).with_width(width));
            }
        }

        let hovered = cursor
            .position_in(bounds)
            .and_then(|at| self.history.near(self.series, Self::ago_at(bounds, at.x)));
        let Some((ago, reading)) = hovered else {
            let minutes = SPAN.as_secs() / 60;
            label(&mut frame, format!("{minutes} min ago"), colors.muted, true);
            label(&mut frame, "now".to_owned(), colors.muted, false);
            return vec![frame.into_geometry()];
        };

        // A filled sliver: tiny-skia leaves a stroked vertical line behind
        // when it moves.
        let high = self.to_screen(bounds, ago, top);
        let low = self.to_screen(bounds, ago, 0.0);
        frame.fill(
            &Path::rectangle(
                Point::new(high.x - 0.5, high.y),
                Size::new(1.0, low.y - high.y),
            ),
            colors.muted,
        );
        let mut readout = self.readout(&reading);
        let secs = ago.round() as u32;
        if secs > 0 {
            readout += &format!(" · {}:{:02} ago", secs / 60, secs % 60);
        }
        // On the side away from the pointer, clear of it.
        label(
            &mut frame,
            readout,
            colors.text,
            high.x > bounds.width / 2.0,
        );

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPU: Series = Series::Sensor(Fan::Cpu);
    const GPU: Series = Series::Sensor(Fan::Gpu);

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
        // The first is over ten minutes before the last; the second is
        // too far before the last to join it.
        assert_eq!(
            history.runs(CPU, 0),
            [vec![(360.0, 60.0)], vec![(0.0, 70.0)]]
        );
    }

    #[test]
    fn breaks_where_the_gpu_sleeps() {
        let start = Instant::now();
        let mut history = History::default();
        for (secs, gpu) in [(0, (55, 30)), (2, (0, 0)), (4, (0, 0)), (6, (58, 35))] {
            history.push(start + Duration::from_secs(secs), &status((60, 40), gpu));
        }
        assert_eq!(history.runs(GPU, 0), [vec![(6.0, 55.0)], vec![(0.0, 58.0)]]);
        // The fan reads 0, not nothing, while it sleeps.
        assert_eq!(history.runs(GPU, 1).len(), 1);

        let (ago, reading) = history.near(GPU, 3.5).unwrap();
        assert_eq!((ago, reading.lines), (4.0, [None, Some(0.0)]));

        // Shaded from where the line stops to where it starts again.
        assert_eq!(history.asleep(GPU), [(6.0, 0.0)]);
        assert!(history.asleep(CPU).is_empty());
    }

    #[test]
    fn asleep_up_to_now_and_across_a_gap() {
        let start = Instant::now();
        let mut history = History::default();
        for (secs, temp) in [(0, 0), (10, 0), (100, 0), (110, 50), (120, 0)] {
            history.push(
                start + Duration::from_secs(secs),
                &status((60, 40), (temp, 0)),
            );
        }
        // The daemon was away between 10 s and 100 s: no shade there.
        assert_eq!(
            history.asleep(GPU),
            [(120.0, 110.0), (20.0, 10.0), (10.0, 0.0)]
        );
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
        assert_eq!(history.runs(CPU, 0).len(), 1);
        assert!(history.runs(CPU, 1).is_empty());
        assert!(history.near(GPU, 0.0).is_none());
        assert!(history.near(Series::Power, 0.0).is_none());
    }

    #[test]
    fn watts_out_and_in() {
        let start = Instant::now();
        let mut history = History::default();
        for (secs, state, watts) in [
            (0, "Discharging", Some(18.4)),
            (2, "Discharging", Some(33.0)),
            (4, "Charging", Some(45.0)),
            (6, "Not charging", Some(0.0)),
            (8, "Discharging", None),
        ] {
            let status = Status {
                battery_state: Some(state.into()),
                battery_watts: watts,
                ..Status::default()
            };
            history.push(start + Duration::from_secs(secs), &status);
        }
        let power = Series::Power;
        assert_eq!(history.runs(power, 0), [vec![(6.0, 18.4), (4.0, 33.0)]]);
        assert_eq!(history.runs(power, 1), [vec![(2.0, 45.0), (0.0, 0.0)]]);
        // No watts, no reading: the last is the one at 6 s.
        assert_eq!(history.readings(power).len(), 4);
        // Up to the next 10 W over the most.
        assert_eq!(history.top(power), 50.0);
        assert!(history.asleep(power).is_empty());

        let graph = Graph {
            history: &history,
            series: power,
        };
        let readout = |ago| graph.readout(&history.near(power, ago).unwrap().1);
        assert_eq!(readout(4.0), "33 W out");
        assert_eq!(readout(2.0), "45 W in");
        assert_eq!(readout(0.0), "0 W");
    }
}
