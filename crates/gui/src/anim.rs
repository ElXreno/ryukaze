//! What moves: a switch whose knob slides, the fill under a row of choices
//! that slides to the one picked, a page coming in and a button's hover
//! colour fading (tooltips: tip.rs).
//! Each keeps its own animation and asks for frames only while it moves;
//! where the desktop asks for less motion, it doesn't move at all.

use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use iced::{
    Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    animation::Animation,
    mouse,
    widget::{
        Button, PickList, button,
        canvas::{self, Frame, Geometry, Path},
        overlay::menu,
        pick_list,
    },
    window,
};

use crate::theme;

/// The desktop asks for less motion.
static REDUCED: AtomicBool = AtomicBool::new(false);

pub fn set_reduced(reduced: bool) {
    REDUCED.store(reduced, Ordering::Relaxed);
}

fn duration() -> Duration {
    lasting(180)
}

/// `ms` long, or not at all where the desktop asks for less motion.
pub fn lasting(ms: u64) -> Duration {
    if REDUCED.load(Ordering::Relaxed) {
        Duration::ZERO
    } else {
        Duration::from_millis(ms)
    }
}

/// A page coming in from `now`: fading in and rising into place.
pub fn arriving(now: Instant) -> Animation<bool> {
    Animation::new(false).duration(lasting(150)).go(true, now)
}

/// A page in place.
pub fn arrived() -> Animation<bool> {
    Animation::new(true)
}

/// How far up a page rises as it comes in.
pub const RISE: f32 = 12.0;

/// Starts `animation` toward `target` the first time it's seen to differ,
/// and asks for the next frame while it moves.
fn follow<T, Message>(
    animation: &mut Option<Animation<T>>,
    target: T,
    event: &canvas::Event,
) -> Option<canvas::Action<Message>>
where
    T: Clone + Copy + PartialEq + iced::animation::Float,
{
    follow_over(animation, target, event, duration())
}

fn follow_over<T, Message>(
    animation: &mut Option<Animation<T>>,
    target: T,
    event: &canvas::Event,
    duration: Duration,
) -> Option<canvas::Action<Message>>
where
    T: Clone + Copy + PartialEq + iced::animation::Float,
{
    let canvas::Event::Window(window::Event::RedrawRequested(now)) = event else {
        return None;
    };
    let animation = animation.get_or_insert_with(|| Animation::new(target).duration(duration));
    if animation.value() != target {
        *animation = animation.clone().duration(duration);
        animation.go_mut(target, *now);
    }
    animation
        .is_animating(*now)
        .then(canvas::Action::request_redraw)
}

/// What a switch or a row of choices is for, by what it says: the same
/// widget in its place on another page is another one.
pub fn id(what: impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(what)
}

/// An animation's state and what it moves for: shown for something else in
/// its place, on another page, it starts over rather than moving from
/// where the other was.
#[derive(Default)]
pub struct Owned<S> {
    id: u64,
    state: S,
}

impl<S: Default> Owned<S> {
    fn get(&mut self, id: u64) -> &mut S {
        if self.id != id {
            *self = Self {
                id,
                state: S::default(),
            };
        }
        &mut self.state
    }

    fn peek(&self, id: u64) -> Option<&S> {
        (self.id == id).then_some(&self.state)
    }
}

pub fn mix(from: Color, to: Color, amount: f32) -> Color {
    let channel = |a: f32, b: f32| a + (b - a) * amount;
    Color::from_rgba(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
        channel(from.a, to.a),
    )
}

/// A switch, on or off, its knob sliding between: the row it's on takes
/// the click.
pub struct Switch {
    pub on: bool,
    /// What it switches (see [`id`]).
    pub id: u64,
}

pub const SWITCH: Size = Size::new(44.0, 24.0);

