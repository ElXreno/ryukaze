use std::{future::Future, time::Duration};

use iced::{
    Alignment, Element, Length, Subscription, Task, Theme, mouse,
    widget::{button, column, container, mouse_area, row, slider, space, text, toggler},
    window,
};
use ryukaze_core::Status;

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
}

#[derive(Debug, Clone)]
pub enum Message {
    Connected(Result<Client, String>),
    Tick,
    Status(Result<Status, String>),
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
                self.status = Some(status);
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
                }
                Task::none()
            }
            Message::Quit => iced::exit(),
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
        // Tiled wide, the panel keeps its shape in the middle.
        container(container(page).max_width(560))
            .padding(20)
            .center_x(Length::Fill)
            .into()
    }

    fn sections<'a>(&'a self, status: &'a Status) -> Element<'a, Message> {
        let mut sections = column![].spacing(12);

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
        sections = sections.push(section("Fans", fans));

        let mut battery = column![].spacing(12);
        if let Some(on) = status.super_battery {
            battery = battery.push(switch("Super battery", on, Message::SuperBattery));
        }
        if let Some(limit) = self.charge_limit.or(status.charge_limit) {
            battery = battery.push(charge_limit(limit));
        }
        sections = sections.push(section("Battery", battery));

        sections.into()
    }

    fn open(&mut self) -> Task<Message> {
        let (id, opened) = window::open(window::Settings {
            size: (420.0, 600.0).into(),
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

    fn refresh(&self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(async move { client.status().await }, |result| {
            Message::Status(result.map_err(client::describe))
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

/// The whole row toggles, label included; a click on the switch itself is
/// the switch's, so it doesn't toggle twice.
fn switch<'a>(label: &'a str, on: bool, toggle: fn(bool) -> Message) -> Element<'a, Message> {
    let switch = toggler(on).on_toggle(toggle).size(22).style(theme::switch);
    mouse_area(row![text(label), space::horizontal(), switch].align_y(Alignment::Center))
        .on_press(toggle(!on))
        .interaction(mouse::Interaction::Pointer)
        .into()
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
}
