//! Phase 3, the link with Duck Forge (docs/phase3-design.md).
//!
//! Step 1: `mix_check` reads Duck Forge's `log.json` as a real headless run
//! writes it, and places, pans and turns up its sounds as the game does.
//! `tests/data/duck_craft_log.json` was written by Duck Forge (`main`,
//! commit 9ebb6ca) replaying Duck Craft's playthrough headless; it has 21
//! synthesizer sounds and 5 WAV loops.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// An empty project folder for a test.
fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-forge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run a named command in `dir`, as an agent does.
fn run_in(dir: &Path, name: &str, args: Value) -> Result<Value, duck_echo_core::Report> {
    let root = duck_echo_tools::root::Root::new(dir).unwrap();
    let project = duck_echo_tools::project::Project::new(&root);
    duck_echo_tools::commands::parse(name, &args, "arguments.")
        .and_then(|c| {
            duck_echo_tools::commands::run(
                &project,
                c,
                duck_echo_tools::history::Who::Agent,
                None,
                "arguments.",
            )
        })
        .map_err(|e| e.into_report())
}

/// A WAV at 44.1 kHz holding `level` (mono) for `frames` samples.
fn flat_wav(path: &Path, level: f32, frames: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let buffer = duck_echo_audio::AudioBuffer {
        sample_rate: 44_100,
        channels: 1,
        samples: vec![level; frames],
    };
    duck_echo_audio::write_wav(path, &buffer, duck_echo_audio::WavFormat::Float32).unwrap();
}

/// A Duck Forge folder: its `Cargo.toml` names the package `duckforge`.
fn duck_forge_folder(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"duckforge\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
}

/// The mix a check wrote, as (left, right) samples.
fn mix_of(dir: &Path, answer: &Value) -> (Vec<f32>, Vec<f32>) {
    let wav = dir.join(answer["mix_wav"].as_str().unwrap());
    let mix = duck_echo_audio::read_wav(&wav).unwrap();
    assert_eq!((mix.sample_rate, mix.channels), (44_100, 2));
    let left = mix.samples.iter().step_by(2).copied().collect();
    let right = mix.samples.iter().skip(1).step_by(2).copied().collect();
    (left, right)
}

#[test]
fn a_real_duck_forge_log_is_read_as_it_is() {
    let dir = project("real-log");
    duck_forge_folder(&dir);
    let log = include_str!("data/duck_craft_log.json");
    std::fs::create_dir_all(dir.join("out/duck_craft")).unwrap();
    std::fs::write(dir.join("out/duck_craft/log.json"), log).unwrap();
    // Short stand-ins for the five loops' WAVs, where the game named them
    // (from the Duck Forge folder it ran in).
    for name in [
        "music_title",
        "music_day",
        "music_night",
        "hum",
        "heartbeat",
    ] {
        let file = format!("examples/duck_craft/assets/{name}.wav");
        flat_wav(&dir.join(file), 0.1, 4410);
    }

    let a = run_in(
        &dir,
        "mix_check",
        json!({"cues": "out/duck_craft/log.json"}),
    )
    .unwrap();
    assert_eq!(a["form"], "duck_forge_log");
    // 797 frames at 60 a second: the game ran 797 x 735 samples.
    let length = a["length_seconds"].as_f64().unwrap();
    assert!((length - 797.0 * 735.0 / 44_100.0).abs() < 1e-9, "{length}");
    // The loops are mixed; the synthesizer sounds are named, not mixed.
    let cues = a["cues"].as_array().unwrap();
    assert_eq!(cues.len(), 5);
    assert!(cues.iter().all(|c| c["looping"] == true));
    let not_mixed = a["not_mixed"].as_array().unwrap();
    assert_eq!(not_mixed.len(), 21);
    assert_eq!(not_mixed[0]["name"], "click");
    // Frame 31 starts 30 frames (0.5 s) in.
    assert_eq!(not_mixed[0]["start_seconds"], 0.5);
    let summary = a["summary"].as_str().unwrap();
    assert!(
        summary.contains(
            "not in the mix: 21 of Duck Forge's own synthesizer sounds (click, dig, break, place"
        ),
        "{summary}"
    );
    let (left, _) = mix_of(&dir, &a);
    assert_eq!(left.len(), 797 * 735);
}

