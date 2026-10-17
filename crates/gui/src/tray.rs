//! The StatusNotifierItem: the modes at a right click, the window at a left
//! one. It runs as a subscription, so it lives exactly as long as the app.

use std::fmt;

use iced::{
    futures::{SinkExt, Stream},
    stream,
};
use ksni::{
    MenuItem, TrayMethods,
    menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu},
};
use ryukaze_core::Status;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::app::{FAN_ORDER, Message, SHIFT_ORDER, capitalize, ordered};

pub struct Tray {
    /// The last status the window's poll got, or why there was none.
    status: Result<Status, String>,
    messages: UnboundedSender<Message>,
}

/// Lets the app push status into the tray.
#[derive(Clone)]
pub struct Handle(ksni::Handle<Tray>);

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tray::Handle")
    }
}

impl Handle {
    pub async fn show(self, status: Result<Status, String>) {
        self.0.update(|tray| tray.status = status).await;
    }
}

/// Puts the item up, hands the app a `Handle`, then forwards what's clicked.
pub fn run() -> impl Stream<Item = Message> {
    stream::channel(16, async |mut output| {
        let (messages, mut clicked) = mpsc::unbounded_channel();
        let tray = Tray {
            status: Err("Connecting…".into()),
            messages,
        };
        // At login the panel may put up its watcher after this: then the
        // icon registers once it appears, instead of never.
        match tray.assume_sni_available(true).spawn().await {
            Ok(handle) => {
                let _ = output.send(Message::TrayReady(Handle(handle))).await;
            }
            // No session bus or the like: no tray, the window still works.
            Err(err) => {
                eprintln!("ryukaze: no tray icon: {err}");
                return;
            }
        }
        while let Some(message) = clicked.recv().await {
            if output.send(message).await.is_err() {
                return;
            }
        }
    })
}

impl Tray {
    fn send(&self, message: Message) {
        let _ = self.messages.send(message);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "ryukaze".into()
    }

    fn title(&self) -> String {
        "ryukaze".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        crate::icon::tray()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = match &self.status {
            Ok(status) => [
                // The scenario says the modes in a word.
                match &status.scenario {
                    Some(scenario) => Some(capitalize(scenario)),
                    None => status.shift_mode.as_deref().map(capitalize),
                },
                status
                    .fan_mode
                    .as_deref()
                    .filter(|_| status.scenario.is_none())
                    .map(|mode| format!("fans {mode}")),
                status.cpu_temp.map(|temp| format!("CPU {temp}°C")),
                status.gpu_temp.map(|temp| match temp {
                    0 => "GPU asleep".to_owned(),
                    temp => format!("GPU {temp}°C"),
                }),
                status
                    .battery_level
                    .map(|level| format!("battery {level}%")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
            Err(err) => err.clone(),
        };
        ksni::ToolTip {
            title: "ryukaze".into(),
            description,
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Message::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = Vec::new();
        match &self.status {
            Ok(status) => {
                if let Some(scenarios) = &status.scenarios {
                    let current = status.scenario.as_deref().unwrap_or_default();
                    items.push(modes_menu(
                        "Scenario",
                        current,
                        scenarios,
                        &[],
                        Message::Scenario,
                    ));
                }
                if let (Some(current), Some(modes)) = (&status.shift_mode, &status.shift_modes) {
                    items.push(modes_menu(
                        "Performance",
                        current,
                        modes,
                        SHIFT_ORDER,
                        Message::ShiftMode,
                    ));
                }
                if let (Some(current), Some(modes)) = (&status.fan_mode, &status.fan_modes) {
                    items.push(modes_menu(
                        "Fans",
                        current,
                        modes,
                        FAN_ORDER,
                        Message::FanMode,
                    ));
                }
                if let Some(on) = status.cooler_boost {
                    items.push(check("Cooler boost", on, Message::CoolerBoost));
                }
                if let Some(on) = status.auto_cooler_boost {
                    items.push(check("Cooler boost when hot", on, Message::AutoCoolerBoost));
                }
                if let Some(on) = status.super_battery {
                    items.push(check("Super battery", on, Message::SuperBattery));
                }
                if let Some(on) = status.webcam {
                    items.push(check("Camera", on, Message::Webcam));
                }
                if let Some(on) = status.usb_share {
                    items.push(check("Charge USB when off", on, Message::UsbShare));
                }
                if let (Some(now), Some(next)) = (&status.gpu_mode, &status.gpu_mode_next) {
                    items.push(graphics_menu(now, next));
                }
            }
            Err(err) => items.push(
                StandardItem {
                    label: err.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Open ryukaze".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Message::Show)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Message::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

fn modes_menu(
    label: &str,
    current: &str,
    modes: &[String],
    order: &[&str],
    pick: fn(String) -> Message,
) -> MenuItem<Tray> {
    let modes: Vec<String> = ordered(modes, order).into_iter().cloned().collect();
    let selected = modes
        .iter()
        .position(|mode| mode == current)
        .unwrap_or(usize::MAX);
    let options = modes
        .iter()
        .map(|mode| RadioItem {
            label: capitalize(mode),
            ..Default::default()
        })
        .collect();
    SubMenu {
        label: label.into(),
        submenu: vec![
            RadioGroup {
                selected,
                select: Box::new(move |tray: &mut Tray, index| {
                    if let Some(mode) = modes.get(index) {
                        tray.send(pick(mode.clone()));
                    }
                }),
                options,
            }
            .into(),
        ],
        ..Default::default()
    }
    .into()
}

fn check(label: &str, on: bool, toggle: fn(bool) -> Message) -> MenuItem<Tray> {
    CheckmarkItem {
        label: label.into(),
        checked: on,
        activate: Box::new(move |tray: &mut Tray| tray.send(toggle(!on))),
        ..Default::default()
    }
    .into()
}

/// The MUX: the pick is the mode after a restart, as in the window.
fn graphics_menu(now: &str, next: &str) -> MenuItem<Tray> {
    let modes = ["hybrid", "discrete"];
    let mut submenu = vec![
        RadioGroup {
            selected: modes
                .iter()
                .position(|mode| *mode == next)
                .unwrap_or(usize::MAX),
            select: Box::new(move |tray: &mut Tray, index| {
                if let Some(mode) = modes.get(index) {
                    tray.send(Message::GpuMode((*mode).to_owned()));
                }
            }),
            options: modes
                .iter()
                .map(|mode| RadioItem {
                    label: capitalize(mode),
                    ..Default::default()
                })
                .collect(),
        }
        .into(),
    ];
    if now != next {
        submenu.push(
            StandardItem {
                label: format!("Switches to {next} at the next restart"),
                enabled: false,
                ..Default::default()
            }
            .into(),
        );
    }
    SubMenu {
        label: "Graphics".into(),
        submenu,
        ..Default::default()
    }
    .into()
}
