//! Loading recipes from TOML or JSON with exact error paths.
//!
//! Deserialization errors from serde are turned into [`Issue`]s with the
//! field path they occurred at (via `serde_path_to_error`), the source line
//! when the format provides it, and a "did you mean" suggestion for unknown
//! field names and enum values.

use crate::recipe::Recipe;
use crate::report::{Code, Issue, Report};
use crate::validate::validate;
use std::path::Path;

/// Text format of a recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// TOML, the on-disk format.
    Toml,
    /// JSON, the tool-call format.
    Json,
}

impl Format {
    /// Pick a format from a file extension. Unknown extensions are TOML.
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Format::Json,
            _ => Format::Toml,
        }
    }
}

/// A recipe that parsed and validated, plus any warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The recipe.
    pub recipe: Recipe,
    /// Non-blocking observations from validation.
    pub report: Report,
}

/// Parse a recipe without semantic validation. Use [`load_str`] for the
/// full pipeline.
pub fn parse_str(src: &str, format: Format) -> Result<Recipe, Report> {
    match format {
        Format::Toml => parse_toml(src),
        Format::Json => parse_json(src),
    }
}

/// Parse and validate a recipe. On success the report holds only warnings.
pub fn load_str(src: &str, format: Format) -> Result<Loaded, Report> {
    let recipe = parse_str(src, format)?;
    let report = validate(&recipe);
    if report.is_ok() {
        Ok(Loaded { recipe, report })
    } else {
        Err(report)
    }
}

/// Read, parse and validate a recipe file. Format is chosen by extension.
pub fn load_path(path: &Path) -> Result<Loaded, Report> {
    let src = std::fs::read_to_string(path).map_err(|e| {
        Report::from_error(Issue::error(
            Code::Other,
            "",
            format!("cannot read {}: {e}", path.display()),
        ))
    })?;
    load_str(&src, Format::from_path(path))
}

/// Serialize a recipe as TOML.
pub fn to_toml(recipe: &Recipe) -> String {
    toml::to_string_pretty(recipe).expect("recipe serializes to TOML")
}

/// Serialize a recipe as pretty JSON.
pub fn to_json(recipe: &Recipe) -> String {
    serde_json::to_string_pretty(recipe).expect("recipe serializes to JSON")
}

fn parse_toml(src: &str) -> Result<Recipe, Report> {
    let de = toml::de::Deserializer::parse(src).map_err(|e| {
        let mut issue = Issue::error(Code::Syntax, "", e.message().to_string());
        if let Some(span) = e.span() {
            let (line, col) = line_col(src, span.start);
            issue = issue.at(line, col);
        }
        Report::from_error(issue)
    })?;
    serde_path_to_error::deserialize(de).map_err(|e| {
        let path = path_string(e.path());
        let inner = e.into_inner();
        let pos = inner.span().map(|s| line_col(src, s.start));
        Report::from_error(issue_from_message(inner.message(), &path, pos))
    })
}

fn parse_json(src: &str) -> Result<Recipe, Report> {
    let mut de = serde_json::Deserializer::from_str(src);
    serde_path_to_error::deserialize(&mut de).map_err(|e| {
        let path = path_string(e.path());
        let inner = e.into_inner();
        let pos = Some((inner.line(), inner.column()));
        if inner.is_syntax() || inner.is_eof() {
            let msg = strip_json_position(&inner.to_string());
            let mut issue = Issue::error(Code::Syntax, "", msg);
            if let Some((l, c)) = pos {
                issue = issue.at(l, c);
            }
            return Report::from_error(issue);
        }
        let msg = strip_json_position(&inner.to_string());
        Report::from_error(issue_from_message(&msg, &path, pos))
    })
}

/// serde_json appends " at line N column M" to messages; the position is
/// reported separately so strip it.
fn strip_json_position(msg: &str) -> String {
    match msg.rfind(" at line ") {
        Some(i) => msg[..i].to_string(),
        None => msg.to_string(),
    }
}

