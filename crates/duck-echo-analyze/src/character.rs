//! Words for how a sound feels, measured cheaply: roughness, sharpness,
//! boom and movement, and the strongest spectral peaks.
//!
//! Duck Ear (Phase 4) will listen properly. Until then these four numbers
//! answer the questions a horror film's sounds kept raising: is the
//! scream rough enough, is the creak piercing, does the slam have any
//! boom, does the wind move or sit still? Each is a plain measurement with
//! fixed thresholds for `low`, `medium` and `high` (see [`Level`]), listed
//! in `docs/ANALYSIS.md`.
//!
//! * **Roughness**: how much the loudness wobbles fast, 20 to 150 times a
//!   second. Growls, screams, rasps and buzzes wobble there; a clean tone
//!   or a smooth swell does not. The envelope (the signal made positive,
//!   smoothed to about 200 Hz) minus its slow shape (a 30 ms average),
//!   as a share of the envelope's mean.
//! * **Sharpness**: the share of the sound's power between 2 and 5 kHz,
//!   where the ear is most sensitive: harsh, piercing sounds have much
//!   there.
//! * **Boom**: the share of power below 80 Hz.
//! * **Movement**: how much the sound changes over time: how far its
//!   brightness (spectral centroid, in octaves) and its level (in dB, over
//!   20) spread across 50 ms frames of its loud part.
//! * **Peaks**: the strongest peaks of the sound's average spectrum, so
//!   side tones (a note plus and minus a fast wobble's rate) can be read.

use serde::{Deserialize, Serialize};

use crate::fft::power_spectrum;

/// How much of something a sound has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Little or none.
    Low,
    /// Some.
    Medium,
    /// A lot: the summary says so.
    High,
}

impl Level {
    /// The level of `value` with these two thresholds: below `medium` is
    /// low, from `high` on is high.
    pub fn of(value: f32, medium: f32, high: f32) -> Level {
        if value >= high {
            Level::High
        } else if value >= medium {
            Level::Medium
        } else {
            Level::Low
        }
    }

    /// The word: `low`, `medium` or `high`.
    pub fn word(self) -> &'static str {
        match self {
            Level::Low => "low",
            Level::Medium => "medium",
            Level::High => "high",
        }
    }
}

/// Where `low` ends and `high` begins, for each measure. Chosen from the
/// presets and a few test sounds (see `docs/ANALYSIS.md`).
pub mod thresholds {
    /// Roughness: medium from 0.12, high from 0.3.
    pub const ROUGHNESS: (f32, f32) = (0.12, 0.3);
    /// Sharpness: medium from 0.15, high from 0.35 of the power.
    pub const SHARPNESS: (f32, f32) = (0.15, 0.35);
    /// Boom: medium from 0.1, high from 0.35 of the power.
    pub const BOOM: (f32, f32) = (0.1, 0.35);
    /// Movement: medium from 0.3, high from 0.75.
    pub const MOVEMENT: (f32, f32) = (0.3, 0.75);
}

/// The four measures as words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feel {
    /// Fast loudness wobble: growls, screams.
    pub roughness: Level,
    /// Energy around 2 to 5 kHz: harsh, piercing.
    pub sharpness: Level,
    /// Energy below 80 Hz.
    pub boom: Level,
    /// How much the sound changes over time.
    pub movement: Level,
}

/// One peak of a sound's average spectrum.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpectralPeak {
    /// Its frequency, Hz.
    pub hz: f32,
    /// Its level against the strongest peak, dB (0 is the strongest).
    pub db: f32,
}

/// The measures of one sound.
#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    /// See the module text.
    pub roughness: f32,
    /// See the module text.
    pub sharpness: f32,
    /// See the module text.
    pub boom: f32,
    /// See the module text.
    pub movement: f32,
    /// The four as words.
    pub feel: Feel,
    /// Up to 8 strongest spectral peaks, strongest first.
    pub peaks: Vec<SpectralPeak>,
}

