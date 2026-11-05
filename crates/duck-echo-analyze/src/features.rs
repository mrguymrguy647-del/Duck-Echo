//! Spectral and envelope features, and the one-line summary.
//!
//! These are measurements for agents that cannot listen. Each has a plain
//! definition so an agent can predict how an edit moves it:
//!
//! * **Spectral centroid**: magnitude-weighted mean frequency of the sound,
//!   over the non-silent part. Higher means brighter.
//! * **Tonality** (0..1): the share of power in spectral peaks that stand at
//!   least 10 dB above their neighbourhood (the median of ±16 bins). Tones
//!   and their harmonics read near 1, broadband noise near 0. Noise
//!   squeezed into a band narrower than about 700 Hz also reads as tonal.
//! * **Pitch track**: up to 8 evenly spaced windows over the non-silent
//!   part, each with the dominant (strongest) frequency, its level and
//!   tonality. The dominant frequency is usually the fundamental of chip
//!   waveforms; for noisy windows (low tonality) it means little.
//! * **Attack**: seconds from the first non-silent sample until the
//!   short-term peak level first reaches 90% of the sound's peak. The
//!   short-term level is the largest sample magnitude in the 20 ms before
//!   each moment (1 ms steps), which stays steady for content down to
//!   50 Hz, so bass does not make it ripple.
//! * **Decay**: seconds from the last sample at 90% of the peak until the
//!   moment after which no sample in the next 20 ms reaches 10% of the
//!   peak (-20 dB). A held note is not counted as decay. `null` if it never
//!   falls that far.

use crate::fft::power_spectrum;
use crate::{Analysis, LoudnessMethod, SILENCE_THRESHOLD_LINEAR};
use serde::{Deserialize, Serialize};

/// Maximum number of windows in a pitch track.
pub const PITCH_TRACK_MAX_POINTS: usize = 8;

/// One window of the pitch track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PitchPoint {
    /// Centre of the window, seconds from the start of the file.
    pub time_seconds: f64,
    /// Strongest frequency in the window, Hz. `null` when the window's RMS
    /// is below the -60 dBFS silence threshold.
    pub dominant_hz: Option<f32>,
    /// RMS level of the window in dBFS. `null` only for digital silence.
    pub level_dbfs: Option<f32>,
    /// Tonality of the window, 0..1. `null` when below the silence
    /// threshold.
    pub tonality: Option<f32>,
}

/// Spectral measurements of a mono signal.
#[derive(Debug, Clone, PartialEq)]
pub struct Spectral {
    /// Spectral centroid in Hz.
    pub centroid_hz: Option<f32>,
    /// Overall tonality, 0..1.
    pub tonality: Option<f32>,
    /// Pitch track.
    pub track: Vec<PitchPoint>,
}

/// FFT size for a sample rate: about 46 ms, a power of two.
pub fn fft_size(sample_rate: u32) -> usize {
    ((sample_rate as f64 * 0.046).round() as usize)
        .next_power_of_two()
        .max(256)
}

fn rms(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    (xs.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / xs.len() as f64).sqrt() as f32
}

/// Lowest bin considered (skips DC and sub-audio rumble).
fn min_bin(sample_rate: u32, n: usize) -> usize {
    ((20.0 * n as f64 / sample_rate as f64).ceil() as usize).max(1)
}

/// Power in prominent peaks and total power, over bins from `lo`.
fn peak_and_total(p: &[f64], lo: usize) -> (f64, f64) {
    const RADIUS: usize = 16;
    let hi = p.len().saturating_sub(1);
    let mut peak = 0.0;
    let mut total = 0.0;
    let mut neighbours = Vec::with_capacity(2 * RADIUS);
    for k in lo..hi {
        total += p[k];
        neighbours.clear();
        let a = k.saturating_sub(RADIUS).max(lo);
        let b = (k + RADIUS).min(hi - 1);
        neighbours.extend((a..=b).filter(|&j| j != k).map(|j| p[j]));
        if neighbours.is_empty() {
            continue;
        }
        let mid = neighbours.len() / 2;
        let median = *neighbours
            .select_nth_unstable_by(mid, |x, y| x.total_cmp(y))
            .1;
        if p[k] >= 10.0 * median {
            peak += p[k];
        }
    }
    (peak, total)
}

/// Strongest bin from `lo`, refined by parabolic interpolation of the log
/// power, in Hz.
fn dominant_hz(p: &[f64], lo: usize, sample_rate: u32, n: usize) -> Option<f32> {
    let hi = p.len().saturating_sub(1);
    let k = (lo..hi).max_by(|&a, &b| p[a].total_cmp(&p[b]))?;
    if p[k] <= 0.0 {
        return None;
    }
    let mut delta = 0.0;
    if k > lo && k + 1 < hi {
        let (a, b, c) = (
            p[k - 1].max(1e-300).ln(),
            p[k].ln(),
            p[k + 1].max(1e-300).ln(),
        );
        let denom = a - 2.0 * b + c;
        if denom.abs() > 1e-12 {
            delta = (0.5 * (a - c) / denom).clamp(-0.5, 0.5);
        }
    }
    Some(((k as f64 + delta) * sample_rate as f64 / n as f64) as f32)
}

