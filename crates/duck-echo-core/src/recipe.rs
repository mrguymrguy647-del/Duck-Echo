//! Recipe types: the single source of truth for every sound.
//!
//! A recipe is plain data. TOML files on disk and JSON tool calls both
//! deserialize into these types, so there is exactly one schema. Every field
//! that is optional has a default, so very short recipes work. Ranges in the
//! doc comments are enforced by [`crate::validate`].
//!
//! Field documentation here is also exported into the JSON Schema, so keep
//! each comment short, concrete and agent-readable.

use crate::note::Frequency;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The recipe schema version this crate understands.
pub const CURRENT_VERSION: u32 = 0;

/// Default sample rate in Hz.
pub const DEFAULT_SAMPLE_RATE: u32 = 44_100;

fn default_sample_rate() -> u32 {
    DEFAULT_SAMPLE_RATE
}
fn default_channels() -> u16 {
    1
}
fn default_volume() -> f32 {
    0.8
}
fn default_duty() -> f32 {
    0.5
}
fn default_attack() -> f32 {
    0.005
}
fn default_decay() -> f32 {
    0.2
}
fn default_pitch_start() -> Frequency {
    Frequency::hz_value(440.0)
}
fn default_vibrato_rate() -> f32 {
    6.0
}
fn default_vibrato_depth() -> f32 {
    0.5
}
fn default_step_time() -> f32 {
    0.05
}
fn default_true() -> bool {
    true
}
fn default_cutoff() -> f32 {
    2000.0
}
fn default_resonance() -> f32 {
    0.707
}
fn default_gain() -> f32 {
    1.0
}
fn default_normalize_peak_db() -> f32 {
    -1.0
}
fn default_echo_delay() -> f32 {
    0.25
}
fn default_echo_feedback() -> f32 {
    0.4
}
fn default_echo_mix() -> f32 {
    0.35
}
fn default_reverb_size() -> f32 {
    0.5
}
fn default_reverb_decay() -> f32 {
    1.0
}
fn default_reverb_damping() -> f32 {
    0.5
}
fn default_reverb_mix() -> f32 {
    0.3
}
fn default_drive() -> f32 {
    2.0
}
fn default_full_mix() -> f32 {
    1.0
}
fn default_bits() -> u32 {
    8
}

/// A complete sound recipe. This is the root document of a `.toml` file or a
/// JSON tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Schema version. Must be `0` for this version of Duck Echo. Required.
    pub version: u32,

    /// Optional name for humans and agents. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Optional one-sentence description. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional tags for search, such as `["pickup", "bright"]`. Not used by
    /// the renderer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Random seed. Same recipe + same seed = byte-identical audio.
    /// Drives the noise source and the `variation` jitter. Default `0`.
    #[serde(default)]
    pub seed: u64,

    /// Output sample rate in Hz. Range 8000..=192000. Default `44100`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Output channels: `1` (mono, default) or `2` (stereo). In stereo each
    /// layer is placed with its `pan`.
    #[serde(default = "default_channels")]
    pub channels: u16,

    /// How much the seed may jitter pitch and envelope times, 0..=1.
    /// `0` (default) means the seed only affects noise. `1` jitters pitch by
    /// up to +-3 semitones and envelope times by up to +-30%.
    #[serde(default)]
    pub variation: f32,

    /// Force the total output length in seconds, range 0.001..=60.
    /// When omitted the length is computed from the layers and effect tails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,

    /// Master output stage, applied after the master `effects`.
    #[serde(default)]
    pub master: Master,

    /// The sound layers, mixed together. At least one layer is required.
    #[serde(default)]
    pub layers: Vec<Layer>,

    /// Master effect chain, applied in order to the mixed layers.
    #[serde(default)]
    pub effects: Vec<Effect>,
}

