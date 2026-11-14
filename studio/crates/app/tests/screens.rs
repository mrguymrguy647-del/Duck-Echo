//! Every screen of the Studio, drawn with no window.
//!
//! `egui_kittest` runs the app's own drawing code against a pretend screen
//! and finds things on it by their words, as a person would. Nothing here
//! plays a sound or uses the internet.
//!
//! With `STUDIO_SCREENSHOTS=<folder>` it also saves a picture of each
//! screen (this needs a graphics adapter; CI uses a software one).

use std::path::{Path, PathBuf};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use studio_app::settings::Settings;
use studio_app::update::{Asset, NewVersion};
use studio_app::update_banner::UpdateState;
use studio_app::{AppMenu, StudioApp, app_menu, theme};

/// A folder of the test's own.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-screens-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app with fresh settings (never saved) and its project folder in
/// `dir`.
fn app(dir: &Path, app_menu: AppMenu) -> StudioApp {
    let settings = Settings {
        project_folder: Some(dir.join("Duck Echo")),
        check_for_updates: false,
        ..Settings::default()
    };
    StudioApp::new(settings, app_menu)
}

/// The app on a pretend 1100 x 760 screen, run the way eframe runs it.
fn harness(app: StudioApp, dark: bool) -> Harness<'static, StudioApp> {
    let render = std::env::var_os("STUDIO_SCREENSHOTS").is_some();
    let builder = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .with_pixels_per_point(1.0)
        .with_context_setup(theme::apply_brand);
    let builder = if render { builder.wgpu() } else { builder };
    let harness = builder.build_eframe(|_| app);
    // Building the eframe app sets eframe's own default theme, so the
    // theme is set again here.
    harness.ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    harness
}

/// Save a picture of the screen as it is now, when asked to.
fn shoot(harness: &mut Harness<'static, StudioApp>, name: &str) {
    let Some(folder) = std::env::var_os("STUDIO_SCREENSHOTS") else {
        return;
    };
    harness.run_steps(3);
    let picture = harness.render().expect("the screen can be drawn");
    std::fs::create_dir_all(&folder).unwrap();
    let path = Path::new(&folder).join(format!("{name}.png"));
    picture.save(&path).unwrap();
    println!("saved {}", path.display());
}

