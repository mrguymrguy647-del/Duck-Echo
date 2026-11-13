//! Loading recipes from TOML or JSON with exact error paths.
//!
//! The pipeline reports every problem in one call:
//!
//! 1. Syntax: the text is parsed into a positioned tree ([`crate::tree`]).
//!    A syntax error stops here, since nothing after it can be trusted.
//! 2. Structure: the tree is walked against the JSON Schema
//!    ([`crate::structure`]), collecting every unknown field, unknown enum
//!    value, missing field and type mismatch, and pruning them.
//! 3. Types: a clean document is decoded straight from the source with
//!    serde; a document with structural errors is decoded from the pruned
//!    tree so that step 4 can still run.
//! 4. Semantics: [`validate`] checks ranges and rules. Issues under a path
//!    that step 2 already reported are dropped as duplicates.

use crate::recipe::Recipe;
use crate::report::{Code, Issue, Report};
use crate::structure::{self, is_under};
use crate::tree::{self, line_col};
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

/// Outcome of steps 1 to 3 of the pipeline.
struct Parsed {
    /// The decoded recipe; for a document with structural errors this is
    /// the pruned version, used only for further checking.
    recipe: Option<Recipe>,
    /// Syntax and structural issues.
    report: Report,
    /// Paths whose semantic issues are duplicates of structural ones.
    poisoned: Vec<String>,
    /// The positioned document, used to give semantic issues a line.
    tree: Option<tree::Node>,
    /// (decoded path, written path) for misspelled entries that were decoded
    /// under their suggested name, innermost first.
    renames: Vec<(String, String)>,
}

fn parse_full(src: &str, format: Format) -> Parsed {
    let tree = match format {
        Format::Toml => tree::parse_toml(src),
        Format::Json => tree::parse_json(src),
    };
    let tree = match tree {
        Ok(t) => t,
        Err(issue) => {
            return Parsed {
                recipe: None,
                report: Report::from_error(*issue),
                poisoned: Vec::new(),
                tree: None,
                renames: Vec::new(),
            }
        }
    };
    let checked = structure::check(&tree);
    if checked.issues.is_empty() {
        let decoded = match format {
            Format::Toml => typed_toml(src),
            Format::Json => typed_json(src),
        };
        return match decoded {
            Ok(recipe) => Parsed {
                recipe: Some(recipe),
                report: Report::new(),
                poisoned: Vec::new(),
                tree: Some(tree),
                renames: Vec::new(),
            },
            Err(report) => Parsed {
                recipe: None,
                report,
                poisoned: Vec::new(),
                tree: Some(tree),
                renames: Vec::new(),
            },
        };
    }
    let mut report = Report::new();
    let mut issues = checked.issues;
    // Report in document order, whatever order the parser stored keys in.
    issues.sort_by_key(|i| (i.line.unwrap_or(usize::MAX), i.column.unwrap_or(0)));
    for issue in issues {
        report.push(issue);
    }
    let recipe = checked
        .pruned
        .and_then(|v| serde_json::from_value::<Recipe>(v).ok());
    Parsed {
        recipe,
        report,
        poisoned: checked.poisoned,
        tree: Some(tree),
        renames: checked.renames,
    }
}

/// Parse a recipe without semantic validation. Use [`load_str`] for the
/// full pipeline. On error the report lists every syntax and structural
/// problem (but not range problems).
pub fn parse_str(src: &str, format: Format) -> Result<Recipe, Report> {
    let parsed = parse_full(src, format);
    match parsed.recipe {
        Some(recipe) if parsed.report.is_ok() => Ok(recipe),
        _ => Err(parsed.report),
    }
}

