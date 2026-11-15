//! Semantic validation: range checks and structural rules that serde cannot
//! express. Collects every problem instead of stopping at the first.

use crate::applies::{self, Applies};
use crate::ranges::{self, Range};
use crate::recipe::*;
use crate::report::{Code, Issue, Report};

/// Maximum total length of a render in seconds.
pub const MAX_DURATION: f32 = 60.0;
/// Quietest `master.normalize_lufs` target.
pub const MIN_NORMALIZE_LUFS: f32 = -60.0;
/// Maximum length of any single effect tail in seconds.
pub const MAX_TAIL: f32 = 10.0;
/// Silence floor used to decide when a tail has died out (-60 dB).
const TAIL_FLOOR: f32 = 0.001;

/// Validate a parsed recipe. Errors mean it must not render; warnings mean it
/// renders but something looks off.
pub fn validate(recipe: &Recipe) -> Report {
    let mut v = Validator {
        report: Report::new(),
    };
    v.recipe(recipe);
    v.report
}

/// Check the parts a song shares with a recipe: the `master` table and a
/// top-level `effects` list. Paths are the same as in a recipe.
pub(crate) fn validate_master_and_effects(
    master: &Master,
    effects: &[Effect],
    sample_rate: u32,
) -> Report {
    let mut v = Validator {
        report: Report::new(),
    };
    v.master(master);
    for (i, effect) in effects.iter().enumerate() {
        v.effect(&format!("effects[{i}]"), effect, sample_rate);
    }
    v.report
}

/// Widen an `f32` to the `f64` that prints the same digits, so `0.99`
/// does not surface as `0.9900000095367432` in JSON reports.
pub(crate) fn f64_clean(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(v as f64)
}

struct Validator {
    report: Report,
}

impl Validator {
    fn range(&mut self, path: &str, value: f32, min: f32, max: f32) -> bool {
        if value.is_nan() || value < min || value > max {
            self.report.push(
                Issue::error(
                    Code::OutOfRange,
                    path,
                    format!("value {value} is outside the allowed range {min}..={max}"),
                )
                .got(f64_clean(value))
                .range(f64_clean(min), f64_clean(max)),
            );
            false
        } else {
            true
        }
    }

    /// Check `value` against its range from the table in [`ranges`].
    fn check(&mut self, path: &str, value: f32, r: Range) -> bool {
        self.range(path, value, r.min as f32, r.max as f32)
    }

    fn check_opt(&mut self, path: &str, value: Option<f32>, r: Range) {
        if let Some(v) = value {
            self.check(path, v, r);
        }
    }

    fn warn(&mut self, code: Code, path: &str, msg: impl Into<String>) {
        self.report.push(Issue::warning(code, path, msg));
    }

    /// The `master` table: ranges, and settings that cancel each other.
    fn master(&mut self, m: &Master) {
        self.check("master.gain", m.gain, ranges::MASTER_GAIN);
        self.check(
            "master.normalize_peak_db",
            m.normalize_peak_db,
            ranges::MASTER_NORMALIZE_PEAK_DB,
        );
        self.check_opt(
            "master.normalize_lufs",
            m.normalize_lufs,
            ranges::MASTER_NORMALIZE_LUFS,
        );
        if m.normalize && m.normalize_lufs.is_some() {
            self.warn(
                Code::Unused,
                "master.normalize",
                format!(
                    "normalize and normalize_peak_db have no effect when normalize_lufs is set; loudness normalization replaces them and keeps the peak at or under {LOUDNESS_CEILING_DBFS} dBFS; remove normalize"
                ),
            );
        }
    }

