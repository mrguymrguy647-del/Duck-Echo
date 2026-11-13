//! Roadmap task 4: pitched noise and LFOs change the sound as the spec
//! says, are checked by validation, and change nothing when left out.

use duck_echo_analyze::analyze;
use duck_echo_core::{load_str, Code, Format, Recipe, Report};

fn recipe(body: &str) -> Recipe {
    load_str(&format!("version = 0\n{body}"), Format::Toml)
        .unwrap_or_else(|r| panic!("{r}"))
        .recipe
}

fn errors(body: &str) -> Report {
    load_str(&format!("version = 0\n{body}"), Format::Toml).expect_err("should be invalid")
}

fn render(body: &str) -> Vec<f32> {
    duck_echo_synth::render(&recipe(body)).unwrap().samples
}

/// RMS level of each slice of `seconds`.
fn levels(samples: &[f32], seconds: f32) -> Vec<f32> {
    let n = (seconds * 44_100.0) as usize;
    samples
        .chunks(n)
        .filter(|c| c.len() == n)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt())
        .collect()
}

const HELD: &str = "envelope = { attack = 0.0, hold = 1.0, decay = 0.0 }";

#[test]
fn pitched_noise_gets_brighter_with_pitch_and_white_noise_ignores_it() {
    let centroid = |body: &str| {
        analyze(&render(body), 44_100, 1)
            .spectral_centroid_hz
            .unwrap()
    };
    let pitched = |hz: u32| {
        centroid(&format!(
            "[[layers]]\nsource = \"noise\"\nnoise_mode = \"pitched\"\n{HELD}\npitch = {{ start = {hz} }}\n"
        ))
    };
    let (low, mid, high) = (pitched(200), pitched(2000), pitched(12_000));
    assert!(low < mid && mid < high, "{low} {mid} {high}");
    // Held noise has sharp steps, so even a low pitch keeps some hiss;
    // what changes is how much. From 200 Hz to 12 kHz it nearly doubles.
    assert!(high > low * 1.6, "{low} {high}");

    // White noise: `pitch` does nothing, and the default is white.
    let white = |extra: &str| render(&format!("[[layers]]\nsource = \"noise\"\n{HELD}\n{extra}"));
    assert_eq!(white(""), white("pitch = { start = 200 }\n"));
    assert_eq!(white(""), white("noise_mode = \"white\"\n"));
    assert_ne!(
        white(""),
        white("noise_mode = \"pitched\"\npitch = { start = 200 }\n")
    );
}

#[test]
fn a_volume_lfo_is_a_tremolo_at_its_rate_and_depth() {
    // 2 Hz, so the level dips twice a second, down to 1 - 0.8 = 0.2.
    let samples = render(&format!(
        "[[layers]]\nsource = \"sine\"\nvolume = 1.0\n{HELD}\npitch = {{ start = 1000 }}\nlfo = {{ target = \"volume\", rate = 2.0, depth = 0.8 }}\n"
    ));
    let l = levels(&samples, 0.01);
    // Loudest at the start of each cycle (0 s and 0.5 s), quietest half a
    // cycle later (0.25 s and 0.75 s). A sine of peak 1 has RMS 0.707.
    for (at, want) in [(0, 0.707), (25, 0.141), (50, 0.707), (75, 0.141)] {
        assert!(
            (l[at] - want).abs() < 0.02,
            "slice {at}: {} vs {want}",
            l[at]
        );
    }
    let dips = l
        .windows(3)
        .filter(|w| w[1] < w[0] && w[1] <= w[2] && w[1] < 0.2)
        .count();
    assert_eq!(dips, 2);

    // `delay` holds the tremolo back.
    let delayed = render(&format!(
        "[[layers]]\nsource = \"sine\"\nvolume = 1.0\n{HELD}\npitch = {{ start = 1000 }}\nlfo = {{ target = \"volume\", rate = 2.0, depth = 0.8, delay = 0.5 }}\n"
    ));
    let l = levels(&delayed, 0.01);
    assert!(l[..49].iter().all(|x| (x - 0.707).abs() < 0.02));
    assert!((l[75] - 0.141).abs() < 0.02);
}

