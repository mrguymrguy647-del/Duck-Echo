//! End-to-end tests over the public crates: determinism, format
//! equivalence, validation messages and the example recipes.

use duck_echo_analyze::analyze;
use duck_echo_audio::{encode_wav, WavFormat};
use duck_echo_core::{load_str, parse_str, to_json, to_toml, Code, Format, Recipe};
use std::path::{Path, PathBuf};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/recipes")
}

fn example_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("examples dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    assert!(files.len() >= 6, "expected the six example recipes");
    files
}

fn load_file(path: &Path) -> Recipe {
    let src = std::fs::read_to_string(path).expect("read");
    match load_str(&src, Format::from_path(path)) {
        Ok(l) => l.recipe,
        Err(r) => panic!("{} failed to load:\n{r}", path.display()),
    }
}

#[test]
fn same_seed_renders_byte_identical_wav() {
    for path in example_files() {
        let recipe = load_file(&path);
        let a = duck_echo_synth::render(&recipe).expect("render a");
        let b = duck_echo_synth::render(&recipe).expect("render b");
        assert_eq!(a.samples, b.samples, "{}", path.display());
        let wa = encode_wav(&a, WavFormat::Pcm16).unwrap();
        let wb = encode_wav(&b, WavFormat::Pcm16).unwrap();
        assert_eq!(wa, wb, "{} wav bytes differ", path.display());
    }
}

#[test]
fn different_seed_changes_noise_and_variation() {
    let mut recipe = load_file(&examples_dir().join("explosion.toml"));
    let a = duck_echo_synth::render(&recipe).unwrap();
    recipe.seed += 1;
    let b = duck_echo_synth::render(&recipe).unwrap();
    assert_ne!(a.samples, b.samples, "noise should follow the seed");

    // A pure tone ignores the seed until variation is enabled.
    let mut tone = load_file(&examples_dir().join("coin.toml"));
    let a = duck_echo_synth::render(&tone).unwrap();
    tone.seed = 99;
    let b = duck_echo_synth::render(&tone).unwrap();
    assert_eq!(
        a.samples, b.samples,
        "no noise and no variation: seed is inert"
    );
    tone.variation = 0.5;
    let c = duck_echo_synth::render(&tone).unwrap();
    assert_ne!(a.samples, c.samples, "variation should jitter the tone");
}

#[test]
fn toml_and_json_are_equivalent() {
    let toml_recipe = load_file(&examples_dir().join("coin.toml"));
    let json_recipe = load_file(&examples_dir().join("coin.json"));
    assert_eq!(toml_recipe, json_recipe, "coin.toml and coin.json differ");

    for path in example_files() {
        let recipe = load_file(&path);
        let via_json = parse_str(&to_json(&recipe), Format::Json).expect("json round trip");
        let via_toml = parse_str(&to_toml(&recipe), Format::Toml).expect("toml round trip");
        assert_eq!(recipe, via_json, "{}", path.display());
        assert_eq!(recipe, via_toml, "{}", path.display());
        let a = duck_echo_synth::render(&recipe).unwrap();
        let b = duck_echo_synth::render(&via_json).unwrap();
        assert_eq!(
            a.samples,
            b.samples,
            "{} renders differ after round trip",
            path.display()
        );
    }
}

#[test]
fn every_example_renders_clean() {
    for path in example_files() {
        let recipe = load_file(&path);
        let buf =
            duck_echo_synth::render(&recipe).unwrap_or_else(|r| panic!("{}: {r}", path.display()));
        let a = analyze(&buf.samples, buf.sample_rate, buf.channels);
        assert!(!a.is_silent, "{} is silent", path.display());
        assert!(
            a.loudness_lufs.is_some() && a.loudness_method.is_some(),
            "{} has no loudness",
            path.display()
        );
        assert!(
            !a.clipping.clipped,
            "{} clips ({} samples)",
            path.display(),
            a.clipping.clipped_samples
        );
        assert!(
            a.peak_dbfs.unwrap() > -20.0,
            "{} is very quiet: {:?}",
            path.display(),
            a.peak_dbfs
        );
        assert!(
            a.leading_silence_seconds < 0.01,
            "{} starts late",
            path.display()
        );
        assert!(
            a.duration_seconds > 0.05 && a.duration_seconds < 5.0,
            "{} odd length",
            path.display()
        );
        assert!(
            a.dc_offset.abs() < 0.05,
            "{} dc offset {}",
            path.display(),
            a.dc_offset
        );
        assert!(
            buf.samples.iter().all(|s| s.is_finite()),
            "{} has NaN",
            path.display()
        );
    }
}

#[test]
fn minimal_recipe_renders() {
    let loaded = load_str("version = 0\n[[layers]]\n", Format::Toml).expect("minimal loads");
    assert!(loaded.report.warnings.is_empty());
    let buf = duck_echo_synth::render(&loaded.recipe).unwrap();
    let a = analyze(&buf.samples, buf.sample_rate, 1);
    assert!(!a.is_silent && !a.clipping.clipped);
    assert!((a.duration_seconds - 0.205).abs() < 0.002);
}

fn first_error(src: &str, format: Format) -> duck_echo_core::Issue {
    match load_str(src, format) {
        Ok(_) => panic!("expected an error for:\n{src}"),
        Err(r) => r.errors.into_iter().next().expect("at least one error"),
    }
}

