//! The sound editor without a window: the text and the file kept in step,
//! the slow saves of a slider drag, problems with their suggestions, and
//! Export WAV.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use duck_echo_tools::commands;
use duck_echo_tools::history::Who;
use duck_echo_tools::project::Project;
use duck_echo_tools::root::Root;
use serde_json::{Value, json};
use studio_app::editor::{self, Editor, SAVE_EVERY, TYPING_PAUSE};

/// A project of the test's own, holding a copy of the coin preset.
fn project(name: &str) -> (PathBuf, Root) {
    let dir = std::env::temp_dir().join(format!("duck-echo-editor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = Root::new(&dir).unwrap();
    run(
        &root,
        "file_copy_from",
        json!({"from": "preset:coin", "to": "coin.toml"}),
    );
    (dir, root)
}

fn run(root: &Root, name: &str, args: Value) -> Value {
    let command = commands::parse(name, &args, "").unwrap();
    commands::run(&Project::new(root), command, Who::Person, None, "")
        .unwrap_or_else(|e| panic!("{name}: {:?}", e.into_report()))
}

fn steps(root: &Root) -> usize {
    Project::new(root).history_list(1000).unwrap().steps.len()
}

fn file(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("coin.toml")).unwrap()
}

#[test]
fn a_drag_through_200_values_in_one_second_saves_at_most_6_times_as_one_undo_step() {
    let (dir, root) = project("drag");
    let before = file(&dir);
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    let steps_before = steps(&root);
    let start = Instant::now();
    let value = |i: u32| json!(0.1 + f64::from(i) * 0.002);
    for i in 0..200 {
        let now = start + Duration::from_millis(5 * u64::from(i));
        editor
            .slide(&root, "layers[0].envelope.decay", value(i), now)
            .unwrap();
        // A frame of the window between two moves.
        assert_eq!(editor.tick(&root, now + Duration::from_millis(2)), None);
        // The text shows the value at once, before it is saved.
        let shown = format!("decay = {}", value(i));
        assert!(
            editor.text.contains(&shown),
            "{shown} not in\n{}",
            editor.text
        );
    }
    editor
        .release(&root, start + Duration::from_millis(1000))
        .unwrap();
    assert!(
        editor.saves <= 6,
        "a one-second drag saved {} times (at most 6: about 5 a second, and one on release)",
        editor.saves
    );
    assert!(editor.saves >= 2, "{}", editor.saves);
    // One Undo step, ending at the value let go on.
    assert_eq!(steps(&root), steps_before + 1);
    let after = file(&dir);
    assert!(
        after.contains(&format!("decay = {}", value(199))),
        "{after}"
    );
    // The preset's comments are still in the file.
    let first_comment = before.lines().find(|l| l.starts_with('#')).unwrap();
    assert!(after.contains(first_comment), "{after}");
    // Only the one number changed.
    assert_eq!(
        before.replace("decay = 0.25", &format!("decay = {}", value(199))),
        after
    );
    // One Undo puts the whole drag back.
    run(&root, "edit_undo", json!({}));
    assert_eq!(file(&dir), before);
    editor.reload(&root);
    assert_eq!(editor.text, before);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_held_slider_that_stops_moving_is_saved_by_the_next_frame() {
    let (dir, root) = project("held");
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    let start = Instant::now();
    let path = "layers[0].volume";
    editor.slide(&root, path, json!(0.5), start).unwrap();
    editor
        .slide(&root, path, json!(0.6), start + Duration::from_millis(50))
        .unwrap();
    assert!(file(&dir).contains("volume = 0.5"));
    editor.tick(&root, start + SAVE_EVERY + Duration::from_millis(10));
    assert!(file(&dir).contains("volume = 0.6"));
    // Two sliders are two Undo steps.
    let steps_before = steps(&root);
    editor
        .slide(
            &root,
            "layers[0].duty",
            json!(0.3),
            start + Duration::from_secs(1),
        )
        .unwrap();
    editor
        .release(&root, start + Duration::from_secs(1))
        .unwrap();
    assert_eq!(steps(&root), steps_before + 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn typing_is_saved_after_a_pause_and_a_mistake_never_is() {
    let (dir, root) = project("typing");
    let before = file(&dir);
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    let start = Instant::now();

    // A changed number: saved once the typing stops, and the sliders read it.
    editor.text = before.replace("decay = 0.25", "decay = 0.6");
    editor.typed(start);
    assert_eq!(editor.tick(&root, start + Duration::from_millis(100)), None);
    assert_eq!(file(&dir), before, "saved while still typing");
    editor.tick(&root, start + TYPING_PAUSE + Duration::from_millis(10));
    assert!(file(&dir).contains("decay = 0.6"));
    assert_eq!(
        editor.doc().unwrap()["layers"][0]["envelope"]["decay"],
        json!(0.6)
    );

    // A misspelt field: the problem is on its line, with the suggestion.
    let saved = file(&dir);
    editor.text = saved.replace("source = ", "volme = 0.8\nsource = ");
    let later = start + Duration::from_secs(2);
    editor.typed(later);
    assert!(!editor.works());
    let (i, problem) = editor
        .problems
        .iter()
        .enumerate()
        .find(|(_, p)| p.message.contains("volme"))
        .expect("a problem names volme");
    let line = problem.line.expect("the problem has its line");
    assert_eq!(
        editor.text.lines().nth(line - 1).unwrap().trim(),
        "volme = 0.8"
    );
    assert_eq!(problem.suggestion.as_deref(), Some("volume"));
    editor.tick(&root, later + TYPING_PAUSE * 2);
    assert_eq!(file(&dir), saved, "a text with a mistake was saved");

    // "Use `volume`" fixes the text, and then it is saved.
    editor.use_suggestion(i, later + TYPING_PAUSE * 2);
    assert!(editor.works(), "{:?}", editor.problems);
    assert!(editor.text.contains("volume = 0.8"));
    editor.tick(&root, later + TYPING_PAUSE * 4);
    assert!(file(&dir).contains("volume = 0.8"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn undo_with_a_mistake_typed_takes_the_typing_back() {
    let (dir, root) = project("undo-typing");
    let before = file(&dir);
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    editor.text = before.replace("source = ", "volme = 1\nsource = ");
    editor.typed(Instant::now());
    assert!(editor.before_undo(&root, Instant::now()));
    assert_eq!(editor.text, before);
    // A working text is saved first, so Undo then takes it back.
    editor.text = before.replace("decay = 0.25", "decay = 0.4");
    editor.typed(Instant::now());
    assert!(!editor.before_undo(&root, Instant::now()));
    assert!(file(&dir).contains("decay = 0.4"));
    run(&root, "edit_undo", json!({}));
    editor.reload(&root);
    assert_eq!(editor.text, before);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_change_made_elsewhere_shows_in_the_text() {
    let (dir, root) = project("elsewhere");
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    // An AI assistant changes the coin.
    let command = commands::parse(
        "sound_set",
        &json!({"file": "coin.toml", "path": "layers[0].volume", "value": 0.3}),
        "",
    )
    .unwrap();
    commands::run(&Project::new(&root), command, Who::Agent, None, "").unwrap();
    editor.tick(&root, Instant::now() + Duration::from_secs(1));
    assert!(editor.text.contains("volume = 0.3"), "{}", editor.text);
    assert_eq!(editor.history[0].who, "AI assistant");
    // Deleted elsewhere: the editor says so.
    run(&root, "file_delete", json!({"file": "coin.toml"}));
    editor.tick(&root, Instant::now() + Duration::from_secs(2));
    assert!(editor.gone);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn every_change_is_rendered_with_its_picture_and_summary() {
    let (dir, root) = project("render");
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().expect("a render");
    assert!(heard.seconds > 0.1, "{}", heard.seconds);
    assert!(!heard.summary.is_empty());
    assert_eq!(
        heard.picture.size,
        [
            duck_echo_analyze::picture::WIDTH as usize,
            duck_echo_analyze::picture::HEIGHT as usize
        ]
    );
    // The render is the file's render, sample for sample.
    let recipe = duck_echo_core::load_str(&file(&dir), duck_echo_core::Format::Toml)
        .unwrap()
        .recipe;
    assert_eq!(
        heard.buffer.samples,
        duck_echo_tools::render_buffer(&recipe).unwrap().samples
    );
    let first = heard.generation;
    editor
        .slide(
            &root,
            "layers[0].envelope.decay",
            json!(0.8),
            Instant::now(),
        )
        .unwrap();
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().unwrap();
    assert!(heard.generation > first);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn export_wav_writes_exactly_what_render_writes() {
    let (dir, root) = project("export");
    let out = dir.join("out");
    let path = editor::export_wav(&root, "coin.toml", &out.join("my coin.wav")).unwrap();
    assert_eq!(path.file_name().unwrap(), "my coin.wav");
    // The command-line tool's render of the same file.
    let loaded = duck_echo_core::load_str(&file(&dir), duck_echo_core::Format::Toml).unwrap();
    let expected = dir.join("expected.wav");
    duck_echo_tools::render_to_file(
        &loaded,
        &expected,
        None,
        duck_echo_audio::WavFormat::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(&expected).unwrap()
    );
    // With the same timing and level facts beside it.
    assert_eq!(
        std::fs::read_to_string(out.join("my coin.wav.json")).unwrap(),
        std::fs::read_to_string(dir.join("expected.wav.json"))
            .unwrap()
            .replace("expected.wav", "my coin.wav")
    );
    // A name without .wav gets it; the exports folder takes a free name.
    let path = editor::export_wav(&root, "coin.toml", &out.join("again")).unwrap();
    assert_eq!(path.file_name().unwrap(), "again.wav");
    let place = editor::export_place(&root, "coin.toml");
    assert!(place.ends_with("exports/coin.wav"), "{}", place.display());
    editor::export_wav(&root, "coin.toml", &place).unwrap();
    assert!(editor::export_place(&root, "coin.toml").ends_with("exports/coin 2.wav"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn typing_never_saves_over_a_change_made_meanwhile() {
    let (dir, root) = project("conflict");
    let before = file(&dir);
    let mut editor = Editor::open(&root, "coin.toml").unwrap();
    let start = Instant::now();
    // The person types...
    editor.text = before.replace("decay = 0.25", "decay = 0.6");
    editor.typed(start);
    // ...and meanwhile an AI assistant changes the volume.
    let command = commands::parse(
        "sound_set",
        &json!({"file": "coin.toml", "path": "layers[0].duty", "value": 0.3}),
        "",
    )
    .unwrap();
    commands::run(&Project::new(&root), command, Who::Agent, None, "").unwrap();
    let theirs = file(&dir);
    // The typing is not saved over it: the person is asked.
    let said = editor.tick(&root, start + TYPING_PAUSE * 2);
    assert!(said.is_some_and(|s| s.contains("Keep mine")));
    assert_eq!(file(&dir), theirs);
    assert!(editor.conflict.is_some());
    assert!(
        editor
            .visit
            .as_ref()
            .is_some_and(|v| v.text.starts_with("AI assistant"))
    );
    // Use theirs: the file as it is, the typing dropped.
    editor.use_theirs(&root);
    assert_eq!(editor.text, theirs);
    // Again, and this time Keep mine: the typing is saved over theirs,
    // and their change is still in the History.
    editor.text = theirs.replace("decay = 0.25", "decay = 0.6");
    editor.typed(start);
    let command = commands::parse(
        "sound_set",
        &json!({"file": "coin.toml", "path": "layers[0].duty", "value": 0.4}),
        "",
    )
    .unwrap();
    commands::run(&Project::new(&root), command, Who::Agent, None, "").unwrap();
    assert!(editor.tick(&root, start + TYPING_PAUSE * 2).is_some());
    editor.keep_mine(&root).unwrap();
    assert!(file(&dir).contains("decay = 0.6"));
    let steps = Project::new(&root).history_list(5).unwrap().steps;
    assert_eq!(steps[0].who, "You");
    assert_eq!(steps[1].who, "AI assistant");
    std::fs::remove_dir_all(&dir).ok();
}
