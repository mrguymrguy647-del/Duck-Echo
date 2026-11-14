//! The rule of the audio thread: `Mixer::process` never allocates and
//! never frees memory.
//!
//! How it is proven: this test program replaces Rust's memory allocator
//! with `assert_no_alloc`'s. Inside `assert_no_alloc(|| ...)`, every call
//! to allocate *or free* memory on that thread is counted as a violation.
//! Every block of every test below is mixed inside it, and each test ends
//! by checking that the count is still zero.
//!
//! The first test shows the detector itself works, so a zero means
//! something.

use assert_no_alloc::{assert_no_alloc, reset_violation_count, violation_count, AllocDisabler};
use duck_echo_core::Recipe;
use duck_echo_live::{live, Config, Mixer, ParamUpdate, PlayError, Player, Sound, SLOTS};
use duck_echo_tools::presets::{self, PRESETS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Frames in one block, as the sound card asks for them.
const BLOCK: usize = 1024;

fn preset_recipes() -> Vec<Recipe> {
    PRESETS
        .iter()
        .map(|p| presets::load(p).unwrap().recipe)
        .collect()
}

/// Mix one block with allocation and freeing forbidden.
fn block(mixer: &mut Mixer, out: &mut [f32]) {
    assert_no_alloc(|| mixer.process(out));
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn the_detector_catches_an_allocation_and_a_free() {
    reset_violation_count();
    // Plain arithmetic: nothing to catch.
    let sum = assert_no_alloc(|| (0..1000u64).map(std::hint::black_box).sum::<u64>());
    assert_eq!(sum, 499_500);
    assert_eq!(violation_count(), 0);

    // Making a small list: caught.
    assert_no_alloc(|| {
        let list: Vec<u8> = std::hint::black_box(Vec::with_capacity(64));
        std::mem::forget(list);
    });
    let after_allocating = violation_count();
    assert!(after_allocating >= 1, "an allocation was not noticed");

    // Dropping a voice, which is what the mixer must never do: caught.
    let coin = presets::load(presets::get("coin").unwrap()).unwrap().recipe;
    let voice = duck_echo_synth::prepare(&coin).unwrap();
    assert_no_alloc(|| drop(voice));
    assert!(
        violation_count() > after_allocating,
        "freeing a voice was not noticed"
    );
    reset_violation_count();
}

#[test]
fn a_long_busy_session_never_allocates_or_frees_in_the_audio_block() {
    reset_violation_count();
    let recipes = preset_recipes();
    let (mut player, mut mixer) = live(Config::default());
    let mut out = vec![0.0f32; BLOCK * 2];
    // A rendered song, played as finished samples in the last slot.
    let song_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/songs/four_bars.toml");
    let files = duck_echo_tools::song::files_next_to(&song_path);
    let song =
        duck_echo_core::load_song_path(&song_path, &duck_echo_tools::song::Library::new(&files))
            .unwrap();
    let render_song = || duck_echo_synth::song::render_song(&song.song, &song.arrangement).0;

    let mut collected = 0;
    let mut loudest = 0.0f32;
    // About 60 seconds of sound: 2600 blocks.
    for i in 0..2600usize {
        // The control thread's work between blocks. It may allocate.
        let slot = (i / 7) % (SLOTS - 1);
        if i % 7 == 0 {
            let recipe = &recipes[(i / 7) % recipes.len()];
            let sound = Sound::from_recipe(recipe).unwrap();
            player.play(slot, sound, i % 3 == 0).unwrap();
        }
        if i % 3 == 0 {
            let volume = 0.2 + 0.1 * ((i / 3) % 8) as f32;
            let _ = player.update(slot, ParamUpdate::LayerVolume { layer: 0, volume });
            let _ = player.update(
                slot,
                ParamUpdate::FilterCutoff {
                    layer: 0,
                    cutoff: 300.0 + 100.0 * (i % 40) as f32,
                    cutoff_end: (i % 2 == 0).then_some(2000.0),
                },
            );
            let _ = player.update(
                slot,
                ParamUpdate::MasterGain {
                    gain: 0.5 + 0.05 * (i % 10) as f32,
                },
            );
        }
        if i % 5 == 0 {
            player
                .set_level((i / 5) % SLOTS, 0.3 + 0.1 * (i % 7) as f32)
                .unwrap();
        }
        if i % 11 == 0 {
            // An edit that needs a new voice: cross-fade to it.
            let target = (i / 11) % (SLOTS - 1);
            let recipe = &recipes[(i / 11) % recipes.len()];
            player
                .replace(target, Sound::from_recipe(recipe).unwrap())
                .unwrap();
        }
        if i % 13 == 0 {
            player.stop((i / 13) % SLOTS).unwrap();
        }
        if i % 17 == 0 {
            player.set_looping((i / 17) % SLOTS, i % 2 == 0).unwrap();
        }
        if i % 400 == 10 {
            player
                .play(SLOTS - 1, Sound::from_samples(render_song()), true)
                .unwrap();
        }
        if i % 400 == 150 {
            player
                .replace(SLOTS - 1, Sound::from_samples(render_song()))
                .unwrap();
        }
        // Sometimes the control thread is slow to collect.
        if i % 2 == 0 && !(900..1100).contains(&i) {
            collected += player.collect();
        }

        // The audio thread's work: one block, nothing allocated or freed.
        block(&mut mixer, &mut out);
        loudest = loudest.max(peak(&out));
    }
    assert_eq!(
        violation_count(),
        0,
        "the audio block allocated or freed memory"
    );

    // The session really did things: sound came out, sounds repeated,
    // and hundreds of finished sounds came back to be dropped here.
    assert!(loudest > 0.3, "peak {loudest}");
    assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
    assert_eq!(player.status().frames(), 2600 * BLOCK as u64);
    player.stop_all().unwrap();
    for _ in 0..4 {
        block(&mut mixer, &mut out);
        collected += player.collect();
    }
    assert!(collected > 500, "{collected} sounds came back");
    assert!((0..SLOTS).all(|s| !player.status().is_playing(s)));
    assert_eq!(violation_count(), 0);
}

#[test]
fn when_the_control_thread_stalls_the_audio_thread_neither_waits_nor_drops_a_sound() {
    reset_violation_count();
    let coin = presets::load(presets::get("coin").unwrap()).unwrap().recipe;
    let (mut player, mut mixer) = live(Config::default());
    let mut out = vec![0.0f32; BLOCK * 2];

    // The control thread plays and stops sounds but never collects the
    // finished ones. The ring back to it fills (256), then the mixer's
    // parking area fills, then the mixer stops taking messages, and at
    // last the player is told to wait.
    let mut sent = 0usize;
    let mut busy = 0usize;
    for i in 0..3000usize {
        match player.play(i % SLOTS, Sound::from_recipe(&coin).unwrap(), false) {
            Ok(()) => sent += 1,
            Err(PlayError::Busy) => busy += 1,
            Err(other) => panic!("{other}"),
        }
        block(&mut mixer, &mut out);
    }
    assert_eq!(violation_count(), 0, "the stalled mixer allocated or freed");
    assert!(busy > 1000, "the player was told to wait {busy} times");
    assert!(
        sent > 300,
        "{sent} sounds were sent before the queues filled"
    );
    // The mixer kept making blocks all the while.
    assert_eq!(player.status().frames(), 3000 * BLOCK as u64);

    // The control thread wakes up and collects. Every sound that was sent
    // comes back: none was dropped on the audio thread, none was lost.
    let mut collected = 0;
    for _ in 0..200 {
        collected += player.collect();
        let _ = player.stop_all();
        block(&mut mixer, &mut out);
    }
    collected += player.collect();
    assert_eq!(collected, sent);
    assert_eq!(violation_count(), 0);
}

#[test]
fn two_real_threads_the_audio_thread_never_allocates_or_frees() {
    let recipes = preset_recipes();
    let (mut player, mut mixer) = live(Config::default());
    let done = Arc::new(AtomicBool::new(false));

    // The audio thread: 4000 blocks (about 93 seconds of sound), as fast
    // as it can, each with allocation and freeing forbidden.
    let audio_done = done.clone();
    let audio = std::thread::spawn(move || {
        let mut out = vec![0.0f32; BLOCK * 2];
        let mut loudest = 0.0f32;
        reset_violation_count();
        for i in 0..4000 {
            block(&mut mixer, &mut out);
            loudest = loudest.max(peak(&out));
            if i % 8 == 0 {
                // Give the control thread a turn, as a real sound card's
                // pauses between blocks would.
                std::thread::yield_now();
            }
        }
        let violations = violation_count();
        audio_done.store(true, Ordering::SeqCst);
        // The mixer goes back to the control thread to be dropped there.
        (violations, loudest, mixer)
    });

    // The control thread: plays, changes, replaces, stops and collects
    // until the audio thread is done.
    let mut i = 0usize;
    let mut collected = 0usize;
    while !done.load(Ordering::SeqCst) {
        let slot = i % SLOTS;
        let recipe = &recipes[i % recipes.len()];
        let _ = match i % 4 {
            0 => player.play(slot, Sound::from_recipe(recipe).unwrap(), i % 8 == 0),
            1 => player.update(
                slot,
                ParamUpdate::LayerVolume {
                    layer: 0,
                    volume: 0.1 + 0.1 * (i % 9) as f32,
                },
            ),
            2 => player.replace(slot, Sound::from_recipe(recipe).unwrap()),
            _ => player.stop((i * 7) % SLOTS),
        };
        collected += player.collect();
        i += 1;
    }
    let (violations, loudest, mixer) = audio.join().unwrap();
    assert_eq!(violations, 0, "the audio thread allocated or freed memory");
    assert!(loudest > 0.1, "peak {loudest}");
    assert!(i > 100, "the control thread sent {i} messages");
    collected += player.collect();
    assert!(collected > 20, "{collected} sounds came back");
    drop(mixer);
}

/// A second, independent check that nothing is ever lost between the two
/// threads: every sound sent is either still in the mixer or came back.
#[test]
fn every_sound_sent_comes_back_exactly_once() {
    reset_violation_count();
    let recipes = preset_recipes();
    let (mut player, mut mixer): (Player, Mixer) = live(Config::default());
    let mut out = vec![0.0f32; BLOCK * 2];
    let (mut sent, mut collected) = (0usize, 0usize);
    for i in 0..600usize {
        let recipe = &recipes[i % recipes.len()];
        // Plays, replacements and stops, several in one block, so that
        // requests pile up behind a fade.
        for k in 0..3 {
            let slot = (i + k) % 5;
            let sound = Sound::from_recipe(recipe).unwrap();
            let ok = match (i + k) % 3 {
                0 => player.play(slot, sound, false).is_ok(),
                1 => player.replace(slot, sound).is_ok(),
                _ => {
                    drop(sound);
                    let _ = player.stop(slot);
                    false
                }
            };
            sent += ok as usize;
        }
        block(&mut mixer, &mut out);
        collected += player.collect();
    }
    player.stop_all().unwrap();
    for _ in 0..8 {
        block(&mut mixer, &mut out);
        collected += player.collect();
    }
    assert_eq!(collected, sent);
    assert_eq!(violation_count(), 0);
}