#[test]
fn typo_in_field_name_gets_path_and_suggestion() {
    let e = first_error(
        "version = 0\n[[layers]]\n[layers.pitch]\nstrat = 440\n",
        Format::Toml,
    );
    assert_eq!(e.code, Code::UnknownField);
    assert_eq!(e.path, "layers[0].pitch.strat");
    assert_eq!(e.suggestion.as_deref(), Some("start"));
    assert_eq!(e.line, Some(4));

    let e = first_error(r#"{"version":0,"layers":[{"gain":0.5}]}"#, Format::Json);
    assert_eq!(e.code, Code::UnknownField);
    assert_eq!(e.path, "layers[0].gain");
    assert_eq!(e.suggestion.as_deref(), Some("volume"));

    let e = first_error("version = 0\nlayer = []\n", Format::Toml);
    assert_eq!(e.path, "layer");
    assert_eq!(e.suggestion.as_deref(), Some("layers"));
}

#[test]
fn bad_enum_value_lists_options() {
    let e = first_error(
        "version = 0\n[[layers]]\nsource = \"sqaure\"\n",
        Format::Toml,
    );
    assert_eq!(e.code, Code::UnknownVariant);
    assert_eq!(e.path, "layers[0].source");
    assert_eq!(e.suggestion.as_deref(), Some("square"));
    match e.allowed {
        Some(duck_echo_core::Allowed::OneOf { options }) => {
            assert_eq!(options, ["sine", "square", "saw", "triangle", "noise"]);
        }
        other => panic!("expected options, got {other:?}"),
    }

    let e = first_error(
        "version = 0\n[[layers]]\n[[effects]]\ntype = \"delay\"\n",
        Format::Toml,
    );
    assert_eq!(e.path, "effects[0].type");
    assert_eq!(e.suggestion.as_deref(), Some("echo"));
}

#[test]
fn out_of_range_reports_every_problem_with_bounds() {
    let src = "version = 0\n[[layers]]\nvolume = 3\n[layers.pitch]\nstart = 0\n[[effects]]\ntype = \"echo\"\nfeedback = 1.5\n";
    let report = load_str(src, Format::Toml).unwrap_err();
    let paths: Vec<&str> = report.errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "layers[0].volume",
            "layers[0].pitch.start",
            "effects[0].feedback"
        ]
    );
    for e in &report.errors {
        assert_eq!(e.code, Code::OutOfRange);
        assert!(matches!(
            e.allowed,
            Some(duck_echo_core::Allowed::Range { .. })
        ));
        assert!(e.got.is_some());
    }
    assert!(report.errors[0].to_string().contains("0..=2"));
    assert!(!report.ok);

    // JSON form is stable and parseable.
    let json: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["errors"][2]["allowed"]["max"], 0.99);
}

#[test]
fn missing_version_wrong_type_and_no_layers() {
    let e = first_error("[[layers]]\n", Format::Toml);
    assert_eq!(e.code, Code::MissingField);
    assert_eq!(e.path, "version");

    let e = first_error("version = 1\n[[layers]]\n", Format::Toml);
    assert_eq!(e.code, Code::UnsupportedVersion);

    let e = first_error("version = 0\n[[layers]]\nvolume = \"loud\"\n", Format::Toml);
    assert_eq!(e.code, Code::TypeMismatch);
    assert_eq!(e.path, "layers[0].volume");

    let e = first_error("version = 0\n", Format::Toml);
    assert_eq!(e.code, Code::Invalid);
    assert_eq!(e.path, "layers");

    let e = first_error(
        "version = 0\n[[layers]]\n[layers.pitch.arpeggio]\nsteps = []\n",
        Format::Toml,
    );
    assert_eq!(e.path, "layers[0].pitch.arpeggio.steps");
}

#[test]
fn warnings_do_not_block() {
    let loaded = load_str(
        "version = 0\n[[layers]]\nsource = \"sine\"\nduty = 0.2\n",
        Format::Toml,
    )
    .expect("warnings only");
    assert_eq!(loaded.report.warnings.len(), 1);
    assert_eq!(loaded.report.warnings[0].code, Code::Unused);
    assert_eq!(loaded.report.warnings[0].path, "layers[0].duty");

    // `aliasing = "retro"` changes nothing on a sine, so it is flagged too.
    let loaded = load_str(
        "version = 0\n[[layers]]\nsource = \"sine\"\naliasing = \"retro\"\n",
        Format::Toml,
    )
    .expect("warnings only");
    assert_eq!(loaded.report.warnings[0].code, Code::Unused);
    assert_eq!(loaded.report.warnings[0].path, "layers[0].aliasing");
    // A typo in the value is caught with a suggestion.
    let report = load_str(
        "version = 0\n[[layers]]\naliasing = \"retor\"\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "layers[0].aliasing");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("retro"));
}

#[test]
fn checked_in_schema_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/recipe.schema.json");
    let on_disk = std::fs::read_to_string(&path).expect("schema file");
    let fresh = duck_echo_core::schema::recipe_schema_json();
    assert_eq!(
        on_disk.trim(),
        fresh.trim(),
        "schema/recipe.schema.json is stale; run `duck-echo schema > schema/recipe.schema.json`"
    );
}

