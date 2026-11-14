//! The click test (the roadmap's "recorded loopback test").
//!
//! A click is a sudden jump from one sample to the next. So the measure
//! is the biggest step between two neighbouring samples of what the mixer
//! put out, compared with the biggest step the sound makes by itself (the
//! same measure as a loop's `loop_seam`).
//!
//! Every test here runs the same scene twice: with the 10 ms smoothing,
//! where the output must have no step bigger than the sound's own; and
//! with the smoothing turned off, where the same scene must click. The
//! second run proves the test can tell the difference.

use duck_echo_core::{load_str, Format, Recipe};
use duck_echo_live::{live, Config, Mixer, ParamUpdate, Player, Sound};

const BLOCK: usize = 1024;

/// After this many blocks the tone is at full level and near the top of a
/// wave: the worst moment to cut it.
const LOUD: usize = 8;

/// A low, steady tone that swells in and dies away, 0.7 seconds long.
/// Low on purpose: a 55 Hz sine moves very little from sample to sample
/// (at most 0.0078 at full volume), so any jump stands out.
fn tone(volume: f32) -> Recipe {
    let src = format!(
        "version = 0\n[[layers]]\nsource = \"sine\"\nvolume = {volume}\nenvelope = {{ attack = 0.03, sustain = 1.0, sustain_time = 0.4, release = 0.07 }}\npitch = {{ start = 55 }}\n"
    );
    load_str(&src, Format::Toml).unwrap().recipe
}

fn pair(fade_seconds: f32) -> (Player, Mixer) {
    live(Config {
        sample_rate: 44_100,
        channels: 1,
        fade_seconds,
    })
}

fn blocks(mixer: &mut Mixer, count: usize, into: &mut Vec<f32>) {
    let mut out = vec![0.0f32; BLOCK];
    for _ in 0..count {
        mixer.process(&mut out);
        into.extend_from_slice(&out);
    }
}

/// The biggest step between two neighbouring samples.
fn biggest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max)
}

/// The tone at full volume, repeating, left alone for `count` blocks:
/// what the speakers get when nobody touches anything.
fn untouched(count: usize) -> Vec<f32> {
    let (mut player, mut mixer) = pair(0.01);
    player
        .play(0, Sound::from_recipe(&tone(1.0)).unwrap(), true)
        .unwrap();
    let mut out = Vec::new();
    blocks(&mut mixer, count, &mut out);
    out
}

/// The sound's own biggest step: the tone at full volume, repeating.
fn own_step() -> f32 {
    let step = biggest_step(&untouched(60));
    // 2 pi 55 / 44100 = 0.00784: the steepest part of a 55 Hz sine.
    assert!((step - 0.00784).abs() < 0.0001, "{step}");
    step
}

/// Drag the tone's volume from 0.2 up to 1.0 and back in 30 steps, one
/// step per block, while it repeats.
fn drag_the_volume(smooth: bool) -> Vec<f32> {
    let (mut player, mut mixer) = pair(0.01);
    let mut sound = Sound::from_recipe(&tone(0.2)).unwrap();
    if !smooth {
        sound.set_smoothing(0.0);
    }
    player.play(0, sound, true).unwrap();
    let mut out = Vec::new();
    blocks(&mut mixer, 4, &mut out);
    for step in 1..=30 {
        let up = if step <= 15 { step } else { 30 - step };
        let volume = 0.2 + 0.8 * up as f32 / 15.0;
        player
            .update(0, ParamUpdate::LayerVolume { layer: 0, volume })
            .unwrap();
        blocks(&mut mixer, 1, &mut out);
    }
    blocks(&mut mixer, 4, &mut out);
    assert_eq!(player.status().refused(), 0);
    out
}

#[test]
fn dragging_the_volume_does_not_click_and_without_smoothing_it_does() {
    let own = own_step();
    let smooth = drag_the_volume(true);
    let jumpy = drag_the_volume(false);
    // The drag was heard: the level went up to full and came back.
    let loudest = smooth.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(loudest > 0.95, "{loudest}");
    // With smoothing: no step bigger than the sound's own.
    let step = biggest_step(&smooth);
    assert!(
        step <= own * 1.02,
        "smoothed: a step of {step}, the sound's own is {own}"
    );
    // Without it, the very same drag clicks: a step several times bigger.
    let step = biggest_step(&jumpy);
    assert!(
        step > own * 3.0,
        "unsmoothed: a step of only {step}, the sound's own is {own}"
    );
}

