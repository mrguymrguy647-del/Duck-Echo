//! Roadmap task 21: checking a song without ears. Every written note is
//! listened for on its own track, and anything missing, quiet or at the
//! wrong pitch is reported with its track, bar, beat and both pitches.

use duck_echo_analyze::notes::cents;
use duck_echo_core::{load_song_str, Format, LoadedSong};
use duck_echo_synth::song::{render_song, render_song_tracks};
use duck_echo_tools::measure::{measure_document, measure_song, Options};
use duck_echo_tools::notecheck::{check_track, NoteCheck, TrackNoteCheck};
use duck_echo_tools::song::{files_next_to, Library};
use duck_echo_tools::{load_document, SongFacts};
use std::path::{Path, PathBuf};

fn songs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs")
}

fn load(src: &str) -> LoadedSong {
    let files = files_next_to(Path::new("song.toml"));
    load_song_str(src, Format::Toml, &Library::new(&files)).unwrap_or_else(|r| panic!("{r}"))
}

/// The note check of a song, listing every note.
fn check(song: &LoadedSong) -> NoteCheck {
    let names: Vec<String> = song.song.tracks.iter().map(|t| t.name.clone()).collect();
    let options = Options {
        check_notes: true,
        list_notes: true,
        ..Options::default()
    };
    let extras = options.for_song(&names, "").unwrap();
    measure_song(
        &song.song,
        &song.arrangement,
        &SongFacts::of(song),
        &[],
        &extras,
    )
    .unwrap()
    .measured
    .note_check
    .unwrap()
}

/// Check the audio of `played` against the notes of `written`: the way
/// to ask "is this render the song I wrote?".
fn check_against(played: &LoadedSong, written: &LoadedSong, track: usize) -> TrackNoteCheck {
    let (_, mastering) = render_song(&played.song, &played.arrangement);
    let stem = render_song_tracks(&played.song, &played.arrangement, &[track], &mastering);
    let onsets = duck_echo_analyze::analyze(&stem.samples, stem.sample_rate, stem.channels).onsets;
    check_track(
        &written.song,
        &written.arrangement.tracks[track],
        &written.song.tracks[track].instrument,
        &stem,
        &onsets,
        false,
    )
}

const TONE: &str = r#"
[instruments.tone]
version = 0
[[instruments.tone.layers]]
source = "sine"
volume = 0.4
envelope = { attack = 0.01, decay = 0.05, sustain = 0.8, release = 0.05 }
pitch = { start = "A4" }
"#;

fn tune(notes: &str) -> String {
    format!(
        "version = 0\ntempo = 120\nbars = 2\n{TONE}\n[[tracks]]\nname = \"bass\"\ninstrument = \"tone\"\nnotes = [{{ beat = 0, note = \"A2\", length = 8 }}]\n\n[[tracks]]\nname = \"tune\"\ninstrument = \"tone\"\nnotes = [{notes}]\n"
    )
}

const RIGHT: &str = r#"{ beat = 0, note = "A4" }, { beat = 1, note = "C5" }, { beat = 2, note = "E5" }, { beat = 3, note = "A5" }, { bar = 2, beat = 0, note = "G5" }, { bar = 2, beat = 1.5, note = "E5", length = 2 }"#;

