//! Per-layer building blocks: envelope, pitch curve, oscillator, filter.
//! Everything here is allocation-free once constructed.

use crate::math::{self, Rng};
use duck_echo_core::{Curve, Envelope, FilterKind, Lfo, LfoShape, LfoTarget, Source};

/// Shortest release of a note in a song, in seconds. A note whose
/// envelope has `release = 0` would stop dead when let go, which clicks.
pub const MIN_NOTE_RELEASE: f32 = 0.005;

/// Envelope with times already converted to seconds (and jittered).
#[derive(Debug, Clone)]
pub struct EnvShape {
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    sustain_time: f32,
    release: f32,
    /// For a note in a song: when the note is let go, and the level the
    /// envelope had reached by then. `None` for a plain sound effect.
    gate: Option<(f32, f32)>,
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
            gate: None,
        }
    }

    /// Build the envelope of a note that is held for `gate` seconds, like
    /// a key on a keyboard:
    ///
    /// * While held, the envelope plays as written, and stays at the
    ///   sustain level for as long as the note lasts (the recipe's own
    ///   `sustain_time` is not used).
    /// * When let go, it falls to silence over `release`, starting from
    ///   whatever level it had reached.
    pub fn gated(e: &Envelope, time_scale: f32, gate: f32) -> Self {
        let mut shape = Self::new(e, time_scale);
        shape.sustain_time = f32::INFINITY;
        shape.release = shape.release.max(MIN_NOTE_RELEASE);
        let gate = gate.max(0.0);
        shape.gate = Some((gate, shape.level(gate)));
        shape
    }

    /// Total length in seconds.
    pub fn length(&self) -> f32 {
        match self.gate {
            Some((gate, _)) => {
                // A sound with no sustain may fade out by itself before
                // the note is let go. Then it simply ends there.
                let fades_alone = self.attack + self.hold + self.decay;
                if self.sustain <= 0.0 && fades_alone <= gate {
                    fades_alone
                } else {
                    gate + self.release
                }
            }
            None => self.attack + self.hold + self.decay + self.sustain_time + self.release,
        }
    }

    /// Level in 0..=1 at time `t` seconds from the layer start.
    #[inline]
    pub fn level(&self, t: f32) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        if let Some((gate, level)) = self.gate {
            if t >= gate {
                let u = (t - gate) / self.release;
                return if u < 1.0 { level * (1.0 - u) } else { 0.0 };
            }
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

/// A low-frequency oscillator, ready to run: a slow wave that moves one
/// setting of a layer. It keeps no state; the wave is worked out from the
/// time, so it is the same however the sound is cut into blocks.
#[derive(Debug, Clone)]
pub struct LfoShaper {
    target: LfoTarget,
    shape: LfoShape,
    rate: f32,
    depth: f32,
    delay: f32,
}

impl LfoShaper {
    /// Build from a recipe LFO.
    pub fn new(lfo: &Lfo) -> Self {
        Self {
            target: lfo.target,
            shape: lfo.shape,
            rate: lfo.rate,
            depth: lfo.depth,
            delay: lfo.delay,
        }
    }

    /// What the wave moves.
    pub fn target(&self) -> LfoTarget {
        self.target
    }

    /// The wave, in -1..=1, `turns` cycles after it started. Every shape
    /// starts at 0 and goes up first.
    #[inline]
    fn wave(&self, turns: f32) -> f32 {
        let p = turns - turns.floor();
        match self.shape {
            LfoShape::Sine => math::sin_turns(p),
            LfoShape::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            LfoShape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    /// The gain for target `volume` at time `t`: 1 before `delay`, then
    /// swinging between 1 and `1 - depth`, starting at 1.
    #[inline]
    pub fn gain(&self, t: f32) -> f32 {
        if t < self.delay {
            return 1.0;
        }
        // A quarter turn ahead, so the wave starts at its top (gain 1)
        // and the volume does not jump when the LFO begins.
        let w = self.wave((t - self.delay) * self.rate + 0.25);
        1.0 - self.depth * 0.5 * (1.0 - w)
    }

    /// The wave times `depth` at time `t`, for targets `cutoff` (octaves)
    /// and `duty` (an amount to add). 0 before `delay`, and it starts
    /// from 0, so nothing jumps when the LFO begins.
    #[inline]
    pub fn offset(&self, t: f32) -> f32 {
        if t < self.delay {
            return 0.0;
        }
        self.depth * self.wave((t - self.delay) * self.rate)
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
    /// Smooth the jumps in square and saw waves (`aliasing = "clean"`).
    clean: bool,
    duty: f32,
    /// Level of the high part of the pulse.
    pulse_high: f32,
    /// Level of the low part of the pulse (negative).
    pulse_low: f32,
    phase: f32,
    rng: Rng,
    /// Pitched noise: the random value being held, or `None` for white
    /// noise (a new value every sample).
    held: Option<f32>,
}

impl Oscillator {
    /// New oscillator at phase 0. `clean` turns on the smoothing of the
    /// jumps in square and saw waves (see [`blep_near`]). `pitched_noise`
    /// makes a noise source hold each random value for one cycle of its
    /// pitch instead of drawing a new one every sample.
    pub fn new(source: Source, duty: f32, clean: bool, pitched_noise: bool, mut rng: Rng) -> Self {
        // White noise must draw nothing here, so its samples stay exactly
        // as they were before pitched noise existed.
        let held = (pitched_noise && source == Source::Noise).then(|| rng.bipolar());
        let mut osc = Self {
            source,
            clean,
            duty: 0.5,
            pulse_high: 1.0,
            pulse_low: -1.0,
            phase: 0.0,
            rng,
            held,
        };
        osc.set_duty(duty);
        osc
    }

    /// Set the pulse width of a square wave (the part of each cycle that
    /// is high), kept inside 0.01..=0.99.
    #[inline]
    pub fn set_duty(&mut self, duty: f32) {
        // Pulse levels chosen so the wave has no DC offset and a peak of 1:
        // a 50% duty gives +1/-1, a 25% duty gives +1/-0.333.
        let duty = duty.clamp(0.01, 0.99);
        let widest = duty.max(1.0 - duty);
        self.duty = duty;
        self.pulse_high = (1.0 - duty) / widest;
        self.pulse_low = -duty / widest;
    }

    /// Next sample for a phase increment of `hz / sample_rate` (in turns).
    #[inline]
    pub fn next(&mut self, increment: f32) -> f32 {
        let p = self.phase;
        let mut out = match self.source {
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
            Source::Noise => match self.held {
                Some(value) => value,
                None => self.rng.bipolar(),
            },
        };
        // Clean mode: add a small correction next to each jump in the wave.
        // The raw wave above is untouched, so retro mode gives exactly the
        // same numbers as before this option existed.
        if self.clean && increment > 0.0 {
            match self.source {
                // The saw drops by 2 (from +1 to -1) where its phase wraps.
                Source::Saw => out -= 2.0 * blep_near(p, 0.0, increment),
                // The pulse jumps up where the phase wraps and down at `duty`.
                Source::Square => {
                    let jump = self.pulse_high - self.pulse_low;
                    out +=
                        jump * (blep_near(p, 0.0, increment) - blep_near(p, self.duty, increment));
                }
                _ => {}
            }
        }
        let mut np = p + increment;
        if np >= 1.0 {
            np -= np.floor();
            // Pitched noise: one new random value per cycle.
            if self.held.is_some() {
                self.held = Some(self.rng.bipolar());
            }
        }
        self.phase = np;
        out
    }
}

/// The correction that turns a sudden jump of height 1 into a smooth one
/// ("BLEP residual"), for a sample `u` samples after the jump (negative
/// `u`: the jump is still ahead).
///
/// A raw jump has energy at every frequency, and everything above half the
/// sample rate folds back down as harsh extra tones (aliasing). Here the
/// jump is smoothed by a cubic B-spline curve, which spreads it over 2
/// samples on each side; its spectrum falls off fast enough that a 2 kHz
/// saw's folded tones are more than 40 dB under its fundamental (tested in
/// `duck-echo-tools/tests/clean_oscillators.rs`). The price is a gentler
/// top end: about -3 dB at 10 kHz and -7 dB at 15 kHz at 44.1 kHz.
///
/// The value is "smooth step minus raw step", so it is zero more than 2
/// samples from the jump, and only `+ - * /` are used, for determinism.
#[inline]
pub fn blep_residual(u: f32) -> f32 {
    if !(-2.0..2.0).contains(&u) {
        0.0
    } else if u < -1.0 {
        let v = u + 2.0;
        let v2 = v * v;
        v2 * v2 * (1.0 / 24.0)
    } else if u < 0.0 {
        let u2 = u * u;
        0.5 + (4.0 * u - 2.0 * u2 * u - 0.75 * u2 * u2) * (1.0 / 6.0)
    } else if u < 1.0 {
        let u2 = u * u;
        -0.5 + (4.0 * u - 2.0 * u2 * u + 0.75 * u2 * u2) * (1.0 / 6.0)
    } else {
        let v = 2.0 - u;
        let v2 = v * v;
        -(v2 * v2) * (1.0 / 24.0)
    }
}

/// The total correction at phase `p` (0..1) for a wave that jumps up by 1
/// each time its phase passes `edge`, when the phase moves `dt` per sample.
///
/// The phase distance to the jump is turned into samples by dividing by
/// `dt`. On very high notes the 4-sample reach covers more than half a
/// cycle, so the jumps one cycle before and after can be in reach too;
/// all of them are added.
#[inline]
pub fn blep_near(p: f32, edge: f32, dt: f32) -> f32 {
    // Distance from the nearest copy of the edge, in -0.5..0.5 turns.
    let mut d = p - edge;
    d -= (d + 0.5).floor();
    let reach = 2.0 * dt;
    let mut sum = 0.0;
    for k in [-1.0f32, 0.0, 1.0] {
        let dk = d + k;
        if dk > -reach && dk < reach {
            sum += blep_residual(dk / dt);
        }
    }
    sum
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
    fn blep_residual_is_a_smooth_step_minus_a_raw_step() {
        // Zero outside 2 samples on each side.
        for u in [-3.0, -2.0, 2.0, 2.5] {
            assert_eq!(blep_residual(u), 0.0, "u = {u}");
        }
        // It cancels the raw jump: +0.5 just before, -0.5 just after.
        assert!((blep_residual(-1e-6) - 0.5).abs() < 1e-5);
        assert!((blep_residual(0.0) + 0.5).abs() < 1e-6);
        // The four curve pieces meet without gaps.
        for u in [-1.0f32, 1.0] {
            let below = blep_residual(u - 1e-4);
            let above = blep_residual(u + 1e-4);
            assert!(
                (below - above).abs() < 1e-3,
                "gap at {u}: {below} vs {above}"
            );
        }
        assert!((blep_residual(-1.0) - 1.0 / 24.0).abs() < 1e-6);
        assert!((blep_residual(1.0) + 1.0 / 24.0).abs() < 1e-6);
        // Odd symmetry around the jump.
        for u in [0.3f32, 0.9, 1.4, 1.9] {
            assert!(
                (blep_residual(u) + blep_residual(-u)).abs() < 1e-6,
                "u = {u}"
            );
        }
    }

    #[test]
    fn clean_saw_has_no_jump_and_stays_in_range() {
        // A 2 kHz saw at 44.1 kHz: the raw wave jumps by 2 in one sample
        // each cycle. The clean one spreads that drop over 4 samples; the
        // biggest single step is about 60% of it (1.2, depending on where
        // the jump falls between samples) plus the ramp's own slope.
        let mut raw = Oscillator::new(Source::Saw, 0.5, false, false, Rng::new(0));
        let mut clean = Oscillator::new(Source::Saw, 0.5, true, false, Rng::new(0));
        let dt = 2000.0 / 44_100.0;
        let (mut prev_raw, mut prev_clean) = (raw.next(dt), clean.next(dt));
        let (mut big_raw, mut big_clean, mut peak) = (0.0f32, 0.0f32, 0.0f32);
        for _ in 0..4410 {
            let (r, c) = (raw.next(dt), clean.next(dt));
            big_raw = big_raw.max((r - prev_raw).abs());
            big_clean = big_clean.max((c - prev_clean).abs());
            peak = peak.max(c.abs());
            (prev_raw, prev_clean) = (r, c);
        }
        assert!(big_raw > 1.8, "raw saw should jump, largest step {big_raw}");
        assert!(big_clean < 1.3, "clean saw still jumps by {big_clean}");
        assert!(peak <= 1.0, "clean saw peak {peak}");
    }

    #[test]
    fn retro_and_clean_only_differ_on_square_and_saw() {
        for source in [Source::Sine, Source::Triangle, Source::Noise] {
            let mut a = Oscillator::new(source, 0.5, false, false, Rng::new(7));
            let mut b = Oscillator::new(source, 0.5, true, false, Rng::new(7));
            for _ in 0..1000 {
                assert_eq!(a.next(0.05), b.next(0.05), "{source:?}");
            }
        }
    }

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

    #[test]
    fn pitched_noise_holds_each_value_for_one_cycle() {
        // 441 Hz at 44.1 kHz: a new value every 100 samples.
        let mut osc = Oscillator::new(Source::Noise, 0.5, true, true, Rng::new(3));
        let samples: Vec<f32> = (0..1000).map(|_| osc.next(441.0 / 44_100.0)).collect();
        let mut changes = 0;
        for pair in samples.windows(2) {
            if pair[0] != pair[1] {
                changes += 1;
            }
        }
        assert!((9..=10).contains(&changes), "{changes} changes");
        assert!(samples.iter().all(|s| (-1.0..1.0).contains(s)));

        // White noise is exactly what it was: a new value every sample,
        // and the same values whether or not the mode exists.
        let mut white = Oscillator::new(Source::Noise, 0.5, true, false, Rng::new(3));
        let mut rng = Rng::new(3);
        for _ in 0..100 {
            assert_eq!(white.next(441.0 / 44_100.0), rng.bipolar());
        }
    }

    #[test]
    fn lfo_rate_and_depth() {
        let lfo = |target, shape, rate, depth, delay| {
            LfoShaper::new(&Lfo {
                target,
                rate,
                depth,
                shape,
                delay,
            })
        };
        // Volume: starts at full, dips to 1 - depth half a cycle later,
        // and is back after one cycle. 4 Hz: a cycle is 0.25 s.
        for shape in [LfoShape::Sine, LfoShape::Triangle] {
            let l = lfo(LfoTarget::Volume, shape, 4.0, 0.6, 0.0);
            assert!((l.gain(0.0) - 1.0).abs() < 1e-4, "{shape:?}");
            assert!((l.gain(0.125) - 0.4).abs() < 1e-4, "{shape:?}");
            assert!((l.gain(0.25) - 1.0).abs() < 1e-4, "{shape:?}");
            assert!((l.gain(0.0625) - 0.7).abs() < 1e-4, "{shape:?}");
        }
        // A square wave is at one end or the other.
        let l = lfo(LfoTarget::Volume, LfoShape::Square, 4.0, 0.6, 0.0);
        assert_eq!(l.gain(0.03), 1.0);
        assert!((l.gain(0.1) - 0.4).abs() < 1e-6);

        // Cutoff and duty: starts at 0, up to +depth a quarter cycle in,
        // down to -depth three quarters in.
        for shape in [LfoShape::Sine, LfoShape::Triangle] {
            let l = lfo(LfoTarget::Cutoff, shape, 2.0, 1.5, 0.0);
            assert!(l.offset(0.0).abs() < 1e-4);
            assert!((l.offset(0.125) - 1.5).abs() < 1e-4);
            assert!((l.offset(0.375) + 1.5).abs() < 1e-4);
            assert!(l.offset(0.5).abs() < 1e-3);
        }

        // Nothing happens before `delay`, and nothing jumps when it ends.
        let l = lfo(LfoTarget::Volume, LfoShape::Sine, 4.0, 0.6, 0.5);
        assert_eq!(l.gain(0.49), 1.0);
        assert!((l.gain(0.5) - 1.0).abs() < 1e-4);
        assert!((l.gain(0.625) - 0.4).abs() < 1e-4);
        let l = lfo(LfoTarget::Duty, LfoShape::Sine, 4.0, 0.2, 0.5);
        assert_eq!(l.offset(0.49), 0.0);
        assert!(l.offset(0.5).abs() < 1e-4);

        // Counting cycles: a 5 Hz wave crosses zero going up 5 times a
        // second.
        let l = lfo(LfoTarget::Duty, LfoShape::Sine, 5.0, 0.2, 0.0);
        let mut rising = 0;
        let mut last = l.offset(0.0005);
        for i in 1..1000 {
            let now = l.offset(i as f32 / 1000.0 + 0.0005);
            if last < 0.0 && now >= 0.0 {
                rising += 1;
            }
            last = now;
        }
        assert_eq!(rising, 4, "four more starts after the first");
    }
}