/// Master output stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Master {
    /// Linear gain applied to the mix, range 0..=4. Default `1.0`.
    pub gain: f32,
    /// Scale the final render so its peak sits at `normalize_peak_db`.
    /// Offline only. Default `false`.
    pub normalize: bool,
    /// Target peak in dBFS when `normalize` is on, range -40..=0. Default `-1`.
    pub normalize_peak_db: f32,
    /// Scale the render so its loudness, as `analyze` measures it, is this
    /// many LUFS, range -60..=0. Replaces `normalize` and
    /// `normalize_peak_db`; the peak never goes above -1 dBFS, so a target
    /// that needs more gain renders quieter (the render result warns).
    /// Offline only. Default: off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalize_lufs: Option<f32>,
    /// Cut the end of the render after the last sample above -60 dBFS (the
    /// analyzer's silence threshold). Removes the silent part of computed
    /// effect tails. Offline only; has no effect when `duration` is set.
    /// Default `false`.
    pub trim_silence: bool,
}

/// The peak ceiling of `master.normalize_lufs`, in dBFS: the gain never
/// puts the peak above it.
pub const LOUDNESS_CEILING_DBFS: f32 = -1.0;

/// Trailing samples at or below this magnitude are removed by
/// `master.trim_silence`. Equals -60 dBFS, the analyzer's silence threshold.
pub const TRIM_THRESHOLD: f32 = 0.001;

impl Default for Master {
    fn default() -> Self {
        Self {
            gain: default_gain(),
            normalize: false,
            normalize_peak_db: default_normalize_peak_db(),
            normalize_lufs: None,
            trim_silence: false,
        }
    }
}

/// One sound layer: an oscillator shaped by an envelope, pitch curve, filter
/// and optional effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Optional name for humans and agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Waveform. One of `sine`, `square`, `saw`, `triangle`, `noise`.
    /// Default `square`.
    #[serde(default)]
    pub source: Source,

    /// Layer volume, linear, range 0..=2. Default `0.8`.
    #[serde(default = "default_volume")]
    pub volume: f32,

    /// Start time of this layer in seconds from the start of the sound,
    /// range 0..=30. Default `0`.
    #[serde(default)]
    pub offset: f32,

    /// Pulse width for `square`, range 0.01..=0.99. Ignored by other
    /// sources. Default `0.5`.
    #[serde(default = "default_duty")]
    pub duty: f32,

    /// How `square` and `saw` handle high notes. `clean` (default) smooths
    /// each jump in the wave, so high notes do not fold back into harsh
    /// extra tones ("aliasing"). `retro` keeps the raw wave of old sound
    /// chips, as Duck Echo made it before this option. Sine, triangle and
    /// noise sound the same either way.
    #[serde(default)]
    pub aliasing: Aliasing,

    /// Stereo position, -1 (left) ..= 1 (right), equal-power. Only used
    /// when the recipe has `channels = 2`. Default `0` (centre).
    #[serde(default)]
    pub pan: f32,

    /// Volume envelope.
    #[serde(default)]
    pub envelope: Envelope,

    /// Pitch over time.
    #[serde(default)]
    pub pitch: Pitch,

    /// Optional filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,

    /// Per-layer effect chain, applied in order before mixing.
    #[serde(default)]
    pub effects: Vec<Effect>,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            name: None,
            source: Source::default(),
            volume: default_volume(),
            offset: 0.0,
            duty: default_duty(),
            aliasing: Aliasing::default(),
            pan: 0.0,
            envelope: Envelope::default(),
            pitch: Pitch::default(),
            filter: None,
            effects: Vec::new(),
        }
    }
}

/// Oscillator waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Pure sine wave.
    Sine,
    /// Pulse wave with `duty` width. The classic chip sound.
    #[default]
    Square,
    /// Rising sawtooth.
    Saw,
    /// Triangle wave.
    Triangle,
    /// White noise driven by `seed`.
    Noise,
}

/// How square and saw waves handle high notes (`layers[].aliasing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aliasing {
    /// Smooth each jump in the wave (band-limited), so high notes stay
    /// clean. The very top of the sound gets a little softer.
    #[default]
    Clean,
    /// The raw wave, as on old sound chips: bright, with extra tones on
    /// high notes.
    Retro,
}

