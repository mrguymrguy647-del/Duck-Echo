//! Roadmap task 6: each new measurement checked on a synthetic signal
//! whose answer is known.

use duck_echo_analyze::{analyze, compare, Signal};
use std::f32::consts::TAU;

const SR: u32 = 44100;

fn sine(hz: f32, seconds: f32, amp: f32) -> Vec<f32> {
    (0..(seconds * SR as f32) as usize)
        .map(|i| amp * (TAU * hz * i as f32 / SR as f32).sin())
        .collect()
}

/// Deterministic white noise in -amp..amp.
fn noise(seconds: f32, amp: f32) -> Vec<f32> {
    let mut state = 0x1234_5678_9abc_def0u64;
    (0..(seconds * SR as f32) as usize)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            amp * (((state >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0)
        })
        .collect()
}

fn mono(xs: &[f32]) -> Signal<'_> {
    Signal {
        samples: xs,
        sample_rate: SR,
        channels: 1,
    }
}

#[test]
fn sine_is_tonal_at_its_frequency_with_3db_crest() {
    let a = analyze(&sine(1000.0, 0.5, 0.5), SR, 1);
    let c = a.spectral_centroid_hz.unwrap();
    assert!((c - 1000.0).abs() < 50.0, "centroid {c}");
    assert!(a.tonality.unwrap() > 0.95, "tonality {:?}", a.tonality);
    assert!((a.crest_factor_db.unwrap() - 3.01).abs() < 0.05);
    assert!(!a.pitch_track.is_empty() && a.pitch_track.len() <= 8);
    for p in &a.pitch_track {
        let f = p.dominant_hz.unwrap();
        assert!((f - 1000.0).abs() < 3.0, "dominant {f}");
        assert!((p.level_dbfs.unwrap() + 9.03).abs() < 0.2);
    }
    assert!(a.summary.contains("tonal, around 1000 Hz"), "{}", a.summary);
}

#[test]
fn white_noise_is_noisy_and_bright() {
    let a = analyze(&noise(0.5, 0.5), SR, 1);
    assert!(a.tonality.unwrap() < 0.1, "tonality {:?}", a.tonality);
    // White noise has a flat spectrum, so its magnitude centroid is about
    // half the Nyquist frequency.
    let c = a.spectral_centroid_hz.unwrap();
    assert!((c - SR as f32 / 4.0).abs() < 1000.0, "centroid {c}");
    assert!(a.summary.contains("noisy"), "{}", a.summary);
    assert!(a.summary.contains("very bright"), "{}", a.summary);
}

#[test]
fn square_wave_has_zero_crest_factor() {
    let sq: Vec<f32> = sine(500.0, 0.3, 1.0)
        .iter()
        .map(|&s| if s >= 0.0 { 0.5 } else { -0.5 })
        .collect();
    let a = analyze(&sq, SR, 1);
    assert!(a.crest_factor_db.unwrap().abs() < 0.01);
}

#[test]
fn attack_and_decay_match_the_envelope() {
    // 100 ms of silence, a 50 ms linear rise, a 200 ms hold, then an
    // exponential fall with time constant 0.1 s.
    let tau = 0.1f32;
    let tone = sine(1000.0, 1.5, 1.0);
    let signal: Vec<f32> = tone
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let t = i as f32 / SR as f32;
            let g = if t < 0.1 {
                0.0
            } else if t < 0.15 {
                (t - 0.1) / 0.05
            } else if t < 0.35 {
                1.0
            } else {
                (-(t - 0.35) / tau).exp()
            };
            s * g * 0.5
        })
        .collect();
    let a = analyze(&signal, SR, 1);
    // 90% of a linear 50 ms ramp is reached after 45 ms.
    let attack = a.attack_seconds.unwrap();
    assert!((attack - 0.045).abs() < 0.005, "attack {attack}");
    // From 90% (tau ln(1/0.9) after the hold) to 10% (tau ln 10).
    let want = (tau * (10f32.ln() - (1.0 / 0.9f32).ln())) as f64;
    let decay = a.decay_seconds.unwrap();
    assert!((decay - want).abs() < 0.006, "decay {decay} want {want}");
    assert!(a.summary.contains("quick attack"), "{}", a.summary);
}

