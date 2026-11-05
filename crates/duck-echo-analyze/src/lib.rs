//! Duck Echo analysis: numbers an agent can read instead of listening.
//!
//! [`analyze`] takes float samples and returns an [`Analysis`] with length,
//! peak, RMS, integrated loudness, clipping and silence. Multi-channel input
//! is averaged to mono first.

use serde::{Deserialize, Serialize};

/// Samples at or above this magnitude count as clipped. 16-bit full scale
/// is 32767/32768, so a hard-clipped render still lands above this.
pub const CLIP_THRESHOLD: f32 = 0.99;

/// Level below which audio counts as silence, in dBFS.
pub const SILENCE_THRESHOLD_DBFS: f32 = -60.0;

/// Measurements of one sound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    /// Samples per second.
    pub sample_rate: u32,
    /// Channels in the input.
    pub channels: u16,
    /// Frames (samples per channel).
    pub frames: usize,
    /// Length in seconds.
    pub duration_seconds: f64,
    /// Largest absolute sample, linear 0..=1.
    pub peak: f32,
    /// Largest absolute sample in dBFS (`null` when silent).
    pub peak_dbfs: Option<f32>,
    /// Root mean square level, linear.
    pub rms: f32,
    /// RMS level in dBFS (`null` when silent).
    pub rms_dbfs: Option<f32>,
    /// Integrated loudness in LUFS (ITU-R BS.1770 K-weighting with gating).
    /// `null` when the sound is shorter than 400 ms or fully gated.
    pub loudness_lufs: Option<f32>,
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
    /// Threshold used for the silence measurements, in dBFS.
    pub silence_threshold_dbfs: f32,
}

/// Clipping report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clipping {
    /// True when any sample reaches the clip threshold.
    pub clipped: bool,
    /// Number of samples at or above the clip threshold.
    pub clipped_samples: usize,
    /// Longest run of consecutive clipped samples.
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
    let mono: Vec<f32> = if channels <= 1 {
        samples.to_vec()
    } else {
        let ch = channels as usize;
        samples
            .chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    let frames = mono.len();
    let sr = sample_rate as f64;

    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    let mut sum = 0.0f64;
    let mut clipped_samples = 0;
    let mut longest_run = 0;
    let mut run = 0;
    for &s in &mono {
        let a = s.abs();
        peak = peak.max(a);
        sum_sq += (s as f64) * (s as f64);
        sum += s as f64;
        if a >= CLIP_THRESHOLD {
            clipped_samples += 1;
            run += 1;
            longest_run = longest_run.max(run);
        } else {
            run = 0;
        }
    }
    let rms = if frames > 0 {
        (sum_sq / frames as f64).sqrt() as f32
    } else {
        0.0
    };
    let dc_offset = if frames > 0 {
        (sum / frames as f64) as f32
    } else {
        0.0
    };

    let silence_lin = 10f32.powf(SILENCE_THRESHOLD_DBFS / 20.0);
    let first = mono.iter().position(|s| s.abs() > silence_lin);
    let last = mono.iter().rposition(|s| s.abs() > silence_lin);
    let (leading, trailing, is_silent) = match (first, last) {
        (Some(f), Some(l)) => (f as f64 / sr, (frames - 1 - l) as f64 / sr, false),
        _ => (frames as f64 / sr, 0.0, true),
    };

    Analysis {
        sample_rate,
        channels,
        frames,
        duration_seconds: frames as f64 / sr,
        peak,
        peak_dbfs: to_dbfs(peak),
        rms,
        rms_dbfs: to_dbfs(rms),
        loudness_lufs: integrated_loudness(&mono, sample_rate),
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
        silence_threshold_dbfs: SILENCE_THRESHOLD_DBFS,
    }
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

/// Integrated loudness per ITU-R BS.1770-4 for a mono signal: K-weighting,
/// 400 ms blocks with 75% overlap, absolute gate at -70 LUFS, relative gate
/// at -10 LU under the ungated mean.
pub fn integrated_loudness(mono: &[f32], sample_rate: u32) -> Option<f32> {
    let sr = sample_rate as f64;
    let block = (0.4 * sr).round() as usize;
    let hop = block / 4;
    if mono.len() < block || block == 0 {
        return None;
    }
    // Pre-filter and RLB filter with the parameters behind the 48 kHz
    // coefficients in the spec, recomputed for the actual sample rate.
    let mut shelf = Biquad::high_shelf(
        sr,
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_196_848_751_9,
    );
    let mut hp = Biquad::high_pass(sr, 38.135_470_876_002_17, 0.500_327_040_491_404_6);
    let weighted: Vec<f64> = mono
        .iter()
        .map(|&s| hp.process(shelf.process(s as f64)))
        .collect();

    // Block mean squares via a running sum.
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + block <= weighted.len() {
        let ms = weighted[start..start + block]
            .iter()
            .map(|v| v * v)
            .sum::<f64>()
            / block as f64;
        blocks.push(ms);
        start += hop;
    }
    let to_lufs = |ms: f64| -0.691 + 10.0 * ms.log10();

    let abs_gate: Vec<f64> = blocks
        .iter()
        .copied()
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
    Some(to_lufs(mean) as f32)
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
}
