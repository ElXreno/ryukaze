//! ryukaze: a window and a tray icon onto ryukazed.

mod anim;
mod app;
mod appearance;
mod bootlogo;
mod client;
mod curve;
mod demo;
mod history;
mod icon;
mod instance;
mod laptop;
mod layouts;
mod lights;
mod machine;
mod nix;
mod osd;
mod picker;
mod renderer;
mod report;
mod scenarios;
mod system;
mod theme;
mod tip;
mod tray;

const USAGE: &str = "usage: ryukaze [--hidden] [--session] [--demo] [--next-scenario | --next-keyboard-profile | --toggle-local-dimming | --osd | --status [--json]]";

fn main() -> iced::Result {
    for (var, value) in renderer::choose() {
        // SAFETY: nothing else runs yet that could read the environment.
        unsafe { std::env::set_var(var, value) };
    }
    let (mut session, mut hidden, mut next_scenario, mut osd) = (false, false, false, false);
    let (mut next_keyboard_profile, mut toggle_local_dimming) = (false, false);
    let (mut status, mut json, mut demo) = (false, false, false);
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            // Start in the tray only, as at login.
            "--hidden" => hidden = true,
            // Talk to `ryukazed --session`.
            "--session" => session = true,
            // Step to the next scenario and exit, for a key binding.
            "--next-scenario" => next_scenario = true,
            // Step to the next keyboard light profile and exit, for Fn+F9.
            "--next-keyboard-profile" => next_keyboard_profile = true,
            // Local dimming the other way round and exit, for a key binding.
            "--toggle-local-dimming" => toggle_local_dimming = true,
            // The on-screen card for mode changes, a process of its own.
            "--osd" => osd = true,
            // Print what the window shows and exit; as JSON for scripts.
            "--status" => status = true,
            "--json" => json = true,
            // The window over a made-up laptop, ryukazed left alone.
            "--demo" => demo = true,
            _ => usage(&format!("unknown argument {arg:?}")),
        }
    }
    if json && !status {
        usage("--json goes with --status");
    }

    if osd {
        if let Err(err) = osd::run(session) {
            eprintln!("ryukaze: {err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    if next_scenario {
        exit_with(client::once(session, |client| async move {
            client.next_scenario().await
        }));
    }

    if next_keyboard_profile {
        exit_with(client::once(session, |client| async move {
            client.next_keyboard_profile().await
        }));
    }

    if toggle_local_dimming {
        exit_with(client::once(session, |client| async move {
            client.toggle_local_dimming().await
        }));
    }

    if status {
        let status = client::once(session, |client| async move { client.status().await });
        exit_with(status.and_then(|status| {
            if json {
                report::json(&status)
            } else {
                Ok(report::text(&status))
            }
        }));
    }

    if !demo && instance::hand_over(hidden) {
        return Ok(());
    }
    theme::load();

    iced::daemon(
        move || app::App::new(session, hidden, demo),
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

fn usage(problem: &str) -> ! {
    eprintln!("{problem}\n{USAGE}");
    std::process::exit(2);
}

/// Print what a command-line call gave and exit, 1 if it failed.
fn exit_with(result: Result<String, String>) -> ! {
    match result {
        Ok(output) => {
            println!("{}", output.trim_end());
            std::process::exit(0);
        }
        Err(err) => {
            eprintln!("ryukaze: {err}");
            std::process::exit(1);
        }
    }
}
