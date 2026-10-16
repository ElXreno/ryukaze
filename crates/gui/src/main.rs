//! ryukaze: a window onto ryukazed.

mod app;
mod appearance;
mod client;
mod theme;

use iced::window;

const USAGE: &str = "usage: ryukaze [--session]";

fn main() -> iced::Result {
    let mut session = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            // Talk to `ryukazed --session`.
            "--session" => session = true,
            _ => {
                eprintln!("unknown argument {arg:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    iced::application(
        move || app::App::new(session),
        app::App::update,
        app::App::view,
    )
    .title("ryukaze")
    .theme(app::App::theme)
    .style(|_, theme| theme::window(theme))
    .subscription(app::App::subscription)
    .window(window::Settings {
        size: (420.0, 600.0).into(),
        min_size: Some((360.0, 420.0).into()),
        platform_specific: window::settings::PlatformSpecific {
            application_id: "ryukaze".into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
}
