//! What happens when: a coarse level-over-time list and note onsets.
//!
//! * **Energy over time**: the sound cut into [`ENERGY_SLICES`] equal
//!   slices, each with its RMS level, so an agent can see a fade, a gap or a
//!   second hit without a picture.
//! * **Onsets**: the moments a new note or burst starts. The first onset is
//!   always the start of the sound (its first sample above -60 dBFS). Later
//!   ones are peaks in *spectral flux*: how much new energy appears in the
//!   spectrum from one 23 ms frame to the frame 11.6 ms earlier. New energy
//!   at a bin only counts if no bin within about half a semitone had it
//!   before, so vibrato and slow sweeps do not read as notes, while a new
//!   note (an arpeggio step, a chord entering, an echo repeat) or a noise
//!   burst does. A peak must also be a large enough share of the frame's
//!   spectrum ([`MIN_NEW_SHARE_TONAL`], or [`MIN_NEW_SHARE_NOISY`] in a frame
//!   that is not tonal), so the flicker of sustained noise is not read as
//!   hits, and must not leave the sound more than 10 dB quieter, so a note
//!   cut off abruptly is not read as a new one.
//!   Each onset carries the pitch of what started there (the strongest
//!   frequency that is louder after the onset than before it, so a melody
//!   over a held bass reads as the melody) and the level of the stretch up
//!   to the next onset.

use crate::features::{a_weights, dominant_hz, fft_size, min_bin, peak_and_total, rms, words};
use crate::fft::power_spectrum;
use serde::{Deserialize, Serialize};

/// Number of slices in [`EnergyOverTime`].
pub const ENERGY_SLICES: usize = 20;

/// RMS level in equal slices over the whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyOverTime {
    /// Length of each slice in seconds; slice `i` starts at
    /// `i × slice_seconds`.
    pub slice_seconds: f64,
    /// RMS level of each slice over all channels, in dBFS rounded to
    /// 0.1 dB. `null` for digital silence.
    pub rms_dbfs: Vec<Option<f32>>,
}

/// One note or burst onset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Onset {
    /// Seconds from the start of the file.
    pub time_seconds: f64,
    /// The pitch that starts here: the strongest frequency in the stretch
    /// up to the next onset (at most 100 ms) that is louder than in the
    /// same length just before the onset, Hz. A note entering over a held
    /// bass reads as the note; for a chord it is its loudest new note.
    /// `null` when the stretch is noisy (tonality at or below 0.3) or
    /// silent.
    pub note_hz: Option<f32>,
    /// RMS level of the stretch, dBFS. `null` for digital silence.
    pub level_dbfs: Option<f32>,
}

/// Energy over time of interleaved samples.
pub fn energy_over_time(samples: &[f32], channels: usize, sample_rate: u32) -> EnergyOverTime {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let slices = ENERGY_SLICES.min(frames);
    let rms_dbfs = (0..slices)
        .map(|i| {
            let (a, b) = (i * frames / slices, (i + 1) * frames / slices);
            let r = rms(&samples[a * channels..b * channels]);
            (r > 0.0).then(|| (200.0 * r.log10()).round() / 10.0)
        })
        .collect();
    EnergyOverTime {
        slice_seconds: if slices == 0 {
            0.0
        } else {
            frames as f64 / sample_rate as f64 / slices as f64
        },
        rms_dbfs,
    }
}

