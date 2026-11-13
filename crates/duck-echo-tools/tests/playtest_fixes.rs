//! Fixes from playtest 1 (`playtest/REVIEW.md` on branch `playtest-1`):
//! loudness normalization, per-layer analysis with solo and mute, onsets,
//! energy over time and pictures, through the shared tools.

use duck_echo_audio::{decode_wav, encode_wav, AudioBuffer, WavFormat};
use duck_echo_core::{load_str, Code, Format, Recipe};
use duck_echo_tools::measure::{measure_recipe, Extras};
use duck_echo_tools::presets::{self, PRESETS};
use duck_echo_tools::{analyze_buffer, ToolError};
use std::path::Path;

fn recipe(src: &str) -> Recipe {
    load_str(src, Format::Toml)
        .unwrap_or_else(|r| panic!("{r}"))
        .recipe
}

fn example(name: &str) -> Recipe {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/recipes")
        .join(format!("{name}.toml"));
    recipe(&std::fs::read_to_string(path).unwrap())
}

/// The buffer as a 16-bit WAV reads back, which is what `analyze` sees.
fn as_written(buf: &AudioBuffer) -> AudioBuffer {
    decode_wav(&encode_wav(buf, WavFormat::Pcm16).unwrap()).unwrap()
}

fn category(r: &Recipe) -> &str {
    r.tags
        .iter()
        .find(|t| presets::CATEGORIES.contains(&t.as_str()))
        .unwrap()
}

/// `master.normalize_lufs` set to each preset's category target lands
/// within 0.1 LU of it as `analyze` measures the written WAV, never above
/// the peak ceiling, in one render.
#[test]
fn normalize_lufs_hits_every_category_target_in_one_render() {
    for p in PRESETS {
        let mut r = presets::load(p).unwrap().recipe;
        let target = presets::target_lufs(category(&r)).unwrap();
        // Added to the preset as it is, `normalize` and its peak included:
        // loudness normalization replaces them.
        r.master.normalize_lufs = Some(target);
        let (buf, mastering) = duck_echo_synth::render_mastered(&r).unwrap();
        assert!(!mastering.ceiling_limited, "{}", p.name);
        let a = analyze_buffer(&as_written(&buf));
        let lufs = a.loudness_lufs.unwrap();
        assert!(
            (lufs - target).abs() <= 0.1,
            "{}: {lufs:.3} LUFS, target {target}",
            p.name
        );
        assert!(a.peak_dbfs.unwrap() <= -1.0 + 0.01, "{}", p.name);
        assert!(!a.clipping.clipped, "{}", p.name);
        // Deterministic like every render.
        assert_eq!(duck_echo_synth::render(&r).unwrap(), buf, "{}", p.name);
    }
    // The example uses it as written.
    let a = analyze_buffer(&as_written(
        &duck_echo_synth::render(&example("fanfare")).unwrap(),
    ));
    assert!(
        (a.loudness_lufs.unwrap() + 19.0).abs() <= 0.1,
        "{}",
        a.summary
    );
}

/// The renderer's loudness meter (no libm, so renders match everywhere)
/// agrees with the analyzer's reference meter on every preset and example,
/// for both methods, mono and stereo, at more than one sample rate.
#[test]
fn render_loudness_meter_matches_the_analyzer() {
    let mut recipes: Vec<(String, Recipe)> = PRESETS
        .iter()
        .map(|p| (p.name.to_string(), presets::load(p).unwrap().recipe))
        .collect();
    for name in ["coin", "explosion", "ricochet", "fanfare"] {
        recipes.push((name.into(), example(name)));
        let mut r = example(name);
        r.sample_rate = 48_000;
        recipes.push((format!("{name}@48k"), r));
    }
    let (mut gated, mut short) = (0, 0);
    for (name, r) in recipes {
        let buf = duck_echo_synth::render(&r).unwrap();
        let ch = buf.channels as usize;
        let (ms, is_gated) =
            duck_echo_synth::loudness::mean_square(&buf.samples, ch, buf.sample_rate).unwrap();
        let planar: Vec<Vec<f32>> = (0..ch)
            .map(|c| buf.samples.iter().skip(c).step_by(ch).copied().collect())
            .collect();
        let slices: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
        let (reference, method) = duck_echo_analyze::loudness(&slices, buf.sample_rate).unwrap();
        let ours = -0.691 + 10.0 * ms.log10();
        assert!(
            (ours - reference as f64).abs() < 0.001,
            "{name}: {ours} vs {reference}"
        );
        assert_eq!(
            is_gated,
            method == duck_echo_analyze::LoudnessMethod::Bs1770Gated,
            "{name}"
        );
        if is_gated {
            gated += 1;
        } else {
            short += 1;
        }
    }
    assert!(gated > 5 && short > 5, "{gated} gated, {short} short");
}

