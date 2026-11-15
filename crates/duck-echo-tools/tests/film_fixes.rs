//! What making the sound for "Pick a Door: Blocks" (a 58.6 s horror film
//! made with Duck Forge) taught us, item by item.

use duck_echo_core::{load_str, Format};

fn recipe(text: &str) -> duck_echo_core::Recipe {
    load_str(text, Format::Toml)
        .unwrap_or_else(|r| panic!("{r}"))
        .recipe
}

/// Item 1: a layer longer than 30 s used to be cut at 30 s without a word.
#[test]
fn a_45_second_layer_plays_for_45_seconds() {
    let r = recipe(
        "version = 0\nsample_rate = 8000\n[[layers]]\nsource = \"noise\"\nvolume = 0.3\nenvelope = { attack = 2, hold = 25, decay = 18 }\n",
    );
    let buffer = duck_echo_tools::render_buffer(&r).unwrap();
    let seconds = buffer.duration_seconds();
    assert!((seconds - 45.0).abs() < 0.01, "{seconds} s");
    // It fades out by itself, so the end is quiet, not cut.
    let tail = &buffer.samples[buffer.samples.len() - 80..];
    assert!(tail.iter().all(|s| s.abs() < 0.01), "{tail:?}");
}

fn analysis(text: &str) -> duck_echo_analyze::Analysis {
    let buffer = duck_echo_tools::render_buffer(&recipe(text)).unwrap();
    duck_echo_tools::analyze_buffer(&buffer)
}

fn has_peak(a: &duck_echo_analyze::Analysis, hz: f32) -> bool {
    a.peaks
        .iter()
        .any(|p| (p.hz - hz).abs() < 4.0 && p.db > -30.0)
}

/// Item 5: an LFO on volume or pitch may move at audio rates, and the
/// side tones (the note plus and minus the rate) show in `peaks`.
#[test]
fn an_audio_rate_wobble_makes_side_tones_at_the_note_plus_and_minus_the_rate() {
    // Volume at 300 per second ("ring" style): 700 and 1300 Hz.
    let ring = analysis(
        "version = 0\n[[layers]]\nsource = \"sine\"\npitch = { start = 1000 }\nenvelope = { attack = 0.01, hold = 1, decay = 0.1 }\nlfo = { target = \"volume\", rate = 300, depth = 1 }\n",
    );
    for hz in [1000.0, 700.0, 1300.0] {
        assert!(has_peak(&ring, hz), "no peak at {hz}: {:?}", ring.peaks);
    }
    // Pitch at 150 per second, half a semitone (FM style): 850 and
    // 1150 Hz.
    let fm = analysis(
        "version = 0\n[[layers]]\nsource = \"sine\"\npitch = { start = 1000 }\nenvelope = { attack = 0.01, hold = 1, decay = 0.1 }\nlfo = { target = \"pitch\", rate = 150, depth = 0.5 }\n",
    );
    for hz in [1000.0, 850.0, 1150.0] {
        assert!(has_peak(&fm, hz), "no peak at {hz}: {:?}", fm.peaks);
    }
    // A slow wobble makes no side tones a peak list can see apart.
    let slow = analysis(
        "version = 0\n[[layers]]\nsource = \"sine\"\npitch = { start = 1000 }\nenvelope = { attack = 0.01, hold = 1, decay = 0.1 }\nlfo = { target = \"volume\", rate = 4, depth = 0.5 }\n",
    );
    assert!(has_peak(&slow, 1000.0));
    assert!(!has_peak(&slow, 700.0));
}

