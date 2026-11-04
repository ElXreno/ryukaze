use std::{
    collections::BTreeMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use iced::{
    Alignment, Element, Length, Subscription, Task, Theme, event, keyboard, mouse,
    widget::{
        Stack, button, canvas, column, container, mouse_area, pick_list, progress_bar, responsive,
        row, slider, space, text, text_input,
    },
    window,
};
use ryukaze_core::{
    COOLER_BOOST_MARKS, Status,
    fans::{Curve, Fan},
};

mod pages;

pub use pages::Page;

use crate::anim::Hover;
use crate::{
    anim, appearance,
    client::{self, Client},
    history::History,
    icon, instance,
    laptop::{self, Laptop},
    lights, report, scenarios, theme, tray,
};

/// Picks the EC makes on its own (Fn keys, the daemon's power profiles)
/// show up within this while the window is open.
const REFRESH: Duration = Duration::from_secs(2);

/// With only the tray up, its menu and tooltip can lag a little.
const REFRESH_HIDDEN: Duration = Duration::from_secs(10);

/// Around a page.
const PAGE_PADDING: f32 = 20.0;

/// How long the note says the settings were copied.
const COPIED_FOR: Duration = Duration::from_secs(4);

/// Between cells of a grid.
const CELL_GAP: f32 = 8.0;

pub struct App {
    session: bool,
    /// Over a made-up laptop: no daemon, no tray (see demo.rs).
    demo: bool,
    client: Option<Client>,
    /// `None` until the first answer; `Err` while the daemon can't be reached.
    status: Option<Result<Status, String>>,
    /// The sensors at each answer, for the graphs.
    history: History,
    /// The charge limit while its slider is held, applied on release.
    charge_limit: Option<u8>,
    /// Where cooler boost comes on by itself (CPU, GPU) while a slider is
    /// held, applied on release.
    boost_marks: Option<(u8, u8)>,
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
    /// The settings went to the clipboard since the last change.
    copied: bool,
    /// The keyboard light's editor, kept unsaved while another page is shown.
    lights: Option<lights::Editor>,
    /// The editor's changes on the keyboard: a preview to take back.
    editing_lights: bool,
    /// The Lighting page's automation shown.
    lighting_more: bool,
    /// A preview on its way to the daemon, and the one asked for since.
    previewing: bool,
    next_preview: Option<String>,
    /// The last preview was turned down: not sent again unasked.
    preview_refused: bool,
    /// Exiting once the previews are through.
    quitting: bool,
    /// The previews sent, for the tests.
    #[cfg(test)]
    previews: Vec<String>,
    /// The scenarios' editor, the same way.
    scenarios: Option<scenarios::Editor>,
    editing_scenarios: bool,
    /// The power button's colour open to change, and it as typed.
    power_color: Option<String>,
    power_hex: String,
    /// The laptop itself, once read.
    laptop: Option<Arc<Laptop>>,
    /// The page shown, and it coming in, as of `now`.
    page: Page,
    arrival: iced::animation::Animation<bool>,
    now: Instant,
}

#[derive(Debug, Clone)]
pub enum Message {
    Connected(Result<Client, String>),
    /// Show a page, leaving an editor open on another.
    Page(Page),
    /// A frame while something moves.
    Frame(Instant),
    /// The laptop itself, read.
    Laptop(Arc<Laptop>),
    Tick,
    Status(Box<Result<Status, String>>),
    /// ryukazed says the modes changed, from wherever.
    ModesChanged(Box<Status>),
    ShiftMode(String),
    FanMode(String),
    SuperBattery(bool),
    CoolerBoost(bool),
    ChargeLimitDragged(u8),
    ChargeLimitReleased,
    Applied(Result<(), String>),
    /// A preview shown, or not.
    Previewed(Result<(), String>),
    Appearance(appearance::Change),
    /// The page this far down the sidebar, from 0.
    PageAt(usize),
    TrayReady(tray::Handle),
    /// From the tray or a second start.
    Show,
    WindowClosed(window::Id),
    Quit,
    CurveFan(Fan),
    CurveDragged(Curve),
    CurveReleased,
    CurveReset,
    CopyAsNix,
    /// "Copied" said long enough.
    CopiedGone,
    /// The saved keyboard profiles, in TOML, for the settings copied.
    ProfilesRead(Vec<String>),
    AdaptiveFans(bool),
    GpuFollowsCpu(bool),
    AutoCoolerBoost(bool),
    BoostMarkDragged(Fan, u8),
    BoostMarksReleased,
    Webcam(bool),
    FnKey(String),
    UsbShare(bool),
    LocalDimming(bool),
    NumpadFollowsNumlock(bool),
    /// What the power button's light follows.
    PowerLight(&'static str),
    PowerLightDgpu(bool),
    /// The power button's colour for a shift mode or `dgpu`, open or shut.
    EditPowerColor(String),
    PowerHex(String),
    /// A colour for it, `#rrggbb`.
    PowerColor(String, String),
    /// A scenario and the keyboard profile it lights, empty for the picked.
    ScenarioLight(String, String),
    /// A power source, `ac` or `battery`, and its scenario, empty for none.
    SourceScenario(&'static str, String),
    PowerProfilesSync(bool),
    GpuMode(String),
    Scenario(String),
    KeyboardProfile(String),
    /// The Lighting page's automation shown or hidden.
    LightingMore,
    /// The profile shown, in TOML, for the editor to start from.
    LightsRead(Result<String, String>),
    Lights(lights::Msg),
    LightsSaved(Result<(), String>),
    LightsDeleted(Result<(), String>),
    /// Open the scenarios' editor.
    EditScenarios,
    ScenariosRead(Result<String, String>),
    Scenarios(scenarios::Msg),
    ScenariosSaved(Result<(), String>),
}

impl App {
    /// `hidden` starts with only the tray, as at login.
    pub fn new(session: bool, hidden: bool, demo: bool) -> (Self, Task<Message>) {
        let mut app = Self {
            session,
            demo,
            client: None,
            status: demo.then(|| Ok(crate::demo::status())),
            history: History::default(),
            charge_limit: None,
            error: None,
            boost_marks: None,
            dark: false,
            window: None,
            tray: None,
            curve_fan: Fan::Cpu,
            curve_draft: None,
            copied: false,
            lights: None,
            editing_lights: false,
            lighting_more: false,
            #[cfg(test)]
            previews: Vec::new(),
            scenarios: None,
            editing_scenarios: false,
            power_color: None,
            power_hex: String::new(),
            previewing: false,
            next_preview: None,
            preview_refused: false,
            quitting: false,
            laptop: None,
            page: Page::Overview,
            arrival: anim::arrived(),
            now: Instant::now(),
        };
        let connect = Task::batch([
            Task::perform(laptop::read(), Message::Laptop),
            if demo {
                Task::none()
            } else {
                Task::perform(client::connect(session), Message::Connected)
            },
        ]);
        if hidden && !demo {
            (app, connect)
        } else {
            let open = app.open();
            (app, Task::batch([connect, open]))
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.demo
            && let Some(Ok(status)) = &mut self.status
        {
            if crate::demo::apply(status, &message) {
                return Task::none();
            }
            match message {
                Message::Tick => {
                    self.history.push(Instant::now(), status);
                    return Task::none();
                }
                Message::ChargeLimitReleased => {
                    status.charge_limit = self.charge_limit.take().or(status.charge_limit);
                    return Task::none();
                }
                _ => {}
            }
        }
        match message {
            Message::PageAt(at) => {
                let Some(Ok(status)) = &self.status else {
                    return Task::none();
                };
                match Page::ALL
                    .into_iter()
                    .filter(|page| page.shown(status))
                    .nth(at)
                {
                    Some(page) if page != self.page => self.update(Message::Page(page)),
                    _ => Task::none(),
                }
            }
            Message::Frame(now) => {
                self.now = now;
                Task::none()
            }
            Message::Laptop(laptop) => {
                if let Some(editor) = &mut self.lights {
                    editor.set_laptop(Arc::clone(&laptop));
                }
                self.laptop = Some(laptop);
                Task::none()
            }
            Message::Page(page) => {
                self.editing_scenarios = false;
                let from = std::mem::replace(&mut self.page, page);
                // Another page comes in from its top.
                let top = if from == page {
                    Task::none()
                } else {
                    self.now = Instant::now();
                    self.arrival = anim::arriving(self.now);
                    iced::widget::operation::snap_to(
                        pages::scroller(),
                        iced::widget::scrollable::RelativeOffset::START,
                    )
                };
                let rest = match page {
                    Page::Lighting if from == Page::Lighting => Task::none(),
                    Page::Lighting => self.open_lights(),
                    // As when the window closes: the keyboard back to its
                    // profile, what was left unsaved kept for the next time.
                    _ => self.end_preview(),
                };
                Task::batch([top, rest])
            }
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
                if let Ok(status) = &*status {
                    self.history.push(Instant::now(), status);
                }
                let before = self.shown_lights();
                self.status = Some(*status);
                let follow = self.follow_keyboard(before);
                let keep = self.keep_preview();
                Task::batch([follow, keep, self.show_in_tray()])
            }
            Message::ModesChanged(modes) => {
                let before = self.shown_lights();
                if let Some(Ok(status)) = &mut self.status {
                    status.shift_mode = modes.shift_mode;
                    status.fan_mode = modes.fan_mode;
                    status.super_battery = modes.super_battery;
                    status.cooler_boost = modes.cooler_boost;
                    status.local_dimming = modes.local_dimming;
                    status.scenario = modes.scenario;
                    status.scenarios = modes.scenarios;
                    status.keyboard_profile = modes.keyboard_profile;
                    status.keyboard_profiles = modes.keyboard_profiles;
                    status.keyboard_previewing = modes.keyboard_previewing;
                }
                let follow = self.follow_keyboard(before);
                let keep = self.keep_preview();
                Task::batch([follow, keep, self.show_in_tray()])
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
            Message::Previewed(result) => {
                self.previewing = false;
                self.preview_refused = result.is_err();
                let next = match self.next_preview.take() {
                    Some(profile) => self.preview(profile),
                    None if self.quitting => return iced::exit(),
                    None => Task::none(),
                };
                Task::batch([self.update(Message::Applied(result)), next])
            }
            Message::Applied(result) => {
                let told = match unseen(self.window, &result) {
                    Some(err) => Task::future(tray::notify(err)).discard(),
                    None => Task::none(),
                };
                self.error = result.err();
                // What was copied no longer matches.
                self.copied = false;
                Task::batch([told, self.refresh()])
            }
            Message::Appearance(change) => {
                match change {
                    appearance::Change::Dark(dark) => {
                        self.dark = dark;
                        theme::read_desktop();
                    }
                    appearance::Change::Accent(accent) => theme::set_accent(accent),
                    appearance::Change::Reduced(reduced) => anim::set_reduced(reduced),
                    appearance::Change::Other => theme::read_desktop(),
                }
                Task::none()
            }
            Message::TrayReady(tray) => {
                self.tray = Some(tray);
                self.show_in_tray()
            }
            Message::Show => match self.window {
                Some(id) => window::gain_focus(id),
                None => {
                    let opened = self.open();
                    // What was left unsaved goes back on the keyboard.
                    let lights = match self.page {
                        Page::Lighting => self.open_lights(),
                        _ => Task::none(),
                    };
                    Task::batch([opened, lights])
                }
            },
            Message::WindowClosed(id) => {
                // No tray to stay in.
                if self.demo {
                    return iced::exit();
                }
                if self.window == Some(id) {
                    self.window = None;
                    self.charge_limit = None;
                    self.boost_marks = None;
                    self.curve_draft = None;
                    self.editing_scenarios = false;
                    // The keyboard back to its profile, what was left
                    // unsaved kept for the next time.
                    return self.end_preview();
                }
                Task::none()
            }
            // The keyboard back to its profile first, rather than left
            // on what the editor shows, for the flash to keep at power-off:
            // what's going goes through, and the last answer exits.
            Message::Quit => {
                let back = self.end_preview();
                if !self.previewing {
                    return iced::exit();
                }
                self.quitting = true;
                back
            }
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
            Message::EditPowerColor(name) => {
                if self.power_color.as_ref() == Some(&name) {
                    self.power_color = None;
                } else {
                    self.power_hex = match &self.status {
                        Some(Ok(status)) => status
                            .power_colors
                            .as_ref()
                            .and_then(|colors| colors.get(&name))
                            .cloned()
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    self.power_color = Some(name);
                }
                Task::none()
            }
            Message::PowerHex(hex) => {
                let valid = hex.parse::<ryukaze_core::lighting::Color>().is_ok();
                self.power_hex = hex.clone();
                match (&self.power_color, valid) {
                    (Some(name), true) => self.update(Message::PowerColor(name.clone(), hex)),
                    _ => Task::none(),
                }
            }
            Message::PowerColor(name, color) => {
                self.power_hex.clone_from(&color);
                self.call(move |client| async move { client.set_power_color(&name, &color).await })
            }
            Message::PowerLightDgpu(on) => {
                self.call(move |client| async move { client.set_power_light_dgpu(on).await })
            }
            Message::PowerLight(follows) => {
                self.call(move |client| async move { client.set_power_light(follows).await })
            }
            Message::ScenarioLight(scenario, profile) => self.call(move |client| async move {
                client
                    .set_scenario_keyboard_profile(&scenario, &profile)
                    .await
            }),
            Message::SourceScenario(source, scenario) => self.call(move |client| async move {
                client.set_source_scenario(source, &scenario).await
            }),
            Message::NumpadFollowsNumlock(on) => {
                self.call(move |client| async move { client.set_numpad_follows_numlock(on).await })
            }
            Message::PowerProfilesSync(on) => {
                self.call(move |client| async move { client.set_power_profiles_sync(on).await })
            }
            Message::UsbShare(on) => {
                self.call(move |client| async move { client.set_usb_share(on).await })
            }
            Message::LocalDimming(on) => {
                self.call(move |client| async move { client.set_local_dimming(on).await })
            }
            Message::GpuMode(mode) => {
                self.call(move |client| async move { client.set_gpu_mode(&mode).await })
            }
            Message::Scenario(name) => {
                self.call(move |client| async move { client.set_scenario(&name).await })
            }
            Message::KeyboardProfile(name) => {
                self.call(move |client| async move { client.set_keyboard_profile(&name).await })
            }
            Message::LightingMore => {
                self.lighting_more = !self.lighting_more;
                Task::none()
            }
            Message::LightsRead(read) => {
                match read.and_then(|profile| lights::Editor::from_toml(&profile)) {
                    Ok(editor) => self.lights = Some(self.on_laptop(editor)),
                    // Not read: what's open stays, rather than a blank one
                    // over it. Asked for nothing in particular, a blank one.
                    Err(err) if !err.is_empty() && self.lights.is_some() => self.error = Some(err),
                    Err(_) => self.lights = Some(self.on_laptop(lights::Editor::new())),
                }
                Task::none()
            }
            Message::Lights(msg) => {
                let Some(editor) = &mut self.lights else {
                    return Task::none();
                };
                let outcome = editor.update(msg);
                self.lights_outcome(outcome)
            }
            // Back to what was left unsaved, or else from the daemon's.
            Message::EditScenarios => {
                self.page = Page::System;
                if self
                    .scenarios
                    .as_ref()
                    .is_some_and(scenarios::Editor::unsaved)
                {
                    self.editing_scenarios = true;
                    return Task::none();
                }
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move { client.read_scenarios().await.map_err(client::describe) },
                    Message::ScenariosRead,
                )
            }
            Message::ScenariosRead(read) => {
                match read.and_then(|text| scenarios::Editor::from_toml(&text)) {
                    Ok(editor) => {
                        self.scenarios = Some(editor);
                        self.editing_scenarios = true;
                    }
                    Err(err) => self.error = Some(err),
                }
                Task::none()
            }
            Message::Scenarios(msg) => {
                let Some(editor) = &mut self.scenarios else {
                    return Task::none();
                };
                match editor.update(msg) {
                    scenarios::Outcome::Nothing => Task::none(),
                    scenarios::Outcome::Save(list) => {
                        let Some(client) = self.client.clone() else {
                            return Task::none();
                        };
                        Task::perform(
                            async move { client.set_scenarios(&list).await.map_err(client::describe) },
                            Message::ScenariosSaved,
                        )
                    }
                    scenarios::Outcome::Done => {
                        self.editing_scenarios = false;
                        Task::none()
                    }
                }
            }
            Message::ScenariosSaved(saved) => {
                match saved {
                    Ok(()) => {
                        if let Some(editor) = &mut self.scenarios {
                            editor.saved();
                        }
                        self.error = None;
                    }
                    Err(err) => self.error = Some(err),
                }
                self.refresh()
            }
            Message::LightsDeleted(deleted) => {
                match deleted {
                    // The editor opens what the keyboard shows now, once the
                    // status says.
                    Ok(()) => {
                        self.lights = None;
                        self.error = None;
                    }
                    Err(err) => self.error = Some(err),
                }
                self.refresh()
            }
            Message::LightsSaved(saved) => {
                let next = match saved {
                    // The daemon shows the profile saved, and picks it.
                    Ok(()) => {
                        self.error = None;
                        self.editing_lights = false;
                        match &mut self.lights {
                            Some(editor) => {
                                if let Some(Ok(status)) = &mut self.status {
                                    status.keyboard_profile = Some(editor.name().to_owned());
                                }
                                editor.saved()
                            }
                            None => lights::Outcome::Nothing,
                        }
                    }
                    Err(err) => {
                        self.error = Some(err);
                        lights::Outcome::Nothing
                    }
                };
                let next = self.lights_outcome(next);
                Task::batch([next, self.refresh()])
            }
            Message::AutoCoolerBoost(on) => {
                self.call(move |client| async move { client.set_auto_cooler_boost(on).await })
            }
            Message::BoostMarkDragged(fan, mark) => {
                let Some(Ok(status)) = &self.status else {
                    return Task::none();
                };
                let Some(mut marks) = self.boost_marks.or(boost_marks(status)) else {
                    return Task::none();
                };
                match fan {
                    Fan::Cpu => marks.0 = mark,
                    Fan::Gpu => marks.1 = mark,
                }
                self.boost_marks = Some(marks);
                Task::none()
            }
            Message::BoostMarksReleased => {
                let Some((cpu, gpu)) = self.boost_marks.take() else {
                    return Task::none();
                };
                // Shown until the daemon's status has them.
                if let Some(Ok(status)) = &mut self.status {
                    (status.cooler_boost_cpu_c, status.cooler_boost_gpu_c) = (Some(cpu), Some(gpu));
                }
                self.call(
                    move |client| async move { client.set_cooler_boost_marks(cpu, gpu).await },
                )
            }
            Message::GpuFollowsCpu(on) => {
                self.call(move |client| async move { client.set_gpu_fan_follows_cpu(on).await })
            }
            // With the saved keyboard profiles, read from the daemon first.
            Message::CopyAsNix => {
                let Some(Ok(status)) = &self.status else {
                    return Task::none();
                };
                let saved = status.keyboard_saved_profiles.clone().unwrap_or_default();
                match self.client.clone() {
                    Some(client) if !saved.is_empty() => Task::perform(
                        async move {
                            let mut read = Vec::new();
                            for name in saved {
                                if let Ok(profile) = client.read_keyboard_profile(&name).await {
                                    read.push(profile);
                                }
                            }
                            read
                        },
                        Message::ProfilesRead,
                    ),
                    _ => self.update(Message::ProfilesRead(Vec::new())),
                }
            }
            Message::CopiedGone => {
                self.copied = false;
                Task::none()
            }
            Message::ProfilesRead(profiles) => {
                let Some(Ok(status)) = &self.status else {
                    return Task::none();
                };
                let profiles = profiles
                    .iter()
                    .filter_map(|profile| lights::Editor::from_toml(profile).ok())
                    .map(|editor| editor.nix_value())
                    .collect();
                self.copied = true;
                let copy =
                    iced::clipboard::write(crate::nix::settings(status, &curves(status), profiles));
                // The note says so for a moment, then what it said before.
                let gone = Task::perform(async { tokio::time::sleep(COPIED_FOR).await }, |()| {
                    Message::CopiedGone
                });
                Task::batch([copy, gone])
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
        let common = [
            iced::time::every(refresh).map(|_| Message::Tick),
            Subscription::run(appearance::changes).map(Message::Appearance),
            window::close_events().map(Message::WindowClosed),
            event::listen_with(hotkey),
            if self.arrival.is_animating(self.now) {
                window::frames().map(Message::Frame)
            } else {
                Subscription::none()
            },
        ];
        if self.demo {
            return Subscription::batch(common);
        }
        Subscription::batch(
            common.into_iter().chain([
                Subscription::run_with(self.session, client::modes)
                    .map(|modes| Message::ModesChanged(Box::new(modes))),
                Subscription::run(tray::run),
                Subscription::run(instance::serve),
            ]),
        )
    }

    /// The desktop's colours, light or dark as it prefers (see theme.rs).
    pub fn theme(&self, _window: window::Id) -> Theme {
        theme::theme(theme::is_dark(self.dark))
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        responsive(move |size| self.layout(size.width)).into()
    }

    fn open(&mut self) -> Task<Message> {
        let (id, opened) = window::open(window::Settings {
            size: (880.0, 680.0).into(),
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

    /// Advanced's curves, one fan at a time: a step graph to drag, and a
    /// way back to the config's.
    fn curve_editor<'a>(&'a self, status: &'a Status) -> Option<Element<'a, Message>> {
        let (_, reported) = curves(status)
            .into_iter()
            .find(|&(fan, _)| fan == self.curve_fan)?;
        let curve = self.curve_draft.unwrap_or(reported);
        let now = match self.curve_fan {
            Fan::Cpu => status.cpu_temp.zip(status.cpu_fan),
            Fan::Gpu => status.gpu_temp.zip(status.gpu_fan),
        };

        let tabs = container(choices(
            Fan::ALL
                .map(|fan| (fan.to_string().to_uppercase(), Message::CurveFan(fan)))
                .into(),
            Fan::ALL.iter().position(|&fan| fan == self.curve_fan),
            Fan::ALL.len(),
            true,
        ))
        .width(152);
        let adaptive = status.adaptive_fans.unwrap_or(false);
        let graph = canvas(crate::curve::Editor {
            curve,
            now,
            smooth: adaptive,
        })
        .width(Length::Fill)
        .height(200);
        let hint = "Drag a dot: sideways for where a step starts, up or down for its speed";
        let actions = row![
            text(hint).size(14).style(theme::muted).width(Length::Fill),
            small_button("Reset", Message::CurveReset),
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

    /// The keyboard light's profile shown.
    fn shown_lights(&self) -> Option<String> {
        match &self.status {
            Some(Ok(status)) => status.keyboard_profile.clone(),
            _ => None,
        }
    }

    /// A profile of the daemon's, for the editor.
    fn read_lights(&self, name: String) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .read_keyboard_profile(&name)
                    .await
                    .map_err(client::describe)
            },
            Message::LightsRead,
        )
    }

    /// `editor` for this laptop, once it's read.
    fn on_laptop(&self, mut editor: lights::Editor) -> lights::Editor {
        if let Some(laptop) = &self.laptop {
            editor.set_laptop(Arc::clone(laptop));
        }
        editor
    }

    /// Whether it runs NixOS, once that's known.
    fn nixos(&self) -> bool {
        self.laptop
            .as_ref()
            .is_some_and(|laptop| laptop.machine.nixos)
    }

    /// The profile `name` in the editor: the daemon's, or without one a
    /// standard one by its name.
    fn load_lights(&mut self, name: Option<String>) -> Task<Message> {
        if self.client.is_none() {
            self.lights = Some(self.on_laptop(lights::Editor::standard(
                name.as_deref().unwrap_or("custom"),
            )));
            return Task::none();
        }
        match name {
            Some(name) => self.read_lights(name),
            None => self.update(Message::LightsRead(Err(String::new()))),
        }
    }

    /// The Lighting page opened: what was left unsaved shown again, or else
    /// the profile on the keyboard, which shows it already.
    fn open_lights(&mut self) -> Task<Message> {
        let shown = self.shown_lights();
        match &self.lights {
            Some(editor) if editor.unsaved() => {
                let profile = editor.preview();
                self.editing_lights = true;
                self.preview(profile)
            }
            Some(editor) if shown.as_deref() == Some(editor.name()) => Task::none(),
            _ => self.load_lights(shown),
        }
    }

    /// The profile on the keyboard changed from `before`, by the daemon: an
    /// unsaved edit shown again over it, else the editor follows it.
    fn follow_keyboard(&mut self, before: Option<String>) -> Task<Message> {
        let now = self.shown_lights();
        let Some(editor) = &self.lights else {
            return if self.page == Page::Lighting {
                self.load_lights(now)
            } else {
                Task::none()
            };
        };
        if now == before {
            return Task::none();
        }
        if editor.unsaved() {
            if !self.editing_lights {
                return Task::none();
            }
            let profile = editor.preview();
            return self.preview(profile);
        }
        if self.page == Page::Lighting {
            self.load_lights(now)
        } else {
            Task::none()
        }
    }

    /// The keyboard back to its profile, if the editor's changes are on it.
    fn end_preview(&mut self) -> Task<Message> {
        if !self.editing_lights {
            return Task::none();
        }
        self.editing_lights = false;
        self.preview(String::new())
    }

    /// What the editor asks for, done.
    fn lights_outcome(&mut self, outcome: lights::Outcome) -> Task<Message> {
        match outcome {
            lights::Outcome::Nothing => Task::none(),
            lights::Outcome::Preview(profile) => {
                self.editing_lights = true;
                self.preview(profile)
            }
            lights::Outcome::Save(profile) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move {
                        client
                            .save_keyboard_profile(&profile)
                            .await
                            .map_err(client::describe)
                    },
                    Message::LightsSaved,
                )
            }
            lights::Outcome::New => {
                // "profile 2" if there's a "profile 1", and so on.
                let names = match &self.status {
                    Some(Ok(status)) => status.keyboard_profiles.clone().unwrap_or_default(),
                    _ => Vec::new(),
                };
                let name = (1..)
                    .map(|n| format!("profile {n}"))
                    .find(|name| !names.contains(name))
                    .unwrap_or_default();
                let editor = self.on_laptop(lights::Editor::fresh(&name));
                let profile = editor.preview();
                self.lights = Some(editor);
                self.editing_lights = true;
                self.preview(profile)
            }
            // Picked, shown on the keyboard as it is, and opened.
            lights::Outcome::Open(name) => {
                let back = self.end_preview();
                if let Some(Ok(status)) = &mut self.status {
                    status.keyboard_profile = Some(name.clone());
                }
                let picked = name.clone();
                let pick = self
                    .call(move |client| async move { client.set_keyboard_profile(&picked).await });
                let load = self.load_lights(Some(name));
                Task::batch([back.chain(pick), load])
            }
            lights::Outcome::Delete(name) => {
                let back = self.end_preview();
                let Some(client) = self.client.clone() else {
                    return back;
                };
                back.chain(Task::perform(
                    async move {
                        client
                            .delete_keyboard_profile(&name)
                            .await
                            .map_err(client::describe)
                    },
                    Message::LightsDeleted,
                ))
            }
            lights::Outcome::Revert => {
                let back = self.end_preview();
                let load = self.load_lights(self.shown_lights());
                Task::batch([back, load])
            }
        }
    }

    /// Shows a profile from the editor on the keyboard; empty, the one
    /// picked again. One at a time: those asked for meanwhile wait, and
    /// only the last goes, so a quick run of changes doesn't pile up on
    /// the controller.
    fn preview(&mut self, profile: String) -> Task<Message> {
        #[cfg(test)]
        self.previews.push(profile.clone());
        if self.previewing {
            self.next_preview = Some(profile);
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.previewing = true;
        Task::perform(
            async move { client.preview_keyboard_profile(&profile).await },
            |result| Message::Previewed(result.map_err(client::describe)),
        )
    }

    /// An unsaved edit taken off the keyboard by the daemon, as a new
    /// scenario does: shown again.
    fn keep_preview(&mut self) -> Task<Message> {
        let dropped = matches!(
            &self.status,
            Some(Ok(status)) if status.keyboard_previewing == Some(false)
        );
        if !self.editing_lights || !dropped || self.previewing || self.preview_refused {
            return Task::none();
        }
        let Some(editor) = &self.lights else {
            return Task::none();
        };
        let profile = editor.preview();
        self.preview(profile)
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
pub const SHIFT_ORDER: &[&str] = &["eco", "comfort", "sport", "turbo", "apex"];
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

fn modes_row<'a>(
    current: &str,
    modes: &'a [String],
    order: &[&str],
    pick: fn(String) -> Message,
) -> Element<'a, Message> {
    let modes = ordered(modes, order);
    let chosen = modes.iter().position(|&mode| mode == current);
    let across = modes.len();
    let labels = modes
        .into_iter()
        .map(|mode| (capitalize(mode), pick(mode.clone())))
        .collect();
    choices(labels, chosen, across, false)
}

/// The charge, what the battery is doing, and how worn it is.
fn battery_summary<'a>(status: &Status) -> Option<Element<'a, Message>> {
    let level = status.battery_level?;
    let doing = report::battery_doing(status)
        .map(|doing| capitalize(&doing))
        .unwrap_or_default();
    let health = status
        .battery_health
        .map(|health| format!("Health {health}%"))
        .unwrap_or_default();
    Some(
        column![
            row![
                text(format!("{level}%")).size(26),
                space::horizontal(),
                text(health).size(14).style(theme::muted),
            ]
            .align_y(Alignment::Center),
            text(doing).size(14).style(theme::muted),
        ]
        .spacing(2)
        .into(),
    )
}

/// Where Fn sits: a choice of two rather than a "swap", which would depend
/// on where it started.
fn fn_key<'a>(side: &str) -> Element<'a, Message> {
    let sides = [("left", "Left of Win"), ("right", "Right of Win")];
    let labels = sides
        .map(|(value, label)| (label.to_owned(), Message::FnKey(value.into())))
        .into();
    let chosen = sides.iter().position(|&(value, _)| value == side);
    column![text("Fn key"), choices(labels, chosen, sides.len(), false)]
        .spacing(8)
        .into()
}

/// The MUX. The choice is the mode after a restart: a pick changes nothing
/// before that.
fn gpu_mode<'a>(now: &str, next: &str, modes: &[String]) -> Element<'a, Message> {
    let hint = match now {
        _ if now != next => format!("Switches to {next} at the next restart"),
        "hybrid" => "The iGPU drives the screen; the dGPU wakes when needed".to_owned(),
        "integrated" => "The iGPU drives the screen; the dGPU is off".to_owned(),
        _ => "The dGPU drives the screen and stays awake".to_owned(),
    };
    column![
        grid(next, modes, modes.len(), Message::GpuMode),
        text(hint).size(14).style(theme::muted),
    ]
    .spacing(8)
    .into()
}

/// Whether the dGPU sleeps, for how long, and who keeps it up.
fn dgpu<'a>(status: &Status) -> Option<Element<'a, Message>> {
    let awake = status.dgpu_awake?;
    let line = text(format!("dGPU {}", report::dgpu_state(status)?)).size(14);
    Some(
        if awake {
            line
        } else {
            line.style(theme::muted)
        }
        .into(),
    )
}

fn small_button<'a>(label: &'a str, press: Message) -> Element<'a, Message> {
    button(text(label).size(14))
        .on_press(press)
        .padding([4, 12])
        .hover(theme::segment(false))
}

/// What the power button's light follows: the profile, the shift mode, or
/// on a hybrid laptop the dGPU.
fn power_light<'a>(follows: &str, hybrid: bool) -> Element<'a, Message> {
    let mut options = vec![("profile", "The profile's"), ("shift_mode", "The mode")];
    if hybrid {
        options.push(("dgpu", "The dGPU"));
    }
    let chosen = options.iter().position(|&(value, _)| value == follows);
    let across = options.len();
    let labels = options
        .into_iter()
        .map(|(value, label)| (label.to_owned(), Message::PowerLight(value)))
        .collect();
    choices(labels, chosen, across, true)
}

/// The power button's colours for what it follows: the shift modes', and
/// the dGPU's awake; the one open, with its hex code and a few to pick.
fn power_colors<'a>(
    status: &Status,
    follows: &str,
    open: Option<&str>,
    hex: &str,
) -> Element<'a, Message> {
    let colors = status.power_colors.clone().unwrap_or_default();
    let mut names: Vec<(String, String)> = Vec::new();
    if follows == "shift_mode" {
        for mode in ordered(
            status.shift_modes.as_deref().unwrap_or_default(),
            SHIFT_ORDER,
        ) {
            names.push((mode.clone(), capitalize(mode)));
        }
    }
    if follows == "dgpu" || status.power_light_dgpu == Some(true) {
        names.push(("dgpu".into(), "dGPU".into()));
    }
    let swatch = |color: Option<&String>, size: f32| {
        let color = color
            .and_then(|hex| hex.parse::<ryukaze_core::lighting::Color>().ok())
            .map_or(iced::Color::TRANSPARENT, lights::led);
        container(space().width(size).height(size)).style(move |theme: &Theme| container::Style {
            background: Some(color.into()),
            border: iced::Border::default()
                .rounded(4)
                .width(1)
                .color(theme::colors(theme).border),
            ..container::Style::default()
        })
    };
    let mut cells = row![].spacing(4);
    for (name, label) in &names {
        let chosen = open == Some(name.as_str());
        cells = cells.push(
            button(
                row![swatch(colors.get(name), 14.0), text(label.clone()).size(14)]
                    .spacing(6)
                    .align_y(Alignment::Center),
            )
            .on_press(Message::EditPowerColor(name.clone()))
            .padding([4, 8])
            .hover(theme::segment(chosen)),
        );
    }
    let mut column = column![cells.wrap().vertical_spacing(4)].spacing(6);
    if let Some(name) = open
        && names.iter().any(|(known, _)| known == name)
    {
        let mut palette = row![
            text_input("#rrggbb", hex)
                .style(theme::field)
                .on_input(Message::PowerHex)
                .size(14)
                .width(90),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        for color in lights::SWATCHES {
            let led = lights::led(color);
            palette = palette.push(
                button(space().width(18).height(18))
                    .padding(0)
                    .on_press(Message::PowerColor(name.to_owned(), color.to_string()))
                    .style(move |theme: &Theme, _| button::Style {
                        background: Some(led.into()),
                        border: iced::Border::default()
                            .rounded(4)
                            .width(1)
                            .color(theme::colors(theme).border),
                        ..button::Style::default()
                    }),
            );
        }
        column = column.push(palette.wrap().vertical_spacing(4));
    }
    column.into()
}

/// The profile a scenario lights, if one of its own: a pick list each.
fn scenario_lights<'a>(
    scenarios: &[String],
    own: &BTreeMap<String, String>,
    standard: &[String],
    saved: &[String],
) -> Element<'a, Message> {
    let picked = Choice::new("the picked one", "");
    let options: Vec<Choice> = std::iter::once(picked.clone())
        .chain(
            standard
                .iter()
                .chain(saved)
                .map(|profile| Choice::new(profile, profile)),
        )
        .collect();
    let lists = scenarios
        .iter()
        .map(|scenario| {
            let shown = own
                .get(scenario)
                .and_then(|profile| options.iter().find(|choice| choice.value == *profile))
                .unwrap_or(&picked)
                .clone();
            let scenario = scenario.clone();
            labelled(
                capitalize(&scenario),
                pick(options.clone(), shown, move |profile| {
                    Message::ScenarioLight(scenario.clone(), profile)
                }),
            )
        })
        .collect();
    cells(lists, 2)
}

/// What a change here lasts: until the next start for what the config
/// sets, which it sets again then, for good for the rest.
fn kept_hint(status: &Status) -> String {
    let mut set: Vec<&str> = Vec::new();
    for key in status.config_keys.iter().flatten() {
        let name = match key.as_str() {
            "charge_limit" => "the charge limit",
            "webcam" => "the camera",
            "fn_key" => "the Fn key",
            "usb_share" => "USB Power Share",
            "local_dimming" => "local dimming",
            "fans.cpu" | "fans.gpu" => "fan curves",
            "fans.adaptive" => "adaptive fans",
            "fans.gpu_follows_cpu" => "the GPU fan's help",
            "cooler_boost.auto" => "cooler boost when hot",
            "cooler_boost.cpu_c" | "cooler_boost.gpu_c" => "its marks",
            "power_profiles_daemon.sync" => "power-profiles-daemon in step",
            "keyboard.numpad_follows_numlock" => "the numpad with NumLock",
            "profiles.ac" | "profiles.battery" => "what AC and battery apply",
            _ if key.starts_with("keyboard.scenarios.") => "the scenarios' keyboard light",
            _ => continue,
        };
        if !set.contains(&name) {
            set.push(name);
        }
    }
    match set.as_slice() {
        [] => "Changes here are kept".to_owned(),
        [one] => format!("Changes here are kept; the config sets {one} again at every start"),
        [rest @ .., last] => format!(
            "Changes here are kept; the config sets {} and {last} again at every start",
            rest.join(", ")
        ),
    }
}

/// Whether the config sets `key`, as it writes it: then it sets it again
/// at every start.
fn config_sets(status: &Status, key: &str) -> bool {
    status
        .config_keys
        .as_ref()
        .is_some_and(|keys| keys.iter().any(|known| known == key))
}

/// What a power source applies: a scenario, nothing, or the config's own
/// modes, which only the config changes.
fn source_pick<'a>(
    source: &'static str,
    label: &'a str,
    picked: &str,
    scenarios: &[String],
) -> Element<'a, Message> {
    let options: Vec<Choice> = std::iter::once(Choice::new("Nothing", ""))
        .chain(
            scenarios
                .iter()
                .map(|scenario| Choice::new(&capitalize(scenario), scenario)),
        )
        .collect();
    // The config's own modes: shown, not to pick.
    let shown = match picked {
        "*" => Choice::new("Own modes", "*"),
        _ => options
            .iter()
            .find(|choice| choice.value == picked)
            .cloned()
            .unwrap_or_else(|| options[0].clone()),
    };
    labelled(
        label.to_owned(),
        pick(options, shown, move |scenario| {
            Message::SourceScenario(source, scenario)
        }),
    )
}