#[test]
fn rising_sweep_is_tracked() {
    // Exponential sweep from 200 to 1600 Hz over 0.4 s.
    let mut phase = 0.0f32;
    let sweep: Vec<f32> = (0..(0.4 * SR as f32) as usize)
        .map(|i| {
            let t = i as f32 / SR as f32;
            let f = 200.0 * 8f32.powf(t / 0.4);
            phase += f / SR as f32;
            0.5 * (TAU * phase).sin()
        })
        .collect();
    let a = analyze(&sweep, SR, 1);
    let first = a.pitch_track.first().unwrap().dominant_hz.unwrap();
    let last = a.pitch_track.last().unwrap().dominant_hz.unwrap();
    assert!(first < 300.0 && last > 1200.0, "{first} {last}");
    let hz: Vec<f32> = a
        .pitch_track
        .iter()
        .map(|p| p.dominant_hz.unwrap())
        .collect();
    assert!(hz.windows(2).all(|w| w[1] > w[0]), "{hz:?}");
    assert!(a.summary.contains("pitch rising"), "{}", a.summary);
}

#[test]
fn silence_has_no_features() {
    let a = analyze(&vec![0.0; 4410], SR, 1);
    assert_eq!(a.spectral_centroid_hz, None);
    assert_eq!(a.tonality, None);
    assert_eq!(a.attack_seconds, None);
    assert_eq!(a.crest_factor_db, None);
    assert!(a.pitch_track.is_empty());
    assert_eq!(a.summary, "0.10 s, silent");
}

#[test]
fn compare_reports_level_shift_inversion_and_identity() {
    let a = sine(440.0, 0.5, 0.5);

    let same = compare(mono(&a), mono(&a));
    assert!(same.identical);
    assert_eq!(same.summary, "identical audio");

    let half: Vec<f32> = a.iter().map(|s| s * 0.5).collect();
    let c = compare(mono(&a), mono(&half));
    assert!(!c.identical);
    assert!((c.delta.loudness_lu.unwrap() + 6.02).abs() < 0.05);
    assert!((c.delta.peak_db.unwrap() + 6.02).abs() < 0.01);
    let corr = c.correlation.unwrap();
    assert!((corr.peak - 1.0).abs() < 1e-4 && corr.lag_seconds == 0.0);
    assert!(c.summary.contains("6.0 LU quieter"), "{}", c.summary);

    let inverted: Vec<f32> = a.iter().map(|s| -s).collect();
    let c = compare(mono(&a), mono(&inverted));
    assert!((c.correlation.unwrap().at_zero_lag + 1.0).abs() < 1e-4);

    let mut delayed = vec![0.0f32; 4410];
    delayed.extend(&a);
    let c = compare(mono(&a), mono(&delayed));
    let corr = c.correlation.unwrap();
    assert!((corr.lag_seconds - 0.1).abs() < 1e-6, "{corr:?}");
    assert!(corr.peak > 0.99);
    assert!((c.delta.duration_seconds - 0.1).abs() < 1e-6);

    let other = sine(1500.0, 0.5, 0.5);
    let c = compare(mono(&a), mono(&other));
    assert!(c.correlation.unwrap().peak.abs() < 0.1);
    assert!(c.delta.spectral_centroid_ratio.unwrap() > 3.0);
    assert!(c.summary.contains("brighter"), "{}", c.summary);
}

#[test]
fn compare_does_not_mix_loudness_methods() {
    // A 0.2 s clip is measured ungated, a 1 s clip gated.
    let short = sine(440.0, 0.2, 0.5);
    let long = sine(440.0, 1.0, 0.5);
    let c = compare(mono(&short), mono(&long));
    assert_eq!(c.delta.loudness_lu, None);
    assert!(c.a.loudness_lufs.is_some() && c.b.loudness_lufs.is_some());
}

#[test]
fn json_output_only_adds_keys() {
    // Every key published before task 6 must still be there.
    let v = serde_json::to_value(analyze(&sine(440.0, 0.5, 0.5), SR, 1)).unwrap();
    for key in [
        "sample_rate",
        "channels",
        "frames",
        "duration_seconds",
        "peak",
        "peak_dbfs",
        "rms",
        "rms_dbfs",
        "loudness_lufs",
        "loudness_method",
        "per_channel",
        "clipping",
        "leading_silence_seconds",
        "trailing_silence_seconds",
        "is_silent",
        "dc_offset",
        "silence_threshold_dbfs",
    ] {
        assert!(v.get(key).is_some(), "missing {key}");
    }
    for key in ["clipped", "clipped_samples", "longest_run", "threshold"] {
        assert!(v["clipping"].get(key).is_some(), "missing clipping.{key}");
    }
}