#[test]
fn schema_describes_fields_with_defaults() {
    let schema = duck_echo_core::schema::recipe_schema();
    let layer = &schema["$defs"]["Layer"]["properties"];
    assert_eq!(layer["volume"]["default"], 0.8);
    assert!(layer["volume"]["description"]
        .as_str()
        .unwrap()
        .contains("0..=2"));
    assert_eq!(schema["properties"]["version"]["type"], "integer");
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn duration_override_and_tails() {
    let mut r = load_str(
        "version = 0\n[[layers]]\n[[effects]]\ntype = \"echo\"\ndelay = 0.1\nfeedback = 0.5\n",
        Format::Toml,
    )
    .unwrap()
    .recipe;
    let buf = duck_echo_synth::render(&r).unwrap();
    // 0.205 s of envelope plus an echo tail of ~1 s (10 repeats at 0.5 feedback).
    assert!(
        buf.duration_seconds() > 1.0 && buf.duration_seconds() < 1.4,
        "{}",
        buf.duration_seconds()
    );
    r.duration = Some(0.5);
    let buf = duck_echo_synth::render(&r).unwrap();
    assert_eq!(buf.frames(), 22050);
}

#[test]
fn voice_streams_in_arbitrary_block_sizes() {
    let mut recipe = load_file(&examples_dir().join("laser.toml"));
    // Trimming is an offline post-process; compare the raw stream.
    recipe.master.trim_silence = false;
    let whole = duck_echo_synth::render(&recipe).unwrap();
    let mut voice = duck_echo_synth::prepare(&recipe).unwrap();
    let mut out = Vec::new();
    let mut block = [0.0f32; 97];
    loop {
        let n = voice.process(&mut block);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&block[..n]);
    }
    let clipped: Vec<f32> = out.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    assert_eq!(clipped, whole.samples);
}

/// Review fix 1: one call reports every structural and range problem.
#[test]
fn one_call_reports_all_six_mistakes_in_toml_and_json() {
    let toml_src = r#"
version = 0
sampel_rate = 48000

[[layers]]
sorce = "sine"
volume = 3

[layers.pitch]
strat = 440

[[layers]]
envelope = { atack = 0.01, decay = 99 }

[[effects]]
type = "echo"
feedbak = 0.5
"#;
    let json_src = r#"{
  "version": 0,
  "sampel_rate": 48000,
  "layers": [
    { "sorce": "sine", "volume": 3, "pitch": { "strat": 440 } },
    { "envelope": { "atack": 0.01, "decay": 99 } }
  ],
  "effects": [ { "type": "echo", "feedbak": 0.5 } ]
}"#;
    let expected = [
        ("sampel_rate", Code::UnknownField, Some("sample_rate")),
        ("layers[0].sorce", Code::UnknownField, Some("source")),
        ("layers[0].volume", Code::OutOfRange, None),
        ("layers[0].pitch.strat", Code::UnknownField, Some("start")),
        (
            "layers[1].envelope.atack",
            Code::UnknownField,
            Some("attack"),
        ),
        ("layers[1].envelope.decay", Code::OutOfRange, None),
        ("effects[0].feedbak", Code::UnknownField, Some("feedback")),
    ];
    for (src, format) in [(toml_src, Format::Toml), (json_src, Format::Json)] {
        let report = load_str(src, format).unwrap_err();
        assert!(report.warnings.is_empty(), "{format:?}: {report}");
        let mut got: Vec<(&str, Code, Option<&str>)> = report
            .errors
            .iter()
            .map(|e| (e.path.as_str(), e.code, e.suggestion.as_deref()))
            .collect();
        got.sort_by_key(|g| g.0);
        let mut want = expected.to_vec();
        want.sort_by_key(|w| w.0);
        // Seven entries: the six mistakes in the review plus the
        // `sampel_rate` typo, so both formats are exercised on root keys.
        assert_eq!(got, want, "{format:?}:\n{report}");
        for e in &report.errors {
            assert!(e.line.is_some(), "{format:?}: no line for {}", e.path);
        }
    }
}

#[test]
fn structural_errors_do_not_hide_each_other() {
    // A broken effect tag, a wrong container type, a wrong scalar type and
    // a non-finite number, all in one document.
    let src = "version = \"zero\"\nlayers = 5\n[[effects]]\ndelay = 1\n[[effects]]\ntype = \"delay\"\n[master]\nnormalize = \"yes\"\ngain = nan\n";
    let report = load_str(src, Format::Toml).unwrap_err();
    let got: Vec<(&str, Code)> = report
        .errors
        .iter()
        .map(|e| (e.path.as_str(), e.code))
        .collect();
    assert_eq!(
        got,
        [
            ("version", Code::TypeMismatch),
            ("layers", Code::TypeMismatch),
            ("effects[0].type", Code::MissingField),
            ("effects[1].type", Code::UnknownVariant),
            ("master.normalize", Code::TypeMismatch),
            ("master.gain", Code::OutOfRange),
        ],
        "{report}"
    );
    // The broken `layers` must not also produce a "no layers" error, and
    // the broken `version` must not produce an "unsupported version" one.
    assert_eq!(report.errors.len(), 6);
    assert_eq!(report.errors[3].suggestion.as_deref(), Some("echo"));
}