/// Spectral flux log compression: `ln(1 + GAMMA · |X|)` with `|X|`
/// normalized so a full-scale sine is 1. Content 60 dB down still counts a
/// little; the noise floor does not.
const GAMMA: f64 = 100.0;
/// Half-width of the previous-frame maximum filter, as a share of the bin
/// frequency (0.03 ≈ half a semitone).
const SPREAD: f64 = 0.03;
/// Frames compared: the current one and the one `LAG` hops earlier. With
/// hops of a quarter frame this is half a frame, which puts the flux peak
/// at the moment the new sound starts.
const LAG: usize = 2;
/// A flux peak must be the largest within this many hops on each side.
const PEAK_RADIUS: usize = 3;
/// ...and exceed the mean of the hops from `MEAN_BEFORE` before to
/// `MEAN_AFTER` after it by [`DELTA`] of the sound's largest flux.
const MEAN_BEFORE: usize = 10;
const MEAN_AFTER: usize = 3;
/// Share of the largest flux a peak must stand above its neighbourhood.
const DELTA: f64 = 0.1;
/// Least share of a frame's (log-compressed) spectrum that must be new
/// for a flux peak to count as an onset in a tonal frame. Note steps reach
/// 0.18 and more; vibrato and sweeps stay near 0.1.
pub const MIN_NEW_SHARE_TONAL: f64 = 0.15;
/// The same in a frame that is not tonal (tonality below 0.6, the
/// "tonal" word). Sustained noise, even a narrow resonant band that reads
/// partly tonal, flickers up to about 0.21; a burst over silence or over a
/// quieter sound is well above this.
pub const MIN_NEW_SHARE_NOISY: f64 = 0.35;
/// The RMS of the 23 ms after an onset must be at least this share of the
/// 23 ms before it (-10 dB), so the click of a note cut off is not read as
/// a new one, while a softer next note still is.
const MIN_LEVEL_AFTER: f32 = 0.316;
/// Onsets closer than this (seconds) are one onset.
pub const MIN_ONSET_GAP: f64 = 0.025;
/// Longest stretch after an onset used for its pitch and level (seconds).
const NOTE_WINDOW: f64 = 0.1;
/// Highest frequency the flux looks at; above it are mostly harmonics and
/// aliases of naive oscillators, which move with every sweep.
const FLUX_MAX_HZ: f64 = 8000.0;

