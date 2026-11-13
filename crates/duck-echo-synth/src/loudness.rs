//! Loudness measurement for `master.normalize_lufs`, on the audio path.
//!
//! This is the same measurement as `duck_echo_analyze::loudness` (BS.1770-4
//! K-weighting; gated integrated loudness for clips of at least 400 ms,
//! ungated over the active span for shorter ones), rewritten so a render
//! is the same on every platform: `f64` arithmetic with only `+ - * /` and
//! `sqrt`, the filter's `tan` from a Taylor series, and the constants that
//! need `pow` written out. Gates compare mean squares instead of LUFS, so
//! no logarithm is needed either. The analysis crate keeps the reference
//! version; a test checks that the two agree.
//!
//! Results are mean squares of the K-weighted channel sum (what BS.1770
//! calls `z`); loudness in LUFS is `-0.691 + 10·log10(z)`.

/// BS.1770 gating block length in seconds.
const BLOCK_SECONDS: f64 = 0.4;

/// Samples at or below this magnitude count as silence (-60 dBFS), as in
/// the analyzer.
const SILENCE: f32 = duck_echo_core::TRIM_THRESHOLD;

/// The -70 LUFS absolute gate as a mean square: `10^((-70 + 0.691) / 10)`.
const ABSOLUTE_GATE_MS: f64 = 1.172_465_304_582_298_1e-7;

/// High-shelf gain of the BS.1770 pre-filter as a linear amplitude:
/// `10^(3.999843853973347 / 20)`.
const SHELF_VH: f64 = 1.584_864_701_130_855_6;
/// `SHELF_VH^0.4996667741545416`, the shelf's band gain.
const SHELF_VB: f64 = 1.258_720_930_232_561_7;

/// `tan(x)` for `0 <= x < π/2`, from Taylor series of sine and cosine.
/// Only `+ - * /`, so every platform gets the same bits. Used for filter
/// coefficients, where `x = π·f0/sample_rate` stays below 0.7.
fn tan(x: f64) -> f64 {
    let x2 = x * x;
    let (mut s_term, mut sin) = (x, x);
    let (mut c_term, mut cos) = (1.0, 1.0);
    for k in 1..=12 {
        let k = k as f64;
        s_term *= -x2 / ((2.0 * k) * (2.0 * k + 1.0));
        sin += s_term;
        c_term *= -x2 / ((2.0 * k - 1.0) * (2.0 * k));
        cos += c_term;
    }
    sin / cos
}

/// A biquad in `f64`, direct form II transposed.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }

    /// The BS.1770 pre-filter (high shelf), for any sample rate.
    fn shelf(sample_rate: f64) -> Self {
        let (f0, q) = (1_681.974_450_955_533, 0.707_175_196_848_751_9);
        let k = tan(core::f64::consts::PI * f0 / sample_rate);
        let a0 = 1.0 + k / q + k * k;
        Self {
            b: [
                (SHELF_VH + SHELF_VB * k / q + k * k) / a0,
                2.0 * (k * k - SHELF_VH) / a0,
                (SHELF_VH - SHELF_VB * k / q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    }

    /// The BS.1770 RLB high-pass, for any sample rate.
    fn high_pass(sample_rate: f64) -> Self {
        let (f0, q) = (38.135_470_876_002_17, 0.500_327_040_491_404_6);
        let k = tan(core::f64::consts::PI * f0 / sample_rate);
        let a0 = 1.0 + k / q + k * k;
        Self {
            b: [1.0, -2.0, 1.0],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            z: [0.0; 2],
        }
    }
}

/// Loudness of interleaved `samples` as a mean square (see the module
/// docs), and whether it was gated (clips of at least 400 ms). `None` for
/// silence, or a gated clip entirely below -70 LUFS.
pub fn mean_square(samples: &[f32], channels: usize, sample_rate: u32) -> Option<(f64, bool)> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    if frames == 0 {
        return None;
    }
    let sr = sample_rate as f64;
    let squared: Vec<Vec<f64>> = (0..channels)
        .map(|c| {
            let (mut shelf, mut hp) = (Biquad::shelf(sr), Biquad::high_pass(sr));
            (0..frames)
                .map(|f| {
                    let w = hp.process(shelf.process(samples[f * channels + c] as f64));
                    w * w
                })
                .collect()
        })
        .collect();
    let block_ms = |a: usize, b: usize| -> f64 {
        let n = (b - a) as f64;
        squared
            .iter()
            .map(|ch| ch[a..b].iter().sum::<f64>() / n)
            .sum()
    };

    let block = (BLOCK_SECONDS * sr).round() as usize;
    if frames < block || block == 0 {
        let loud = |f: usize| {
            samples[f * channels..(f + 1) * channels]
                .iter()
                .any(|s| s.abs() > SILENCE)
        };
        let first = (0..frames).find(|&f| loud(f))?;
        let last = (0..frames).rev().find(|&f| loud(f))?;
        let ms = block_ms(first, last + 1);
        return (ms > 0.0).then_some((ms, false));
    }
    let hop = block / 4;
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + block <= frames {
        blocks.push(block_ms(start, start + block));
        start += hop;
    }
    let gated: Vec<f64> = blocks
        .into_iter()
        .filter(|&ms| ms > ABSOLUTE_GATE_MS)
        .collect();
    if gated.is_empty() {
        return None;
    }
    // Relative gate: 10 LU under the mean, i.e. a tenth of its mean square.
    let threshold = gated.iter().sum::<f64>() / gated.len() as f64 * 0.1;
    let kept: Vec<f64> = gated.into_iter().filter(|&ms| ms > threshold).collect();
    if kept.is_empty() {
        return None;
    }
    Some((kept.iter().sum::<f64>() / kept.len() as f64, true))
}

/// The linear gain that moves a sound measuring `mean_square` to
/// `target_lufs`. Uses [`crate::math::db_to_gain`], so it is deterministic.
pub fn gain_to(target_lufs: f32, mean_square: f64) -> f32 {
    // target z = 10^((L + 0.691)/10), so its square root is the amplitude
    // ratio 10^((L + 0.691)/20).
    (crate::math::db_to_gain(target_lufs + 0.691) as f64 / mean_square.sqrt()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_tan_matches_libm() {
        for i in 1..70 {
            let x = i as f64 * 0.01;
            assert!((tan(x) - x.tan()).abs() < 1e-14, "{x}");
        }
    }

    #[test]
    fn constants_match_their_formulas() {
        let vh = 10f64.powf(3.999_843_853_973_347 / 20.0);
        assert!((SHELF_VH - vh).abs() < 1e-14);
        assert!((SHELF_VB - vh.powf(0.499_666_774_154_541_6)).abs() < 1e-14);
        assert!((ABSOLUTE_GATE_MS - 10f64.powf((-70.0 + 0.691) / 10.0)).abs() < 1e-20);
    }

    #[test]
    fn full_scale_sine_reads_minus_three_lufs() {
        let sr = 48_000;
        let s: Vec<f32> = (0..sr * 2)
            .map(|i| (core::f64::consts::TAU * 997.0 * i as f64 / sr as f64).sin() as f32)
            .collect();
        let (ms, gated) = mean_square(&s, 1, sr as u32).unwrap();
        assert!(gated);
        let lufs = -0.691 + 10.0 * ms.log10();
        assert!((lufs + 3.01).abs() < 0.05, "{lufs}");
        let g_db = 20.0 * (gain_to(-23.0, ms) as f64).log10();
        assert!((g_db - (-23.0 - lufs)).abs() < 0.001, "{g_db}");
    }
}
