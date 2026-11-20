//! Settings > Duck Ear, drawn with no window: installing is offered with
//! its size, an installed Ear shows, a big graphics card offers the Large
//! model, and its results come with Save and (only after a pass) Use.
//!
//! Nothing here downloads anything or starts a model: the Ear's folder is
//! a pretend one of the test's own, and so are the graphics cards.

use std::path::{Path, PathBuf};

use duck_echo_tools::ear_cards::Card;
use duck_echo_tools::ear_install::{self, Places};
use duck_echo_tools::ear_net::megabytes;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use studio_app::ear_panel::EarPanel;
use studio_app::settings::Settings;
use studio_app::{AppMenu, StudioApp, theme};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-ear-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app with the Ear's folder in `dir/ear` and these graphics cards.
fn app(dir: &Path, cards: Vec<Card>) -> StudioApp {
    let settings = Settings {
        project_folder: Some(dir.join("Duck Echo")),
        check_for_updates: false,
        ..Settings::default()
    };
    let mut app = StudioApp::new(settings, AppMenu::none());
    app.ear = EarPanel::at(dir.join("ear"), cards);
    app
}

fn harness(app: StudioApp) -> Harness<'static, StudioApp> {
    let render = std::env::var_os("STUDIO_SCREENSHOTS").is_some();
    let builder = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(theme::apply_brand);
    let builder = if render { builder.wgpu() } else { builder };
    let harness = builder.build_eframe(|_| app);
    harness.ctx.set_theme(egui::ThemePreference::Dark);
    harness
}

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

/// Open Settings and bring the Duck Ear section into view.
fn open_settings(harness: &mut Harness<'static, StudioApp>) {
    harness.run_steps(2);
    harness.get_by_label("Settings").click();
    harness.run_steps(30);
    harness.get_by_label("Duck Ear").scroll_to_me();
    harness.run_steps(3);
}

/// Click the button labelled `label`, scrolled into view first.
fn press(harness: &mut Harness<'static, StudioApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    harness.run_steps(3);
}

fn big_card() -> Card {
    Card {
        name: "NVIDIA GeForce RTX 4090".into(),
        memory_mb: 24564,
    }
}

/// A pretend installed Ear in `home`: what `ear install` leaves behind,
/// with empty programs.
fn pretend_installed(home: &Path) {
    let places = Places::new(home);
    for file in [places.launcher(), places.python()] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"").unwrap();
    }
    std::fs::create_dir_all(places.score_model()).unwrap();
    std::fs::write(
        places.score_model().join("audio.onnx"),
        vec![0u8; 5_000_000],
    )
    .unwrap();
    let record = json!({
        "version": 1, "duck_echo": "0.1.0", "ear_version": "0.4.0", "uv": "0.13.9",
        "python": "3.12", "score_model": "laion-clap-general@ada0c23a36c4", "capabilities": ["score"],
    });
    std::fs::write(places.record(), record.to_string()).unwrap();
}

/// The results of a try of the Large model, as `ear try` leaves them
/// beside the model: it said yes to `yes` of 47 true texts.
fn pretend_tried(home: &Path, yes: u64, passed: bool) {
    let folder = home.join("models").join("qwen3-omni-30b-a3b");
    std::fs::create_dir_all(&folder).unwrap();
    let measure = |name: &str, right: u64, of: u64, bar: f64| {
        let share = right as f64 / of as f64;
        json!({"name": name, "right": right, "of": of, "share": share, "bar": bar, "passed": share >= bar})
    };
    let report = json!({"describe": {
        "model": "qwen3-omni-30b-a3b@6e35a28f4a19+q4_k_m+prompts1",
        "device": "vulkan",
        "device_name": "NVIDIA GeForce RTX 4090 (24564 MB) for the describe model; the score model on the CPU",
        "measures": [
            measure("Describe: yes on true texts", yes, 47, 0.8),
            measure("Describe: no on wrong texts", 125, 137, 0.8),
            measure("Describe: silence", 5, 5, 1.0),
            measure("Describe: noise", 4, 4, 1.0),
            measure("Same words twice", 8, 8, 1.0),
        ],
        "times": {"load": 21.0, "describe_mean": 1.4, "ask_mean": 0.6},
    }});
    std::fs::write(folder.join("ear-check.json"), report.to_string()).unwrap();
    std::fs::write(folder.join("ear-check.md"), "# Duck Ear check\n").unwrap();
    let missed: Vec<String> = if passed {
        vec![]
    } else {
        vec![format!(
            "Describe: yes on true texts: {yes} of 47 (64%, the bar is 80%)"
        )]
    };
    let last = json!({"model": "qwen3-omni-30b-a3b@6e35a28f4a19", "whole_set": true, "passed": passed, "missed": missed});
    std::fs::write(folder.join("last-check.json"), last.to_string()).unwrap();
}

