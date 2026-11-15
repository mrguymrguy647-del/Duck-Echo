//! Duck Echo Studio: make sound effects and music without a terminal.
//!
//!   DuckEchoStudio                       opens the Studio
//!   DuckEchoStudio --version             says which version this is
//!   DuckEchoStudio --play coin           plays a built-in sound, a recipe
//!                                        or song file, or an example song
//!                                        (example:four_bars) through the
//!                                        speakers and waits for it to end,
//!                                        with no window
//!   DuckEchoStudio --check-for-updates   says whether a newer Studio is out
//!                                        (as the window does when it starts)

// A window app: no black console window behind it on Windows. (Debug
// builds keep it, so what the Studio prints can be read.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::Path;
use std::time::Duration;

use duck_echo_live::Sound;
use eframe::egui;
use studio_app::settings::Settings;
use studio_app::{AppMenu, StudioApp, app_menu, speaker, theme, update};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("Duck Echo Studio {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--play") => std::process::exit(play(args.get(1).map(String::as_str))),
        Some("--check-for-updates") => {
            check_for_updates();
            return Ok(());
        }
        Some("--help") => {
            println!(
                "Duck Echo Studio {}\n\n  DuckEchoStudio                      open the Studio\n  DuckEchoStudio --version            say which version this is\n  DuckEchoStudio --play NAME|FILE     play a built-in sound or a recipe through the speakers\n  DuckEchoStudio --check-for-updates  say whether a newer Studio is out",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        _ => {}
    }

    // The window and taskbar icon. (DuckEchoStudio.exe's own icon, the one
    // File Explorer shows, is put in by build.rs.)
    let icon = eframe::icon_data::from_png_bytes(app_menu::ICON)
        .expect("brand/duck-echo-icon-256.png should be a valid PNG file");
    let settings = Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Duck Echo Studio")
            // On Linux this names the window for the desktop, so it belongs
            // to the app-menu entry (StartupWMClass).
            .with_app_id(app_menu::APP_ID)
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "Duck Echo Studio",
        options,
        Box::new(move |creation| {
            let ctx = &creation.egui_ctx;
            ctx.set_theme(theme::from_name(settings.theme.as_deref()));
            theme::apply_brand(ctx);
            let app_menu = AppMenu::detect(&settings);
            Ok(Box::new(
                StudioApp::new(settings, app_menu).checking_for_updates(),
            ))
        }),
    )
}

/// Play a built-in sound or a recipe file through the speakers, with no
/// window. Exit code 0 when it played to its end, 2 when the sound could
/// not be read, 3 when there are no speakers.
fn play(what: Option<&str>) -> i32 {
    let Some(what) = what else {
        eprintln!("usage: DuckEchoStudio --play coin (or a recipe file)");
        return 2;
    };
    // A song (an example song, or a song file) plays from its background
    // render, as Home plays it.
    let song_text = match what.strip_prefix("example:") {
        Some(name) => duck_echo_tools::examples::get(name)
            .ok()
            .map(|e| e.source.to_string()),
        None => std::fs::read_to_string(what).ok().filter(|text| {
            duck_echo_core::document_kind(text, duck_echo_core::Format::from_path(Path::new(what)))
                == duck_echo_core::DocumentKind::Song
        }),
    };
    if let Some(text) = song_text {
        let folder = Path::new(what)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let files = duck_echo_tools::song::files_next_to(&folder.join("song.toml"));
        let loaded = duck_echo_core::load_song_str(
            &text,
            duck_echo_core::Format::Toml,
            &duck_echo_tools::song::Library::new(&files),
        );
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(report) => {
                eprintln!("Could not read {what}:\n{report}");
                return 2;
            }
        };
        return match speaker::play_song_to_the_end(
            loaded.song,
            loaded.arrangement,
            Duration::from_secs(120),
        ) {
            Ok(played) => {
                println!(
                    "Played the song {what} through the speakers; the sound card took {} frames.",
                    played.frames
                );
                0
            }
            Err(why) => {
                eprintln!("{why}");
                3
            }
        };
    }
    let recipe = if Path::new(what).is_file() {
        duck_echo_core::load_path(Path::new(what))
            .map(|l| l.recipe)
            .map_err(|r| r.to_string())
    } else {
        duck_echo_tools::presets::get(what)
            .and_then(duck_echo_tools::presets::load)
            .map(|l| l.recipe)
            .map_err(|e| e.into_report().to_string())
    };
    let sound = recipe.and_then(|r| Sound::from_recipe(&r).map_err(|report| report.to_string()));
    let sound = match sound {
        Ok(sound) => sound,
        Err(why) => {
            eprintln!("Could not read {what}:\n{why}");
            return 2;
        }
    };
    match speaker::play_to_the_end(sound, Duration::from_secs(60)) {
        Ok(played) => {
            println!(
                "Played {what} through the speakers: {:.2} s; the sound card took {} frames.",
                played.seconds, played.frames
            );
            0
        }
        Err(why) => {
            eprintln!("{why}");
            3
        }
    }
}

/// Look for a newer Studio, as the window does when it starts, and say
/// what was found. It never downloads anything.
fn check_for_updates() {
    let running = env!("CARGO_PKG_VERSION");
    match update::check_now(&update::Curl, running) {
        update::Found::Newer(new) => {
            println!(
                "Duck Echo Studio {} is out (this is {running}).",
                new.version
            );
            println!("What's new: {}", new.whats_new);
            match new.file {
                Some(file) => println!(
                    "{}: {} ({} bytes)",
                    update::this_systems_file(),
                    file.address,
                    file.size
                ),
                None => println!("{}: none in that release", update::this_systems_file()),
            }
            match new.sums {
                Some(sums) => println!("{}: {}", update::SUMS, sums.address),
                None => println!("{}: none in that release", update::SUMS),
            }
        }
        update::Found::UpToDate => println!("Up to date: this is the newest version ({running})."),
        update::Found::NoAnswer(why) => println!("No answer from GitHub ({why}); nothing to say."),
    }
}
