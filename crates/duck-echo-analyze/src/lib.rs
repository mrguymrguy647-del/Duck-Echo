//! Duck Echo analysis: numbers an agent can read instead of listening.
//!
//! [`analyze`] takes float samples and returns an [`Analysis`] with length,
//! peak, RMS, loudness, clipping and silence. Multi-channel input is
//! measured across all channels (peak, clipping, loudness summed per
//! BS.1770; a frame is silent only when every channel is), with
//! per-channel peak and RMS in `per_channel`.

use serde::{Deserialize, Serialize};

pub mod compare;
pub mod features;
pub mod fft;
pub mod picture;
pub mod png;
pub mod timeline;

pub use compare::{compare, Comparison, Signal};
pub use features::PitchPoint;
pub use picture::{Picture, PictureInfo};
pub use timeline::{EnergyOverTime, Onset};

/// Samples at or above this magnitude count as clipped. 16-bit full scale
/// is 32767/32768, so a hard-clipped render still lands above this.
pub const CLIP_THRESHOLD: f32 = 0.99;

/// Level below which audio counts as silence, in dBFS.
pub const SILENCE_THRESHOLD_DBFS: f32 = -60.0;

/// [`SILENCE_THRESHOLD_DBFS`] as a linear magnitude. The renderer's
/// `trim_silence` uses the same value, so a trimmed render analyzes with no
/// trailing silence.
pub const SILENCE_THRESHOLD_LINEAR: f32 = 0.001;

/// BS.1770 gating block length in seconds. Clips shorter than this cannot
/// be measured with gating and use [`LoudnessMethod::KWeightedUngated`].
pub const GATING_BLOCK_SECONDS: f64 = 0.4;

/// How `loudness_lufs` was measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoudnessMethod {
    /// ITU-R BS.1770-4 integrated loudness: K-weighting, 400 ms blocks with
    /// 75% overlap, absolute gate at -70 LUFS, relative gate at -10 LU.
    /// Used for clips of at least 400 ms.
    Bs1770Gated,
    /// Approximation for clips shorter than 400 ms, where gating is
    /// impossible: the same K-weighting and channel sum, with the mean
    /// square taken over the active span (first to last sample above the
    /// silence threshold) instead of gated blocks, so leading and trailing
    /// padding do not change it. For a steady tone it agrees with the gated
    /// method within 0.2 dB (tested). It is an approximation: no standard
    /// defines loudness for clips this short, so compare short sounds with
    /// each other rather than with long ones.
    KWeightedUngated,
}

/// Measurements of one sound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// One line in words, built from the numbers below with fixed
    /// thresholds (see `features::words`), for example
    /// "0.31 s, ~-5.3 LUFS, bright, sharp attack, tonal around 988 Hz,
    /// no clipping".
    pub summary: String,
    /// Samples per second.
    pub sample_rate: u32,
    /// Channels in the input.
    pub channels: u16,
    /// Frames (samples per channel).
    pub frames: usize,
    /// Length in seconds.
    pub duration_seconds: f64,
    /// Largest absolute sample over all channels, linear 0..=1.
    pub peak: f32,
    /// Largest absolute sample in dBFS (`null` when silent).
    pub peak_dbfs: Option<f32>,
    /// Root mean square level over all samples of all channels, linear.
    pub rms: f32,
    /// RMS level in dBFS (`null` when silent).
    pub rms_dbfs: Option<f32>,
    /// Loudness in LUFS. See `loudness_method` for how it was measured.
    /// `null` only when the sound is silent (or, for clips of 400 ms or
    /// more, entirely below the -70 LUFS absolute gate).
    pub loudness_lufs: Option<f32>,
    /// `bs1770_gated` for clips of at least 400 ms, `k_weighted_ungated`
    /// for shorter ones, `null` when `loudness_lufs` is `null`.
    pub loudness_method: Option<LoudnessMethod>,
    /// Peak and RMS of each channel, in channel order.
    pub per_channel: Vec<ChannelLevels>,
    /// Clipping report.
    pub clipping: Clipping,
    /// Seconds of silence before the first non-silent sample.
    pub leading_silence_seconds: f64,
    /// Seconds of silence after the last non-silent sample.
    pub trailing_silence_seconds: f64,
    /// True when no sample rises above the silence threshold.
    pub is_silent: bool,
    /// Mean sample value; should be near 0.
    pub dc_offset: f32,
    /// Peak over RMS of the non-silent part, in dB. A sine is 3 dB, a
    /// square 0 dB; spiky, percussive sounds are higher. `null` when silent.
    pub crest_factor_db: Option<f32>,
    /// Seconds from the first non-silent sample until the short-term peak
    /// level (largest sample in the previous 20 ms) reaches 90% of the
    /// sound's peak. `null` when silent.
    pub attack_seconds: Option<f64>,
    /// Seconds from the last sample at 90% of the peak until no sample in
    /// the next 20 ms reaches 10% of it (-20 dB). `null` when silent or
    /// when it never falls that far.
    pub decay_seconds: Option<f64>,
    /// Magnitude-weighted mean frequency of the non-silent part, Hz. Higher
    /// is brighter. `null` when silent.
    pub spectral_centroid_hz: Option<f32>,
    /// Share of power in prominent spectral peaks, 0..1: tones near 1,
    /// broadband noise near 0. `null` when silent.
    pub tonality: Option<f32>,
    /// Dominant frequency, level and tonality in up to 8 evenly spaced
    /// windows over the non-silent part. Spectral measures use the mean of
    /// the channels.
    pub pitch_track: Vec<PitchPoint>,
    /// RMS level in 20 equal slices over the whole file, a coarse picture
    /// of the level over time.
    pub energy_over_time: EnergyOverTime,
    /// Where notes or bursts start: the start of the sound, then every peak
    /// of new spectral energy, each with the dominant frequency and level
    /// of the stretch up to the next onset. Uses the mean of the channels.
    pub onsets: Vec<Onset>,
    /// Threshold used for the silence measurements, in dBFS.
    pub silence_threshold_dbfs: f32,
}

