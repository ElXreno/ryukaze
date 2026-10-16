use std::{future::Future, time::Duration};

use iced::{
    Alignment, Element, Length, Subscription, Task, Theme, mouse,
    widget::{
        button, canvas, column, container, mouse_area, progress_bar, row, scrollable, slider,
        space, text, toggler,
    },
    window,
};
use ryukaze_core::{
    Status,
    fans::{Curve, Fan},
};

use crate::{
    appearance,
    client::{self, Client},
    icon, instance, theme, tray,
};

/// Picks the EC makes on its own (Fn keys, the daemon's power profiles)
/// show up within this while the window is open.
const REFRESH: Duration = Duration::from_secs(2);

/// With only the tray up, its menu and tooltip can lag a little.
const REFRESH_HIDDEN: Duration = Duration::from_secs(10);

pub struct App {
    session: bool,
    client: Option<Client>,
    /// `None` until the first answer; `Err` while the daemon can't be reached.
    status: Option<Result<Status, String>>,
    /// The charge limit while its slider is held, applied on release.
    charge_limit: Option<u8>,
    /// Why the last change didn't go through.
    error: Option<String>,
    /// What the desktop prefers; light until it says.
    dark: bool,
    /// The window while it's open; without it the app lives in the tray.
    window: Option<window::Id>,
    tray: Option<tray::Handle>,
    /// Whose curve the editor shows.
    curve_fan: Fan,
    /// The curve while a dot is dragged, set on release.
    curve_draft: Option<Curve>,
    /// The curves went to the clipboard since the last change.
    copied: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Connected(Result<Client, String>),
    Tick,
    Status(Box<Result<Status, String>>),
    ShiftMode(String),
    FanMode(String),
    SuperBattery(bool),
    CoolerBoost(bool),
    ChargeLimitDragged(u8),
    ChargeLimitReleased,
    Applied(Result<(), String>),
    Dark(bool),
    TrayReady(tray::Handle),
    /// From the tray or a second start.
    Show,
    WindowClosed(window::Id),
    Quit,
    CurveFan(Fan),
    CurveDragged(Curve),
    CurveReleased,
    CurveReset,
    CurvesCopied,
    AdaptiveFans(bool),
    GpuFollowsCpu(bool),
    AutoCoolerBoost(bool),
    Webcam(bool),
    FnKey(String),
    UsbShare(bool),
}

