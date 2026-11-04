//! The window's pages: where there's room, a sidebar of them beside the
//! one shown; where there isn't, the overview, with a row for each of the
//! others and a way back from them. Each page is a few groups, a card of
//! rows each, as GNOME's settings lay them out.

use iced::{
    Alignment, Element, Length, Padding, Theme,
    widget::{Stack, button, canvas, column, container, row, rule, scrollable, space, text},
};
use ryukaze_core::{Status, fans::Fan};

use super::{
    App, FAN_ORDER, Message, PAGE_PADDING, SHIFT_ORDER, banner, battery_summary, boost_marks,
    capitalize, cells, charge_limit, config_sets, dgpu, fn_key, gauge, gpu_mode, grid, kept_hint,
    keyboard_profiles, marks_sliders, modes_row, power_colors, power_light, scenario_lights,
    small_button, source_pick, switch,
};
use crate::anim::Hover;
use crate::{
    anim,
    history::{self, Series},
    lights,
    machine::Machine,
    report, scenarios, theme, tip,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Overview,
    Cooling,
    Battery,
    Lighting,
    System,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Overview,
        Page::Cooling,
        Page::Battery,
        Page::Lighting,
        Page::System,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Cooling => "Cooling",
            Page::Battery => "Battery",
            Page::Lighting => "Lighting",
            Page::System => "System",
        }
    }

    /// Whether the laptop has anything for the page.
    pub fn shown(self, status: &Status) -> bool {
        match self {
            Page::Overview | Page::System => true,
            Page::Cooling => {
                status.cpu_temp.is_some()
                    || status.fan_mode.is_some()
                    || status.cooler_boost.is_some()
            }
            Page::Battery => {
                status.battery_level.is_some()
                    || status.charge_limit.is_some()
                    || status.super_battery.is_some()
            }
            Page::Lighting => status.keyboard_profiles.is_some(),
        }
    }

    /// One line on what the page holds now, for its row on the overview.
    fn summary(self, status: &Status) -> String {
        let parts: Vec<String> = match self {
            Page::Overview => Vec::new(),
            Page::Cooling => [
                status.fan_mode.as_deref().map(capitalize),
                status.cpu_fan.map(|duty| format!("CPU fan {duty}%")),
                status.gpu_fan.map(|duty| format!("GPU fan {duty}%")),
                (status.cooler_boost == Some(true)).then(|| "cooler boost".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect(),
            Page::Battery => [
                status.battery_level.map(|level| format!("{level}%")),
                status.charge_limit.map(|limit| format!("limit {limit}%")),
                status
                    .battery_health
                    .map(|health| format!("health {health}%")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            Page::Lighting => status
                .keyboard_profile
                .as_deref()
                .map(capitalize)
                .into_iter()
                .collect(),
            Page::System => [
                status.gpu_mode_next.as_deref().map(capitalize),
                status
                    .webcam
                    .map(|on| format!("camera {}", if on { "on" } else { "off" })),
                (status.local_dimming == Some(true)).then(|| "local dimming".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        parts.join(" · ")
    }
}

/// Wider than this, the pages go in a sidebar.
const WIDE: f32 = 720.0;
const SIDEBAR: f32 = 220.0;
/// A page's groups are no wider than this, the editors as wide as there is.
const CONTENT_MAX: f32 = 760.0;
/// The Lighting page's, its effect beside the picture from 900.
const LIGHTING_MAX: f32 = 960.0;

/// The page's scroller, to bring another page in at its top.
pub(super) fn scroller() -> iced::widget::Id {
    iced::widget::Id::new("page")
}

impl App {
    pub(super) fn layout(&self, width: f32) -> Element<'_, Message> {
        let status = match &self.status {
            None => {
                return container(text("Connecting…").style(theme::muted))
                    .center(Length::Fill)
                    .into();
            }
            Some(Err(err)) => {
                return container(banner(err))
                    .padding(PAGE_PADDING)
                    .width(Length::Fill)
                    .into();
            }
            Some(Ok(status)) => status,
        };
        let page = if self.page.shown(status) {
            self.page
        } else {
            Page::Overview
        };
        let wide = width >= WIDE;
        let inside = if wide { width - SIDEBAR - 1.0 } else { width } - 2.0 * PAGE_PADDING;
        let max = if self.editing_scenarios {
            f32::INFINITY
        } else if page == Page::Lighting {
            LIGHTING_MAX
        } else {
            CONTENT_MAX
        };
        let content = container(self.content(status, page, wide, inside.min(max))).max_width(max);
        // A page just shown fades in, rising into place.
        let shown = self.arrival.interpolate(0.0, 1.0, self.now);
        let rise = (1.0 - shown) * anim::RISE;
        let content = scrollable(
            container(content)
                .padding(
                    Padding::new(PAGE_PADDING)
                        .top(PAGE_PADDING + rise)
                        .bottom(PAGE_PADDING - rise),
                )
                .center_x(Length::Fill),
        )
        .id(scroller())
        .style(theme::scroller)
        .height(Length::Fill);
        let veil = container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |theme: &Theme| container::Style {
                background: Some(
                    iced::Color {
                        a: 1.0 - shown,
                        ..theme::colors(theme).background
                    }
                    .into(),
                ),
                ..container::Style::default()
            });
        let content = Stack::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .push(content)
            .push(veil);
        if wide {
            row![
                self.sidebar(status, page),
                rule::vertical(1).style(theme::divider),
                content
            ]
            .into()
        } else {
            content.into()
        }
    }

    fn sidebar<'a>(&'a self, status: &'a Status, shown: Page) -> Element<'a, Message> {
        let mut pages = column![
            container(text("ryukaze").size(18).font(theme::heading())).padding([6, 10]),
            space().height(8),
        ]
        .spacing(2);
        for page in Page::ALL.into_iter().filter(|page| page.shown(status)) {
            let chosen = page == shown;
            let unsaved =
                page == Page::Lighting && self.lights.as_ref().is_some_and(lights::Editor::unsaved);
            let bar = container(space().width(3).height(18)).style(move |theme| {
                if chosen {
                    theme::accent_bar(theme)
                } else {
                    container::Style::default()
                }
            });
            let label = if unsaved {
                format!("{} •", page.title())
            } else {
                page.title().to_owned()
            };
            pages = pages.push(
                button(
                    row![bar, text(label)]
                        .spacing(10)
                        .align_y(Alignment::Center),
                )
                .on_press(Message::Page(page))
                .padding([8, 8])
                .width(Length::Fill)
                .hover(theme::nav(chosen)),
            );
        }
        pages = pages.push(space::vertical());
        if let Some(laptop) = &self.laptop {
            pages = pages
                .push(rule::horizontal(1).style(theme::divider))
                .push(container(about(status, &laptop.machine)).padding([10, 8]));
        }
        // At the very bottom, with its line, on NixOS only.
        if self.nixos() {
            let kept = if self.copied {
                "Copied, for services.ryukaze.settings".to_owned()
            } else {
                kept_hint(status)
            };
            let copy = button(text("Copy as Nix").width(Length::Fill).center())
                .on_press(Message::CopyAsNix)
                .padding([6, 10])
                .width(Length::Fill)
                .hover(theme::segment(false));
            pages = pages
                .push(rule::horizontal(1).style(theme::divider))
                .push(space().height(6))
                .push(tip::tooltip(copy, kept, tip::Side::Above));
        }
        container(pages)
            .width(SIDEBAR)
            .height(Length::Fill)
            .padding(10)
            .style(theme::sidebar)
            .into()
    }

    fn content<'a>(
        &'a self,
        status: &'a Status,
        page: Page,
        wide: bool,
        width: f32,
    ) -> Element<'a, Message> {
        if self.editing_scenarios
            && let Some(editor) = &self.scenarios
        {
            let modes = scenarios::Modes {
                shift: status.shift_modes.as_deref().unwrap_or_default(),
                fan: status.fan_modes.as_deref().unwrap_or_default(),
                super_battery: status.super_battery.is_some(),
            };
            let editor = editor.view(width, &modes).map(Message::Scenarios);
            return match &self.error {
                Some(err) => column![banner(err), editor].spacing(20).into(),
                None => editor,
            };
        }

        let title = text(page.title()).size(24).font(theme::heading());
        let header: Element<'a, Message> = if !wide && page != Page::Overview {
            row![
                button(text("‹ Overview").size(14))
                    .on_press(Message::Page(Page::Overview))
                    .padding([4, 8])
                    .hover(theme::link),
                space::horizontal(),
                title,
            ]
            .align_y(Alignment::Center)
            .into()
        } else {
            title.into()
        };
        let mut body = column![header].spacing(20);
        if let Some(err) = &self.error {
            body = body.push(banner(err));
        }
        let groups = match page {
            Page::Overview => self.overview(status, wide, width),
            Page::Cooling => self.cooling(status),
            Page::Battery => self.battery(status),
            Page::Lighting => self.lighting(status, width),
            Page::System => self.system(status, wide),
        };
        body.extend(groups).into()
    }

    /// What's glanced at and changed every day; a row for each other page.
    fn overview<'a>(
        &'a self,
        status: &'a Status,
        wide: bool,
        width: f32,
    ) -> Vec<Element<'a, Message>> {
        let mut groups = Vec::new();

        let fan = |duty: Option<u8>, rpm: Option<u16>| match (duty, rpm) {
            (Some(duty), Some(rpm)) => format!("Fan {duty}% · {rpm} rpm"),
            (Some(duty), None) => format!("Fan {duty}%"),
            _ => String::new(),
        };
        let mut tiles: Vec<Element<'a, Message>> = Vec::new();
        if let Some(temp) = status.cpu_temp {
            tiles.push(stat(
                "CPU",
                format!("{temp}°C"),
                fan(status.cpu_fan, status.cpu_rpm),
                None,
                Page::Cooling,
            ));
        }
        if let Some(temp) = status.gpu_temp {
            // Who keeps the dGPU up says more than its fan, asleep or awake.
            let line = match (status.dgpu_awake, status.dgpu_users.as_deref()) {
                (Some(true), Some([only])) => {
                    format!("Held by {only}")
                }
                (Some(true), Some([first, rest @ ..])) => {
                    format!("Held by {first} +{}", rest.len())
                }
                (Some(true), _) => "Awake".to_owned(),
                (Some(false), _) => "Asleep".to_owned(),
                (None, _) => fan(status.gpu_fan, status.gpu_rpm),
            };
            let full = report::dgpu_state(status).map(|state| capitalize(&state));
            let value = match temp {
                0 => "Asleep".to_owned(),
                temp => format!("{temp}°C"),
            };
            tiles.push(stat("GPU", value, line, full, Page::Cooling));
        }
        if let Some(level) = status.battery_level {
            let full = report::battery_doing(status).map(|doing| capitalize(&doing));
            let short = full
                .as_deref()
                .map(|doing| doing.replace("Held at the charge limit", "Held at limit"))
                .unwrap_or_default();
            tiles.push(stat(
                "Battery",
                format!("{level}%"),
                short,
                full,
                Page::Battery,
            ));
        }
        if let Some(profile) = &status.keyboard_profile {
            let previewing =
                self.editing_lights && self.lights.as_ref().is_some_and(lights::Editor::unsaved);
            let line = if previewing {
                "Previewing an edit".to_owned()
            } else {
                status
                    .scenario
                    .as_deref()
                    .filter(|scenario| {
                        status
                            .keyboard_scenarios
                            .as_ref()
                            .is_some_and(|own| own.contains_key(*scenario))
                    })
                    .map(|scenario| format!("For {}", capitalize(scenario)))
                    .unwrap_or_else(|| "All scenarios".to_owned())
            };
            tiles.push(stat(
                "Keyboard",
                capitalize(profile),
                line,
                None,
                Page::Lighting,
            ));
        }
        // Four across where they fit, two where they don't.
        let across = if width >= 560.0 {
            tiles.len().max(1)
        } else {
            2
        };
        // Titled as every other page's first group, so nothing moves between pages.
        groups.push(
            column![
                container(text("Now").size(14).font(theme::heading())).padding([0, 4]),
                cells(tiles, across),
            ]
            .spacing(8)
            .into(),
        );

        let mut modes: Vec<Element<'a, Message>> = Vec::new();
        if let Some(scenarios) = &status.scenarios {
            let current = status.scenario.as_deref().unwrap_or_default();
            modes.push(labelled_control(
                "Scenario",
                grid(current, scenarios, 3, Message::Scenario),
            ));
        }
        if let (Some(current), Some(shift)) = (&status.shift_mode, &status.shift_modes) {
            modes.push(labelled_control(
                "Performance",
                modes_row(current, shift, SHIFT_ORDER, Message::ShiftMode),
            ));
        }
        if let (Some(current), Some(fan)) = (&status.fan_mode, &status.fan_modes) {
            modes.push(labelled_control(
                "Fans",
                modes_row(current, fan, FAN_ORDER, Message::FanMode),
            ));
        }
        if let Some(on) = status.cooler_boost {
            modes.push(switch("Cooler boost", on, Message::CoolerBoost));
        }
        groups.extend(group("Modes", modes));

        // The sidebar has them where there's room.
        if !wide {
            let links = Page::ALL
                .into_iter()
                .filter(|page| *page != Page::Overview && page.shown(status))
                .map(|page| link_row(page, page.summary(status)))
                .collect();
            groups.extend(group("More", links));
        }
        groups
    }

    fn cooling<'a>(&'a self, status: &'a Status) -> Vec<Element<'a, Message>> {
        let mut groups = Vec::new();

        let gauges: Vec<Element<'a, Message>> = [
            (Fan::Cpu, status.cpu_temp, status.cpu_fan, status.cpu_rpm),
            (Fan::Gpu, status.gpu_temp, status.gpu_fan, status.gpu_rpm),
        ]
        .into_iter()
        .filter_map(|(fan, temp, duty, rpm)| {
            let graph = canvas(history::Graph {
                history: &self.history,
                series: Series::Sensor(fan),
            })
            .width(Length::Fill)
            .height(64);
            gauge(fan, temp, duty, rpm, graph)
        })
        .collect();
        if !gauges.is_empty() {
            groups.extend(group(
                "Sensors",
                vec![
                    column![
                        row(gauges).spacing(24),
                        hint("The temperature, and the fan fainter, both from 0 to 100"),
                    ]
                    .spacing(10)
                    .into(),
                ],
            ));
        }

        let mut fans: Vec<Element<'a, Message>> = Vec::new();
        if let (Some(current), Some(modes)) = (&status.fan_mode, &status.fan_modes) {
            fans.push(labelled_control(
                "Mode",
                modes_row(current, modes, FAN_ORDER, Message::FanMode),
            ));
        }
        if let Some(on) = status.cooler_boost {
            fans.push(switch("Cooler boost", on, Message::CoolerBoost));
        }
        if let Some(on) = status.auto_cooler_boost {
            fans.push(switch(
                "Cooler boost when hot",
                on,
                Message::AutoCoolerBoost,
            ));
            if on && let Some(marks) = self.boost_marks.or(boost_marks(status)) {
                fans.push(marks_sliders(marks));
            }
        }
        groups.extend(group("Fans", fans));

        let advanced = status.fan_mode.as_deref() == Some("advanced");
        let curve = if advanced {
            self.curve_editor(status)
        } else if status
            .fan_modes
            .as_ref()
            .is_some_and(|modes| modes.iter().any(|mode| mode == "advanced"))
        {
            Some(
                row![
                    text("The fan curves apply in Advanced")
                        .style(theme::muted)
                        .width(Length::Fill),
                    small_button("Use Advanced", Message::FanMode("advanced".into())),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .into(),
            )
        } else {
            None
        };
        groups.extend(group("Fan curves", curve.into_iter().collect()));
        groups
    }

    fn battery<'a>(&'a self, status: &'a Status) -> Vec<Element<'a, Message>> {
        let mut groups = Vec::new();
        // The figures and their graph as one row, as the sensors are.
        let mut now = column![].spacing(12);
        if let Some(summary) = battery_summary(status) {
            now = now.push(summary);
        }
        if status.battery_watts.is_some() {
            let top = self.history.top(Series::Power);
            now = now.push(
                column![
                    canvas(history::Graph {
                        history: &self.history,
                        series: Series::Power,
                    })
                    .width(Length::Fill)
                    .height(64),
                    hint(format!(
                        "Watts out, and in fainter; the top line is {top:.0} W"
                    )),
                ]
                .spacing(10),
            );
        }
        if status.battery_level.is_some() || status.battery_watts.is_some() {
            groups.extend(group("Now", vec![now.into()]));
        }

        let mut charging: Vec<Element<'a, Message>> = Vec::new();
        if let Some(limit) = self.charge_limit.or(status.charge_limit) {
            charging.push(charge_limit(limit));
        }
        if let Some(on) = status.super_battery {
            charging.push(switch("Super battery", on, Message::SuperBattery));
        }
        if let Some(on) = status.usb_share {
            charging.push(switch("Charge USB when off", on, Message::UsbShare));
        }
        groups.extend(group("Charging", charging));
        groups
    }

    /// The keyboard light's editor over the profile on the keyboard, and
    /// what lights it by itself, folded away.
    fn lighting<'a>(&'a self, status: &'a Status, width: f32) -> Vec<Element<'a, Message>> {
        let mut groups = Vec::new();
        let (saved, standard) = keyboard_profiles(status);

        let known = lights::Known {
            standard: standard.clone(),
            saved: saved.clone(),
            shown: status.keyboard_profile.clone(),
            parts: lights::parts(status.lighting_parts.as_deref()),
            previewing: self.editing_lights,
        };
        groups.push(match (&self.lights, &self.laptop) {
            (_, None) => hint("Reading the laptop").into(),
            (Some(editor), Some(_)) => editor.view(width, &known).map(Message::Lights),
            (None, _) => hint("Reading the profile on the keyboard").into(),
        });

        let mut more: Vec<Element<'a, Message>> = Vec::new();
        if let (Some(scenarios), Some(own)) = (&status.scenarios, &status.keyboard_scenarios) {
            more.push(labelled_control(
                "In the scenarios",
                scenario_lights(scenarios, own, &standard, &saved),
            ));
        }
        if let Some(follows) = &status.power_light {
            let hybrid = status.dgpu_awake.is_some();
            let mut power: Vec<Element<'a, Message>> = vec![power_light(follows, hybrid)];
            if let Some(on) = status.power_light_dgpu
                && hybrid
                && follows == "shift_mode"
            {
                power.push(switch(
                    "Show the dGPU waking, for five seconds",
                    on,
                    Message::PowerLightDgpu,
                ));
            }
            if follows != "profile" {
                power.push(power_colors(
                    status,
                    follows,
                    self.power_color.as_deref(),
                    &self.power_hex,
                ));
            }
            more.push(labelled_control(
                "Power button",
                iced::widget::Column::with_children(power).spacing(12),
            ));
        }
        if let Some(on) = status.numpad_follows_numlock {
            more.push(switch(
                "Numpad dark while NumLock is off",
                on,
                Message::NumpadFollowsNumlock,
            ));
        }
        if !more.is_empty() {
            let open = self.lighting_more;
            groups.push(
                button(
                    row![
                        text("Automation").size(14).font(theme::heading()),
                        space::horizontal(),
                        hint(if open { "Hide" } else { "Show" }),
                    ]
                    .align_y(Alignment::Center),
                )
                .on_press(Message::LightingMore)
                .padding([4, 4])
                .width(Length::Fill)
                .hover(theme::link),
            );
            if open {
                groups.push(card(more));
            }
        }
        groups
    }

    fn system<'a>(&'a self, status: &'a Status, wide: bool) -> Vec<Element<'a, Message>> {
        let mut groups = Vec::new();

        let mut graphics: Vec<Element<'a, Message>> = Vec::new();
        if let (Some(now), Some(next)) = (&status.gpu_mode, &status.gpu_mode_next) {
            graphics.push(gpu_mode(now, next, &report::gpu_modes(status)));
        }
        if let Some(dgpu) = dgpu(status) {
            graphics.push(dgpu);
        }
        groups.extend(group("Graphics", graphics));

        let mut devices: Vec<Element<'a, Message>> = Vec::new();
        if let Some(on) = status.webcam {
            devices.push(switch("Camera", on, Message::Webcam));
        }
        if let Some(side) = &status.fn_key {
            devices.push(fn_key(side));
        }
        if let Some(on) = status.local_dimming {
            devices.push(switch("Local dimming", on, Message::LocalDimming));
        }
        groups.extend(group("Devices", devices));

        let mut automation: Vec<Element<'a, Message>> = Vec::new();
        if let Some(scenarios) = &status.scenarios {
            let sources = [
                ("ac", "On AC", &status.ac_scenario),
                ("battery", "On battery", &status.battery_scenario),
            ];
            let picks: Vec<Element<'a, Message>> = sources
                .iter()
                .filter_map(|(source, label, picked)| {
                    Some(source_pick(source, label, picked.as_deref()?, scenarios))
                })
                .collect();
            if !picks.is_empty() {
                automation.push(cells(picks, 2));
            }
            let from_config: Vec<&str> = sources
                .iter()
                .filter(|(source, ..)| config_sets(status, &format!("profiles.{source}")))
                .map(|(_, label, _)| *label)
                .collect();
            if !from_config.is_empty() {
                automation.push(
                    hint(format!(
                        "{}: set in the config, which sets it again at the next start",
                        from_config.join(" and ")
                    ))
                    .into(),
                );
            }
            automation.push(
                row![
                    text("Scenarios").width(Length::Fill),
                    small_button("Edit the scenarios", Message::EditScenarios),
                ]
                .align_y(Alignment::Center)
                .into(),
            );
        }
        if let Some(on) = status.power_profiles_sync {
            automation.push(switch(
                "In step with power-profiles-daemon",
                on,
                Message::PowerProfilesSync,
            ));
        }
        groups.extend(group("Automation", automation));

        // In the sidebar, on NixOS, where there's one.
        let nixos = self.nixos();
        if !(wide && nixos) {
            let kept = if self.copied {
                "Copied, for services.ryukaze.settings".to_owned()
            } else {
                kept_hint(status)
            };
            let mut config = row![hint(kept).width(Length::Fill)]
                .spacing(12)
                .align_y(Alignment::Center);
            if nixos {
                config = config.push(small_button("Copy as Nix", Message::CopyAsNix));
            }
            groups.extend(group("Config", vec![config.into()]));
        }
        if !wide && let Some(laptop) = &self.laptop {
            groups.extend(group("This laptop", vec![about(status, &laptop.machine)]));
        }
        groups
    }
}

