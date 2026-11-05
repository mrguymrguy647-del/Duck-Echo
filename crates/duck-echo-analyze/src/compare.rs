//! Compare two sounds: the analysis of each, what changed from `a` to `b`,
//! and how similar the waveforms are.
//!
//! An agent uses this to check that an edit moved the sound the intended
//! way ("louder by 3 LU, brighter, same length") without listening.

use crate::fft::{fft, ifft};
use crate::{analyze, Analysis};
use serde::{Deserialize, Serialize};

/// Longest combined length (in frames) for which the cross-correlation is
/// computed; beyond this `correlation` is `null`. About 95 s at 44.1 kHz.
pub const MAX_CORRELATION_FRAMES: usize = 1 << 22;

/// Interleaved samples with their format.
#[derive(Debug, Clone, Copy)]
pub struct Signal<'a> {
    /// Interleaved samples in -1..=1.
    pub samples: &'a [f32],
    /// Samples per second.
    pub sample_rate: u32,
    /// Interleaved channels.
    pub channels: u16,
}

/// Result of [`compare`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// One line in words: what changed from `a` to `b`.
    pub summary: String,
    /// True when both have the same format and exactly the same samples.
    pub identical: bool,
    /// `b` minus `a` for each measurement.
    pub delta: Delta,
    /// Waveform similarity of the channel means. `null` when the sample
    /// rates differ, either sound is silent, or they are too long.
    pub correlation: Option<Correlation>,
    /// Analysis of the first sound.
    pub a: Analysis,
    /// Analysis of the second sound.
    pub b: Analysis,
}

/// Differences, always `b` minus `a`. A field is `null` when either side
/// lacks the measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    /// Length difference in seconds.
    pub duration_seconds: f64,
    /// Peak difference in dB.
    pub peak_db: Option<f32>,
    /// RMS difference in dB.
    pub rms_db: Option<f32>,
    /// Loudness difference in LU. `null` unless both were measured with the
    /// same `loudness_method`, since gated and ungated values do not mix.
    pub loudness_lu: Option<f32>,
    /// Spectral centroid difference in Hz.
    pub spectral_centroid_hz: Option<f32>,
    /// Spectral centroid ratio `b / a`; above 1 is brighter.
    pub spectral_centroid_ratio: Option<f32>,
    /// Tonality difference.
    pub tonality: Option<f32>,
    /// Attack time difference in seconds.
    pub attack_seconds: Option<f64>,
    /// Decay time difference in seconds.
    pub decay_seconds: Option<f64>,
    /// Crest factor difference in dB.
    pub crest_factor_db: Option<f32>,
    /// Trailing silence difference in seconds.
    pub trailing_silence_seconds: f64,
}

/// Normalized cross-correlation of the two waveforms (channel means).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Correlation {
    /// The correlation with the largest magnitude over all time shifts,
    /// -1..=1. 1 is the same waveform (up to level), -1 the same inverted,
    /// near 0 unrelated.
    pub peak: f32,
    /// Shift of `b` relative to `a` at that peak, seconds; positive means
    /// `b` is later.
    pub lag_seconds: f64,
    /// Correlation with no shift.
    pub at_zero_lag: f32,
}

fn sub<T: std::ops::Sub<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
    Some(b? - a?)
}

/// Compare two sounds.
pub fn compare(a: Signal<'_>, b: Signal<'_>) -> Comparison {
    let aa = analyze(a.samples, a.sample_rate, a.channels);
    let ab = analyze(b.samples, b.sample_rate, b.channels);
    let identical =
        a.sample_rate == b.sample_rate && a.channels == b.channels && a.samples == b.samples;
    let delta = Delta {
        duration_seconds: ab.duration_seconds - aa.duration_seconds,
        peak_db: sub(aa.peak_dbfs, ab.peak_dbfs),
        rms_db: sub(aa.rms_dbfs, ab.rms_dbfs),
        loudness_lu: if aa.loudness_method == ab.loudness_method {
            sub(aa.loudness_lufs, ab.loudness_lufs)
        } else {
            None
        },
        spectral_centroid_hz: sub(aa.spectral_centroid_hz, ab.spectral_centroid_hz),
        spectral_centroid_ratio: aa
            .spectral_centroid_hz
            .zip(ab.spectral_centroid_hz)
            .and_then(|(x, y)| (x > 0.0).then(|| y / x)),
        tonality: sub(aa.tonality, ab.tonality),
        attack_seconds: sub(aa.attack_seconds, ab.attack_seconds),
        decay_seconds: sub(aa.decay_seconds, ab.decay_seconds),
        crest_factor_db: sub(aa.crest_factor_db, ab.crest_factor_db),
        trailing_silence_seconds: ab.trailing_silence_seconds - aa.trailing_silence_seconds,
    };
    let correlation = if a.sample_rate == b.sample_rate {
        correlate(&mono(a), &mono(b), a.sample_rate)
    } else {
        None
    };
    let summary = summarize(identical, &delta, correlation.as_ref(), &aa, &ab);
    Comparison {
        summary,
        identical,
        delta,
        correlation,
        a: aa,
        b: ab,
    }
}

