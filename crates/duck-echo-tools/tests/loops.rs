//! Roadmap task 20: a looping song is exactly as long as its bars, and
//! its end joins its start with no click.

use duck_echo_audio::AudioBuffer;
use duck_echo_core::{load_song_str, Code, Format, LoadedSong};
use duck_echo_synth::song::{render_song, render_song_tracks};
use duck_echo_tools::measure::{loop_seam, measure_song, Options};
use duck_echo_tools::song::{files_next_to, Library};
use duck_echo_tools::SongFacts;
use std::path::Path;

fn try_load(src: &str) -> Result<LoadedSong, duck_echo_core::Report> {
    let files = files_next_to(Path::new("song.toml"));
    load_song_str(src, Format::Toml, &Library::new(&files))
}

fn load(src: &str) -> LoadedSong {
    try_load(src).unwrap_or_else(|r| panic!("{r}"))
}

fn render(song: &LoadedSong) -> AudioBuffer {
    render_song(&song.song, &song.arrangement).0
}

/// Two bars at 120 BPM (4 seconds), a sine instrument with a long reverb
/// and a long release, and nothing written on the first beat.
fn ringing(extra_top: &str, notes: &str) -> String {
    format!(
        r#"
version = 0
tempo = 120
bars = 2
{extra_top}
[instruments.bell]
version = 0
effects = [{{ type = "reverb", size = 0.8, decay = 2.5, mix = 0.5 }}]
[[instruments.bell.layers]]
source = "sine"
volume = 0.4
envelope = {{ attack = 0.01, decay = 0.1, sustain = 0.7, release = 0.6 }}
pitch = {{ start = "A4" }}

[[tracks]]
name = "bells"
instrument = "bell"
notes = [{notes}]
"#
    )
}

const NOTES: &str = r#"{ beat = 1, note = "A4" }, { beat = 3, note = "E5" }, { bar = 2, beat = 1, note = "C5" }, { bar = 2, beat = 3, note = "A5", length = 0.9 }"#;

#[test]
fn a_loop_is_exactly_as_long_as_its_bars() {
    for (tempo, bars, frames) in [
        (120.0, 2, 176_400usize),
        (120.0, 4, 352_800),
        // 16 beats at 110 BPM are 384872.7 samples: rounded to the nearest.
        (110.0, 4, 384_873),
        (84.0, 8, 1_008_000),
        (97.0, 3, 327_340),
    ] {
        let text = ringing("loop = true", NOTES)
            .replace("tempo = 120", &format!("tempo = {tempo}"))
            .replace("bars = 2", &format!("bars = {bars}"));
        let song = load(&text);
        let buf = render(&song);
        assert_eq!(buf.samples.len(), frames * 2, "{tempo} BPM, {bars} bars");
        assert_eq!(SongFacts::of(&song).song_frames, frames);
    }
    // Without `loop` the same song is longer: the reverb rings on.
    let open = render(&load(&ringing("", NOTES)));
    assert!(open.samples.len() > 176_400 * 2 + 44_100);
}

#[test]
fn the_tail_is_folded_onto_the_start() {
    let looped = render(&load(&ringing("loop = true", NOTES)));
    let open = render(&load(&ringing("", NOTES)));
    let len = looped.samples.len();
    // The loop is the open render's first two bars plus its tail.
    for i in (0..len).step_by(997) {
        let tail = open.samples.get(len + i).copied().unwrap_or(0.0);
        assert!(
            (looped.samples[i] - (open.samples[i] + tail)).abs() < 1e-6,
            "sample {i}"
        );
    }
    // Nothing is written on beat 0, yet the loop's first half second is
    // not silent: the end of the "last time round" rings there.
    let first = &looped.samples[..22_050];
    assert!(first.iter().any(|s| s.abs() > 0.01));
    assert!(open.samples[..22_050].iter().all(|s| s.abs() < 1e-6));
}

