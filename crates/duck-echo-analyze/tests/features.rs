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

    // A burst of noise turned upside down: the summary says so, instead
    // of calling a correlation of -1 "partly similar" (playtest 2).
    let mut seed = 12345u32;
    let burst: Vec<f32> = (0..22_050)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            noise * 0.5 * (-(i as f32) / 4000.0).exp()
        })
        .collect();
    let flipped: Vec<f32> = burst.iter().map(|s| -s).collect();
    let c = compare(mono(&burst), mono(&flipped));
    assert!(c.correlation.as_ref().unwrap().peak < -0.99);
    assert!(
        c.summary
            .contains("waveforms closely match, upside down (-1.00)"),
        "{}",
        c.summary
    );
    let c = compare(
        mono(&burst),
        mono(&burst.iter().map(|s| s * 0.8).collect::<Vec<_>>()),
    );
    assert!(!c.summary.contains("upside down"), "{}", c.summary);

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

#[test]
fn a_weighting_matches_the_standard() {
    use duck_echo_analyze::features::a_weight_power;
    let db = |f: f64| 10.0 * a_weight_power(f).log10();
    // IEC 61672 reference values.
    assert!(db(1000.0).abs() < 0.05);
    assert!((db(100.0) + 19.1).abs() < 0.1, "{}", db(100.0));
    assert!((db(50.0) + 30.2).abs() < 0.1, "{}", db(50.0));
    assert!((db(10_000.0) + 2.5).abs() < 0.1, "{}", db(10_000.0));
}

#[test]
fn sub_bass_tone_under_noise_reads_noisy() {
    // Most raw power is the 60 Hz sine, but the ear hears mostly the noise.
    let tone = sine(60.0, 0.5, 0.6);
    let hiss = noise(0.5, 0.2);
    let mix: Vec<f32> = tone.iter().zip(&hiss).map(|(a, b)| a + b).collect();
    let a = analyze(&mix, SR, 1);
    assert!(a.tonality.unwrap() < 0.3, "tonality {:?}", a.tonality);
    assert!(a.summary.contains("noisy"), "{}", a.summary);
}

#[test]
fn low_harmonic_tone_reads_tonal() {
    // A 55 Hz saw: harmonics only 2.5 bins apart at 46 ms, resolved by the
    // longer frames.
    let saw: Vec<f32> = (0..SR as usize)
        .map(|i| {
            let ph = (55.0 * i as f32 / SR as f32).fract();
            0.5 * (2.0 * ph - 1.0)
        })
        .collect();
    let a = analyze(&saw, SR, 1);
    assert!(a.tonality.unwrap() > 0.9, "tonality {:?}", a.tonality);
    assert!(a.summary.contains("tonal, around 55 Hz"), "{}", a.summary);
}

#[test]
fn faint_tail_does_not_decide_pitch_direction() {
    // A loud 1 kHz tone, then a tail at 300 Hz 40 dB down.
    let mut s = sine(1000.0, 0.4, 0.5);
    s.extend(sine(300.0, 0.3, 0.005));
    let a = analyze(&s, SR, 1);
    let quiet = a
        .pitch_track
        .iter()
        .any(|p| p.dominant_hz.is_some_and(|f| (f - 300.0).abs() < 10.0));
    assert!(quiet, "the tail should still appear in the track");
    assert!(a.summary.contains("around 1000 Hz"), "{}", a.summary);
    assert!(!a.summary.contains("falling"), "{}", a.summary);
}

/// Playtest 1: onset times and the pitch that starts at each.
fn onset_notes(a: &duck_echo_analyze::Analysis) -> Vec<(f64, Option<f32>)> {
    a.onsets
        .iter()
        .map(|o| (o.time_seconds, o.note_hz))
        .collect()
}

fn close(got: &[(f64, Option<f32>)], want: &[(f64, Option<f32>)]) -> bool {
    got.len() == want.len()
        && got.iter().zip(want).all(|((t, hz), (wt, whz))| {
            (t - wt).abs() < 0.012
                && match (hz, whz) {
                    (Some(a), Some(b)) => (12.0 * (a / b).log2()).abs() < 0.3,
                    (None, None) => true,
                    _ => false,
                }
        })
}

