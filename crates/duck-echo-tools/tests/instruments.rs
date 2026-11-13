//! Roadmap task 19: the starter instruments are in tune across their
//! range, sit at their loudness targets, and are pinned.

use duck_echo_analyze::notes::{cents, spectrum_peak};
use duck_echo_audio::{encode_wav, AudioBuffer, WavFormat};
use duck_echo_core::note::{note_name, note_number, note_number_to_hz};
use duck_echo_core::{load_song_str, Format, LoadedSong};
use duck_echo_synth::song::{render_song, render_song_tracks};
use duck_echo_tools::instruments::{self, INSTRUMENTS, KITS};
use duck_echo_tools::song::{files_next_to, Library};
use std::path::{Path, PathBuf};

fn instruments_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../instruments")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn load(src: &str) -> LoadedSong {
    let files = files_next_to(Path::new("song.toml"));
    load_song_str(src, Format::Toml, &Library::new(&files)).unwrap_or_else(|r| panic!("{r}"))
}

fn mono(buf: &AudioBuffer) -> Vec<f32> {
    buf.to_mono()
}

/// Fingerprints of the built-in instruments rendered alone: (name, WAV
/// bytes, FNV-1a 64 of the PCM16 WAV). Change one only when the sound is
/// meant to change, and say so in the commit.
const INSTRUMENT_GOLDEN: &[(&str, usize, u64)] = &[
    ("bass", 69192, 0xb186_4692_203b_a7df),
    ("chip_lead", 46260, 0x3af0_8ab7_0cfd_cc6d),
    ("clap", 39116, 0x3d69_e522_145d_f4c9),
    ("hat", 4102, 0x0f91_2ade_a8dd_407c),
    ("kick", 24828, 0x74e1_3d6a_89f7_3bed),
    ("pad", 890864, 0x3309_7cce_df93_3395),
    ("piano", 222484, 0x2120_fe89_a14d_451f),
    ("snare", 14244, 0x1e24_0232_f17f_c4fd),
];

