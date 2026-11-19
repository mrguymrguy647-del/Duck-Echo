//! Phase 3, step 4: Send to game in the Studio, the Duck Forge folder in
//! Settings, and `DuckEchoStudio --open FILE` (docs/phase3-design.md).

use std::path::{Path, PathBuf};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use studio_app::settings::Settings;
use studio_app::{AppMenu, StudioApp, theme};

/// A folder of the test's own.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-send-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A Duck Forge folder with one game, duck_craft.
fn duck_forge(dir: &Path) -> PathBuf {
    let forge = dir.join("Duck Forge");
    std::fs::create_dir_all(forge.join("examples/duck_craft")).unwrap();
    std::fs::write(
        forge.join("Cargo.toml"),
        "[package]\nname = \"duckforge\"\n",
    )
    .unwrap();
    std::fs::write(forge.join("examples/duck_craft/main.rs"), "fn main() {}\n").unwrap();
    forge
}

fn settings(dir: &Path) -> Settings {
    Settings {
        project_folder: Some(dir.join("Duck Echo")),
        check_for_updates: false,
        ..Settings::default()
    }
}

fn harness(app: StudioApp) -> Harness<'static, StudioApp> {
    let harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(theme::apply_brand)
        .build_eframe(|_| app);
    harness.ctx.set_theme(egui::ThemePreference::Dark);
    harness
}

