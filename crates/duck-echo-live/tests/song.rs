//! The background song player: a song plays from a render made on a
//! worker thread, and after an edit a new render takes over at the play
//! head.

use duck_echo_core::{load_song_str, Format, LoadedSong};
use duck_echo_live::{live, Config, Mixer, PlayError, Player, SongPlayback, SongState};
use duck_echo_synth::song::render_song;
use duck_echo_tools::song::{files_next_to, Library};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BLOCK: usize = 1024;

fn songs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs")
}

/// A song from the examples, after `edit` has changed its text.
fn song(name: &str, edit: impl Fn(String) -> String) -> LoadedSong {
    let path = songs().join(name);
    let text = edit(std::fs::read_to_string(&path).unwrap());
    let files = files_next_to(&path);
    load_song_str(&text, Format::Toml, &Library::new(&files)).unwrap()
}

fn pair(song: &LoadedSong) -> (Player, Mixer) {
    live(Config {
        sample_rate: song.song.sample_rate,
        channels: song.song.channels as usize,
        ..Config::default()
    })
}

fn blocks(mixer: &mut Mixer, count: usize, into: &mut Vec<f32>) {
    let mut out = vec![0.0f32; BLOCK * mixer.channels()];
    for _ in 0..count {
        mixer.process(&mut out);
        into.extend_from_slice(&out);
    }
}

/// Poll until `done` says so. The render runs on another thread, so this
/// waits for it (two minutes at most).
fn wait(playback: &mut SongPlayback, player: &mut Player, done: impl Fn(&SongPlayback) -> bool) {
    let started = Instant::now();
    loop {
        playback.poll(player).unwrap();
        if done(playback) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "the render never came"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn biggest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max)
}

#[test]
fn a_song_plays_its_render_and_stops_at_the_end() {
    let loaded = song("four_bars.toml", |t| t);
    let file = render_song(&loaded.song, &loaded.arrangement).0;
    let (mut player, mut mixer) = pair(&loaded);
    let mut playback = SongPlayback::new(5);
    assert_eq!(playback.state(), SongState::Stopped);

    playback.play(loaded.song.clone(), loaded.arrangement.clone());
    // Until the render is ready, it says so, and nothing sounds.
    assert_eq!(playback.state(), SongState::GettingReady);
    assert!(playback.is_rendering());
    let mut out = Vec::new();
    blocks(&mut mixer, 2, &mut out);
    assert!(out.iter().all(|s| *s == 0.0));

    wait(&mut playback, &mut player, |p| {
        p.state() == SongState::Playing
    });
    assert!(!playback.is_rendering());
    let mut out = Vec::new();
    let needed = file.samples.len() / mixer.channels() / BLOCK + 2;
    blocks(&mut mixer, needed, &mut out);
    // The speakers got the song's file, sample for sample, then silence.
    assert!(out[..file.samples.len()] == file.samples[..]);
    assert!(out[file.samples.len()..].iter().all(|s| *s == 0.0));
    // It played to its end.
    assert_eq!(playback.poll(&mut player), Ok(SongState::Stopped));
    assert_eq!(player.collect(), 1);
}

#[test]
fn a_looping_song_repeats_with_no_gap() {
    let loaded = song("duck_craft_day.toml", |t| t);
    assert!(loaded.song.loop_);
    let file = render_song(&loaded.song, &loaded.arrangement).0;
    let (mut player, mut mixer) = pair(&loaded);
    let mut playback = SongPlayback::new(0);
    playback.play(loaded.song.clone(), loaded.arrangement.clone());
    wait(&mut playback, &mut player, |p| {
        p.state() == SongState::Playing
    });
    let mut out = Vec::new();
    let frames = file.samples.len() / mixer.channels();
    blocks(&mut mixer, frames * 2 / BLOCK + 1, &mut out);
    // Twice round is the file twice, with nothing in between.
    assert!(out[..file.samples.len()] == file.samples[..]);
    assert!(out[file.samples.len()..file.samples.len() * 2] == file.samples[..]);
    assert_eq!(playback.poll(&mut player), Ok(SongState::Playing));
    playback.stop(&mut player).unwrap();
    blocks(&mut mixer, 2, &mut out);
    assert!(!player.status().is_playing(0));
    assert_eq!(playback.state(), SongState::Stopped);
}