#[test]
fn embedded_list_matches_the_instruments_folder() {
    let mut on_disk: Vec<String> = std::fs::read_dir(instruments_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let embedded: Vec<String> = INSTRUMENTS.iter().map(|i| i.name.to_string()).collect();
    assert_eq!(
        embedded, on_disk,
        "add new instruments to the instruments! list in duck-echo-tools/src/instruments.rs"
    );
    let pinned: Vec<String> = INSTRUMENT_GOLDEN.iter().map(|g| g.0.to_string()).collect();
    assert_eq!(pinned, on_disk, "pin every instrument");
    // Every kit piece is a real instrument.
    for kit in KITS {
        for (_, instrument) in kit.pieces {
            assert!(instruments::find(instrument).is_some(), "{instrument}");
        }
    }
}

#[test]
fn every_instrument_is_clean_levelled_and_pinned() {
    let mut failures = Vec::new();
    for i in INSTRUMENTS {
        let loaded = instruments::load(i).unwrap_or_else(|e| panic!("{}: {e}", i.name));
        let r = &loaded.recipe;
        assert!(
            loaded.report.warnings.is_empty(),
            "{}: {}",
            i.name,
            loaded.report
        );
        assert_eq!(r.name.as_deref(), Some(i.name));
        assert!(r.description.as_deref().is_some_and(|d| d.ends_with('.')));
        assert_eq!(r.tags.first().map(String::as_str), Some("instrument"));
        // Clean oscillators, and no jitter: a note must land on its pitch.
        assert_eq!(r.variation, 0.0, "{}", i.name);
        assert!(r
            .layers
            .iter()
            .all(|l| l.aliasing == duck_echo_core::Aliasing::Clean));

        let target = r.master.normalize_lufs.expect("a loudness target");
        let (buf, mastering) = duck_echo_synth::render_mastered(r).unwrap();
        assert!(
            !mastering.ceiling_limited,
            "{} cannot reach its target",
            i.name
        );
        let a = duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels);
        assert!(!a.clipping.clipped, "{} clips", i.name);
        let loudness = a.loudness_lufs.unwrap();
        assert!(
            (loudness - target).abs() < 0.1,
            "{}: {loudness} vs {target}",
            i.name
        );

        let info = instruments::info(i).unwrap();
        assert_eq!(info.target_lufs, Some(target));
        assert_eq!(info.range.is_some(), i.range.is_some());

        let wav = encode_wav(&buf, WavFormat::Pcm16).unwrap();
        let (_, len, hash) = INSTRUMENT_GOLDEN.iter().find(|g| g.0 == i.name).unwrap();
        if (wav.len(), fnv1a64(&wav)) != (*len, *hash) {
            failures.push(format!(
                "    (\"{}\", {}, {:#018x}),",
                i.name,
                wav.len(),
                fnv1a64(&wav)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "instruments changed:\n{}",
        failures.join("\n")
    );
}

/// The notes of a C major scale from `from` (a C) up one octave, as MIDI
/// note numbers.
fn c_major(from: i32) -> Vec<i32> {
    [0, 2, 4, 5, 7, 9, 11, 12]
        .iter()
        .map(|s| from + s)
        .collect()
}

/// Play `notes` one after another on `instrument` and read each back.
/// Returns (note name, cents off) for every note.
fn play_and_read(instrument: &str, notes: &[i32], beats: f64) -> Vec<(String, f64)> {
    let written: Vec<String> = notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            format!(
                "{{ beat = {}, note = \"{}\", length = {} }}",
                i as f64 * beats,
                note_name(*n).unwrap(),
                beats * 0.9
            )
        })
        .collect();
    let song = load(&format!(
        "version = 0\ntempo = 60\n[[tracks]]\nname = \"t\"\ninstrument = \"{instrument}\"\nnotes = [{}]\n",
        written.join(", ")
    ));
    let (_, mastering) = render_song(&song.song, &song.arrangement);
    let buf = render_song_tracks(&song.song, &song.arrangement, &[0], &mastering);
    let samples = mono(&buf);
    let sr = buf.sample_rate as f64;
    notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let hz = note_number_to_hz(*n) as f64;
            // Read the held part of the note: from half way in (past the
            // attack, and past the tail of the note before) to its end.
            let start = ((i as f64 + 0.5) * beats * sr) as usize;
            let end = ((i as f64 + 0.9) * beats * sr) as usize;
            let peak = spectrum_peak(&samples[start..end], buf.sample_rate, hz / 1.06, hz * 1.06)
                .unwrap_or_else(|| {
                    panic!("{instrument} {}: nothing heard", note_name(*n).unwrap())
                });
            (note_name(*n).unwrap(), cents(peak.hz, hz))
        })
        .collect()
}

#[test]
fn every_pitched_instrument_plays_a_c_major_scale_within_10_cents() {
    for i in INSTRUMENTS {
        let Some((lo, hi)) = i.range else { continue };
        let (lo, hi) = (note_number(lo).unwrap(), note_number(hi).unwrap());
        // A C major scale in every octave of the instrument's range, and
        // the two ends of the range themselves.
        let mut notes = vec![lo, hi];
        let mut c = (lo + 11) / 12 * 12;
        while c + 12 <= hi {
            notes.extend(c_major(c));
            c += 12;
        }
        notes.sort();
        notes.dedup();
        // A pad swells in slowly and rings on for a second, so it is
        // played, and read, in long notes.
        let beats = if i.name == "pad" { 3.0 } else { 1.0 };
        let read = play_and_read(i.name, &notes, beats);
        let worst = read.iter().map(|(_, c)| c.abs()).fold(0.0f64, f64::max);
        assert!(
            worst < 10.0,
            "{} is out of tune: {:?}",
            i.name,
            read.iter()
                .filter(|(_, c)| c.abs() >= 10.0)
                .collect::<Vec<_>>()
        );
        assert!(instruments::in_range(i, &note_name(lo).unwrap()));
        assert!(!instruments::in_range(i, &note_name(hi + 1).unwrap()));
    }
}