/// Render a serde_path_to_error path as `layers[1].pitch.start`.
fn path_string(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let mut out = String::new();
    for seg in path.iter() {
        match seg {
            Segment::Seq { index } => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
            Segment::Map { key } => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Segment::Enum { variant } => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(variant);
            }
            Segment::Unknown => {}
        }
    }
    out
}

fn join_path(parent: &str, field: &str) -> String {
    if parent.is_empty() {
        field.to_string()
    } else {
        format!("{parent}.{field}")
    }
}

fn line_col(src: &str, byte_offset: usize) -> (usize, usize) {
    let offset = byte_offset.min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rsplit('\n')
        .next()
        .map(|s| s.chars().count())
        .unwrap_or(0)
        + 1;
    (line, col)
}

/// Turn a serde error message into a structured issue. serde's messages have
/// a small, stable set of shapes, which we parse here so agents get a code,
/// a path and a suggestion instead of prose.
fn issue_from_message(msg: &str, path: &str, pos: Option<(usize, usize)>) -> Issue {
    let mut issue = if let Some(rest) = msg.strip_prefix("unknown field `") {
        let (name, expected) = split_name_and_expected(rest);
        // serde_path_to_error already ends the path with the unknown key.
        let full = if path.ends_with(&name) {
            path.to_string()
        } else {
            join_path(path, &name)
        };
        let mut i = Issue::error(Code::UnknownField, full, format!("unknown field `{name}`"))
            .got(name.clone());
        if !expected.is_empty() {
            i = i.one_of(expected.iter().cloned());
            if let Some(s) = did_you_mean(&name, &expected) {
                i = i.suggest(s);
            }
        } else {
            i.message = format!("unknown field `{name}` (this object takes no fields)");
        }
        i
    } else if let Some(rest) = msg.strip_prefix("unknown variant `") {
        let (name, expected) = split_name_and_expected(rest);
        let mut i = Issue::error(
            Code::UnknownVariant,
            path,
            format!("unknown value `{name}`"),
        )
        .got(name.clone());
        if !expected.is_empty() {
            i = i.one_of(expected.iter().cloned());
            if let Some(s) = did_you_mean(&name, &expected) {
                i = i.suggest(s);
            }
        }
        i
    } else if let Some(rest) = msg.strip_prefix("missing field `") {
        let name = rest.trim_end_matches('`').to_string();
        Issue::error(
            Code::MissingField,
            join_path(path, &name),
            format!("missing required field `{name}`"),
        )
    } else if msg.starts_with("invalid type") || msg.starts_with("invalid value") {
        Issue::error(Code::TypeMismatch, path, msg.to_string())
    } else {
        Issue::error(Code::Other, path, msg.to_string())
    };
    if let Some((l, c)) = pos {
        issue = issue.at(l, c);
    }
    issue
}

/// Parse "name`, expected one of `a`, `b`" or "name`, expected `a` or `b`"
/// or "name`, there are no fields" into (name, [a, b]).
fn split_name_and_expected(rest: &str) -> (String, Vec<String>) {
    let end = rest.find('`').unwrap_or(rest.len());
    let name = rest[..end].to_string();
    let tail = &rest[end..];
    // tail looks like "`, expected one of `a`, `b`" so the names sit at the
    // even positions after the first split.
    let expected: Vec<String> = tail
        .split('`')
        .skip(2)
        .step_by(2)
        .map(|s| s.to_string())
        .collect();
    (name, expected)
}

