//! Tooltips that fade in once the pointer rests on what they explain, and
//! out once it leaves; moving on from one shown to another's, the next
//! shows at once, as a desktop's do.

use std::{
    cell::Cell,
    rc::Rc,
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
    widget::{container, text},
    window,
};

use crate::{anim, theme};

/// How long the pointer rests before a tooltip shows.
const DELAY: Duration = Duration::from_millis(350);

/// How soon after one was shown another shows without the wait.
const WARM: Duration = Duration::from_millis(400);

/// Between what it explains and the tooltip.
const GAP: f32 = 6.0;

thread_local! {
    /// When a tooltip was last shown.
    static LAST_SHOWN: Cell<Option<Instant>> = const { Cell::new(None) };
}

fn warm(now: Instant) -> bool {
    LAST_SHOWN
        .get()
        .is_some_and(|at| now.duration_since(at) < WARM)
}

/// Where the tooltip goes, room allowing; else the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Above,
    Below,
}

/// `tip` on the `side` of `content` where the pointer rests on it.
pub fn tooltip<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    tip: impl text::IntoFragment<'a>,
    side: Side,
) -> Element<'a, Message> {
    let shown = Rc::new(Cell::new(1.0));
    let alpha = Rc::clone(&shown);
    let tip = container(text(tip).size(14))
        .padding([6, 10])
        .max_width(320)
        .style(move |theme: &Theme| faded(theme::tip(theme), alpha.get()));
    Element::new(Tip {
        content: content.into(),
        tip: tip.into(),
        shown,
        side,
    })
}

/// `style` `shown` of the way in, 0 to 1.
fn faded(style: container::Style, shown: f32) -> container::Style {
    let fade = |color: Color| Color {
        a: color.a * shown,
        ..color
    };
    container::Style {
        background: style.background.map(|background| match background {
            iced::Background::Color(color) => fade(color).into(),
            other => other,
        }),
        text_color: style.text_color.map(fade),
        border: iced::Border {
            color: fade(style.border.color),
            ..style.border
        },
        shadow: iced::Shadow {
            color: fade(style.shadow.color),
            ..style.shadow
        },
        ..style
    }
}

struct Tip<'a, Message> {
    content: Element<'a, Message>,
    tip: Element<'a, Message>,
    /// How far in the tooltip is, 0 to 1, for its style.
    shown: Rc<Cell<f32>>,
    side: Side,
}

/// Since when the pointer rests on it, and the tooltip showing or going.
struct State {
    resting: Option<Instant>,
    showing: Animation<bool>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            resting: None,
            showing: Animation::new(false),
        }
    }
}

impl State {
    fn show(&mut self, on: bool, now: Instant) {
        if self.showing.value() != on {
            let ms = if on { 150 } else { 120 };
            self.showing = self.showing.clone().duration(anim::lasting(ms));
            self.showing.go_mut(on, now);
        }
    }

    /// The pointer `over` it or not at `now`; when to look again.
    fn step(&mut self, over: bool, now: Instant) -> Option<Instant> {
        match (self.resting, over) {
            (None, true) => {
                self.resting = Some(now);
                if warm(now) {
                    self.show(true, now);
                }
            }
            (Some(_), false) => {
                self.resting = None;
                if self.showing.value() {
                    LAST_SHOWN.set(Some(now));
                }
                self.show(false, now);
            }
            (Some(since), true) if !self.showing.value() && now >= since + DELAY => {
                self.show(true, now);
            }
            _ => {}
        }
        if self.showing.value() {
            LAST_SHOWN.set(Some(now));
        }
        match self.resting {
            Some(since) if !self.showing.value() => Some(since + DELAY),
            _ => self.showing.is_animating(now).then_some(now),
        }
    }

    fn visible(&self, now: Instant) -> bool {
        self.showing.value() || self.showing.is_animating(now)
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Tip<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::new(&self.tip)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content, &self.tip]);
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
            let state = tree.state.downcast_mut::<State>();
            let was = state.visible(Instant::now());
            if let Some(at) = state.step(cursor.is_over(layout.bounds()), Instant::now()) {
                shell.request_redraw_at(at);
            }
            if was != state.visible(Instant::now()) {
                shell.invalidate_layout();
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
        let state = tree.state.downcast_ref::<State>();
        let now = Instant::now();
        let shown = state
            .visible(now)
            .then(|| state.showing.interpolate(0.0, 1.0, now));
        let [content_tree, tip_tree] = tree.children.as_mut_slice() else {
            return None;
        };
        let content = self.content.as_widget_mut().overlay(
            content_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let tip = shown.map(|shown| {
            self.shown.set(shown);
            overlay::Element::new(Box::new(Shown {
                tip: &mut self.tip,
                tree: tip_tree,
                host: layout.bounds() + translation,
                side: self.side,
            }))
        });
        let all: Vec<_> = content.into_iter().chain(tip).collect();
        (!all.is_empty()).then(|| overlay::Group::with_children(all).overlay())
    }
}

/// The tooltip shown, beside `host`.
struct Shown<'a, 'b, Message> {
    tip: &'b mut Element<'a, Message>,
    tree: &'b mut Tree,
    host: Rectangle,
    side: Side,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Shown<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.tip.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let at = place(self.host, node.size(), bounds, self.side);
        node.move_to(at)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.tip.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &Rectangle::with_size(Size::INFINITE),
        );
    }
}

/// Where a tooltip of `size` goes by `host` in a window of `bounds`: on
/// its `side` and over its middle, kept inside, on the other side where
/// there's no room.
fn place(host: Rectangle, size: Size, bounds: Size, side: Side) -> Point {
    let above = host.y - GAP - size.height;
    let below = host.y + host.height + GAP;
    let y = match side {
        Side::Above if above < 0.0 => below,
        Side::Above => above,
        Side::Below if below + size.height > bounds.height => above.max(0.0),
        Side::Below => below,
    };
    let x = (host.center_x() - size.width / 2.0).clamp(0.0, (bounds.width - size.width).max(0.0));
    Point::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_by_what_it_explains() {
        let host = Rectangle::new(Point::new(100.0, 100.0), Size::new(50.0, 20.0));
        let size = Size::new(80.0, 30.0);
        let window = Size::new(400.0, 400.0);
        assert_eq!(
            place(host, size, window, Side::Above),
            Point::new(85.0, 64.0)
        );
        assert_eq!(
            place(host, size, window, Side::Below),
            Point::new(85.0, 126.0)
        );
        // No room above: below; kept inside at the left.
        let top = Rectangle::new(Point::new(0.0, 0.0), Size::new(20.0, 20.0));
        assert_eq!(place(top, size, window, Side::Above), Point::new(0.0, 26.0));
    }

    #[test]
    fn waits_then_shows_and_hands_over() {
        LAST_SHOWN.set(None);
        let start = Instant::now();
        let mut first = State::default();
        // Resting: not yet, look again after the wait.
        assert_eq!(first.step(true, start), Some(start + DELAY));
        assert!(!first.visible(start));
        let _ = first.step(true, start + DELAY);
        assert!(first.showing.value());
        // Off it and straight onto another: that one shows at once.
        let left = start + DELAY + Duration::from_millis(500);
        let _ = first.step(false, left);
        assert!(!first.showing.value());
        let mut next = State::default();
        let _ = next.step(true, left + Duration::from_millis(50));
        assert!(next.showing.value());
        // Long after, the wait again.
        let mut later = State::default();
        let _ = later.step(true, left + Duration::from_secs(5));
        assert!(!later.showing.value());
    }
}