impl App {
    /// `hidden` starts with only the tray, as at login.
    pub fn new(session: bool, hidden: bool) -> (Self, Task<Message>) {
        let mut app = Self {
            session,
            client: None,
            status: None,
            charge_limit: None,
            error: None,
            dark: false,
            window: None,
            tray: None,
            curve_fan: Fan::Cpu,
            curve_draft: None,
            copied: false,
        };
        let connect = Task::perform(client::connect(session), Message::Connected);
        if hidden {
            (app, connect)
        } else {
            let open = app.open();
            (app, Task::batch([connect, open]))
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Connected(Ok(client)) => {
                self.client = Some(client);
                self.refresh()
            }
            Message::Connected(Err(err)) => {
                self.status = Some(Err(err));
                self.show_in_tray()
            }
            Message::Tick if self.client.is_none() => {
                Task::perform(client::connect(self.session), Message::Connected)
            }
            Message::Tick => self.refresh(),
            Message::Status(status) => {
                self.status = Some(*status);
                self.show_in_tray()
            }
            Message::ShiftMode(mode) => {
                self.call(move |client| async move { client.set_shift_mode(&mode).await })
            }
            Message::FanMode(mode) => {
                self.call(move |client| async move { client.set_fan_mode(&mode).await })
            }
            Message::SuperBattery(on) => {
                self.call(move |client| async move { client.set_super_battery(on).await })
            }
            Message::CoolerBoost(on) => {
                self.call(move |client| async move { client.set_cooler_boost(on).await })
            }
            Message::ChargeLimitDragged(percent) => {
                self.charge_limit = Some(percent);
                Task::none()
            }
            Message::ChargeLimitReleased => match self.charge_limit.take() {
                Some(percent) => {
                    self.call(move |client| async move { client.set_charge_limit(percent).await })
                }
                None => Task::none(),
            },
            Message::Applied(result) => {
                self.error = result.err();
                self.refresh()
            }
            Message::Dark(dark) => {
                self.dark = dark;
                Task::none()
            }
            Message::TrayReady(tray) => {
                self.tray = Some(tray);
                self.show_in_tray()
            }
            Message::Show => match self.window {
                Some(id) => window::gain_focus(id),
                None => self.open(),
            },
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                    self.charge_limit = None;
                    self.curve_draft = None;
                }
                Task::none()
            }
            Message::Quit => iced::exit(),
            Message::CurveFan(fan) => {
                self.curve_fan = fan;
                self.curve_draft = None;
                Task::none()
            }
            Message::CurveDragged(curve) => {
                self.curve_draft = Some(curve);
                self.copied = false;
                Task::none()
            }
            Message::CurveReleased => match self.curve_draft.take() {
                Some(curve) => {
                    // Shown until the daemon's status has it.
                    self.set_curve_in_status(curve);
                    let fan = self.curve_fan.to_string();
                    self.call(move |client| async move {
                        client
                            .set_fan_curve(&fan, &curve.temps, &curve.speeds)
                            .await
                    })
                }
                None => Task::none(),
            },
            Message::CurveReset => {
                self.copied = false;
                let fan = self.curve_fan.to_string();
                self.call(move |client| async move { client.reset_fan_curve(&fan).await })
            }
            Message::AdaptiveFans(on) => {
                self.call(move |client| async move { client.set_adaptive_fans(on).await })
            }
            Message::Webcam(on) => {
                self.call(move |client| async move { client.set_webcam(on).await })
            }
            Message::FnKey(side) => {
                self.call(move |client| async move { client.set_fn_key(&side).await })
            }
            Message::UsbShare(on) => {
                self.call(move |client| async move { client.set_usb_share(on).await })
            }
            Message::AutoCoolerBoost(on) => {
                self.call(move |client| async move { client.set_auto_cooler_boost(on).await })
            }
            Message::GpuFollowsCpu(on) => {
                self.call(move |client| async move { client.set_gpu_fan_follows_cpu(on).await })
            }
            Message::CurvesCopied => {
                let Some(Ok(status)) = &self.status else {
                    return Task::none();
                };
                self.copied = true;
                iced::clipboard::write(crate::curve::as_nix(&curves(status)))
            }
        }
    }

    fn set_curve_in_status(&mut self, curve: Curve) {
        if let Some(Ok(status)) = &mut self.status {
            let slot = match self.curve_fan {
                Fan::Cpu => &mut status.cpu_curve,
                Fan::Gpu => &mut status.gpu_curve,
            };
            *slot = Some((curve.temps.to_vec(), curve.speeds.to_vec()));
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let refresh = if self.window.is_some() {
            REFRESH
        } else {
            REFRESH_HIDDEN
        };
        Subscription::batch([
            iced::time::every(refresh).map(|_| Message::Tick),
            Subscription::run(appearance::dark).map(Message::Dark),
            Subscription::run(tray::run),
            Subscription::run(instance::serve),
            window::close_events().map(Message::WindowClosed),
        ])
    }

    /// iced's plain Light or Dark; the styles in theme.rs only ask which.
    pub fn theme(&self, _window: window::Id) -> Theme {
        if self.dark { Theme::Dark } else { Theme::Light }
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let firmware = match &self.status {
            Some(Ok(status)) => status.firmware.clone().unwrap_or_default(),
            _ => String::new(),
        };
        let header = row![
            text("ryukaze").size(20),
            space::horizontal(),
            text(firmware).size(13).style(theme::muted),
        ]
        .align_y(Alignment::Center);

        let body: Element<'_, Message> = match &self.status {
            None => text("Connecting…").style(theme::muted).into(),
            Some(Err(err)) => banner(err),
            Some(Ok(status)) => self.sections(status),
        };

        let mut page = column![header, body].spacing(16);
        if let Some(err) = &self.error {
            page = page.push(banner(err));
        }
        // Tiled wide, the panel keeps its shape in the middle; tiled short,
        // it scrolls.
        scrollable(
            container(container(page).max_width(560))
                .padding(20)
                .center_x(Length::Fill),
        )
        .into()
    }

    fn sections<'a>(&'a self, status: &'a Status) -> Element<'a, Message> {
        let mut sections = column![].spacing(12);

        let gauges: Vec<Element<'a, Message>> = [
            gauge("CPU", status.cpu_temp, status.cpu_fan, status.cpu_rpm),
            gauge("GPU", status.gpu_temp, status.gpu_fan, status.gpu_rpm),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !gauges.is_empty() {
            sections = sections.push(section("Sensors", row(gauges).spacing(24)));
        }

        if let (Some(current), Some(modes)) = (&status.shift_mode, &status.shift_modes) {
            sections = sections.push(section(
                "Performance",
                column![modes_row(current, modes, SHIFT_ORDER, Message::ShiftMode)],
            ));
        }

        let mut fans = column![].spacing(12);
        if let (Some(current), Some(modes)) = (&status.fan_mode, &status.fan_modes) {
            fans = fans.push(modes_row(current, modes, FAN_ORDER, Message::FanMode));
        }
        if let Some(on) = status.cooler_boost {
            fans = fans.push(switch("Cooler boost", on, Message::CoolerBoost));
        }
        if let Some(on) = status.auto_cooler_boost {
            fans = fans.push(switch(
                "Cooler boost when hot",
                on,
                Message::AutoCoolerBoost,
            ));
        }
        if status.fan_mode.as_deref() == Some("advanced")
            && let Some(editor) = self.curve_editor(status)
        {
            fans = fans.push(editor);
        }
        sections = sections.push(section("Fans", fans));

        let mut battery = column![].spacing(12);
        if let Some(summary) = battery_summary(status) {
            battery = battery.push(summary);
        }
        if let Some(on) = status.super_battery {
            battery = battery.push(switch("Super battery", on, Message::SuperBattery));
        }
        if let Some(limit) = self.charge_limit.or(status.charge_limit) {
            battery = battery.push(charge_limit(limit));
        }
        sections = sections.push(section("Battery", battery));

        let mut devices = column![].spacing(12);
        if let Some(on) = status.webcam {
            devices = devices.push(switch("Camera", on, Message::Webcam));
        }
        if let Some(side) = &status.fn_key {
            devices = devices.push(fn_key(side));
        }
        if let Some(on) = status.usb_share {
            devices = devices.push(switch("Charge USB when off", on, Message::UsbShare));
        }
        if status.webcam.is_some() || status.fn_key.is_some() || status.usb_share.is_some() {
            sections = sections.push(section("Devices", devices));
        }

        sections.into()
    }

    fn open(&mut self) -> Task<Message> {
        let (id, opened) = window::open(window::Settings {
            size: (420.0, 700.0).into(),
            min_size: Some((360.0, 420.0).into()),
            icon: icon::window(),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "ryukaze".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        self.window = Some(id);
        opened.discard()
    }

    fn show_in_tray(&self) -> Task<Message> {
        match (&self.tray, &self.status) {
            (Some(tray), Some(status)) => Task::future(tray.clone().show(status.clone())).discard(),
            _ => Task::none(),
        }
    }

    /// Advanced's curves, one fan at a time: a step graph to drag, a way
    /// back to the config's (reset) and into it (copy as Nix).
    fn curve_editor<'a>(&'a self, status: &'a Status) -> Option<Element<'a, Message>> {
        let (_, reported) = curves(status)
            .into_iter()
            .find(|&(fan, _)| fan == self.curve_fan)?;
        let curve = self.curve_draft.unwrap_or(reported);
        let now = match self.curve_fan {
            Fan::Cpu => status.cpu_temp.zip(status.cpu_fan),
            Fan::Gpu => status.gpu_temp.zip(status.gpu_fan),
        };

        let tabs = row(Fan::ALL.map(|fan| {
            let chosen = fan == self.curve_fan;
            button(
                text(fan.to_string().to_uppercase())
                    .size(13)
                    .width(Length::Fill)
                    .center(),
            )
            .on_press_maybe((!chosen).then_some(Message::CurveFan(fan)))
            .padding([4, 12])
            .width(72)
            .style(theme::segment(chosen))
            .into()
        }))
        .spacing(6);
        let adaptive = status.adaptive_fans.unwrap_or(false);
        let graph = canvas(crate::curve::Editor {
            curve,
            now,
            smooth: adaptive,
        })
        .width(Length::Fill)
        .height(200);
        let hint = if self.copied {
            "Copied, for services.ryukaze.settings"
        } else {
            "Drag a dot: sideways for where a step starts, up or down for its speed"
        };
        let small = |label| {
            button(text(label).size(13))
                .padding([4, 12])
                .style(theme::segment(false))
        };
        let actions = row![
            text(hint).size(12).style(theme::muted).width(Length::Fill),
            small("Reset").on_press(Message::CurveReset),
            small("Copy as Nix").on_press(Message::CurvesCopied),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let mut editor = column![tabs, graph, actions].spacing(10);
        editor = editor.push(switch("Adaptive", adaptive, Message::AdaptiveFans));
        if adaptive && let Some(on) = status.gpu_fan_follows_cpu {
            editor = editor.push(switch("GPU fan helps the CPU", on, Message::GpuFollowsCpu));
        }
        Some(editor.into())
    }

    fn refresh(&self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(async move { client.status().await }, |result| {
            Message::Status(Box::new(result.map_err(client::describe)))
        })
    }

    fn call<F, Fut>(&self, call: F) -> Task<Message>
    where
        F: FnOnce(Client) -> Fut,
        Fut: Future<Output = zbus::Result<()>> + Send + 'static,
    {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(call(client), |result| {
            Message::Applied(result.map_err(client::describe))
        })
    }
}

/// Modes in this order, whatever order the driver lists them in.
pub const SHIFT_ORDER: &[&str] = &["eco", "comfort", "sport", "turbo"];
pub const FAN_ORDER: &[&str] = &["auto", "silent", "basic", "advanced"];

/// `modes` in `order`, unknown ones last.
pub fn ordered<'a>(modes: &'a [String], order: &[&str]) -> Vec<&'a String> {
    let mut modes: Vec<&String> = modes.iter().collect();
    modes.sort_by_key(|mode| {
        order
            .iter()
            .position(|known| known == mode)
            .unwrap_or(order.len())
    });
    modes
}