/// Measure `mono` (the mean of the channels), whose sound is between
/// frames `first` and `last`.
pub fn measure(mono: &[f32], sample_rate: u32, first: usize, last: usize) -> Character {
    let part = &mono[first..=last.min(mono.len().saturating_sub(1))];
    let sr = sample_rate as f64;
    let (spectrum, n) = average_spectrum(part);
    let bin_hz = sr / n as f64;
    let total: f64 = spectrum.iter().skip(1).sum::<f64>().max(1e-30);
    let band = |lo: f64, hi: f64| -> f64 {
        spectrum
            .iter()
            .enumerate()
            .filter(|(k, _)| {
                let f = *k as f64 * bin_hz;
                f >= lo && f < hi
            })
            .map(|(_, p)| p)
            .sum::<f64>()
    };
    let sharpness = (band(2000.0, 5000.0) / total) as f32;
    let boom = (band(bin_hz * 0.5, 80.0) / total) as f32;
    let roughness = roughness(part, sample_rate);
    let movement = movement(part, sample_rate);
    let feel = Feel {
        roughness: Level::of(roughness, thresholds::ROUGHNESS.0, thresholds::ROUGHNESS.1),
        sharpness: Level::of(sharpness, thresholds::SHARPNESS.0, thresholds::SHARPNESS.1),
        boom: Level::of(boom, thresholds::BOOM.0, thresholds::BOOM.1),
        movement: Level::of(movement, thresholds::MOVEMENT.0, thresholds::MOVEMENT.1),
    };
    Character {
        roughness: round3(roughness),
        sharpness: round3(sharpness),
        boom: round3(boom),
        movement: round3(movement),
        feel,
        peaks: peaks(&spectrum, bin_hz),
    }
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// The mean power spectrum of `part`, in Hann frames of about 93 ms (a
/// fine enough grid, 10.8 Hz at 44.1 kHz, to tell side tones apart) with
/// half overlap. Returns the spectrum and the FFT size.
fn average_spectrum(part: &[f32]) -> (Vec<f64>, usize) {
    let n = 4096usize;
    let mut sum = vec![0.0f64; n / 2 + 1];
    let mut frames = 0;
    let hop = n / 2;
    let mut start = 0;
    loop {
        let end = (start + n).min(part.len());
        if end <= start {
            break;
        }
        let p = power_spectrum(&part[start..end], n);
        for (s, v) in sum.iter_mut().zip(p) {
            *s += v;
        }
        frames += 1;
        if end == part.len() {
            break;
        }
        start += hop;
    }
    if frames > 0 {
        for s in &mut sum {
            *s /= frames as f64;
        }
    }
    (sum, n)
}

/// The strongest local peaks of `spectrum`, at most 8, within 40 dB of the
/// strongest, each placed between bins by a parabola through its
/// neighbours' levels.
fn peaks(spectrum: &[f64], bin_hz: f64) -> Vec<SpectralPeak> {
    let db: Vec<f64> = spectrum
        .iter()
        .map(|p| 10.0 * p.max(1e-30).log10())
        .collect();
    let mut found: Vec<(f64, f64)> = Vec::new();
    for k in 3..db.len().saturating_sub(3) {
        let here = db[k];
        let is_peak = (k - 2..=k + 2).all(|j| j == k || db[j] < here);
        if is_peak {
            let (a, b, c) = (db[k - 1], here, db[k + 1]);
            let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
            let shift = if shift.is_finite() {
                shift.clamp(-0.5, 0.5)
            } else {
                0.0
            };
            let level = b - 0.25 * (a - c) * shift;
            found.push(((k as f64 + shift) * bin_hz, level));
        }
    }
    found.sort_by(|x, y| y.1.total_cmp(&x.1));
    let top = found.first().map_or(0.0, |p| p.1);
    found
        .into_iter()
        .filter(|p| p.1 >= top - 40.0)
        .take(8)
        .map(|(hz, level)| SpectralPeak {
            hz: ((hz * 10.0).round() / 10.0) as f32,
            db: (((level - top) * 10.0).round() / 10.0) as f32,
        })
        .collect()
}

/// Fast loudness wobble, 20 to 150 times a second, as a share of the
/// envelope's mean.
fn roughness(part: &[f32], sample_rate: u32) -> f32 {
    // The envelope: the size of the analytic signal (the Hilbert
    // envelope), which follows the loudness without the ripple at twice
    // the note that a rectified signal has (a low hum would otherwise
    // read as rough), averaged to one value a millisecond.
    let magnitude = hilbert_envelope(part);
    let step = (f64::from(sample_rate) / 1000.0).round().max(1.0) as usize;
    let env: Vec<f64> = magnitude
        .chunks(step)
        .map(|c| c.iter().sum::<f64>() / c.len() as f64)
        .collect();
    if env.len() < 40 {
        return 0.0;
    }
    // Its slow shape: a 31 ms moving average.
    let half = 15usize;
    let mut slow = vec![0.0; env.len()];
    for (i, s) in slow.iter_mut().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(env.len());
        *s = env[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
    }
    // Only the loud part counts: within 20 dB of the envelope's top.
    let top = env.iter().copied().fold(0.0, f64::max);
    if top <= 0.0 {
        return 0.0;
    }
    // The fast part of the loud stretch (the rest counts as still).
    let mut fast = vec![0.0f64; env.len()];
    let (mut mean, mut count) = (0.0, 0usize);
    for i in half..env.len() - half {
        if slow[i] >= top * 0.1 {
            fast[i] = env[i] - slow[i];
            mean += env[i];
            count += 1;
        }
    }
    if count == 0 || mean <= 0.0 {
        return 0.0;
    }
    let mean = mean / count as f64;
    // Its wobble rates (one value a millisecond, so 1000 per second). A
    // growl wobbles at one steady rate; noise flickers at all rates. So
    // only the strongest rate between 20 and 150 per second counts, with
    // 3 per second either side of it.
    let n = fast.len().next_power_of_two().max(1024);
    let mut re = fast;
    re.resize(n, 0.0);
    let mut im = vec![0.0f64; n];
    crate::fft::fft(&mut re, &mut im);
    let per_bin = 1000.0 / n as f64;
    let power = |k: usize| re[k] * re[k] + im[k] * im[k];
    let lo = (20.0 / per_bin).ceil() as usize;
    let hi = ((150.0 / per_bin).floor() as usize).min(n / 2 - 1);
    let Some(peak) = (lo..=hi).max_by(|a, b| power(*a).total_cmp(&power(*b))) else {
        return 0.0;
    };
    let width = ((3.0 / per_bin).round() as usize).max(1);
    let band: f64 = (peak.saturating_sub(width)..=(peak + width).min(n / 2 - 1))
        .map(power)
        .sum();
    // Parseval: the mean square of the wobble at that rate.
    let mean_square = 2.0 * band / (n as f64 * count as f64);
    (mean_square.sqrt() / mean) as f32
}

/// The size of the analytic signal of `x`, sample by sample: frames of
/// 16384 samples, each turned into its analytic signal by the FFT (the
/// negative frequencies removed), keeping the middle half of each frame,
/// where the frame's edges do not reach.
fn hilbert_envelope(x: &[f32]) -> Vec<f64> {
    const N: usize = 16384;
    let mut out = vec![0.0f64; x.len()];
    let quarter = N / 4;
    let mut start = 0usize;
    loop {
        let mut re = vec![0.0f64; N];
        let mut im = vec![0.0f64; N];
        let len = (x.len() - start).min(N);
        for (r, v) in re.iter_mut().zip(&x[start..start + len]) {
            *r = f64::from(*v);
        }
        crate::fft::fft(&mut re, &mut im);
        for k in 1..N / 2 {
            re[k] *= 2.0;
            im[k] *= 2.0;
        }
        for k in N / 2 + 1..N {
            re[k] = 0.0;
            im[k] = 0.0;
        }
        crate::fft::ifft(&mut re, &mut im);
        // Keep the middle half (all of it at the very start and end).
        let keep_from = if start == 0 { 0 } else { quarter };
        let last = start + len >= x.len();
        let keep_to = if last { len } else { N - quarter };
        for i in keep_from..keep_to.min(len) {
            out[start + i] = (re[i] * re[i] + im[i] * im[i]).sqrt();
        }
        if last {
            break;
        }
        start += N / 2;
    }
    out
}

/// How much brightness and level change across 50 ms frames of the loud
/// part.
fn movement(part: &[f32], sample_rate: u32) -> f32 {
    let size = (sample_rate as f64 * 0.05) as usize;
    let n = size.next_power_of_two();
    if part.len() < size * 2 {
        return 0.0;
    }
    let bin_hz = sample_rate as f64 / n as f64;
    let mut frames: Vec<(f64, f64)> = Vec::new();
    let mut start = 0;
    while start + size <= part.len() {
        let frame = &part[start..start + size];
        let power = frame
            .iter()
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum::<f64>()
            / size as f64;
        if power > 0.0 {
            let p = power_spectrum(frame, n);
            let total: f64 = p.iter().skip(1).sum();
            if total > 0.0 {
                let centroid = p
                    .iter()
                    .enumerate()
                    .skip(1)
                    .map(|(k, v)| k as f64 * bin_hz * v)
                    .sum::<f64>()
                    / total;
                frames.push((10.0 * power.log10(), centroid.max(20.0).log2()));
            }
        }
        start += size / 2;
    }
    let loudest = frames.iter().map(|f| f.0).fold(f64::NEG_INFINITY, f64::max);
    let loud: Vec<&(f64, f64)> = frames.iter().filter(|f| f.0 >= loudest - 30.0).collect();
    if loud.len() < 2 {
        return 0.0;
    }
    let spread = |values: Vec<f64>| {
        let m = values.iter().sum::<f64>() / values.len() as f64;
        (values.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / values.len() as f64).sqrt()
    };
    let level = spread(loud.iter().map(|f| f.0).collect());
    let brightness = spread(loud.iter().map(|f| f.1).collect());
    (brightness + level / 20.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    fn tone(hz: f32, seconds: f32, am: Option<(f32, f32)>) -> Vec<f32> {
        let n = (SR as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let mut x = 0.5 * (std::f32::consts::TAU * hz * t).sin();
                if let Some((rate, depth)) = am {
                    x *= 1.0 - depth * 0.5 * (1.0 - (std::f32::consts::TAU * rate * t).cos());
                }
                x
            })
            .collect()
    }

    fn of(x: &[f32]) -> Character {
        measure(x, SR, 0, x.len() - 1)
    }

    #[test]
    fn a_steady_tone_is_smooth_still_and_has_one_peak() {
        let c = of(&tone(440.0, 1.0, None));
        assert_eq!(c.feel.roughness, Level::Low, "{c:?}");
        assert_eq!(c.feel.movement, Level::Low, "{c:?}");
        assert_eq!(c.feel.boom, Level::Low, "{c:?}");
        assert!((c.peaks[0].hz - 440.0).abs() < 2.0, "{c:?}");
        assert_eq!(c.peaks[0].db, 0.0);
    }

    #[test]
    fn a_fast_wobble_is_rough_and_shows_side_tones() {
        let c = of(&tone(1000.0, 1.0, Some((70.0, 1.0))));
        assert_eq!(c.feel.roughness, Level::High, "{c:?}");
        for side in [930.0, 1070.0] {
            assert!(
                c.peaks
                    .iter()
                    .any(|p| (p.hz - side).abs() < 3.0 && p.db > -12.0),
                "no side tone at {side}: {:?}",
                c.peaks
            );
        }
    }

    #[test]
    fn piercing_and_booming_tones_say_so() {
        assert_eq!(of(&tone(3200.0, 0.5, None)).feel.sharpness, Level::High);
        assert_eq!(of(&tone(45.0, 0.5, None)).feel.boom, Level::High);
        assert_eq!(of(&tone(440.0, 0.5, None)).feel.sharpness, Level::Low);
    }

    #[test]
    fn a_sweep_moves() {
        let n = SR as usize;
        let mut phase = 0.0f32;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let hz = 200.0 * (16.0f32).powf(i as f32 / n as f32);
                phase += hz / SR as f32;
                0.5 * (std::f32::consts::TAU * phase).sin()
            })
            .collect();
        assert_eq!(of(&x).feel.movement, Level::High, "{:?}", of(&x));
    }
}