/// A pick list's choice: what it shows, and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    label: String,
    value: String,
}

impl Choice {
    fn new(label: &str, value: &str) -> Self {
        Self {
            label: label.to_owned(),
            value: value.to_owned(),
        }
    }
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn pick<'a>(
    options: Vec<Choice>,
    shown: Choice,
    on: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    anim::dropdown(
        pick_list(options, Some(shown), move |choice: Choice| on(choice.value))
            .text_size(15)
            .padding([6, 10])
            .width(Length::Fill),
        theme::pick,
        theme::menu,
    )
}

/// A muted label over what it names.
fn labelled<'a>(label: String, what: Element<'a, Message>) -> Element<'a, Message> {
    column![text(label).size(14).style(theme::muted), what]
        .spacing(4)
        .width(Length::Fill)
        .into()
}

/// In rows of `across`, each as wide.
fn cells<'a>(cells: Vec<Element<'a, Message>>, across: usize) -> Element<'a, Message> {
    let mut rows = column![].spacing(CELL_GAP);
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let mut line = row![].spacing(CELL_GAP);
        for _ in 0..across {
            line = match cells.next() {
                Some(cell) => line.push(cell),
                None => line.push(space().width(Length::Fill)),
            };
        }
        rows = rows.push(line);
    }
    rows.into()
}

