//! Smooth live changes, the way the sound editor makes them: a slider in
//! the editor, the live changes it sends, and the mixer's output recorded
//! with no sound card.
//!
//! A click is a sudden jump from one sample to the next, so the measure
//! is the biggest step between neighbouring samples (as in
//! `crates/duck-echo-live/tests/clicks.rs`). A low sine moves very little
//! from sample to sample, so any jump stands out.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use duck_echo_live::{Config, Mixer, Player, Sound, live};
use duck_echo_tools::root::Root;
use serde_json::json;
use studio_app::editor::{Arrival, Editor};

const BLOCK: usize = 1024;

/// A project holding `tone.toml`: a low, steady sine at `volume`, long
/// enough that the whole test happens while it holds its level.
fn project(name: &str, volume: f32, normalize: bool) -> (PathBuf, Root) {
    let dir = std::env::temp_dir().join(format!("duck-echo-smooth-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let master = if normalize {
        "[master]\nnormalize = true\n\n"
    } else {
        ""
    };
    std::fs::write(
        dir.join("tone.toml"),
        format!(
            "# A low tone.\nversion = 0\n\n{master}[[layers]]\nsource = \"sine\"\nvolume = {volume}\nenvelope = {{ attack = 0.03, sustain = 1.0, sustain_time = 3.0, release = 0.07 }}\npitch = {{ start = 55 }}\n"
        ),
    )
    .unwrap();
    let root = Root::new(&dir).unwrap();
    (dir, root)
}

fn blocks(mixer: &mut Mixer, count: usize, into: &mut Vec<f32>) {
    let mut out = vec![0.0f32; BLOCK];
    for _ in 0..count {
        mixer.process(&mut out);
        into.extend_from_slice(&out);
    }
}

fn biggest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max)
}

/// Open the tone and play it, repeating, as Loop does.
fn open_and_loop(root: &Root, smooth: bool) -> (Editor, Player, Mixer) {
    let mut editor = Editor::open(root, "tone.toml").unwrap();
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().unwrap();
    let (mut player, mixer) = live(Config {
        sample_rate: 44_100,
        channels: 1,
        fade_seconds: 0.01,
    });
    let mut sound = Sound::from_recipe_mastered(&heard.recipe, &heard.mastering).unwrap();
    if !smooth {
        sound.set_smoothing(0.0);
    }
    player.play(0, sound, true).unwrap();
    editor.live.started(heard);
    (editor, player, mixer)
}

/// Send what the editor's newest text changes to the playing voice, as
/// the editor screen does every frame.
fn send(editor: &mut Editor, player: &mut Player) -> usize {
    let recipe = editor.recipe().cloned().unwrap();
    let updates = editor.live.changed(&recipe);
    for u in &updates {
        player.update(0, *u).unwrap();
    }
    updates.len()
}

/// Drag the volume slider from 0.2 up to 1.0 and back in 30 steps, one
/// step per block of the sound card, while the tone repeats.
fn drag_the_volume(smooth: bool) -> Vec<f32> {
    let (dir, root) = project(if smooth { "smooth" } else { "jumpy" }, 0.2, false);
    let (mut editor, mut player, mut mixer) = open_and_loop(&root, smooth);
    let mut out = Vec::new();
    blocks(&mut mixer, 4, &mut out);
    let start = Instant::now();
    for step in 1..=30u32 {
        let up = if step <= 15 { step } else { 30 - step };
        let volume = 0.2 + 0.8 * f64::from(up) / 15.0;
        let now = start + Duration::from_millis(23 * u64::from(step));
        editor
            .slide(&root, "layers[0].volume", json!(volume), now)
            .unwrap();
        assert_eq!(send(&mut editor, &mut player), 1, "one live change");
        blocks(&mut mixer, 1, &mut out);
    }
    editor
        .release(&root, start + Duration::from_secs(1))
        .unwrap();
    blocks(&mut mixer, 4, &mut out);
    assert_eq!(player.status().refused(), 0);
    std::fs::remove_dir_all(&dir).ok();
    out
}

#[test]
fn dragging_the_volume_in_the_editor_does_not_click() {
    // The tone's own biggest step at full volume.
    let (dir, root) = project("own", 1.0, false);
    let (_editor, _player, mut mixer) = open_and_loop(&root, true);
    let mut untouched = Vec::new();
    blocks(&mut mixer, 40, &mut untouched);
    let own = biggest_step(&untouched);
    std::fs::remove_dir_all(&dir).ok();

    let smooth = drag_the_volume(true);
    let loudest = smooth.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(loudest > 0.95, "the drag was heard: {loudest}");
    let step = biggest_step(&smooth);
    assert!(
        step <= own * 1.02,
        "smoothed: a step of {step}, the sound's own is {own}"
    );
    // The same drag with the 10 ms slide turned off clicks: the test can
    // tell the difference.
    let jumpy = biggest_step(&drag_the_volume(false));
    assert!(
        jumpy > own * 3.0,
        "unsmoothed: a step of only {jumpy}, the sound's own is {own}"
    );
}

#[test]
fn volume_slides_live_and_other_changes_take_over_with_a_new_voice() {
    let (dir, root) = project("kinds", 0.5, true);
    let (mut editor, mut player, mut mixer) = open_and_loop(&root, true);
    let mut out = Vec::new();
    blocks(&mut mixer, 2, &mut out);
    let now = Instant::now();

    // A volume change slides at once; when its render arrives, only the
    // level moves (the sound is normalized, so the file keeps its
    // loudness).
    editor
        .slide(&root, "layers[0].volume", json!(0.9), now)
        .unwrap();
    editor.release(&root, now).unwrap();
    assert_eq!(send(&mut editor, &mut player), 1);
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().unwrap();
    let Arrival::Level(level) = editor.live.rendered(heard) else {
        panic!("a volume change needs no new voice");
    };
    assert_eq!(level, heard.mastering.gain);
    player.set_level(0, level).unwrap();
    blocks(&mut mixer, 2, &mut out);

    // A cutoff needs a filter first: adding one is a new voice.
    editor
        .slide(&root, "layers[0].filter", json!({"cutoff": 400}), now)
        .unwrap();
    editor.release(&root, now).unwrap();
    assert_eq!(send(&mut editor, &mut player), 0);
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().unwrap();
    assert_eq!(editor.live.rendered(heard), Arrival::Replace);
    let sound = Sound::from_recipe_mastered(&heard.recipe, &heard.mastering).unwrap();
    player.replace(0, sound).unwrap();
    blocks(&mut mixer, 2, &mut out);

    // Then the cutoff slides live.
    editor
        .slide(&root, "layers[0].filter.cutoff", json!(900), now)
        .unwrap();
    editor.release(&root, now).unwrap();
    assert_eq!(send(&mut editor, &mut player), 1);

    // The wave from sine to saw: a new voice, with a cross-fade.
    editor
        .slide(&root, "layers[0].source", json!("saw"), now)
        .unwrap();
    editor.release(&root, now).unwrap();
    assert_eq!(send(&mut editor, &mut player), 0);
    assert!(editor.wait_for_render(Duration::from_secs(60)));
    let heard = editor.heard.as_ref().unwrap();
    assert_eq!(editor.live.rendered(heard), Arrival::Replace);
    assert_eq!(player.status().refused(), 0);
    std::fs::remove_dir_all(&dir).ok();
}