    fn recipe(&mut self, r: &Recipe) {
        if r.version != CURRENT_VERSION {
            self.report.push(
                Issue::error(
                    Code::UnsupportedVersion,
                    "version",
                    format!(
                        "schema version {} is not supported by this build (supports {})",
                        r.version, CURRENT_VERSION
                    ),
                )
                .got(r.version)
                .one_of([CURRENT_VERSION.to_string()]),
            );
        }
        self.check("sample_rate", r.sample_rate as f32, ranges::SAMPLE_RATE);
        self.check("channels", r.channels as f32, ranges::CHANNELS);
        self.check("variation", r.variation, ranges::VARIATION);
        self.check_opt("duration", r.duration, ranges::DURATION);
        self.master(&r.master);
        if r.master.trim_silence && r.duration.is_some() {
            self.warn(
                Code::Unused,
                "master.trim_silence",
                "trim_silence has no effect when `duration` is set; the render keeps exactly `duration` seconds",
            );
        }

        if r.layers.is_empty() {
            self.report.push(Issue::error(
                Code::Invalid,
                "layers",
                "recipe has no layers, so it would render silence; add at least one [[layers]] entry",
            ));
        }
        for (i, layer) in r.layers.iter().enumerate() {
            self.layer(&format!("layers[{i}]"), layer, r.sample_rate);
            if r.channels == 1 && layer.pan != 0.0 {
                self.warn(
                    Code::Unused,
                    &format!("layers[{i}].pan"),
                    "pan only affects stereo output; set `channels = 2` at the top level",
                );
            }
        }
        for (i, effect) in r.effects.iter().enumerate() {
            self.effect(&format!("effects[{i}]"), effect, r.sample_rate);
        }

        // Total length check uses the un-jittered values; variation may
        // stretch times by up to 30%, which the renderer caps separately.
        // Skipped when any individual value is already out of range: the
        // length would only repeat that error.
        if r.duration.is_none() && self.report.errors.is_empty() {
            let est = estimate_length(r);
            if est > MAX_DURATION {
                self.report.push(
                    Issue::error(
                        Code::Invalid,
                        "layers",
                        format!(
                            "computed length {est:.2} s exceeds the {MAX_DURATION} s limit; shorten envelopes or offsets, or set `duration`"
                        ),
                    )
                    .got(f64_clean(est))
                    .range(0.0, MAX_DURATION as f64),
                );
            }
        }
    }