/// The keyboard light's profiles: those saved in the window, and the
/// others, the standard ones and the config's.
pub fn keyboard_profiles(status: &Status) -> (Vec<String>, Vec<String>) {
    let saved = status.keyboard_saved_profiles.clone().unwrap_or_default();
    status
        .keyboard_profiles
        .iter()
        .flatten()
        .cloned()
        .partition(|name| saved.contains(name))
}

/// Choices in rows of `across`, every button as wide.
fn grid<'a>(
    current: &str,
    names: &[String],
    across: usize,
    pick: fn(String) -> Message,
) -> Element<'a, Message> {
    let chosen = names.iter().position(|name| name == current);
    let labels = names
        .iter()
        .map(|name| (capitalize(name), pick(name.clone())))
        .collect();
    choices(labels, chosen, across, false)
}

/// Choices in rows of `across`, every button as wide, smaller text where
/// `small`; the chosen one does nothing, and its fill slides to it.
fn choices<'a>(
    labels: Vec<(String, Message)>,
    chosen: Option<usize>,
    across: usize,
    small: bool,
) -> Element<'a, Message> {
    let count = labels.len();
    let labels_id: Vec<&str> = labels.iter().map(|(label, _)| label.as_str()).collect();
    let labels_id = labels_id.join("\u{1f}");
    let across = across.clamp(1, count.max(1));
    let buttons = labels
        .into_iter()
        .enumerate()
        .map(|(index, (label, pick))| {
            let is_chosen = chosen == Some(index);
            let label = text(label).width(Length::Fill).center();
            let (label, padding) = if small {
                (label.size(14), [4, 8])
            } else {
                (label, [8, 4])
            };
            button(label)
                .on_press_maybe((!is_chosen).then_some(pick))
                .padding(padding)
                .width(Length::Fill)
                .hover(theme::choice(is_chosen))
        })
        .collect();
    let fills = anim::Choices {
        id: anim::id(&labels_id),
        count,
        across,
        chosen,
        gap: CELL_GAP,
    };
    Stack::new()
        .width(Length::Fill)
        .push(cells(buttons, across))
        .push_under(canvas(fills).width(Length::Fill).height(Length::Fill))
        .into()
}

