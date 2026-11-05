//! Deterministic math for the audio path.
//!
//! Rust's `f32::sin`, `exp2`, `powf` and friends call the platform's libm,
//! whose results differ by a few ULPs between operating systems. Everything
//! here uses only `+ - * /`, `floor`, `sqrt` and bit casts, which are
//! IEEE-754 exact, so the same recipe renders to the same bytes everywhere.
//! Accuracy is far beyond what 16-bit audio can show.

/// SplitMix64: tiny, fast, well distributed, and trivially seedable.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator for `seed`.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// A statistically independent generator derived from this seed and a
    /// stream tag. Used so each layer and each feature gets its own stream
    /// and adding a layer does not change the noise of the others.
    pub fn fork(&self, tag: u64) -> Self {
        let mut r = Rng(self.0 ^ tag.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        r.next_u64();
        r
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in `[-1, 1)`.
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

const PI: f32 = core::f32::consts::PI;
const TAU: f32 = core::f32::consts::TAU;

/// `sin(2π · turns)`. Input is in turns (1.0 = one full cycle), any value.
#[inline]
pub fn sin_turns(turns: f32) -> f32 {
    // Shift by half a turn so the [-π, π) window below is centred on 0.
    let shifted = turns + 0.5;
    let t = shifted - shifted.floor(); // [0, 1)
    let mut x = t * TAU - PI; // [-π, π), equal to 2π·turns mod 2π
                              // Fold into [-π/2, π/2] where the polynomial is accurate.
    if x > PI * 0.5 {
        x = PI - x;
    } else if x < -PI * 0.5 {
        x = -PI - x;
    }
    let x2 = x * x;
    // Taylor to x^9: max error ~3.5e-6 on the folded range.
    x * (1.0
        + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0)))))
}

/// `cos(2π · turns)`.
#[inline]
pub fn cos_turns(turns: f32) -> f32 {
    sin_turns(turns + 0.25)
}

/// `2^x` for `x` in roughly -126..=127. Clamped outside that range.
#[inline]
pub fn exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 127.0);
    let n = x.floor();
    let f = x - n; // [0, 1)
                   // Taylor series of 2^f to degree 6: relative error < 2e-5.
    #[allow(clippy::approx_constant)]
    let p = 1.0
        + f * (0.693_147_2
            + f * (0.240_226_5
                + f * (0.055_504_1 + f * (0.009_618_13 + f * (0.001_333_36 + f * 0.000_154_035)))));
    let scale = f32::from_bits(((n as i32 + 127) as u32) << 23);
    p * scale
}

/// `log2(x)` for positive `x`. Non-positive inputs return -126.
#[inline]
pub fn log2(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return -126.0;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
                                                                // ln(m) = 2·atanh((m-1)/(m+1)); y ≤ 1/3 so the series converges fast.
    let y = (m - 1.0) / (m + 1.0);
    let y2 = y * y;
    let ln_m =
        2.0 * y * (1.0 + y2 * (1.0 / 3.0 + y2 * (1.0 / 5.0 + y2 * (1.0 / 7.0 + y2 * (1.0 / 9.0)))));
    exp as f32 + ln_m * core::f32::consts::LOG2_E
}

/// `base^exponent` for positive `base`.
#[inline]
pub fn pow(base: f32, exponent: f32) -> f32 {
    exp2(exponent * log2(base))
}

/// Frequency ratio for a pitch offset in semitones.
#[inline]
pub fn semitones_to_ratio(semitones: f32) -> f32 {
    exp2(semitones * (1.0 / 12.0))
}

/// Linear gain for a level in decibels.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    exp2(db * (1.0 / 6.020_6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_matches_std_closely() {
        for i in 0..2000 {
            let t = i as f32 / 1234.5 - 0.7;
            let want = (t * TAU).sin();
            assert!((sin_turns(t) - want).abs() < 1e-5, "t={t}");
        }
    }

    #[test]
    fn exp2_and_log2_match_std_closely() {
        for i in -200..200 {
            let x = i as f32 / 17.0;
            let want = x.exp2();
            assert!(((exp2(x) - want) / want).abs() < 5e-5, "x={x}");
        }
        for i in 1..5000 {
            let x = i as f32 / 7.0;
            let want = x.log2();
            assert!((log2(x) - want).abs() < 2e-5, "x={x}");
        }
        assert!((pow(2.0, 10.0) - 1024.0).abs() < 0.05);
        assert!((semitones_to_ratio(12.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn rng_is_deterministic_and_forks_differ() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut f1 = Rng::new(42).fork(1);
        let mut f2 = Rng::new(42).fork(2);
        assert_ne!(f1.next_u64(), f2.next_u64());
        for _ in 0..1000 {
            let u = Rng::new(7).unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