/// Item 5: only volume and pitch go past 50 per second, and a pitch LFO
/// needs a pitch.
#[test]
fn audio_rates_are_for_volume_and_pitch_only() {
    let report = load_str(
        "version = 0\n[[layers]]\nsource = \"saw\"\nfilter = { cutoff = 800 }\nlfo = { target = \"cutoff\", rate = 300, depth = 1 }\n",
        Format::Toml,
    )
    .unwrap_err();
    let issue = &report.errors[0];
    assert_eq!(issue.path, "layers[0].lfo.rate");
    assert!(issue.message.contains("audio rates"), "{}", issue.message);
    let report = load_str(
        "version = 0\n[[layers]]\nsource = \"noise\"\nlfo = { target = \"pitch\", rate = 30, depth = 1 }\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "layers[0].lfo.target");
    // 2000 per second on volume is fine; 2001 is not.
    assert!(load_str(
        "version = 0\n[[layers]]\nlfo = { target = \"volume\", rate = 2000, depth = 0.5 }\n",
        Format::Toml
    )
    .is_ok());
    assert!(load_str(
        "version = 0\n[[layers]]\nlfo = { target = \"volume\", rate = 2001, depth = 0.5 }\n",
        Format::Toml
    )
    .is_err());
}

/// Item 6: a recipe says what it should be, and analyze checks each line.
#[test]
fn expect_lines_pass_or_fail_against_the_analysis() {
    let growl = "version = 0\nexpect = { roughness = \"high\", boom = \"low\", loudness_lufs = [-30, -5], duration_seconds = [0.9, 1.2] }\n[[layers]]\nsource = \"sine\"\npitch = { start = 300 }\nenvelope = { attack = 0.02, hold = 0.9, decay = 0.1 }\nlfo = { target = \"volume\", rate = 70, depth = 1 }\n";
    let loaded = load_str(growl, Format::Toml).unwrap();
    let m = duck_echo_tools::measure::measure_recipe(&loaded.recipe, &[], &Default::default())
        .unwrap()
        .measured;
    let e = m.expect.expect("checked");
    assert!(e.pass, "{e:?}");
    assert_eq!(e.lines.len(), 4);
    assert!(
        m.analysis.summary.ends_with("expect: all 4 met"),
        "{}",
        m.analysis.summary
    );
    assert!(
        m.analysis.summary.contains("rough"),
        "{}",
        m.analysis.summary
    );

    // The same tone with no wobble is not rough: that line fails, and the
    // summary says which.
    let smooth = growl.replace("lfo = { target = \"volume\", rate = 70, depth = 1 }\n", "");
    let loaded = load_str(&smooth, Format::Toml).unwrap();
    let m = duck_echo_tools::measure::measure_recipe(&loaded.recipe, &[], &Default::default())
        .unwrap()
        .measured;
    let e = m.expect.unwrap();
    assert!(!e.pass);
    assert_eq!(e.met, 3);
    let line = e.lines.iter().find(|l| l.what == "roughness").unwrap();
    assert!(!line.pass && line.got.as_deref().unwrap().starts_with("low"));
    assert!(
        m.analysis
            .summary
            .contains("EXPECT: 3 of 4 met (roughness is low, wanted high)"),
        "{}",
        m.analysis.summary
    );

    // A range written the wrong way round, or not as two numbers, is an
    // error at its path.
    let report = load_str(
        "version = 0\nexpect = { loudness_lufs = [-16, -20] }\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "expect.loudness_lufs");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("[-20, -16]"));
    let report = load_str(
        "version = 0\nexpect = { peak_dbfs = [-1] }\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "expect.peak_dbfs");
    // An unknown line is an error with a suggestion, like any field.
    let report = load_str(
        "version = 0\nexpect = { roughnes = \"high\" }\n[[layers]]\n",
        Format::Toml,
    )
    .unwrap_err();
    assert_eq!(report.errors[0].path, "expect.roughnes");
    assert_eq!(report.errors[0].suggestion.as_deref(), Some("roughness"));
}

/// Item 3: every rendered WAV gets its timing and level facts next to it.
#[test]
fn a_render_writes_its_timing_and_level_facts_next_to_the_wav() {
    let dir = std::env::temp_dir().join(format!("duck-echo-asset-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A thump: silent for 50 ms, then a hit whose loudest moment comes a
    // little after its start.
    let text = "version = 0\n[[layers]]\nsource = \"sine\"\noffset = 0.05\npitch = { start = 70, end = 40, sweep_time = 0.2 }\nenvelope = { attack = 0.006, decay = 0.3 }\n";
    let loaded = load_str(text, Format::Toml).unwrap();
    let out = dir.join("thump.wav");
    let rendered = duck_echo_tools::render_to_file(
        &loaded,
        &out,
        None,
        duck_echo_audio::WavFormat::Pcm16,
        &Default::default(),
    )
    .unwrap();
    let sidecar = dir.join("thump.wav.json");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).unwrap()).unwrap();
    // The file and render --json say the same.
    let said: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&rendered.asset).unwrap()).unwrap();
    assert_eq!(written, said);
    assert_eq!(written["kind"], "duck-echo-asset");
    assert_eq!(written["file"], "thump.wav");
    assert_eq!(written["loops"], false);
    let leading = written["leading_silence_seconds"].as_f64().unwrap();
    assert!((leading - 0.05).abs() < 0.002, "{leading}");
    // The hit is after the first sample: the attack takes 6 ms.
    let hit = written["hit_seconds"].as_f64().unwrap();
    assert!(hit > leading + 0.002 && hit < leading + 0.02, "{hit}");
    assert!(written["loudness_lufs"].is_number());
    assert!(written["peak_dbfs"].as_f64().unwrap() <= 0.0);
    // The hash follows the recipe's data, not its comments.
    let hash = written["recipe_hash"].as_str().unwrap().to_string();
    assert!(
        hash.starts_with("sha256:") && hash.len() == 7 + 64,
        "{hash}"
    );
    let commented = load_str(&format!("# A thump.\n{text}"), Format::Toml).unwrap();
    let again = duck_echo_tools::render_to_file(
        &commented,
        &out,
        None,
        duck_echo_audio::WavFormat::Pcm16,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(again.asset.recipe_hash, hash);
    let louder = load_str(
        &text.replace("source", "volume = 0.5\nsource"),
        Format::Toml,
    )
    .unwrap();
    let other = duck_echo_tools::render_to_file(
        &louder,
        &out,
        None,
        duck_echo_audio::WavFormat::Pcm16,
        &Default::default(),
    )
    .unwrap();
    assert_ne!(other.asset.recipe_hash, hash);
    std::fs::remove_dir_all(&dir).ok();
}

/// Item 7: any number of a sound may be written as `{ min = x, max = y }`.
const RANGED: &str = "version = 0\nsample_rate = 8000\n[[layers]]\nsource = \"saw\"\nvolume = { min = 0.3, max = 0.9 }\npitch = { start = { min = 300, max = 500 } }\nfilter = { kind = \"lowpass\", cutoff = 2000 }\nenvelope = { attack = 0.001, decay = { min = 0.05, max = 0.15 } }\n";

#[test]
fn a_ranged_field_stays_within_its_bounds_across_100_seeds() {
    let r = recipe(RANGED);
    assert_eq!(r.ranges.len(), 3, "{:?}", r.ranges);
    assert_eq!(r.ranges[0].path, "layers[0].volume");
    assert_eq!(r.ranges[1].path, "layers[0].pitch.start");
    assert_eq!(r.ranges[2].path, "layers[0].envelope.decay");
    let mut volumes = Vec::new();
    for seed in 0..100 {
        let picked = duck_echo_core::random::resolve(&r, seed);
        assert!(picked.ranges.is_empty());
        let layer = &picked.layers[0];
        assert!((0.3..=0.9).contains(&layer.volume), "{}", layer.volume);
        let hz = layer.pitch.start.hz();
        assert!((300.0..=500.0).contains(&hz), "{hz}");
        assert!((0.05..=0.15).contains(&layer.envelope.decay));
        volumes.push(layer.volume);
    }
    volumes.sort_by(f32::total_cmp);
    volumes.dedup();
    assert!(volumes.len() > 90, "the seeds pick different values");
    // Low seeds and high seeds both reach both halves of the range.
    assert!(volumes[0] < 0.4 && volumes[volumes.len() - 1] > 0.8);
}

#[test]
fn renders_with_ranges_are_the_same_for_a_seed_and_differ_between_seeds() {
    let mut r = recipe(RANGED);
    let render = |r: &duck_echo_core::Recipe| duck_echo_tools::render_buffer(r).unwrap().samples;
    r.seed = 4;
    let a = render(&r);
    assert_eq!(a, render(&r), "same recipe and seed, same samples");
    r.seed = 5;
    let b = render(&r);
    assert_ne!(a.len().min(b.len()), 0);
    assert_ne!(a, b);
    // The length follows the picked decay.
    let picked = duck_echo_core::random::resolve(&r, 5);
    let seconds = b.len() as f32 / 8000.0;
    assert!(
        (seconds - (0.001 + picked.layers[0].envelope.decay)).abs() < 0.01,
        "{seconds}"
    );
}

#[test]
fn variation_and_ranges_work_together() {
    let mut r = recipe(RANGED);
    r.variation = 0.5;
    r.seed = 9;
    let both = duck_echo_tools::render_buffer(&r).unwrap().samples;
    // The same as picking the ranges first and then letting `variation`
    // jitter the picked recipe with the same seed.
    let mut picked = duck_echo_core::random::resolve(&r, 9);
    picked.seed = 9;
    assert_eq!(
        both,
        duck_echo_tools::render_buffer(&picked).unwrap().samples
    );
    // And not the same as the ranges alone.
    picked.variation = 0.0;
    assert_ne!(
        both,
        duck_echo_tools::render_buffer(&picked).unwrap().samples
    );
}

#[test]
fn a_recipe_written_as_json_takes_ranges_too() {
    let json =
        r#"{"version": 0, "layers": [{"source": "sine", "volume": {"min": 0.2, "max": 0.4}}]}"#;
    let loaded = load_str(json, Format::Json).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(loaded.recipe.ranges[0].path, "layers[0].volume");
    assert_eq!(loaded.recipe.ranges[0].max, 0.4);
}

#[test]
fn range_errors_point_at_the_end_at_fault() {
    let errors = |text: &str| -> Vec<(String, String)> {
        let report = load_str(text, Format::Toml).unwrap_err();
        report
            .errors
            .iter()
            .map(|e| (e.path.clone(), format!("{:?}", e.code)))
            .collect()
    };
    let base = "version = 0\n[[layers]]\nsource = \"sine\"\n";
    // Above the field's own range at one end.
    let e = errors(&format!("{base}volume = {{ min = 0.5, max = 3 }}\n"));
    assert_eq!(e, [("layers[0].volume.max".into(), "OutOfRange".into())]);
    let e = errors(&format!("{base}volume = {{ min = -1, max = 0.5 }}\n"));
    assert_eq!(e, [("layers[0].volume.min".into(), "OutOfRange".into())]);
    // min above max.
    let e = errors(&format!("{base}volume = {{ min = 0.9, max = 0.3 }}\n"));
    assert_eq!(e, [("layers[0].volume.max".into(), "OutOfRange".into())]);
    // A missing end, a stranger and a word.
    let e = errors(&format!("{base}volume = {{ min = 0.5 }}\n"));
    assert_eq!(e, [("layers[0].volume.max".into(), "MissingField".into())]);
    let e = errors(&format!(
        "{base}volume = {{ min = 0.5, max = 0.6, mx = 1 }}\n"
    ));
    assert_eq!(e, [("layers[0].volume.mx".into(), "UnknownField".into())]);
    let e = errors(&format!("{base}volume = {{ min = \"low\", max = 0.6 }}\n"));
    assert_eq!(e, [("layers[0].volume.min".into(), "TypeMismatch".into())]);
    // Whole numbers (the seed, the sample rate) take no range.
    let e = errors(&format!("seed = {{ min = 1, max = 2 }}\n{base}"));
    assert_eq!(e, [("seed".into(), "TypeMismatch".into())]);
    // Neither do the lines of `expect`.
    let e = errors(&format!(
        "expect = {{ loudness_lufs = [{{ min = 1, max = 2 }}, -16] }}\n{base}"
    ));
    assert_eq!(e[0].0, "expect.loudness_lufs[0]");
}

#[test]
fn a_song_instruments_ranges_play_note_by_note_but_a_song_field_takes_none() {
    use duck_echo_core::NoInstruments;
    let song = "version = 0\ntempo = 120\nbars = 1\n[instruments.tick]\nversion = 0\n[[instruments.tick.layers]]\nsource = \"sine\"\nvolume = { min = 0.2, max = 0.6 }\nenvelope = { attack = 0.001, decay = 0.05 }\n[[tracks]]\nname = \"t\"\ninstrument = \"tick\"\nnotes = [{ beat = 0, note = \"C4\" }, { beat = 1, note = \"C4\" }]\n";
    let loaded = duck_echo_core::load_song_str(song, Format::Toml, &NoInstruments)
        .unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(
        loaded.song.instruments["tick"].ranges[0].path,
        "layers[0].volume"
    );
    let bad = song.replace("tempo = 120", "tempo = { min = 100, max = 140 }");
    let report = duck_echo_core::load_song_str(&bad, Format::Toml, &NoInstruments).unwrap_err();
    assert_eq!(report.errors[0].path, "tempo", "{report}");
}

#[test]
fn sound_variants_exports_numbered_files_and_a_list_to_pick_from() {
    use duck_echo_tools::commands;
    let dir = std::env::temp_dir().join(format!("duck-echo-variants-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sfx")).unwrap();
    std::fs::write(dir.join("sfx/step.toml"), RANGED).unwrap();
    let root = duck_echo_tools::root::Root::new(&dir).unwrap();
    let project = duck_echo_tools::project::Project::new(&root);
    let run = |args: serde_json::Value| {
        commands::parse("sound_variants", &args, "arguments.").and_then(|c| {
            commands::run(
                &project,
                c,
                duck_echo_tools::history::Who::Agent,
                None,
                "arguments.",
            )
        })
    };
    let answer = run(serde_json::json!({"file": "sfx/step.toml", "count": 5})).unwrap();
    assert_eq!(answer["list_file"], "sfx/step.variants.json");
    let list: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("sfx/step.variants.json")).unwrap())
            .unwrap();
    assert_eq!(list["recipe_hash"], answer["list"]["recipe_hash"]);
    assert_eq!(list["files"][4]["file"], answer["list"]["files"][4]["file"]);
    assert_eq!(list["kind"], "duck-echo-variants");
    assert_eq!(list["pick"], "random");
    assert_eq!(list["seeds_matter"], true);
    let files = list["files"].as_array().unwrap();
    assert_eq!(files.len(), 5);
    let mut wavs = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let name = format!("step_{}.wav", i + 1);
        assert_eq!(f["file"], name.as_str());
        assert_eq!(f["seed"], i as u64 + 1);
        assert!(dir.join("sfx").join(format!("{name}.json")).is_file());
        wavs.push(std::fs::read(dir.join("sfx").join(&name)).unwrap());
    }
    wavs.dedup();
    assert_eq!(wavs.len(), 5, "every variant is different");
    // Exporting again writes the very same files.
    let first = std::fs::read(dir.join("sfx/step_3.wav")).unwrap();
    run(serde_json::json!({"file": "sfx/step.toml", "count": 5})).unwrap();
    assert_eq!(first, std::fs::read(dir.join("sfx/step_3.wav")).unwrap());
    // Another folder and name, and a preset with nothing to vary.
    let answer = run(serde_json::json!({"file": "preset:coin", "count": 2, "folder": "out/coins", "name": "pling"})).unwrap();
    assert_eq!(answer["list_file"], "out/coins/pling.variants.json");
    assert_eq!(answer["list"]["seeds_matter"], false);
    assert!(answer["list"]["note"].as_str().unwrap().contains("range"));
    // Bad arguments say where.
    let e = run(serde_json::json!({"file": "sfx/step.toml", "count": 0})).unwrap_err();
    assert_eq!(e.into_report().errors[0].path, "arguments.count");
    let e =
        run(serde_json::json!({"file": "sfx/step.toml", "count": 2, "variation": 2})).unwrap_err();
    assert_eq!(e.into_report().errors[0].path, "arguments.variation");
    let e = run(serde_json::json!({"file": "../step.toml", "count": 2})).unwrap_err();
    assert_eq!(e.into_report().errors[0].path, "arguments.file");
}
