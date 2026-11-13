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
pub mod loudness;
pub mod math;
pub mod song;

use duck_echo_audio::AudioBuffer;
use duck_echo_core::validate::chain_tail_seconds;
use duck_echo_core::{Layer, Master, Recipe, Report, Source};

use dsp::{ArpShape, Biquad, EnvShape, LfoShaper, Oscillator, PitchCurve, Sweep, VibratoShape};
use duck_echo_core::{LfoTarget, NoiseMode};
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
    /// Position in the recipe's `layers`.
    index: usize,
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
    /// The layer's LFO, if it has one, and the `duty` it moves around.
    lfo: Option<LfoShaper>,
    duty: f32,
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
    build(recipe, None)
}

/// How one note of a song plays an instrument recipe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotePlay {
    /// Every layer's pitch is multiplied by this: `note / home`.
    pub ratio: f32,
    /// How long the note is held, in seconds. `None` plays the recipe's
    /// own envelope from start to end (a drum hit).
    pub gate: Option<f32>,
    /// The voice's output gain (velocity times the instrument's level).
    pub gain: f32,
    /// Seed for this note's noise and variation.
    pub seed: u64,
    /// The song's sample rate, used instead of the recipe's.
    pub sample_rate: u32,
    /// The song's channels (1 or 2), used instead of the recipe's.
    pub channels: usize,
}

/// Prepare a [`Voice`] for one note of a song. It plays the recipe's
/// layers only: the recipe's master effects are left out, because in a
/// song they run once per track on the sum of its notes (see [`song`]).
///
/// Layers that would play above 20 kHz at this note are left out; they
/// could not be heard and would fold back down as a false tone.
pub fn prepare_note(recipe: &Recipe, play: &NotePlay) -> Voice {
    build(recipe, Some(play))
}

/// Highest pitch a layer may start at in a note, Hz.
const NOTE_MAX_HZ: f32 = 20_000.0;

