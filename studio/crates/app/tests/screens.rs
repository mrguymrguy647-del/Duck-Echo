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
        // A frame a sixtieth of a second, as on a real screen, so a
        // double-click (two clicks within 0.3 s) is one.
        .with_step_dt(1.0 / 60.0)
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

/// The button labelled `button` on the card whose name is `card`: of all
/// such buttons, the nearest one below the name. (Every card has its own
/// "Play", so a button is found by its card.)
fn button_on<'h>(
    harness: &'h Harness<'static, StudioApp>,
    card: &'h str,
    button: &'h str,
) -> egui_kittest::Node<'h> {
    let names: Vec<_> = harness.query_all_by_label(card).collect();
    assert!(!names.is_empty(), "no card named {card}");
    let mut best = None;
    for name in &names {
        let at = name.rect().center();
        for node in harness.query_all_by_label(button) {
            let to = node.rect().center();
            let below = to.y - at.y;
            if below > 0.0 && (to.x - at.x).abs() < 220.0 {
                let distance = below + (to.x - at.x).abs();
                if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                    best = Some((distance, node));
                }
            }
        }
    }
    best.unwrap_or_else(|| panic!("no {button} on the card {card}"))
        .1
}

/// Draw frames until `done`, waiting for the background thumbnails.
fn until(harness: &mut Harness<'static, StudioApp>, what: &str, done: impl Fn(&StudioApp) -> bool) {
    let started = std::time::Instant::now();
    while !done(harness.state()) {
        assert!(
            started.elapsed().as_secs() < 30,
            "waited too long for {what}; the status line says {:?}",
            harness.state().note
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        harness.state_mut().last_scan = None;
        harness.run_steps(1);
    }
    harness.run_steps(2);
}

#[test]
fn home_shows_my_sounds_and_the_gallery_with_its_categories() {
    let dir = folder("home");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(3);
    for label in [
        "Duck Echo Studio",
        "Undo",
        "Redo",
        "New sound",
        "New song",
        "Search:",
        "My sounds and songs",
        "Start from",
        "Presets",
        "Instruments",
        "Example songs",
    ] {
        harness.get_by_label(label);
    }
    for category in [
        "Pickup", "Impact", "Weapon", "Alert", "Movement", "UI", "Ambient",
    ] {
        harness.get_by_label(category);
    }
    // Pickups first: coin is there, explosion is not.
    harness.get_by_label("coin");
    harness.get_by_label("Classic coin pickup.");
    assert!(harness.query_by_label("explosion").is_none());
    harness.get_by_label_contains("Nothing here yet.");
    // The project folder was made, and the status line says where it is.
    assert!(dir.join("Duck Echo").is_dir());
    harness.get_by_label(&format!(
        "Your sounds and songs: {}",
        dir.join("Duck Echo").display()
    ));
    shoot(&mut harness, "01-home-empty");
    // Impact shows the impacts.
    harness.get_by_label("Impact").click();
    harness.run_steps(2);
    harness.get_by_label("explosion");
    harness.get_by_label("big_explosion");
    assert!(harness.query_by_label("coin").is_none());
    button_on(&harness, "explosion", "Play");
    button_on(&harness, "explosion", "Make a copy");
    shoot(&mut harness, "02-home-impact");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn make_a_copy_rename_delete_and_undo_as_the_checklist_does() {
    let dir = folder("copy");
    let project = dir.join("Duck Echo");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(3);
    // Make a copy of coin: it appears under My sounds and songs, with its
    // picture and length.
    button_on(&harness, "coin", "Make a copy").click();
    harness.run_steps(2);
    assert!(project.join("coin.toml").is_file());
    let text = std::fs::read_to_string(project.join("coin.toml")).unwrap();
    assert!(
        text.contains("description = \"Copy of the coin preset.\""),
        "{text}"
    );
    harness.get_by_label_contains("Made a copy of coin");
    until(&mut harness, "the coin's picture", |app| !app.thumbs.busy());
    harness.get_by_label("sound, 0.31 s");
    assert_eq!(
        harness.query_all_by_label("coin").count(),
        2,
        "the gallery's and mine"
    );
    harness.get_by_label("Undo");
    shoot(&mut harness, "03-home-a-copy-of-coin");

    // Rename it to "my coin" through its ... menu.
    let mine = harness
        .query_all_by_label("...")
        .next()
        .expect("my card's ... menu");
    mine.click();
    harness.run_steps(2);
    harness.get_by_label("Duplicate");
    harness.get_by_label("Delete");
    harness.get_by_label("Rename...").click();
    harness.run_steps(2);
    harness.get_by_label("Rename coin");
    harness.state_mut().rename = Some(("coin.toml".into(), "my coin".into()));
    harness.run_steps(1);
    shoot(&mut harness, "04-rename");
    harness.get_by_label("Rename").click();
    harness.run_steps(3);
    assert!(project.join("my coin.toml").is_file());
    assert!(!project.join("coin.toml").exists());
    let text = std::fs::read_to_string(project.join("my coin.toml")).unwrap();
    assert!(
        text.contains("name = \"my coin\""),
        "the name follows: {text}"
    );
    harness.get_by_label("my coin");

    // Delete it: it goes. Ctrl+Z: it comes back. Redo: it goes again.
    harness.query_all_by_label("...").next().unwrap().click();
    harness.run_steps(2);
    harness.get_by_label("Delete").click();
    harness.run_steps(3);
    assert!(!project.join("my coin.toml").exists());
    assert!(harness.query_by_label("my coin").is_none());
    harness.get_by_label_contains("Deleted my coin.");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert!(
        project.join("my coin.toml").is_file(),
        "Ctrl+Z brings it back"
    );
    harness.get_by_label("my coin");
    harness.get_by_label("Redo").click();
    harness.run_steps(3);
    assert!(
        !project.join("my coin.toml").exists(),
        "Redo deletes it again"
    );
    // Every step was the person's, in the shared history.
    let history = std::fs::read_dir(project.join(".duck-echo").join("history"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "json")
        })
        .count();
    assert_eq!(history, 3, "copy, rename and delete");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn small_text_is_big_enough_to_read() {
    let ctx = egui::Context::default();
    theme::apply_brand(&ctx);
    for t in [egui::Theme::Dark, egui::Theme::Light] {
        let size = ctx.style_of(t).text_styles[&egui::TextStyle::Small].size;
        assert_eq!(size, theme::SMALL_TEXT);
        assert!(size >= 12.0, "small text is {size} px");
    }
}

/// Wait until the open sound's newest render has arrived.
fn heard(harness: &mut Harness<'static, StudioApp>) {
    until(harness, "the sound's render", |app| {
        app.editor
            .as_ref()
            .is_some_and(|e| e.heard.is_some() && !e.rendering())
    });
}

#[test]
fn open_a_sound_and_edit_it_as_the_checklist_does() {
    let dir = folder("editor");
    let project = dir.join("Duck Echo");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(3);
    button_on(&harness, "coin", "Make a copy").click();
    harness.run_steps(2);
    let before = std::fs::read_to_string(project.join("coin.toml")).unwrap();

    // 1. Open your coin: click its card. The text is on the left and the
    // sliders, made from the schema, on the right.
    let mine = harness
        .query_all_by_label("coin")
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()).reverse())
        .unwrap();
    mine.click();
    harness.run_steps(3);
    assert!(
        harness.state().editor.is_some(),
        "the card opens the editor: {:?}",
        harness.state().note
    );
    for label in [
        "< Home",
        "coin.toml",
        "Play",
        "Loop",
        "Stop",
        "History",
        "TEXT",
        "SOUND",
        "MASTER",
        "LAYER 1  square",
        "decay",
        "attack",
        "volume",
        "duty",
        "+ Add a layer",
        "WHAT IT SOUNDS LIKE",
    ] {
        // A field's name also labels its slider and its number box.
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "{label} is not on the screen"
        );
    }
    // A square layer has no noise settings.
    assert!(harness.query_by_label("noise mode").is_none());
    heard(&mut harness);
    harness.get_by_label_contains("0.31 s,");
    shoot(&mut harness, "05-sound-editor");

    // 4. In the text, change the decay to 0.6: it is saved after a
    // moment, and the slider reads it.
    let changed = before.replace("decay = 0.25", "decay = 0.6");
    {
        let app = harness.state_mut();
        let editor = app.editor.as_mut().unwrap();
        editor.text = changed.clone();
        editor.typed(std::time::Instant::now());
    }
    until(&mut harness, "the typing to be saved", |_| {
        std::fs::read_to_string(project.join("coin.toml")).unwrap() == changed
    });
    let doc = harness
        .state()
        .editor
        .as_ref()
        .unwrap()
        .doc()
        .unwrap()
        .clone();
    assert_eq!(
        doc["layers"][0]["envelope"]["decay"],
        serde_json::json!(0.6)
    );

    // 5. Misspell volume: the message is under the line, with the button.
    let wrong = changed.replace("source = ", "volme = 0.8\nsource = ");
    {
        let app = harness.state_mut();
        let editor = app.editor.as_mut().unwrap();
        editor.text = wrong;
        editor.typed(std::time::Instant::now());
    }
    harness.run_steps(3);
    harness.get_by_label_contains("volme");
    let button = harness.get_by_label("Use `volume`");
    // The message is in the text, not in a list somewhere else: below the
    // TEXT heading and left of the sliders. (The screenshot shows it right
    // under its line.)
    let text = harness.get_by_label("TEXT").rect();
    let sliders = harness.get_by_label("SOUND").rect();
    assert!(
        button.rect().top() > text.bottom() + 100.0,
        "{:?}",
        button.rect()
    );
    assert!(
        button.rect().right() < sliders.left(),
        "{:?}",
        button.rect()
    );
    shoot(&mut harness, "06-a-problem-under-its-line");
    harness.get_by_label("Use `volume`").click();
    harness.run_steps(2);
    let fixed = changed.replace("source = ", "volume = 0.8\nsource = ");
    until(&mut harness, "the fixed text to be saved", |_| {
        std::fs::read_to_string(project.join("coin.toml")).unwrap() == fixed
    });

    // 7. History lists every change with "You".
    harness.get_by_label("History").click();
    harness.run_steps(2);
    harness.get_by_label("HISTORY");
    assert!(harness.query_all_by_label("You").count() >= 3);
    assert_eq!(
        harness
            .query_all_by_label_contains("Edit the text of coin.toml")
            .count(),
        2,
        "the decay typed, and the volume fixed"
    );
    shoot(&mut harness, "07-history");

    // 6. Ctrl+Z takes the changes back one at a time, even with the text
    // box in use.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert_eq!(
        std::fs::read_to_string(project.join("coin.toml")).unwrap(),
        changed
    );
    assert_eq!(harness.state().editor.as_ref().unwrap().text, changed);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert_eq!(
        std::fs::read_to_string(project.join("coin.toml")).unwrap(),
        before
    );

    // Export WAV to the Duck Echo folder, from the ... menu.
    harness.get_by_label("...").click();
    harness.run_steps(2);
    harness.get_by_label("Export WAV...");
    harness
        .get_by_label("Export WAV to my Duck Echo folder")
        .click();
    harness.run_steps(2);
    assert!(project.join("exports").join("coin.wav").is_file());
    harness.get_by_label_contains("Exported");

    // Back to Home.
    harness.get_by_label("< Home").click();
    harness.run_steps(2);
    assert!(harness.state().editor.is_none());
    harness.get_by_label("My sounds and songs");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn open_a_song_in_the_timeline_as_the_checklist_does() {
    let dir = folder("timeline");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    // 1. Example songs: Make a copy of the day theme, then open it.
    harness.get_by_label("Example songs").click();
    harness.run_steps(2);
    button_on(&harness, "duck_craft_day", "Make a copy").click();
    harness.run_steps(2);
    let mine = harness
        .query_all_by_label("duck_craft_day")
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .unwrap();
    mine.click();
    harness.run_steps(3);
    assert!(
        harness.state().song_view.is_some(),
        "the card opens the timeline: {:?}",
        harness.state().note
    );
    for label in ["< Home", "duck_craft_day.toml", "Play", "Loop", "Stop"] {
        harness.get_by_label(label);
    }
    harness.get_by_label_contains("84 BPM, 4/4, 8 bars, loops");
    // A lane per track, each with Mute and Solo.
    for track in ["pad", "bass", "keys", "drums"] {
        // (The pad, the bass and the drums play instruments of the same
        // name, shown under it.)
        assert!(harness.query_all_by_label(track).next().is_some());
        button_on(&harness, track, "Mute");
        button_on(&harness, track, "Solo");
    }
    // 5. Hover over a note: its name, bar and beat show.
    let note = harness.get_by_label("keys: E5, bar 1, beat 1");
    note.hover();
    // A tooltip waits half a second.
    harness.run_steps(60);
    harness.get_by_label_contains("held 1 beat");
    // The note check: the summary line, and every note heard.
    until(&mut harness, "the note check", |app| {
        app.song_view
            .as_ref()
            .is_some_and(|v| v.checked.is_some() && !v.checking())
    });
    harness.get_by_label_contains("8 bars looping at 84 BPM");
    let checked = harness
        .state()
        .song_view
        .as_ref()
        .unwrap()
        .checked
        .clone()
        .unwrap();
    assert!(checked.problems.is_empty(), "{:?}", checked.problems);
    shoot(&mut harness, "08-song-timeline");

    // 3. Solo on the drums: only the drums are heard. Again: everything.
    button_on(&harness, "drums", "Solo").click();
    harness.run_steps(2);
    let drums = 3;
    assert_eq!(
        harness.state().song_view.as_ref().unwrap().heard(),
        Some(vec![drums])
    );
    button_on(&harness, "drums", "Solo").click();
    harness.run_steps(2);
    assert_eq!(harness.state().song_view.as_ref().unwrap().heard(), None);
    // Mute the pad: everything but the pad.
    button_on(&harness, "pad", "Mute").click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().song_view.as_ref().unwrap().heard(),
        Some(vec![1, 2, 3])
    );

    // A change made elsewhere shows: an AI assistant moves the tempo.
    let project = dir.join("Duck Echo");
    let text = std::fs::read_to_string(project.join("duck_craft_day.toml")).unwrap();
    std::fs::write(
        project.join("duck_craft_day.toml"),
        text.replace("tempo = 84", "tempo = 100"),
    )
    .unwrap();
    until(&mut harness, "the new tempo", |app| {
        app.song_view
            .as_ref()
            .and_then(|v| v.loaded.as_ref())
            .is_some_and(|(song, _)| song.tempo == 100.0)
    });
    harness.get_by_label_contains("100 BPM");

    harness.get_by_label("< Home").click();
    harness.run_steps(2);
    assert!(harness.state().song_view.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// Press and let go of the mouse at `at`, `times` times (2 is a
/// double-click).
fn click_at(harness: &mut Harness<'static, StudioApp>, at: egui::Pos2, times: usize) {
    harness.hover_at(at);
    // Half a second first: two quick clicks soon after an earlier click
    // count as a triple-click, not a double-click.
    harness.run_steps(30);
    for _ in 0..times {
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    harness.run_steps(1);
    harness.run_steps(2);
}

/// Drag with the mouse from `from` to `to`, in small moves.
fn drag(harness: &mut Harness<'static, StudioApp>, from: egui::Pos2, to: egui::Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.drag_at(from);
    harness.run_steps(1);
    for step in 1..=10 {
        let at = from + (to - from) * (step as f32 / 10.0);
        harness.hover_at(at);
        harness.run_steps(1);
    }
    harness.drop_at(to);
    harness.run_steps(3);
}

fn song_text(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("Duck Echo").join("duck_craft_day.toml")).unwrap()
}

#[test]
fn edit_the_song_in_the_timeline_as_the_checklist_does() {
    let dir = folder("song-edit");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    harness.get_by_label("Example songs").click();
    harness.run_steps(2);
    button_on(&harness, "duck_craft_day", "Make a copy").click();
    harness.run_steps(2);
    harness.state_mut().open_song("duck_craft_day.toml");
    harness.run_steps(3);
    let before = song_text(&dir);
    let keys_before = {
        let view = harness.state().song_view.as_ref().unwrap();
        view.lanes[2].blocks.len()
    };

    // 1. Double-click an empty spot in the keys lane: a note appears.
    // The lane's bottom row (two below its lowest note) is empty.
    let lane = harness.get_by_label("keys lane").rect();
    let (x0, beat_px) = {
        let first = harness.get_by_label("keys: E5, bar 1, beat 1").rect();
        let second = harness.get_by_label("keys: G5, bar 1, beat 2").rect();
        (first.left() - 1.0, second.left() - first.left())
    };
    let spot = egui::pos2(x0 + beat_px * 5.0 + 3.0, lane.bottom() - 11.0);
    click_at(&mut harness, spot, 2);
    let added = song_text(&dir);
    assert_ne!(
        added,
        before,
        "a double-click adds a note: {:?}",
        harness.state().note
    );
    harness.get_by_label_contains("Added");
    let view = harness.state().song_view.as_ref().unwrap();
    assert_eq!(view.lanes[2].blocks.len(), keys_before + 1);
    // The bottom row is A#4, two below the tune's lowest note.
    let new_note = view.lanes[2]
        .blocks
        .iter()
        .find(|b| b.bar == 2 && b.beat_in_bar == 1.0 && b.label == "A#4")
        .expect("the new A#4 is at bar 2, beat 2")
        .clone();
    assert_eq!(new_note.length, 1.0);
    shoot(&mut harness, "09-a-note-added");

    // 2. Drag the note up three rows: a higher pitch.
    let name = studio_app::song_screen::note_name("keys", &new_note);
    let block = harness.get_by_label(&name).rect();
    let row_px = 7.0;
    drag(
        &mut harness,
        block.left_center() + egui::vec2(4.0, 0.0),
        block.left_center() + egui::vec2(4.0, -3.0 * row_px),
    );
    let moved = song_text(&dir);
    assert_ne!(moved, added, "the drag moved the note");
    let view = harness.state().song_view.as_ref().unwrap();
    // Three rows up: A#4 to C#5, still at bar 2, beat 2.
    let higher = view.lanes[2]
        .blocks
        .iter()
        .find(|b| b.bar == 2 && b.beat_in_bar == 1.0 && b.label == "C#5")
        .expect("the note is C#5 now")
        .clone();
    assert!(!view.lanes[2].blocks.iter().any(|b| b.label == "A#4"));

    // 3. Drag its right edge: it gets longer.
    let name = studio_app::song_screen::note_name("keys", &higher);
    let block = harness.get_by_label(&name).rect();
    drag(
        &mut harness,
        block.right_center() - egui::vec2(2.0, 0.0),
        block.right_center() + egui::vec2(beat_px, 0.0),
    );
    let view = harness.state().song_view.as_ref().unwrap();
    let longer = view.lanes[2]
        .blocks
        .iter()
        .find(|b| b.label == "C#5")
        .unwrap();
    assert_eq!(longer.length, 2.0);

    // Move a bass clip from bar 3 to bar 4... bar 4 has one; to bar 1?
    // Bar 3's clip is alone in its placement, so move it to bar 3's
    // neighbour that its placement does not have: the placement is
    // [3], so any bar works. Drag it one bar to the right.
    let clip = harness.get_by_label("bass: root_and_fifth at bar 3").rect();
    drag(
        &mut harness,
        clip.center(),
        clip.center() + egui::vec2(beat_px * 4.0, 0.0),
    );
    harness.get_by_label_contains("Moved root_and_fifth on bass to bar 4");
    assert!(
        harness
            .query_by_label("bass: root_and_fifth at bar 3")
            .is_none()
    );

    // The track volume: type a new one.
    let edits_before = song_text(&dir);
    harness.state_mut().run_command(
        "song_set",
        serde_json::json!({"file": "duck_craft_day.toml", "path": "tracks[0].volume", "value": 0.5}),
    );
    harness.state_mut().song_after_edit();
    harness.run_steps(2);
    assert_ne!(song_text(&dir), edits_before);
    assert_eq!(
        harness.state().song_view.as_ref().unwrap().lanes[0].volume,
        0.5
    );

    // 5. Ctrl+Z until the note is gone: every change was one step.
    for _ in 0..5 {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run_steps(3);
    }
    assert_eq!(song_text(&dir), before);
    let view = harness.state().song_view.as_ref().unwrap();
    assert_eq!(view.lanes[2].blocks.len(), keys_before);

    // Click a note, press Delete: it goes. Ctrl+Z: it is back.
    let first = harness.get_by_label("keys: E5, bar 1, beat 1").rect();
    click_at(&mut harness, first.center(), 1);
    harness.key_press(egui::Key::Delete);
    harness.run_steps(3);
    assert!(harness.query_by_label("keys: E5, bar 1, beat 1").is_none());
    harness.get_by_label_contains("Deleted the note");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    harness.get_by_label("keys: E5, bar 1, beat 1");

    // Right-click an empty spot of the keys lane at bar 3: place a
    // pattern there. (Every bar of the bass is a clip already.)
    let lane = harness.get_by_label("keys lane").rect();
    let at = egui::pos2(x0 + beat_px * 9.0 + 3.0, lane.bottom() - 11.0);
    harness.hover_at(at);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(3);
    harness.get_by_label("Place a pattern at bar 3").click();
    harness.run_steps(3);
    harness.get_by_label("root_and_fifth").click();
    harness.run_steps(3);
    harness.get_by_label_contains("Placed root_and_fifth on keys at bar 3");
    assert!(song_text(&dir).contains("bars = [3]"));

    // The keys played by another instrument.
    harness.get_by_label("keys instrument").click();
    harness.run_steps(2);
    harness.get_by_label("chip_lead").click();
    harness.run_steps(3);
    harness.get_by_label_contains("keys is played by chip_lead now");
    assert!(song_text(&dir).contains("instrument = \"chip_lead\""));
    shoot(&mut harness, "10-song-edited");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn every_preset_and_instrument_gets_its_sliders() {
    let dir = folder("all-sliders");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(1);
    let mut sources: Vec<String> = duck_echo_tools::presets::PRESETS
        .iter()
        .map(|p| format!("preset:{}", p.name))
        .collect();
    sources.extend(
        duck_echo_tools::instruments::list()
            .unwrap()
            .into_iter()
            .filter(|i| i.kind != "kit")
            .map(|i| format!("instrument:{}", i.name)),
    );
    for source in sources {
        let name = source.split_once(':').unwrap().1.to_string();
        let rel = format!("{name}.toml");
        harness.state_mut().make_copy(&source, &name);
        let copied = std::fs::read_to_string(dir.join("Duck Echo").join(&rel)).unwrap();
        harness.state_mut().open_sound(&rel);
        harness.run_steps(4);
        assert!(harness.state().editor.is_some(), "{source} opens");
        // Opening a sound never changes it.
        assert_eq!(
            std::fs::read_to_string(dir.join("Duck Echo").join(&rel)).unwrap(),
            copied,
            "opening {source} changed it"
        );
        harness.get_by_label_contains("LAYER 1");
        harness.get_by_label("+ Add a layer");
        harness.state_mut().close_editor();
        harness.run_steps(1);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_sound_new_song_duplicate_and_search() {
    let dir = folder("new");
    let project = dir.join("Duck Echo");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    harness.get_by_label("New sound").click();
    harness.run_steps(2);
    harness.get_by_label("New sound").click();
    harness.run_steps(2);
    harness.get_by_label("New song").click();
    harness.run_steps(2);
    assert!(project.join("new sound.toml").is_file());
    assert!(project.join("new sound 2.toml").is_file());
    assert!(project.join("new song.toml").is_file());
    until(&mut harness, "the pictures", |app| !app.thumbs.busy());
    harness.get_by_label("song, 4 bars");
    assert_eq!(harness.query_all_by_label("sound, 0.31 s").count(), 2);
    // Duplicate the song.
    let song = button_on(&harness, "new song", "...");
    song.click();
    harness.run_steps(2);
    harness.get_by_label("Duplicate").click();
    harness.run_steps(2);
    assert!(project.join("new song copy.toml").is_file());

    // Search: only what matches, in both halves.
    harness.get_by_label("Search:").focus();
    harness.run_steps(1);
    harness.get_by_label("Search:").type_text("song");
    harness.run_steps(3);
    harness.get_by_label("new song");
    harness.get_by_label("new song copy");
    assert!(harness.query_by_label("new sound").is_none());
    harness.state_mut().search = "explo".into();
    harness.run_steps(3);
    harness.get_by_label("explosion");
    harness.get_by_label("big_explosion");
    harness.get_by_label("None of your sounds or songs match the search.");
    shoot(&mut harness, "05-search");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn instruments_and_example_songs_can_be_copied_too() {
    let dir = folder("shelves");
    let project = dir.join("Duck Echo");
    let mut harness = harness(app(&dir, AppMenu::none()), true);
    harness.run_steps(2);
    harness.get_by_label("Instruments").click();
    harness.run_steps(2);
    for name in [
        "piano",
        "pad",
        "bass",
        "chip_lead",
        "kick",
        "snare",
        "hat",
        "clap",
        "drums",
    ] {
        harness.get_by_label(name);
    }
    harness.get_by_label("Play its pieces: kick, snare, hat and clap.");
    shoot(&mut harness, "06-instruments");
    button_on(&harness, "piano", "Make a copy").click();
    harness.run_steps(2);
    assert!(
        project.join("piano.toml").is_file(),
        "{:?} {:?}",
        harness.state().note,
        std::fs::read_dir(&project)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>()
    );

    harness.get_by_label("Example songs").click();
    harness.run_steps(2);
    for name in ["duck_craft_day", "duck_craft_night", "four_bars"] {
        harness.get_by_label(name);
    }
    button_on(&harness, "four_bars", "Make a copy").click();
    harness.run_steps(2);
    let text = std::fs::read_to_string(project.join("four_bars.toml")).unwrap();
    assert!(
        text.contains("Copy of the four_bars example song."),
        "{text}"
    );
    until(&mut harness, "the song's picture", |app| !app.thumbs.busy());
    harness.get_by_label("song, 4 bars");
    shoot(&mut harness, "07-example-songs");
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
    // Let the window size and centre itself before clicking in it.
    harness.run_steps(30);
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