#[test]
fn every_note_of_the_demo_is_heard_in_tune() {
    let path = songs_dir().join("four_bars.toml");
    let doc = load_document(&path, &files_next_to(&path)).unwrap();
    let options = Options {
        check_notes: true,
        list_notes: true,
        ..Options::default()
    };
    let m = measure_document(&doc, None, &options, "").unwrap().measured;
    let check = m.note_check.unwrap();
    assert_eq!((check.notes_written, check.notes_heard), (29, 29));
    assert!(
        m.analysis.summary.contains(", 29 of 29 notes heard"),
        "{}",
        m.analysis.summary
    );
    for track in &check.tracks {
        assert!(track.problems.is_empty(), "{:?}", track.problems);
        let notes = track.notes.as_ref().unwrap();
        assert_eq!(notes.len(), track.notes_written);
        for note in notes {
            assert_eq!(note.status, "heard");
            // Unlike the onsets, this reads the held part of each note,
            // so even the low bass notes are within 10 cents.
            let off = note.cents_off.unwrap();
            assert!(
                off.abs() < 10.0,
                "{} {}: {off} cents",
                track.name,
                note.written
            );
            assert_eq!(note.heard.as_deref(), Some(note.written.as_str()));
        }
    }
    // Pattern notes say where they are written, and where they landed.
    let bass = &check.tracks[0].notes.as_ref().unwrap()[9];
    assert_eq!(bass.path, "patterns.walk.notes[1]");
    assert_eq!((bass.bar, bass.beat), (3, 1.0));
    assert_eq!(bass.written, "A2");
}

#[test]
fn one_wrong_note_is_reported_with_its_track_beat_and_both_pitches() {
    // The song as it should be, and a render in which the fifth note of
    // the tune was played a tone too low (F5 instead of G5).
    let written = load(&tune(RIGHT));
    let played = load(&tune(&RIGHT.replace("\"G5\"", "\"F5\"")));
    let track = check_against(&played, &written, 1);
    assert_eq!(track.name, "tune");
    assert_eq!((track.notes_written, track.notes_heard), (6, 5));
    assert_eq!(track.problems.len(), 1, "{:?}", track.problems);
    let p = &track.problems[0];
    assert_eq!(p.status, "wrong_pitch");
    assert_eq!(p.path, "tracks[1].notes[4]");
    assert_eq!((p.bar, p.beat), (2, 0.0));
    assert_eq!(p.written, "G5");
    assert!((p.written_hz.unwrap() - 783.99).abs() < 0.1);
    assert_eq!(p.heard.as_deref(), Some("F5"));
    assert!((p.heard_hz.unwrap() - 698.46).abs() < 1.0);
    assert!(
        (p.cents_off.unwrap() + 200.0).abs() < 2.0,
        "{:?}",
        p.cents_off
    );
    // The other track is untouched.
    assert!(check_against(&played, &written, 0).problems.is_empty());
    // And the render checked against itself is all there.
    assert!(check_against(&played, &played, 1).problems.is_empty());
}

#[test]
fn missing_quiet_and_too_short_notes_are_told_apart() {
    let notes = r#"{ beat = 0, note = "A4" }, { beat = 1, note = "C5", velocity = 0.0 }, { beat = 2, note = "E5", velocity = 0.02 }, { beat = 3, note = "A5" }, { bar = 2, beat = 0, note = "C2", length = 0.04 }, { bar = 2, beat = 2, note = "G5" }"#;
    let check = check(&load(&tune(notes)));
    let tune = &check.tracks[1];
    assert_eq!(
        (tune.notes_written, tune.notes_heard, tune.notes_uncertain),
        (6, 3, 1)
    );
    let status: Vec<(&str, &str)> = tune
        .problems
        .iter()
        .map(|p| (p.written.as_str(), p.status.as_str()))
        .collect();
    assert_eq!(
        status,
        [("C5", "missing"), ("E5", "quiet"), ("C2", "uncertain")]
    );
    let quiet = &tune.problems[1];
    // The quiet note is 34 dB under the others, and still at its pitch.
    assert!(quiet.cents_off.unwrap().abs() < 10.0);
    assert!(quiet.level_dbfs.unwrap() < -40.0);
    assert_eq!(check.summary(), "4 of 7 notes heard (1 too short to read)");
}

