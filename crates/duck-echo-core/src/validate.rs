//! Semantic validation: range checks and structural rules that serde cannot
//! express. Collects every problem instead of stopping at the first.

use crate::recipe::*;
use crate::report::{Code, Issue, Report};

/// Maximum total length of a render in seconds.
pub const MAX_DURATION: f32 = 60.0;
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

/// Widen an `f32` to the `f64` that prints the same digits, so `0.99`
/// does not surface as `0.9900000095367432` in JSON reports.
fn f64_clean(v: f32) -> f64 {
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

    fn range_opt(&mut self, path: &str, value: Option<f32>, min: f32, max: f32) {
        if let Some(v) = value {
            self.range(path, v, min, max);
        }
    }

    fn warn(&mut self, code: Code, path: &str, msg: impl Into<String>) {
        self.report.push(Issue::warning(code, path, msg));
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
        self.range("sample_rate", r.sample_rate as f32, 8000.0, 192_000.0);
        self.range("variation", r.variation, 0.0, 1.0);
        self.range_opt("duration", r.duration, 0.001, MAX_DURATION);
        self.range("master.gain", r.master.gain, 0.0, 4.0);
        self.range(
            "master.normalize_peak_db",
            r.master.normalize_peak_db,
            -40.0,
            0.0,
        );

        if r.layers.is_empty() {
            self.report.push(Issue::error(
                Code::Invalid,
                "layers",
                "recipe has no layers, so it would render silence; add at least one [[layers]] entry",
            ));
        }
        for (i, layer) in r.layers.iter().enumerate() {
            self.layer(&format!("layers[{i}]"), layer, r.sample_rate);
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
        self.range(&format!("{p}.volume"), l.volume, 0.0, 2.0);
        self.range(&format!("{p}.offset"), l.offset, 0.0, 30.0);
        self.range(&format!("{p}.duty"), l.duty, 0.01, 0.99);
        if l.source != Source::Square && l.duty != 0.5 {
            self.warn(
                Code::Unused,
                &format!("{p}.duty"),
                format!(
                    "duty only affects `square`; this layer is `{}`",
                    source_name(l.source)
                ),
            );
        }

        // Envelope
        let e = &l.envelope;
        let ep = format!("{p}.envelope");
        let mut all_ok = true;
        for (name, val) in [
            ("attack", e.attack),
            ("hold", e.hold),
            ("decay", e.decay),
            ("sustain_time", e.sustain_time),
            ("release", e.release),
        ] {
            all_ok &= self.range(&format!("{ep}.{name}"), val, 0.0, 30.0);
        }
        self.range(&format!("{ep}.sustain"), e.sustain, 0.0, 1.0);
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
        self.range(&format!("{pp}.start"), pitch.start, 1.0, 20_000.0);
        self.range_opt(&format!("{pp}.end"), pitch.end, 1.0, 20_000.0);
        self.range_opt(&format!("{pp}.sweep_time"), pitch.sweep_time, 0.001, 30.0);
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
            self.range(&format!("{vp}.rate"), v.rate, 0.0, 100.0);
            self.range(&format!("{vp}.depth"), v.depth, 0.0, 48.0);
            self.range(&format!("{vp}.delay"), v.delay, 0.0, 30.0);
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
                self.range(&format!("{ap}.steps[{i}]"), *s, -96.0, 96.0);
            }
            self.range(&format!("{ap}.step_time"), a.step_time, 0.001, 30.0);
        }
        let nyquist = sample_rate as f32 / 2.0;
        if pitch.start > nyquist || pitch.end.is_some_and(|e| e > nyquist) {
            self.warn(
                Code::Suspicious,
                &pp,
                format!("pitch exceeds half the sample rate ({nyquist} Hz) and will be clamped"),
            );
        }

        // Filter
        if let Some(f) = &l.filter {
            let fp = format!("{p}.filter");
            self.range(&format!("{fp}.cutoff"), f.cutoff, 10.0, 20_000.0);
            self.range_opt(&format!("{fp}.cutoff_end"), f.cutoff_end, 10.0, 20_000.0);
            self.range_opt(&format!("{fp}.sweep_time"), f.sweep_time, 0.001, 30.0);
            self.range(&format!("{fp}.resonance"), f.resonance, 0.1, 30.0);
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
                self.range(&format!("{p}.delay"), x.delay, 0.001, 5.0);
                self.range(&format!("{p}.feedback"), x.feedback, 0.0, 0.99);
                self.range(&format!("{p}.mix"), x.mix, 0.0, 1.0);
            }
            Effect::Reverb(x) => {
                self.range(&format!("{p}.size"), x.size, 0.0, 1.0);
                self.range(&format!("{p}.decay"), x.decay, 0.05, MAX_TAIL);
                self.range(&format!("{p}.damping"), x.damping, 0.0, 1.0);
                self.range(&format!("{p}.mix"), x.mix, 0.0, 1.0);
            }
            Effect::Distortion(x) => {
                self.range(&format!("{p}.drive"), x.drive, 1.0, 100.0);
                self.range(&format!("{p}.mix"), x.mix, 0.0, 1.0);
            }
            Effect::Bitcrush(x) => {
                self.range(&format!("{p}.bits"), x.bits as f32, 1.0, 16.0);
                self.range_opt(&format!("{p}.rate"), x.rate, 100.0, 192_000.0);
                self.range(&format!("{p}.mix"), x.mix, 0.0, 1.0);
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