fn section<'a>(title: &'a str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    let title = text(title.to_uppercase()).size(12).style(theme::muted);
    container(column![title, content.into()].spacing(10))
        .padding(16)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

fn modes_row<'a>(
    current: &str,
    modes: &'a [String],
    order: &[&str],
    pick: fn(String) -> Message,
) -> Element<'a, Message> {
    let mut buttons = row![].spacing(6);
    for mode in ordered(modes, order) {
        let chosen = mode == current;
        buttons = buttons.push(
            button(text(capitalize(mode)).width(Length::Fill).center())
                .on_press_maybe((!chosen).then(|| pick(mode.clone())))
                .padding([8, 12])
                .width(Length::Fill)
                .style(theme::segment(chosen)),
        );
    }
    buttons.into()
}

/// The charge, what the battery is doing, and how worn it is.
fn battery_summary<'a>(status: &Status) -> Option<Element<'a, Message>> {
    let level = status.battery_level?;
    let watts = status
        .battery_watts
        .filter(|&watts| watts >= 0.5)
        .map(|watts| format!(" · {watts:.0} W"))
        .unwrap_or_default();
    // Not charging on AC at or over the limit's lower edge: the EC holds it.
    let held = status.on_ac == Some(true)
        && status
            .charge_limit
            .is_some_and(|limit| limit < 100 && level + 10 >= limit);
    let doing = match status.battery_state.as_deref() {
        Some("Charging") => format!("Charging{watts}"),
        Some("Discharging") => format!("On battery{watts}"),
        Some("Not charging") if held => "Held at the charge limit".to_owned(),
        Some("Full") => "Full".to_owned(),
        Some(other) => other.to_owned(),
        None => String::new(),
    };
    let health = status
        .battery_health
        .map(|health| format!("Health {health}%"))
        .unwrap_or_default();
    Some(
        column![
            row![
                text(format!("{level}%")).size(26),
                space::horizontal(),
                text(health).size(13).style(theme::muted),
            ]
            .align_y(Alignment::Center),
            text(doing).size(13).style(theme::muted),
        ]
        .spacing(2)
        .into(),
    )
}