#[test]
fn an_instrument_that_plays_another_pitch_is_caught() {
    // The first layer sets the home note but is far too quiet to hear;
    // the loud layer is a fifth higher. Every note sounds a fifth up.
    let song = load(
        r#"
version = 0
tempo = 120
bars = 1
[instruments.odd]
version = 0
[[instruments.odd.layers]]
source = "sine"
volume = 0.0001
envelope = { attack = 0.01, decay = 0.05, sustain = 0.8, release = 0.05 }
pitch = { start = "C4" }
[[instruments.odd.layers]]
source = "sine"
volume = 0.5
envelope = { attack = 0.01, decay = 0.05, sustain = 0.8, release = 0.05 }
pitch = { start = "G4" }
[[tracks]]
name = "t"
instrument = "odd"
notes = [{ beat = 0, note = "C4", length = 2 }, { beat = 2, note = "E4", length = 2 }]
"#,
    );
    let check = check(&song);
    let t = &check.tracks[0];
    assert_eq!(t.notes_heard, 0);
    let got: Vec<(&str, &str, Option<&str>)> = t
        .problems
        .iter()
        .map(|p| (p.written.as_str(), p.status.as_str(), p.heard.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("C4", "wrong_pitch", Some("G4")),
            ("E4", "wrong_pitch", Some("B4"))
        ]
    );
    assert!((t.problems[0].cents_off.unwrap() - 700.0).abs() < 2.0);
}

#[test]
fn chords_drums_loops_and_echoes() {
    let song = load(
        r#"
version = 0
tempo = 120
bars = 2
loop = true

[instruments.blip]
version = 0
effects = [{ type = "echo", delay = 0.25, feedback = 0.3, mix = 0.5 }]
[[instruments.blip.layers]]
source = "sine"
volume = 0.5
envelope = { attack = 0.005, decay = 0.08 }
pitch = { start = "A4" }

[[tracks]]
name = "chords"
instrument = "pad"
notes = [
  { beat = 0, note = ["C4", "E4", "G4"], length = 4 },
  { bar = 2, beat = 0, note = ["A3", "C4", "E4"], length = 3 },
  { bar = 2, beat = 3, note = "G3", length = 3 },
]

[[tracks]]
name = "drums"
instrument = "drums"
notes = [
  { beat = 0, note = "kick" },
  { beat = 1, note = "snare" },
  { beat = 2, note = "kick", velocity = 0.0 },
  { beat = 3, note = ["snare", "hat"] },
]

[[tracks]]
name = "blips"
instrument = "blip"
notes = [{ beat = 1, note = "A5", length = 0.25 }]
"#,
    );
    let check = check(&song);
    // Every chord note is heard under the others, and so is the last
    // note, which is held across the loop's join.
    let chords = &check.tracks[0];
    assert_eq!(
        (chords.notes_written, chords.notes_heard),
        (7, 7),
        "{:?}",
        chords.problems
    );
    let last = chords.notes.as_ref().unwrap().last().unwrap();
    assert_eq!(
        (last.written.as_str(), last.status.as_str()),
        ("G3", "heard")
    );
    assert!(last.cents_off.unwrap().abs() < 10.0);

    // Drum hits are checked by level; the silent kick is missing.
    let drums = &check.tracks[1];
    assert_eq!((drums.notes_written, drums.notes_heard), (5, 4));
    assert_eq!(drums.problems[0].written, "kick");
    assert_eq!(drums.problems[0].status, "missing");
    assert_eq!((drums.problems[0].bar, drums.problems[0].beat), (1, 2.0));
    assert!(drums.problems[0].written_hz.is_none());

    // The echo's repeats are heard but not written: they are listed as
    // extra onsets, not as problems.
    let blips = &check.tracks[2];
    assert_eq!((blips.notes_written, blips.notes_heard), (1, 1));
    assert!(blips.problems.is_empty());
    assert!(!blips.extra_onsets.is_empty());
    let first = &blips.extra_onsets[0];
    assert!((first.time_seconds - 0.75).abs() < 0.03, "{first:?}");
    assert_eq!((first.bar, first.beat), (1, 1.5));
}