#[test]
fn the_drum_kit_plays_each_piece_at_its_time() {
    let song = load(
        r#"
version = 0
tempo = 120
bars = 2
[[tracks]]
name = "drums"
instrument = "drums"
notes = [
  { beat = 0, note = "kick" },
  { beat = 1, note = "hat" },
  { beat = 2, note = "snare" },
  { beat = 3, note = "hat", velocity = 0.5 },
  { beat = 4, note = "kick" },
  { beat = 5, note = "clap" },
  { beat = 6, note = "snare" },
  { beat = 7, note = ["kick", "hat"] },
]
"#,
    );
    let (_, mastering) = render_song(&song.song, &song.arrangement);
    let buf = render_song_tracks(&song.song, &song.arrangement, &[0], &mastering);
    let a = duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels);
    // Eight hits, half a second apart, each found within 15 ms.
    assert_eq!(a.onsets.len(), 8, "{:?}", a.onsets);
    for (k, onset) in a.onsets.iter().enumerate() {
        let at = k as f64 * 0.5;
        assert!(
            (onset.time_seconds - at).abs() < 0.015,
            "hit {k} at {}",
            onset.time_seconds
        );
    }
    // Each piece sounds like itself: the kick is the darkest, the hat the
    // brightest.
    let slice = |beat: usize| {
        let ch = buf.channels as usize;
        let from = beat * 22_050 * ch;
        let part = &buf.samples[from..from + 8_000 * ch];
        duck_echo_analyze::analyze(part, buf.sample_rate, buf.channels)
            .spectral_centroid_hz
            .unwrap()
    };
    let (kick, hat, snare, clap) = (slice(0), slice(1), slice(2), slice(5));
    assert!(kick < 400.0, "kick {kick}");
    assert!(hat > 7000.0, "hat {hat}");
    assert!(snare > 1500.0 && snare < hat, "snare {snare}");
    assert!(clap > 800.0 && clap < hat, "clap {clap}");
    // The quiet hat is quieter: velocity works on kit pieces.
    let level = |beat: usize| {
        let ch = buf.channels as usize;
        let from = beat * 22_050 * ch;
        buf.samples[from..from + 4_000 * ch]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert!(
        (level(3) / level(1) - 0.5).abs() < 0.1,
        "{} {}",
        level(3),
        level(1)
    );
}

#[test]
fn instruments_are_listed_and_named_like_presets() {
    let list = instruments::list().unwrap();
    let names: Vec<&str> = list.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "bass",
            "chip_lead",
            "clap",
            "hat",
            "kick",
            "pad",
            "piano",
            "snare",
            "drums"
        ]
    );
    let piano = &list[6];
    assert_eq!(piano.home.as_deref(), Some("C4"));
    assert_eq!(piano.range, Some(["C2".to_string(), "C7".to_string()]));
    assert_eq!(piano.target_lufs, Some(-18.0));
    let drums = &list[8];
    assert_eq!(drums.kind, "kit");
    assert_eq!(drums.pieces, ["clap", "hat", "kick", "snare"]);

    // `instrument:NAME` works wherever a recipe path does.
    let r = duck_echo_tools::load_recipe(Path::new("instrument:piano")).unwrap();
    assert_eq!(r.stem, "piano");
    // A typo is suggested, and a kit explains itself.
    let e = duck_echo_tools::load_recipe(Path::new("instrument:pianno"))
        .unwrap_err()
        .into_report();
    assert_eq!(e.errors[0].suggestion.as_deref(), Some("piano"));
    let e = duck_echo_tools::load_recipe(Path::new("instrument:drums"))
        .unwrap_err()
        .into_report();
    assert!(e.errors[0].message.contains("is a kit"));

    // In a song, a wrong name suggests the right one at the track.
    let files = files_next_to(Path::new("song.toml"));
    let report = load_song_str(
        "version = 0\ntempo = 100\n[[tracks]]\nname = \"t\"\ninstrument = \"pian\"\nnotes = [{ beat = 0, note = \"C4\" }]\n",
        Format::Toml,
        &Library::new(&files),
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "tracks[0].instrument");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("piano"));
    // A song's own instrument wins over a built-in one of the same name.
    let own = load(
        "version = 0\ntempo = 100\n[instruments.piano]\nversion = 0\n[[instruments.piano.layers]]\nsource = \"saw\"\n[[tracks]]\nname = \"t\"\ninstrument = \"piano\"\nnotes = [{ beat = 0, note = \"C4\" }]\n",
    );
    let duck_echo_core::Instrument::Sound(recipe) = &own.arrangement.tracks[0].instrument else {
        panic!("a sound");
    };
    assert_eq!(recipe.layers.len(), 1);
}