/// Where Fn sits: a choice of two rather than a "swap", which would depend
/// on where it started.
fn fn_key<'a>(side: &str) -> Element<'a, Message> {
    let choice = |value: &'static str, label: &'static str| {
        let chosen = side == value;
        button(text(label).width(Length::Fill).center())
            .on_press_maybe((!chosen).then(|| Message::FnKey(value.to_owned())))
            .padding([8, 12])
            .width(Length::Fill)
            .style(theme::segment(chosen))
    };
    column![
        text("Fn key"),
        row![
            choice("left", "Left of Win"),
            choice("right", "Right of Win")
        ]
        .spacing(6),
    ]
    .spacing(8)
    .into()
}

/// The whole row toggles, label included; a click on the switch itself is
/// the switch's, so it doesn't toggle twice.
fn switch<'a>(label: &'a str, on: bool, toggle: fn(bool) -> Message) -> Element<'a, Message> {
    let switch = toggler(on).on_toggle(toggle).size(22).style(theme::switch);
    mouse_area(row![text(label), space::horizontal(), switch].align_y(Alignment::Center))
        .on_press(toggle(!on))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// A temperature and the fan under it. The EC reads 0 °C off a sleeping
/// GPU, which says it's asleep rather than cold.
fn gauge<'a>(
    name: &'a str,
    temp: Option<u8>,
    fan: Option<u8>,
    rpm: Option<u16>,
) -> Option<Element<'a, Message>> {
    if temp.is_none() && fan.is_none() {
        return None;
    }
    let reading = match temp {
        Some(0) => text("Asleep").size(26).style(theme::muted),
        Some(temp) => text(format!("{temp}°C")).size(26),
        None => text("–").size(26).style(theme::muted),
    };
    let mut gauge = column![text(name).size(13).style(theme::muted), reading].spacing(2);
    if let Some(fan) = fan {
        gauge = gauge.push(
            column![
                text(match rpm {
                    Some(rpm) => format!("Fan {fan}% · {rpm} rpm"),
                    None => format!("Fan {fan}%"),
                })
                .size(13)
                .style(theme::muted),
                progress_bar(0.0..=100.0, f32::from(fan))
                    .girth(4)
                    .style(theme::duty),
            ]
            .spacing(6)
            .padding(iced::Padding::ZERO.top(6)),
        );
    }
    Some(gauge.width(Length::Fill).into())
}