/// Volume envelope, all times in seconds (each 0..=30).
///
/// Shape: rise to 1 over `attack`, stay at 1 for `hold`, fall to `sustain`
/// over `decay`, stay there for `sustain_time`, fall to 0 over `release`.
/// The layer length is the sum of the five times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Envelope {
    /// Time to rise from 0 to full level. Default `0.005`.
    pub attack: f32,
    /// Time held at full level. Default `0`.
    pub hold: f32,
    /// Time to fall from full level to `sustain`. Default `0.2`.
    pub decay: f32,
    /// Sustain level, 0..=1. Default `0`.
    pub sustain: f32,
    /// Time held at `sustain` level. Default `0`.
    pub sustain_time: f32,
    /// Time to fall from `sustain` to 0. Default `0`.
    pub release: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            attack: default_attack(),
            hold: 0.0,
            decay: default_decay(),
            sustain: 0.0,
            sustain_time: 0.0,
            release: 0.0,
        }
    }
}

impl Envelope {
    /// Total envelope length in seconds.
    pub fn length(&self) -> f32 {
        self.attack + self.hold + self.decay + self.sustain_time + self.release
    }
}

/// Pitch over time. Frequencies are numbers in Hz (range 1..=20000) or note
/// names such as `"C5"`, `"F#3"`, `"Bb2"` (A4 = 440 Hz).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Pitch {
    /// Frequency at the start of the layer: Hz or a note name such as
    /// `"A4"`. Default `440`.
    pub start: Frequency,
    /// Frequency at the end of the sweep: Hz or a note name. Omit for a
    /// constant pitch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Frequency>,
    /// Shape of the sweep from `start` to `end`. Default `linear`.
    pub curve: Curve,
    /// Sweep length in seconds, range 0.001..=30. Omit to sweep over the whole
    /// layer. After the sweep the pitch stays at `end`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_time: Option<f32>,
    /// Optional vibrato.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<Vibrato>,
    /// Optional arpeggio (stepped pitch offsets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arpeggio: Option<Arpeggio>,
}

impl Default for Pitch {
    fn default() -> Self {
        Self {
            start: default_pitch_start(),
            end: None,
            curve: Curve::default(),
            sweep_time: None,
            vibrato: None,
            arpeggio: None,
        }
    }
}

/// Interpolation curve for sweeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Straight line in the value's own unit (Hz).
    #[default]
    Linear,
    /// Constant rate of change in octaves: musically even pitch sweeps.
    Exponential,
    /// Starts slow, ends fast.
    EaseIn,
    /// Starts fast, ends slow.
    EaseOut,
}

/// Periodic pitch wobble.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Vibrato {
    /// Wobble rate in Hz, range 0..=100. Default `6`.
    pub rate: f32,
    /// Wobble depth in semitones (peak), range 0..=48. Default `0.5`.
    pub depth: f32,
    /// Seconds before the vibrato starts, range 0..=30. Default `0`.
    pub delay: f32,
}

impl Default for Vibrato {
    fn default() -> Self {
        Self {
            rate: default_vibrato_rate(),
            depth: default_vibrato_depth(),
            delay: 0.0,
        }
    }
}

/// Stepped pitch offsets, the classic chiptune arpeggio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Arpeggio {
    /// Pitch offsets in semitones, played in order. At least one step,
    /// each in -96..=96. Example: `[0, 4, 7]` is a major chord.
    pub steps: Vec<f32>,
    /// Length of each step in seconds, range 0.001..=30. Default `0.05`.
    #[serde(default = "default_step_time")]
    pub step_time: f32,
    /// Loop the steps (`true`, default) or hold the last step (`false`).
    #[serde(default = "default_true")]
    pub repeat: bool,
}

/// Filter type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    /// Passes frequencies below `cutoff`.
    #[default]
    Lowpass,
    /// Passes frequencies above `cutoff`.
    Highpass,
    /// Passes frequencies around `cutoff`.
    Bandpass,
}

