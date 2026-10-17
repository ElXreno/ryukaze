//! `ryukaze --osd`: a card at the bottom of the screen for a moment when
//! the modes change, whoever changed them: Fn+F7, Fn+F8, a power profile,
//! the automatic boost. A process with a layer-shell surface of its own,
//! as winit has no layer shell and iced's software windows have no alpha:
//! iced draws the card off screen, and the pixels go to the compositor as
//! they are, see-through around it.

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

use iced::{
    Alignment, Border, Color, Element, Length, Pixels, Shadow, Size, Theme, Vector,
    advanced::renderer::{self, Headless},
    futures::{StreamExt, stream},
    mouse,
    widget::{column, container, row, text},
};
use iced_runtime::{UserInterface, user_interface};
use ryukaze_core::Status;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, channel,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
        client::{
            Connection, QueueHandle,
            globals::registry_queue_init,
            protocol::{wl_output, wl_shm, wl_surface},
        },
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

use crate::{
    app::capitalize,
    appearance, client,
    theme::{self, Colors},
};

/// How long a card stays, and how long it takes to come and to go.
const SHOWN: Duration = Duration::from_millis(1600);
const FADE: Duration = Duration::from_millis(180);

/// The surface, in logical pixels: the card sits at its bottom, with room
/// for its shadow; the rest is see-through, and clicks go through it all.
const SIZE: (u32, u32) = (480, 120);
const SHADOW: f32 = 14.0;

pub fn run(session: bool) -> Result<(), String> {
    theme::load();
    let connection =
        Connection::connect_to_env().map_err(|err| format!("no Wayland display: {err}"))?;
    let (globals, queue) =
        registry_queue_init(&connection).map_err(|err| format!("Wayland: {err}"))?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<Osd> =
        EventLoop::try_new().map_err(|err| format!("event loop: {err}"))?;
    WaylandSource::new(connection, queue)
        .insert(event_loop.handle())
        .map_err(|err| format!("Wayland: {err}"))?;

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| "the compositor has no wl_compositor".to_owned())?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| "the compositor has no layer shell".to_owned())?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| "the compositor has no wl_shm".to_owned())?;
    let pool = SlotPool::new((SIZE.0 * SIZE.1 * 4) as usize, &shm)
        .map_err(|err| format!("shared memory: {err}"))?;

    let (heard, hearing) = channel::channel();
    event_loop
        .handle()
        .insert_source(hearing, |event, _, osd: &mut Osd| {
            if let channel::Event::Msg(heard) = event {
                osd.heard(heard);
            }
        })
        .map_err(|err| format!("event loop: {err}"))?;
    thread::spawn(move || listen(session, heard));

    let mut osd = Osd {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        qh,
        handle: event_loop.handle(),
        renderer: software_renderer()?,
        modes: None,
        dark: false,
        shown: None,
        timer: false,
    };
    loop {
        event_loop
            .dispatch(None, &mut osd)
            .map_err(|err| format!("event loop: {err}"))?;
    }
}

/// What the D-Bus side passes on.
enum Heard {
    /// The modes at start, to tell later changes from.
    Start(Status),
    Modes(Status),
    Dark(bool),
}

/// ryukazed's mode changes and the desktop's light or dark, on a thread
/// of their own: zbus wants tokio, the surface wants calloop.
fn listen(session: bool, heard: channel::Sender<Heard>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("ryukaze: {err}");
            return;
        }
    };
    runtime.block_on(async move {
        if let Ok(client) = client::connect(session).await
            && let Ok(status) = client.status().await
            && heard.send(Heard::Start(status.modes())).is_err()
        {
            return;
        }
        let mut events = pin!(stream::select(
            client::modes(&session).map(Heard::Modes),
            appearance::dark().map(Heard::Dark),
        ));
        while let Some(event) = events.next().await {
            if heard.send(event).is_err() {
                return;
            }
        }
    });
}

/// iced's software renderer, off screen. Making it awaits nothing.
fn software_renderer() -> Result<iced::Renderer, String> {
    let new = pin!(<iced::Renderer as Headless>::new(
        theme::font(),
        Pixels(16.0),
        None
    ));
    match new.poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Some(renderer)) => Ok(renderer),
        _ => Err("no software renderer".into()),
    }
}