impl<Message> canvas::Program<Message> for Switch {
    type State = Owned<Option<Animation<bool>>>;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        follow(state.get(self.id), self.on, event)
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
        let now = Instant::now();
        let on = state
            .peek(self.id)
            .and_then(Option::as_ref)
            .map_or(if self.on { 1.0 } else { 0.0 }, |animation| {
                animation.interpolate(0.0, 1.0, now)
            });
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let off = if cursor.is_over(bounds) {
            colors.track_hover
        } else {
            colors.track
        };
        frame.fill(
            &Path::rounded_rectangle(Point::ORIGIN, size, (size.height / 2.0).into()),
            mix(off, colors.strong, on),
        );
        let radius = size.height / 2.0 - 3.0;
        let x = 3.0 + radius + on * (size.width - 2.0 * (3.0 + radius));
        frame.fill(
            &Path::circle(Point::new(x, size.height / 2.0), radius),
            mix(colors.knob, colors.on_strong, on),
        );
        vec![frame.into_geometry()]
    }
}

/// Under `count` choices as wide each, in rows of `across`, `gap` apart:
/// every one's fill, and the chosen one's over it, sliding when another is
/// chosen.
pub struct Choices {
    /// What it chooses between (see [`id`]).
    pub id: u64,
    pub count: usize,
    pub across: usize,
    pub chosen: Option<usize>,
    pub gap: f32,
}

impl Choices {
    fn at(&self, index: usize) -> [f32; 2] {
        let across = self.across.max(1);
        [(index % across) as f32, (index / across) as f32]
    }
}

impl<Message> canvas::Program<Message> for Choices {
    /// Across and down.
    type State = Owned<[Option<Animation<f32>>; 2]>;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let [x, y] = self.at(self.chosen?);
        let [across, down] = state.get(self.id);
        let across = follow(across, x, event);
        let down = follow(down, y, event);
        across.or(down)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let colors = theme::colors(theme);
        let mut frame = Frame::new(renderer, bounds.size());
        let across = self.across.clamp(1, self.count.max(1)) as f32;
        let down = self.count.div_ceil(self.across.max(1)).max(1) as f32;
        let width = (bounds.width - self.gap * (across - 1.0)) / across;
        let height = (bounds.height - self.gap * (down - 1.0)) / down;
        let cell = |[x, y]: [f32; 2]| {
            Path::rounded_rectangle(
                Point::new(x * (width + self.gap), y * (height + self.gap)),
                Size::new(width, height),
                8.0.into(),
            )
        };
        for index in 0..self.count {
            frame.fill(&cell(self.at(index)), colors.idle);
        }
        if let Some(chosen) = self.chosen {
            let now = Instant::now();
            let target = self.at(chosen);
            let own = state.peek(self.id);
            let at = [0, 1].map(|axis| {
                own.and_then(|own| own[axis].as_ref())
                    .map_or(target[axis], |animation| {
                        animation.interpolate_with(|value| value, now)
                    })
            });
            frame.fill(&cell(at), colors.strong);
        }
        vec![frame.into_geometry()]
    }
}

/// A button whose hover colour fades in and out, in `style`'s colours: the
/// way [`Button::style`] takes them, which it replaces.
pub trait Hover<'a, Message> {
    fn hover(
        self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Element<'a, Message>;
}

impl<'a, Message: Clone + 'a> Hover<'a, Message> for Button<'a, Message> {
    fn hover(
        self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Element<'a, Message> {
        let hovered = Rc::new(Cell::new(0.0));
        let amount = Rc::clone(&hovered);
        let button = self.style(move |theme, status| match status {
            button::Status::Active | button::Status::Hovered => blend(
                style(theme, button::Status::Active),
                style(theme, button::Status::Hovered),
                amount.get(),
            ),
            other => style(theme, other),
        });
        Element::new(Hovering {
            content: button.into(),
            hovered,
            menu: None,
        })
    }
}

