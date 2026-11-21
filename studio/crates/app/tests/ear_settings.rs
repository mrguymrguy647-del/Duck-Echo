//! Settings > Duck Ear, drawn with no window: installing is offered with
//! its size, an installed Ear shows, a big graphics card offers the Large
//! model, and its results come with Save and (only after a pass) Use.
//!
//! Nothing here downloads anything or starts a model: the Ear's folder is
//! a pretend one of the test's own, and so are the graphics cards. Test
//! listens with the stub Ear (`ear/stub`), which needs Python: those tests
//! skip without one, unless `DUCK_ECHO_REQUIRE_PYTHON=1` (as in CI).
//!
//! Step 6: Test, Automatic or CPU only, Clear kept answers, Repair, an
//! install that carries on, the note when an AI assistant wanted to
//! listen, and the Studio's Ear following the 10-minute rule.

use std::path::{Path, PathBuf};

use duck_echo_tools::ear_cards::Card;
use duck_echo_tools::ear_home;
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
    for file in &ear_install::manifest().score.files {
        std::fs::write(places.score_model().join(file), b"pretend").unwrap();
    }
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

/// Draw frames until the panel's job is done.
fn wait_for_the_job(harness: &mut Harness<'static, StudioApp>, what: &str) {
    let started = std::time::Instant::now();
    while harness.state().ear.job.is_some() {
        assert!(started.elapsed().as_secs() < 120, "{what} took too long");
        std::thread::sleep(std::time::Duration::from_millis(20));
        harness.run_steps(1);
    }
    harness.run_steps(3);
}

/// The stub Ear, if there is a Python to run it.
fn stub_ear() -> Option<PathBuf> {
    let stub = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ear/stub/duck_ear_stub.py");
    let python = if cfg!(windows) { "python" } else { "python3" };
    let python = std::env::var_os("DUCK_ECHO_EAR_PYTHON").unwrap_or(python.into());
    let found = std::process::Command::new(&python)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        assert!(
            std::env::var("DUCK_ECHO_REQUIRE_PYTHON").as_deref() != Ok("1"),
            "no Python, and DUCK_ECHO_REQUIRE_PYTHON=1"
        );
        eprintln!("skipped: no Python for the stub Ear");
        return None;
    }
    Some(stub)
}