struct Osd {
    registry: RegistryState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    qh: QueueHandle<Osd>,
    handle: LoopHandle<'static, Osd>,
    renderer: iced::Renderer,
    /// The modes as last heard of; none until the daemon first answers.
    modes: Option<Status>,
    /// What the desktop prefers.
    dark: bool,
    shown: Option<Shown>,
    /// A timer waits for the end of the card's time.
    timer: bool,
}

/// The card on screen.
struct Shown {
    layer: LayerSurface,
    card: Card,
    /// Nothing is drawn before the compositor's first configure.
    configured: bool,
    phase: Phase,
    /// When it starts to go, unless another card comes first.
    until: Instant,
    scale: i32,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    In(Instant),
    Full,
    Out(Instant),
}

impl Osd {
    fn heard(&mut self, heard: Heard) {
        match heard {
            Heard::Start(modes) => {
                if self.modes.is_none() {
                    self.modes = Some(modes);
                }
            }
            Heard::Modes(modes) => {
                let card = self.modes.as_ref().and_then(|before| card(before, &modes));
                self.modes = Some(modes);
                if let Some(card) = card {
                    self.show(card);
                }
            }
            Heard::Dark(dark) => {
                self.dark = dark;
                self.draw();
            }
        }
    }

    /// A card already up is swapped in place, without fading again.
    fn show(&mut self, card: Card) {
        match &mut self.shown {
            Some(shown) => {
                shown.card = card;
                shown.until = Instant::now() + SHOWN;
                if let Phase::Out(_) = shown.phase {
                    shown.phase = Phase::Full;
                }
                self.draw();
            }
            None => {
                let surface = self.compositor.create_surface(&self.qh);
                // An empty input region: clicks go through.
                if let Ok(region) = Region::new(&self.compositor) {
                    surface.set_input_region(Some(region.wl_region()));
                }
                let layer = self.layer_shell.create_layer_surface(
                    &self.qh,
                    surface,
                    Layer::Overlay,
                    Some("ryukaze-osd"),
                    None,
                );
                layer.set_anchor(Anchor::BOTTOM);
                layer.set_size(SIZE.0, SIZE.1);
                layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer.commit();
                self.shown = Some(Shown {
                    layer,
                    card,
                    configured: false,
                    phase: Phase::In(Instant::now()),
                    until: Instant::now() + FADE + SHOWN,
                    scale: 1,
                });
            }
        }
        self.wait();
    }

    /// Starts the fade-out at the card's `until`, however often it moves.
    fn wait(&mut self) {
        let Some(shown) = &self.shown else {
            return;
        };
        if self.timer {
            return;
        }
        self.timer = true;
        let _ = self
            .handle
            .insert_source(Timer::from_deadline(shown.until), |_, _, osd| {
                let Some(shown) = &mut osd.shown else {
                    osd.timer = false;
                    return TimeoutAction::Drop;
                };
                if Instant::now() < shown.until {
                    return TimeoutAction::ToInstant(shown.until);
                }
                osd.timer = false;
                shown.phase = Phase::Out(Instant::now());
                osd.draw();
                TimeoutAction::Drop
            });
    }

    fn theme(&self) -> Theme {
        if theme::is_dark(self.dark) {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    /// The card at its point in the fade; while it fades, the next frame
    /// draws again. Gone at the end of it.
    fn draw(&mut self) {
        let theme = self.theme();
        let Some(shown) = &mut self.shown else {
            return;
        };
        if !shown.configured {
            return;
        }
        let opacity = match shown.phase {
            Phase::In(start) => {
                let t = progress(start);
                if t >= 1.0 {
                    shown.phase = Phase::Full;
                }
                smooth(t)
            }
            Phase::Full => 1.0,
            Phase::Out(start) => {
                let t = progress(start);
                if t >= 1.0 {
                    self.shown = None;
                    return;
                }
                1.0 - smooth(t)
            }
        };
        let scale = shown.scale.max(1);
        let pixels = render(&mut self.renderer, &shown.card, &theme, opacity, scale);
        let (width, height) = (SIZE.0 as i32 * scale, SIZE.1 as i32 * scale);
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)
        else {
            return;
        };
        argb(&pixels, canvas);
        let surface = shown.layer.wl_surface();
        surface.set_buffer_scale(scale);
        surface.damage_buffer(0, 0, width, height);
        if !matches!(shown.phase, Phase::Full) {
            surface.frame(&self.qh, surface.clone());
        }
        if buffer.attach_to(surface).is_ok() {
            shown.layer.commit();
        }
    }
}

/// How far into the fade that started at `start`, 0 to 1.
fn progress(start: Instant) -> f32 {
    (start.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0)
}

/// No jolt at either end.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// iced's pixels are RGBA, alpha premultiplied, as Wayland's ARGB8888 has
/// it too, but stored B, G, R, A.
fn argb(rgba: &[u8], canvas: &mut [u8]) {
    let (pixels, _) = rgba.as_chunks::<4>();
    for (out, &[r, g, b, a]) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
        *out = [b, g, r, a];
    }
}