/// What the laptop is: the model, the board and its BIOS, the EC's
/// firmware, and the system.
fn about<'a>(status: &Status, machine: &Machine) -> Element<'a, Message> {
    // The EC's date as its firmware writes it: the day, without the time.
    let ec_date = status
        .firmware_date
        .as_ref()
        .map(|date| date.split('T').next().unwrap_or(date).to_owned());

    let mut lines = column![].spacing(6);
    if let Some(model) = &machine.model {
        lines = lines.push(text(model.clone()).size(14));
    }
    let mut facts = column![].spacing(3);
    for (label, value) in [
        ("Board".to_owned(), machine.board.clone()),
        ("BIOS".to_owned(), machine.bios.clone()),
        ("EC".to_owned(), status.firmware.clone()),
        ("OS".to_owned(), machine.os.clone()),
        ("Linux".to_owned(), machine.kernel.clone()),
    ] {
        if let Some(value) = value {
            facts = facts.push(
                row![
                    text(label).size(13).style(theme::muted).width(52),
                    text(value).size(13),
                ]
                .spacing(8),
            );
        }
    }
    lines = lines.push(facts);

    let dates: Vec<String> = [
        machine
            .bios_date
            .as_ref()
            .map(|date| format!("BIOS of {date}")),
        ec_date.map(|date| format!("EC firmware of {date}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if dates.is_empty() {
        return lines.into();
    }
    tip::tooltip(lines, dates.join("\n"), tip::Side::Above)
}

/// A group: its title over a card of rows, a line between each. None
/// without rows.
fn group<'a>(title: &'a str, rows: Vec<Element<'a, Message>>) -> Option<Element<'a, Message>> {
    if rows.is_empty() {
        return None;
    }
    Some(
        column![
            container(text(title).size(14).font(theme::heading())).padding([0, 4]),
            card(rows),
        ]
        .spacing(8)
        .into(),
    )
}

/// A card of rows, a line between each.
fn card<'a>(rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut card = column![].spacing(12);
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            card = card.push(rule::horizontal(1).style(theme::divider));
        }
        card = card.push(row);
    }
    container(card)
        .padding(16)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// A control under its name.
