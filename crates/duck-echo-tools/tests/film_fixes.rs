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