#[test]
fn typo_does_not_also_report_the_defaulted_field() {
    // `layer` instead of `layers`: one error with a suggestion, not a second
    // "recipe has no layers" error about the default.
    let report = load_str("version = 0\n[[layer]]\n", Format::Toml).unwrap_err();
    assert_eq!(report.errors.len(), 1, "{report}");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("layers"));
}

#[test]
fn array_items_keep_their_indices_when_one_is_broken() {
    let src = r#"{"version":0,"layers":[{"pitch":{"arpeggio":{"steps":[0,"x",7]}}},{"effects":[{"type":"reverb","size":2}]}]}"#;
    let report = load_str(src, Format::Json).unwrap_err();
    let paths: Vec<&str> = report.errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "layers[0].pitch.arpeggio.steps[1]",
            "layers[1].effects[0].size"
        ]
    );
}

/// FNV-1a 64: tiny, dependency-free fingerprint for golden WAV bytes.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// PCM16 WAV size and fingerprint of every example as first rendered, with
/// `trim_silence` off: the first six by Phase 0 (commit c5acbdb), chime and
/// ricochet by commit c6f131e, fanfare (the first `normalize_lufs` render)
/// with the playtest 1 fixes. Any change to the render path must keep
/// these. If one changes on purpose, say so in the commit and update the
/// value; never update it to make a test pass.
const GOLDEN: &[(&str, usize, u64)] = &[
    ("coin", 27562, 0x5fa2_e4ec_072f_3abb),
    ("explosion", 140282, 0x26f2_8b89_fd8a_4a93),
    ("hit", 10716, 0x1d32_6886_9b8b_82ca),
    ("jump", 24300, 0x40a5_e896_885d_d69c),
    ("laser", 76072, 0x5084_0496_2617_46b2),
    ("powerup", 135872, 0x4587_bbac_ba84_9319),
    ("chime", 127494, 0xb5a1_c5f4_8c03_8d1f),
    ("ricochet", 152100, 0xce1b_45c1_7590_5b49),
    ("fanfare", 138870, 0x3059_bbd5_3975_1723),
    ("siren", 247004, 0x9503_ea9b_e0ba_de68),
];

#[test]
fn every_example_is_pinned_by_the_golden_list() {
    for path in example_files() {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        assert!(
            GOLDEN.iter().any(|(n, _, _)| *n == stem),
            "{stem}.toml is not in GOLDEN; render it once and add its fingerprint"
        );
    }
}

#[test]
fn default_render_path_is_byte_identical_to_phase0() {
    for (name, len, hash) in GOLDEN {
        let mut recipe = load_file(&examples_dir().join(format!("{name}.toml")));
        recipe.master.trim_silence = false;
        let buf = duck_echo_synth::render(&recipe).unwrap();
        let wav = encode_wav(&buf, WavFormat::Pcm16).unwrap();
        assert_eq!(
            (wav.len(), fnv1a64(&wav)),
            (*len, *hash),
            "{name}: render differs from Phase 0"
        );
    }
}

