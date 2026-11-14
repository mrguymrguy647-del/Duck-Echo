//! Roadmap task 16: songs render the same every time, a note plays its
//! instrument at the right pitch and length, changing one thing only
//! changes that thing, and the stems of a song add up to its mix.

use duck_echo_audio::{encode_wav, AudioBuffer, WavFormat};
use duck_echo_core::{load_song_str, Code, Format, LoadedSong, Report};
use duck_echo_synth::song::{render_song, render_song_tracks};
use duck_echo_tools::measure::{measure_document, Options};
use duck_echo_tools::song::{files_next_to, Library};
use duck_echo_tools::{load_document, Document};
use std::path::{Path, PathBuf};

fn songs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Load a song from TOML text. Instrument files are looked for next to
/// `base` (a made-up file name in the folder to read from).
fn load_at(src: &str, base: &Path) -> Result<LoadedSong, Report> {
    let files = files_next_to(base);
    load_song_str(src, Format::Toml, &Library::new(&files))
}

fn load(src: &str) -> LoadedSong {
    load_at(src, Path::new("song.toml")).unwrap_or_else(|r| panic!("{r}"))
}

fn errors(src: &str) -> Report {
    load_at(src, Path::new("song.toml")).expect_err("the song should be invalid")
}

/// Render with no master gain change, so renders can be compared sample
/// for sample.
fn raw(song: &LoadedSong) -> AudioBuffer {
    render_song(&song.song, &song.arrangement).0
}