/// Ctrl and a digit, unless a field took it: the page that far down the
/// sidebar.
fn hotkey(event: iced::Event, status: event::Status, _window: window::Id) -> Option<Message> {
    use keyboard::key::{Code, Physical};
    const DIGITS: [Code; 9] = [
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    let (
        event::Status::Ignored,
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key: Physical::Code(code),
            modifiers,
            ..
        }),
    ) = (status, event)
    else {
        return None;
    };
    if modifiers != keyboard::Modifiers::CTRL {
        return None;
    }
    DIGITS
        .iter()
        .position(|&digit| digit == code)
        .map(Message::PageAt)
}

/// The whole row toggles, label included.
fn switch<'a>(label: &'a str, on: bool, toggle: fn(bool) -> Message) -> Element<'a, Message> {
    let switch = canvas(anim::Switch {
        on,
        id: anim::id(label),
    })
    .width(anim::SWITCH.width)
    .height(anim::SWITCH.height);
    mouse_area(row![text(label), space::horizontal(), switch].align_y(Alignment::Center))
        .on_press(toggle(!on))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

/// A temperature, the fan under it, and how both went. The EC reads 0 °C
/// off a sleeping GPU, which says it's asleep rather than cold.
fn gauge<'a>(
    sensor: Fan,
    temp: Option<u8>,
    fan: Option<u8>,
    rpm: Option<u16>,
    graph: impl Into<Element<'a, Message>>,
) -> Option<Element<'a, Message>> {
    if temp.is_none() && fan.is_none() {
        return None;
    }
    let reading = match temp {
        Some(0) => text("Asleep").size(26).style(theme::muted),
        Some(temp) => text(format!("{temp}°C")).size(26),
        None => text("–").size(26).style(theme::muted),
    };
    let name = sensor.to_string().to_uppercase();
    let mut gauge = column![text(name).size(14).style(theme::muted), reading].spacing(2);
    if let Some(fan) = fan {
        gauge = gauge.push(
            column![
                text(match rpm {
                    Some(rpm) => format!("Fan {fan}% · {rpm} rpm"),
                    None => format!("Fan {fan}%"),
                })
                .size(14)
                .style(theme::muted),
                progress_bar(0.0..=100.0, f32::from(fan))
                    .girth(4)
                    .style(theme::duty),
            ]
            .spacing(6)
            .padding(iced::Padding::ZERO.top(6)),
        );
    }
    let gauge = gauge.push(container(graph).padding(iced::Padding::ZERO.top(10)));
    Some(gauge.width(Length::Fill).into())
}

