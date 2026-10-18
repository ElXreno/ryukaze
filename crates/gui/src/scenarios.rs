//! The scenarios' editor: each one's name and the modes it puts the EC in,
//! in the order Fn+F7 steps through them, sent to the daemon whole.

use iced::{
    Alignment, Element, Length,
    widget::{button, column, container, row, space, text, text_input},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::{FAN_ORDER, SHIFT_ORDER, capitalize},
    theme,
};

/// One as the config writes it; a mode unset is left as it is.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Scenario {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub super_battery: Option<bool>,
}

#[derive(Default, Deserialize, Serialize)]
struct File {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

pub struct Editor {
    scenarios: Vec<Scenario>,
    /// Changed since it was opened or saved.
    unsaved: bool,
}

/// The modes this EC has, to put scenarios in.
pub struct Modes<'a> {
    pub shift: &'a [String],
    pub fan: &'a [String],
    pub super_battery: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Name(usize, String),
    ShiftMode(usize, Option<String>),
    FanMode(usize, Option<String>),
    SuperBattery(usize, Option<bool>),
    Up(usize),
    Down(usize),
    Remove(usize),
    Add,
    Save,
    Done,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// Send these, TOML, to the daemon.
    Save(String),
    Done,
}

impl Editor {
    /// The daemon's, as ReadScenarios gives them.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let File { scenarios } = toml::from_str(text).map_err(|err| err.to_string())?;
        Ok(Self {
            scenarios,
            unsaved: false,
        })
    }

    fn toml(&self) -> String {
        toml::to_string(&File {
            scenarios: self.scenarios.clone(),
        })
        .unwrap_or_default()
    }

    pub fn saved(&mut self) {
        self.unsaved = false;
    }

    /// Changed since it was opened or saved.
    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    pub fn update(&mut self, msg: Msg) -> Outcome {
        let count = self.scenarios.len();
        match msg {
            Msg::Name(i, name) => self.scenarios[i].name = name,
            Msg::ShiftMode(i, mode) => self.scenarios[i].shift_mode = mode,
            Msg::FanMode(i, mode) => self.scenarios[i].fan_mode = mode,
            Msg::SuperBattery(i, on) => self.scenarios[i].super_battery = on,
            Msg::Up(i) if i > 0 => self.scenarios.swap(i - 1, i),
            Msg::Down(i) if i + 1 < count => self.scenarios.swap(i, i + 1),
            Msg::Up(_) | Msg::Down(_) => return Outcome::Nothing,
            Msg::Remove(i) => {
                self.scenarios.remove(i);
            }
            Msg::Add => {
                let taken = |name: &String| self.scenarios.iter().any(|known| known.name == *name);
                let name = (1..)
                    .map(|n| format!("scenario {n}"))
                    .find(|name| !taken(name))
                    .unwrap_or_default();
                self.scenarios.push(Scenario {
                    name,
                    ..Scenario::default()
                });
            }
            Msg::Save => return Outcome::Save(self.toml()),
            Msg::Done => return Outcome::Done,
        }
        self.unsaved = true;
        Outcome::Nothing
    }

    /// In `width`.
    pub fn view<'a>(&'a self, width: f32, modes: &Modes) -> Element<'a, Msg> {
        let wide = width >= 1300.0;
        let header = row![
            text("Scenarios").size(20),
            text(if self.unsaved {
                "changed, not saved"
            } else {
                ""
            })
            .size(12)
            .style(theme::muted),
            space::horizontal(),
            button(text("Save").size(13))
                .on_press(Msg::Save)
                .padding([4, 10])
                .style(theme::segment(self.unsaved)),
            small("Done", Msg::Done),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let mut list = column![].spacing(12);
        for (i, scenario) in self.scenarios.iter().enumerate() {
            list = list.push(self.row(i, scenario, modes, wide));
        }
        column![
            header,
            list,
            row![small("New scenario", Msg::Add)],
            text(
                "Fn+F7 steps through them in this order. A scenario puts the EC in its modes; \
                 \u{2014} leaves one as it is. The window shows the one whose modes the EC is in.",
            )
            .size(12)
            .style(theme::muted),
        ]
        .spacing(12)
        .into()
    }

    /// A scenario's card: in a row if `wide`, the modes under the name if
    /// not.
    fn row<'a>(
        &self,
        i: usize,
        scenario: &'a Scenario,
        modes: &Modes,
        wide: bool,
    ) -> Element<'a, Msg> {
        let pick = |label: &'static str, choices: Element<'a, Msg>| -> Element<'a, Msg> {
            column![text(label).size(12).style(theme::muted), choices]
                .spacing(4)
                .width(Length::Fill)
                .into()
        };
        let shift = ordered(modes.shift, SHIFT_ORDER);
        let fan = ordered(modes.fan, FAN_ORDER);
        let name = column![
            text("Name").size(12).style(theme::muted),
            text_input("a name", &scenario.name)
                .on_input(move |name| Msg::Name(i, name))
                .size(13)
                .width(160),
        ]
        .spacing(4);
        let mut picks = vec![
            pick(
                "Performance",
                choices(&scenario.shift_mode, &shift, move |mode| {
                    Msg::ShiftMode(i, mode)
                }),
            ),
            pick(
                "Fans",
                choices(&scenario.fan_mode, &fan, move |mode| Msg::FanMode(i, mode)),
            ),
        ];
        if modes.super_battery {
            let on = |value: Option<bool>| scenario.super_battery == value;
            let battery = row![
                choice("\u{2014}".into(), on(None), Msg::SuperBattery(i, None)),
                choice(
                    "On".into(),
                    on(Some(true)),
                    Msg::SuperBattery(i, Some(true))
                ),
                choice(
                    "Off".into(),
                    on(Some(false)),
                    Msg::SuperBattery(i, Some(false))
                ),
            ]
            .spacing(4);
            picks.push(pick("Super battery", battery.into()));
        }
        let last = self.scenarios.len() - 1;
        let tools = row![
            button(text("\u{2191}").size(13))
                .on_press_maybe((i > 0).then_some(Msg::Up(i)))
                .padding([4, 10])
                .style(theme::segment(false)),
            button(text("\u{2193}").size(13))
                .on_press_maybe((i < last).then_some(Msg::Down(i)))
                .padding([4, 10])
                .style(theme::segment(false)),
            small("Delete", Msg::Remove(i)),
        ]
        .spacing(4);
        let card: Element<'a, Msg> = if wide {
            row![name]
                .extend(picks)
                .push(tools)
                .spacing(16)
                .align_y(Alignment::End)
                .into()
        } else {
            column![row![name, space::horizontal(), tools].align_y(Alignment::End)]
                .extend(picks)
                .spacing(10)
                .into()
        };
        container(card)
            .padding(16)
            .width(Length::Fill)
            .style(theme::card)
            .into()
    }
}

