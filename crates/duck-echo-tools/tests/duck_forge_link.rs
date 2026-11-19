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

// ---------------------------------------------------------------------
// Step 3: game_send and game_list.
// ---------------------------------------------------------------------

/// A Duck Echo project and a Duck Forge folder with one game, duck_craft.
fn project_and_game(name: &str) -> (PathBuf, PathBuf) {
    let dir = project(name);
    let forge = dir.join("forge");
    duck_forge_folder(&forge);
    std::fs::create_dir_all(forge.join("examples/duck_craft")).unwrap();
    std::fs::write(forge.join("examples/duck_craft/main.rs"), "fn main() {}\n").unwrap();
    let proj = dir.join("project");
    std::fs::create_dir_all(proj.join("sfx")).unwrap();
    std::fs::write(
        proj.join("sfx/step.toml"),
        duck_echo_tools::presets::get("footstep").unwrap().source,
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs/four_bars.toml"),
        proj.join("tune.toml"),
    )
    .unwrap();
    (proj, forge)
}

/// Run a named command with the Duck Forge folder set, as the MCP server
/// started with --duck-forge does.
fn run_with_forge(
    proj: &Path,
    forge: &Path,
    name: &str,
    args: Value,
) -> Result<Value, duck_echo_core::Report> {
    let root = duck_echo_tools::root::Root::new(proj)
        .unwrap()
        .with_duck_forge(duck_echo_tools::forge::DuckForge::open(forge).unwrap());
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

/// Every file under `dir`, with its bytes and modified time.
fn files_in(dir: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let modified = std::fs::metadata(&p).unwrap().modified().unwrap();
                out.push((rel, std::fs::read(&p).unwrap(), modified));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn facts(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_sound_a_song_and_variants_go_into_a_game_and_sending_again_writes_nothing() {
    let (proj, forge) = project_and_game("send");
    let game = forge.join("examples/duck_craft");
    let send = |args: Value| run_with_forge(&proj, &forge, "game_send", args).unwrap();

    // A sound: WAV, its facts, the recipe copy.
    let a = send(json!({"file": "sfx/step.toml", "game": "duck_craft"}));
    assert_eq!(a["kind"], "sound");
    assert_eq!(a["format"], "wav");
    assert_eq!(a["unchanged"], false);
    assert_eq!(
        a["written"],
        json!(["assets/step.wav", "assets/step.wav.json", "art/step.toml"])
    );
    let f = facts(&game.join("assets/step.wav.json"));
    assert_eq!(f["recipe"], "art/step.toml");
    let sha = duck_echo_tools::asset::file_sha256(&game.join("assets/step.wav")).unwrap();
    assert_eq!(f["file_sha256"], sha.as_str());
    assert_eq!(
        std::fs::read_to_string(game.join("art/step.toml")).unwrap(),
        std::fs::read_to_string(proj.join("sfx/step.toml")).unwrap()
    );

    // A song: OGG by default.
    let s = send(json!({"file": "tune.toml", "game": "duck_craft", "name": "theme"}));
    assert_eq!(
        (s["kind"].as_str(), s["format"].as_str()),
        (Some("song"), Some("ogg"))
    );
    assert_eq!(s["audio"], "assets/theme.ogg");
    assert_eq!(facts(&game.join("assets/theme.ogg.json"))["format"], "ogg");

    // Variants: numbered WAVs and the list, each facts file naming the recipe.
    let v = send(json!({"file": "sfx/step.toml", "game": "duck_craft", "variants": 3}));
    assert_eq!(v["list"], "assets/step.variants.json");
    assert_eq!(
        facts(&game.join("assets/step_2.wav.json"))["recipe"],
        "art/step.toml"
    );
    assert_eq!(
        facts(&game.join("assets/step.variants.json"))["recipe"],
        "art/step.toml"
    );

    // Sending all three again writes nothing at all: same bytes, same times.
    let before = files_in(&game);
    for args in [
        json!({"file": "sfx/step.toml", "game": "duck_craft"}),
        json!({"file": "tune.toml", "game": "duck_craft", "name": "theme"}),
        json!({"file": "sfx/step.toml", "game": "duck_craft", "variants": 3}),
    ] {
        let again = send(args);
        assert_eq!(again["unchanged"], true, "{again}");
        assert_eq!(again["written"], json!([]), "{again}");
        assert!(again["summary"]
            .as_str()
            .unwrap()
            .contains("nothing written"));
    }
    assert_eq!(files_in(&game), before);

    // game_list sees all three.
    let list = run_with_forge(&proj, &forge, "game_list", json!({})).unwrap();
    let sounds = &list["games"][0]["sounds"];
    let names: Vec<(&str, &str)> = sounds
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s["file"].as_str().unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            ("step", "assets/step.variants.json"),
            ("step", "assets/step.wav"),
            ("theme", "assets/theme.ogg")
        ]
    );
    assert_eq!(sounds[0]["variants"], 3);
    assert_eq!(sounds[2]["loops"], json!(false));
    // A song says its shape, read from its recipe copy, so a theme the
    // game fades to can match it (playtest 4 had to guess the tempo). A
    // sound has none.
    // (tune.toml is examples/songs/four_bars.toml: tempo = 110, bars = 4,
    // and beats_per_bar left at its default, 4.)
    assert_eq!(
        sounds[2]["song"],
        json!({"tempo": 110.0, "beats_per_bar": 4, "bars": 4})
    );
    assert!(sounds[0].get("song").is_none(), "{}", sounds[0]);
    assert!(sounds[1].get("song").is_none(), "{}", sounds[1]);
}

#[test]
fn a_game_folder_from_another_computer_with_the_same_recipe_is_unchanged() {
    // An OGG's bytes differ from one computer to another (CI showed it).
    // Here the game folder holds the theme as "Windows" made it: different
    // OGG bytes, its facts written there (with the SHA-256 of those
    // bytes), and the text files checked out by Git with \r\n line endings.
    // Sent again from this computer with the same recipe, nothing changes.
    let (proj, forge) = project_and_game("other-computer");
    let game = forge.join("examples/duck_craft");
    let send = |args: Value| run_with_forge(&proj, &forge, "game_send", args).unwrap();
    send(json!({"file": "tune.toml", "game": "duck_craft", "name": "theme"}));

    // "Windows" bytes: the same music, encoded with another stream serial,
    // so a valid OGG that differs from this computer's.
    let ogg = game.join("assets/theme.ogg");
    let here = std::fs::read(&ogg).unwrap();
    let decoded = duck_echo_audio::ogg::decode_ogg(&here).unwrap();
    let elsewhere = duck_echo_audio::ogg::encode_ogg(
        &decoded.buffer,
        &duck_echo_audio::ogg::OggSettings {
            quality: 4.0,
            serial: 12345,
            tags: Vec::new(),
        },
    )
    .unwrap();
    assert_ne!(here, elsewhere);
    std::fs::write(&ogg, &elsewhere).unwrap();
    let facts_path = game.join("assets/theme.ogg.json");
    let mut f = facts(&facts_path);
    f["file_sha256"] = json!(format!(
        "sha256:{}",
        duck_echo_tools::asset::sha256_hex(&elsewhere)
    ));
    let crlf = |text: String| text.replace('\n', "\r\n");
    std::fs::write(&facts_path, crlf(serde_json::to_string_pretty(&f).unwrap())).unwrap();
    let recipe = game.join("art/theme.toml");
    std::fs::write(&recipe, crlf(std::fs::read_to_string(&recipe).unwrap())).unwrap();

    let before = files_in(&game);
    let again = send(json!({"file": "tune.toml", "game": "duck_craft", "name": "theme"}));
    assert_eq!(again["unchanged"], true, "{again}");
    assert_eq!(again["written"], json!([]));
    assert_eq!(
        files_in(&game),
        before,
        "the other computer's files are kept as they are"
    );

    // But a sound changed by hand (its facts no longer match it) is sent
    // again.
    std::fs::write(&ogg, b"not the sound the facts describe").unwrap();
    let fixed = send(json!({"file": "tune.toml", "game": "duck_craft", "name": "theme"}));
    assert_eq!(fixed["unchanged"], false);
    assert_eq!(&std::fs::read(&ogg).unwrap()[..4], b"OggS");
}

#[test]
fn a_changed_sound_is_sent_again_and_a_changed_comment_only_rewrites_the_recipe() {
    let (proj, forge) = project_and_game("changes");
    let game = forge.join("examples/duck_craft");
    let send = || {
        run_with_forge(
            &proj,
            &forge,
            "game_send",
            json!({"file": "sfx/step.toml", "game": "duck_craft"}),
        )
        .unwrap()
    };
    send();
    let wav = std::fs::read(game.join("assets/step.wav")).unwrap();

    // A comment: the sound is the same, only the recipe copy is written.
    let path = proj.join("sfx/step.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("# louder later\n{text}")).unwrap();
    let a = send();
    assert_eq!(a["unchanged"], true);
    assert_eq!(a["written"], json!(["art/step.toml"]));
    assert!(std::fs::read_to_string(game.join("art/step.toml"))
        .unwrap()
        .starts_with("# louder later"));

    // A new seed changes the sound: everything is written again.
    let reseeded: String = text
        .lines()
        .map(|l| {
            if l.starts_with("seed =") {
                "seed = 99"
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(reseeded, text.trim_end());
    std::fs::write(&path, reseeded).unwrap();
    let b = send();
    assert_eq!(b["unchanged"], false);
    assert_eq!(
        b["written"],
        json!(["assets/step.wav", "assets/step.wav.json", "art/step.toml"])
    );
    assert_ne!(std::fs::read(game.join("assets/step.wav")).unwrap(), wav);
}

#[test]
fn game_send_writes_only_where_it_may() {
    let (proj, forge) = project_and_game("refusals");
    let paths = |r: duck_echo_core::Report| -> Vec<String> {
        r.errors.iter().map(|e| e.path.clone()).collect()
    };
    let fail = |args: Value| paths(run_with_forge(&proj, &forge, "game_send", args).unwrap_err());
    // The game is a name, never a path.
    assert_eq!(
        fail(json!({"file": "sfx/step.toml", "game": "../duck_craft"})),
        ["arguments.game"]
    );
    assert_eq!(
        fail(json!({"file": "sfx/step.toml", "game": "duck_crft"})),
        ["arguments.game"]
    );
    // The file is a file of the project.
    assert_eq!(
        fail(json!({"file": "../step.toml", "game": "duck_craft"})),
        ["arguments.file"]
    );
    assert_eq!(
        fail(json!({"file": "instrument:piano", "game": "duck_craft"})),
        ["arguments.file"]
    );
    // Settings that do not fit.
    assert_eq!(
        fail(json!({"file": "sfx/step.toml", "game": "duck_craft", "format": "mp3"})),
        ["arguments.format"]
    );
    assert_eq!(
        fail(json!({"file": "sfx/step.toml", "game": "duck_craft", "quality": 5})),
        ["arguments.quality"]
    );
    assert_eq!(
        fail(json!({"file": "tune.toml", "game": "duck_craft", "variants": 3})),
        ["arguments.variants"]
    );
    assert_eq!(
        fail(json!({"file": "sfx/step.toml", "game": "duck_craft", "variants": 0})),
        ["arguments.variants"]
    );
    // A name that is not plain is refused, not changed: the error names
    // the argument, says what is allowed and suggests a safe name.
    for (bad, safe) in [
        ("../../evil", "evil"),
        ("..\\evil", "evil"),
        ("sfx/step", "sfx_step"),
        ("my step", "my_step"),
        ("step.v2", "step_v2"),
        ("..", "sound"),
    ] {
        let r = run_with_forge(
            &proj,
            &forge,
            "game_send",
            json!({"file": "sfx/step.toml", "game": "duck_craft", "name": bad}),
        )
        .unwrap_err();
        let e = &r.errors[0];
        assert_eq!(e.path, "arguments.name", "{bad}: {r}");
        assert!(
            e.message.contains("letters, digits, `-` and `_`"),
            "{bad}: {}",
            e.message
        );
        assert_eq!(e.suggestion.as_deref(), Some(safe), "{bad}");
    }
    let long = "a".repeat(65);
    let r = run_with_forge(
        &proj,
        &forge,
        "game_send",
        json!({"file": "sfx/step.toml", "game": "duck_craft", "name": long}),
    )
    .unwrap_err();
    assert_eq!(r.errors[0].path, "arguments.name");
    let game = forge.join("examples/duck_craft");
    assert!(
        !game.join("assets").exists() && !game.join("art").exists(),
        "nothing was written"
    );
    // A plain name is used as it is.
    let a = run_with_forge(
        &proj,
        &forge,
        "game_send",
        json!({"file": "sfx/step.toml", "game": "duck_craft", "name": "Step-2_b"}),
    )
    .unwrap();
    assert_eq!(a["audio"], "assets/Step-2_b.wav");
    // With no Duck Forge folder, it says how to set one.
    let r = run_in(
        &proj,
        "game_send",
        json!({"file": "sfx/step.toml", "game": "duck_craft"}),
    )
    .unwrap_err();
    assert_eq!(r.errors[0].path, "arguments.game");
    assert!(r.errors[0].message.contains("--duck-forge"), "{r}");
    // A folder that is not Duck Forge's is not taken.
    assert!(duck_echo_tools::forge::DuckForge::open(&proj).is_err());
}

#[cfg(unix)]
#[test]
fn a_linked_assets_folder_is_refused() {
    let (proj, forge) = project_and_game("link");
    let outside = proj.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, forge.join("examples/duck_craft/assets")).unwrap();
    let r = run_with_forge(
        &proj,
        &forge,
        "game_send",
        json!({"file": "sfx/step.toml", "game": "duck_craft"}),
    )
    .unwrap_err();
    assert_eq!(r.errors[0].code, duck_echo_core::Code::PathRefused, "{r}");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

// ---------------------------------------------------------------------
// Step 9: a game's log in the project, the Duck Forge folder beside it.
// ---------------------------------------------------------------------

#[test]
fn mix_check_reads_a_games_sounds_from_the_duck_forge_folder_outside_the_project() {
    // The project and the Duck Forge folder side by side, as with the MCP
    // server's --duck-forge: the game's headless log written into the
    // project (--out), its sounds in the game's folder.
    let (proj, forge) = project_and_game("mix-outside");
    run_with_forge(
        &proj,
        &forge,
        "game_send",
        json!({"file": "sfx/step.toml", "game": "duck_craft"}),
    )
    .unwrap();
    let log = json!({
        "game": "duck_craft",
        "updates_per_second": 60,
        "frames": 120,
        "sounds": [
            {"frame": 10, "sound": "step", "file": "examples/duck_craft/assets/step.wav"},
            {"frame": 40, "sound": "step", "file": "examples/duck_craft/assets/step.wav", "volume": 0.5},
            {"frame": 50, "sound": "beep", "wave": "square", "freq": 440, "ms": 100}
        ],
        "loops": []
    });
    std::fs::create_dir_all(proj.join("out/duck_craft")).unwrap();
    std::fs::write(proj.join("out/duck_craft/log.json"), log.to_string()).unwrap();

    let answer = run_with_forge(
        &proj,
        &forge,
        "mix_check",
        json!({"cues": "out/duck_craft/log.json"}),
    )
    .unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(answer["cues"].as_array().map(Vec::len), Some(2), "{answer}");
    assert_eq!(answer["not_mixed"][0]["name"], "beep", "{answer}");
    let (left, _) = mix_of(&proj, &answer);
    // The first step starts on frame 10: sample 9 x 735.
    assert!(left[..9 * 735].iter().all(|&s| s == 0.0));
    assert!(left[9 * 735..9 * 735 + 2000].iter().any(|&s| s != 0.0));

    // Without the Duck Forge folder the game's files can't be found, and
    // the error says which.
    let without = run_in(
        &proj,
        "mix_check",
        json!({"cues": "out/duck_craft/log.json"}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        without.contains("examples/duck_craft/assets/step.wav"),
        "{without}"
    );

    // A log can't reach outside the Duck Forge folder.
    let sneaky = json!({
        "updates_per_second": 60,
        "sounds": [{"frame": 1, "sound": "x", "file": "../project/sfx/step.toml"}],
        "loops": []
    });
    std::fs::write(proj.join("out/sneaky.json"), sneaky.to_string()).unwrap();
    let refused = run_with_forge(
        &proj,
        &forge,
        "mix_check",
        json!({"cues": "out/sneaky.json"}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        refused.contains("not a plain path inside the Duck Forge folder"),
        "{refused}"
    );
}