/// Review fix 3: trim_silence removes the silent tail and nothing else.
#[test]
fn trim_silence_only_cuts_the_silent_end() {
    for name in ["explosion", "laser", "powerup"] {
        let trimmed_recipe = load_file(&examples_dir().join(format!("{name}.toml")));
        assert!(trimmed_recipe.master.trim_silence, "{name} should use trim");
        let mut full_recipe = trimmed_recipe.clone();
        full_recipe.master.trim_silence = false;

        let full = duck_echo_synth::render(&full_recipe).unwrap();
        let trimmed = duck_echo_synth::render(&trimmed_recipe).unwrap();
        assert!(trimmed.samples.len() < full.samples.len());
        assert_eq!(
            trimmed.samples[..],
            full.samples[..trimmed.samples.len()],
            "{name}: trimmed render must be a prefix of the full one"
        );

        let before = analyze(&full.samples, full.sample_rate, 1);
        assert!(before.trailing_silence_seconds / before.duration_seconds > 0.2);
        // Analyze the WAV as written, after 16-bit quantization.
        let wav = encode_wav(&trimmed, WavFormat::Pcm16).unwrap();
        let back = duck_echo_audio::decode_wav(&wav).unwrap();
        let after = analyze(&back.samples, back.sample_rate, 1);
        assert_eq!(after.trailing_silence_seconds, 0.0, "{name}");
    }

    // A forced duration wins over trimming, with a warning.
    let loaded = load_str(
        "version = 0\nduration = 1.0\n[master]\ntrim_silence = true\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap();
    assert_eq!(loaded.report.warnings[0].path, "master.trim_silence");
    let buf = duck_echo_synth::render(&loaded.recipe).unwrap();
    assert_eq!(buf.frames(), 44100);
}

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

fn rms(xs: &[f32]) -> f32 {
    (xs.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / xs.len() as f64).sqrt() as f32
}

fn channel(buf: &duck_echo_audio::AudioBuffer, c: usize) -> Vec<f32> {
    buf.samples
        .iter()
        .skip(c)
        .step_by(buf.channels as usize)
        .copied()
        .collect()
}

const TWO_TONES: &str = r#"
version = 0
channels = 2
[[layers]]
source = "sine"
volume = 0.5
pan = -1
[layers.envelope]
hold = 0.3
decay = 0
[layers.pitch]
start = 440
[[layers]]
source = "saw"
# Retro: the raw saw peaks exactly at its volume, which the test checks.
aliasing = "retro"
volume = 0.3
pan = 1
[layers.envelope]
hold = 0.2
decay = 0
[layers.pitch]
start = 660
"#;

/// Roadmap task 1: hard-panned layers land in their own channel at the
/// level they would have alone in mono.
#[test]
fn stereo_hard_pan_puts_each_layer_in_its_channel() {
    let stereo = load_str(TWO_TONES, Format::Toml).unwrap().recipe;
    let buf = duck_echo_synth::render(&stereo).unwrap();
    assert_eq!(buf.channels, 2);
    let (left, right) = (channel(&buf, 0), channel(&buf, 1));

    for (keep, side, label) in [(0usize, &left, "left"), (1, &right, "right")] {
        let mut solo = stereo.clone();
        solo.channels = 1;
        solo.layers = vec![solo.layers[keep].clone()];
        solo.layers[0].pan = 0.0;
        solo.duration = Some(buf.duration_seconds() as f32);
        let mono = duck_echo_synth::render(&solo).unwrap();
        let diff = db(rms(side)) - db(rms(&mono.samples));
        assert!(diff.abs() < 0.1, "{label}: {diff} dB from the solo layer");
    }
    // No leakage: the left channel holds only the sine, which ends at
    // 0.305 s; the right channel's saw ends at 0.205 s.
    let after = |xs: &[f32], t: f64| xs[(t * 44100.0) as usize..].iter().all(|s| s.abs() < 1e-6);
    assert!(
        after(&right, 0.21),
        "left layer leaked into the right channel"
    );
    assert!(!after(&left, 0.21));

    let a = analyze(&buf.samples, buf.sample_rate, buf.channels);
    assert_eq!(a.channels, 2);
    assert_eq!(a.per_channel.len(), 2);
    assert!((a.per_channel[0].peak - 0.5).abs() < 0.01);
    assert!((a.per_channel[1].peak - 0.3).abs() < 0.01);
}

#[test]
fn centre_pan_is_minus_three_db_per_channel() {
    let mut recipe = load_file(&examples_dir().join("coin.toml"));
    let mono = duck_echo_synth::render(&recipe).unwrap();
    recipe.channels = 2;
    let stereo = duck_echo_synth::render(&recipe).unwrap();
    let (left, right) = (channel(&stereo, 0), channel(&stereo, 1));
    assert_eq!(
        left, right,
        "centred layer must be identical in both channels"
    );
    let diff = db(rms(&left)) - db(rms(&mono.samples));
    assert!((diff + 3.01).abs() < 0.01, "{diff}");
}

#[test]
fn master_effects_run_independently_per_channel() {
    // With a master echo, removing the right-only layer must leave the left
    // channel bit-identical. A shared effect state would leak it across.
    let src = format!(
        "{TWO_TONES}\n[[effects]]\ntype = \"echo\"\ndelay = 0.1\nfeedback = 0.5\nmix = 0.5\n"
    );
    let both = load_str(&src, Format::Toml).unwrap().recipe;
    let mut left_only = both.clone();
    left_only.layers.truncate(1);
    left_only.duration = Some(1.0);
    let mut both = both;
    both.duration = Some(1.0);
    let a = duck_echo_synth::render(&both).unwrap();
    let b = duck_echo_synth::render(&left_only).unwrap();
    assert_eq!(channel(&a, 0), channel(&b, 0));
    assert_ne!(channel(&a, 1), channel(&b, 1));
}

#[test]
fn stereo_streams_in_arbitrary_block_sizes() {
    let mut recipe = load_file(&examples_dir().join("ricochet.toml"));
    recipe.master.trim_silence = false;
    let whole = duck_echo_synth::render(&recipe).unwrap();
    let mut voice = duck_echo_synth::prepare(&recipe).unwrap();
    assert_eq!(voice.channels(), 2);
    let mut out = Vec::new();
    // Odd length: the trailing partial frame is zeroed and not counted.
    let mut block = [0.0f32; 2 * 97 + 1];
    loop {
        let n = voice.process(&mut block);
        if n == 0 {
            break;
        }
        assert_eq!(block[2 * 97], 0.0);
        out.extend_from_slice(&block[..n * 2]);
    }
    let clipped: Vec<f32> = out.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    assert_eq!(clipped, whole.samples);
}

#[test]
fn stereo_wav_round_trips_and_trims_by_frame() {
    let recipe = load_file(&examples_dir().join("ricochet.toml"));
    let buf = duck_echo_synth::render(&recipe).unwrap();
    assert_eq!(buf.samples.len() % 2, 0);
    let wav = encode_wav(&buf, WavFormat::Pcm16).unwrap();
    let back = duck_echo_audio::decode_wav(&wav).unwrap();
    assert_eq!(back.channels, 2);
    assert_eq!(back.frames(), buf.frames());
    let a = analyze(&back.samples, back.sample_rate, back.channels);
    assert_eq!(a.trailing_silence_seconds, 0.0);
    assert!(!a.clipping.clipped);
}

#[test]
fn channel_and_pan_validation() {
    let report = load_str(
        "version = 0\nchannels = 3\n[[layers]]\npan = 1.5\n",
        Format::Toml,
    )
    .unwrap_err();
    let got: Vec<(&str, Code)> = report
        .errors
        .iter()
        .map(|e| (e.path.as_str(), e.code))
        .collect();
    assert_eq!(
        got,
        [
            ("channels", Code::OutOfRange),
            ("layers[0].pan", Code::OutOfRange)
        ]
    );

    let loaded = load_str("version = 0\n[[layers]]\npan = 0.5\n", Format::Toml).unwrap();
    assert_eq!(loaded.report.warnings[0].path, "layers[0].pan");
    assert_eq!(loaded.report.warnings[0].code, Code::Unused);
}

/// Roadmap task 2: note names render like the equivalent Hz and keep their
/// spelling through TOML and JSON.
#[test]
fn note_names_match_hz_and_round_trip() {
    let notes = "version = 0\n[[layers]]\nsource = \"sine\"\n[layers.pitch]\nstart = \"A4\"\nend = \"A5\"\ncurve = \"exponential\"\n";
    let hz = notes.replace("\"A4\"", "440").replace("\"A5\"", "880");
    let a = load_str(notes, Format::Toml).unwrap().recipe;
    let b = load_str(&hz, Format::Toml).unwrap().recipe;
    assert_eq!(a.layers[0].pitch.start.note(), Some("A4"));
    assert_eq!(a.layers[0].pitch.start.hz(), 440.0);
    assert_eq!(b.layers[0].pitch.start.note(), None);
    assert_eq!(
        duck_echo_synth::render(&a).unwrap().samples,
        duck_echo_synth::render(&b).unwrap().samples
    );

    let json = to_json(&a);
    assert!(json.contains("\"start\": \"A4\""), "{json}");
    assert_eq!(parse_str(&json, Format::Json).unwrap(), a);
    let toml = to_toml(&a);
    assert!(toml.contains("start = \"A4\""), "{toml}");
    assert_eq!(parse_str(&toml, Format::Toml).unwrap(), a);
    // Numbers stay numbers.
    assert!(to_json(&b).contains("\"start\": 440.0"));

    let chime = load_file(&examples_dir().join("chime.toml"));
    assert_eq!(chime.layers[1].pitch.start.note(), Some("B5"));
    assert!((chime.layers[1].pitch.start.hz() - 987.77).abs() < 0.01);
}

#[test]
fn bad_note_name_is_reported_at_its_path() {
    for (src, format) in [
        (
            "version = 0\n[[layers]]\n[layers.pitch]\nstart = \"H4\"\n",
            Format::Toml,
        ),
        (
            r#"{"version":0,"layers":[{"pitch":{"start":"H4"}}]}"#,
            Format::Json,
        ),
    ] {
        let report = load_str(src, format).unwrap_err();
        assert_eq!(report.errors.len(), 1, "{report}");
        let e = &report.errors[0];
        assert_eq!(e.code, Code::InvalidFormat);
        assert_eq!(e.path, "layers[0].pitch.start");
        assert_eq!(e.suggestion.as_deref(), Some("B4"));
        assert!(e.message.contains("for example \"C5\""), "{}", e.message);
        assert!(e.line.is_some());
    }
    let e = first_error(
        "version = 0\n[[layers]]\n[layers.pitch]\nend = true\n",
        Format::Toml,
    );
    assert_eq!(e.code, Code::TypeMismatch);
    assert!(
        e.message.contains("a number or a note name"),
        "{}",
        e.message
    );
}

#[test]
fn schema_offers_number_or_note_name() {
    let schema = duck_echo_core::schema::recipe_schema();
    let freq = &schema["$defs"]["Frequency"]["anyOf"];
    assert_eq!(freq[0]["type"], "number");
    assert_eq!(freq[1]["type"], "string");
    assert_eq!(freq[1]["format"], "note-name");
    assert_eq!(freq[1]["pattern"], duck_echo_core::note::NOTE_PATTERN);
    assert_eq!(
        schema["$defs"]["Pitch"]["properties"]["start"]["$ref"],
        "#/$defs/Frequency"
    );
}

/// A misspelled key or tag with a confident suggestion has its contents
/// checked as the intended name, in the same call.
#[test]
fn misspelled_entries_are_checked_as_the_suggested_name() {
    let toml_src = r#"version = 0

[[layer]]
sorce = "sine"
volume = 3
[layer.pich]
strat = 440

[[effects]]
type = "delay"
feedbak = 0.5
mix = 2
"#;
    let json_src = r#"{"version": 0,
 "layer": [{"sorce": "sine", "volume": 3, "pich": {"strat": 440}}],
 "effects": [{"type": "delay", "feedbak": 0.5, "mix": 2}]}"#;
    for (src, format) in [(toml_src, Format::Toml), (json_src, Format::Json)] {
        let report = load_str(src, format).unwrap_err();
        let mut got: Vec<(&str, Code, Option<&str>)> = report
            .errors
            .iter()
            .map(|e| (e.path.as_str(), e.code, e.suggestion.as_deref()))
            .collect();
        got.sort_by_key(|g| g.0);
        assert_eq!(
            got,
            [
                ("effects[0].feedbak", Code::UnknownField, Some("feedback")),
                ("effects[0].mix", Code::OutOfRange, None),
                ("effects[0].type", Code::UnknownVariant, Some("echo")),
                ("layer", Code::UnknownField, Some("layers")),
                ("layer[0].pich", Code::UnknownField, Some("pitch")),
                ("layer[0].pich.strat", Code::UnknownField, Some("start")),
                ("layer[0].sorce", Code::UnknownField, Some("source")),
                ("layer[0].volume", Code::OutOfRange, None),
            ],
            "{format:?}:\n{report}"
        );
        assert!(report.errors.iter().all(|e| e.line.is_some()), "{report}");
        // No "recipe has no layers": the misspelled table stands in for it.
        assert!(!report.errors.iter().any(|e| e.code == Code::Invalid));
    }
}