fn charge_limit<'a>(percent: u8) -> Element<'a, Message> {
    let resumes = capitalize(&report::resumes(percent));
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
        text(resumes).size(14).style(theme::muted),
    ]
    .spacing(8)
    .into()
}

fn boost_marks(status: &Status) -> Option<(u8, u8)> {
    status.cooler_boost_cpu_c.zip(status.cooler_boost_gpu_c)
}

/// Where cooler boost comes on by itself, a slider per sensor.
fn marks_sliders<'a>((cpu, gpu): (u8, u8)) -> Element<'a, Message> {
    let mark = |name, fan, value: u8| {
        row![
            text(name).width(40),
            slider(COOLER_BOOST_MARKS, value, move |mark| {
                Message::BoostMarkDragged(fan, mark)
            })
            .on_release(Message::BoostMarksReleased)
            .style(theme::level),
            text(format!("{value}°C")).width(48).align_x(Alignment::End),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
    };
    column![
        mark("CPU", Fan::Cpu, cpu),
        mark("GPU", Fan::Gpu, gpu),
        text("On after 6 s at its mark, off after 10 s with both 10 °C under theirs")
            .size(14)
            .style(theme::muted),
    ]
    .spacing(8)
    .into()
}

/// A failure with no window to show it in, as after a pick in the tray,
/// where the item only goes back to what it was.
fn unseen(window: Option<window::Id>, result: &Result<(), String>) -> Option<String> {
    window.is_none().then(|| result.clone().err()).flatten()
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
    use iced_test::{Simulator, simulator};
    use ryukaze_core::lighting::Color;

    fn app(status: Status) -> App {
        App {
            session: true,
            demo: false,
            client: None,
            status: Some(Ok(status)),
            history: History::default(),
            charge_limit: None,
            error: None,
            boost_marks: None,
            dark: false,
            window: None,
            tray: None,
            curve_fan: Fan::Cpu,
            curve_draft: None,
            copied: false,
            lights: None,
            editing_lights: false,
            lighting_more: false,
            previews: Vec::new(),
            scenarios: None,
            editing_scenarios: false,
            power_color: None,
            power_hex: String::new(),
            previewing: false,
            next_preview: None,
            preview_refused: false,
            quitting: false,
            laptop: Some(Laptop::any()),
            page: Page::Overview,
            arrival: anim::arrived(),
            now: Instant::now(),
        }
    }

    /// The readme's window and keyboard light editor, as the demo's laptop
    /// has them after ten minutes, into target/shots: `cargo test -p
    /// ryukaze screenshots -- --ignored`, with `XDG_CONFIG_HOME` and
    /// `XDG_CONFIG_DIRS` at an empty directory for the built-in colours.
    #[test]
    #[ignore]
    fn screenshots() {
        use iced::{
            Size,
            advanced::renderer::{Headless as _, Style},
        };
        use iced_runtime::{UserInterface, user_interface};
        use resvg::tiny_skia::{IntSize, Pixmap};

        const SCALE: f32 = 2.0;
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        std::fs::create_dir_all(&out).unwrap();
        let mut renderer = crate::osd::software_renderer().unwrap();
        let laptop = Arc::new(Laptop {
            machine: crate::machine::Machine {
                model: Some("Titan 18 HX".into()),
                board: Some("MS-1824".into()),
                os: Some("NixOS".into()),
                nixos: true,
                ..Default::default()
            },
            layout: Arc::new(crate::layouts::built_in("ms-1824.toml")),
            logo: crate::bootlogo::read(),
        });

        // Ten minutes of readings, every two seconds, rising and falling.
        let status = crate::demo::status();
        let ten_minutes = || {
            let mut history = History::default();
            let start = Instant::now() - Duration::from_secs(600);
            for tick in 0..300u32 {
                let (cpu, gpu) = ((tick as f32 / 23.0).sin(), (tick as f32 / 31.0).cos());
                let mut now = status.clone();
                now.cpu_temp = Some((62.0 + 14.0 * cpu) as u8);
                now.cpu_fan = Some((45.0 + 20.0 * cpu) as u8);
                now.gpu_temp = Some((50.0 + 8.0 * gpu) as u8);
                now.gpu_fan = Some((35.0 + 10.0 * gpu) as u8);
                history.push(start + Duration::from_secs(2 * u64::from(tick)), &now);
            }
            history
        };

        // Drawn on the page's colour, cut below where it ends where `trim`.
        let mut shoot =
            |element: Element<'_, Message>, theme: &Theme, size: Size, name: &str, trim: bool| {
                let background = theme::colors(theme).background;
                let mut ui = UserInterface::build(
                    element,
                    size,
                    user_interface::Cache::default(),
                    &mut renderer,
                );
                ui.draw(
                    &mut renderer,
                    theme,
                    &Style {
                        text_color: theme::colors(theme).text,
                    },
                    mouse::Cursor::Unavailable,
                );
                let width = (size.width * SCALE) as u32;
                let mut height = (size.height * SCALE) as u32;
                let mut pixels = renderer.screenshot(Size::new(width, height), SCALE, background);
                if trim {
                    let page = background.into_rgba8();
                    let row = (width * 4) as usize;
                    let used = pixels
                        .chunks(row)
                        .rposition(|line| line.chunks(4).any(|pixel| pixel != page))
                        .map_or(0, |last| last + 1);
                    height = (used as u32 + (PAGE_PADDING * SCALE) as u32).min(height);
                    pixels.truncate(height as usize * row);
                }
                Pixmap::from_vec(pixels, IntSize::from_wh(width, height).unwrap())
                    .unwrap()
                    .save_png(out.join(name))
                    .unwrap();
            };
        for (dark, mode) in [(false, "light"), (true, "dark")] {
            let mut app = App {
                dark,
                history: ten_minutes(),
                laptop: Some(Arc::clone(&laptop)),
                demo: true,
                ..on(Page::Overview, status.clone())
            };
            let theme = app.theme(window::Id::unique());
            shoot(
                app.view(window::Id::unique()),
                &theme,
                Size::new(880.0, 680.0),
                &format!("window-{mode}.png"),
                false,
            );
            // The editor alone, over a wave.
            if let Some(Ok(status)) = &mut app.status {
                status.keyboard_profile = Some("wave".into());
            }
            let _ = app.update(Message::Page(Page::Lighting));
            let Some(Ok(status)) = &app.status else {
                unreachable!("the demo's status");
            };
            let (saved, standard) = keyboard_profiles(status);
            let known = lights::Known {
                standard,
                saved,
                shown: status.keyboard_profile.clone(),
                parts: lights::parts(status.lighting_parts.as_deref()),
                previewing: false,
            };
            // Every colour of the wave across the keys, one of them open.
            let wave = "name = \"wave\"\neffect = \"wave\"\norigin = \"left\"\nwavelength = 15\n\
                 colors = [\"#ff0000\", \"#ffff00\", \"#00ff00\", \"#00ffff\", \"#0000ff\", \"#ff00ff\"]\n".to_string();
            let mut editor = app.on_laptop(lights::Editor::from_toml(&wave).unwrap());
            let _ = editor.update(lights::Msg::Slot(lights::Slot::Round(0)));
            app.lights = Some(editor);
            let editor = app.lights.as_ref().unwrap();
            let width = 640.0;
            let element = container(editor.view(width, &known).map(Message::Lights))
                .padding(PAGE_PADDING)
                .width(width + 2.0 * PAGE_PADDING)
                .into();
            shoot(
                element,
                &theme,
                Size::new(width + 2.0 * PAGE_PADDING, 2400.0),
                &format!("lights-{mode}.png"),
                true,
            );
        }
    }

    /// Previews asked for while one is on its way: only the last goes,
    /// once it's there.
    #[test]
    fn sends_only_the_last_preview_of_a_run() {
        let mut app = on(Page::Lighting, alpha17());
        app.previewing = true;
        let _ = app.preview("a".into());
        let _ = app.preview("b".into());
        assert_eq!(app.next_preview.as_deref(), Some("b"));
        let _ = app.update(Message::Previewed(Ok(())));
        assert!(app.next_preview.is_none());
        assert_eq!(app.previews.last().map(String::as_str), Some("b"));
    }

    /// An unsaved edit the daemon took off the keyboard, for a new
    /// scenario, goes back on.
    #[test]
    fn shows_an_edit_again_when_the_daemon_drops_it() {
        let mut app = on(Page::Lighting, alpha17());
        app.lights = Some(lights::Editor::new());
        app.editing_lights = true;
        let sent = app.previews.len();
        let dropped = Status {
            keyboard_previewing: Some(false),
            ..alpha17()
        };
        let _ = app.update(Message::Status(Box::new(Ok(dropped))));
        assert_eq!(app.previews.len(), sent + 1);
        let shown = Status {
            keyboard_previewing: Some(true),
            ..alpha17()
        };
        let _ = app.update(Message::Status(Box::new(Ok(shown))));
        assert_eq!(app.previews.len(), sent + 1);
    }

    /// One turned down isn't sent again by the status it brings; the next
    /// edit tries.
    #[test]
    fn leaves_a_refused_preview_at_that() {
        let mut app = on(Page::Lighting, alpha17());
        app.lights = Some(lights::Editor::new());
        app.editing_lights = true;
        let _ = app.update(Message::Previewed(Err("denied".into())));
        let sent = app.previews.len();
        let dropped = Status {
            keyboard_previewing: Some(false),
            ..alpha17()
        };
        let _ = app.update(Message::Status(Box::new(Ok(dropped.clone()))));
        assert_eq!(app.previews.len(), sent);
        let _ = app.update(Message::Previewed(Ok(())));
        let _ = app.update(Message::Status(Box::new(Ok(dropped))));
        assert_eq!(app.previews.len(), sent + 1);
    }

    /// Quit with a preview on its way: the profile goes back after it,
    /// not never.
    #[test]
    fn quits_after_the_preview_on_its_way() {
        let mut app = titan_lighting();
        edit(&mut app);
        app.previewing = true;
        let _ = app.update(Message::Quit);
        assert_eq!(app.next_preview.as_deref(), Some(""));
        assert!(app.quitting);
        let _ = app.update(Message::Previewed(Ok(())));
        assert_eq!(app.previews.last().map(String::as_str), Some(""));
    }

    /// The window opened again on Lighting shows what was left unsaved.
    #[test]
    fn shows_a_kept_edit_when_the_window_comes_back() {
        let mut app = titan_lighting();
        edit(&mut app);
        let id = app.window.unwrap();
        let _ = app.update(Message::WindowClosed(id));
        assert!(!app.editing_lights);
        let _ = app.update(Message::Show);
        assert!(app.editing_lights);
        assert_eq!(app.previews.last(), app.previews.first());
    }

    /// A failed save shows in the scenarios' editor too.
    #[test]
    fn the_scenarios_editor_shows_errors() {
        let mut app = app(alpha17());
        app.scenarios = Some(scenarios::Editor::from_toml("").unwrap());
        app.editing_scenarios = true;
        app.error = Some("no such mode".into());
        let mut ui = simulator(app.view(window::Id::unique()));
        assert!(ui.find("no such mode").is_ok());
    }

    /// On `page`, as its row in the sidebar opens it.
    fn on(page: Page, status: Status) -> App {
        App {
            page,
            ..app(status)
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

    /// A window as tall as the page, for a click on anything on it.
    fn tall(app: &App) -> Simulator<'_, Message> {
        Simulator::with_size(
            iced::Settings::default(),
            (1024.0, 3000.0),
            app.view(window::Id::unique()),
        )
    }

    fn clicks(app: &App, targets: &[&str]) -> Vec<Message> {
        let mut ui = tall(app);
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
    fn picks_a_keyboard_profile() {
        let mut status = alpha17();
        status.keyboard_profiles = Some(["orange", "rainbow", "typing"].map(Into::into).into());
        status.keyboard_profile = Some("orange".into());
        let mut lighting = app(status);
        let _ = lighting.update(Message::Page(Page::Lighting));
        let messages = clicks(&lighting, &["Orange", "Rainbow"]);
        assert!(matches!(&messages[..], [
            Message::Lights(lights::Msg::Open(name))
        ] if name == "rainbow"));
        // Picked, and opened in the editor.
        for message in messages {
            let _ = lighting.update(message);
        }
        assert_eq!(lighting.shown_lights().as_deref(), Some("rainbow"));
        assert_eq!(lighting.lights.as_ref().unwrap().name(), "rainbow");
        assert!(lighting.previews.is_empty());
        // Without a controller, no Lighting.
        let without = app(alpha17());
        let mut ui = simulator(without.view(window::Id::unique()));
        assert!(ui.find("Lighting").is_err());
    }

    /// A Titan on the Lighting page's profile, `wave`, with its window.
    fn titan_lighting() -> App {
        let mut status = alpha17();
        status.keyboard_profiles = Some(["wave", "cycle", "mine"].map(Into::into).into());
        status.keyboard_saved_profiles = Some(vec!["mine".into()]);
        status.keyboard_profile = Some("wave".into());
        status.lighting_parts = Some(vec!["bar".into(), "logo".into()]);
        let mut app = app(status);
        app.window = Some(window::Id::unique());
        let _ = app.update(Message::Page(Page::Lighting));
        app
    }

    fn edit(app: &mut App) {
        let _ = app.update(Message::Lights(lights::Msg::Swatch(Color([255, 0, 0]))));
    }

    /// The page opened on the profile shown sends nothing; an edit is
    /// shown, and every way out takes it back, but only then.
    #[test]
    fn every_way_out_takes_the_preview_back() {
        let mut app = titan_lighting();
        assert_eq!(app.lights.as_ref().unwrap().name(), "wave");
        assert!(app.previews.is_empty() && !app.editing_lights);
        let _ = app.update(Message::Page(Page::Overview));
        let _ = app.update(Message::Page(Page::Lighting));
        assert!(app.previews.is_empty());

        type WayOut = (&'static str, fn(&mut App));
        let ways_out: [WayOut; 5] = [
            ("another page", |app| {
                let _ = app.update(Message::Page(Page::Cooling));
            }),
            ("the window closed", |app| {
                let id = app.window.unwrap();
                let _ = app.update(Message::WindowClosed(id));
            }),
            ("quit", |app| {
                let _ = app.update(Message::Quit);
            }),
            ("revert", |app| {
                let _ = app.update(Message::Lights(lights::Msg::Revert));
            }),
            ("another profile", |app| {
                let _ = app.update(Message::Lights(lights::Msg::Open("cycle".into())));
                let _ = app.update(Message::Lights(lights::Msg::Discard));
            }),
        ];
        for (way, out) in ways_out {
            let mut clean = titan_lighting();
            out(&mut clean);
            assert!(clean.previews.is_empty(), "{way}, nothing changed");

            let mut edited = titan_lighting();
            edit(&mut edited);
            assert!(edited.editing_lights);
            assert_eq!(edited.previews.len(), 1);
            assert!(!edited.previews[0].is_empty());
            out(&mut edited);
            assert_eq!(edited.previews[1..], [String::new()], "{way}");
            assert!(!edited.editing_lights, "{way}");
        }

        // Back on the page, the edit kept is shown again.
        let mut app = titan_lighting();
        edit(&mut app);
        let _ = app.update(Message::Page(Page::Battery));
        let _ = app.update(Message::Page(Page::Lighting));
        assert!(app.editing_lights);
        assert_eq!(app.previews.len(), 3);
        assert_eq!(app.previews[2], app.previews[0]);
        // Picked anew, the profile chosen.
        let _ = app.update(Message::Lights(lights::Msg::Open("cycle".into())));
        let _ = app.update(Message::Lights(lights::Msg::Discard));
        assert_eq!(app.shown_lights().as_deref(), Some("cycle"));
        let editor = app.lights.as_ref().unwrap();
        assert!(editor.name() == "cycle" && !editor.unsaved());
    }

    /// Saved, the daemon shows and picks it, the preview gone with it.
    #[test]
    fn saved_it_is_picked() {
        let mut app = titan_lighting();
        edit(&mut app);
        let _ = app.update(Message::Lights(lights::Msg::SaveAs("wave 2".into())));
        let _ = app.update(Message::LightsSaved(Ok(())));
        assert!(!app.editing_lights);
        assert_eq!(app.shown_lights().as_deref(), Some("wave 2"));
        assert!(!app.lights.as_ref().unwrap().unsaved());
        assert_eq!(app.previews.len(), 1);

        // Saved on the way to another: there after.
        edit(&mut app);
        let _ = app.update(Message::Lights(lights::Msg::Open("cycle".into())));
        let _ = app.update(Message::Lights(lights::Msg::SaveAndLeave(None)));
        assert!(app.editing_lights);
        let _ = app.update(Message::LightsSaved(Ok(())));
        assert_eq!(app.shown_lights().as_deref(), Some("cycle"));
        assert_eq!(app.lights.as_ref().unwrap().name(), "cycle");
        assert!(!app.editing_lights);

        // Not saved: the edit stays on the keyboard, to take back.
        edit(&mut app);
        let _ = app.update(Message::Lights(lights::Msg::Save));
        let _ = app.update(Message::LightsSaved(Err("no".into())));
        assert!(app.editing_lights && app.lights.as_ref().unwrap().unsaved());
    }

    /// The profile changed by the daemon: a clean editor follows it, an
    /// edit is shown again over it while on the page, and only then.
    #[test]
    fn follows_the_keyboard_unless_edited() {
        let modes = |profile: &str| {
            Message::ModesChanged(Box::new(Status {
                keyboard_profile: Some(profile.into()),
                keyboard_profiles: Some(["wave", "cycle", "mine"].map(Into::into).into()),
                ..Status::default()
            }))
        };
        let mut app = titan_lighting();
        let _ = app.update(modes("cycle"));
        assert_eq!(app.lights.as_ref().unwrap().name(), "cycle");
        assert!(app.previews.is_empty());

        edit(&mut app);
        let _ = app.update(modes("wave"));
        let editor = app.lights.as_ref().unwrap();
        assert!(editor.name() == "cycle" && editor.unsaved());
        assert_eq!(app.previews.len(), 2);
        assert_eq!(app.previews[1], app.previews[0]);

        let _ = app.update(Message::Page(Page::Overview));
        let _ = app.update(modes("cycle"));
        assert_eq!(app.previews.len(), 3);
        assert!(!app.editing_lights);
    }

    /// Without the daemon, the page still shows the profile picked.
    #[test]
    fn the_demo_opens_its_profile() {
        let mut demo = App {
            demo: true,
            ..app(crate::demo::status())
        };
        let _ = demo.update(Message::Page(Page::Lighting));
        assert_eq!(demo.lights.as_ref().unwrap().name(), "breathe");
        let messages = clicks(&demo, &["Static 2"]);
        for message in messages {
            let _ = demo.update(message);
        }
        assert_eq!(demo.shown_lights().as_deref(), Some("Static 2"));
        assert_eq!(demo.lights.as_ref().unwrap().name(), "Static 2");
    }

    #[test]
    fn automation_folded_away() {
        let mut status = alpha17();
        status.keyboard_profiles = Some(vec!["wave".into()]);
        status.keyboard_profile = Some("wave".into());
        status.numpad_follows_numlock = Some(false);
        let mut app = app(status);
        let _ = app.update(Message::Page(Page::Lighting));
        let numpad = "Numpad dark while NumLock is off";
        assert!(tall(&app).find(numpad).is_err());
        let messages = clicks(&app, &["Automation"]);
        assert!(matches!(&messages[..], [Message::LightingMore]));
        let _ = app.update(Message::LightingMore);
        assert!(tall(&app).find(numpad).is_ok());
    }

    #[test]
    fn picks_a_scenario() {
        let mut status = alpha17();
        status.scenarios = Some(["silent", "balanced", "performance"].map(Into::into).into());
        status.scenario = Some("balanced".into());
        // The one in force isn't picked again.
        let messages = clicks(&app(status), &["Balanced", "Performance"]);
        assert!(matches!(&messages[..], [
            Message::Scenario(name)
        ] if name == "performance"));
    }

    #[test]
    fn the_current_mode_is_not_picked_again() {
        assert!(clicks(&app(alpha17()), &["Comfort", "Auto"]).is_empty());
    }

    #[test]
    fn toggles_by_label() {
        let messages = clicks(&on(Page::Cooling, alpha17()), &["Cooler boost"]);
        assert!(matches!(&messages[..], [Message::CoolerBoost(true)]));
        let messages = clicks(&on(Page::Battery, alpha17()), &["Super battery"]);
        assert!(matches!(&messages[..], [Message::SuperBattery(true)]));
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
        let holding = on(Page::Battery, held);
        let mut ui = simulator(holding.view(window::Id::unique()));
        for shown in ["91%", "Held at the charge limit", "Health 96%"] {
            assert!(ui.find(shown).is_ok(), "{shown}");
        }

        let charging = on(
            Page::Battery,
            Status {
                on_ac: Some(true),
                battery_level: Some(40),
                battery_state: Some("Charging".into()),
                battery_watts: Some(44.6),
                ..alpha17()
            },
        );
        let mut ui = simulator(charging.view(window::Id::unique()));
        assert!(ui.find("Charging · 45 W").is_ok());
    }

    #[test]
    fn takes_the_modes_ryukazed_announces() {
        let mut app = app(alpha17());
        let _ = app.update(Message::ModesChanged(Box::new(Status {
            shift_mode: Some("eco".into()),
            fan_mode: Some("silent".into()),
            cooler_boost: Some(true),
            scenario: Some("silent".into()),
            ..Status::default()
        })));
        let Some(Ok(status)) = &app.status else {
            panic!("no status");
        };
        assert_eq!(status.shift_mode.as_deref(), Some("eco"));
        assert_eq!(status.cooler_boost, Some(true));
        assert_eq!(status.scenario.as_deref(), Some("silent"));
        // The rest stays as polled.
        assert_eq!(status.charge_limit, Some(80));
    }

    #[test]
    fn usb_share_switch() {
        let status = Status {
            usb_share: Some(false),
            ..alpha17()
        };
        let messages = clicks(&on(Page::Battery, status), &["Charge USB when off"]);
        assert!(matches!(&messages[..], [Message::UsbShare(true)]));
    }

    #[test]
    fn cooler_boost_marks() {
        let status = Status {
            cooler_boost: Some(false),
            auto_cooler_boost: Some(true),
            cooler_boost_cpu_c: Some(95),
            cooler_boost_gpu_c: Some(85),
            ..alpha17()
        };
        let shown =
            |app: &App, text: &str| simulator(app.view(window::Id::unique())).find(text).is_ok();

        let mut marked = on(Page::Cooling, status.clone());
        assert!(shown(&marked, "95°C") && shown(&marked, "85°C"));
        let _ = marked.update(Message::BoostMarkDragged(Fan::Cpu, 90));
        assert!(shown(&marked, "90°C"));
        let _ = marked.update(Message::BoostMarksReleased);
        let Some(Ok(status_now)) = &marked.status else {
            panic!("no status");
        };
        assert_eq!(boost_marks(status_now), Some((90, 85)));
        assert!(marked.boost_marks.is_none());

        // Only while it boosts by itself.
        let off = on(
            Page::Cooling,
            Status {
                auto_cooler_boost: Some(false),
                ..status
            },
        );
        assert!(!shown(&off, "95°C"));
    }

    #[test]
    fn gpu_mode_after_a_restart() {
        let mode = |now: &str, next: &str| Status {
            gpu_mode: Some(now.into()),
            gpu_mode_next: Some(next.into()),
            ..alpha17()
        };
        let system = |status| on(Page::System, status);
        let messages = clicks(&system(mode("hybrid", "hybrid")), &["Discrete", "Hybrid"]);
        assert!(matches!(&messages[..], [Message::GpuMode(mode)] if mode == "discrete"));

        let switching = system(mode("hybrid", "discrete"));
        let mut ui = simulator(switching.view(window::Id::unique()));
        assert!(ui.find("Switches to discrete at the next restart").is_ok());
        // Back to where it is now takes the switch back.
        let messages = clicks(&switching, &["Hybrid"]);
        assert!(matches!(&messages[..], [Message::GpuMode(mode)] if mode == "hybrid"));

        assert!(
            simulator(system(alpha17()).view(window::Id::unique()))
                .find("Discrete")
                .is_err()
        );
        assert!(
            simulator(system(mode("hybrid", "hybrid")).view(window::Id::unique()))
                .find("Integrated")
                .is_err()
        );
    }

    #[test]
    fn gpu_modes_the_laptop_has() {
        let titan = |now: &str, next: &str, modes: &[&str]| {
            on(
                Page::System,
                Status {
                    gpu_mode: Some(now.into()),
                    gpu_mode_next: Some(next.into()),
                    gpu_modes: Some(modes.iter().map(|&mode| mode.into()).collect()),
                    ..alpha17()
                },
            )
        };
        let all = ["hybrid", "discrete", "integrated"];
        let messages = clicks(&titan("discrete", "discrete", &all), &["Integrated"]);
        assert!(matches!(&messages[..], [Message::GpuMode(mode)] if mode == "integrated"));

        let integrated = titan("integrated", "integrated", &all);
        let mut ui = simulator(integrated.view(window::Id::unique()));
        assert!(
            ui.find("The iGPU drives the screen; the dGPU is off")
                .is_ok()
        );

        let two = titan("discrete", "discrete", &["hybrid", "discrete"]);
        assert!(
            simulator(two.view(window::Id::unique()))
                .find("Integrated")
                .is_err()
        );
    }

    #[test]
    fn says_who_keeps_the_dgpu_up() {
        let dgpu = |awake: bool, secs: u64, users: &[&str]| Status {
            dgpu_awake: Some(awake),
            dgpu_for_secs: Some(secs),
            dgpu_users: awake.then(|| users.iter().map(|&user| user.to_owned()).collect()),
            ..alpha17()
        };
        let shown = |status: Status, line: &str| {
            simulator(on(Page::System, status).view(window::Id::unique()))
                .find(line)
                .is_ok()
        };
        assert!(shown(dgpu(false, 7500, &[]), "dGPU asleep for 2 h 5 min"));
        assert!(shown(
            dgpu(true, 200, &["firefox", "steam"]),
            "dGPU awake for 3 min, held by firefox, steam"
        ));
        assert!(shown(dgpu(true, 4, &[]), "dGPU awake for 4 s"));
        // No dGPU: no Graphics either, without a MUX.
        assert!(!shown(alpha17(), "Graphics"));
    }

    #[test]
    fn camera_and_fn_key() {
        let status = Status {
            webcam: Some(true),
            fn_key: Some("right".into()),
            ..alpha17()
        };
        let messages = clicks(
            &on(Page::System, status),
            &["Camera", "Left of Win", "Right of Win"],
        );
        // The side it's on already isn't picked again.
        assert!(matches!(
            &messages[..],
            [Message::Webcam(false), Message::FnKey(side)] if side == "left"
        ));
        let none = on(Page::System, alpha17());
        let mut ui = simulator(none.view(window::Id::unique()));
        assert!(ui.find("Camera").is_err());
    }

    #[test]
    fn cooler_boost_when_hot() {
        let status = Status {
            auto_cooler_boost: Some(false),
            ..alpha17()
        };
        let messages = clicks(
            &on(Page::Cooling, status),
            &["Cooler boost when hot", "Cooler boost"],
        );
        assert!(matches!(
            &messages[..],
            [Message::AutoCoolerBoost(true), Message::CoolerBoost(true)]
        ));
        // Not offered where the daemon doesn't say.
        let app = on(Page::Cooling, alpha17());
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
        let editing = on(Page::Cooling, advanced());
        let mut ui = simulator(editing.view(window::Id::unique()));
        assert!(ui.find("CPU").is_ok());
        assert!(ui.find("Reset").is_ok());

        let auto = on(Page::Cooling, alpha17());
        let mut ui = simulator(auto.view(window::Id::unique()));
        assert!(ui.find("Reset").is_err());
    }

    #[test]
    fn adaptive_and_its_gpu_switch() {
        let messages = clicks(
            &on(Page::Cooling, advanced()),
            &["GPU fan helps the CPU", "Adaptive"],
        );
        assert!(matches!(
            &messages[..],
            [Message::GpuFollowsCpu(false), Message::AdaptiveFans(false)]
        ));

        // Stepping: no GPU switch, the EC goes its own way there.
        let stepping = on(
            Page::Cooling,
            Status {
                adaptive_fans: Some(false),
                ..advanced()
            },
        );
        let mut ui = simulator(stepping.view(window::Id::unique()));
        assert!(ui.find("Adaptive").is_ok());
        assert!(ui.find("GPU fan helps the CPU").is_err());
    }

    /// In a narrow window, where the System page has the config's line.
    #[test]
    fn copies_the_settings_as_nix() {
        fn narrow(app: &App) -> Simulator<'_, Message> {
            Simulator::with_size(
                iced::Settings::default(),
                (600.0, 768.0),
                app.view(window::Id::unique()),
            )
        }
        let mut app = on(Page::System, alpha17());
        app.laptop = Some(Arc::new(Laptop {
            machine: crate::machine::Machine {
                nixos: true,
                ..Default::default()
            },
            ..Arc::unwrap_or_clone(Laptop::any())
        }));
        {
            let mut ui = narrow(&app);
            ui.click("Copy as Nix").unwrap();
            let messages: Vec<Message> = ui.into_messages().collect();
            assert!(matches!(&messages[..], [Message::CopyAsNix]));
        }

        let shown = |app: &App, text: &str| narrow(app).find(text).is_ok();
        let _ = app.update(Message::CopyAsNix);
        assert!(shown(&app, "Copied, for services.ryukaze.settings"));
        // A change after the copy isn't in it.
        let _ = app.update(Message::Applied(Ok(())));
        assert!(shown(&app, "Changes here are kept"));
        let mut status = alpha17();
        status.config_keys = Some(
            ["cooler_boost.auto", "charge_limit", "cooler_boost.cpu_c"]
                .map(Into::into)
                .into(),
        );
        assert_eq!(
            kept_hint(&status),
            "Changes here are kept; the config sets cooler boost when hot, the charge limit and its marks again at every start"
        );
    }

    #[test]
    fn a_profile_not_read_leaves_the_editor_as_it_is() {
        let mut app = app(alpha17());
        let profile = "name = \"mine\"\neffect = \"static\"\n".to_owned();
        let _ = app.update(Message::LightsRead(Ok(profile)));
        let _ = app.update(Message::LightsRead(Err("gone".into())));
        assert_eq!(app.lights.as_ref().unwrap().name(), "mine");
        assert_eq!(app.error.as_deref(), Some("gone"));
    }

    #[test]
    fn says_a_failure_with_no_window_to_show_it() {
        let failed = Err("shift_mode: no".to_owned());
        assert_eq!(unseen(None, &failed), Some("shift_mode: no".to_owned()));
        assert_eq!(unseen(Some(window::Id::unique()), &failed), None);
        assert_eq!(unseen(None, &Ok(())), None);
    }

    /// Shows a real notification: `cargo test -p ryukaze notifies --
    /// --ignored`, to see the server takes what `tray::notify` sends.
    #[test]
    #[ignore]
    fn notifies() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(tray::notify("A test of ryukaze's notification".into()));
    }

    #[test]
    fn switches_fans_and_resets() {
        let messages = clicks(&on(Page::Cooling, advanced()), &["GPU", "Reset"]);
        assert!(matches!(
            &messages[..],
            [Message::CurveFan(Fan::Gpu), Message::CurveReset]
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