    fn layer(&mut self, p: &str, l: &Layer, sample_rate: u32) {
        self.check(&format!("{p}.volume"), l.volume, ranges::LAYER_VOLUME);
        self.check(&format!("{p}.offset"), l.offset, ranges::LAYER_OFFSET);
        self.check(&format!("{p}.duty"), l.duty, ranges::LAYER_DUTY);
        self.check(&format!("{p}.pan"), l.pan, ranges::LAYER_PAN);
        if l.aliasing == Aliasing::Retro && !does_something(&applies::LAYER_ALIASING, l) {
            self.warn(
                Code::Unused,
                &format!("{p}.aliasing"),
                format!(
                    "aliasing only changes `square` and `saw`; this layer is `{}`",
                    source_name(l.source)
                ),
            );
        }
        if l.duty != 0.5 && !does_something(&applies::LAYER_DUTY, l) {
            self.warn(
                Code::Unused,
                &format!("{p}.duty"),
                format!(
                    "duty only affects `square`; this layer is `{}`",
                    source_name(l.source)
                ),
            );
        }

        if l.noise_mode == NoiseMode::Pitched && !does_something(&applies::LAYER_NOISE_MODE, l) {
            self.warn(
                Code::Unused,
                &format!("{p}.noise_mode"),
                format!(
                    "noise_mode only affects `noise`; this layer is `{}`",
                    source_name(l.source)
                ),
            );
        }
        if let Some(lfo) = &l.lfo {
            let lp = format!("{p}.lfo");
            self.check(&format!("{lp}.rate"), lfo.rate, ranges::LFO_RATE);
            self.check(&format!("{lp}.delay"), lfo.delay, ranges::LFO_DELAY);
            let max = ranges::lfo_depth(lfo.target).max as f32;
            let unit = match lfo.target {
                LfoTarget::Volume => "a part of the volume",
                LfoTarget::Cutoff => "octaves",
                LfoTarget::Duty => "an amount of duty",
            };
            if lfo.depth.is_nan() || lfo.depth < 0.0 || lfo.depth > max {
                self.report.push(
                    Issue::error(
                        Code::OutOfRange,
                        format!("{lp}.depth"),
                        format!(
                            "value {} is outside the allowed range 0..={max} for target `{}` (depth is {unit} there)",
                            lfo.depth,
                            lfo_target_name(lfo.target)
                        ),
                    )
                    .got(f64_clean(lfo.depth))
                    .range(0.0, f64_clean(max)),
                );
            }
            match lfo.target {
                LfoTarget::Cutoff if l.filter.is_none() => self.report.push(
                    Issue::error(
                        Code::Invalid,
                        format!("{lp}.target"),
                        "an LFO on `cutoff` needs a filter on this layer; add `filter = { cutoff = 1000 }`, or use target `volume`",
                    )
                    .got("cutoff"),
                ),
                LfoTarget::Duty if l.source != Source::Square => self.report.push(
                    Issue::error(
                        Code::Invalid,
                        format!("{lp}.target"),
                        format!(
                            "an LFO on `duty` needs `source = \"square\"`; this layer is `{}`",
                            source_name(l.source)
                        ),
                    )
                    .got("duty"),
                ),
                _ => {}
            }
        }

        // Envelope
        let e = &l.envelope;
        let ep = format!("{p}.envelope");
        let mut all_ok = true;
        for (name, val, range) in [
            ("attack", e.attack, ranges::ENVELOPE_ATTACK),
            ("hold", e.hold, ranges::ENVELOPE_HOLD),
            ("decay", e.decay, ranges::ENVELOPE_DECAY),
            (
                "sustain_time",
                e.sustain_time,
                ranges::ENVELOPE_SUSTAIN_TIME,
            ),
            ("release", e.release, ranges::ENVELOPE_RELEASE),
        ] {
            all_ok &= self.check(&format!("{ep}.{name}"), val, range);
        }
        self.check(
            &format!("{ep}.sustain"),
            e.sustain,
            ranges::ENVELOPE_SUSTAIN,
        );
        if all_ok && e.length() <= 0.0 {
            self.report.push(Issue::error(
                Code::Invalid,
                &ep,
                "envelope has zero length; set attack, hold, decay, sustain_time or release above 0",
            ));
        }
        if e.sustain > 0.0 && e.sustain_time == 0.0 && e.release == 0.0 {
            self.warn(
                Code::Suspicious,
                &format!("{ep}.sustain"),
                "sustain is above 0 but sustain_time and release are 0, so the sound ends abruptly at the sustain level (a click)",
            );
        }

        // Pitch
        let pitch = &l.pitch;
        let pp = format!("{p}.pitch");
        self.check(&format!("{pp}.start"), pitch.start.hz(), ranges::FREQUENCY);
        self.check_opt(
            &format!("{pp}.end"),
            pitch.end.as_ref().map(|e| e.hz()),
            ranges::FREQUENCY,
        );
        self.check_opt(
            &format!("{pp}.sweep_time"),
            pitch.sweep_time,
            ranges::PITCH_SWEEP_TIME,
        );
        if pitch.end.is_none() && pitch.sweep_time.is_some() {
            self.warn(
                Code::Unused,
                &format!("{pp}.sweep_time"),
                "sweep_time has no effect without `end`",
            );
        }
        if pitch.end.is_none() && pitch.curve != Curve::Linear {
            self.warn(
                Code::Unused,
                &format!("{pp}.curve"),
                "curve has no effect without `end`",
            );
        }
        if let Some(v) = &pitch.vibrato {
            let vp = format!("{pp}.vibrato");
            self.check(&format!("{vp}.rate"), v.rate, ranges::VIBRATO_RATE);
            self.check(&format!("{vp}.depth"), v.depth, ranges::VIBRATO_DEPTH);
            self.check(&format!("{vp}.delay"), v.delay, ranges::VIBRATO_DELAY);
        }
        if let Some(a) = &pitch.arpeggio {
            let ap = format!("{pp}.arpeggio");
            if a.steps.is_empty() {
                self.report.push(Issue::error(
                    Code::Invalid,
                    format!("{ap}.steps"),
                    "arpeggio needs at least one step, for example [0, 4, 7]",
                ));
            }
            for (i, s) in a.steps.iter().enumerate() {
                self.check(&format!("{ap}.steps[{i}]"), *s, ranges::ARPEGGIO_STEP);
            }
            self.check(
                &format!("{ap}.step_time"),
                a.step_time,
                ranges::ARPEGGIO_STEP_TIME,
            );
        }
        let nyquist = sample_rate as f32 / 2.0;
        if pitch.start.hz() > nyquist || pitch.end.as_ref().is_some_and(|e| e.hz() > nyquist) {
            self.warn(
                Code::Suspicious,
                &pp,
                format!("pitch exceeds half the sample rate ({nyquist} Hz) and will be clamped"),
            );
        }

        // Filter
        if let Some(f) = &l.filter {
            let fp = format!("{p}.filter");
            self.check(&format!("{fp}.cutoff"), f.cutoff, ranges::FILTER_CUTOFF);
            self.check_opt(
                &format!("{fp}.cutoff_end"),
                f.cutoff_end,
                ranges::FILTER_CUTOFF_END,
            );
            self.check_opt(
                &format!("{fp}.sweep_time"),
                f.sweep_time,
                ranges::FILTER_SWEEP_TIME,
            );
            self.check(
                &format!("{fp}.resonance"),
                f.resonance,
                ranges::FILTER_RESONANCE,
            );
            if f.cutoff_end.is_none() && f.sweep_time.is_some() {
                self.warn(
                    Code::Unused,
                    &format!("{fp}.sweep_time"),
                    "sweep_time has no effect without `cutoff_end`",
                );
            }
            if f.cutoff > nyquist || f.cutoff_end.is_some_and(|c| c > nyquist) {
                self.warn(
                    Code::Suspicious,
                    &fp,
                    format!(
                        "cutoff exceeds half the sample rate ({nyquist} Hz) and will be clamped"
                    ),
                );
            }
        }

        for (i, effect) in l.effects.iter().enumerate() {
            self.effect(&format!("{p}.effects[{i}]"), effect, sample_rate);
        }
    }