#[test]
fn misspelled_entry_next_to_the_real_one_hides_nothing() {
    // Both `pitch` and `pich`: the real one's range error must survive.
    let src = "version = 0\n[[layers]]\n[layers.pitch]\nstart = 0\n[layers.pich]\nstrat = 1\n";
    let report = load_str(src, Format::Toml).unwrap_err();
    let paths: Vec<&str> = report.errors.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"layers[0].pitch.start"), "{report}");
    assert!(paths.contains(&"layers[0].pich.strat"), "{report}");

    // A required field written with a typo is one error, not two.
    let report = load_str("verison = 0\n[[layers]]\n", Format::Toml).unwrap_err();
    assert_eq!(report.errors.len(), 1, "{report}");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("version"));

    // A weak guess does not check contents: no nested noise.
    let report = load_str(
        "version = 0\n[[layers]]\n[layers.pitchbend]\namount = 2\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors.len(), 1, "{report}");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("pitch"));
}

#[test]
fn quoted_numbers_suggest_writing_the_number() {
    for (src, format, path, want) in [
        (
            "version = 0\n[[layers]]\n[layers.pitch]\nstart = \"440\"\n",
            Format::Toml,
            "layers[0].pitch.start",
            "440",
        ),
        (
            r#"{"version":0,"layers":[{"pitch":{"end":"880hz"}}]}"#,
            Format::Json,
            "layers[0].pitch.end",
            "880",
        ),
        (
            "version = 0\n[[layers]]\nvolume = \"0.5\"\n",
            Format::Toml,
            "layers[0].volume",
            "0.5",
        ),
    ] {
        let e = first_error(src, format);
        assert_eq!(e.path, path);
        assert_eq!(e.suggestion.as_deref(), Some(want), "{}", e.message);
        assert!(e.message.contains("without quotes"), "{}", e.message);
    }
}