/// A pick list whose hover colour fades as a button's does, and whose
/// menu fades in as it opens, in `style`'s and `menu_style`'s colours.
pub fn dropdown<'a, T, L, V, Message>(
    list: PickList<'a, T, L, V, Message>,
    style: impl Fn(&Theme, pick_list::Status) -> pick_list::Style + 'a,
    menu_style: impl Fn(&Theme) -> menu::Style + 'a,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
    L: std::borrow::Borrow<[T]> + 'a,
    V: std::borrow::Borrow<T> + 'a,
    Message: Clone + 'a,
{
    let hovered = Rc::new(Cell::new(0.0));
    let menu = Menu {
        opened: Rc::new(Cell::new(None)),
        shown: Rc::new(Cell::new(1.0)),
    };
    let (amount, opened, shown) = (
        Rc::clone(&hovered),
        Rc::clone(&menu.opened),
        Rc::clone(&menu.shown),
    );
    let list = list
        .style(move |theme, status| match status {
            pick_list::Status::Opened { .. } => {
                if opened.get().is_none() {
                    opened.set(Some(Instant::now()));
                }
                style(theme, status)
            }
            pick_list::Status::Active | pick_list::Status::Hovered => {
                opened.set(None);
                let (from, to) = (
                    style(theme, pick_list::Status::Active),
                    style(theme, pick_list::Status::Hovered),
                );
                pick_list::Style {
                    background: mix_background(from.background, to.background, amount.get()),
                    text_color: mix(from.text_color, to.text_color, amount.get()),
                    border: iced::Border {
                        color: mix(from.border.color, to.border.color, amount.get()),
                        ..to.border
                    },
                    ..to
                }
            }
        })
        .menu_style(move |theme| {
            let style = menu_style(theme);
            let fade = |color: Color| Color {
                a: color.a * shown.get(),
                ..color
            };
            menu::Style {
                background: fade_background(style.background, shown.get()),
                border: iced::Border {
                    color: fade(style.border.color),
                    ..style.border
                },
                text_color: fade(style.text_color),
                selected_text_color: fade(style.selected_text_color),
                selected_background: fade_background(style.selected_background, shown.get()),
                shadow: iced::Shadow {
                    color: fade(style.shadow.color),
                    ..style.shadow
                },
            }
        });
    Element::new(Hovering {
        content: list.into(),
        hovered,
        menu: Some(menu),
    })
}

/// How long a menu takes to fade in.
fn menu_fade() -> Duration {
    lasting(140)
}

fn mix_background(from: iced::Background, to: iced::Background, amount: f32) -> iced::Background {
    match (from, to) {
        (iced::Background::Color(a), iced::Background::Color(b)) => mix(a, b, amount).into(),
        (from, to) => {
            if amount < 0.5 {
                from
            } else {
                to
            }
        }
    }
}

fn fade_background(background: iced::Background, shown: f32) -> iced::Background {
    match background {
        iced::Background::Color(color) => Color {
            a: color.a * shown,
            ..color
        }
        .into(),
        other => other,
    }
}

/// A pick list's menu: since when it's open, and how far in, 0 to 1.
struct Menu {
    opened: Rc<Cell<Option<Instant>>>,
    shown: Rc<Cell<f32>>,
}

/// `from` `amount` of the way to `to`, colour by colour.
fn blend(from: button::Style, to: button::Style, amount: f32) -> button::Style {
    let background = match (from.background, to.background) {
        (Some(iced::Background::Color(a)), Some(iced::Background::Color(b))) => {
            Some(mix(a, b, amount).into())
        }
        (None, Some(iced::Background::Color(b))) => Some(
            Color {
                a: b.a * amount,
                ..b
            }
            .into(),
        ),
        (Some(iced::Background::Color(a)), None) => Some(
            Color {
                a: a.a * (1.0 - amount),
                ..a
            }
            .into(),
        ),
        (from, to) => {
            if amount < 0.5 {
                from
            } else {
                to
            }
        }
    };
    button::Style {
        background,
        text_color: mix(from.text_color, to.text_color, amount),
        border: iced::Border {
            color: mix(from.border.color, to.border.color, amount),
            ..to.border
        },
        shadow: iced::Shadow {
            color: mix(from.shadow.color, to.shadow.color, amount),
            ..to.shadow
        },
        ..to
    }
}

/// A button and how far its hover colour is in, 0 to 1, for its style.
struct Hovering<'a, Message> {
    content: Element<'a, Message>,
    hovered: Rc<Cell<f32>>,
    /// A pick list's.
    menu: Option<Menu>,
}