/// A target the ceiling does not allow stops at the ceiling, and the tools
/// say so with a warning at `master.normalize_lufs`.
#[test]
fn normalize_lufs_stops_at_the_ceiling_and_warns() {
    // A sine peaking at -1 dBFS measures about -4 LUFS, so -2 is too loud.
    let r = recipe("version = 0\n[master]\nnormalize_lufs = -2\n[[layers]]\nsource = \"sine\"\n");
    let (buf, mastering) = duck_echo_synth::render_mastered(&r).unwrap();
    assert!(mastering.ceiling_limited);
    let a = analyze_buffer(&buf);
    assert!(
        (a.peak_dbfs.unwrap() + 1.0).abs() < 0.01,
        "{:?}",
        a.peak_dbfs
    );
    let m = measure_recipe(&r, &[], &Extras::default())
        .unwrap()
        .measured;
    let w = &m.warnings[0];
    assert_eq!(
        (w.code, w.path.as_str()),
        (Code::Suspicious, "master.normalize_lufs")
    );
    assert!(w.message.contains("-1 dBFS ceiling"), "{}", w.message);

    // A preset's own peak setting does not hold a louder target back.
    let mut coin = presets::load(presets::get("coin").unwrap()).unwrap().recipe;
    coin.master.normalize_lufs = Some(-14.0);
    let a = analyze_buffer(&duck_echo_synth::render(&coin).unwrap());
    assert!(
        (a.loudness_lufs.unwrap() + 14.0).abs() < 0.1,
        "{}",
        a.summary
    );

    // Reachable targets do not warn, and silence is left alone.
    let fine = recipe("version = 0\n[master]\nnormalize_lufs = -20\n[[layers]]\n");
    assert!(measure_recipe(&fine, &[], &Extras::default())
        .unwrap()
        .measured
        .warnings
        .is_empty());
    let silent = recipe("version = 0\n[master]\nnormalize_lufs = -20\n[[layers]]\nvolume = 0\n");
    let (buf, m) = duck_echo_synth::render_mastered(&silent).unwrap();
    assert!(buf.samples.iter().all(|&s| s == 0.0));
    assert_eq!(m.gain, 1.0);
}