/// One track alone, at the level it has in the mix when the master gain
/// is 1.
fn stem(song: &LoadedSong, track: usize) -> AudioBuffer {
    let (full, mastering) = render_song(&song.song, &song.arrangement);
    assert_eq!(mastering.gain, 1.0);
    let _ = full;
    render_song_tracks(&song.song, &song.arrangement, &[track], &mastering)
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// The loudest sample in a stretch of time, any channel.
fn peak_between(buf: &AudioBuffer, from: f64, to: f64) -> f32 {
    let ch = buf.channels as usize;
    let a = ((from * buf.sample_rate as f64) as usize * ch).min(buf.samples.len());
    let b = ((to * buf.sample_rate as f64) as usize * ch).min(buf.samples.len());
    peak(&buf.samples[a..b])
}

fn cents(heard: f64, written: f64) -> f64 {
    1200.0 * (heard / written).log2()
}

/// A plain sine instrument written at A4, and a noise instrument. Neither
/// is levelled, so the mix is not normalized.
const INSTRUMENTS: &str = r#"
[instruments.tone]
version = 0
[[instruments.tone.layers]]
source = "sine"
volume = 0.3
envelope = { attack = 0.01, decay = 0.05, sustain = 0.8, release = 0.05 }
pitch = { start = "A4" }

[instruments.hiss]
version = 0
[[instruments.hiss.layers]]
source = "noise"
volume = 0.2
envelope = { attack = 0.001, decay = 0.08 }
"#;

fn two_tracks() -> String {
    format!(
        r#"
version = 0
tempo = 120
bars = 2
{INSTRUMENTS}
[[tracks]]
name = "tune"
instrument = "tone"
notes = [
  {{ beat = 0, note = "A4" }},
  {{ beat = 1, note = "C5" }},
  {{ beat = 2, note = "E5", length = 2 }},
  {{ bar = 2, beat = 0, note = "A5", length = 2 }},
]

[[tracks]]
name = "hats"
instrument = "hiss"
notes = [
  {{ beat = 0, note = "A4" }},
  {{ beat = 1, note = "A4" }},
  {{ beat = 2, note = "A4" }},
  {{ beat = 3, note = "A4" }},
]
"#
    )
}

/// Fingerprints of the example songs: (name, WAV bytes, FNV-1a 64 of the
/// PCM16 WAV). Change one only when the sound is meant to change, and say
/// so in the commit.
const SONG_GOLDEN: &[(&str, usize, u64)] = &[
    ("duck_craft_day", 4_032_044, 0x6358_3802_2088_7df6),
    ("duck_craft_night", 4_032_044, 0x9694_28e7_a634_e1e5),
    ("four_bars", 1_553_648, 0x9f83_fe29_2800_2521),
];

#[test]
fn example_songs_are_pinned() {
    let mut on_disk: Vec<String> = std::fs::read_dir(songs_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut pinned: Vec<String> = SONG_GOLDEN.iter().map(|g| g.0.to_string()).collect();
    pinned.sort();
    assert_eq!(pinned, on_disk, "pin every song in examples/songs");

    for (name, len, hash) in SONG_GOLDEN {
        let path = songs_dir().join(format!("{name}.toml"));
        let doc = load_document(&path, &files_next_to(&path)).unwrap_or_else(|e| panic!("{e}"));
        assert!(doc.report().warnings.is_empty(), "{name}: {}", doc.report());
        let render = || {
            let m = measure_document(&doc, None, &Options::default(), "").unwrap();
            encode_wav(&m.buffer, WavFormat::Pcm16).unwrap()
        };
        let wav = render();
        assert_eq!(wav, render(), "{name} renders differently the second time");
        assert_eq!(
            (wav.len(), fnv1a64(&wav)),
            (*len, *hash),
            "{name} changed: (\"{name}\", {}, {:#018x})",
            wav.len(),
            fnv1a64(&wav)
        );
    }
}

#[test]
fn four_bar_demo_is_stereo_44k_and_its_notes_read_back() {
    let path = songs_dir().join("four_bars.toml");
    let doc = load_document(&path, &files_next_to(&path)).unwrap();
    let Document::Song(song) = &doc else {
        panic!("four_bars is a song");
    };
    let options = Options {
        per_track: true,
        ..Options::default()
    };
    let m = measure_document(&doc, None, &options, "").unwrap();
    assert_eq!((m.buffer.sample_rate, m.buffer.channels), (44_100, 2));
    let facts = m.measured.song.as_ref().unwrap();
    assert_eq!((facts.bars, facts.tracks, facts.notes), (4, 2, 29));
    // 16 beats at 110 BPM, to the sample.
    assert_eq!(facts.song_frames, 384_873);
    assert!(m
        .measured
        .analysis
        .summary
        .starts_with("4 bars at 110 BPM, 2 tracks; "));
    let loudness = m.measured.analysis.loudness_lufs.unwrap();
    assert!((loudness + 16.0).abs() < 0.1, "{loudness}");

    // Every written note shows up as an onset, at its time and pitch.
    let tracks = m.measured.tracks.as_ref().unwrap();
    assert_eq!(tracks.len(), 2);
    for (track, plan) in tracks.iter().zip(&song.loaded.arrangement.tracks) {
        assert_eq!(track.name, plan.name);
        assert_eq!(track.notes, plan.events.len());
        let onsets = &track.analysis.onsets;
        assert_eq!(onsets.len(), plan.events.len(), "{}", track.name);
        for (onset, event) in onsets.iter().zip(&plan.events) {
            let at = event.beat * song.loaded.song.seconds_per_beat();
            assert!(
                (onset.time_seconds - at).abs() < 0.03,
                "{} at {at}",
                track.name
            );
            let duck_echo_core::Sound::Pitch { hz, .. } = &event.sound else {
                panic!("a pitched note");
            };
            let off = cents(onset.note_hz.unwrap() as f64, *hz as f64);
            // Onsets read a short stretch, so low notes are rough; half a
            // semitone still means "the right note".
            assert!(
                off.abs() < 50.0,
                "{} {hz} Hz read {off} cents off",
                track.name
            );
        }
    }
}

#[test]
fn stems_add_up_to_the_mix() {
    let path = songs_dir().join("four_bars.toml");
    let Document::Song(song) = load_document(&path, &files_next_to(&path)).unwrap() else {
        panic!("a song");
    };
    let (full, mastering) = render_song(&song.loaded.song, &song.loaded.arrangement);
    // The demo is levelled, so the stems must carry the mix's gain.
    assert!((mastering.gain - 1.0).abs() > 0.05, "{}", mastering.gain);
    assert!(
        peak(&full.samples) < 0.999,
        "the mix must not clip for stems to add up"
    );
    let mut sum = vec![0.0f32; full.samples.len()];
    for track in 0..song.loaded.song.tracks.len() {
        let stem = render_song_tracks(
            &song.loaded.song,
            &song.loaded.arrangement,
            &[track],
            &mastering,
        );
        // Same length as the mix: lined up in time.
        assert_eq!(stem.samples.len(), full.samples.len());
        for (s, x) in sum.iter_mut().zip(&stem.samples) {
            *s += x;
        }
    }
    let worst = sum
        .iter()
        .zip(&full.samples)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    // Only float rounding is left: far below one step of a 16-bit file.
    assert!(worst < 1e-5, "stems differ from the mix by {worst}");
}

#[test]
fn adding_or_moving_a_track_does_not_change_the_others() {
    let base = load(&two_tracks());
    let hats = stem(&base, 1);
    assert!(peak(&hats.samples) > 0.01);

    // A third track at the end.
    let extra = r#"
[[tracks]]
name = "low"
instrument = "tone"
notes = [{ beat = 0, note = "A2", length = 4 }]
"#;
    let more = load(&format!("{}{extra}", two_tracks()));
    assert_eq!(stem(&more, 1).samples, hats.samples);

    // The same third track put first: the hats are now track 2, and still
    // the same samples, because their seed comes from their name.
    let text = two_tracks();
    let at = text.find("[[tracks]]").unwrap();
    let first = load(&format!("{}{extra}{}", &text[..at], &text[at..]));
    assert_eq!(first.song.tracks[2].name, "hats");
    assert_eq!(stem(&first, 2).samples, hats.samples);

    // Renaming the track changes its noise (and nothing else is asked).
    let renamed = load(&two_tracks().replace("name = \"hats\"", "name = \"cymbals\""));
    assert_ne!(stem(&renamed, 1).samples, hats.samples);
}

#[test]
fn changing_a_note_does_not_change_the_other_notes() {
    let base = load(&two_tracks());
    // Drop the hat on beat 1. The hats on beats 0, 2 and 3 keep their
    // noise, because a note's seed comes from where it starts.
    let fewer = load(&two_tracks().replace(
        "  { beat = 1, note = \"A4\" },\n  { beat = 2, note = \"A4\" },",
        "  { beat = 2, note = \"A4\" },",
    ));
    assert_eq!(fewer.arrangement.tracks[1].events.len(), 3);
    let (a, b) = (stem(&base, 1), stem(&fewer, 1));
    let sr = a.sample_rate as usize * a.channels as usize;
    // At 120 BPM a beat is half a second.
    assert_eq!(a.samples[..sr / 2], b.samples[..sr / 2], "beat 0");
    assert_eq!(a.samples[sr..], b.samples[sr..], "beats 2 and 3");
    assert!(peak(&b.samples[sr / 2..sr]) < 1e-6, "beat 1 is gone");
    // Two hits never share their noise.
    assert_ne!(a.samples[..sr / 2], a.samples[sr..sr + sr / 2]);
}

#[test]
fn the_same_song_with_another_seed_differs_only_in_its_random_parts() {
    let base = load(&two_tracks());
    let seeded = load(&two_tracks().replace("tempo = 120", "tempo = 120\nseed = 7"));
    assert_eq!(stem(&base, 0).samples, stem(&seeded, 0).samples, "sines");
    assert_ne!(stem(&base, 1).samples, stem(&seeded, 1).samples, "noise");
}

#[test]
fn a_note_is_held_for_its_length_then_released() {
    // A slow piano-like sound: no sustain, two seconds of decay.
    let song = |length: f64| {
        load(&format!(
            r#"
version = 0
tempo = 60
bars = 4
[instruments.pluck]
version = 0
[[instruments.pluck.layers]]
source = "sine"
volume = 0.5
envelope = {{ attack = 0.005, decay = 2.0, release = 0.1 }}
pitch = {{ start = "A4" }}
[[tracks]]
name = "p"
instrument = "pluck"
notes = [{{ beat = 0, note = "A4", length = {length} }}]
"#
        ))
    };
    // Held for half a second: it rings while held, and is silent 0.1 s
    // (the release) after it is let go, long before the decay ends.
    let short = raw(&song(0.5));
    assert!(peak_between(&short, 0.3, 0.5) > 0.2);
    assert!(peak_between(&short, 0.61, 4.0) < 1e-6);
    // Held for three seconds: the sound fades by itself after two.
    let long = raw(&song(3.0));
    assert!(peak_between(&long, 1.0, 1.5) > 0.05);
    assert!(peak_between(&long, 2.01, 4.0) < 1e-6);

    // A sustained sound is held for the whole note, whatever
    // `sustain_time` the recipe has.
    let held = raw(&load(&format!(
        "version = 0\ntempo = 60\nbars = 4\n{INSTRUMENTS}\n[[tracks]]\nname = \"t\"\ninstrument = \"tone\"\nnotes = [{{ beat = 0, note = \"A4\", length = 3 }}]\n"
    )));
    assert!(peak_between(&held, 2.5, 3.0) > 0.1);
    assert!(peak_between(&held, 3.06, 4.0) < 1e-6);
}

#[test]
fn velocity_scales_the_note_and_pitch_follows_it() {
    let song = |note: &str, velocity: f32| {
        raw(&load(&format!(
            "version = 0\ntempo = 60\nbars = 1\nchannels = 1\n{INSTRUMENTS}\n[[tracks]]\nname = \"t\"\ninstrument = \"tone\"\nnotes = [{{ beat = 0, note = {note}, length = 2, velocity = {velocity} }}]\n"
        )))
    };
    let full = song("\"A4\"", 1.0);
    let half = song("\"A4\"", 0.5);
    assert_eq!((full.channels, full.sample_rate), (1, 44_100));
    assert!((peak(&half.samples) / peak(&full.samples) - 0.5).abs() < 1e-3);

    // A note name and its frequency are the same note.
    assert_eq!(song("440", 1.0).samples, full.samples);
    // Notes read back at their pitch across four octaves.
    for (note, hz) in [("\"A2\"", 110.0), ("\"E4\"", 329.63), ("\"A6\"", 1760.0)] {
        let buf = song(note, 1.0);
        let a = duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels);
        let heard = a.onsets[0].note_hz.unwrap() as f64;
        assert!(cents(heard, hz).abs() < 10.0, "{note} read {heard}");
    }
}

#[test]
fn instrument_effects_run_once_per_track_and_levels_are_set_once() {
    // An echo on the instrument: the repeat is heard after the note.
    let echo = raw(&load(
        r#"
version = 0
tempo = 60
bars = 2
[instruments.blip]
version = 0
effects = [{ type = "echo", delay = 0.5, feedback = 0.0, mix = 0.5 }]
[[instruments.blip.layers]]
source = "sine"
volume = 0.5
envelope = { attack = 0.005, decay = 0.05 }
pitch = { start = "A4" }
[[tracks]]
name = "t"
instrument = "blip"
notes = [{ beat = 0, note = "A4" }]
"#,
    ));
    assert!(peak_between(&echo, 0.1, 0.45) < 1e-6);
    assert!(peak_between(&echo, 0.5, 0.6) > 0.05);

    // An instrument that normalizes itself is as loud in a song as alone,
    // however quiet its layers are; velocity still works on top.
    let levelled = |volume: f32, velocity: f32| {
        peak(
            &raw(&load(&format!(
                r#"
version = 0
tempo = 60
bars = 1
channels = 1
[instruments.blip]
version = 0
master = {{ normalize = true, normalize_peak_db = -6 }}
[[instruments.blip.layers]]
source = "sine"
volume = {volume}
envelope = {{ attack = 0.005, decay = 0.3 }}
pitch = {{ start = "A4" }}
[[tracks]]
name = "t"
instrument = "blip"
notes = [{{ beat = 0, note = "A4", velocity = {velocity} }}]
"#
            )))
            .samples,
        )
    };
    let target = 10f32.powf(-6.0 / 20.0);
    assert!((levelled(0.1, 1.0) - target).abs() < 0.01);
    assert!((levelled(0.9, 1.0) - target).abs() < 0.01);
    assert!((levelled(0.9, 0.5) - target * 0.5).abs() < 0.01);
}

#[test]
fn kits_presets_and_files_can_be_instruments() {
    let dir = std::env::temp_dir().join(format!("duck-echo-songs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lead.toml"),
        "version = 0\n[[layers]]\nsource = \"square\"\npitch = { start = \"C4\" }\n",
    )
    .unwrap();
    let text = r#"
version = 0
tempo = 120
bars = 1
[instruments.tick]
version = 0
[[instruments.tick.layers]]
source = "noise"
envelope = { attack = 0.001, decay = 0.03 }

[kits.kit]
kick = "preset:thud"
hat = "tick"

[[tracks]]
name = "drums"
instrument = "kit"
notes = [
  { beat = 0, note = "kick" },
  { beat = 1, note = ["kick", "hat"] },
  { beat = 2, note = "hat", velocity = 0.5 },
]

[[tracks]]
name = "coins"
instrument = "preset:coin"
notes = [{ beat = 0, note = "C6" }]

[[tracks]]
name = "lead"
instrument = "lead.toml"
notes = [{ beat = 2, note = "G4" }]
"#;
    let song = load_at(text, &dir.join("song.toml")).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(song.arrangement.note_count(), 6);
    for track in 0..3 {
        assert!(peak(&stem(&song, track).samples) > 0.01, "track {track}");
    }

    // The same song read from another folder cannot find the file.
    let report = load_at(text, Path::new("elsewhere/song.toml")).unwrap_err();
    assert_eq!(report.errors.len(), 1, "{report}");
    assert_eq!(report.errors[0].code, Code::NotFound);
    assert_eq!(report.errors[0].path, "tracks[2].instrument");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn song_errors_come_all_at_once_with_exact_paths() {
    let report = errors(
        r#"
version = 0
tempo = 500
bars = 2
[patterns.riff]
notes = [{ beat = 0, note = "Q3" }]
[instruments.tone]
version = 0
[[instruments.tone.layers]]
source = "sine"
volum = 0.5
[[tracks]]
name = "a"
instrument = "tone"
notes = [{ beat = 0, note = "C4" }]
[[tracks]]
name = "a"
instrument = "tone"
velocity = 1
notes = [
  { beat = 0, note = "C4" },
  { bar = 0, beat = 0, note = "C4" },
  { beat = 0, note = "C4", length = 0 },
  { beat = 1, note = "H5" },
]
play = [{ pattern = "rif", bars = [1] }]
"#,
    );
    let paths: Vec<(&str, Code)> = report
        .errors
        .iter()
        .map(|i| (i.path.as_str(), i.code))
        .collect();
    for expected in [
        ("instruments.tone.layers[0].volum", Code::UnknownField),
        ("tracks[1].velocity", Code::UnknownField),
        ("tempo", Code::OutOfRange),
        ("tracks[1].name", Code::Invalid),
        ("tracks[1].play[0].pattern", Code::NotFound),
    ] {
        assert!(
            paths.contains(&expected),
            "{expected:?} missing from {paths:?}"
        );
    }
    let find = |path: &str| report.errors.iter().find(|i| i.path == path).unwrap();
    assert_eq!(
        find("instruments.tone.layers[0].volum")
            .suggestion
            .as_deref(),
        Some("volume")
    );
    assert_eq!(
        find("tracks[1].play[0].pattern").suggestion.as_deref(),
        Some("riff")
    );
    assert!(report.errors.iter().all(|i| i.line.is_some()), "{report}");

    // With the tempo fixed, the notes are checked too.
    let report = errors(
        r#"
version = 0
tempo = 100
bars = 2
[patterns.riff]
notes = [{ beat = 0, note = "Q3" }]
[instruments.tone]
version = 0
[[instruments.tone.layers]]
source = "sine"
[[tracks]]
name = "a"
instrument = "tone"
notes = [{ beat = 9, note = "C4" }]
[[tracks]]
name = "b"
instrument = "organ"
notes = [
  { beat = 0, note = "C4" },
  { bar = 0, beat = 0, note = "C4" },
  { beat = 0, note = "C4", length = 0 },
  { beat = 1, note = "H5" },
  { beat = 1, note = ["C4", 30000] },
  { beat = 1, note = "kick" },
]
play = [{ pattern = "riff", bars = [1, 2, 3], transpose = 99 }]
"#,
    );
    let paths: Vec<&str> = report.errors.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "tracks[0].notes[0].beat",
            "tracks[1].instrument",
            "tracks[1].notes[1].bar",
            "tracks[1].notes[2].length",
            "tracks[1].play[0].transpose",
            "tracks[1].play[0].bars[2]",
        ],
        "{report}"
    );
    // The instrument is unknown, so its notes cannot be checked against
    // it; with it fixed they are.
    let report = errors(
        r#"
version = 0
tempo = 100
[patterns.riff]
notes = [{ beat = 0, note = "Q3" }]
[instruments.tone]
version = 0
[[instruments.tone.layers]]
source = "sine"
[kits.kit]
kick = "tone"
[[tracks]]
name = "b"
instrument = "tone"
notes = [
  { beat = 1, note = "H5" },
  { beat = 1, note = ["C4", 30000] },
  { beat = 1, note = "kick" },
]
play = [{ pattern = "riff", bars = [1, 2] }]
[[tracks]]
name = "d"
instrument = "kit"
notes = [{ beat = 0, note = "kik" }, { beat = 0, note = 440 }]
"#,
    );
    let paths: Vec<&str> = report.errors.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "tracks[0].notes[0].note",
            "tracks[0].notes[1].note[1]",
            "tracks[0].notes[2].note",
            // Reported once, though the pattern is placed twice.
            "patterns.riff.notes[0].note",
            "tracks[1].notes[0].note",
            "tracks[1].notes[1].note",
        ],
        "{report}"
    );
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("B5"));
    assert!(report.errors[2].message.contains("must use a kit"));
    assert_eq!(report.errors[4].suggestion.as_deref(), Some("kick"));
}