#[test]
fn the_join_has_no_jump() {
    let song = load(&ringing("loop = true", NOTES));
    let looped = render(&song);
    let seam = loop_seam(&looped);
    assert!(seam.clean, "{seam:?}");
    assert!(seam.seam_step <= seam.largest_step);

    // Cutting the open render at the bar line instead, with no fold, does
    // jump: the reverb is cut off mid-ring.
    let open = render(&load(&ringing("", NOTES)));
    let cut = AudioBuffer {
        sample_rate: open.sample_rate,
        channels: open.channels,
        samples: open.samples[..looped.samples.len()].to_vec(),
    };
    let hard = loop_seam(&cut);
    assert!(
        hard.seam_step > seam.seam_step * 5.0,
        "{hard:?} vs {seam:?}"
    );

    // A note held across the join: a hard cut would click loudly, the
    // loop does not.
    let held = r#"{ beat = 1, note = "A4" }, { bar = 2, beat = 2.5, note = "E4", length = 3 }"#;
    let seam = loop_seam(&render(&load(&ringing("loop = true", held))));
    assert!(seam.clean, "{seam:?}");
}

#[test]
fn played_twice_there_is_no_onset_at_the_join() {
    let looped = render(&load(&ringing("loop = true", NOTES)));
    let mut twice = looped.samples.clone();
    twice.extend_from_slice(&looped.samples);
    let a = duck_echo_analyze::analyze(&twice, looped.sample_rate, looped.channels);
    // The file starts mid-ring, so its very first sample is an onset.
    // That is the start of the file, not the join. The join is at 4 s,
    // and nothing may be found there.
    for onset in &a.onsets {
        let from_join = (onset.time_seconds - 4.0).abs();
        assert!(from_join > 0.3, "an onset at the join: {onset:?}");
    }
    // The second turn is heard exactly like the first: the same notes at
    // the same places, 4 seconds later.
    let first: Vec<f64> = a
        .onsets
        .iter()
        .map(|o| o.time_seconds)
        .filter(|t| *t > 0.1 && *t < 4.0)
        .collect();
    let second: Vec<f64> = a
        .onsets
        .iter()
        .map(|o| o.time_seconds - 4.0)
        .filter(|t| *t > 0.0)
        .collect();
    assert!(first.len() >= 3, "{:?}", a.onsets);
    assert_eq!(first.len(), second.len(), "{:?}", a.onsets);
    for (x, y) in first.iter().zip(&second) {
        assert!((x - y).abs() < 0.01, "{x} vs {y}");
    }
}

/// The strict proof: a loop is one turn of a song that has been playing
/// forever. So it must equal the second pass of the same song written out
/// twice and rendered without looping.
#[test]
fn a_loop_equals_the_second_pass_of_the_song_played_twice() {
    let looped = render(&load(&ringing("loop = true", NOTES)));
    let twice_notes = format!(
        "{NOTES}, {}",
        NOTES
            .replace("bar = 2", "bar = 4")
            .replace("{ beat = 1,", "{ bar = 3, beat = 1,")
            .replace("{ beat = 3,", "{ bar = 3, beat = 3,")
    );
    let twice = render(&load(
        &ringing("", &twice_notes).replace("bars = 2", "bars = 4"),
    ));
    let len = looped.samples.len();
    let second = &twice.samples[len..2 * len];
    let worst = looped
        .samples
        .iter()
        .zip(second)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    // The reverb's tail from the first pass has faded to under -60 dB by
    // the time the second pass ends, and the rest is float rounding.
    assert!(worst < 2e-3, "loop differs from the second pass by {worst}");
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    let diff: Vec<f32> = looped
        .samples
        .iter()
        .zip(second)
        .map(|(a, b)| a - b)
        .collect();
    assert!(rms(&diff) < rms(&looped.samples) * 0.01);
}