/// Modes in their usual order.
fn ordered(modes: &[String], order: &[&str]) -> Vec<String> {
    crate::app::ordered(modes, order)
        .into_iter()
        .cloned()
        .collect()
}

/// A mode or none, one of a row.
fn choices<'a>(
    current: &Option<String>,
    modes: &[String],
    pick: impl Fn(Option<String>) -> Msg,
) -> Element<'a, Msg> {
    let mut choices = row![choice("\u{2014}".into(), current.is_none(), pick(None))].spacing(4);
    for mode in modes {
        choices = choices.push(choice(
            capitalize(mode),
            current.as_ref() == Some(mode),
            pick(Some(mode.clone())),
        ));
    }
    choices.into()
}

fn choice<'a>(label: String, chosen: bool, pick: Msg) -> Element<'a, Msg> {
    button(text(label).size(13).width(Length::Fill).center())
        .on_press_maybe((!chosen).then_some(pick))
        .padding([4, 4])
        .width(Length::Fill)
        .style(theme::segment(chosen))
        .into()
}

fn small<'a>(label: &'a str, press: Msg) -> Element<'a, Msg> {
    button(text(label).size(13))
        .on_press(press)
        .padding([4, 10])
        .style(theme::segment(false))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_and_writes_them_back() {
        let read = "[[scenarios]]\nname = \"silent\"\nshift_mode = \"eco\"\nfan_mode = \"silent\"\nsuper_battery = false\n\n[[scenarios]]\nname = \"performance\"\nshift_mode = \"turbo\"\n";
        let mut editor = Editor::from_toml(read).unwrap();
        assert_eq!(editor.toml(), read);
        let _ = editor.update(Msg::Down(0));
        let _ = editor.update(Msg::Add);
        let _ = editor.update(Msg::FanMode(2, Some("advanced".into())));
        let _ = editor.update(Msg::SuperBattery(1, None));
        let _ = editor.update(Msg::Name(1, "quiet".into()));
        assert_eq!(editor.update(Msg::Up(0)), Outcome::Nothing);
        let Outcome::Save(sent) = editor.update(Msg::Save) else {
            panic!("no save");
        };
        assert_eq!(
            sent,
            "[[scenarios]]\nname = \"performance\"\nshift_mode = \"turbo\"\n\n[[scenarios]]\nname = \"quiet\"\nshift_mode = \"eco\"\nfan_mode = \"silent\"\n\n[[scenarios]]\nname = \"scenario 1\"\nfan_mode = \"advanced\"\n"
        );
        let _ = editor.update(Msg::Remove(2));
        assert_eq!(editor.scenarios.len(), 2);
    }
}