/// Slide the slot's level (the gain a normalized recipe plays at) from
/// 0.2 to 1.0 and back in 30 steps.
fn drag_the_level(fade_seconds: f32) -> Vec<f32> {
    let (mut player, mut mixer) = pair(fade_seconds);
    player
        .play(0, Sound::from_recipe(&tone(1.0)).unwrap(), true)
        .unwrap();
    player.set_level(0, 0.2).unwrap();
    let mut out = Vec::new();
    blocks(&mut mixer, 4, &mut out);
    for step in 1..=30 {
        let up = if step <= 15 { step } else { 30 - step };
        player.set_level(0, 0.2 + 0.8 * up as f32 / 15.0).unwrap();
        blocks(&mut mixer, 1, &mut out);
    }
    blocks(&mut mixer, 4, &mut out);
    out
}

#[test]
fn sliding_the_level_does_not_click_and_without_smoothing_it_does() {
    let own = own_step();
    let step = biggest_step(&drag_the_level(0.01));
    assert!(step <= own * 1.02, "smoothed: {step}, own {own}");
    let step = biggest_step(&drag_the_level(0.0));
    assert!(step > own * 3.0, "unsmoothed: only {step}, own {own}");
}

/// A whole fade (to silence, or between two sounds) changes the level far
/// more than one drag step does, so it adds a little slope of its own: at
/// most 1 / 441 per sample for a 10 ms fade. On this very low tone, in the
/// worst case, that makes a step 4% bigger than the sound's own biggest
/// (the square root of 0.00784 squared plus 0.00227 squared). A cut with
/// no fade is about a hundred times bigger.
const FADE_ALLOWANCE: f32 = 1.05;

/// Play the tone, and stop it in the middle of its loudest part.
fn stop_while_loud(fade_seconds: f32) -> Vec<f32> {
    let (mut player, mut mixer) = pair(fade_seconds);
    player
        .play(0, Sound::from_recipe(&tone(1.0)).unwrap(), true)
        .unwrap();
    let mut out = Vec::new();
    // 8 blocks is 8192 frames: 0.186 s in, at full level, near the top
    // of a wave (a 55 Hz wave is 801.8 frames long).
    blocks(&mut mixer, LOUD, &mut out);
    assert!(out.last().unwrap().abs() > 0.5, "{:?}", out.last());
    player.stop(0).unwrap();
    blocks(&mut mixer, 4, &mut out);
    // It did stop, and the voice came back.
    assert!(out[out.len() - BLOCK..].iter().all(|s| *s == 0.0));
    assert_eq!(player.collect(), 1);
    out
}

#[test]
fn stopping_a_sound_fades_it_out_and_without_the_fade_it_clicks() {
    let own = own_step();
    let step = biggest_step(&stop_while_loud(0.01));
    assert!(step <= own * FADE_ALLOWANCE, "faded: {step}, own {own}");
    let step = biggest_step(&stop_while_loud(0.0));
    assert!(step > own * 20.0, "cut: only {step}, own {own}");
}

/// Play the quiet tone, then swap in the loud one (a new voice, as after
/// an edit that a slider cannot express) in the middle of a loud part.
fn swap_quiet_for_loud(fade_seconds: f32) -> Vec<f32> {
    let (mut player, mut mixer) = pair(fade_seconds);
    player
        .play(0, Sound::from_recipe(&tone(0.2)).unwrap(), true)
        .unwrap();
    let mut out = Vec::new();
    blocks(&mut mixer, LOUD, &mut out);
    assert!(out.last().unwrap().abs() > 0.15, "{:?}", out.last());
    player
        .replace(0, Sound::from_recipe(&tone(1.0)).unwrap())
        .unwrap();
    blocks(&mut mixer, 49 - LOUD, &mut out);
    // The quiet voice came back to be dropped.
    assert_eq!(player.collect(), 1);
    assert!(player.status().is_playing(0));
    out
}