#[test]
fn a_tail_longer_than_the_loop_wraps_round_more_than_once() {
    // One bar at 240 BPM is one second; the reverb rings for several.
    let text = ringing("loop = true", r#"{ beat = 1, note = "A4" }"#)
        .replace("tempo = 120", "tempo = 240")
        .replace("bars = 2", "bars = 1");
    let song = load(&text);
    let buf = render(&song);
    assert_eq!(buf.samples.len(), 44_100 * 2);
    assert!(loop_seam(&buf).clean);
    let open = render(&load(&text.replace("loop = true", "")));
    assert!(
        open.samples.len() > buf.samples.len() * 2,
        "the tail is longer than a turn"
    );
    // Every sample of the open render is in the loop somewhere.
    let sum = |x: &[f32]| x.iter().map(|s| *s as f64).sum::<f64>();
    assert!((sum(&open.samples) - sum(&buf.samples)).abs() < 1e-2);
}

#[test]
fn stems_of_a_loop_are_loops_and_add_up() {
    let text = format!(
        "{}\n[[tracks]]\nname = \"low\"\ninstrument = \"bell\"\nvolume = 0.7\nnotes = [{{ beat = 0, note = \"A2\", length = 7.5 }}]\n",
        ringing("loop = true\nmaster = { normalize_lufs = -18 }", NOTES)
    );
    let song = load(&text);
    let (full, mastering) = render_song(&song.song, &song.arrangement);
    let mut sum = vec![0.0f32; full.samples.len()];
    for track in 0..2 {
        let stem = render_song_tracks(&song.song, &song.arrangement, &[track], &mastering);
        assert_eq!(stem.samples.len(), full.samples.len());
        assert!(loop_seam(&stem).clean);
        for (s, x) in sum.iter_mut().zip(&stem.samples) {
            *s += x;
        }
    }
    let worst = sum
        .iter()
        .zip(&full.samples)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(worst < 1e-5, "{worst}");
    // The loop is levelled as a whole.
    let a = duck_echo_analyze::analyze(&full.samples, full.sample_rate, full.channels);
    assert!((a.loudness_lufs.unwrap() + 18.0).abs() < 0.1);
}

#[test]
fn analyze_reports_the_join_and_validation_guards_the_length() {
    let song = load(&ringing("loop = true", NOTES));
    let facts = SongFacts::of(&song);
    assert!(facts.loop_);
    let extras = Options::default()
        .for_song(&["bells".to_string()], "")
        .unwrap();
    let m = measure_song(&song.song, &song.arrangement, &facts, &[], &extras)
        .unwrap()
        .measured;
    let seam = m.loop_seam.unwrap();
    assert!(seam.clean);
    assert!(m
        .analysis
        .summary
        .starts_with("2 bars looping at 120 BPM, 1 track; "));
    assert!(
        m.analysis.summary.ends_with("loop joins cleanly"),
        "{}",
        m.analysis.summary
    );
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["song"]["loop"], true);
    assert_eq!(json["loop_seam"]["clean"], true);

    // A song that does not loop reports no join.
    let open = load(&ringing("", NOTES));
    let m = measure_song(
        &open.song,
        &open.arrangement,
        &SongFacts::of(&open),
        &[],
        &extras,
    )
    .unwrap()
    .measured;
    assert!(m.loop_seam.is_none());

    // Trimming would change the length, so it is refused with a loop.
    let report = try_load(&ringing(
        "loop = true\nmaster = { trim_silence = true }",
        NOTES,
    ))
    .unwrap_err();
    assert_eq!(report.errors.len(), 1, "{report}");
    assert_eq!(report.errors[0].path, "master.trim_silence");
    assert_eq!(report.errors[0].code, Code::Invalid);

    // Without `bars`, a loop is as long as the bars its notes fill.
    let auto = load(&ringing("loop = true", NOTES).replace("bars = 2\n", ""));
    assert_eq!(render(&auto).samples.len(), 176_400 * 2);
}