/// A resonant 2-pole filter with an optional cutoff sweep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Filter {
    /// One of `lowpass`, `highpass`, `bandpass`. Default `lowpass`.
    pub kind: FilterKind,
    /// Cutoff frequency in Hz at the start, range 10..=20000. Default `2000`.
    pub cutoff: f32,
    /// Cutoff at the end of the sweep in Hz. Omit for a fixed cutoff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutoff_end: Option<f32>,
    /// Sweep shape. Default `linear`.
    pub curve: Curve,
    /// Sweep length in seconds, range 0.001..=30. Omit to sweep over the
    /// whole layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sweep_time: Option<f32>,
    /// Resonance (Q), range 0.1..=30. `0.707` is flat. Default `0.707`.
    pub resonance: f32,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            kind: FilterKind::default(),
            cutoff: default_cutoff(),
            cutoff_end: None,
            curve: Curve::default(),
            sweep_time: None,
            resonance: default_resonance(),
        }
    }
}

/// One effect in a chain. Selected by the `type` field.
///
/// Every effect has a `mix` in 0..=1: `0` is fully dry, `1` is fully wet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    /// Repeating delay.
    Echo(Echo),
    /// Simple room reverb.
    Reverb(Reverb),
    /// Waveshaping distortion.
    Distortion(Distortion),
    /// Bit depth and sample rate reduction.
    Bitcrush(Bitcrush),
}

/// Repeating delay (`type = "echo"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Echo {
    /// Delay time in seconds, range 0.001..=5. Default `0.25`.
    pub delay: f32,
    /// How much of each echo feeds the next, range 0..=0.99. Default `0.4`.
    pub feedback: f32,
    /// Wet/dry mix, range 0..=1. Default `0.35`.
    pub mix: f32,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            delay: default_echo_delay(),
            feedback: default_echo_feedback(),
            mix: default_echo_mix(),
        }
    }
}

/// Simple room reverb (`type = "reverb"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Reverb {
    /// Room size, range 0..=1. Default `0.5`.
    pub size: f32,
    /// Time in seconds for the tail to fall by 60 dB, range 0.05..=10.
    /// Default `1.0`.
    pub decay: f32,
    /// High-frequency damping of the tail, range 0..=1. Default `0.5`.
    pub damping: f32,
    /// Wet/dry mix, range 0..=1. Default `0.3`.
    pub mix: f32,
}

impl Default for Reverb {
    fn default() -> Self {
        Self {
            size: default_reverb_size(),
            decay: default_reverb_decay(),
            damping: default_reverb_damping(),
            mix: default_reverb_mix(),
        }
    }
}

/// Distortion shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DistortionKind {
    /// Smooth saturation.
    #[default]
    Soft,
    /// Hard clipping.
    Hard,
    /// Wave folding: harsh, metallic.
    Fold,
}

/// Waveshaping distortion (`type = "distortion"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Distortion {
    /// Input gain before shaping, range 1..=100. Default `2`.
    pub drive: f32,
    /// One of `soft`, `hard`, `fold`. Default `soft`.
    pub kind: DistortionKind,
    /// Wet/dry mix, range 0..=1. Default `1`.
    pub mix: f32,
}

impl Default for Distortion {
    fn default() -> Self {
        Self {
            drive: default_drive(),
            kind: DistortionKind::default(),
            mix: default_full_mix(),
        }
    }
}

/// Bit depth and sample rate reduction (`type = "bitcrush"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Bitcrush {
    /// Bit depth, range 1..=16. Default `8`.
    pub bits: u32,
    /// Sample-and-hold rate in Hz, range 100..=192000. Omit to keep the full
    /// sample rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
    /// Wet/dry mix, range 0..=1. Default `1`.
    pub mix: f32,
}

impl Default for Bitcrush {
    fn default() -> Self {
        Self {
            bits: default_bits(),
            rate: None,
            mix: default_full_mix(),
        }
    }
}

impl Recipe {
    /// A minimal valid recipe with one default layer. Handy for tests and for
    /// agents that want to start from something that renders.
    pub fn minimal() -> Self {
        Self {
            version: CURRENT_VERSION,
            name: None,
            description: None,
            tags: Vec::new(),
            seed: 0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: 1,
            variation: 0.0,
            duration: None,
            master: Master::default(),
            layers: vec![Layer::default()],
            effects: Vec::new(),
        }
    }
}