/// Level of one channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelLevels {
    /// Largest absolute sample, linear.
    pub peak: f32,
    /// Peak in dBFS (`null` when silent).
    pub peak_dbfs: Option<f32>,
    /// Root mean square level, linear.
    pub rms: f32,
    /// RMS in dBFS (`null` when silent).
    pub rms_dbfs: Option<f32>,
}

/// Clipping report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clipping {
    /// True when any sample reaches the clip threshold.
    pub clipped: bool,
    /// Number of samples at or above the clip threshold.
    pub clipped_samples: usize,
    /// Longest run of consecutive clipped samples within one channel.
    pub longest_run: usize,
    /// Threshold used, linear.
    pub threshold: f32,
}

fn to_dbfs(x: f32) -> Option<f32> {
    if x > 0.0 {
        Some(20.0 * x.log10())
    } else {
        None
    }
}

/// Analyze interleaved samples.
pub fn analyze(samples: &[f32], sample_rate: u32, channels: u16) -> Analysis {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let samples = &samples[..frames * ch];
    let planar: Vec<Vec<f32>> = (0..ch)
        .map(|c| samples.iter().skip(c).step_by(ch).copied().collect())
        .collect();
    let sr = sample_rate as f64;

    let mut peak = 0.0f32;
    let mut sum_sq_all = 0.0f64;
    let mut sum_all = 0.0f64;
    let mut clipped_samples = 0;
    let mut longest_run = 0;
    let mut per_channel = Vec::with_capacity(ch);
    for chan in &planar {
        let mut ch_peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut run = 0;
        for &s in chan {
            let a = s.abs();
            ch_peak = ch_peak.max(a);
            sum_sq += (s as f64) * (s as f64);
            sum_all += s as f64;
            if a >= CLIP_THRESHOLD {
                clipped_samples += 1;
                run += 1;
                longest_run = longest_run.max(run);
            } else {
                run = 0;
            }
        }
        peak = peak.max(ch_peak);
        sum_sq_all += sum_sq;
        let rms = if frames > 0 {
            (sum_sq / frames as f64).sqrt() as f32
        } else {
            0.0
        };
        per_channel.push(ChannelLevels {
            peak: ch_peak,
            peak_dbfs: to_dbfs(ch_peak),
            rms,
            rms_dbfs: to_dbfs(rms),
        });
    }
    let total = samples.len();
    let rms = if total > 0 {
        (sum_sq_all / total as f64).sqrt() as f32
    } else {
        0.0
    };
    let dc_offset = if total > 0 {
        (sum_all / total as f64) as f32
    } else {
        0.0
    };

    let active = |f: usize| {
        samples[f * ch..(f + 1) * ch]
            .iter()
            .any(|s| s.abs() > SILENCE_THRESHOLD_LINEAR)
    };
    let first = (0..frames).find(|&f| active(f));
    let last = (0..frames).rev().find(|&f| active(f));
    let (leading, trailing, is_silent) = match (first, last) {
        (Some(f), Some(l)) => (f as f64 / sr, (frames - 1 - l) as f64 / sr, false),
        _ => (frames as f64 / sr, 0.0, true),
    };

    let slices: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
    let (loudness_lufs, loudness_method) = match loudness(&slices, sample_rate) {
        Some((l, m)) => (Some(l), Some(m)),
        None => (None, None),
    };

    let mono: Vec<f32> = if ch == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    let active = first.zip(last);
    let spectral = features::spectral(&mono, sample_rate, active);
    let onsets = timeline::onsets(&mono, sample_rate, active);
    let energy_over_time = timeline::energy_over_time(samples, ch, sample_rate);
    let times = features::envelope_times(samples, ch, sample_rate, first);
    let crest_factor_db = active.and_then(|(f, l)| {
        let span = &samples[f * ch..(l + 1) * ch];
        let r = (span.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / span.len() as f64)
            .sqrt() as f32;
        let p = span.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        (r > 0.0).then(|| 20.0 * (p / r).log10())
    });

    let mut analysis = Analysis {
        summary: String::new(),
        sample_rate,
        channels,
        frames,
        duration_seconds: frames as f64 / sr,
        peak,
        peak_dbfs: to_dbfs(peak),
        rms,
        rms_dbfs: to_dbfs(rms),
        loudness_lufs,
        loudness_method,
        per_channel,
        clipping: Clipping {
            clipped: clipped_samples > 0,
            clipped_samples,
            longest_run,
            threshold: CLIP_THRESHOLD,
        },
        leading_silence_seconds: leading,
        trailing_silence_seconds: trailing,
        is_silent,
        dc_offset,
        crest_factor_db,
        attack_seconds: times.attack_seconds,
        decay_seconds: times.decay_seconds,
        spectral_centroid_hz: spectral.centroid_hz,
        tonality: spectral.tonality,
        pitch_track: spectral.track,
        energy_over_time,
        onsets,
        silence_threshold_dbfs: SILENCE_THRESHOLD_DBFS,
    };
    analysis.summary = features::summary(&analysis);
    analysis
}

