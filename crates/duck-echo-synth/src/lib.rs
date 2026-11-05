//! Duck Echo synthesis: turns a validated [`Recipe`] into samples.
//!
//! The API is split in two so the same code can later drive realtime
//! playback:
//!
//! * [`prepare`] does all validation, planning and allocation and returns a
//!   [`Voice`].
//! * [`Voice::process`] fills a block of samples. It never allocates, never
//!   locks and never touches the recipe again, so it is safe on an audio
//!   thread.
//!
//! [`render`] is the offline convenience that runs both and applies the
//! offline-only master `normalize` step.
//!
//! Determinism: all math on the audio path goes through [`math`], which
//! avoids platform libm, and all randomness comes from the recipe `seed`.

pub mod dsp;
pub mod effects;
pub mod math;

use duck_echo_audio::AudioBuffer;
use duck_echo_core::validate::chain_tail_seconds;
use duck_echo_core::{Layer, Recipe, Report, Source};

use dsp::{ArpShape, Biquad, EnvShape, Oscillator, PitchCurve, Sweep, VibratoShape};
use effects::EffectState;
use math::Rng;

/// Samples between filter coefficient updates. Small enough that sweeps are
/// smooth, large enough to be cheap.
const CONTROL_INTERVAL: usize = 16;

/// Block size used by [`render`].
pub const RENDER_BLOCK: usize = 512;

/// Stream tags for forking the seed. Changing these changes every render.
const STREAM_VARIATION: u64 = 0x5641_5249;
const STREAM_LAYER: u64 = 0x4C41_5945;

/// A prepared, playable sound.
#[derive(Debug, Clone)]
pub struct Voice {
    sample_rate: u32,
    channels: usize,
    total_frames: usize,
    position: usize,
    gain: f32,
    layers: Vec<LayerVoice>,
    /// Master effect chain, one independent copy per channel.
    master_effects: Vec<Vec<EffectState>>,
    /// One layer's mono signal for the current block.
    scratch: Vec<f32>,
    /// Planar per-channel mix for the current block (stereo only).
    mix: Vec<f32>,
}

#[derive(Debug, Clone)]
struct LayerVoice {
    start_frame: usize,
    end_frame: usize,
    env_frames: usize,
    local: usize,
    volume: f32,
    inv_sample_rate: f32,
    env: EnvShape,
    pitch: PitchCurve,
    osc: Oscillator,
    filter: Option<(Biquad, Sweep)>,
    control_counter: usize,
    effects: Vec<EffectState>,
    /// Equal-power left and right gains from `pan`.
    pan_gains: [f32; 2],
}

/// Validate, plan and allocate. Errors are the validation report.
pub fn prepare(recipe: &Recipe) -> Result<Voice, Report> {
    let report = duck_echo_core::validate(recipe);
    if !report.is_ok() {
        return Err(report);
    }
    Ok(prepare_unchecked(recipe))
}

/// Plan and allocate without validating. Only for recipes that already
/// passed [`duck_echo_core::validate`]; out-of-range values may misbehave.
pub fn prepare_unchecked(recipe: &Recipe) -> Voice {
    let sr = recipe.sample_rate as f32;
    let root = Rng::new(recipe.seed);
    let mut variation_rng = root.fork(STREAM_VARIATION);

    let mut layers = Vec::with_capacity(recipe.layers.len());
    let mut last_end = 0usize;
    for (i, layer) in recipe.layers.iter().enumerate() {
        // Always draw so a layer's jitter does not depend on `variation`
        // being non-zero for earlier layers.
        let pitch_jitter = variation_rng.bipolar();
        let time_jitter = variation_rng.bipolar();
        let pitch_mult = math::semitones_to_ratio(recipe.variation * 3.0 * pitch_jitter);
        let time_scale = 1.0 + recipe.variation * 0.3 * time_jitter;

        let lv = LayerVoice::new(
            layer,
            sr,
            pitch_mult,
            time_scale,
            root.fork(STREAM_LAYER + i as u64),
        );
        last_end = last_end.max(lv.end_frame);
        layers.push(lv);
    }

    let master_tail = chain_tail_seconds(&recipe.effects);
    let total_frames = match recipe.duration {
        Some(d) => ((d * sr).round() as usize).max(1),
        None => (last_end + (master_tail * sr).round() as usize).max(1),
    };

    let channels = recipe.channels.clamp(1, 2) as usize;
    Voice {
        sample_rate: recipe.sample_rate,
        channels,
        total_frames,
        position: 0,
        gain: recipe.master.gain,
        layers,
        master_effects: (0..channels)
            .map(|_| {
                recipe
                    .effects
                    .iter()
                    .map(|e| EffectState::new(e, sr))
                    .collect()
            })
            .collect(),
        scratch: vec![0.0; RENDER_BLOCK],
        mix: if channels > 1 {
            vec![0.0; RENDER_BLOCK * channels]
        } else {
            Vec::new()
        },
    }
}

