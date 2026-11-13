//! Roadmap task 3: `aliasing = "clean"` keeps high notes free of folded
//! tones. Each test renders a steady tone through the normal recipe path
//! and looks at its spectrum with the analyzer's own FFT.

use duck_echo_analyze::fft::power_spectrum;
use duck_echo_core::{load_str, Format};

const SR: f64 = 44_100.0;
/// FFT length: 0.74 s at 44.1 kHz, so each bin is 1.35 Hz wide.
const N: usize = 32_768;

/// Render one steady layer and return its middle `N` samples.
fn steady(source: &str, hz: f64, aliasing: &str) -> Vec<f32> {
    let src = format!(
        "version = 0\n[[layers]]\nsource = \"{source}\"\naliasing = \"{aliasing}\"\nvolume = 0.5\n\
         envelope = {{ attack = 0.0, hold = 1.0, decay = 0.0 }}\npitch = {{ start = {hz} }}\n"
    );
    let recipe = load_str(&src, Format::Toml).expect("valid recipe").recipe;
    let buf = duck_echo_synth::render(&recipe).unwrap();
    let start = (buf.samples.len() - N) / 2;
    buf.samples[start..start + N].to_vec()
}

/// The strongest part of the spectrum that is not a harmonic of `f0`, in dB
/// relative to the fundamental, and where it is. Harmonics are every
/// multiple of `f0` below half the sample rate; anything else in a pure
/// tone is aliasing.
fn worst_alias_db(samples: &[f32], f0: f64) -> (f64, f64) {
    let p = power_spectrum(samples, N);
    let bin_hz = SR / N as f64;
    let bin = |hz: f64| (hz / bin_hz).round() as usize;
    // Hann-window leakage is far below -100 dB 30 bins (40 Hz) away.
    let guard = 30;
    let fundamental = p[bin(f0) - 3..=bin(f0) + 3]
        .iter()
        .cloned()
        .fold(0.0, f64::max);
    let mut is_harmonic = vec![false; p.len()];
    let mut k = 1.0;
    while k * f0 < SR / 2.0 {
        let c = bin(k * f0);
        for flag in &mut is_harmonic[c.saturating_sub(guard)..(c + guard + 1).min(p.len())] {
            *flag = true;
        }
        k += 1.0;
    }
    let (worst, at) = (bin(50.0)..p.len())
        .filter(|&i| !is_harmonic[i])
        .map(|i| (p[i], i))
        .fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a });
    (10.0 * (worst / fundamental).log10(), at as f64 * bin_hz)
}

#[test]
fn clean_2khz_saw_keeps_folded_tones_40db_down() {
    let (clean, at) = worst_alias_db(&steady("saw", 2000.0, "clean"), 2000.0);
    assert!(
        clean <= -40.0,
        "clean saw: worst folded tone {clean:.1} dB at {at:.0} Hz"
    );
    // The same measurement catches the raw wave, so the test can fail.
    let (retro, _) = worst_alias_db(&steady("saw", 2000.0, "retro"), 2000.0);
    assert!(
        retro > -30.0,
        "retro saw should alias, measured {retro:.1} dB"
    );
    println!("2 kHz saw: clean {clean:.1} dB (at {at:.0} Hz), retro {retro:.1} dB");
}

#[test]
fn clean_2khz_square_keeps_folded_tones_40db_down() {
    let (clean, at) = worst_alias_db(&steady("square", 2000.0, "clean"), 2000.0);
    assert!(
        clean <= -40.0,
        "clean square: worst folded tone {clean:.1} dB at {at:.0} Hz"
    );
    let (retro, _) = worst_alias_db(&steady("square", 2000.0, "retro"), 2000.0);
    assert!(
        retro > -30.0,
        "retro square should alias, measured {retro:.1} dB"
    );
    println!("2 kHz square: clean {clean:.1} dB (at {at:.0} Hz), retro {retro:.1} dB");
}