    fn effect(&mut self, p: &str, e: &Effect, sample_rate: u32) {
        match e {
            Effect::Echo(x) => {
                self.check(&format!("{p}.delay"), x.delay, ranges::ECHO_DELAY);
                self.check(&format!("{p}.feedback"), x.feedback, ranges::ECHO_FEEDBACK);
                self.check(&format!("{p}.mix"), x.mix, ranges::ECHO_MIX);
            }
            Effect::Reverb(x) => {
                self.check(&format!("{p}.size"), x.size, ranges::REVERB_SIZE);
                self.check(&format!("{p}.decay"), x.decay, ranges::REVERB_DECAY);
                self.check(&format!("{p}.damping"), x.damping, ranges::REVERB_DAMPING);
                self.check(&format!("{p}.mix"), x.mix, ranges::REVERB_MIX);
            }
            Effect::Distortion(x) => {
                self.check(&format!("{p}.drive"), x.drive, ranges::DISTORTION_DRIVE);
                self.check(&format!("{p}.mix"), x.mix, ranges::DISTORTION_MIX);
            }
            Effect::Bitcrush(x) => {
                self.check(&format!("{p}.bits"), x.bits as f32, ranges::BITCRUSH_BITS);
                self.check_opt(&format!("{p}.rate"), x.rate, ranges::BITCRUSH_RATE);
                self.check(&format!("{p}.mix"), x.mix, ranges::BITCRUSH_MIX);
                if x.rate.is_some_and(|r| r >= sample_rate as f32) {
                    self.warn(
                        Code::Unused,
                        &format!("{p}.rate"),
                        format!("rate is at or above the sample rate ({sample_rate} Hz), so it has no effect"),
                    );
                }
            }
        }
    }
}

