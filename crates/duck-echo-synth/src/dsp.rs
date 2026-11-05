//! Per-layer building blocks: envelope, pitch curve, oscillator, filter.
//! Everything here is allocation-free once constructed.

use crate::math::{self, Rng};
use duck_echo_core::{Curve, Envelope, FilterKind, Source};

/// Envelope with times already converted to seconds (and jittered).
#[derive(Debug, Clone)]
pub struct EnvShape {
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    sustain_time: f32,
    release: f32,
}

impl EnvShape {
    /// Build from a recipe envelope, scaling every time by `time_scale`.
    pub fn new(e: &Envelope, time_scale: f32) -> Self {
        Self {
            attack: e.attack * time_scale,
            hold: e.hold * time_scale,
            decay: e.decay * time_scale,
            sustain: e.sustain,
            sustain_time: e.sustain_time * time_scale,
            release: e.release * time_scale,
        }
    }

    /// Total length in seconds.
    pub fn length(&self) -> f32 {
        self.attack + self.hold + self.decay + self.sustain_time + self.release
    }

    /// Level in 0..=1 at time `t` seconds from the layer start.
    #[inline]
    pub fn level(&self, t: f32) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        let mut t = t;
        if t < self.attack {
            return t / self.attack;
        }
        t -= self.attack;
        if t < self.hold {
            return 1.0;
        }
        t -= self.hold;
        if t < self.decay {
            let u = t / self.decay;
            return 1.0 + (self.sustain - 1.0) * u;
        }
        t -= self.decay;
        if t < self.sustain_time {
            return self.sustain;
        }
        t -= self.sustain_time;
        if t < self.release {
            let u = t / self.release;
            return self.sustain * (1.0 - u);
        }
        0.0
    }
}

/// Apply a curve to a 0..=1 progress value, for the Hz-domain curves.
#[inline]
fn shape(curve: Curve, u: f32) -> f32 {
    match curve {
        Curve::Linear | Curve::Exponential => u,
        Curve::EaseIn => u * u,
        Curve::EaseOut => 1.0 - (1.0 - u) * (1.0 - u),
    }
}

/// A value sweeping from `start` to `end` over `time` seconds.
#[derive(Debug, Clone)]
pub struct Sweep {
    start: f32,
    end: f32,
    time: f32,
    curve: Curve,
    /// `log2(end / start)`, precomputed for the exponential curve.
    log_ratio: f32,
}

impl Sweep {
    /// A sweep. When `end` is `None` the value is constant.
    pub fn new(start: f32, end: Option<f32>, time: f32, curve: Curve) -> Self {
        let end = end.unwrap_or(start);
        Self {
            start,
            end,
            time: time.max(1e-6),
            curve,
            log_ratio: math::log2(end / start),
        }
    }

    /// Value at time `t` seconds.
    #[inline]
    pub fn at(&self, t: f32) -> f32 {
        let u = (t / self.time).clamp(0.0, 1.0);
        match self.curve {
            Curve::Exponential => self.start * math::exp2(u * self.log_ratio),
            c => self.start + (self.end - self.start) * shape(c, u),
        }
    }
}

/// Vibrato parameters.
#[derive(Debug, Clone)]
pub struct VibratoShape {
    /// Rate in Hz.
    pub rate: f32,
    /// Depth in semitones.
    pub depth: f32,
    /// Start delay in seconds.
    pub delay: f32,
}

/// Arpeggio parameters.
#[derive(Debug, Clone)]
pub struct ArpShape {
    /// Offsets in semitones.
    pub steps: Vec<f32>,
    /// Step length in seconds.
    pub step_time: f32,
    /// Loop or hold last.
    pub repeat: bool,
}

/// Complete pitch curve for a layer.
#[derive(Debug, Clone)]
pub struct PitchCurve {
    /// Base sweep in Hz.
    pub sweep: Sweep,
    /// Optional vibrato.
    pub vibrato: Option<VibratoShape>,
    /// Optional arpeggio.
    pub arpeggio: Option<ArpShape>,
    /// Upper clamp (just below Nyquist).
    pub max_hz: f32,
}

impl PitchCurve {
    /// Frequency in Hz at time `t` seconds.
    #[inline]
    pub fn at(&self, t: f32) -> f32 {
        let mut semis = 0.0f32;
        if let Some(a) = &self.arpeggio {
            let idx = (t / a.step_time).floor() as usize;
            let n = a.steps.len();
            let i = if a.repeat { idx % n } else { idx.min(n - 1) };
            semis += a.steps[i];
        }
        if let Some(v) = &self.vibrato {
            let tv = t - v.delay;
            if tv > 0.0 {
                semis += v.depth * math::sin_turns(v.rate * tv);
            }
        }
        let hz = self.sweep.at(t);
        let hz = if semis != 0.0 {
            hz * math::semitones_to_ratio(semis)
        } else {
            hz
        };
        hz.clamp(0.0, self.max_hz)
    }
}