/// Common names agents reach for, mapped to the field we actually have.
/// Only used when the target exists in the candidate list.
const ALIASES: &[(&str, &str)] = &[
    ("layer", "layers"),
    ("track", "layers"),
    ("tracks", "layers"),
    ("voices", "layers"),
    ("effect", "effects"),
    ("fx", "effects"),
    ("gain", "volume"),
    ("amplitude", "volume"),
    ("amp", "volume"),
    ("level", "volume"),
    ("loudness", "volume"),
    ("waveform", "source"),
    ("wave", "source"),
    ("osc", "source"),
    ("oscillator", "source"),
    ("shape", "source"),
    ("frequency", "start"),
    ("freq", "start"),
    ("hz", "start"),
    ("note", "start"),
    ("pulse_width", "duty"),
    ("pwm", "duty"),
    ("env", "envelope"),
    ("adsr", "envelope"),
    ("amp_env", "envelope"),
    ("freq_env", "pitch"),
    ("frequency_envelope", "pitch"),
    ("delay_time", "delay"),
    ("time", "delay"),
    ("wet", "mix"),
    ("dry_wet", "mix"),
    ("amount", "mix"),
    ("q", "resonance"),
    ("res", "resonance"),
    ("freq_end", "end"),
    ("target", "end"),
    ("to", "end"),
    ("from", "start"),
    ("length", "duration"),
    ("seconds", "duration"),
    ("samplerate", "sample_rate"),
    ("rate_hz", "sample_rate"),
    ("random_seed", "seed"),
    ("lowpass", "kind"),
    ("filter_type", "kind"),
    ("mode", "kind"),
    ("steps", "steps"),
    ("notes", "steps"),
    ("speed", "step_time"),
    ("depth_semitones", "depth"),
    ("bit_depth", "bits"),
    ("start_time", "offset"),
    ("delay_start", "offset"),
    ("at", "offset"),
    // enum values
    ("delay", "echo"),
    ("reverberation", "reverb"),
    ("overdrive", "distortion"),
    ("crush", "bitcrush"),
    ("bit_crush", "bitcrush"),
    ("pulse", "square"),
    ("sawtooth", "saw"),
    ("tri", "triangle"),
    ("white_noise", "noise"),
    ("white", "noise"),
    ("low_pass", "lowpass"),
    ("lpf", "lowpass"),
    ("lp", "lowpass"),
    ("high_pass", "highpass"),
    ("hpf", "highpass"),
    ("hp", "highpass"),
    ("band_pass", "bandpass"),
    ("bpf", "bandpass"),
    ("bp", "bandpass"),
    ("log", "exponential"),
    ("exp", "exponential"),
    ("lin", "linear"),
];

/// Best guess at what an agent meant by `name`, given the valid `candidates`.
pub fn did_you_mean(name: &str, candidates: &[String]) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    // 1. Exact alias.
    for (alias, target) in ALIASES {
        if *alias == lower && candidates.iter().any(|c| c == target) {
            return Some(target.to_string());
        }
    }
    // 2. Case or separator slip (e.g. `sampleRate`, `sample-rate`).
    let normalized = lower.replace(['-', ' '], "_");
    let camel_split = split_camel(name);
    for c in candidates {
        if *c == normalized || *c == camel_split {
            return Some(c.clone());
        }
    }
    // 3. Closest by string similarity.
    let mut best: Option<(f64, &String)> = None;
    for c in candidates {
        let score = strsim::jaro_winkler(&lower, c);
        if score > best.map(|b| b.0).unwrap_or(0.0) {
            best = Some((score, c));
        }
    }
    match best {
        Some((score, c)) if score >= 0.78 => Some(c.clone()),
        _ => None,
    }
}

fn split_camel(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            if !out.is_empty() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_you_mean_handles_typos_aliases_and_case() {
        let fields: Vec<String> = ["source", "volume", "pitch", "envelope", "sample_rate"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(did_you_mean("pich", &fields).as_deref(), Some("pitch"));
        assert_eq!(did_you_mean("gain", &fields).as_deref(), Some("volume"));
        assert_eq!(did_you_mean("waveform", &fields).as_deref(), Some("source"));
        assert_eq!(
            did_you_mean("sampleRate", &fields).as_deref(),
            Some("sample_rate")
        );
        assert_eq!(did_you_mean("Volume", &fields).as_deref(), Some("volume"));
        assert_eq!(did_you_mean("zzzzzz", &fields), None);
    }
}