fn lfo_target_name(t: LfoTarget) -> &'static str {
    match t {
        LfoTarget::Volume => "volume",
        LfoTarget::Cutoff => "cutoff",
        LfoTarget::Duty => "duty",
    }
}

/// True when the field of `rule` does something on layer `l`.
fn does_something(rule: &Applies, l: &Layer) -> bool {
    rule.holds(|sibling| layer_word(l, sibling).to_string())
}

/// The value of one of a layer's word fields, as written in a recipe.
/// Only the siblings that the rules of [`applies`] name are needed.
fn layer_word(l: &Layer, field: &str) -> &'static str {
    match field {
        "source" => source_name(l.source),
        "noise_mode" => match l.noise_mode {
            NoiseMode::White => "white",
            NoiseMode::Pitched => "pitched",
        },
        "aliasing" => match l.aliasing {
            Aliasing::Clean => "clean",
            Aliasing::Retro => "retro",
        },
        _ => "",
    }
}

fn source_name(s: Source) -> &'static str {
    match s {
        Source::Sine => "sine",
        Source::Square => "square",
        Source::Saw => "saw",
        Source::Triangle => "triangle",
        Source::Noise => "noise",
    }
}

/// How long an effect keeps producing sound after its input stops, in
/// seconds. Uses only multiplication so every platform agrees.
pub fn effect_tail_seconds(e: &Effect) -> f32 {
    let tail = match e {
        Effect::Echo(x) => {
            if x.mix <= 0.0 {
                0.0
            } else {
                // Number of repeats until the echo falls below the floor.
                let mut g = 1.0f32;
                let mut n = 1u32;
                while g * x.feedback > TAIL_FLOOR && n < 1000 {
                    g *= x.feedback;
                    n += 1;
                }
                x.delay * n as f32
            }
        }
        Effect::Reverb(x) => {
            if x.mix <= 0.0 {
                0.0
            } else {
                x.decay
            }
        }
        Effect::Distortion(_) | Effect::Bitcrush(_) => 0.0,
    };
    tail.min(MAX_TAIL)
}

/// Total tail of an effect chain. Tails add because each effect can extend
/// the output of the previous one.
pub fn chain_tail_seconds(effects: &[Effect]) -> f32 {
    effects
        .iter()
        .map(effect_tail_seconds)
        .sum::<f32>()
        .min(MAX_TAIL)
}

/// Computed length of a recipe in seconds, before `variation` jitter and
/// ignoring the `duration` override.
pub fn estimate_length(r: &Recipe) -> f32 {
    let layers = r
        .layers
        .iter()
        .map(|l| l.offset + l.envelope.length() + chain_tail_seconds(&l.effects))
        .fold(0.0f32, f32::max);
    layers + chain_tail_seconds(&r.effects)
}

#[cfg(test)]
mod applies_tests {
    use super::*;

    #[test]
    fn the_applies_table_agrees_with_the_renderer() {
        use crate::applies::{LAYER_DUTY, LAYER_NOISE_MODE, LAYER_PITCH};
        for source in [
            Source::Sine,
            Source::Square,
            Source::Saw,
            Source::Triangle,
            Source::Noise,
        ] {
            for noise_mode in [NoiseMode::White, NoiseMode::Pitched] {
                let l = Layer {
                    source,
                    noise_mode,
                    ..Layer::default()
                };
                // `has_pitch` is what songs and the synth use.
                assert_eq!(does_something(&LAYER_PITCH, &l), l.has_pitch());
                assert_eq!(does_something(&LAYER_DUTY, &l), source == Source::Square);
                assert_eq!(
                    does_something(&LAYER_NOISE_MODE, &l),
                    source == Source::Noise
                );
            }
        }
    }
}
