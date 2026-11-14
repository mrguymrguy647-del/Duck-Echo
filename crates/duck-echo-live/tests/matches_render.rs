//! What plays through the mixer is what `render` writes to a file: the
//! same samples, the same level, the same length. Checked on every preset
//! and every example recipe, played once and repeated.

use duck_echo_core::Recipe;
use duck_echo_live::{live, Config, Mixer, PlayError, Player, Sound};
use duck_echo_synth::render;
use duck_echo_tools::presets::{self, PRESETS};
use std::path::Path;

/// Every preset and every example recipe, with a name.
fn recipes() -> Vec<(String, Recipe)> {
    let mut out: Vec<(String, Recipe)> = PRESETS
        .iter()
        .map(|p| {
            (
                format!("preset:{}", p.name),
                presets::load(p).unwrap().recipe,
            )
        })
        .collect();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/recipes");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let loaded = duck_echo_core::load_path(&path).unwrap();
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            loaded.recipe,
        ));
    }
    assert!(out.len() >= 40, "{} recipes", out.len());
    out
}

fn pair(recipe: &Recipe, channels: usize) -> (Player, Mixer) {
    live(Config {
        sample_rate: recipe.sample_rate,
        channels,
        ..Config::default()
    })
}

/// Run the mixer for `frames` frames in blocks of 1024 (the sound card's
/// usual block), the last one shorter.
fn run(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * mixer.channels()];
    for block in out.chunks_mut(1024 * mixer.channels()) {
        mixer.process(block);
    }
    out
}

#[test]
fn every_preset_and_example_plays_exactly_its_rendered_file() {
    for (name, recipe) in recipes() {
        let file = render(&recipe).unwrap();
        let channels = file.channels as usize;
        let (mut player, mut mixer) = pair(&recipe, channels);
        let sound = Sound::from_recipe(&recipe).unwrap();
        assert_eq!(sound.length() * channels, file.samples.len(), "{name}");
        player.play(0, sound, false).unwrap();
        // The whole sound, and half a second more.
        let extra = recipe.sample_rate as usize / 2;
        let heard = run(&mut mixer, sound_frames(&file) + extra);
        let (sound_part, after) = heard.split_at(file.samples.len());
        assert!(
            sound_part == file.samples.as_slice(),
            "{name}: the speakers differ from the file"
        );
        assert!(
            after.iter().all(|s| *s == 0.0),
            "{name}: sound after the end"
        );
        // It ended, and the mixer handed the voice back to be dropped.
        assert!(!player.status().is_playing(0), "{name}");
        assert_eq!(player.collect(), 1, "{name}");
        assert_eq!(player.status().refused(), 0, "{name}");
    }
}

fn sound_frames(file: &duck_echo_audio::AudioBuffer) -> usize {
    file.samples.len() / file.channels as usize
}

#[test]
fn a_repeated_sound_is_its_rendered_file_again_and_again() {
    // The mixer repeats a sound by rewinding its voice. Three rounds of
    // every preset and example must be the file three times.
    for (name, recipe) in recipes() {
        let file = render(&recipe).unwrap();
        let (mut player, mut mixer) = pair(&recipe, file.channels as usize);
        player
            .play(3, Sound::from_recipe(&recipe).unwrap(), true)
            .unwrap();
        let heard = run(&mut mixer, sound_frames(&file) * 3);
        for (round, part) in heard.chunks(file.samples.len()).enumerate() {
            assert!(
                part == file.samples.as_slice(),
                "{name}: round {round} differs from the file"
            );
        }
        assert_eq!(player.status().loops(3), 2, "{name}");
        assert!(player.status().is_playing(3), "{name}");
        // Stopping fades it out and hands the voice back.
        player.stop(3).unwrap();
        run(&mut mixer, 2048);
        assert!(!player.status().is_playing(3), "{name}");
        assert_eq!(player.collect(), 1, "{name}");
    }
}

#[test]
fn a_mono_sound_plays_from_both_speakers_and_stereo_folds_to_one() {
    let coin = presets::load(presets::get("coin").unwrap()).unwrap().recipe;
    let file = render(&coin).unwrap();
    assert_eq!(file.channels, 1);
    let (mut player, mut mixer) = pair(&coin, 2);
    player
        .play(0, Sound::from_recipe(&coin).unwrap(), false)
        .unwrap();
    let heard = run(&mut mixer, file.samples.len());
    for (frame, mono) in heard.chunks_exact(2).zip(&file.samples) {
        assert!(frame[0] == *mono && frame[1] == *mono);
    }
    // A stereo sound on a one-channel mixer: the mean of its two sides.
    let mut wide = coin.clone();
    wide.channels = 2;
    wide.layers[0].pan = -1.0;
    let file = render(&wide).unwrap();
    let (mut player, mut mixer) = pair(&wide, 1);
    player
        .play(0, Sound::from_recipe(&wide).unwrap(), false)
        .unwrap();
    let heard = run(&mut mixer, file.samples.len() / 2);
    for (mono, frame) in heard.iter().zip(file.samples.chunks_exact(2)) {
        assert!(*mono == (frame[0] + frame[1]) * 0.5);
    }
}

#[test]
fn many_sounds_at_once_are_added_and_then_clipped() {
    // One loud square wave alone, then four at once.
    let src = "version = 0\n[[layers]]\nsource = \"square\"\nvolume = 0.6\nenvelope = { attack = 0.0, sustain = 1.0, sustain_time = 0.2, release = 0.01 }\npitch = { start = 110 }\n";
    let recipe = duck_echo_core::load_str(src, duck_echo_core::Format::Toml)
        .unwrap()
        .recipe;
    let alone = render(&recipe).unwrap().samples;
    let (mut player, mut mixer) = pair(&recipe, 1);
    for slot in 0..4 {
        player
            .play(slot, Sound::from_recipe(&recipe).unwrap(), false)
            .unwrap();
    }
    let heard = run(&mut mixer, alone.len());
    assert!(heard.iter().all(|s| (-1.0..=1.0).contains(s)));
    let mut clipped = 0;
    for (h, a) in heard.iter().zip(&alone) {
        let sum = a + a + a + a;
        assert!(
            (h - sum.clamp(-1.0, 1.0)).abs() < 1e-6,
            "{h} for a sum of {sum}"
        );
        clipped += (sum.abs() > 1.0) as u64;
    }
    assert!(clipped > 1000);
    assert_eq!(player.status().clipped_samples(), clipped);
    run(&mut mixer, 1024);
    assert_eq!(player.collect(), 4);
}

#[test]
fn a_sound_at_another_sample_rate_and_a_slot_that_is_not_there_are_refused() {
    let mut coin = presets::load(presets::get("coin").unwrap()).unwrap().recipe;
    let (mut player, mut mixer) = pair(&coin, 2);
    assert_eq!(
        player.play(32, Sound::from_recipe(&coin).unwrap(), false),
        Err(PlayError::NoSuchSlot { slot: 32 })
    );
    assert_eq!(player.stop(99), Err(PlayError::NoSuchSlot { slot: 99 }));
    coin.sample_rate = 22_050;
    let error = player
        .play(0, Sound::from_recipe(&coin).unwrap(), false)
        .unwrap_err();
    assert_eq!(
        error,
        PlayError::SampleRate {
            sound: 22_050,
            speakers: 44_100
        }
    );
    assert!(error.to_string().contains("22050 Hz"), "{error}");
    // Nothing was sent, so nothing plays.
    assert!(run(&mut mixer, 2048).iter().all(|s| *s == 0.0));
    assert_eq!(player.status().frames(), 2048);
}