/// A phase-accumulating oscillator.
#[derive(Debug, Clone)]
pub struct Oscillator {
    source: Source,
    duty: f32,
    /// Level of the high part of the pulse.
    pulse_high: f32,
    /// Level of the low part of the pulse (negative).
    pulse_low: f32,
    phase: f32,
    rng: Rng,
}

impl Oscillator {
    /// New oscillator at phase 0.
    pub fn new(source: Source, duty: f32, rng: Rng) -> Self {
        // Pulse levels chosen so the wave has no DC offset and a peak of 1:
        // a 50% duty gives +1/-1, a 25% duty gives +1/-0.333.
        let duty = duty.clamp(0.01, 0.99);
        let widest = duty.max(1.0 - duty);
        Self {
            source,
            duty,
            pulse_high: (1.0 - duty) / widest,
            pulse_low: -duty / widest,
            phase: 0.0,
            rng,
        }
    }

    /// Next sample for a phase increment of `hz / sample_rate` (in turns).
    #[inline]
    pub fn next(&mut self, increment: f32) -> f32 {
        let p = self.phase;
        let out = match self.source {
            Source::Sine => math::sin_turns(p),
            Source::Square => {
                if p < self.duty {
                    self.pulse_high
                } else {
                    self.pulse_low
                }
            }
            Source::Saw => 2.0 * p - 1.0,
            Source::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Source::Noise => self.rng.bipolar(),
        };
        let mut np = p + increment;
        if np >= 1.0 {
            np -= np.floor();
        }
        self.phase = np;
        out
    }
}

/// Audio EQ Cookbook biquad, transposed direct form II.
#[derive(Debug, Clone)]
pub struct Biquad {
    kind: FilterKind,
    q: f32,
    sample_rate: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// A filter of `kind` with resonance `q`.
    pub fn new(kind: FilterKind, q: f32, sample_rate: f32) -> Self {
        let mut f = Self {
            kind,
            q,
            sample_rate,
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        };
        f.set_cutoff(1000.0);
        f
    }

    /// Recompute coefficients for a cutoff in Hz.
    pub fn set_cutoff(&mut self, hz: f32) {
        let hz = hz.clamp(10.0, self.sample_rate * 0.45);
        let turns = hz / self.sample_rate;
        let sin = math::sin_turns(turns);
        let cos = math::cos_turns(turns);
        let alpha = sin / (2.0 * self.q);
        let (b0, b1, b2) = match self.kind {
            FilterKind::Lowpass => ((1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5),
            FilterKind::Highpass => ((1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5),
            FilterKind::Bandpass => (alpha, 0.0, -alpha),
        };
        let a0 = 1.0 + alpha;
        let inv = 1.0 / a0;
        self.b0 = b0 * inv;
        self.b1 = b1 * inv;
        self.b2 = b2 * inv;
        self.a1 = -2.0 * cos * inv;
        self.a2 = (1.0 - alpha) * inv;
    }

    /// Filter one sample.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_shape() {
        let e = EnvShape::new(
            &Envelope {
                attack: 0.1,
                hold: 0.1,
                decay: 0.1,
                sustain: 0.5,
                sustain_time: 0.1,
                release: 0.1,
            },
            1.0,
        );
        assert_eq!(e.level(0.0), 0.0);
        assert!((e.level(0.05) - 0.5).abs() < 1e-6);
        assert_eq!(e.level(0.15), 1.0);
        assert!((e.level(0.25) - 0.75).abs() < 1e-6);
        assert!((e.level(0.35) - 0.5).abs() < 1e-6);
        assert!((e.level(0.45) - 0.25).abs() < 1e-6);
        assert_eq!(e.level(0.6), 0.0);
    }

    #[test]
    fn exponential_sweep_hits_octave_midpoint() {
        let s = Sweep::new(100.0, Some(400.0), 1.0, Curve::Exponential);
        assert!((s.at(0.5) - 200.0).abs() < 0.05);
        assert!((s.at(2.0) - 400.0).abs() < 0.05);
        let l = Sweep::new(100.0, Some(400.0), 1.0, Curve::Linear);
        assert!((l.at(0.5) - 250.0).abs() < 1e-3);
    }

    #[test]
    fn lowpass_attenuates_high_frequency() {
        let sr = 44100.0;
        let mut f = Biquad::new(FilterKind::Lowpass, 0.707, sr);
        f.set_cutoff(500.0);
        let mut peak = 0.0f32;
        for i in 0..4410 {
            let x = math::sin_turns(8000.0 * i as f32 / sr);
            let y = f.process(x);
            if i > 2000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 0.01, "peak {peak}");
    }
}