#[test]
fn a_cutoff_lfo_moves_the_brightness_and_a_duty_lfo_the_pulse() {
    // A saw through a low-pass at 800 Hz, moved 2 octaves each way once
    // a second: open (3200 Hz) a quarter of a second in, closed (200 Hz)
    // three quarters in.
    let body = format!(
        "[[layers]]\nsource = \"saw\"\n{HELD}\npitch = {{ start = 110 }}\nfilter = {{ cutoff = 800 }}\nlfo = {{ target = \"cutoff\", rate = 1.0, depth = 2.0 }}\n"
    );
    let samples = render(&body);
    let slice = |from: f32| {
        let a = (from * 44_100.0) as usize;
        analyze(&samples[a..a + 4410], 44_100, 1)
            .spectral_centroid_hz
            .unwrap()
    };
    let (start, open, closed) = (slice(0.0), slice(0.2), slice(0.7));
    assert!(open > start * 1.5, "{start} {open}");
    assert!(closed < start * 0.7, "{start} {closed}");

    // Without the LFO the sound is steady.
    let steady = render(&body.replace(
        "lfo = { target = \"cutoff\", rate = 1.0, depth = 2.0 }\n",
        "",
    ));
    let a = analyze(&steady[..4410], 44_100, 1)
        .spectral_centroid_hz
        .unwrap();
    let b = analyze(&steady[30_870..35_280], 44_100, 1)
        .spectral_centroid_hz
        .unwrap();
    assert!((a / b - 1.0).abs() < 0.05, "{a} {b}");

    // Duty: a square wave at 50% has no even harmonics; moving the duty
    // away from 50% brings the second harmonic in and out.
    let pulse = render(&format!(
        "[[layers]]\nsource = \"square\"\n{HELD}\npitch = {{ start = 220 }}\nlfo = {{ target = \"duty\", rate = 1.0, depth = 0.3 }}\n"
    ));
    let plain = render(&format!(
        "[[layers]]\nsource = \"square\"\n{HELD}\npitch = {{ start = 220 }}\n"
    ));
    assert_ne!(pulse, plain);
    // At the very start the offset is 0, so the two begin alike.
    let early = pulse[..40]
        .iter()
        .zip(&plain[..40])
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(early < 0.05, "{early}");
}

#[test]
fn lfo_and_noise_mode_are_validated() {
    let one = |body: &str| {
        let report = errors(body);
        assert_eq!(report.errors.len(), 1, "{report}");
        report.errors[0].clone()
    };
    let e = one("[[layers]]\nsource = \"saw\"\nlfo = { target = \"cutoff\" }\n");
    assert_eq!(
        (e.path.as_str(), e.code),
        ("layers[0].lfo.target", Code::Invalid)
    );
    assert!(e.message.contains("needs a filter"));

    let e = one("[[layers]]\nsource = \"saw\"\nlfo = { target = \"duty\" }\n");
    assert_eq!(e.path, "layers[0].lfo.target");
    assert!(e.message.contains("square"));

    // The depth range depends on the target.
    let e = one("[[layers]]\nlfo = { target = \"volume\", depth = 1.5 }\n");
    assert_eq!(
        (e.path.as_str(), e.code),
        ("layers[0].lfo.depth", Code::OutOfRange)
    );
    assert!(recipe(
        "[[layers]]\nfilter = { cutoff = 500 }\nlfo = { target = \"cutoff\", depth = 1.5 }\n"
    )
    .layers[0]
        .lfo
        .is_some());
    let e = one("[[layers]]\nlfo = { target = \"duty\", depth = 0.6 }\n");
    assert_eq!(e.path, "layers[0].lfo.depth");
    // The default depth works for every target.
    assert!(
        recipe("[[layers]]\nlfo = { target = \"duty\" }\n").layers[0]
            .lfo
            .is_some()
    );

    let e = one("[[layers]]\nlfo = { rate = 0 }\n");
    assert_eq!(e.path, "layers[0].lfo.rate");

    // Typos get suggestions, like every other field.
    let e = one("[[layers]]\nlfo = { target = \"volum\" }\n");
    assert_eq!(e.suggestion.as_deref(), Some("volume"));
    let e = one("[[layers]]\nsource = \"noise\"\nnoise_mode = \"pitch\"\n");
    assert_eq!(e.path, "layers[0].noise_mode");
    assert_eq!(e.suggestion.as_deref(), Some("pitched"));
    let e = one("[[layers]]\nlfo = { taget = \"volume\" }\n");
    assert_eq!(e.suggestion.as_deref(), Some("target"));

    // noise_mode on another source does nothing, and says so.
    let loaded = load_str(
        "version = 0\n[[layers]]\nsource = \"sine\"\nnoise_mode = \"pitched\"\n",
        Format::Toml,
    )
    .unwrap();
    assert_eq!(loaded.report.warnings[0].path, "layers[0].noise_mode");
    assert_eq!(loaded.report.warnings[0].code, Code::Unused);
}

#[test]
fn pitched_noise_follows_the_note_in_a_song() {
    use duck_echo_core::{load_song_str, NoInstruments};
    let song = |note: &str| {
        let loaded = load_song_str(
            &format!(
                "version = 0\ntempo = 60\nbars = 1\nchannels = 1\n[instruments.hat]\nversion = 0\n[[instruments.hat.layers]]\nsource = \"noise\"\nnoise_mode = \"pitched\"\nenvelope = {{ attack = 0.0, hold = 0.5, decay = 0.0 }}\npitch = {{ start = \"C5\" }}\n[[tracks]]\nname = \"t\"\ninstrument = \"hat\"\nnotes = [{{ beat = 0, note = \"{note}\", length = 0.5 }}]\n"
            ),
            Format::Toml,
            &NoInstruments,
        )
        .unwrap();
        let buf = duck_echo_synth::song::render_song(&loaded.song, &loaded.arrangement).0;
        analyze(&buf.samples, buf.sample_rate, buf.channels)
            .spectral_centroid_hz
            .unwrap()
    };
    let (low, high) = (song("C3"), song("C8"));
    assert!(high > low * 1.4, "{low} {high}");
}