#[test]
fn test_listens_and_the_studios_ear_stops_after_its_idle_time() {
    let Some(stub) = stub_ear() else {
        return;
    };
    let dir = folder("test");
    let home = dir.join("ear");
    pretend_installed(&home);
    let mut app = app(&dir, vec![]);
    // The Studio's Ear keeps the MCP server's rules.
    assert_eq!(app.ear.ear_idle, duck_echo_tools::ear::EAR_IDLE);
    app.ear.program = Some(stub);
    // A short idle time, to see the rule work in a test.
    app.ear.ear_idle = std::time::Duration::from_millis(700);
    let mut harness = harness(app);
    open_settings(&mut harness);
    harness.get_by_label_contains("The Ear stops 1 second after its last question");
    harness.get_by_label("It keeps no answers yet.");
    press(&mut harness, "Test");
    wait_for_the_job(&mut harness, "the test");
    let tested = harness.state().ear.tested.clone().expect("tested");
    assert_eq!(tested.lines.len(), 2);
    harness.get_by_label_contains("coin: \"a coin pickup\":");
    harness.get_by_label_contains("night song: \"calm music\":");
    harness.get_by_label("This is the stub Ear: it does not listen.");
    assert!(
        harness.state().ear.ear_running(),
        "the Ear is kept after Test"
    );
    shoot(&mut harness, "20-settings-duck-ear-test");
    // Nothing is kept by a test, so the next one listens again.
    assert_eq!(ear_home::answers(&home).0, 0);
    let started = std::time::Instant::now();
    while harness.state().ear.ear_running() {
        assert!(started.elapsed().as_secs() < 10, "the Ear was not stopped");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_device_and_the_kept_answers_are_chosen_in_settings() {
    let dir = folder("device");
    let home = dir.join("ear");
    pretend_installed(&home);
    for name in ["ab1.json", "ab2.json", "ab3.json"] {
        std::fs::create_dir_all(home.join("answers/ab")).unwrap();
        std::fs::write(home.join("answers/ab").join(name), "{}").unwrap();
    }
    let mut harness = harness(app(&dir, vec![]));
    open_settings(&mut harness);
    harness.get_by_label_contains("It keeps 3 answers");
    // The MCP server's two rules, in the Studio too.
    harness.get_by_label_contains(
        "The Ear stops 10 minutes after its last question; a describe model lets go of the \
         graphics card after 2 minutes.",
    );
    harness.get_by_label_contains("A describe model runs on the graphics card");
    press(&mut harness, "CPU only");
    assert_eq!(ear_home::device(&home), ear_home::Device::CpuOnly);
    harness.get_by_label_contains("also for an AI assistant's Ear");
    shoot(&mut harness, "21-settings-duck-ear-cpu-only");
    press(&mut harness, "Automatic");
    assert_eq!(ear_home::device(&home), ear_home::Device::Automatic);
    press(&mut harness, "Clear kept answers");
    harness.get_by_label_contains("3 kept answers were forgotten");
    assert_eq!(ear_home::answers(&home).0, 0);
    assert!(home.join("ear.json").exists(), "only the answers go");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_broken_ear_offers_repair_and_says_what_is_missing() {
    let dir = folder("broken");
    let home = dir.join("ear");
    pretend_installed(&home);
    std::fs::remove_file(Places::new(&home).python()).unwrap();
    let mut harness = harness(app(&dir, vec![]));
    open_settings(&mut harness);
    harness.get_by_label_contains("The Duck Ear is broken.");
    harness.get_by_label_contains("private Python is missing");
    shoot(&mut harness, "22-settings-duck-ear-broken");
    press(&mut harness, "Repair the Duck Ear");
    harness.get_by_label_contains("to download");
    harness.get_by_label("Repair");
    assert!(harness.state().ear.job.is_none(), "it asks first");
    press(&mut harness, "Not now");
    std::fs::remove_dir_all(&dir).ok();
}

/// A note in the Ear's folder that an install stopped: how, and who
/// began it, as `ear install` leaves it.
fn pretend_stopped(home: &Path, by: &str, stopped: Option<&str>) {
    std::fs::create_dir_all(home.join("downloads")).unwrap();
    std::fs::write(home.join("downloads/uv.zip.part"), vec![0u8; 3_000_000]).unwrap();
    let mut note = json!({"version": 1, "by": by, "started": 1, "beat": 1});
    if let Some(stopped) = stopped {
        note["stopped"] = json!(stopped);
    }
    std::fs::write(home.join("installing.json"), note.to_string()).unwrap();
}

#[test]
fn an_install_the_studio_was_running_when_it_closed_carries_on_by_itself() {
    let dir = folder("carry-on");
    let home = dir.join("ear");
    pretend_stopped(&home, "studio", None);
    let mut app = app(&dir, vec![]);
    // Nothing on this machine answers there, so nothing is downloaded.
    app.ear.uv_url = Some("http://127.0.0.1:9/uv.zip".into());
    let mut harness = harness(app);
    harness.run_steps(2);
    let job = harness.state().ear.job.as_ref().map(|job| job.title);
    assert_eq!(job, Some("Installing the Duck Ear"));
    harness.get_by_label_contains("Installing the Duck Ear:");
    wait_for_the_job(&mut harness, "the install");
    // It failed here (no internet at that address), and says so; now it
    // waits for a person.
    let note = harness.state().ear.note.clone().expect("a note");
    assert!(note.0, "{note:?}");
    assert!(matches!(
        ear_install::state(&home),
        ear_install::State::Stopped(ear_install::Stopped {
            ended: ear_install::Ended::Failed(_),
            ..
        })
    ));
    open_settings(&mut harness);
    harness.get_by_label_contains("An install of the Duck Ear did not finish");
    harness.get_by_label("Carry on installing");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_install_stopped_by_a_person_or_begun_elsewhere_waits_for_a_person() {
    let dir = folder("waits");
    let home = dir.join("ear");
    for (by, stopped) in [("studio", Some("person")), ("command_line", None)] {
        pretend_stopped(&home, by, stopped);
        let mut harness = harness(app(&dir, vec![]));
        harness.run_steps(3);
        assert!(harness.state().ear.job.is_none(), "{by}: nothing by itself");
        open_settings(&mut harness);
        harness.get_by_label_contains("An install of the Duck Ear did not finish");
        harness.get_by_label_contains("3 MB is already here");
        if by == "studio" {
            shoot(&mut harness, "23-settings-duck-ear-stopped");
        }
        press(&mut harness, "Carry on installing");
        harness.get_by_label_contains("already there and right is kept");
        harness.get_by_label("Carry on installing");
        press(&mut harness, "Not now");
    }
    // One that runs now, in another program: no buttons.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let note = json!({"version": 1, "by": "command_line", "started": now, "beat": now});
    std::fs::write(home.join("installing.json"), note.to_string()).unwrap();
    let mut harness = harness(app(&dir, vec![]));
    open_settings(&mut harness);
    harness.get_by_label("The Duck Ear is being installed by duck-echo ear install now.");
    assert!(harness.query_by_label("Carry on installing").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_assistant_that_wanted_to_listen_gets_a_note_once() {
    let dir = folder("wanted");
    let home = dir.join("ear");
    ear_home::note_wanted(&home, "listen");
    ear_home::note_wanted(&home, "listen");
    let mut harness = harness(app(&dir, vec![]));
    harness.run_steps(3);
    harness.get_by_label("An AI assistant wanted to listen to a sound.");
    harness.get_by_label_contains("it asked twice");
    shoot(&mut harness, "24-assistant-wanted-to-listen");
    let plan = ear_install::plan(&home).unwrap();
    let size = megabytes(plan.download_bytes);
    // Install opens Settings with the question; nothing starts by itself.
    harness
        .get_by_label(&format!("Install the Duck Ear (about {size})..."))
        .click();
    harness.run_steps(30);
    assert!(harness.state().settings_open);
    harness.get_by_label_contains(&format!("about {size} to download"));
    assert!(harness.state().ear.job.is_none());
    press(&mut harness, "Not now");
    harness.state_mut().settings_open = false;
    harness.run_steps(3);
    // Not now: the note goes, and stays gone.
    harness.get_by_label("Not now").click();
    harness.run_steps(3);
    let first = ear_home::wanted(&home).unwrap().first;
    assert_eq!(harness.state().settings.ear_wanted_seen, Some(first));
    assert!(
        harness
            .query_by_label("An AI assistant wanted to listen to a sound.")
            .is_none()
    );
    // Settings still says it, quietly.
    open_settings(&mut harness);
    harness.get_by_label_contains("An AI assistant asked to listen twice");
    std::fs::remove_dir_all(&dir).ok();
}