#[test]
fn compare_recipes_through_the_shared_tools() {
    let dir = examples_dir();
    let same =
        duck_echo_tools::compare_paths(&dir.join("coin.toml"), &dir.join("coin.json")).unwrap();
    assert!(same.identical);
    let diff =
        duck_echo_tools::compare_paths(&dir.join("coin.toml"), &dir.join("jump.toml")).unwrap();
    assert!(!diff.identical);
    assert!(diff.delta.spectral_centroid_ratio.unwrap() < 1.0);
    // A broken recipe is reported as its validation report.
    let bad = std::env::temp_dir().join("duck_echo_compare_bad.toml");
    std::fs::write(&bad, "version = 0\n[[layers]]\nvolume = 9\n").unwrap();
    match duck_echo_tools::compare_paths(&bad, &dir.join("coin.toml")) {
        Err(duck_echo_tools::ToolError::Invalid(r)) => {
            assert_eq!(r.errors[0].path, "layers[0].volume")
        }
        other => panic!("expected an invalid-recipe error, got {other:?}"),
    }
    let _ = std::fs::remove_file(bad);
}

#[test]
fn top_level_key_under_a_table_says_where_it_belongs() {
    let e = first_error(
        "version = 0\n[master]\ngain = 1\nseed = 4\n[[layers]]\n",
        Format::Toml,
    );
    assert_eq!(e.path, "master.seed");
    assert_eq!(e.code, Code::UnknownField);
    assert!(e.message.contains("is a top-level field"), "{}", e.message);
    assert!(
        e.message.contains("above the first [table] header"),
        "{}",
        e.message
    );
}

/// Playtest 1 fixes through the `duck-echo` binary: per-layer analysis,
/// pictures, solo, and errors with specific codes and argument paths.
#[test]
fn cli_analyze_extras_and_error_codes() {
    let exe = env!("CARGO_BIN_EXE_duck-echo");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_extras");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fanfare = examples_dir().join("fanfare.toml");
    let png = dir.join("fanfare.png");
    let out = std::process::Command::new(exe)
        .arg("analyze")
        .arg(&fanfare)
        .args(["--per-layer", "--compact", "--picture"])
        .arg(&png)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["layers"].as_array().unwrap().len(), 4);
    assert_eq!(v["layers"][3]["name"], "bass");
    assert_eq!(v["picture"]["mime_type"], "image/png");
    assert_eq!(&std::fs::read(&png).unwrap()[..4], b"\x89PNG");

    let wav = dir.join("chord.wav");
    let out = std::process::Command::new(exe)
        .arg("render")
        .arg(&fanfare)
        .args(["--solo", "1,2", "--json", "-o"])
        .arg(&wav)
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["heard_layers"], serde_json::json!([1, 2]));

    for (args, code, path, exit) in [
        (vec!["analyze", "no/such.wav"], "not_found", "input", 2),
        (
            vec!["analyze", "preset:coin", "--solo", "4"],
            "out_of_range",
            "--solo[0]",
            1,
        ),
        (
            vec!["render", "preset:gemm", "--json"],
            "not_found",
            "recipe",
            2,
        ),
    ] {
        let out = std::process::Command::new(exe)
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(exit), "{args:?}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["errors"][0]["code"], code, "{args:?}: {v}");
        assert_eq!(v["errors"][0]["path"], path, "{args:?}: {v}");
    }
}