#[test]
fn a_document_is_a_song_or_a_recipe_not_both() {
    use duck_echo_core::load::{document_kind, DocumentKind};
    assert_eq!(
        document_kind("version = 0\n[[layers]]\n", Format::Toml),
        DocumentKind::Recipe
    );
    assert_eq!(
        document_kind("{\"version\": 0, \"tracks\": []}", Format::Json),
        DocumentKind::Song
    );
    let report = errors(&format!(
        "{}\n[[layers]]\nsource = \"sine\"\n",
        two_tracks()
    ));
    assert_eq!(report.errors[0].path, "layers");
    assert_eq!(report.errors[0].code, Code::UnknownField);

    // A song with no tracks says so.
    let report = errors("version = 0\ntempo = 100\n");
    assert_eq!(report.errors[0].path, "tracks");
}

#[test]
fn toml_and_json_songs_are_the_same_song() {
    let toml_song = load(&two_tracks());
    let json = serde_json::to_string(&toml_song.song).unwrap();
    let files = files_next_to(Path::new("song.json"));
    let json_song = load_song_str(&json, Format::Json, &Library::new(&files)).unwrap();
    assert_eq!(toml_song.song, json_song.song);
    assert_eq!(raw(&toml_song).samples, raw(&json_song).samples);
}