/// Wait (in real time, up to 10 s) until the open sound has finished
/// rendering and saving: until then a spinner moves the bar, and a click
/// can miss, as a person's would.
fn settle(harness: &mut Harness<'static, StudioApp>) {
    for _ in 0..1000 {
        harness.run_steps(1);
        let busy = harness
            .state()
            .editor
            .as_ref()
            .is_some_and(|e| e.rendering() || e.unsaved());
        if !busy {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    harness.run_steps(3);
}

/// Open the `...` menu, then Send to game, then click `item`.
fn send_with_menu(harness: &mut Harness<'static, StudioApp>, item: &str) {
    settle(harness);
    harness.get_by_label("...").click();
    harness.run_steps(2);

    harness.get_by_label("Send to game").hover();
    harness.run_steps(3);
    harness.get_by_label("Send to game").click();
    harness.run_steps(3);
    harness.get_by_label(item).click();
    harness.run_steps(3);
}

#[test]
fn a_sound_and_a_song_are_sent_from_their_menus_and_the_game_is_remembered() {
    let dir = folder("menus");
    let forge = duck_forge(&dir);
    let game = forge.join("examples/duck_craft");
    let mut harness = harness(StudioApp::new(settings(&dir), AppMenu::none()));
    harness.run_steps(1);
    assert!(harness.state_mut().use_duck_forge_folder(&forge));
    harness.state_mut().make_copy("preset:coin", "coin");
    harness.state_mut().open_sound("coin.toml");
    harness.run_steps(4);

    // The sound screen's ... menu: Send to game > duck_craft.
    send_with_menu(&mut harness, "duck_craft");
    assert!(game.join("assets/coin.wav").is_file());
    assert!(game.join("assets/coin.wav.json").is_file());
    assert!(game.join("art/coin.toml").is_file());
    harness.get_by_label_contains("Sent coin to duck_craft");
    assert_eq!(harness.state().last_game("coin.toml"), Some("duck_craft"));

    // The game is offered first next time, and a second send writes
    // nothing.
    send_with_menu(&mut harness, "duck_craft (last time)");
    harness.get_by_label_contains("nothing written");

    // The song screen's ... menu sends a song, as OGG.
    harness.state_mut().close_editor();
    harness.state_mut().make_copy("example:four_bars", "tune");
    harness.state_mut().open_song("tune.toml");
    harness.run_steps(4);
    assert!(harness.state().song_view.is_some());
    send_with_menu(&mut harness, "duck_craft");
    assert!(
        game.join("assets/tune.ogg").is_file(),
        "the song went as OGG"
    );
    harness.get_by_label_contains("Sent tune to duck_craft");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_no_duck_forge_folder_the_menu_and_settings_say_how_to_choose_one() {
    let dir = folder("none");
    let mut harness = harness(StudioApp::new(settings(&dir), AppMenu::none()));
    harness.run_steps(1);
    // Whatever this computer has in Documents, this test has none.
    if let Some(root) = &mut harness.state_mut().root {
        root.set_duck_forge(None);
    }
    harness.state_mut().make_copy("preset:coin", "coin");
    harness.state_mut().open_sound("coin.toml");
    harness.run_steps(4);
    harness.get_by_label("...").click();
    harness.run_steps(2);
    harness.get_by_label("Send to game").click();
    harness.run_steps(3);
    harness.get_by_label("No Duck Forge folder yet.");
    harness.get_by_label("Choose the Duck Forge folder...");

    // Settings shows the box, and a folder that is not Duck Forge's is
    // refused with a reason.
    harness.state_mut().settings_open = true;
    harness.run_steps(3);
    harness.get_by_label("Duck Forge games");
    harness.get_by_label_contains("Not found.");
    assert!(!harness.state_mut().use_duck_forge_folder(&dir));
    harness.run_steps(1);
    harness.get_by_label_contains("is not a Duck Forge folder");
    // A real one is taken, and Settings shows it.
    let forge = duck_forge(&dir);
    assert!(harness.state_mut().use_duck_forge_folder(&forge));
    assert_eq!(
        harness.state().settings.duck_forge_folder.as_deref(),
        Some(forge.as_path())
    );
    harness.run_steps(2);
    harness.get_by_label_contains("1 game.");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn open_a_recipe_of_a_game_works_on_the_game_with_its_history_kept_outside() {
    let dir = folder("open");
    let forge = duck_forge(&dir);
    let game = forge.join("examples/duck_craft");
    std::fs::create_dir_all(game.join("art")).unwrap();
    std::fs::create_dir_all(game.join("assets")).unwrap();
    let coin = duck_echo_tools::presets::get("coin").unwrap().source;
    std::fs::write(game.join("art/step.toml"), coin).unwrap();
    // A game's own JSON (a scene) is not a sound, and is not shown.
    std::fs::write(game.join("assets/blocks.json"), "{\"tiles\": []}").unwrap();
    let settings_folder = dir.join("settings");

    let opening =
        studio_app::opening::opening(&game.join("art/step.toml"), Some(&settings_folder)).unwrap();
    let own = opening.own_folder.clone().unwrap();
    let app = StudioApp::opening(settings(&dir), AppMenu::none(), opening);
    let mut harness = harness(app);
    harness.run_steps(4);
    // The recipe is open, and the game is the project.
    assert_eq!(
        harness.state().editor.as_ref().unwrap().rel,
        "art/step.toml"
    );
    assert_eq!(harness.state().game.as_deref(), Some("duck_craft"));
    assert_eq!(harness.state().project, game.canonicalize().unwrap());

    // Send to game offers this game first.
    send_with_menu(&mut harness, "duck_craft (this game)");
    assert!(game.join("assets/step.wav").is_file());

    // A change is one Undo step, kept in Duck Echo's settings folder.
    harness
        .state_mut()
        .run_command(
            "sound_set",
            serde_json::json!({"file": "art/step.toml", "path": "layers[0].volume", "value": 0.5}),
        )
        .unwrap();
    assert!(own.join("history").is_dir(), "{}", own.display());
    assert!(
        !game.join(".duck-echo").exists(),
        "nothing of Duck Echo's in the game"
    );

    // Home shows the game's recipes (art/), not its other files.
    harness.state_mut().close_editor();
    harness.state_mut().rescan();
    harness.run_steps(2);
    let shown: Vec<&str> = harness
        .state()
        .files
        .iter()
        .map(|f| f.rel.as_str())
        .collect();
    assert_eq!(shown, ["art/step.toml"]);
    // A new sound goes into art/.
    harness
        .state_mut()
        .new_file(studio_app::library::Kind::Sound);
    assert!(game.join("art/new sound.toml").is_file());
    // The person's own project folder setting is untouched.
    assert_eq!(
        harness.state().settings.project_folder.as_deref(),
        Some(dir.join("Duck Echo").as_path())
    );
    std::fs::remove_dir_all(&dir).ok();
}