impl<Message> Widget<Message, Theme, Renderer> for Hovering<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Option<Animation<bool>>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(None::<Animation<bool>>)
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_)) = event {
            let over = cursor.is_over(layout.bounds());
            let now = Instant::now();
            let animation = tree.state.downcast_mut::<Option<Animation<bool>>>();
            let animation =
                animation.get_or_insert_with(|| Animation::new(over).duration(lasting(120)));
            if animation.value() != over {
                *animation = animation.clone().duration(lasting(120));
                animation.go_mut(over, now);
            }
            if animation.is_animating(now) {
                shell.request_redraw();
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        if let Some(menu) = &self.menu {
            // A click or a key may open it: a frame to see, then frames
            // while it fades in.
            let opening = matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::ButtonReleased(_))
                    | Event::Keyboard(_)
            );
            let fading = menu
                .opened
                .get()
                .is_some_and(|at| at.elapsed() < menu_fade());
            if opening || fading {
                shell.request_redraw();
            }
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(menu) = &self.menu {
            let over = menu_fade();
            menu.shown.set(menu.opened.get().map_or(0.0, |at| {
                if over.is_zero() {
                    1.0
                } else {
                    (at.elapsed().as_secs_f32() / over.as_secs_f32()).min(1.0)
                }
            }));
        }
        let animation = tree.state.downcast_ref::<Option<Animation<bool>>>();
        let over = cursor.is_over(layout.bounds());
        self.hovered.set(
            animation
                .as_ref()
                .map_or(if over { 1.0 } else { 0.0 }, |animation| {
                    animation.interpolate(0.0, 1.0, Instant::now())
                }),
        );
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_between_two_colours() {
        let black = Color::BLACK;
        let white = Color::WHITE;
        assert_eq!(mix(black, white, 0.0), black);
        assert_eq!(mix(black, white, 1.0), white);
        let half = mix(black, white, 0.5);
        assert!((half.r - 0.5).abs() < 1e-6 && half.a == 1.0);
    }

    #[test]
    fn moves_only_while_it_animates() {
        let mut state: Option<Animation<bool>> = None;
        let start = Instant::now();
        let redraw = |at| canvas::Event::Window(window::Event::RedrawRequested(at));
        // Seen first as it is: nothing to move.
        assert!(follow::<_, ()>(&mut state, false, &redraw(start)).is_none());
        // Turned on: frames until it gets there.
        assert!(follow::<_, ()>(&mut state, true, &redraw(start)).is_some());
        let later = start + Duration::from_secs(1);
        assert!(follow::<_, ()>(&mut state, true, &redraw(later)).is_none());
        // With less motion asked for, it jumps.
        let mut jumped: Option<Animation<bool>> = None;
        let _ = follow_over::<_, ()>(&mut jumped, false, &redraw(start), Duration::ZERO);
        assert!(follow_over::<_, ()>(&mut jumped, true, &redraw(start), Duration::ZERO).is_none());
    }

    /// Half way, half each; a fill fading in from none.
    #[test]
    fn blends_hover_colours() {
        let idle = button::Style {
            background: None,
            text_color: Color::BLACK,
            ..button::Style::default()
        };
        let hover = button::Style {
            background: Some(Color::WHITE.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        };
        let half = blend(idle, hover, 0.5);
        assert_eq!(
            half.background,
            Some(
                Color {
                    a: 0.5,
                    ..Color::WHITE
                }
                .into()
            )
        );
        assert!((half.text_color.r - 0.5).abs() < 1e-6);
        assert_eq!(blend(idle, hover, 1.0).background, hover.background);
    }

    #[test]
    fn slides_between_choices() {
        let mut state: Option<Animation<f32>> = None;
        let start = Instant::now();
        let redraw = |at| canvas::Event::Window(window::Event::RedrawRequested(at));
        let _ = follow::<_, ()>(&mut state, 0.0, &redraw(start));
        let _ = follow::<_, ()>(&mut state, 2.0, &redraw(start));
        let animation = state.unwrap();
        let halfway = animation.interpolate_with(|value| value, start + Duration::from_millis(90));
        assert!(halfway > 0.0 && halfway < 2.0, "{halfway}");
        let there = animation.interpolate_with(|value| value, start + Duration::from_secs(1));
        assert_eq!(there, 2.0);
    }

    #[test]
    fn lays_choices_out_in_rows() {
        let grid = Choices {
            id: 0,
            count: 5,
            across: 3,
            chosen: None,
            gap: 8.0,
        };
        assert_eq!(grid.at(0), [0.0, 0.0]);
        assert_eq!(grid.at(4), [1.0, 1.0]);
    }
}