#[test]
fn an_edit_is_rendered_again_and_takes_over_at_the_play_head() {
    let before = song("duck_craft_day.toml", |t| t);
    // The edit: the keys are played by another instrument.
    let after = song("duck_craft_day.toml", |t| {
        let edited = t.replacen("instrument = \"piano\"", "instrument = \"chip_lead\"", 1);
        assert_ne!(edited, t);
        edited
    });
    let old = render_song(&before.song, &before.arrangement).0.samples;
    let new = render_song(&after.song, &after.arrangement).0.samples;
    assert_eq!(old.len(), new.len());
    assert!(old != new, "the edit changed nothing");

    let (mut player, mut mixer) = pair(&before);
    let channels = mixer.channels();
    let mut playback = SongPlayback::new(0);
    playback.play(before.song.clone(), before.arrangement.clone());
    wait(&mut playback, &mut player, |p| {
        p.state() == SongState::Playing
    });
    let mut out = Vec::new();
    blocks(&mut mixer, 100, &mut out);

    // The song is edited while it plays.
    playback.changed(after.song.clone(), after.arrangement.clone());
    assert!(playback.is_rendering());
    assert_eq!(playback.state(), SongState::Playing);
    wait(&mut playback, &mut player, |p| !p.is_rendering());
    blocks(&mut mixer, 100, &mut out);

    let swap = 100 * BLOCK * channels;
    let settled = swap + 441 * channels;
    // Before the edit: the old render. From 10 ms after the swap: the new
    // render, at the very same place in the song.
    assert!(out[..swap] == old[..swap]);
    assert!(out[settled..] == new[settled..out.len()]);
    // The cross-fade in between has no click: no step bigger than the
    // two renders' own biggest, plus the fade's own slope (2 / 441).
    let own = biggest_step(&old).max(biggest_step(&new));
    let step = biggest_step(&out[swap - 2 * channels..settled + 2 * channels]);
    assert!(
        step <= own + 2.0 / 441.0,
        "a step of {step}; the song's own is {own}"
    );
    // The old render came back to be dropped.
    assert_eq!(player.collect(), 1);
}

#[test]
fn stop_wins_over_a_render_that_is_still_on_its_way() {
    let loaded = song("duck_craft_night.toml", |t| t);
    let (mut player, mut mixer) = pair(&loaded);
    let mut playback = SongPlayback::new(0);
    playback.play(loaded.song.clone(), loaded.arrangement.clone());
    // Stop before the render can be ready, then edit: a stopped song
    // stays stopped.
    playback.stop(&mut player).unwrap();
    playback.changed(loaded.song.clone(), loaded.arrangement.clone());
    assert!(!playback.is_rendering());
    let mut out = Vec::new();
    let started = Instant::now();
    // Long enough for the unwanted render to finish and be thrown away.
    while started.elapsed() < Duration::from_secs(4) {
        assert_eq!(playback.poll(&mut player), Ok(SongState::Stopped));
        blocks(&mut mixer, 1, &mut out);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(out.iter().all(|s| *s == 0.0));
    assert!(!player.status().is_playing(0));
}

#[test]
fn a_song_at_another_sample_rate_says_so() {
    let loaded = song("four_bars.toml", |t| {
        t.replace("bars = 4", "bars = 4\nsample_rate = 22050")
    });
    assert_eq!(loaded.song.sample_rate, 22_050);
    let (mut player, _mixer) = live(Config::default());
    let mut playback = SongPlayback::new(0);
    playback.play(loaded.song.clone(), loaded.arrangement.clone());
    let started = Instant::now();
    let error = loop {
        match playback.poll(&mut player) {
            Ok(_) => {
                assert!(started.elapsed() < Duration::from_secs(120));
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => break e,
        }
    };
    assert_eq!(
        error,
        PlayError::SampleRate {
            sound: 22_050,
            speakers: 44_100
        }
    );
    assert_eq!(playback.state(), SongState::Stopped);
}
