//! ryukaze: a window and a tray icon onto ryukazed.

mod app;
mod appearance;
mod client;
mod curve;
mod icon;
mod instance;
mod nix;
mod theme;
mod tray;

const USAGE: &str = "usage: ryukaze [--hidden] [--session]";

fn main() -> iced::Result {
    let (mut session, mut hidden) = (false, false);
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            // Start in the tray only, as at login.
            "--hidden" => hidden = true,
            // Talk to `ryukazed --session`.
            "--session" => session = true,
            _ => {
                eprintln!("unknown argument {arg:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    if instance::hand_over(hidden) {
        return Ok(());
    }
    theme::load();

    iced::daemon(
        move || app::App::new(session, hidden),
        app::App::update,
        app::App::view,
    )
    .title("ryukaze")
    .theme(app::App::theme)
    .default_font(theme::font())
    .style(|_, theme| theme::window(theme))
    .subscription(app::App::subscription)
    .run()
}