/// Parse and validate a recipe, reporting every problem at once. On
/// success the report holds only warnings.
pub fn load_str(src: &str, format: Format) -> Result<Loaded, Report> {
    let parsed = parse_full(src, format);
    let mut report = parsed.report;
    if let Some(recipe) = &parsed.recipe {
        let semantic = validate(recipe);
        for mut issue in semantic.errors.into_iter().chain(semantic.warnings) {
            // Report issues inside a misspelled entry at the name written.
            // Outer renames were recorded last, so apply them first.
            for (decoded, written) in parsed.renames.iter().rev() {
                if is_under(&issue.path, decoded) {
                    issue.path = format!("{written}{}", &issue.path[decoded.len()..]);
                }
            }
            if parsed
                .poisoned
                .iter()
                .any(|root| is_under(&issue.path, root))
            {
                continue;
            }
            if issue.line.is_none() {
                if let Some((l, c)) = parsed
                    .tree
                    .as_ref()
                    .and_then(|t| tree::position_of(t, &issue.path))
                {
                    issue = issue.at(l, c);
                }
            }
            report.push(issue);
        }
    }
    match parsed.recipe {
        Some(recipe) if report.is_ok() => Ok(Loaded { recipe, report }),
        _ => Err(report),
    }
}

/// Read, parse and validate a recipe file. Format is chosen by extension.
pub fn load_path(path: &Path) -> Result<Loaded, Report> {
    let src = std::fs::read_to_string(path).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            Code::NotFound
        } else {
            Code::Io
        };
        Report::from_error(Issue::error(
            code,
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

/// Typed decode straight from TOML text. Only reached for documents the
/// structural check passed, so errors here mean the walker missed a case;
/// they are still reported with a path.
fn typed_toml(src: &str) -> Result<Recipe, Report> {
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

/// Typed decode straight from JSON text. See [`typed_toml`].
fn typed_json(src: &str) -> Result<Recipe, Report> {
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
        Issue::error(Code::Invalid, path, msg.to_string())
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

/// A likely intended name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The suggested valid name.
    pub name: String,
    /// True for a known alias, a case or separator slip, or a small edit
    /// (at most two typos, and few relative to the length). Only confident
    /// suggestions are used to check the contents of a misspelled entry as
    /// if it had the suggested name.
    pub confident: bool,
}

/// Best guess at what an agent meant by `name`, given the valid `candidates`.
pub fn did_you_mean(name: &str, candidates: &[String]) -> Option<String> {
    suggest(name, candidates).map(|s| s.name)
}

/// Like [`did_you_mean`], with a confidence flag.
pub fn suggest(name: &str, candidates: &[String]) -> Option<Suggestion> {
    let lower = name.to_ascii_lowercase();
    let sure = |c: &str| {
        Some(Suggestion {
            name: c.to_string(),
            confident: true,
        })
    };
    // 1. Exact alias.
    for (alias, target) in ALIASES {
        if *alias == lower && candidates.iter().any(|c| c == target) {
            return sure(target);
        }
    }
    // 2. Case or separator slip (e.g. `sampleRate`, `sample-rate`).
    let normalized = lower.replace(['-', ' '], "_");
    let camel_split = split_camel(name);
    for c in candidates {
        if *c == normalized || *c == camel_split {
            return sure(c);
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
        Some((score, c)) if score >= 0.78 => {
            let edits = strsim::osa_distance(&lower, c);
            let longest = lower.chars().count().max(c.chars().count());
            Some(Suggestion {
                name: c.clone(),
                confident: edits <= 2 && edits * 3 <= longest,
            })
        }
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

    #[test]
    fn suggestion_confidence() {
        let fields: Vec<String> = ["layers", "pitch", "volume", "effects"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let conf = |n: &str| suggest(n, &fields).map(|s| (s.name, s.confident));
        assert_eq!(conf("layer"), Some(("layers".into(), true)));
        assert_eq!(conf("fx"), Some(("effects".into(), true)));
        assert_eq!(conf("pich"), Some(("pitch".into(), true)));
        // A long extra word is a guess, not a typo.
        assert_eq!(conf("pitchbend"), Some(("pitch".into(), false)));
    }
}