impl LayerVoice {
    fn new(layer: &Layer, sr: f32, pitch_mult: f32, time_scale: f32, rng: Rng) -> Self {
        let env = EnvShape::new(&layer.envelope, time_scale);
        let env_len = env.length().min(30.0);
        let tail = chain_tail_seconds(&layer.effects);
        let start_frame = (layer.offset * sr).round() as usize;
        let env_frames = (env_len * sr).round() as usize;
        let end_frame = start_frame + env_frames + (tail * sr).round() as usize;

        let p = &layer.pitch;
        let sweep = Sweep::new(
            p.start * pitch_mult,
            p.end.map(|e| e * pitch_mult),
            p.sweep_time.unwrap_or(env_len),
            p.curve,
        );
        let pitch = PitchCurve {
            sweep,
            vibrato: p.vibrato.as_ref().map(|v| VibratoShape {
                rate: v.rate,
                depth: v.depth,
                delay: v.delay,
            }),
            arpeggio: p
                .arpeggio
                .as_ref()
                .filter(|a| !a.steps.is_empty())
                .map(|a| ArpShape {
                    steps: a.steps.clone(),
                    step_time: a.step_time.max(1e-4),
                    repeat: a.repeat,
                }),
            max_hz: sr * 0.45,
        };

        let filter = layer.filter.as_ref().map(|f| {
            let bq = Biquad::new(f.kind, f.resonance, sr);
            let sweep = Sweep::new(
                f.cutoff,
                f.cutoff_end,
                f.sweep_time.unwrap_or(env_len),
                f.curve,
            );
            (bq, sweep)
        });

        Self {
            start_frame,
            end_frame,
            env_frames,
            local: 0,
            volume: layer.volume,
            inv_sample_rate: 1.0 / sr,
            env,
            pitch,
            osc: Oscillator::new(layer.source, layer.duty, rng),
            filter,
            control_counter: 0,
            effects: layer
                .effects
                .iter()
                .map(|e| EffectState::new(e, sr))
                .collect(),
            pan_gains: pan_gains(layer.pan),
        }
    }

    /// Add this layer's contribution for global frames
    /// `block_start .. block_start + out.len()` into the mono `out`.
    fn process(&mut self, block_start: usize, out: &mut [f32], scratch: &mut [f32]) {
        if let Some((offset, n)) = self.render(block_start, out.len(), scratch) {
            for (o, s) in out[offset..offset + n].iter_mut().zip(scratch[..n].iter()) {
                *o += *s;
            }
        }
    }

    /// Stereo version of [`Self::process`]: the layer's mono signal is
    /// placed with its pan gains.
    fn process_stereo(
        &mut self,
        block_start: usize,
        left: &mut [f32],
        right: &mut [f32],
        scratch: &mut [f32],
    ) {
        if let Some((offset, n)) = self.render(block_start, left.len(), scratch) {
            let [gl, gr] = self.pan_gains;
            let range = offset..offset + n;
            for ((l, r), s) in left[range.clone()]
                .iter_mut()
                .zip(right[range].iter_mut())
                .zip(scratch[..n].iter())
            {
                *l += *s * gl;
                *r += *s * gr;
            }
        }
    }

    /// Render this layer's mono signal (oscillator, envelope, filter, layer
    /// effects) for the part of the block it covers into `scratch[..n]`.
    /// Returns the offset into the block and `n`, or `None` when the layer
    /// is silent for the whole block.
    fn render(
        &mut self,
        block_start: usize,
        block_len: usize,
        scratch: &mut [f32],
    ) -> Option<(usize, usize)> {
        let block_end = block_start + block_len;
        let begin = self.start_frame.max(block_start);
        let end = self.end_frame.min(block_end);
        if begin >= end {
            return None;
        }
        let n = end - begin;
        let buf = &mut scratch[..n];

        // 1. Oscillator shaped by envelope and volume.
        for s in buf.iter_mut() {
            let local = self.local;
            if local < self.env_frames {
                let t = local as f32 * self.inv_sample_rate;
                let hz = self.pitch.at(t);
                let level = self.env.level(t) * self.volume;
                *s = self.osc.next(hz * self.inv_sample_rate) * level;
            } else {
                *s = 0.0;
            }
            // 2. Filter with control-rate cutoff updates.
            if let Some((bq, sweep)) = &mut self.filter {
                if self.control_counter == 0 {
                    bq.set_cutoff(sweep.at(local as f32 * self.inv_sample_rate));
                }
                self.control_counter += 1;
                if self.control_counter == CONTROL_INTERVAL {
                    self.control_counter = 0;
                }
                *s = bq.process(*s);
            }
            self.local += 1;
        }

        // 3. Layer effects, on the mono layer signal (before panning).
        for fx in &mut self.effects {
            fx.process(buf);
        }

        Some((begin - block_start, n))
    }
}

/// Equal-power pan law: `pan` -1..=1 maps to a quarter turn, so the centre
/// gives both channels `cos(π/4) ≈ 0.707` (-3 dB) and hard left or right
/// gives one channel full level and the other silence.
///
/// Both gains come from the same `sin` with mirrored arguments, so a centred
/// layer is bit-identical in both channels and `pan = p` mirrors `-p`.
fn pan_gains(pan: f32) -> [f32; 2] {
    let pan = pan.clamp(-1.0, 1.0);
    [
        math::sin_turns((1.0 - pan) * 0.125),
        math::sin_turns((1.0 + pan) * 0.125),
    ]
}