#[test]
fn the_check_runs_when_asked_and_the_picture_has_bar_lines() {
    let path = songs_dir().join("four_bars.toml");
    let doc = load_document(&path, &files_next_to(&path)).unwrap();
    // Off by default (render uses this), on with per_track or when the
    // front end asks.
    let plain = measure_document(&doc, None, &Options::default(), "")
        .unwrap()
        .measured;
    assert!(plain.note_check.is_none());
    assert!(plain
        .analysis
        .summary
        .ends_with("; per_track describes each"));
    let options = Options {
        check_notes: true,
        picture: true,
        ..Options::default()
    };
    let m = measure_document(&doc, None, &options, "").unwrap().measured;
    let check = m.note_check.as_ref().unwrap();
    // Without `notes`, only problems are listed, so the answer is short.
    assert!(check.tracks.iter().all(|t| t.notes.is_none()));
    assert!(m
        .analysis
        .summary
        .ends_with(", 29 of 29 notes heard; per_track describes each"));
    let json = serde_json::to_value(&m).unwrap();
    assert_eq!(json["note_check"]["tracks"][1]["name"], "lead");
    assert_eq!(
        json["note_check"]["tracks"][1]["problems"],
        serde_json::json!([])
    );
    assert!(json["note_check"]["tracks"][1].get("notes").is_none());

    // One line per bar, at the start of each: 2.18 s apart at 110 BPM.
    let picture = m.picture.unwrap();
    assert_eq!(picture.bar_lines.len(), 4);
    assert!((picture.bar_lines[1] - 240.0 / 110.0).abs() < 1e-9);
    assert!(picture.layout.contains("start of each bar"));

    // A recipe's picture is unchanged and has no bar lines, and `notes`
    // on a recipe is an error, not ignored.
    let coin = load_document(Path::new("preset:coin"), &files_next_to(Path::new("."))).unwrap();
    let options = Options {
        picture: true,
        ..Options::default()
    };
    let m = measure_document(&coin, None, &options, "")
        .unwrap()
        .measured;
    assert!(m.picture.unwrap().bar_lines.is_empty());
    let options = Options {
        list_notes: true,
        ..Options::default()
    };
    let Err(e) = measure_document(&coin, None, &options, "--") else {
        panic!("`notes` on a recipe should be refused");
    };
    let e = e.into_report();
    assert_eq!(e.errors[0].path, "--notes");
}

/// The extra onsets of every track of a song, as (track, count).
fn extras_per_track(name: &str) -> Vec<(String, usize)> {
    let path = songs_dir().join(format!("{name}.toml"));
    let doc = load_document(&path, &files_next_to(&path)).unwrap();
    let options = Options {
        check_notes: true,
        ..Options::default()
    };
    let m = measure_document(&doc, None, &options, "").unwrap().measured;
    m.note_check
        .unwrap()
        .tracks
        .into_iter()
        .map(|t| (t.name, t.extra_onsets.len()))
        .collect()
}

/// Long low notes wobble (a closing filter, layers beating against each
/// other), and the onset finder used to report each wobble as an extra
/// note: 9 on the day bass, 16 on the night bass. None are real.
#[test]
fn held_notes_do_not_make_false_extra_notes() {
    for name in ["duck_craft_day", "duck_craft_night", "four_bars"] {
        for (track, extras) in extras_per_track(name) {
            assert_eq!(extras, 0, "{name}, track {track}");
        }
    }
}

/// A piano holding A3 for two bars, plus `extra` notes on the same track.
fn held_piano(extra: &str) -> LoadedSong {
    load(&format!(
        "version = 0\ntempo = 120\nbars = 2\n[[tracks]]\nname = \"keys\"\ninstrument = \"piano\"\nnotes = [{{ beat = 0, note = \"A3\", length = 8 }}{extra}]\n"
    ))
}