#[test]
fn settings_offer_to_install_the_ear_with_its_size_and_ask_first() {
    let dir = folder("install");
    let mut harness = harness(app(&dir, vec![big_card()]));
    open_settings(&mut harness);
    harness.get_by_label("The Duck Ear is not installed.");
    shoot(&mut harness, "14-settings-duck-ear-not-installed");
    // Nothing is offered to try before the Ear is installed.
    assert!(
        harness
            .query_by_label_contains("Try a bigger model")
            .is_none()
    );
    // The size is this system's own: uv's download differs between
    // Windows and Linux.
    let plan = ear_install::plan(&harness.state().ear.home).expect("this system has a plan");
    let size = megabytes(plan.download_bytes);
    press(
        &mut harness,
        &format!("Install the Duck Ear (about {size})"),
    );
    // The question says what and how much, and nothing has started.
    harness.get_by_label_contains("scores only");
    harness.get_by_label_contains(&format!("about {size} to download"));
    harness.get_by_label("Install");
    assert!(harness.state().ear.job.is_none());
    shoot(&mut harness, "15-settings-duck-ear-install-question");
    press(&mut harness, "Not now");
    harness.get_by_label("The Duck Ear is not installed.");
    assert!(!dir.join("ear").exists(), "saying no writes nothing");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_installed_ear_shows_and_only_a_big_card_offers_the_large_model() {
    let dir = folder("installed");
    pretend_installed(&dir.join("ear"));
    let mut harness = harness(app(&dir, vec![big_card()]));
    open_settings(&mut harness);
    harness.get_by_label_contains("Installed: Duck Ear 0.4.0");
    harness.get_by_label_contains("It gives scores only");
    harness.get_by_label("Your graphics card: NVIDIA GeForce RTX 4090 (24 GB).");
    shoot(&mut harness, "16-settings-duck-ear-installed-large-offered");
    press(&mut harness, "Try a bigger model (Large, 19.9 GB)");
    // The question names the model, the size and that the Ear stays as it is.
    harness.get_by_label_contains("Try qwen3-omni-30b-a3b");
    harness.get_by_label_contains("19.9 GB to download");
    harness.get_by_label_contains("It does not change which model the Ear uses");
    harness.get_by_label("Download and try");
    shoot(&mut harness, "17-settings-duck-ear-try-large-question");
    press(&mut harness, "Not now");
    assert!(harness.state().ear.job.is_none());
    assert!(
        !dir.join("ear").join("llama").exists(),
        "saying no downloads nothing"
    );
    drop(harness);

    // A card of 8 GB: the Large model is not offered, and it says why.
    let small = Card {
        name: "NVIDIA GeForce RTX 4060".into(),
        memory_mb: 8188,
    };
    let mut harness = self::harness(app(&dir, vec![small]));
    open_settings(&mut harness);
    harness.get_by_label_contains("graphics card of 16 GB or more");
    harness.get_by_label_contains("this PC's is NVIDIA GeForce RTX 4060 (8 GB)");
    assert!(
        harness
            .query_by_label_contains("Try a bigger model")
            .is_none()
    );
    assert!(!harness.state().ear.large_offered());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_large_models_results_show_with_save_and_use_only_after_a_pass() {
    let dir = folder("results");
    let home = dir.join("ear");
    pretend_installed(&home);
    pretend_tried(&home, 30, false);
    let mut harness = harness(app(&dir, vec![big_card()]));
    open_settings(&mut harness);
    harness.get_by_label_contains("qwen3-omni-30b-a3b@6e35a28f4a19");
    harness.get_by_label("Describe: yes on true texts");
    harness.get_by_label("30 of 47 (64%)");
    harness.get_by_label_contains("It did not reach every bar, so the Ear keeps scores only");
    harness.get_by_label("Save the results...").scroll_to_me();
    assert!(
        harness.query_by_label("Use Large for describing").is_none(),
        "a model that did not pass is not offered"
    );
    shoot(&mut harness, "18-settings-duck-ear-large-results");
    // Save (the folder is chosen in a system dialog; here it is named).
    harness
        .state_mut()
        .ear
        .save_results_into(&dir.join("to-send"));
    harness.run_steps(3);
    for file in ["ear-check.json", "ear-check.md"] {
        assert!(
            dir.join("to-send")
                .join("ear-check-large")
                .join(file)
                .is_file(),
            "{file}"
        );
    }
    harness.get_by_label_contains("Those two files are the ones to send.");
    assert!(
        !home.join("models").join("describe.txt").exists(),
        "trying never switches the Ear"
    );

    // A try that reached every bar: now the Ear offers to use the model,
    // and uses it only when told to.
    pretend_tried(&home, 40, true);
    harness.state_mut().ear.refresh();
    harness.run_steps(3);
    harness.get_by_label("It reached every bar a describe model must reach.");
    assert!(!home.join("models").join("describe.txt").exists());
    shoot(&mut harness, "19-settings-duck-ear-large-passed");
    press(&mut harness, "Use Large for describing");
    assert_eq!(
        std::fs::read_to_string(home.join("models").join("describe.txt"))
            .unwrap()
            .trim(),
        "qwen3-omni-30b-a3b"
    );
    harness.get_by_label("It scores, and describes with qwen3-omni-30b-a3b.");
    press(&mut harness, "Use scores only");
    assert!(!home.join("models").join("describe.txt").exists());
    harness.get_by_label_contains("It gives scores only");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn remove_asks_first_and_takes_only_the_ears_own_things() {
    let dir = folder("remove");
    let home = dir.join("ear");
    pretend_installed(&home);
    std::fs::write(home.join("my notes.txt"), "keep me").unwrap();
    let mut harness = harness(app(&dir, vec![]));
    open_settings(&mut harness);
    press(&mut harness, "Remove the Duck Ear");
    harness.get_by_label_contains("Your sounds are not touched.");
    assert!(
        home.join("ear.json").exists(),
        "nothing is removed before the answer"
    );
    press(&mut harness, "Remove");
    // The work runs on its own thread: draw frames until it is done.
    let started = std::time::Instant::now();
    while harness.state().ear.job.is_some() {
        assert!(started.elapsed().as_secs() < 30, "removing took too long");
        std::thread::sleep(std::time::Duration::from_millis(20));
        harness.run_steps(1);
    }
    harness.run_steps(3);
    harness.get_by_label_contains("The Duck Ear was removed");
    harness.get_by_label("The Duck Ear is not installed.");
    assert!(!home.join("env").exists() && !home.join("models").exists());
    assert_eq!(
        std::fs::read_to_string(home.join("my notes.txt")).unwrap(),
        "keep me"
    );
    std::fs::remove_dir_all(&dir).ok();
}