/// Spectral centroid, tonality and pitch track of `mono` over the active
/// span `first..=last` (frame indices of the first and last non-silent
/// samples).
pub fn spectral(mono: &[f32], sample_rate: u32, active: Option<(usize, usize)>) -> Spectral {
    let Some((first, last)) = active else {
        return Spectral {
            centroid_hz: None,
            tonality: None,
            track: Vec::new(),
        };
    };
    let n = fft_size(sample_rate);
    let lo = min_bin(sample_rate, n);
    let bin_hz = sample_rate as f64 / n as f64;

    // Whole-sound centroid and tonality from half-overlapping frames.
    let mut mag_sum = vec![0.0f64; n / 2 + 1];
    let (mut peak_power, mut total_power) = (0.0, 0.0);
    let mut start = first;
    loop {
        let end = (start + n).min(last + 1);
        let frame = &mono[start..end];
        if rms(frame) > SILENCE_THRESHOLD_LINEAR {
            let p = power_spectrum(frame, n);
            for (m, &pk) in mag_sum.iter_mut().zip(&p) {
                *m += pk.sqrt();
            }
            let (pp, tp) = peak_and_total(&p, lo);
            peak_power += pp;
            total_power += tp;
        }
        if end > last {
            break;
        }
        start += n / 2;
    }
    let hi = mag_sum.len() - 1;
    let weight: f64 = mag_sum[lo..hi].iter().sum();
    let centroid_hz = (weight > 0.0).then(|| {
        let num: f64 = (lo..hi).map(|k| k as f64 * bin_hz * mag_sum[k]).sum();
        (num / weight) as f32
    });
    let tonality = (total_power > 0.0).then(|| (peak_power / total_power) as f32);

    // Pitch track: evenly spaced windows kept inside the active span, so
    // none is mostly padding.
    let span_seconds = (last - first + 1) as f64 / sample_rate as f64;
    let points = ((span_seconds / 0.05).ceil() as usize).clamp(1, PITCH_TRACK_MAX_POINTS);
    let (lo_c, hi_c) = if last - first + 1 > n {
        (first + n / 2, last + 1 - n / 2)
    } else {
        let mid = (first + last) / 2;
        (mid, mid)
    };
    let track = (0..points)
        .map(|i| {
            let centre = if points == 1 || lo_c == hi_c {
                (lo_c + hi_c) / 2
            } else {
                lo_c + (hi_c - lo_c) * i / (points - 1)
            };
            let a = centre.saturating_sub(n / 2);
            let b = (a + n).min(mono.len());
            let frame = &mono[a..b];
            let level = rms(frame);
            let audible = level > SILENCE_THRESHOLD_LINEAR;
            let p = audible.then(|| power_spectrum(frame, n));
            PitchPoint {
                time_seconds: centre as f64 / sample_rate as f64,
                dominant_hz: p.as_ref().and_then(|p| dominant_hz(p, lo, sample_rate, n)),
                level_dbfs: (level > 0.0).then(|| 20.0 * level.log10()),
                tonality: p.as_ref().and_then(|p| {
                    let (pp, tp) = peak_and_total(p, lo);
                    (tp > 0.0).then(|| (pp / tp) as f32)
                }),
            }
        })
        .collect();

    Spectral {
        centroid_hz,
        tonality,
        track,
    }
}

/// Attack and decay times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeTimes {
    /// Seconds from onset to 90% of the maximum short-term level.
    pub attack_seconds: Option<f64>,
    /// Seconds from the maximum to 10% of it.
    pub decay_seconds: Option<f64>,
}

/// Attack and decay from the peak envelope of interleaved `samples`.
/// `onset` is the frame index of the first non-silent sample.
pub fn envelope_times(
    samples: &[f32],
    channels: usize,
    sample_rate: u32,
    onset: Option<usize>,
) -> EnvelopeTimes {
    let none = EnvelopeTimes {
        attack_seconds: None,
        decay_seconds: None,
    };
    let Some(onset) = onset else { return none };
    let frames = samples.len() / channels;
    let sr = sample_rate as f64;
    // Per-frame magnitude: the largest over channels.
    let mag: Vec<f32> = samples[..frames * channels]
        .chunks_exact(channels)
        .map(|f| f.iter().fold(0.0f32, |m, x| m.max(x.abs())))
        .collect();
    let peak = mag.iter().fold(0.0f32, |m, &x| m.max(x));
    if peak <= 0.0 {
        return none;
    }
    let win = ((0.020 * sr).round() as usize).max(1);
    let hop = ((0.001 * sr).round() as usize).max(1);
    let window_max = |a: usize, b: usize| mag[a..b].iter().fold(0.0f32, |m, &x| m.max(x));

    // Attack: first step whose trailing 20 ms reaches 90% of the peak.
    let mut attack = None;
    let mut t = onset;
    while t < frames {
        if window_max(t.saturating_sub(win), t + 1) >= 0.9 * peak {
            attack = Some((t - onset) as f64 / sr);
            break;
        }
        t += hop;
    }

    // Decay: from the last sample at 90% of the peak (but at or after the
    // first one) to the first step whose leading 20 ms stays under 10%.
    let first90 = mag.iter().position(|&x| x >= 0.9 * peak);
    let decay = first90.and_then(|f90| {
        let mut t = f90;
        let mut last90 = f90;
        while t < frames {
            let end = (t + win).min(frames);
            let m = window_max(t, end);
            if m < 0.1 * peak {
                if let Some(i) = mag[last90..t].iter().rposition(|&x| x >= 0.9 * peak) {
                    last90 += i;
                }
                return Some((t - last90) as f64 / sr);
            }
            t += hop;
        }
        None
    });
    EnvelopeTimes {
        attack_seconds: attack,
        decay_seconds: decay,
    }
}