/// Note onsets of `mono` within its active span `first..=last`.
pub fn onsets(mono: &[f32], sample_rate: u32, active: Option<(usize, usize)>) -> Vec<Onset> {
    let Some((first, last)) = active else {
        return Vec::new();
    };
    let sr = sample_rate as f64;
    let n = (fft_size(sample_rate) / 2).max(256);
    let hop = n / 4;
    let lo = min_bin(sample_rate, n).max((30.0 * n as f64 / sr).ceil() as usize);
    let hi = ((FLUX_MAX_HZ.min(0.45 * sr) * n as f64 / sr) as usize).min(n / 2);
    let norm = n as f64 / 4.0; // |X| of a full-scale sine under a Hann window
                               // Bin ranges for the maximum filter; both ends never decrease with k,
                               // so a sliding-window maximum works.
    let spans: Vec<(usize, usize)> = (lo..=hi)
        .map(|k| {
            let r = ((k as f64 * SPREAD).round() as usize).max(1);
            (k.saturating_sub(r).max(lo), (k + r).min(hi))
        })
        .collect();

    // Frames centred from half a frame before the sound to half after.
    let start = first as isize - (n / 2) as isize;
    let end = last as isize + (n / 2) as isize;
    let mut frame = vec![0.0f32; n];
    let mut history: Vec<Vec<f64>> = Vec::new();
    let mut centres = Vec::new();
    let mut flux = Vec::new();
    let mut totals: Vec<f64> = Vec::new();
    let mut c = start;
    while c <= end {
        let a = c - (n / 2) as isize;
        for (i, x) in frame.iter_mut().enumerate() {
            let j = a + i as isize;
            *x = if j >= 0 && (j as usize) < mono.len() {
                mono[j as usize]
            } else {
                0.0
            };
        }
        let p = power_spectrum(&frame, n);
        let s: Vec<f64> = (lo..=hi)
            .map(|k| (1.0 + GAMMA * p[k].sqrt() / norm).ln())
            .collect();
        let f = if history.len() >= LAG {
            let prev = sliding_max(&history[history.len() - LAG], &spans, lo);
            s.iter()
                .zip(&prev)
                .map(|(x, y)| (x - y).max(0.0))
                .sum::<f64>()
        } else {
            s.iter().sum::<f64>()
        };
        totals.push(s.iter().sum::<f64>());
        history.push(s);
        if history.len() > LAG {
            history.remove(0);
        }
        centres.push(c);
        flux.push(f);
        c += hop as isize;
    }

    let max = flux.iter().copied().fold(0.0, f64::max);
    let mut times = vec![first];
    if max > 0.0 {
        let gap = (MIN_ONSET_GAP * sr) as usize;
        let mut candidates: Vec<(usize, f64)> = Vec::new();
        for t in 0..flux.len() {
            let a = t.saturating_sub(PEAK_RADIUS);
            let b = (t + PEAK_RADIUS + 1).min(flux.len());
            let is_peak = flux[a..t].iter().all(|&x| flux[t] > x)
                && flux[t + 1..b].iter().all(|&x| flux[t] >= x);
            if !is_peak {
                continue;
            }
            let a = t.saturating_sub(MEAN_BEFORE);
            let b = (t + MEAN_AFTER + 1).min(flux.len());
            let mean = flux[a..b].iter().sum::<f64>() / (b - a) as f64;
            if flux[t] < mean + DELTA * max {
                continue;
            }
            let at = centres[t].max(0) as usize;
            if at < first + gap || at > last {
                continue;
            }
            // A note ending abruptly splatters across the spectrum too;
            // an onset must not leave the sound much quieter than before.
            // Both windows count samples past either end as silence.
            let energy = |a: usize, b: usize| {
                let end = b.min(mono.len());
                let start = a.min(end);
                mono[start..end]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
            };
            let before = energy(at.saturating_sub(n), at);
            let after = energy(at, at + n);
            let floor = (MIN_LEVEL_AFTER as f64) * (MIN_LEVEL_AFTER as f64);
            if after < before * floor {
                continue;
            }
            let share = flux[t] / totals[t].max(f64::MIN_POSITIVE);
            let min_share = if tonality_at(mono, at, sample_rate) >= words::TONAL as f64 {
                MIN_NEW_SHARE_TONAL
            } else {
                MIN_NEW_SHARE_NOISY
            };
            if share < min_share {
                continue;
            }
            match candidates.last_mut() {
                Some((prev, strength)) if at < *prev + gap => {
                    if flux[t] > *strength {
                        *prev = at;
                        *strength = flux[t];
                    }
                }
                _ => candidates.push((at, flux[t])),
            }
        }
        times.extend(candidates.into_iter().map(|(at, _)| at));
    }

    let window = (NOTE_WINDOW * sr) as usize;
    // Two resolutions, as for tonality: 46 ms follows sweeps, and a window
    // long enough for the whole stretch resolves the close harmonics of low
    // notes. The more tonal reading is used.
    let short = fft_size(sample_rate);
    let views = [
        NoteView::new(sample_rate, short),
        NoteView::new(sample_rate, window.next_power_of_two().max(short)),
    ];
    (0..times.len())
        .map(|i| {
            let a = times[i];
            let b = times
                .get(i + 1)
                .copied()
                .unwrap_or(last + 1)
                .min(a + window)
                .min(mono.len())
                .max(a + 1);
            let stretch = &mono[a..b];
            let level = rms(stretch);
            let note_hz = (level > crate::SILENCE_THRESHOLD_LINEAR)
                .then(|| {
                    let readings = views.iter().map(|v| (v, v.tonality(stretch)));
                    let (view, (tonality, after)) =
                        readings.max_by(|x, y| x.1 .0.total_cmp(&y.1 .0))?;
                    if tonality <= words::NOISY as f64 {
                        return None;
                    }
                    let before = &mono[a.saturating_sub(b - a)..a];
                    view.new_note(&after, before, sample_rate)
                })
                .flatten();
            Onset {
                time_seconds: a as f64 / sr,
                note_hz,
                level_dbfs: (level > 0.0).then(|| 20.0 * level.log10()),
            }
        })
        .collect()
}

/// Spectra of one FFT size for onset pitches.
struct NoteView {
    size: usize,
    lo: usize,
    weights: Vec<f64>,
    spans: Vec<(usize, usize)>,
}

