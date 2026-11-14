//! Phase 2 step 2: the playtest-2 items for agents. The click check
//! (`ending`), a song's summary words, and the built-in guide.

use duck_echo_core::{load_str, Format};
use duck_echo_tools::measure::{measure_document, Options};
use duck_echo_tools::presets::{self, PRESETS};
use duck_echo_tools::song::files_next_to;
use duck_echo_tools::{analyze_buffer, load_document, render_buffer};
use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn every_preset_and_example_ends_cleanly() {
    for p in PRESETS {
        let recipe = presets::load(p).unwrap().recipe;
        let a = analyze_buffer(&render_buffer(&recipe).unwrap());
        assert!(a.ending.clean, "{}: {:?}", p.name, a.ending);
        assert!(
            a.summary.ends_with("ends cleanly") || a.summary.contains("ends cleanly, "),
            "{}: {}",
            p.name,
            a.summary
        );
    }
    for folder in ["recipes", "songs"] {
        for entry in std::fs::read_dir(examples().join(folder)).unwrap() {
            let path = entry.unwrap().path();
            let doc = load_document(&path, &files_next_to(&path)).unwrap();
            let m = measure_document(&doc, None, &Options::default(), "")
                .unwrap()
                .measured;
            // A loop never stops, so its join is what must be clean (the
            // last sample leads straight into the first).
            match &m.loop_seam {
                Some(seam) => assert!(seam.clean, "{}: {seam:?}", path.display()),
                None => assert!(
                    m.analysis.ending.clean,
                    "{}: {:?}",
                    path.display(),
                    m.analysis.ending
                ),
            }
        }
    }
}

#[test]
fn a_tone_cut_off_by_its_duration_ends_with_a_click() {
    // A held 220 Hz sine, cut while it is still at full level, a quarter
    // wave after 0.1 s, so the cut is at the top of a wave. (A cut at
    // exactly 0.1 s lands on a zero crossing and is clean. A square wave
    // would not count either: its own edges are as big as the cut.)
    let src = "version = 0\nduration = 0.10114\n\n[[layers]]\nsource = \"sine\"\nvolume = 0.5\nenvelope = { attack = 0.005, sustain = 1.0, decay = 0.1, release = 0.5 }\npitch = { start = \"A3\" }\n";
    let recipe = load_str(src, Format::Toml).unwrap().recipe;
    let a = analyze_buffer(&render_buffer(&recipe).unwrap());
    assert!(!a.ending.clean, "{:?}", a.ending);
    assert!(a.ending.jump > 0.1, "{:?}", a.ending);
    assert!(a.summary.contains("ENDS WITH A CLICK"), "{}", a.summary);
    // A release that finishes inside the length ends cleanly.
    let faded = src.replace("duration = 0.10114", "duration = 0.8").replace(
        "sustain = 1.0, decay = 0.1",
        "sustain = 1.0, decay = 0.1, hold = 0.05",
    );
    let recipe = load_str(&faded, Format::Toml).unwrap().recipe;
    let a = analyze_buffer(&render_buffer(&recipe).unwrap());
    assert!(a.ending.clean, "{:?}", a.ending);
}

#[test]
fn a_song_summary_uses_words_for_a_mix() {
    let path = examples().join("songs/duck_craft_day.toml");
    let doc = load_document(&path, &files_next_to(&path)).unwrap();
    let options = Options {
        check_notes: true,
        ..Options::default()
    };
    let s = measure_document(&doc, None, &options, "")
        .unwrap()
        .measured
        .analysis
        .summary;
    assert!(s.starts_with("8 bars looping at 84 BPM, 4 tracks; "), "{s}");
    assert!(
        s.contains(", -18.0 LUFS, no clipping, loop joins cleanly, "),
        "{s}"
    );
    assert!(s.contains(" notes heard; loudest track "), "{s}");
    assert!(s.contains(", quietest "), "{s}");
    // None of a sound effect's words.
    for word in ["Hz", "onsets", "attack", "bright", "tonal", "ends cleanly"] {
        assert!(!s.contains(word), "{word} in {s}");
    }
    // Without the tracks heard alone, there is nothing to rank them by.
    let plain = measure_document(&doc, None, &Options::default(), "")
        .unwrap()
        .measured
        .analysis
        .summary;
    assert!(
        plain.ends_with("loop joins cleanly; per_track describes each"),
        "{plain}"
    );
}

#[test]
fn the_guide_pages_are_the_repository_files() {
    let guide = duck_echo_tools::guide::page("guide", "topic").unwrap();
    let on_disk = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/AGENT_GUIDE.md"),
    )
    .unwrap();
    assert_eq!(guide, on_disk);
    // The guide sends agents to get_guide for the reference pages.
    assert!(guide.contains("get_guide"));
}