/// Words used by the summary, with the thresholds that choose them.
/// Documented here so agents can map words back to numbers.
pub mod words {
    /// Spectral centroid upper bounds (Hz) for each brightness word; above
    /// the last bound the sound is "very bright".
    pub const BRIGHTNESS: [(f32, &str); 4] = [
        (300.0, "very dark"),
        (800.0, "dark"),
        (2500.0, "mid"),
        (6000.0, "bright"),
    ];
    /// Attack upper bounds (seconds) for each attack word; above the last
    /// bound the attack is "slow attack".
    pub const ATTACK: [(f64, &str); 3] = [
        (0.010, "sharp attack"),
        (0.050, "quick attack"),
        (0.200, "soft attack"),
    ];
    /// Tonality at or above this reads "tonal".
    pub const TONAL: f32 = 0.7;
    /// Tonality at or below this reads "noisy"; in between, "tone and noise".
    pub const NOISY: f32 = 0.3;
    /// A dominant frequency change beyond this many semitones between the
    /// first and last tonal windows reads as rising or falling pitch.
    pub const PITCH_MOVE_SEMITONES: f32 = 1.0;
    /// Trailing silence above this share of the length is called out.
    pub const LONG_TAIL_SHARE: f64 = 0.2;
}

/// A short, human-readable description of an analysis, built only from its
/// numbers with the thresholds in [`words`].
pub fn summary(a: &Analysis) -> String {
    use words::*;
    if a.is_silent {
        return format!("{:.2} s, silent", a.duration_seconds);
    }
    let mut parts = vec![format!("{:.2} s", a.duration_seconds)];
    if let Some(l) = a.loudness_lufs {
        let approx = if a.loudness_method == Some(LoudnessMethod::KWeightedUngated) {
            "~"
        } else {
            ""
        };
        parts.push(format!("{approx}{l:.1} LUFS"));
    }
    if let Some(c) = a.spectral_centroid_hz {
        let word = BRIGHTNESS
            .iter()
            .find(|(max, _)| c < *max)
            .map(|(_, w)| *w)
            .unwrap_or("very bright");
        parts.push(word.to_string());
    }
    if let Some(t) = a.attack_seconds {
        let word = ATTACK
            .iter()
            .find(|(max, _)| t < *max)
            .map(|(_, w)| *w)
            .unwrap_or("slow attack");
        parts.push(word.to_string());
    }
    if let Some(t) = a.tonality {
        let character = if t >= TONAL {
            "tonal"
        } else if t <= NOISY {
            "noisy"
        } else {
            "tone and noise"
        };
        // Pitch is only meaningful in windows that are not mostly noise.
        let pitched: Vec<f32> = a
            .pitch_track
            .iter()
            .filter(|p| p.tonality.is_some_and(|t| t > NOISY))
            .filter_map(|p| p.dominant_hz)
            .collect();
        let pitch = match (pitched.first(), pitched.last()) {
            (Some(&f0), Some(&f1)) if pitched.len() >= 2 => {
                let semis = 12.0 * (f1 / f0).log2();
                if semis > PITCH_MOVE_SEMITONES {
                    format!("pitch rising from {f0:.0} to {f1:.0} Hz")
                } else if semis < -PITCH_MOVE_SEMITONES {
                    format!("pitch falling from {f0:.0} to {f1:.0} Hz")
                } else {
                    format!("around {f0:.0} Hz")
                }
            }
            (Some(&f0), _) => format!("around {f0:.0} Hz"),
            _ => String::new(),
        };
        if pitch.is_empty() || t <= NOISY {
            parts.push(character.to_string());
        } else {
            parts.push(format!("{character}, {pitch}"));
        }
    }
    if a.clipping.clipped {
        parts.push(format!("CLIPPING ({} samples)", a.clipping.clipped_samples));
    } else {
        parts.push("no clipping".into());
    }
    if a.duration_seconds > 0.0 && a.trailing_silence_seconds / a.duration_seconds > LONG_TAIL_SHARE
    {
        parts.push(format!("{:.2} s silent tail", a.trailing_silence_seconds));
    }
    parts.join(", ")
}