impl NoteView {
    fn new(sample_rate: u32, size: usize) -> Self {
        let lo = min_bin(sample_rate, size);
        Self {
            size,
            lo,
            weights: a_weights(sample_rate, size),
            spans: (lo..=size / 2)
                .map(|k| {
                    let r = ((k as f64 * SPREAD).round() as usize).max(1);
                    (k.saturating_sub(r).max(lo), (k + r).min(size / 2))
                })
                .collect(),
        }
    }

    /// Tonality of `stretch` at this size, with its power spectrum.
    fn tonality(&self, stretch: &[f32]) -> (f64, Vec<f64>) {
        let p = power_spectrum(stretch, self.size);
        let (peak, total) = peak_and_total(&p, &self.weights, self.lo);
        let tonality = if total > 0.0 { peak / total } else { 0.0 };
        (tonality, p)
    }

    /// The strongest frequency in `after` that is louder than twice (3 dB
    /// over) the strongest nearby bin in `before`; the strongest overall
    /// when nothing is new.
    fn new_note(&self, after: &[f64], before: &[f32], sample_rate: u32) -> Option<f32> {
        let mut new = vec![0.0; after.len()];
        if before.is_empty() {
            new.copy_from_slice(after);
        } else {
            let pb = power_spectrum(before, self.size);
            let held = sliding_max(&pb[self.lo..], &self.spans, self.lo);
            for (k, h) in (self.lo..).zip(held) {
                new[k] = (after[k] - 2.0 * h).max(0.0);
            }
        }
        let spectrum = if new.iter().any(|&x| x > 0.0) {
            &new
        } else {
            after
        };
        dominant_hz(spectrum, self.lo, sample_rate, self.size)
    }
}

/// Tonality of the 46 ms around frame `at`, as in the analysis.
fn tonality_at(mono: &[f32], at: usize, sample_rate: u32) -> f64 {
    let size = fft_size(sample_rate);
    let a = at.saturating_sub(size / 2);
    let b = (a + size).min(mono.len());
    let p = power_spectrum(&mono[a..b], size);
    let lo = min_bin(sample_rate, size);
    let (peak, total) = peak_and_total(&p, &a_weights(sample_rate, size), lo);
    if total > 0.0 {
        peak / total
    } else {
        0.0
    }
}

/// For each bin `lo + i`, the maximum of `s` (indexed from `lo`) over
/// `spans[i]`. Span ends never decrease, so a monotonic queue does it in
/// one pass.
fn sliding_max(s: &[f64], spans: &[(usize, usize)], lo: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(spans.len());
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    let mut next = lo;
    for &(a, b) in spans {
        while next <= b {
            while queue.back().is_some_and(|&j| s[j - lo] <= s[next - lo]) {
                queue.pop_back();
            }
            queue.push_back(next);
            next += 1;
        }
        while queue.front().is_some_and(|&j| j < a) {
            queue.pop_front();
        }
        out.push(queue.front().map_or(0.0, |&j| s[j - lo]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_max_matches_brute_force() {
        let s: Vec<f64> = (0..50).map(|i| ((i * 37) % 11) as f64).collect();
        let lo = 3;
        let spans: Vec<(usize, usize)> = (lo..lo + 50usize)
            .map(|k| {
                (
                    k.saturating_sub(k / 10 + 1).max(lo),
                    (k + k / 10 + 1).min(lo + 49),
                )
            })
            .collect();
        let fast = sliding_max(&s, &spans, lo);
        for (i, &(a, b)) in spans.iter().enumerate() {
            let slow = (a..=b).map(|j| s[j - lo]).fold(f64::MIN, f64::max);
            assert_eq!(fast[i], slow, "bin {}", lo + i);
        }
    }

    #[test]
    fn energy_slices_cover_the_file() {
        let mut s = vec![0.5f32; 1000];
        s.extend(vec![0.0f32; 1000]);
        let e = energy_over_time(&s, 1, 1000);
        assert_eq!(e.rms_dbfs.len(), ENERGY_SLICES);
        assert!((e.slice_seconds - 0.1).abs() < 1e-12);
        assert_eq!(e.rms_dbfs[0], Some(-6.0));
        assert_eq!(e.rms_dbfs[19], None);
        assert!(energy_over_time(&[], 1, 1000).rms_dbfs.is_empty());
    }
}