#[test]
fn the_first_screen_lists_the_built_in_sounds_with_play() {
    let dir = folder("first");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(3);
    harness.get_by_label("Duck Echo Studio");
    harness.get_by_label("Built-in sounds");
    harness.get_by_label("Settings");
    harness.get_by_label("Theme");
    harness.get_by_label("Help");
    // Every preset under its category, each with its own Play button.
    for name in ["coin", "explosion", "laser", "wind", "ui_click"] {
        harness.get_by_label(name);
    }
    harness.get_by_label("Pickups");
    harness.get_by_label("Impacts");
    assert_eq!(harness.query_all_by_label("Play").count(), 30);
    // The project folder was made, and the status line says where it is.
    assert!(dir.join("Duck Echo").is_dir());
    harness.get_by_label(&format!(
        "Your sounds and songs: {}",
        dir.join("Duck Echo").display()
    ));
    shoot(&mut harness, "01-first-screen-dark");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_light_theme_and_the_theme_menu() {
    let dir = folder("theme");
    let mut harness = harness(app(&dir, AppMenu::none()), false);
    harness.run_steps(3);
    shoot(&mut harness, "02-first-screen-light");
    harness.get_by_label("Theme").click();
    harness.run_steps(2);
    harness.get_by_label("Dark");
    harness.get_by_label(theme::system_label());
    shoot(&mut harness, "03-theme-menu");
    harness.get_by_label("Dark").click();
    harness.run_steps(2);
    assert_eq!(harness.state().settings.theme.as_deref(), Some("dark"));
    assert_eq!(
        harness.ctx.options(|o| o.theme_preference),
        egui::ThemePreference::Dark
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn help_about_shows_the_version() {
    let dir = folder("about");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    harness.get_by_label("Help").click();
    harness.run_steps(2);
    harness.get_by_label("Check for new versions");
    harness.get_by_label("About Duck Echo Studio").click();
    harness.run_steps(3);
    harness.get_by_label(&format!("Version {}", env!("CARGO_PKG_VERSION")));
    shoot(&mut harness, "04-about");
    harness.get_by_label("Close").click();
    harness.run_steps(2);
    assert!(!harness.state().about_open);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn settings_show_the_folder_and_the_update_tick_box() {
    let dir = folder("settings");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    harness.get_by_label("Settings").click();
    harness.run_steps(3);
    harness.get_by_label("Your sounds and songs");
    harness.get_by_label("Open the folder");
    harness.get_by_label("Tell me when a new version is out");
    if cfg!(target_os = "linux") {
        harness
            .get_by_label("Only the AppImage of Duck Echo Studio can add itself to the app menu.");
    }
    shoot(&mut harness, "05-settings");
    std::fs::remove_dir_all(&dir).ok();
}

/// The Linux app menu, with a pretend AppImage and a data folder of the
/// test's own (the person's real menu is never touched).
fn linux_app_menu(dir: &Path, ask: bool) -> AppMenu {
    let appimage = dir.join("Apps").join("DuckEchoStudio.AppImage");
    std::fs::create_dir_all(appimage.parent().unwrap()).unwrap();
    std::fs::write(&appimage, "pretend").unwrap();
    AppMenu {
        appimage: Some(appimage),
        data: Some(dir.join("share")),
        ask,
    }
}

#[test]
fn the_first_start_of_the_appimage_asks_to_join_the_app_menu() {
    let dir = folder("ask");
    let mut harness = harness(app(&dir, linux_app_menu(&dir, true)), true);
    harness.run_steps(3);
    harness.get_by_label("Add Duck Echo Studio to your app menu?");
    shoot(&mut harness, "06-add-to-app-menu");
    harness.get_by_label("Add").click();
    harness.run_steps(2);
    let data = dir.join("share");
    assert!(app_menu::is_added(&data));
    assert_eq!(
        app_menu::entry_points_to(&data),
        Some(dir.join("Apps").join("DuckEchoStudio.AppImage"))
    );
    assert!(app_menu::icon_file(&data).is_file());
    let state = harness.state();
    assert!(!state.app_menu.ask);
    assert!(state.settings.asked_about_app_menu, "it does not ask again");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn not_now_adds_nothing_and_does_not_ask_again() {
    let dir = folder("not-now");
    let mut harness = harness(app(&dir, linux_app_menu(&dir, true)), true);
    harness.run_steps(3);
    harness.get_by_label("Not now").click();
    harness.run_steps(2);
    assert!(!app_menu::is_added(&dir.join("share")));
    assert!(harness.state().settings.asked_about_app_menu);
    assert!(
        harness
            .query_by_label("Add Duck Echo Studio to your app menu?")
            .is_none()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(target_os = "linux")]
#[test]
fn settings_add_and_remove_the_app_menu_entry() {
    let dir = folder("menu");
    let data = dir.join("share");
    let mut harness = harness(app(&dir, linux_app_menu(&dir, false)), true);
    harness.run_steps(2);
    harness.get_by_label("Settings").click();
    harness.run_steps(3);
    harness.get_by_label("Duck Echo Studio is not in your app menu.");
    harness.get_by_label("Add to app menu").click();
    harness.run_steps(3);
    assert!(app_menu::is_added(&data));
    harness.get_by_label("Duck Echo Studio is in your app menu.");
    shoot(&mut harness, "07-settings-linux-app-menu");
    harness.get_by_label("Remove from app menu").click();
    harness.run_steps(3);
    assert!(!app_menu::is_added(&data));
    assert!(!app_menu::icon_file(&data).exists());
    harness.get_by_label("Add to app menu");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_new_version_shows_a_banner_and_not_now_hides_it() {
    let dir = folder("banner");
    let mut app = app(&dir, AppMenu::none());
    // A pretend answer from the check: no internet is used.
    app.update = UpdateState::Available(NewVersion {
        version: "9.9.9".into(),
        whats_new: "https://example.test/releases/tag/studio-v9.9.9".into(),
        file: Some(Asset {
            address: "https://example.test/file".into(),
            size: 5,
        }),
        sums: None,
    });
    let mut harness = harness(app, true);
    harness.run_steps(3);
    harness.get_by_label("Duck Echo Studio 9.9.9 is out.");
    harness.get_by_label(&format!("You have {}.", env!("CARGO_PKG_VERSION")));
    harness.get_by_label("What's new");
    harness.get_by_label("Update");
    shoot(&mut harness, "08-new-version-banner");
    harness.get_by_label("Not now").click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label("Duck Echo Studio 9.9.9 is out.")
            .is_none()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failed_update_says_why_and_offers_to_try_again() {
    let dir = folder("failed");
    let mut app = app(&dir, AppMenu::none());
    app.update = UpdateState::Failed(
        NewVersion {
            version: "9.9.9".into(),
            whats_new: "https://example.test/whats-new".into(),
            file: None,
            sums: None,
        },
        "the download's fingerprint does not match the one listed with the release".into(),
    );
    let mut harness = harness(app, true);
    harness.run_steps(3);
    harness.get_by_label("Try again");
    harness.get_by_label(
        "The update did not work: the download's fingerprint does not match the one listed \
         with the release. Try again, or download it from the What's new page.",
    );
    shoot(&mut harness, "09-update-refused");
    std::fs::remove_dir_all(&dir).ok();
}