#[test]
fn a_new_voice_takes_over_at_the_same_place_with_a_cross_fade() {
    let own = own_step();
    let faded = swap_quiet_for_loud(0.01);
    let step = biggest_step(&faded);
    assert!(
        step <= own * FADE_ALLOWANCE,
        "cross-faded: {step}, own {own}"
    );
    let cut = swap_quiet_for_loud(0.0);
    let step = biggest_step(&cut);
    assert!(
        step > own * 20.0,
        "swapped with no fade: only {step}, own {own}"
    );

    // "At the same place": 10 ms after the swap, what plays is exactly
    // the loud tone as if it had been playing all along, sample for
    // sample, through three more rounds of the loop.
    let all_along = untouched(49);
    let swap = LOUD * BLOCK;
    assert!(faded[..swap]
        .iter()
        .zip(&all_along)
        .all(|(a, b)| (a - b * 0.2).abs() < 1e-6));
    assert!(faded[swap + 441..] == all_along[swap + 441..]);
    // During the fade the level rises steadily between the two.
    assert!(faded[swap + 200].abs() > all_along[swap + 200].abs() * 0.4);
    assert!(faded[swap + 200].abs() < all_along[swap + 200].abs() * 0.8);
}

#[test]
fn two_swaps_in_a_row_wait_their_turn_and_still_line_up() {
    // Two replacements arrive in the same block. The second must wait for
    // the first one's fade, then catch up with the frames that played
    // meanwhile (also across the loop's end), or it would be out of step.
    let all_along = untouched(80);
    for start_block in [8usize, 20, 21, 22] {
        let (mut player, mut mixer) = pair(0.01);
        player
            .play(0, Sound::from_recipe(&tone(0.2)).unwrap(), true)
            .unwrap();
        let mut out = Vec::new();
        blocks(&mut mixer, start_block, &mut out);
        player
            .replace(0, Sound::from_recipe(&tone(0.5)).unwrap())
            .unwrap();
        player
            .replace(0, Sound::from_recipe(&tone(1.0)).unwrap())
            .unwrap();
        blocks(&mut mixer, 80 - start_block, &mut out);
        // No click anywhere, and after both fades it is the loud tone at
        // exactly the place it would be.
        let step = biggest_step(&out);
        assert!(
            step <= own_step() * FADE_ALLOWANCE,
            "from block {start_block}: {step}"
        );
        let settled = (start_block + 3) * BLOCK;
        assert!(
            out[settled..] == all_along[settled..],
            "from block {start_block}: out of step after two swaps"
        );
        assert_eq!(player.collect(), 2, "from block {start_block}");
    }
}

#[test]
fn starting_a_sound_over_another_fades_the_old_one_out() {
    let own = own_step();
    for (fade_seconds, clicks) in [(0.01, false), (0.0, true)] {
        let (mut player, mut mixer) = pair(fade_seconds);
        player
            .play(0, Sound::from_recipe(&tone(1.0)).unwrap(), false)
            .unwrap();
        let mut out = Vec::new();
        blocks(&mut mixer, LOUD, &mut out);
        // Press Play again while it is loud: it starts over.
        player
            .play(0, Sound::from_recipe(&tone(1.0)).unwrap(), false)
            .unwrap();
        // The second one plays to its end (0.7 s is 31 blocks).
        blocks(&mut mixer, 34, &mut out);
        let step = biggest_step(&out);
        if clicks {
            assert!(step > own * 20.0, "cut: only {step}");
        } else {
            assert!(step <= own * FADE_ALLOWANCE, "faded: {step}, own {own}");
        }
        assert_eq!(player.collect(), 2);
    }
}

#[test]
fn replacing_a_sound_that_is_not_playing_starts_nothing() {
    let (mut player, mut mixer) = pair(0.01);
    player
        .replace(0, Sound::from_recipe(&tone(1.0)).unwrap())
        .unwrap();
    let mut out = Vec::new();
    blocks(&mut mixer, 3, &mut out);
    assert!(out.iter().all(|s| *s == 0.0));
    assert!(!player.status().is_playing(0));
    // The unheard voice still comes back to be dropped.
    assert_eq!(player.collect(), 1);
}
