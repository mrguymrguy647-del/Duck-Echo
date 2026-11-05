//! Effect processors. Each one owns its state, allocates only in `new`, and
//! processes a block in place.

use crate::math;
use duck_echo_core::{Bitcrush, Distortion, DistortionKind, Echo, Effect, Reverb};

/// Blend dry and wet by `mix` (0 = dry, 1 = wet).
#[inline]
fn blend(dry: f32, wet: f32, mix: f32) -> f32 {
    dry + (wet - dry) * mix
}

/// A running effect.
#[derive(Debug, Clone)]
pub enum EffectState {
    /// Delay line with feedback.
    Echo(EchoState),
    /// Comb/allpass reverb.
    Reverb(ReverbState),
    /// Waveshaper.
    Distortion(DistortionState),
    /// Quantizer and sample-hold.
    Bitcrush(BitcrushState),
}

impl EffectState {
    /// Build the state for a recipe effect.
    pub fn new(effect: &Effect, sample_rate: f32) -> Self {
        match effect {
            Effect::Echo(e) => EffectState::Echo(EchoState::new(e, sample_rate)),
            Effect::Reverb(r) => EffectState::Reverb(ReverbState::new(r, sample_rate)),
            Effect::Distortion(d) => EffectState::Distortion(DistortionState::new(d)),
            Effect::Bitcrush(b) => EffectState::Bitcrush(BitcrushState::new(b, sample_rate)),
        }
    }

    /// Process a block in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        match self {
            EffectState::Echo(s) => s.process(buf),
            EffectState::Reverb(s) => s.process(buf),
            EffectState::Distortion(s) => s.process(buf),
            EffectState::Bitcrush(s) => s.process(buf),
        }
    }
}

/// Echo state.
#[derive(Debug, Clone)]
pub struct EchoState {
    line: Vec<f32>,
    pos: usize,
    feedback: f32,
    mix: f32,
}

impl EchoState {
    fn new(e: &Echo, sample_rate: f32) -> Self {
        let len = ((e.delay * sample_rate).round() as usize).max(1);
        Self {
            line: vec![0.0; len],
            pos: 0,
            feedback: e.feedback,
            mix: e.mix,
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let len = self.line.len();
        for x in buf.iter_mut() {
            let delayed = self.line[self.pos];
            self.line[self.pos] = *x + delayed * self.feedback;
            self.pos += 1;
            if self.pos == len {
                self.pos = 0;
            }
            *x = blend(*x, delayed, self.mix);
        }
    }
}

/// Freeverb-style reverb: 8 damped parallel combs into 4 series allpasses.
#[derive(Debug, Clone)]
pub struct ReverbState {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    mix: f32,
}

#[derive(Debug, Clone)]
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
    /// `sqrt(1 - feedback²)`: unit broadband power gain, so long decays
    /// do not get louder than short ones.
    gain: f32,
    damp: f32,
    store: f32,
}

#[derive(Debug, Clone)]
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

/// Gain on the normalized comb sum, tuned so a fully wet reverb sits near
/// the loudness of its dry input for a one second decay.
const WET_SCALE: f32 = 0.5;

const COMB_LENGTHS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LENGTHS: [usize; 4] = [556, 441, 341, 225];

impl ReverbState {
    fn new(r: &Reverb, sample_rate: f32) -> Self {
        let sr_scale = sample_rate / 44_100.0;
        let size_scale = 0.5 + r.size; // 0.5 .. 1.5
        let combs = COMB_LENGTHS
            .iter()
            .map(|&base| {
                let len = ((base as f32 * sr_scale * size_scale).round() as usize).max(2);
                // Gain per pass so the tail falls 60 dB in `decay` seconds.
                let seconds = len as f32 / sample_rate;
                let feedback = math::exp2(-9.965_784 * seconds / r.decay).min(0.999);
                Comb {
                    buf: vec![0.0; len],
                    pos: 0,
                    feedback,
                    gain: (1.0 - feedback * feedback).sqrt(),
                    damp: r.damping * 0.8,
                    store: 0.0,
                }
            })
            .collect();
        let allpasses = ALLPASS_LENGTHS
            .iter()
            .map(|&base| Allpass {
                buf: vec![0.0; ((base as f32 * sr_scale).round() as usize).max(2)],
                pos: 0,
            })
            .collect();
        Self {
            combs,
            allpasses,
            mix: r.mix,
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let input = *x;
            let mut wet = 0.0f32;
            for c in &mut self.combs {
                let out = c.buf[c.pos];
                c.store = out + (c.store - out) * c.damp;
                c.buf[c.pos] = input + c.store * c.feedback;
                c.pos += 1;
                if c.pos == c.buf.len() {
                    c.pos = 0;
                }
                wet += out * c.gain;
            }
            wet *= WET_SCALE / 8.0;
            for a in &mut self.allpasses {
                let bufout = a.buf[a.pos];
                let out = bufout - wet;
                a.buf[a.pos] = wet + bufout * 0.5;
                a.pos += 1;
                if a.pos == a.buf.len() {
                    a.pos = 0;
                }
                wet = out;
            }
            *x = blend(input, wet, self.mix);
        }
    }
}

/// Distortion state.
#[derive(Debug, Clone)]
pub struct DistortionState {
    drive: f32,
    kind: DistortionKind,
    mix: f32,
}

impl DistortionState {
    fn new(d: &Distortion) -> Self {
        Self {
            drive: d.drive,
            kind: d.kind,
            mix: d.mix,
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let v = *x * self.drive;
            let wet = match self.kind {
                DistortionKind::Soft => {
                    // Cubic soft clip: smooth up to |v| = 1.5, flat beyond.
                    let y = v.clamp(-1.5, 1.5);
                    y - y * y * y * (4.0 / 27.0)
                }
                DistortionKind::Hard => v.clamp(-1.0, 1.0),
                DistortionKind::Fold => {
                    let t = (v + 1.0) * 0.25;
                    let frac = t - t.floor();
                    1.0 - 4.0 * (frac - 0.5).abs()
                }
            };
            *x = blend(*x, wet, self.mix);
        }
    }
}

/// Bitcrush state.
#[derive(Debug, Clone)]
pub struct BitcrushState {
    levels: f32,
    hold_increment: Option<f32>,
    hold_phase: f32,
    held: f32,
    mix: f32,
}

impl BitcrushState {
    fn new(b: &Bitcrush, sample_rate: f32) -> Self {
        Self {
            levels: (1u32 << (b.bits.clamp(1, 16) - 1)) as f32,
            hold_increment: b.rate.map(|r| r / sample_rate),
            hold_phase: 1.0,
            held: 0.0,
            mix: b.mix,
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let q = (*x * self.levels + 0.5).floor() / self.levels;
            let wet = match self.hold_increment {
                None => q,
                Some(inc) => {
                    if self.hold_phase >= 1.0 {
                        self.hold_phase -= 1.0;
                        self.held = q;
                    }
                    self.hold_phase += inc;
                    self.held
                }
            };
            *x = blend(*x, wet, self.mix);
        }
    }
}