#[test]
fn onsets_find_beeps_after_silence_with_their_pitch() {
    let mut s = vec![0.0f32; (0.05 * SR as f32) as usize];
    s.extend(sine(440.0, 0.1, 0.5));
    s.extend(vec![0.0; (0.1 * SR as f32) as usize]);
    s.extend(sine(880.0, 0.1, 0.3));
    s.extend(vec![0.0; (0.05 * SR as f32) as usize]);
    let a = analyze(&s, SR, 1);
    let got = onset_notes(&a);
    assert!(
        close(&got, &[(0.05, Some(440.0)), (0.25, Some(880.0))]),
        "{got:?}"
    );
    assert!(a.summary.contains("2 onsets"), "{}", a.summary);
    assert!((a.onsets[1].level_dbfs.unwrap() - (-13.5)).abs() < 0.5);
}

#[test]
fn onsets_find_steps_without_gaps_and_name_the_new_note_over_a_bass() {
    // A melody stepping every 120 ms over a constant, louder bass: each
    // onset names the melody note, not the bass.
    let notes = [523.25f32, 659.26, 783.99, 1046.5];
    let step = (0.12 * SR as f32) as usize;
    let mut s = vec![0.0f32; step * notes.len()];
    for (i, x) in s.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        let hz = notes[i / step];
        *x = 0.5 * (TAU * 110.0 * t).sin() + 0.25 * (TAU * hz * t).sin();
    }
    let got = onset_notes(&analyze(&s, SR, 1));
    let want: Vec<(f64, Option<f32>)> = std::iter::once((0.0, Some(110.0)))
        .chain((1..4).map(|i| (0.12 * i as f64, Some(notes[i]))))
        .collect();
    assert!(close(&got, &want), "{got:?}");
}

#[test]
fn vibrato_sweeps_and_steady_noise_are_one_onset() {
    // Vibrato: half a semitone at 6 Hz.
    let vib: Vec<f32> = {
        let mut phase = 0.0f32;
        (0..SR as usize)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let hz = 440.0 * 2f32.powf(0.5 * (TAU * 6.0 * t).sin() / 12.0);
                phase += hz / SR as f32;
                0.5 * (TAU * phase).sin()
            })
            .collect()
    };
    // A slow sweep, an octave per second.
    let sweep: Vec<f32> = {
        let mut phase = 0.0f32;
        (0..SR as usize)
            .map(|i| {
                phase += 300.0 * 2f32.powf(i as f32 / SR as f32) / SR as f32;
                0.5 * (TAU * phase).sin()
            })
            .collect()
    };
    for (name, s) in [
        ("vibrato", vib),
        ("sweep", sweep),
        ("noise", noise(1.0, 0.5)),
    ] {
        let a = analyze(&s, SR, 1);
        assert_eq!(a.onsets.len(), 1, "{name}: {:?}", onset_notes(&a));
        assert!(a.onsets[0].time_seconds < 0.001);
        assert!(!a.summary.contains("onsets"), "{name}: {}", a.summary);
    }
    // Noise has no pitch; silence has no onsets.
    assert_eq!(analyze(&noise(0.3, 0.5), SR, 1).onsets[0].note_hz, None);
    assert!(analyze(&vec![0.0; 4410], SR, 1).onsets.is_empty());
}

#[test]
fn a_noise_burst_over_a_tone_is_an_onset() {
    let mut s = sine(220.0, 0.6, 0.2);
    let burst = noise(0.1, 0.8);
    let at = (0.3 * SR as f32) as usize;
    for (x, n) in s[at..].iter_mut().zip(&burst) {
        *x += n;
    }
    let got = onset_notes(&analyze(&s, SR, 1));
    assert_eq!(got.len(), 2, "{got:?}");
    assert!((got[1].0 - 0.3).abs() < 0.012, "{got:?}");
    assert_eq!(got[1].1, None, "a burst has no pitch: {got:?}");
}

#[test]
fn analysis_adds_energy_and_onsets() {
    let v = serde_json::to_value(analyze(&sine(440.0, 0.5, 0.5), SR, 1)).unwrap();
    assert_eq!(
        v["energy_over_time"]["rms_dbfs"].as_array().unwrap().len(),
        20
    );
    assert_eq!(v["energy_over_time"]["rms_dbfs"][3], -9.0);
    assert!(v["onsets"][0]["time_seconds"].as_f64().unwrap() < 0.001);
}
