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
    let recipe = load_file(&examples_dir().join("laser.toml"));
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