fn charge_limit<'a>(percent: u8) -> Element<'a, Message> {
    let resumes = if percent >= 100 {
        "Charges to full".to_owned()
    } else {
        format!("Charging resumes below {}%", percent.saturating_sub(10))
    };
    column![
        row![
            text("Charge limit"),
            space::horizontal(),
            text(format!("{percent}%"))
        ],
        slider(10..=100, percent, Message::ChargeLimitDragged)
            .step(5u8)
            .on_release(Message::ChargeLimitReleased)
            .style(theme::level),
        text(resumes).size(13).style(theme::muted),
    ]
    .spacing(8)
    .into()
}

fn banner<'a>(message: &str) -> Element<'a, Message> {
    container(text(message.to_owned()).size(14))
        .padding([8, 12])
        .width(Length::Fill)
        .style(theme::problem)
        .into()
}

/// The curves the daemon reports, checked the way it checks them.
fn curves(status: &Status) -> Vec<(Fan, Curve)> {
    [(Fan::Cpu, &status.cpu_curve), (Fan::Gpu, &status.gpu_curve)]
        .into_iter()
        .filter_map(|(fan, curve)| {
            let (temps, speeds) = curve.as_ref()?;
            Some((fan, Curve::new(temps, speeds).ok()?))
        })
        .collect()
}