fn labelled_control<'a>(
    label: &'a str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![text(label).size(14).style(theme::muted), control.into()]
        .spacing(8)
        .into()
}

/// A figure on the overview: its name, it large, and a line under it.
/// `full`, where the line had to be shortened to fit, shows on hover.
fn stat<'a>(
    name: &'a str,
    value: String,
    under: String,
    full: Option<String>,
    opens: Page,
) -> Element<'a, Message> {
    // Every line its own fixed height, unwrapped, so every tile is as tall.
    let line = |content: String, size: f32, height: f32| {
        text(content)
            .size(size)
            .line_height(text::LineHeight::Absolute(height.into()))
            .wrapping(text::Wrapping::None)
    };
    let stat = column![
        line(name.to_owned(), 14.0, 18.0).style(theme::muted),
        line(value, 28.0, 34.0),
        line(under.clone(), 14.0, 18.0).style(theme::muted),
    ]
    .spacing(2);
    let tile = button(container(stat).clip(true))
        .on_press(Message::Page(opens))
        .padding(16)
        .width(Length::Fill)
        .hover(theme::tile);
    match full {
        Some(full) if full != under => tip::tooltip(tile, full, tip::Side::Below),
        _ => tile,
    }
}

/// A page, and what it holds, opening it.
fn link_row<'a>(page: Page, summary: String) -> Element<'a, Message> {
    button(
        row![
            column![text(page.title()), hint(summary)].spacing(2),
            space::horizontal(),
            text("›").size(20).style(theme::muted),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(Message::Page(page))
    .padding([4, 4])
    .width(Length::Fill)
    .hover(theme::link)
}

fn hint<'a>(line: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(line).size(14).style(theme::muted)
}