#[test]
fn sounds_land_on_their_frame_and_pan_as_in_the_game() {
    let dir = project("placing");
    duck_forge_folder(&dir);
    flat_wav(&dir.join("assets/a.wav"), 0.5, 1000);
    let log = json!({
        "game": "test", "updates_per_second": 60, "frames": 180,
        "sounds": [
            {"frame": 61, "sound": "middle", "file": "assets/a.wav"},
            {"frame": 121, "sound": "left", "file": "assets/a.wav", "pan": -1, "volume": 0.5}
        ],
        "loops": []
    });
    std::fs::write(dir.join("log.json"), log.to_string()).unwrap();
    let a = run_in(&dir, "mix_check", json!({"cues": "log.json"})).unwrap();
    let (left, right) = mix_of(&dir, &a);
    // Frame 61 starts at sample 60 x 735 = 44100: exactly one second.
    let first = left.iter().position(|s| *s != 0.0).unwrap();
    assert_eq!(first, 44_100);
    // Centred, it is at full volume on both sides, as Duck Forge plays it.
    assert_eq!((left[44_100], right[44_100]), (0.5, 0.5));
    // Panned hard left at half volume: left only.
    assert_eq!((left[88_200], right[88_200]), (0.25, 0.0));

    // A plain list pans with equal power: -3 dB each side in the middle.
    let list = json!([{"time": 1.0, "file": "assets/a.wav"}]);
    std::fs::write(dir.join("list.json"), list.to_string()).unwrap();
    let a = run_in(&dir, "mix_check", json!({"cues": "list.json"})).unwrap();
    let (left, right) = mix_of(&dir, &a);
    assert!((left[44_100] - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    assert_eq!(left[44_100], right[44_100]);
}

#[test]
fn a_loops_volume_holds_until_its_next_change_as_in_the_game() {
    let dir = project("steps");
    duck_forge_folder(&dir);
    flat_wav(&dir.join("assets/bed.wav"), 0.4, 2000);
    let log = json!({
        "updates_per_second": 60, "frames": 120, "sounds": [],
        "loops": [{"loop": "bed", "file": "assets/bed.wav", "start": 1, "stop": null,
                   "volume": [[1, 1], [31, 0.5], [61, 0]]}]
    });
    std::fs::write(dir.join("log.json"), log.to_string()).unwrap();
    let a = run_in(&dir, "mix_check", json!({"cues": "log.json"})).unwrap();
    let (left, _) = mix_of(&dir, &a);
    // Frame 31 starts at 30 x 735 = 22050; frame 61 at 44100. No glide
    // between them: Duck Forge changes a loop's volume on the frame.
    assert_eq!(left[22_049], 0.4);
    assert_eq!(left[22_050], 0.2);
    assert_eq!(left[44_099], 0.2);
    assert_eq!(left[44_100], 0.0);
    // The loop goes round with no gap (2000 samples long).
    assert_eq!(left[1999], left[2000]);
    assert_eq!(left.len(), 120 * 735);
}

#[test]
fn files_are_named_from_the_duck_forge_folder_or_from_base() {
    let dir = project("base");
    // The Duck Forge folder is a folder of the project; the log is not in it.
    duck_forge_folder(&dir.join("forge"));
    flat_wav(&dir.join("forge/examples/g/assets/a.wav"), 0.3, 500);
    let log = json!({"updates_per_second": 60, "frames": 60,
        "sounds": [{"frame": 1, "sound": "a", "file": "examples/g/assets/a.wav"}]});
    std::fs::create_dir_all(dir.join("runs")).unwrap();
    std::fs::write(dir.join("runs/log.json"), log.to_string()).unwrap();
    // Not under the Duck Forge folder: the file is not found, and the
    // error names the line.
    let r = run_in(&dir, "mix_check", json!({"cues": "runs/log.json"})).unwrap_err();
    assert_eq!(r.errors[0].path, "sounds[0].file", "{r}");
    // With base, it is.
    let a = run_in(
        &dir,
        "mix_check",
        json!({"cues": "runs/log.json", "base": "forge"}),
    )
    .unwrap();
    assert_eq!(a["cues"][0]["name"], "a");
    // Under the Duck Forge folder, it is found without base.
    std::fs::create_dir_all(dir.join("forge/out")).unwrap();
    std::fs::write(dir.join("forge/out/log.json"), log.to_string()).unwrap();
    assert!(run_in(&dir, "mix_check", json!({"cues": "forge/out/log.json"})).is_ok());
    // base must be a folder inside the project.
    let r = run_in(
        &dir,
        "mix_check",
        json!({"cues": "runs/log.json", "base": "../x"}),
    )
    .unwrap_err();
    assert_eq!(r.errors[0].path, "arguments.base", "{r}");
}

#[test]
fn a_synthesizer_line_with_two_volumes_is_read() {
    // Duck Forge writes a synthesizer sound's own volume, and the play's
    // volume after it, on one line.
    let dir = project("two-volumes");
    let text = r#"{"updates_per_second": 60, "frames": 10, "sounds": [
        {"frame": 2, "sound": "zap", "wave": "square", "freq": 900, "volume": 0.3, "attack": 2, "decay": 40, "volume": 0.5}
    ], "loops": []}"#;
    std::fs::write(dir.join("log.json"), text).unwrap();
    let a = run_in(&dir, "mix_check", json!({"cues": "log.json"})).unwrap();
    assert_eq!(a["not_mixed"][0]["name"], "zap");
    assert!(a["cues"].as_array().unwrap().is_empty());
}