/// The shared planner behind [`prepare_unchecked`] (`note` is `None`) and
/// [`prepare_note`].
fn build(recipe: &Recipe, note: Option<&NotePlay>) -> Voice {
    let sample_rate = note.map_or(recipe.sample_rate, |n| n.sample_rate);
    let sr = sample_rate as f32;
    let root = Rng::new(note.map_or(recipe.seed, |n| n.seed));
    let mut variation_rng = root.fork(STREAM_VARIATION);

    let mut layers = Vec::with_capacity(recipe.layers.len());
    let mut last_end = 0usize;
    for (i, layer) in recipe.layers.iter().enumerate() {
        // Always draw so a layer's jitter does not depend on `variation`
        // being non-zero for earlier layers.
        let pitch_jitter = variation_rng.bipolar();
        let time_jitter = variation_rng.bipolar();
        let mut pitch_mult = math::semitones_to_ratio(recipe.variation * 3.0 * pitch_jitter);
        let time_scale = 1.0 + recipe.variation * 0.3 * time_jitter;

        let mut gate = None;
        if let Some(n) = note {
            pitch_mult *= n.ratio;
            if layer.has_pitch() && layer.pitch.start.hz() * pitch_mult > NOTE_MAX_HZ {
                continue;
            }
            if let Some(g) = n.gate {
                // The note is let go `g` seconds after it starts; a layer
                // that starts later than that is never played.
                if g <= layer.offset {
                    continue;
                }
                gate = Some(g - layer.offset);
            }
        }

        let lv = LayerVoice::new(
            i,
            layer,
            sr,
            pitch_mult,
            time_scale,
            gate,
            root.fork(STREAM_LAYER + i as u64),
        );
        last_end = last_end.max(lv.end_frame);
        layers.push(lv);
    }

    let (channels, gain, effects, total_frames) = match note {
        Some(n) => (n.channels.clamp(1, 2), n.gain, &[][..], last_end.max(1)),
        None => {
            let master_tail = chain_tail_seconds(&recipe.effects);
            let total = match recipe.duration {
                Some(d) => ((d * sr).round() as usize).max(1),
                None => (last_end + (master_tail * sr).round() as usize).max(1),
            };
            (
                recipe.channels.clamp(1, 2) as usize,
                recipe.master.gain,
                &recipe.effects[..],
                total,
            )
        }
    };
    Voice {
        sample_rate,
        channels,
        total_frames,
        position: 0,
        gain,
        layers,
        master_effects: (0..channels)
            .map(|_| effects.iter().map(|e| EffectState::new(e, sr)).collect())
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
    fn new(
        index: usize,
        layer: &Layer,
        sr: f32,
        pitch_mult: f32,
        time_scale: f32,
        gate: Option<f32>,
        rng: Rng,
    ) -> Self {
        let env = match gate {
            Some(g) => EnvShape::gated(&layer.envelope, time_scale, g),
            None => EnvShape::new(&layer.envelope, time_scale),
        };
        let env_len = env.length().min(30.0);
        let tail = chain_tail_seconds(&layer.effects);
        let start_frame = (layer.offset * sr).round() as usize;
        let env_frames = (env_len * sr).round() as usize;
        let end_frame = start_frame + env_frames + (tail * sr).round() as usize;

        let p = &layer.pitch;
        let sweep = Sweep::new(
            p.start.hz() * pitch_mult,
            p.end.as_ref().map(|e| e.hz() * pitch_mult),
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
            index,
            start_frame,
            end_frame,
            env_frames,
            local: 0,
            volume: layer.volume,
            inv_sample_rate: 1.0 / sr,
            env,
            pitch,
            osc: Oscillator::new(
                layer.source,
                layer.duty,
                layer.aliasing == duck_echo_core::Aliasing::Clean,
                layer.noise_mode == NoiseMode::Pitched,
                rng,
            ),
            filter,
            lfo: layer.lfo.as_ref().map(LfoShaper::new),
            duty: layer.duty,
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

        // What the LFO moves, if there is one. Without an LFO the loop
        // below does exactly what it did before LFOs existed.
        let target = self.lfo.as_ref().map(LfoShaper::target);

        // 1. Oscillator shaped by envelope and volume.
        for s in buf.iter_mut() {
            let local = self.local;
            if local < self.env_frames {
                let t = local as f32 * self.inv_sample_rate;
                let hz = self.pitch.at(t);
                let mut level = self.env.level(t) * self.volume;
                if let Some(lfo) = &self.lfo {
                    match target {
                        Some(LfoTarget::Volume) => level *= lfo.gain(t),
                        Some(LfoTarget::Duty) => self.osc.set_duty(self.duty + lfo.offset(t)),
                        _ => {}
                    }
                }
                *s = self.osc.next(hz * self.inv_sample_rate) * level;
            } else {
                *s = 0.0;
            }
            // 2. Filter with control-rate cutoff updates.
            if let Some((bq, sweep)) = &mut self.filter {
                if self.control_counter == 0 {
                    let t = local as f32 * self.inv_sample_rate;
                    let mut cutoff = sweep.at(t);
                    if let (Some(LfoTarget::Cutoff), Some(lfo)) = (target, &self.lfo) {
                        // `offset` is in octaves: +1 doubles the cutoff.
                        cutoff *= math::exp2(lfo.offset(t));
                    }
                    bq.set_cutoff(cutoff);
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

    /// Silence every layer whose index (its position in the recipe's
    /// `layers`) is not in `heard`. Layers keep their seed streams,
    /// `variation` jitter and timing, and the length stays that of the full
    /// mix, so the heard layers sound exactly as they do in it. Call before
    /// the first [`Self::process`]; indices past the end are ignored.
    pub fn keep_layers(&mut self, heard: &[usize]) {
        self.layers.retain(|l| heard.contains(&l.index));
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

/// What the offline master steps did to a render. [`render_layers`] uses it
/// to finish part of a mix exactly like the whole.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mastering {
    /// Gain applied by `master.normalize` or `master.normalize_lufs`; `1`
    /// when neither is on.
    pub gain: f32,
    /// Frames in the finished render, after `trim_silence`.
    pub frames: usize,
    /// True when `normalize_lufs` needed more gain than the -1 dBFS peak
    /// ceiling allowed, so the render is quieter than its target.
    pub ceiling_limited: bool,
}

/// Most measuring passes `normalize_lufs` makes. The first gain comes from
/// the raw mix; later passes correct for what trimming and the active span
/// change. Two passes usually settle within 0.001 dB.
const LOUDNESS_PASSES: usize = 4;

/// Run a voice to the end, unclipped.
fn mix(mut voice: Voice) -> Vec<f32> {
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
    samples
}

/// Render a recipe offline to a buffer (interleaved when stereo),
/// applying `master.normalize` or `master.normalize_lufs`, clipping to
/// -1..=1 and `master.trim_silence`. Errors are the validation report.
pub fn render(recipe: &Recipe) -> Result<AudioBuffer, Report> {
    render_mastered(recipe).map(|(buffer, _)| buffer)
}

/// [`render`], also returning what the master steps did.
pub fn render_mastered(recipe: &Recipe) -> Result<(AudioBuffer, Mastering), Report> {
    let voice = prepare(recipe)?;
    let channels = voice.channels();
    let trim = recipe.master.trim_silence && recipe.duration.is_none();
    let (samples, mastering) = master(
        &recipe.master,
        recipe.sample_rate,
        trim,
        mix(voice),
        channels,
    );
    Ok((
        AudioBuffer {
            sample_rate: recipe.sample_rate,
            channels: channels as u16,
            samples,
        },
        mastering,
    ))
}

/// Render only the layers whose indices are in `heard`, finished with the
/// full mix's `mastering` (from [`render_mastered`]): the same gain, then
/// clipping, then cut or padded to the same length. The result is what
/// those layers contribute to the mix, time-aligned with it. Master effects
/// still apply, so with a nonlinear master effect (distortion, bitcrush)
/// the parts do not add up to the whole.
pub fn render_layers(
    recipe: &Recipe,
    heard: &[usize],
    mastering: &Mastering,
) -> Result<AudioBuffer, Report> {
    let mut voice = prepare(recipe)?;
    voice.keep_layers(heard);
    let channels = voice.channels();
    let mut samples = mix(voice);
    for s in &mut samples {
        *s = (*s * mastering.gain).clamp(-1.0, 1.0);
    }
    samples.resize(mastering.frames * channels, 0.0);
    Ok(AudioBuffer {
        sample_rate: recipe.sample_rate,
        channels: channels as u16,
        samples,
    })
}

/// The offline master steps on a raw mix: normalize (by peak or loudness),
/// clip, trim. `trim` says whether `trim_silence` may cut the end.
pub(crate) fn master(
    settings: &Master,
    sample_rate: u32,
    trim: bool,
    mut samples: Vec<f32>,
    channels: usize,
) -> (Vec<f32>, Mastering) {
    if let Some(target) = settings.normalize_lufs {
        return normalize_loudness(sample_rate, &samples, channels, target, trim);
    }
    let mut gain = 1.0;
    if settings.normalize {
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > 0.0 {
            let target = math::db_to_gain(settings.normalize_peak_db);
            gain = target / peak;
            for s in &mut samples {
                *s *= gain;
            }
        }
    }
    clip_and_trim(&mut samples, channels, trim);
    let frames = samples.len() / channels;
    (
        samples,
        Mastering {
            gain,
            frames,
            ceiling_limited: false,
        },
    )
}

fn clip_and_trim(samples: &mut Vec<f32>, channels: usize, trim: bool) {
    for s in samples.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
    if trim {
        trim_trailing_silence(samples, channels);
    }
}

/// `master.normalize_lufs`: find the gain that makes the finished render
/// (scaled, clipped and trimmed, as written) measure `target` LUFS, capped
/// so the peak stays at or under [`duck_echo_core::LOUDNESS_CEILING_DBFS`]. Deterministic: the
/// measurement is [`loudness::mean_square`] and the pass count is fixed.
fn normalize_loudness(
    sr: u32,
    raw: &[f32],
    channels: usize,
    target: f32,
    trim: bool,
) -> (Vec<f32>, Mastering) {
    let finish = |gain: f32| {
        let mut out: Vec<f32> = raw.iter().map(|s| s * gain).collect();
        clip_and_trim(&mut out, channels, trim);
        out
    };
    let done = |samples: Vec<f32>, gain: f32, ceiling_limited: bool| {
        let frames = samples.len() / channels;
        (
            samples,
            Mastering {
                gain,
                frames,
                ceiling_limited,
            },
        )
    };
    let peak = raw.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let measured = (peak > 0.0)
        .then(|| loudness::mean_square(raw, channels, sr))
        .flatten();
    let Some((ms, _)) = measured else {
        // Silence (or below the -70 LUFS gate): nothing to level.
        return done(finish(1.0), 1.0, false);
    };
    let ceiling = math::db_to_gain(duck_echo_core::LOUDNESS_CEILING_DBFS) / peak;
    let mut gain = loudness::gain_to(target, ms);
    let mut out = Vec::new();
    let mut used = gain;
    let mut limited = false;
    for _ in 0..LOUDNESS_PASSES {
        let capped = gain >= ceiling;
        used = if capped { ceiling } else { gain };
        out = finish(used);
        let Some((ms, _)) = loudness::mean_square(&out, channels, sr) else {
            break;
        };
        let correction = loudness::gain_to(target, ms);
        // 1e-4 is about 0.001 dB.
        limited = capped && correction > 1.0 + 1e-3;
        if (correction - 1.0).abs() < 1e-4 || (capped && correction >= 1.0) {
            break;
        }
        gain = used * correction;
    }
    done(out, used, limited)
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