#[test]
fn normalize_lufs_validation() {
    let both = load_str(
        "version = 0\n[master]\nnormalize = true\nnormalize_lufs = -18\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap();
    let w = &both.report.warnings[0];
    assert_eq!(
        (w.code, w.path.as_str()),
        (Code::Unused, "master.normalize")
    );
    assert!(w.message.contains("remove normalize"), "{}", w.message);
    let bad = load_str(
        "version = 0\n[master]\nnormalize_lufs = 3\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(bad.errors[0].code, Code::OutOfRange);
    assert_eq!(bad.errors[0].path, "master.normalize_lufs");
    assert_eq!(
        bad.errors[0].allowed,
        Some(duck_echo_core::Allowed::Range {
            min: -60.0,
            max: 0.0
        })
    );
}

const TWO_LAYERS: &str = r#"
version = 0
seed = 7
variation = 0.4
[[layers]]
source = "noise"
volume = 0.3
envelope = { attack = 0.001, decay = 0.2 }
[[layers]]
source = "square"
offset = 0.1
volume = 0.4
envelope = { attack = 0.002, hold = 0.1, decay = 0.2 }
pitch = { start = "A4" }
"#;

/// Solo renders keep each layer's seed stream, variation jitter, timing,
/// gain and length, so with no master effects the parts add up to the mix.
#[test]
fn solo_parts_add_up_to_the_mix() {
    for master in [
        "",
        "[master]\nnormalize = true\ntrim_silence = true\n",
        "[master]\nnormalize_lufs = -20\ntrim_silence = true\n",
    ] {
        let src = TWO_LAYERS.replacen("[[layers]]", &format!("{master}[[layers]]"), 1);
        let r = recipe(&src);
        let (full, mastering) = duck_echo_synth::render_mastered(&r).unwrap();
        let a = duck_echo_synth::render_layers(&r, &[0], &mastering).unwrap();
        let b = duck_echo_synth::render_layers(&r, &[1], &mastering).unwrap();
        assert_eq!(a.samples.len(), full.samples.len(), "{master}");
        assert_eq!(b.samples.len(), full.samples.len(), "{master}");
        for i in 0..full.samples.len() {
            let sum = a.samples[i] + b.samples[i];
            assert!((sum - full.samples[i]).abs() < 1e-5, "{master} at {i}");
        }
        // The square starts at 0.1 s in the solo too.
        assert!(b.samples[..4400].iter().all(|&s| s == 0.0));
        let both = duck_echo_synth::render_layers(&r, &[0, 1], &mastering).unwrap();
        assert_eq!(both, full, "{master}");
    }
}

/// Per-layer analysis reads each part of a layered sound: the fanfare's
/// lead, chord and bass. The mix's onsets carry each new note's pitch.
#[test]
fn per_layer_analysis_and_onsets_read_the_fanfare() {
    let r = example("fanfare");
    let extras = Extras {
        per_layer: true,
        ..Extras::default()
    };
    let m = measure_recipe(&r, &[], &extras).unwrap().measured;
    let notes = |a: &duck_echo_analyze::Analysis| -> Vec<(f64, f32)> {
        a.onsets
            .iter()
            .map(|o| (o.time_seconds, o.note_hz.unwrap_or(0.0)))
            .collect()
    };
    let near = |got: &[(f64, f32)], want: &[(f64, f32)]| {
        got.len() == want.len()
            && got.iter().zip(want).all(|((t, hz), (wt, whz))| {
                (t - wt).abs() < 0.015 && (12.0 * (hz / whz).log2()).abs() < 0.3
            })
    };
    // Mix: the bass starts with the lead, then E5, G5, C6, and the chord.
    let mix = notes(&m.analysis);
    assert!(
        near(
            &mix,
            &[
                (0.0, 130.8),
                (0.09, 659.3),
                (0.18, 784.0),
                (0.27, 1046.5),
                (0.4, 1318.5)
            ]
        ),
        "{mix:?}"
    );
    assert!(
        m.analysis.summary.contains("5 onsets"),
        "{}",
        m.analysis.summary
    );
    let layers = m.layers.unwrap();
    assert_eq!(layers.len(), 4);
    assert_eq!(layers[0].name.as_deref(), Some("lead"));
    let lead = notes(&layers[0].analysis);
    assert!(
        near(
            &lead,
            &[(0.0, 523.3), (0.09, 659.3), (0.18, 784.0), (0.27, 1046.5)]
        ),
        "{lead:?}"
    );
    assert!(near(&notes(&layers[1].analysis), &[(0.4, 1318.5)]));
    assert!(near(&notes(&layers[3].analysis), &[(0.0, 130.8)]));
    // Each layer is measured at its level in the mix, so all are quieter.
    for l in &layers {
        assert!(l.analysis.loudness_lufs.unwrap() < m.analysis.loudness_lufs.unwrap());
        assert_eq!(l.analysis.frames, m.analysis.frames);
    }

    // solo + mute: hear the chord only.
    let chord = Extras {
        solo: vec![1, 2, 3],
        mute: vec![3],
        ..Extras::default()
    };
    let c = measure_recipe(&r, &[], &chord).unwrap().measured;
    assert_eq!(c.heard_layers, Some(vec![1, 2]));
    assert!((c.analysis.onsets[0].time_seconds - 0.4).abs() < 0.015);
}

/// Energy over time shows where the sound is, slice by slice.
#[test]
fn energy_over_time_follows_the_envelope() {
    let r = example("fanfare");
    let a = analyze_buffer(&duck_echo_synth::render(&r).unwrap());
    let e = &a.energy_over_time;
    assert_eq!(e.rms_dbfs.len(), 20);
    assert!((e.slice_seconds * 20.0 - a.duration_seconds).abs() < 1e-9);
    let first = e.rms_dbfs[0].unwrap();
    let last = e.rms_dbfs[19].unwrap();
    assert!(first > -25.0 && last < first - 30.0, "{:?}", e.rms_dbfs);
}

/// A picture comes back as PNG bytes with its axes described.
#[test]
fn picture_is_a_png_with_axes() {
    let r = example("fanfare");
    let extras = Extras {
        picture: true,
        ..Extras::default()
    };
    let m = measure_recipe(&r, &[], &extras).unwrap().measured;
    let png = m.png.unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() < 100_000, "{} bytes", png.len());
    let info = m.picture.unwrap();
    assert_eq!(info.mime_type, "image/png");
    assert!((info.seconds - m.analysis.duration_seconds).abs() < 1e-9);
    // Not asked, not drawn.
    let plain = measure_recipe(&r, &[], &Extras::default())
        .unwrap()
        .measured;
    assert!(plain.png.is_none() && plain.picture.is_none());
    let json = serde_json::to_value(&plain).unwrap();
    for key in ["layers", "heard_layers", "picture", "warnings", "png"] {
        assert!(json.get(key).is_none(), "{key} should be omitted");
    }
    assert!(json.get("onsets").is_some() && json.get("summary").is_some());
}

/// Layer extras name the argument at fault, with the allowed range.
#[test]
fn layer_extras_errors_name_the_argument() {
    let bad = Extras {
        solo: vec![0, 9],
        ..Extras::default()
    };
    match bad.heard(4, "arguments.") {
        Err(ToolError::Invalid(r)) => {
            assert_eq!(r.errors[0].path, "arguments.solo[1]");
            assert_eq!(r.errors[0].code, Code::OutOfRange);
            assert_eq!(
                r.errors[0].allowed,
                Some(duck_echo_core::Allowed::Range { min: 0.0, max: 3.0 })
            );
        }
        other => panic!("{other:?}"),
    }
    let nothing = Extras {
        solo: vec![1],
        mute: vec![1],
        ..Extras::default()
    };
    match nothing.heard(2, "") {
        Err(ToolError::Failed(i)) => {
            assert_eq!((i.code, i.path.as_str()), (Code::InvalidArgument, "mute"))
        }
        other => panic!("{other:?}"),
    }
    let wav = Extras {
        per_layer: true,
        ..Extras::default()
    };
    let buf = duck_echo_synth::render(&example("coin")).unwrap();
    match duck_echo_tools::measure::measure_audio(&buf, &wav) {
        Err(ToolError::Failed(i)) => {
            assert_eq!(
                (i.code, i.path.as_str()),
                (Code::InvalidArgument, "per_layer")
            )
        }
        other => panic!("{other:?}"),
    }
}

/// File errors have specific codes, and front ends fill in the argument.
#[test]
fn file_errors_have_codes_and_take_the_argument_path() {
    let missing = Path::new("definitely/not/here.wav");
    let e = duck_echo_tools::load_audio(missing)
        .unwrap_err()
        .at("input");
    let r = e.into_report();
    assert_eq!(
        (r.errors[0].code, r.errors[0].path.as_str()),
        (Code::NotFound, "input")
    );
    let e = duck_echo_tools::load_recipe(Path::new("nope/x.toml"))
        .unwrap_err()
        .at("recipe")
        .into_report();
    assert_eq!(
        (e.errors[0].code, e.errors[0].path.as_str()),
        (Code::NotFound, "recipe")
    );
    // A recipe error keeps its recipe path.
    let invalid = duck_echo_tools::edit::load_value(
        &serde_json::json!({"version": 0, "layers": [{"volume": 9}]}),
    )
    .unwrap_err()
    .at("arguments.recipe")
    .into_report();
    assert_eq!(invalid.errors[0].path, "layers[0].volume");
}

/// Onsets count the notes a preset is written with, and sustained noise,
/// sweeps, vibrato and hums count as one.
#[test]
fn onsets_count_the_notes_of_each_preset() {
    for (name, notes) in [
        ("alarm", 7),
        ("coin", 2),
        ("extra_life", 7),
        ("gem", 6),
        ("key_pickup", 2),
        ("low_health", 2),
        ("ui_cancel", 2),
        ("ui_confirm", 2),
        ("ui_error", 2),
        ("warning_beep", 3),
        ("big_explosion", 1),
        ("charge_up", 1),
        ("engine_hum", 1),
        ("explosion", 1),
        ("jump", 1),
        ("missile", 1),
        ("shotgun", 1),
        ("wind", 1),
    ] {
        let r = presets::load(presets::get(name).unwrap()).unwrap().recipe;
        let a = analyze_buffer(&duck_echo_synth::render(&r).unwrap());
        let times: Vec<String> = a
            .onsets
            .iter()
            .map(|o| format!("{:.3}", o.time_seconds))
            .collect();
        assert_eq!(a.onsets.len(), notes, "{name}: {times:?}");
    }
}