/// The surface's pixels with `card` on them, see-through around it.
fn render(
    renderer: &mut iced::Renderer,
    card: &Card,
    theme: &Theme,
    opacity: f32,
    scale: i32,
) -> Vec<u8> {
    let colors = Faded(*theme::colors(theme), opacity);
    let element: Element<'_, ()> = container(
        container(colors.card(card))
            .padding([12, 18])
            .style(move |_| colors.panel()),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(SHADOW)
    .align_x(Alignment::Center)
    .align_y(Alignment::End)
    .into();
    let size = Size::new(SIZE.0 as f32, SIZE.1 as f32);
    let mut ui = UserInterface::build(element, size, user_interface::Cache::default(), renderer);
    ui.draw(
        renderer,
        theme,
        &renderer::Style {
            text_color: colors.of(colors.0.text),
        },
        mouse::Cursor::Unavailable,
    );
    renderer.screenshot(
        Size::new(SIZE.0 * scale as u32, SIZE.1 * scale as u32),
        scale as f32,
        Color::TRANSPARENT,
    )
}

/// What a change shows.
#[derive(Debug, Clone, PartialEq)]
enum Card {
    /// A scenario, among all of them.
    Scenario { name: String, all: Vec<String> },
    /// A switch, on or off.
    Switch(&'static str, bool),
    /// A mode under its heading.
    Mode(&'static str, String),
}

/// What to show for a change: the scenario when the modes now make one,
/// else the one thing that changed.
fn card(before: &Status, after: &Status) -> Option<Card> {
    if after.scenario != before.scenario
        && let Some(name) = &after.scenario
    {
        return Some(Card::Scenario {
            name: name.clone(),
            all: after.scenarios.clone().unwrap_or_default(),
        });
    }
    if after.cooler_boost != before.cooler_boost
        && let Some(on) = after.cooler_boost
    {
        return Some(Card::Switch("Cooler boost", on));
    }
    if after.super_battery != before.super_battery
        && let Some(on) = after.super_battery
    {
        return Some(Card::Switch("Super battery", on));
    }
    if after.shift_mode != before.shift_mode
        && let Some(mode) = &after.shift_mode
    {
        return Some(Card::Mode("Performance", capitalize(mode)));
    }
    if after.fan_mode != before.fan_mode
        && let Some(mode) = &after.fan_mode
    {
        return Some(Card::Mode("Fans", capitalize(mode)));
    }
    None
}

/// The theme's colours, faded by the card's opacity.
#[derive(Clone, Copy)]
struct Faded(Colors, f32);

impl Faded {
    fn of(&self, color: Color) -> Color {
        Color {
            a: color.a * self.1,
            ..color
        }
    }

    fn panel(&self) -> container::Style {
        container::Style {
            background: Some(self.of(self.0.card).into()),
            border: Border {
                color: self.of(self.0.border),
                width: 1.0,
                radius: 16.0.into(),
            },
            shadow: Shadow {
                color: self.of(Color::from_rgba(0.0, 0.0, 0.0, 0.35)),
                offset: Vector::new(0.0, 2.0),
                blur_radius: SHADOW,
            },
            text_color: Some(self.of(self.0.text)),
            ..container::Style::default()
        }
    }

    fn heading<'a>(&self, heading: &str) -> Element<'a, ()> {
        text(heading.to_uppercase())
            .size(11)
            .color(self.of(self.0.muted))
            .into()
    }

    /// A rounded label, filled when it's the one in force.
    fn pill<'a>(&self, label: String, on: bool) -> Element<'a, ()> {
        let (fill, ink) = if on {
            (self.0.strong, self.0.on_strong)
        } else {
            (self.0.idle, self.0.muted)
        };
        let (fill, ink) = (self.of(fill), self.of(ink));
        container(text(label).size(14).color(ink))
            .padding([5, 12])
            .style(move |_| container::Style {
                background: Some(fill.into()),
                border: Border {
                    radius: 999.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    }

    fn card<'a>(&self, card: &Card) -> Element<'a, ()> {
        match card {
            Card::Scenario { name, all } if all.contains(name) => column![
                self.heading("Scenario"),
                row(all
                    .iter()
                    .map(|each| self.pill(capitalize(each), each == name)))
                .spacing(6),
            ]
            .spacing(8)
            .align_x(Alignment::Center)
            .into(),
            Card::Scenario { name, .. } => {
                column![self.heading("Scenario"), text(capitalize(name)).size(20),]
                    .spacing(4)
                    .align_x(Alignment::Center)
                    .into()
            }
            Card::Switch(name, on) => row![
                text(*name).size(16),
                self.pill(if *on { "On" } else { "Off" }.to_owned(), *on),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
            .into(),
            Card::Mode(heading, mode) => {
                column![self.heading(heading), text(mode.clone()).size(20)]
                    .spacing(4)
                    .align_x(Alignment::Center)
                    .into()
            }
        }
    }
}

impl CompositorHandler for Osd {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if let Some(shown) = &mut self.shown {
            shown.scale = factor;
        }
        self.draw();
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.draw();
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Osd {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.shown = None;
    }

    /// The size is ours; the first configure lets drawing start.
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(shown) = &mut self.shown
            && !shown.configured
        {
            shown.configured = true;
            shown.phase = Phase::In(Instant::now());
        }
        self.draw();
    }
}

impl OutputHandler for Osd {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Osd {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Osd {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Osd);
delegate_output!(Osd);
delegate_shm!(Osd);
delegate_layer!(Osd);
delegate_registry!(Osd);

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(shift: &str, fan: &str, boost: bool, scenario: Option<&str>) -> Status {
        Status {
            shift_mode: Some(shift.into()),
            fan_mode: Some(fan.into()),
            super_battery: Some(false),
            cooler_boost: Some(boost),
            scenario: scenario.map(Into::into),
            scenarios: Some(vec![
                "silent".into(),
                "balanced".into(),
                "performance".into(),
            ]),
            ..Status::default()
        }
    }

    #[test]
    fn a_scenario_shows_among_the_others() {
        let before = modes("turbo", "auto", false, Some("performance"));
        let after = modes("eco", "silent", false, Some("silent"));
        assert!(matches!(
            card(&before, &after),
            Some(Card::Scenario { name, all }) if name == "silent" && all.len() == 3
        ));
    }

    #[test]
    fn otherwise_the_one_thing_that_changed() {
        let before = modes("turbo", "auto", false, Some("performance"));
        let boost = modes("turbo", "auto", true, Some("performance"));
        assert_eq!(
            card(&before, &boost),
            Some(Card::Switch("Cooler boost", true))
        );
        // Out of any scenario: the mode itself.
        let advanced = modes("turbo", "advanced", false, None);
        assert_eq!(
            card(&before, &advanced),
            Some(Card::Mode("Fans", "Advanced".into()))
        );
        assert_eq!(card(&before, &before), None);
    }

    #[test]
    fn see_through_around_the_card() {
        let mut renderer = software_renderer().unwrap();
        let card = Card::Switch("Cooler boost", true);
        let pixels = render(&mut renderer, &card, &Theme::Dark, 1.0, 1);
        let alpha = |x: u32, y: u32| pixels[((y * SIZE.0 + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "a corner of the surface");
        assert_eq!(
            alpha(SIZE.0 / 2, SIZE.1 - SHADOW as u32 - 4),
            255,
            "the card"
        );

        // Half faded: about half see-through, its shadow under it.
        let pixels = render(&mut renderer, &card, &Theme::Dark, 0.5, 1);
        let middle =
            pixels[(((SIZE.1 - SHADOW as u32 - 4) * SIZE.0 + SIZE.0 / 2) * 4 + 3) as usize];
        assert!((110..=160).contains(&middle), "{middle}");

        let mut canvas = [0; 4];
        argb(&[1, 2, 3, 4], &mut canvas);
        assert_eq!(canvas, [3, 2, 1, 4]);
    }
}
