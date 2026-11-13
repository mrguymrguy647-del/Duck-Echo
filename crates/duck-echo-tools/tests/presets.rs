//! Roadmap task 7: every preset follows the conventions and renders clean;
//! presets and variations behave as documented.

use duck_echo_audio::{encode_wav, WavFormat};
use duck_echo_tools::presets::{self, CATEGORIES, PRESETS};
use duck_echo_tools::{load_recipe, variations, ToolError};
use std::path::{Path, PathBuf};

fn presets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn embedded_list_matches_the_presets_folder() {
    let mut on_disk: Vec<String> = std::fs::read_dir(presets_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let embedded: Vec<String> = PRESETS.iter().map(|p| p.name.to_string()).collect();
    assert_eq!(
        embedded, on_disk,
        "add new presets to the presets! list in duck-echo-tools/src/presets.rs"
    );
    assert!((20..=30).contains(&PRESETS.len()));
}

#[test]
fn every_preset_follows_the_conventions_and_renders_clean() {
    for p in PRESETS {
        let loaded = presets::load(p).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        let r = &loaded.recipe;
        assert!(
            loaded.report.warnings.is_empty(),
            "{}: {}",
            p.name,
            loaded.report
        );
        assert_eq!(
            r.name.as_deref(),
            Some(p.name),
            "name must equal the file name"
        );
        assert!(
            r.description.as_deref().is_some_and(|d| !d.is_empty()),
            "{}",
            p.name
        );
        let cats: Vec<&String> = r
            .tags
            .iter()
            .filter(|t| CATEGORIES.contains(&t.as_str()))
            .collect();
        assert_eq!(cats.len(), 1, "{}: needs exactly one category tag", p.name);
        assert_eq!(&r.tags[0], cats[0], "{}: category tag goes first", p.name);
        assert!(r.master.normalize, "{}", p.name);
        assert!(
            r.master.normalize_peak_db <= -1.0,
            "{}: keep 1 dB of peak headroom",
            p.name
        );
        let target = presets::target_lufs(cats[0]).expect("every category has a target");
        assert!(r.master.trim_silence, "{}", p.name);

        let buf = duck_echo_synth::render(r).unwrap();
        let a = duck_echo_tools::analyze_buffer(&buf);
        assert!(
            !a.is_silent && !a.clipping.clipped,
            "{}: {}",
            p.name,
            a.summary
        );
        assert!(
            (a.peak_dbfs.unwrap() - r.master.normalize_peak_db).abs() < 0.01,
            "{}: peak {:?}",
            p.name,
            a.peak_dbfs
        );
        let lufs = a.loudness_lufs.unwrap();
        assert!(
            (lufs - target).abs() <= 0.25,
            "{}: {lufs:.2} LUFS, target {target} for {}; set normalize_peak_db to about {:.1}",
            p.name,
            cats[0],
            r.master.normalize_peak_db + (target - lufs)
        );
        assert_eq!(a.trailing_silence_seconds, 0.0, "{}", p.name);
        assert!(a.leading_silence_seconds < 0.01, "{}", p.name);
        assert!(a.loudness_lufs.is_some(), "{}", p.name);
        assert!(buf.samples.iter().all(|s| s.is_finite()));

        // Same recipe twice, and through JSON, gives the same audio.
        let again = duck_echo_synth::render(r).unwrap();
        assert_eq!(buf.samples, again.samples, "{}", p.name);
        let via_json =
            duck_echo_core::parse_str(&duck_echo_core::to_json(r), duck_echo_core::Format::Json)
                .unwrap();
        assert_eq!(&via_json, r, "{}", p.name);
    }
}

/// PCM16 WAV size and fingerprint of every preset, rendered by a release
/// build (this test runs unoptimized, so it also checks that the
/// optimization level does not change the bytes). All were re-pinned when
/// presets moved from -3 dBFS peak to per-category loudness targets
/// (R10); before that each was pinned at its first render. The 14 presets
/// with a square or saw layer were re-pinned again when `aliasing = "clean"`
/// became the default (roadmap task 3); the other 16 did not change. Update
/// a value only when the preset is changed on purpose, and say so in the
/// commit.
const PRESET_GOLDEN: &[(&str, usize, u64)] = &[
    ("alarm", 93066, 0x0ca6_1495_8708_b1fa),
    ("big_explosion", 189320, 0xc190_f382_0999_f029),
    ("blaster", 12360, 0x06ae_a6b5_039d_4de5),
    ("charge_up", 74990, 0x1517_8f35_3980_306a),
    ("coin", 27420, 0xf25b_53ed_b3ae_17ec),
    ("engine_hum", 220326, 0xfa5c_e8a5_b63c_2604),
    ("explosion", 98486, 0xab7d_2062_4f63_4afd),
    ("extra_life", 53024, 0x2d4d_6eb4_bec9_4d0b),
    ("footstep", 4940, 0x917d_12c3_80e0_c738),
    ("gem", 50998, 0x7819_da53_7e4f_1e41),
    ("hit", 10564, 0xac5f_0a18_1ee9_7af1),
    ("jump", 24238, 0x4ace_8edb_fdeb_c392),
    ("key_pickup", 56158, 0xea77_66b7_9245_c852),
    ("land", 7156, 0x6ad6_bef9_f9ff_b352),
    ("laser", 57972, 0xc2db_dd4c_9152_94ba),
    ("low_health", 29442, 0x3f90_88db_4603_f3a4),
    ("menu_open", 13232, 0xa19f_55e3_01a3_b1db),
    ("metal_clang", 105586, 0xaa10_b2f1_b330_698e),
    ("missile", 61692, 0xa1c5_97a8_cb77_1b17),
    ("powerup", 91470, 0x58b1_aa7e_7f3e_ca82),
    ("punch", 10630, 0xcffa_739a_3ff8_c656),
    ("shotgun", 39360, 0xf399_2caa_cc94_e9c8),
    ("thud", 17790, 0xa484_4080_8f8d_20b0),
    ("ui_cancel", 20340, 0x66b4_0e13_6628_2b15),
    ("ui_click", 3198, 0x6c24_b85f_d24e_1de9),
    ("ui_confirm", 30682, 0x903a_f3ba_c1c9_ddd5),
    ("ui_error", 33696, 0x2313_3fcb_ab75_d57e),
    ("ui_hover", 5734, 0xe4f9_fd7d_bfcb_009b),
    ("warning_beep", 42634, 0xee89_b207_bd72_582d),
    ("wind", 560624, 0xcc48_19ca_0fd4_7b14),
];

#[test]
fn preset_renders_are_pinned() {
    assert_eq!(PRESET_GOLDEN.len(), PRESETS.len(), "pin every preset");
    for (name, len, hash) in PRESET_GOLDEN {
        let recipe = presets::load(presets::get(name).unwrap()).unwrap().recipe;
        let wav = encode_wav(&duck_echo_synth::render(&recipe).unwrap(), WavFormat::Pcm16).unwrap();
        assert_eq!((wav.len(), fnv1a64(&wav)), (*len, *hash), "{name} changed");
    }
}

#[test]
fn loudness_targets_cover_every_category_in_order() {
    let cats: Vec<&str> = presets::LOUDNESS_TARGETS.iter().map(|(c, _)| *c).collect();
    for c in CATEGORIES {
        assert!(cats.contains(c), "{c} has no loudness target");
    }
    assert!(presets::LOUDNESS_TARGETS
        .windows(2)
        .all(|w| w[0].1 > w[1].1));
    let coin = presets::info(presets::get("coin").unwrap()).unwrap();
    assert_eq!(coin.target_lufs, Some(-19.0));
}

#[test]
fn every_category_has_presets_and_no_two_sound_the_same() {
    for cat in CATEGORIES {
        assert!(
            !presets::list(Some(cat)).unwrap().is_empty(),
            "no {cat} preset"
        );
    }
    let mut seen = std::collections::HashMap::new();
    for (name, _, hash) in PRESET_GOLDEN {
        if let Some(other) = seen.insert(hash, name) {
            panic!("{name} renders the same as {other}");
        }
    }
    let ui = presets::list(Some("ui")).unwrap();
    assert!(ui.iter().all(|p| p.category == "ui"));
    assert_eq!(presets::list(None).unwrap().len(), PRESETS.len());
}

#[test]
fn preset_references_and_typos() {
    let r = load_recipe(Path::new("preset:coin")).unwrap();
    assert_eq!(r.stem, "coin");
    assert_eq!(r.loaded.recipe.name.as_deref(), Some("coin"));
    match load_recipe(Path::new("preset:gemm")) {
        Err(ToolError::Failed(issue)) => {
            assert_eq!(issue.code, duck_echo_core::Code::NotFound);
            assert_eq!(issue.suggestion.as_deref(), Some("gem"));
            // Front ends list presets differently, so the shared message
            // names no command; `allowed` has every name instead.
            assert!(!issue.message.contains("duck-echo"), "{}", issue.message);
            match issue.allowed {
                Some(duck_echo_core::Allowed::OneOf { options }) => {
                    assert_eq!(options.len(), PRESETS.len())
                }
                other => panic!("{other:?}"),
            }
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn variations_render_distinct_seeds_and_explain_when_they_cannot() {
    let coin = load_recipe(Path::new("preset:coin")).unwrap();
    let same = variations(&coin, 3, None, None).unwrap();
    assert!(!same.seeds_matter && same.note.is_some());
    assert_eq!(same.renders.len(), 3);
    assert_eq!(same.renders[0].analysis, same.renders[2].analysis);

    let varied = variations(&coin, 4, Some(0.5), None).unwrap();
    assert!(varied.seeds_matter && varied.note.is_none());
    let seeds: Vec<u64> = varied.renders.iter().map(|r| r.seed).collect();
    assert_eq!(seeds, [1, 2, 3, 4]);
    let pitches: Vec<f32> = varied
        .renders
        .iter()
        .map(|r| r.analysis.pitch_track[0].dominant_hz.unwrap())
        .collect();
    for i in 0..pitches.len() {
        for j in i + 1..pitches.len() {
            assert!((pitches[i] - pitches[j]).abs() > 0.5, "{pitches:?}");
        }
    }

    // Noise makes seeds matter even with no variation.
    let foot = load_recipe(Path::new("preset:footstep")).unwrap();
    assert!(variations(&foot, 2, Some(0.0), None).unwrap().seeds_matter);

    // Written files, and errors for bad arguments.
    let dir = std::env::temp_dir().join("duck_echo_variations_test");
    let _ = std::fs::remove_dir_all(&dir);
    let written = variations(&foot, 2, None, Some(&dir)).unwrap();
    for r in &written.renders {
        assert!(Path::new(r.output.as_ref().unwrap()).exists());
    }
    assert!(dir.join("footstep_seed2.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
    for bad in [0, 65] {
        match variations(&coin, bad, None, None) {
            Err(ToolError::Failed(i)) => {
                assert_eq!(i.code, duck_echo_core::Code::OutOfRange);
                assert_eq!(i.path, "count");
            }
            other => panic!("{other:?}"),
        }
    }
    match variations(&coin, 2, Some(1.5), None) {
        Err(ToolError::Invalid(r)) => assert_eq!(r.errors[0].path, "variation"),
        other => panic!("{other:?}"),
    }
}

fn character(name: &str) -> String {
    let recipe = presets::load(presets::get(name).unwrap()).unwrap().recipe;
    let a = duck_echo_tools::analyze_buffer(&duck_echo_synth::render(&recipe).unwrap());
    for word in ["tone and noise", "tonal", "noisy"] {
        if a.summary.split(", ").any(|part| part.starts_with(word)) {
            return word.to_string();
        }
    }
    panic!("{name}: no character word in {}", a.summary);
}

#[test]
fn summary_words_match_what_the_presets_are() {
    for name in [
        "shotgun",
        "explosion",
        "big_explosion",
        "missile",
        "wind",
        "footstep",
        "blaster",
    ] {
        let c = character(name);
        assert!(c == "noisy" || c == "tone and noise", "{name} reads {c}");
    }
    for name in [
        "alarm",
        "charge_up",
        "coin",
        "engine_hum",
        "extra_life",
        "gem",
        "jump",
        "key_pickup",
        "low_health",
        "menu_open",
        "metal_clang",
        "powerup",
        "ui_cancel",
        "ui_confirm",
        "ui_error",
        "ui_hover",
        "warning_beep",
    ] {
        assert_eq!(character(name), "tonal", "{name}");
    }
}