impl Voice {
    /// Output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Output channels (1 or 2).
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Total length in frames.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Total length in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.total_frames as f64 / self.sample_rate as f64
    }

    /// Frames rendered so far.
    pub fn position(&self) -> usize {
        self.position
    }

    /// True once every frame has been produced.
    pub fn finished(&self) -> bool {
        self.position >= self.total_frames
    }

    /// Fill `out` with the next frames, interleaved when stereo, and return
    /// how many frames were written. Writes zeros past the end (and into a
    /// trailing partial frame, if `out.len()` is not a multiple of
    /// [`Self::channels`]). Allocation-free and lock-free; any length works
    /// (it is processed in pieces of at most [`RENDER_BLOCK`] frames).
    /// Output is not clipped; the caller clips or normalizes.
    pub fn process(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        for chunk in out.chunks_mut(RENDER_BLOCK * self.channels) {
            written += if self.channels == 1 {
                self.process_mono(chunk)
            } else {
                self.process_stereo(chunk)
            };
        }
        written
    }

    fn process_stereo(&mut self, out: &mut [f32]) -> usize {
        out.iter_mut().for_each(|s| *s = 0.0);
        if self.finished() {
            return 0;
        }
        let n = (out.len() / 2).min(self.total_frames - self.position);
        let (left, right) = self.mix.split_at_mut(RENDER_BLOCK);
        let (left, right) = (&mut left[..n], &mut right[..n]);
        left.iter_mut().for_each(|s| *s = 0.0);
        right.iter_mut().for_each(|s| *s = 0.0);
        for layer in &mut self.layers {
            layer.process_stereo(self.position, left, right, &mut self.scratch);
        }
        for (buf, chain) in [&mut *left, &mut *right]
            .into_iter()
            .zip(self.master_effects.iter_mut())
        {
            for fx in chain {
                fx.process(buf);
            }
        }
        for (i, frame) in out[..n * 2].chunks_exact_mut(2).enumerate() {
            frame[0] = left[i] * self.gain;
            frame[1] = right[i] * self.gain;
        }
        self.position += n;
        n
    }

    /// The mono path, kept exactly as in Phase 0 so mono renders stay
    /// byte-identical.
    fn process_mono(&mut self, out: &mut [f32]) -> usize {
        out.iter_mut().for_each(|s| *s = 0.0);
        if self.finished() {
            return 0;
        }
        let n = out.len().min(self.total_frames - self.position);
        let block = &mut out[..n];
        for layer in &mut self.layers {
            layer.process(self.position, block, &mut self.scratch);
        }
        for fx in &mut self.master_effects[0] {
            fx.process(block);
        }
        if self.gain != 1.0 {
            for s in block.iter_mut() {
                *s *= self.gain;
            }
        }
        self.position += n;
        n
    }
}

/// Render a recipe offline to a buffer (interleaved when stereo),
/// applying `master.normalize`, clipping to -1..=1 and
/// `master.trim_silence`. Errors are the validation report.
pub fn render(recipe: &Recipe) -> Result<AudioBuffer, Report> {
    let mut voice = prepare(recipe)?;
    let channels = voice.channels();
    let mut samples = vec![0.0f32; voice.total_frames() * channels];
    let mut pos = 0;
    while pos < samples.len() {
        let end = (pos + RENDER_BLOCK * channels).min(samples.len());
        let n = voice.process(&mut samples[pos..end]);
        if n == 0 {
            break;
        }
        pos += n * channels;
    }

    if recipe.master.normalize {
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > 0.0 {
            let target = math::db_to_gain(recipe.master.normalize_peak_db);
            let g = target / peak;
            for s in &mut samples {
                *s *= g;
            }
        }
    }
    for s in &mut samples {
        *s = s.clamp(-1.0, 1.0);
    }
    if recipe.master.trim_silence && recipe.duration.is_none() {
        trim_trailing_silence(&mut samples, channels);
    }

    Ok(AudioBuffer {
        sample_rate: recipe.sample_rate,
        channels: channels as u16,
        samples,
    })
}

/// Drop trailing frames whose samples are all at or below
/// [`duck_echo_core::TRIM_THRESHOLD`], keeping at least one frame. The cut
/// leaves a step of at most -60 dBFS at the end, which is inaudible.
fn trim_trailing_silence(samples: &mut Vec<f32>, channels: usize) {
    let frames = samples.len() / channels;
    let last = (0..frames).rev().find(|&f| {
        samples[f * channels..(f + 1) * channels]
            .iter()
            .any(|s| s.abs() > duck_echo_core::TRIM_THRESHOLD)
    });
    let keep = last.map(|f| f + 1).unwrap_or(1).min(frames);
    samples.truncate(keep * channels);
}

/// Whether a source uses the seed. Handy for tools that want to know if a
/// seed change can do anything when `variation` is 0.
pub fn source_uses_seed(source: Source) -> bool {
    matches!(source, Source::Noise)
}
