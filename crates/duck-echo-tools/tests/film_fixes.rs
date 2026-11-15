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