pub fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_test::simulator;

    fn app(status: Status) -> App {
        App {
            session: true,
            client: None,
            status: Some(Ok(status)),
            charge_limit: None,
            error: None,
            dark: false,
            window: None,
            tray: None,
            curve_fan: Fan::Cpu,
            curve_draft: None,
            copied: false,
        }
    }

    fn alpha17() -> Status {
        let names = |names: &[&str]| Some(names.iter().map(|&n| n.to_owned()).collect());
        Status {
            firmware: Some("17KKIMS1.115".into()),
            shift_mode: Some("comfort".into()),
            shift_modes: names(&["turbo", "eco", "comfort"]),
            fan_mode: Some("auto".into()),
            fan_modes: names(&["auto", "silent", "advanced"]),
            super_battery: Some(false),
            cooler_boost: Some(false),
            charge_limit: Some(80),
            ..Status::default()
        }
    }

    fn clicks(app: &App, targets: &[&str]) -> Vec<Message> {
        let mut ui = simulator(app.view(window::Id::unique()));
        for &target in targets {
            ui.click(target)
                .unwrap_or_else(|_| panic!("no {target:?} on screen"));
        }
        ui.into_messages().collect()
    }

    #[test]
    fn picks_modes() {
        let messages = clicks(&app(alpha17()), &["Eco", "Silent"]);
        assert!(matches!(&messages[..], [
            Message::ShiftMode(shift),
            Message::FanMode(fan),
        ] if shift == "eco" && fan == "silent"));
    }

    #[test]
    fn the_current_mode_is_not_picked_again() {
        assert!(clicks(&app(alpha17()), &["Comfort", "Auto"]).is_empty());
    }

    #[test]
    fn toggles_by_label() {
        let messages = clicks(&app(alpha17()), &["Cooler boost", "Super battery"]);
        assert!(matches!(
            &messages[..],
            [Message::CoolerBoost(true), Message::SuperBattery(true)]
        ));
    }

    #[test]
    fn tells_what_the_battery_does() {
        let held = Status {
            on_ac: Some(true),
            battery_level: Some(91),
            battery_state: Some("Not charging".into()),
            battery_watts: Some(0.0),
            battery_health: Some(96),
            ..alpha17()
        };
        let holding = app(held);
        let mut ui = simulator(holding.view(window::Id::unique()));
        for shown in ["91%", "Held at the charge limit", "Health 96%"] {
            assert!(ui.find(shown).is_ok(), "{shown}");
        }

        let charging = app(Status {
            on_ac: Some(true),
            battery_level: Some(40),
            battery_state: Some("Charging".into()),
            battery_watts: Some(44.6),
            ..alpha17()
        });
        let mut ui = simulator(charging.view(window::Id::unique()));
        assert!(ui.find("Charging · 45 W").is_ok());
    }

    #[test]
    fn usb_share_switch() {
        let status = Status {
            usb_share: Some(false),
            ..alpha17()
        };
        let messages = clicks(&app(status), &["Charge USB when off"]);
        assert!(matches!(&messages[..], [Message::UsbShare(true)]));
    }
    #[test]
    fn camera_and_fn_key() {
        let status = Status {
            webcam: Some(true),
            fn_key: Some("right".into()),
            ..alpha17()
        };
        let messages = clicks(&app(status), &["Camera", "Left of Win", "Right of Win"]);
        // The side it's on already isn't picked again.
        assert!(matches!(
            &messages[..],
            [Message::Webcam(false), Message::FnKey(side)] if side == "left"
        ));
        let none = app(alpha17());
        let mut ui = simulator(none.view(window::Id::unique()));
        assert!(ui.find("Camera").is_err());
    }

    #[test]
    fn cooler_boost_when_hot() {
        let status = Status {
            auto_cooler_boost: Some(false),
            ..alpha17()
        };
        let messages = clicks(&app(status), &["Cooler boost when hot", "Cooler boost"]);
        assert!(matches!(
            &messages[..],
            [Message::AutoCoolerBoost(true), Message::CoolerBoost(true)]
        ));
        // Not offered where the daemon doesn't say.
        let app = app(alpha17());
        let mut ui = simulator(app.view(window::Id::unique()));
        assert!(ui.find("Cooler boost when hot").is_err());
    }

    #[test]
    fn hides_what_the_ec_lacks() {
        let status = Status {
            super_battery: None,
            charge_limit: None,
            ..alpha17()
        };
        let app = app(status);
        let mut ui = simulator(app.view(window::Id::unique()));
        assert!(ui.find("Super battery").is_err());
        assert!(ui.find("Charge limit").is_err());
        assert!(ui.find("Cooler boost").is_ok());
    }

    #[test]
    fn says_when_the_daemon_is_away() {
        let app = App {
            status: Some(Err("ryukazed isn't running".into())),
            ..app(alpha17())
        };
        let mut ui = simulator(app.view(window::Id::unique()));
        assert!(ui.find("ryukazed isn't running").is_ok());
        assert!(ui.find("Eco").is_err());
    }

    fn advanced() -> Status {
        let curve = |temps: &[u8], speeds: &[u8]| Some((temps.to_vec(), speeds.to_vec()));
        Status {
            fan_mode: Some("advanced".into()),
            cpu_curve: curve(&[50, 57, 64, 71, 78, 85], &[0, 40, 45, 50, 57, 65, 80]),
            gpu_curve: curve(&[55, 60, 65, 70, 75, 80], &[0, 48, 55, 62, 68, 77, 85]),
            adaptive_fans: Some(true),
            gpu_fan_follows_cpu: Some(true),
            ..alpha17()
        }
    }

    #[test]
    fn the_curve_editor_is_advanced_only() {
        let editing = app(advanced());
        let mut ui = simulator(editing.view(window::Id::unique()));
        assert!(ui.find("CPU").is_ok());
        assert!(ui.find("Copy as Nix").is_ok());

        let auto = app(alpha17());
        let mut ui = simulator(auto.view(window::Id::unique()));
        assert!(ui.find("Copy as Nix").is_err());
    }
    #[test]
    fn adaptive_and_its_gpu_switch() {
        let messages = clicks(&app(advanced()), &["GPU fan helps the CPU", "Adaptive"]);
        assert!(matches!(
            &messages[..],
            [Message::GpuFollowsCpu(false), Message::AdaptiveFans(false)]
        ));

        // Stepping: no GPU switch, the EC goes its own way there.
        let stepping = app(Status {
            adaptive_fans: Some(false),
            ..advanced()
        });
        let mut ui = simulator(stepping.view(window::Id::unique()));
        assert!(ui.find("Adaptive").is_ok());
        assert!(ui.find("GPU fan helps the CPU").is_err());
    }

    #[test]
    fn switches_fans_and_resets() {
        let messages = clicks(&app(advanced()), &["GPU", "Reset", "Copy as Nix"]);
        assert!(matches!(
            &messages[..],
            [
                Message::CurveFan(Fan::Gpu),
                Message::CurveReset,
                Message::CurvesCopied
            ]
        ));
    }

    #[test]
    fn a_released_curve_shows_before_the_daemon_answers() {
        let mut app = app(advanced());
        let curve = Curve::new(&[50, 57, 64, 71, 78, 85], &[0, 30, 35, 40, 50, 60, 100]).unwrap();
        let _ = app.update(Message::CurveDragged(curve));
        let _ = app.update(Message::CurveReleased);
        let Some(Ok(status)) = &app.status else {
            panic!("no status");
        };
        assert_eq!(status.cpu_curve.as_ref().unwrap().1, curve.speeds);
        assert!(app.curve_draft.is_none());
    }
}