#[test]
fn the_song_length_is_its_bars_plus_what_still_rings() {
    // Two bars at 120 BPM are exactly 4 seconds.
    let song = load(&two_tracks());
    let buf = raw(&song);
    assert_eq!(buf.samples.len() / 2, 4 * 44_100);
    // A note that rings past the last bar makes the file longer.
    let ringing = load(&two_tracks().replace(
        "{ bar = 2, beat = 0, note = \"A5\", length = 2 }",
        "{ bar = 2, beat = 0, note = \"A5\", length = 4 }",
    ));
    assert_eq!(raw(&ringing).samples.len() / 2, 4 * 44_100 + 2205);
    // Without `bars`, the song ends at the bar line after its last note.
    let open = load(&two_tracks().replace("bars = 2\n", ""));
    assert_eq!(open.arrangement.bars, 2);
}

/// The Duck Craft demo: two loops a game can fade between.
#[test]
fn duck_craft_day_and_night_are_matching_loops() {
    let options = Options {
        check_notes: true,
        ..Options::default()
    };
    let measure = |name: &str| {
        let path = songs_dir().join(format!("{name}.toml"));
        let doc = load_document(&path, &files_next_to(&path)).unwrap();
        measure_document(&doc, None, &options, "").unwrap()
    };
    let (day, night) = (measure("duck_craft_day"), measure("duck_craft_night"));

    for (name, m) in [("day", &day), ("night", &night)] {
        // 44.1 kHz stereo, and exactly eight bars: 32 beats at 84 BPM.
        assert_eq!(
            (m.buffer.sample_rate, m.buffer.channels),
            (44_100, 2),
            "{name}"
        );
        assert_eq!(m.buffer.samples.len(), 1_008_000 * 2, "{name}");
        let facts = m.measured.song.as_ref().unwrap();
        assert!(facts.loop_, "{name}");
        assert_eq!(
            (facts.bars, facts.tempo, facts.song_frames),
            (8, 84.0, 1_008_000)
        );
        // It joins itself with no click, sits at its loudness target, does
        // not clip, and every written note can be heard.
        assert!(m.measured.loop_seam.unwrap().clean, "{name}");
        let a = &m.measured.analysis;
        assert!((a.loudness_lufs.unwrap() + 18.0).abs() < 0.1, "{name}");
        assert!(!a.clipping.clipped, "{name}");
        let check = m.measured.note_check.as_ref().unwrap();
        assert_eq!(check.notes_heard, check.notes_written, "{name}: {check:?}");
        assert!(m.measured.warnings.is_empty(), "{name}");
        // The game engine mixes stereo down to mono for now. The loops
        // must survive that: the two sides may not cancel each other.
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        let mono = m.buffer.to_mono();
        assert!(
            rms(&mono) > 0.85 * rms(&m.buffer.samples),
            "{name} loses level in mono"
        );
    }
    // Day is calm and bright, night is dark.
    let brightness = |m: &duck_echo_tools::measure::RecipeMeasurement| {
        m.measured.analysis.spectral_centroid_hz.unwrap()
    };
    assert!(brightness(&night) < brightness(&day) * 0.8);
    // "dark" or "very dark" (below 800 Hz, see ANALYSIS.md). A song's
    // summary leaves the word out, so read the number.
    assert!(brightness(&night) < 800.0, "{}", brightness(&night));

    // The same length to the sample, so a game can start both together
    // and fade between them. Half way through a fade nothing clips.
    assert_eq!(day.buffer.samples.len(), night.buffer.samples.len());
    let worst = day
        .buffer
        .samples
        .iter()
        .zip(&night.buffer.samples)
        .fold(0.0f32, |m, (d, n)| m.max((0.5 * d + 0.5 * n).abs()));
    assert!(worst < 0.9, "{worst}");
}