#[test]
fn an_unwritten_note_is_still_reported_as_extra() {
    // The song as written: a held A3. What was played: the same, plus an
    // E4 nobody wrote, while the A3 is still held (bar 2, beat 1).
    let written = held_piano("");
    let played = held_piano(r#", { bar = 2, beat = 1, note = "E4", length = 2 }"#);
    let keys = check_against(&played, &written, 0);
    assert_eq!(keys.extra_onsets.len(), 1, "{:?}", keys.extra_onsets);
    let extra = &keys.extra_onsets[0];
    assert_eq!((extra.bar, extra.beat), (2, 1.0));
    // 120 BPM: bar 2, beat 1 is 2.5 seconds in.
    assert!((extra.time_seconds - 2.5).abs() < 0.03, "{extra:?}");
    let hz = extra.note_hz.unwrap() as f64;
    assert!(cents(hz, 329.63).abs() < 50.0, "{hz}");
    // The written note is still heard.
    assert_eq!((keys.notes_written, keys.notes_heard), (1, 1));
    // Played as written: nothing extra.
    assert!(check_against(&written, &written, 0).extra_onsets.is_empty());

    // The same pitch struck again while the first is held: no new
    // pitch, but a clear jump in level, so it is a new note too.
    let again = held_piano(r#", { bar = 2, beat = 0, note = "A3", length = 1 }"#);
    let keys = check_against(&again, &written, 0);
    assert_eq!(keys.extra_onsets.len(), 1, "{:?}", keys.extra_onsets);
    assert_eq!(
        (keys.extra_onsets[0].bar, keys.extra_onsets[0].beat),
        (2, 0.0)
    );
}

/// A held bass note wobbles: its filter closes and its two layers beat.
/// The onset finder reports those wobbles, some with a nonsense pitch
/// near 20 kHz. None of them is an extra note.
#[test]
fn a_wobbling_bass_note_is_not_extra() {
    let song = load(
        "version = 0\ntempo = 120\nbars = 2\n[[tracks]]\nname = \"low\"\ninstrument = \"bass\"\nnotes = [{ beat = 0, note = \"A1\", length = 8 }]\n",
    );
    let (_, mastering) = render_song(&song.song, &song.arrangement);
    let stem = render_song_tracks(&song.song, &song.arrangement, &[0], &mastering);
    let onsets = duck_echo_analyze::analyze(&stem.samples, stem.sample_rate, stem.channels).onsets;
    // The onset finder itself does see many starts here...
    assert!(onsets.len() > 10, "{}", onsets.len());
    // ...but the note check explains every one by the held note.
    let low = check_against(&song, &song, 0);
    assert!(low.extra_onsets.is_empty(), "{:?}", low.extra_onsets);
    assert_eq!(low.notes_heard, 1);
}

#[test]
fn drum_tracks_are_still_checked_by_time() {
    let beat = |extra: &str| {
        load(&format!(
            "version = 0\ntempo = 120\nbars = 1\n[[tracks]]\nname = \"drums\"\ninstrument = \"drums\"\nnotes = [{{ beat = 0, note = \"kick\" }}, {{ beat = 1, note = \"snare\" }}, {{ beat = 2, note = \"kick\" }}{extra}]\n"
        ))
    };
    let written = beat("");
    // A hi-hat nobody wrote, half way between two written hits.
    let played = beat(r#", { beat = 2.5, note = "hat" }"#);
    let drums = check_against(&played, &written, 0);
    assert_eq!((drums.notes_written, drums.notes_heard), (3, 3));
    assert_eq!(drums.extra_onsets.len(), 1, "{:?}", drums.extra_onsets);
    assert_eq!(
        (drums.extra_onsets[0].bar, drums.extra_onsets[0].beat),
        (1, 2.5)
    );
    // Played as written: nothing extra.
    assert!(check_against(&written, &written, 0).extra_onsets.is_empty());
}