/// Audio EQ Cookbook biquad in f64, used only for measurement.
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn from_coeffs(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// High shelf in the form used to derive the BS.1770 pre-filter. At
    /// 48 kHz this reproduces the coefficients printed in the spec.
    fn high_shelf(sample_rate: f64, f0: f64, gain_db: f64, q: f64) -> Self {
        let k = (std::f64::consts::PI * f0 / sample_rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        Self::from_coeffs(
            [
                (vh + vb * k / q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / q + k * k) / a0,
            ],
            [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        )
    }

    /// The BS.1770 RLB high-pass (revised low-frequency B-curve).
    fn high_pass(sample_rate: f64, f0: f64, q: f64) -> Self {
        let k = (std::f64::consts::PI * f0 / sample_rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Self::from_coeffs(
            [1.0, -2.0, 1.0],
            [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        )
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Loudness of a signal given as one slice per channel (equal lengths),
/// with the method used. Channel weights are 1.0 (BS.1770 for left, right
/// and centre). Returns `None` for silence.
pub fn loudness(channels: &[&[f32]], sample_rate: u32) -> Option<(f32, LoudnessMethod)> {
    let sr = sample_rate as f64;
    let frames = channels.iter().map(|c| c.len()).min()?;
    if frames == 0 {
        return None;
    }
    // Squared K-weighted signal per channel. Pre-filter and RLB filter use
    // the parameters behind the 48 kHz coefficients in the spec,
    // recomputed for the actual sample rate.
    let squared: Vec<Vec<f64>> = channels
        .iter()
        .map(|ch| {
            let mut shelf = Biquad::high_shelf(
                sr,
                1_681.974_450_955_533,
                3.999_843_853_973_347,
                0.707_175_196_848_751_9,
            );
            let mut hp = Biquad::high_pass(sr, 38.135_470_876_002_17, 0.500_327_040_491_404_6);
            ch[..frames]
                .iter()
                .map(|&s| {
                    let w = hp.process(shelf.process(s as f64));
                    w * w
                })
                .collect()
        })
        .collect();
    // Mean square of the channel sum over a frame range.
    let mean_square = |range: std::ops::Range<usize>| -> f64 {
        let n = range.len() as f64;
        squared
            .iter()
            .map(|ch| ch[range.clone()].iter().sum::<f64>() / n)
            .sum()
    };
    let to_lufs = |ms: f64| -0.691 + 10.0 * ms.log10();

    let block = (GATING_BLOCK_SECONDS * sr).round() as usize;
    if frames < block || block == 0 {
        let active = |i: usize| {
            channels
                .iter()
                .any(|c| c[i].abs() > SILENCE_THRESHOLD_LINEAR)
        };
        let first = (0..frames).find(|&i| active(i))?;
        let last = (0..frames).rev().find(|&i| active(i))?;
        let ms = mean_square(first..last + 1);
        return (ms > 0.0).then(|| (to_lufs(ms) as f32, LoudnessMethod::KWeightedUngated));
    }

    let hop = block / 4;
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + block <= frames {
        blocks.push(mean_square(start..start + block));
        start += hop;
    }
    let abs_gate: Vec<f64> = blocks
        .into_iter()
        .filter(|&ms| ms > 0.0 && to_lufs(ms) > -70.0)
        .collect();
    if abs_gate.is_empty() {
        return None;
    }
    let mean = abs_gate.iter().sum::<f64>() / abs_gate.len() as f64;
    let rel_threshold = to_lufs(mean) - 10.0;
    let rel_gate: Vec<f64> = abs_gate
        .into_iter()
        .filter(|&ms| to_lufs(ms) > rel_threshold)
        .collect();
    if rel_gate.is_empty() {
        return None;
    }
    let mean = rel_gate.iter().sum::<f64>() / rel_gate.len() as f64;
    Some((to_lufs(mean) as f32, LoudnessMethod::Bs1770Gated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, seconds: f32, amp: f32, sr: u32) -> Vec<f32> {
        (0..(seconds * sr as f32) as usize)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn silence_is_silent() {
        let a = analyze(&vec![0.0; 44100], 44100, 1);
        assert!(a.is_silent);
        assert_eq!(a.peak, 0.0);
        assert_eq!(a.peak_dbfs, None);
        assert_eq!(a.loudness_lufs, None);
        assert_eq!(a.loudness_method, None);
        assert!(!a.clipping.clipped);
        assert!((a.leading_silence_seconds - 1.0).abs() < 1e-9);
    }

    #[test]
    fn full_scale_1khz_sine_is_about_minus_three_lufs() {
        // BS.1770 reference: a 997 Hz full-scale mono sine reads -3.01 LUFS.
        let s = sine(997.0, 2.0, 1.0, 48000);
        let a = analyze(&s, 48000, 1);
        let l = a.loudness_lufs.expect("loudness");
        assert!((l + 3.01).abs() < 0.1, "got {l}");
        assert_eq!(a.loudness_method, Some(LoudnessMethod::Bs1770Gated));
        assert!((a.peak - 1.0).abs() < 1e-3);
        assert!((a.rms_dbfs.unwrap() + 3.01).abs() < 0.05);
    }

    #[test]
    fn clipping_and_silence_edges() {
        let mut s = vec![0.0f32; 4410];
        s.extend(sine(440.0, 0.1, 0.5, 44100));
        s.extend(vec![1.0f32; 10]);
        s.extend(vec![0.0f32; 2205]);
        let a = analyze(&s, 44100, 1);
        assert!(a.clipping.clipped);
        assert_eq!(a.clipping.clipped_samples, 10);
        assert_eq!(a.clipping.longest_run, 10);
        assert!((a.leading_silence_seconds - 0.1).abs() < 1e-3);
        assert!((a.trailing_silence_seconds - 0.05).abs() < 1e-3);
    }

    #[test]
    fn short_clips_get_ungated_loudness_that_agrees_with_gated() {
        // 200 ms full-scale 997 Hz sine: the BS.1770 reference level is
        // -3.01 LUFS, and the short method should land close to it.
        let short = sine(997.0, 0.2, 1.0, 48000);
        let a = analyze(&short, 48000, 1);
        assert_eq!(a.loudness_method, Some(LoudnessMethod::KWeightedUngated));
        let l = a.loudness_lufs.unwrap();
        assert!((l + 3.01).abs() < 0.2, "got {l}");

        // Padding with silence must not change the short measurement.
        let mut padded = vec![0.0; 2400];
        padded.extend(&short);
        padded.extend(vec![0.0; 4800]);
        let p = analyze(&padded, 48000, 1);
        assert_eq!(p.loudness_method, Some(LoudnessMethod::KWeightedUngated));
        assert!((p.loudness_lufs.unwrap() - l).abs() < 0.05);

        // A steady -12 dBFS tone reads the same with both methods.
        let amp = 0.251_188_6;
        let s1 = analyze(&sine(440.0, 0.35, amp, 44100), 44100, 1);
        let s2 = analyze(&sine(440.0, 2.0, amp, 44100), 44100, 1);
        assert_eq!(s2.loudness_method, Some(LoudnessMethod::Bs1770Gated));
        let (u, g) = (s1.loudness_lufs.unwrap(), s2.loudness_lufs.unwrap());
        assert!((u - g).abs() < 0.2, "ungated {u} vs gated {g}");
    }

    #[test]
    fn loudness_sums_channels() {
        // Identical content in two channels is 3 dB louder than in one.
        let s = sine(997.0, 1.0, 0.5, 48000);
        let mono = loudness(&[&s], 48000).unwrap().0;
        let stereo = loudness(&[&s, &s], 48000).unwrap().0;
        assert!((stereo - mono - 3.01).abs() < 0.02, "{mono} {stereo}");
    }
}