#[test]
fn checked_in_song_schema_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/song.schema.json");
    let on_disk = std::fs::read_to_string(&path).expect("schema file");
    let fresh = duck_echo_core::schema::song_schema_json();
    assert_eq!(
        on_disk.trim(),
        fresh.trim(),
        "schema/song.schema.json is stale; run `duck-echo schema song > schema/song.schema.json`"
    );
}

/// Roadmap task 16 through the `duck-echo` binary: the same commands work
/// on a song as on a recipe.
#[test]
fn cli_renders_validates_and_analyzes_songs() {
    let exe = env!("CARGO_BIN_EXE_duck-echo");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_songs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let song = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs/four_bars.toml");
    let run = |args: &[&str]| {
        std::process::Command::new(exe)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let song_arg = song.to_str().unwrap();

    let out = run(&["validate", song_arg, "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);

    let out = run(&[
        "render",
        song_arg,
        "-o",
        "four.wav",
        "--json",
        "--per-track",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["song"]["bars"], 4);
    assert_eq!(v["song"]["song_frames"], 384_873);
    assert_eq!(v["tracks"][1]["name"], "lead");
    let wav = duck_echo_audio::read_wav(&dir.join("four.wav")).unwrap();
    assert_eq!((wav.sample_rate, wav.channels), (44_100, 2));

    // render checked the notes because per-track detail was asked for.
    assert_eq!(v["note_check"]["notes_heard"], 29);
    // analyze always does, and `--notes` lists each one.
    let out = run(&["analyze", song_arg, "--compact", "--notes"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["note_check"]["notes_written"], 29);
    let notes = v["note_check"]["tracks"][1]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 13);
    assert_eq!(notes[0]["written"], "E5");
    assert!(v["summary"]
        .as_str()
        .unwrap()
        .contains("29 of 29 notes heard"));

    // A stem: the WAV holds only the bass, at its level in the mix.
    let out = run(&[
        "render", song_arg, "-o", "bass.wav", "--json", "--solo", "bass",
    ]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["heard_tracks"], serde_json::json!(["bass"]));

    // The song and its render compare as the same sound.
    let out = run(&["compare", song_arg, "four.wav", "--compact"]);
    assert!(out.status.success());

    // An invalid song: every problem, with paths, and exit code 1.
    std::fs::write(
        dir.join("bad.toml"),
        "version = 0\ntempo = 120\n[instruments.t]\nversion = 0\n[[instruments.t.layers]]\nsource = \"sine\"\n[[tracks]]\nname = \"a\"\ninstrument = \"t\"\nnotes = [{ beat = 0, note = \"C4\" }, { beat = 1, note = \"X9\" }, { beat = 2, nte = \"C4\" }]\n",
    )
    .unwrap();
    let out = run(&["validate", "bad.toml", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let paths: Vec<&str> = v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["tracks[0].notes[2].nte", "tracks[0].notes[1].note"],
        "{v}"
    );
    assert_eq!(v["errors"][0]["suggestion"], "note");
    assert_eq!(v["errors"][0]["line"], 10);

    for (args, code, path, exit) in [
        (
            vec!["analyze", song_arg, "--solo", "drums"],
            "not_found",
            "--solo[0]",
            1,
        ),
        (
            vec!["analyze", song_arg, "--per-layer"],
            "invalid_argument",
            "--per_layer",
            2,
        ),
        (
            vec!["variations", song_arg, "--json"],
            "invalid_argument",
            "recipe",
            2,
        ),
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(exit), "{args:?}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["errors"][0]["code"], code, "{args:?}: {v}");
        assert_eq!(v["errors"][0]["path"], path, "{args:?}: {v}");
    }

    let out = run(&["schema", "song"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["title"], "Duck Echo song v0");
}

/// Roadmap task 20 through the binary: a looping song renders to exactly
/// its bars and reports its join.
#[test]
fn cli_renders_a_loop_to_its_exact_length() {
    let exe = env!("CARGO_BIN_EXE_duck-echo");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_loops");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("loop.toml"),
        "version = 0\ntempo = 120\nbars = 2\nloop = true\n[[tracks]]\nname = \"pad\"\ninstrument = \"pad\"\nnotes = [{ beat = 0, note = [\"C4\", \"E4\", \"G4\"], length = 8 }]\n",
    )
    .unwrap();
    let out = std::process::Command::new(exe)
        .args(["render", "loop.toml", "--json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["song"]["loop"], true);
    assert_eq!(v["song"]["song_frames"], 176_400);
    assert_eq!(v["loop_seam"]["clean"], true);
    let wav = duck_echo_audio::read_wav(&dir.join("loop.wav")).unwrap();
    assert_eq!(wav.samples.len(), 176_400 * 2);
    // The pad's long release and reverb would run past the end; in the
    // loop they are at the start instead, so the file does not begin
    // from silence.
    assert!(wav.samples[..4410].iter().any(|s| s.abs() > 0.001));
}
