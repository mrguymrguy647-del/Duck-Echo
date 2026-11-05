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
    total_frames: usize,
    position: usize,
    gain: f32,
    layers: Vec<LayerVoice>,
    master_effects: Vec<EffectState>,
    scratch: Vec<f32>,
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

    Voice {
        sample_rate: recipe.sample_rate,
        total_frames,
        position: 0,
        gain: recipe.master.gain,
        layers,
        master_effects: recipe
            .effects
            .iter()
            .map(|e| EffectState::new(e, sr))
            .collect(),
        scratch: vec![0.0; RENDER_BLOCK],
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
        }
    }

    /// Add this layer's contribution for global frames
    /// `block_start .. block_start + out.len()` into `out`.
    fn process(&mut self, block_start: usize, out: &mut [f32], scratch: &mut [f32]) {
        let block_end = block_start + out.len();
        let begin = self.start_frame.max(block_start);
        let end = self.end_frame.min(block_end);
        if begin >= end {
            return;
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

        // 3. Layer effects.
        for fx in &mut self.effects {
            fx.process(buf);
        }

        // 4. Mix.
        let offset = begin - block_start;
        for (o, s) in out[offset..offset + n].iter_mut().zip(buf.iter()) {
            *o += *s;
        }
    }
}

impl Voice {
    /// Output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
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

    /// Fill `out` with the next mono frames and return how many were
    /// written. Writes zeros past the end. Allocation-free; `out` may be at
    /// most [`RENDER_BLOCK`] long per call (longer slices are processed in
    /// pieces). Output is not clipped; the caller clips or normalizes.
    pub fn process(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        for chunk in out.chunks_mut(RENDER_BLOCK) {
            written += self.process_chunk(chunk);
        }
        written
    }

    fn process_chunk(&mut self, out: &mut [f32]) -> usize {
        out.iter_mut().for_each(|s| *s = 0.0);
        if self.finished() {
            return 0;
        }
        let n = out.len().min(self.total_frames - self.position);
        let block = &mut out[..n];
        for layer in &mut self.layers {
            layer.process(self.position, block, &mut self.scratch);
        }
        for fx in &mut self.master_effects {
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

/// Render a recipe offline to a mono buffer, applying `master.normalize`
/// and clipping to -1..=1. Errors are the validation report.
pub fn render(recipe: &Recipe) -> Result<AudioBuffer, Report> {
    let mut voice = prepare(recipe)?;
    let mut samples = vec![0.0f32; voice.total_frames()];
    let mut pos = 0;
    while pos < samples.len() {
        let end = (pos + RENDER_BLOCK).min(samples.len());
        let n = voice.process(&mut samples[pos..end]);
        if n == 0 {
            break;
        }
        pos += n;
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
        trim_trailing_silence(&mut samples, 1);
    }

    Ok(AudioBuffer {
        sample_rate: recipe.sample_rate,
        channels: 1,
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