fn mono(s: Signal<'_>) -> Vec<f32> {
    let ch = s.channels.max(1) as usize;
    if ch == 1 {
        return s.samples.to_vec();
    }
    s.samples
        .chunks_exact(ch)
        .map(|f| f.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Normalized cross-correlation over all shifts, via FFT.
fn correlate(a: &[f32], b: &[f32], sample_rate: u32) -> Option<Correlation> {
    let ea: f64 = a.iter().map(|&x| (x as f64) * (x as f64)).sum();
    let eb: f64 = b.iter().map(|&x| (x as f64) * (x as f64)).sum();
    if ea <= 0.0 || eb <= 0.0 || a.len() + b.len() > MAX_CORRELATION_FRAMES {
        return None;
    }
    let n = (a.len() + b.len()).next_power_of_two();
    let load = |x: &[f32]| {
        let mut re = vec![0.0f64; n];
        for (r, &v) in re.iter_mut().zip(x) {
            *r = v as f64;
        }
        let mut im = vec![0.0f64; n];
        fft(&mut re, &mut im);
        (re, im)
    };
    let (ar, ai) = load(a);
    let (br, bi) = load(b);
    // conj(A) * B gives r[m] = sum a[i] * b[i + m].
    let mut rr: Vec<f64> = (0..n).map(|k| ar[k] * br[k] + ai[k] * bi[k]).collect();
    let mut ri: Vec<f64> = (0..n).map(|k| ar[k] * bi[k] - ai[k] * br[k]).collect();
    ifft(&mut rr, &mut ri);
    let norm = (ea * eb).sqrt();
    let lag_of = |i: usize| -> i64 {
        if i < n / 2 {
            i as i64
        } else {
            i as i64 - n as i64
        }
    };
    let best = (0..n).max_by(|&x, &y| rr[x].abs().total_cmp(&rr[y].abs()))?;
    Some(Correlation {
        peak: (rr[best] / norm).clamp(-1.0, 1.0) as f32,
        lag_seconds: lag_of(best) as f64 / sample_rate as f64,
        at_zero_lag: (rr[0] / norm).clamp(-1.0, 1.0) as f32,
    })
}

fn summarize(
    identical: bool,
    d: &Delta,
    corr: Option<&Correlation>,
    a: &Analysis,
    b: &Analysis,
) -> String {
    if identical {
        return "identical audio".into();
    }
    let mut parts = Vec::new();
    match d.loudness_lu {
        Some(l) if l.abs() >= 0.5 => parts.push(format!(
            "b is {:.1} LU {}",
            l.abs(),
            if l > 0.0 { "louder" } else { "quieter" }
        )),
        Some(_) => parts.push("same loudness".into()),
        None => {
            if let Some(p) = d.rms_db.filter(|p| p.abs() >= 0.5) {
                parts.push(format!(
                    "b is {:.1} dB {} (RMS; loudness methods differ)",
                    p.abs(),
                    if p > 0.0 { "louder" } else { "quieter" }
                ));
            }
        }
    }
    if let Some(r) = d.spectral_centroid_ratio {
        if (r - 1.0).abs() >= 0.1 {
            let pct = ((r - 1.0) * 100.0).abs();
            parts.push(format!(
                "{pct:.0}% {}",
                if r > 1.0 { "brighter" } else { "darker" }
            ));
        }
    }
    if d.duration_seconds.abs() >= 0.01 {
        parts.push(format!(
            "{:.2} s {}",
            d.duration_seconds.abs(),
            if d.duration_seconds > 0.0 {
                "longer"
            } else {
                "shorter"
            }
        ));
    }
    if let Some(t) = d.attack_seconds.filter(|t| t.abs() >= 0.005) {
        parts.push(format!(
            "attack {:.0} ms {}",
            t.abs() * 1000.0,
            if t > 0.0 { "slower" } else { "faster" }
        ));
    }
    if b.clipping.clipped && !a.clipping.clipped {
        parts.push("b CLIPS".into());
    }
    match corr {
        Some(c) if c.peak.abs() >= 0.9 && c.lag_seconds.abs() < 1e-9 => {
            parts.push(format!("waveforms closely match ({:.2})", c.peak))
        }
        Some(c) if c.peak.abs() >= 0.9 => parts.push(format!(
            "waveforms closely match ({:.2}) with b shifted {:+.3} s",
            c.peak, c.lag_seconds
        )),
        Some(c) if c.peak.abs() < 0.3 => parts.push(format!("waveforms differ ({:.2})", c.peak)),
        Some(c) => parts.push(format!("waveforms partly similar ({:.2})", c.peak)),
        None => {}
    }
    if parts.is_empty() {
        "nearly the same".into()
    } else {
        parts.join(", ")
    }
}
