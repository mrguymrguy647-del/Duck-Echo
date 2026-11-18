//! Check a mix from its cue list (`mix_check`, `duck-echo mix`).
//!
//! A cue list says which sound plays when, how loud and where. It is read
//! in either of two forms:
//!
//! * **Duck Forge's `log.json`, as a headless run writes it.** Only three
//!   of its fields are read: `updates_per_second`, `sounds` (`{"frame",
//!   "sound", "file", "volume", "pan"}`) and `loops` (`{"loop", "file",
//!   "start", "stop", "volume": [[frame, level], ...]}`); its other
//!   fields are the game's own report and are left alone. Times are
//!   frames, placed as Duck Forge places them: frame `f` starts at sample
//!   `(f - 1) x 44100 / updates_per_second`. A loop's volume holds from
//!   its frame until the next point, as in the game. A sound from Duck
//!   Forge's own synthesizer (`"wave"` instead of `"file"`) cannot be
//!   mixed here and is listed in `not_mixed`. Files are named from the
//!   folder the game ran in: the Duck Forge folder (the nearest folder
//!   above the log whose `Cargo.toml` is Duck Forge's), or `base`. Pan is
//!   Duck Forge's: one side is turned down, and a centred sound is at
//!   full volume on both sides.
//! * **A plain list**, times in seconds:
//!   `[{"time": 1.25, "file": "sfx/door.wav", "name": "door",
//!   "volume": 0.8, "pan": 0}]`, where `"loop": true` (with `"stop"`)
//!   plays the file over and over. Unknown fields are errors. Files are
//!   named from the list's own folder (or `base`). Pan is equal-power, as
//!   Duck Echo's own stereo (a centred sound is -3 dB on each side).
//!
//! A cue's `file` is a WAV, a recipe or a song (rendered first) or
//! `preset:NAME`. The check mixes them at 44.1 kHz in stereo and reports:
//!
//! * every moment over full volume, the sounds behind it, and volumes
//!   that would fix it (all of them down together, or the loudest alone);
//! * the short-term loudness (3 s windows, K-weighted) each second, and
//!   the loudness of each section (given, or every 10 s);
//! * every cue buried under the rest of the mix (more than 15 dB under
//!   what plays at the same moment), with the volume that would lift it;
//! * a picture of the mix with a marker and a name for each cue.
//!
//! It writes the mix itself (`<name>.mix.wav`, to listen to), the report
//! (`<name>.mix.json`) and the picture (`<name>.mix.png`).
//!
//! Made for Duck Forge while making the sound of "Pick a Door: Blocks":
//! a door and its music bed clipped at 12 s, and a footstep was lost
//! under the wind; both were found by ear, late.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use duck_echo_analyze::board::{text_width, Board, Ink, CHAR_H, CHAR_W};
use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Code, Issue, Report};
use duck_echo_synth::math;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::root::Root;
use crate::{file_safe, ToolError};

/// The mix's sample rate.
pub const MIX_RATE: u32 = 44_100;
/// Longest mix, seconds.
pub const MAX_MIX_SECONDS: f64 = 600.0;
/// Most cues in one list.
pub const MAX_CUES: usize = 2000;
/// Highest volume a cue may have.
pub const MAX_VOLUME: f32 = 16.0;
/// A cue this far under the rest of the mix is buried, dB.
pub const BURIED_DB: f32 = 15.0;
/// The peak a fix aims for: -1 dBFS.
const FIX_PEAK: f32 = 0.891;
/// Over-full moments closer than this are one moment, seconds.
const OVER_JOIN_SECONDS: f64 = 0.05;
/// The short-term loudness window, seconds (EBU R 128).
const SHORT_TERM_SECONDS: f64 = 3.0;
/// Sections when none are given, seconds.
const DEFAULT_SECTION_SECONDS: f64 = 10.0;

// ---------------------------------------------------------------------
// The two forms of a cue list.
// ---------------------------------------------------------------------

/// One cue of a plain list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlainCue {
    /// When it starts, seconds.
    pub time: f64,
    /// The file it plays.
    pub file: String,
    /// Its name. Default: the file's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its volume, linear. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    /// Its pan, -1 to 1. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
    /// True: play the file over and over until `stop` (or the end).
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub looping: Option<bool>,
    /// When a loop stops, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<f64>,
}

/// A part of the film or game, for its own loudness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// Its name.
    pub name: String,
    /// Where it starts, seconds.
    pub start: f64,
    /// Where it ends, seconds.
    pub end: f64,
}

/// A sound in the cue list that the check cannot mix.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NotMixed {
    /// Its name.
    pub name: String,
    /// Where it starts, seconds.
    pub start_seconds: f64,
    /// Why it is left out.
    pub why: String,
}

/// How a cue list pans a sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanLaw {
    /// Duck Echo's own: equal power, a centred sound -3 dB on each side.
    EqualPower,
    /// Duck Forge's: one side turned down, a centred sound at full volume
    /// on both sides.
    OneSideDown,
}

impl PanLaw {
    /// The left and right gains for `pan`, -1 (left) to 1 (right).
    fn gains(self, pan: f32) -> [f32; 2] {
        match self {
            PanLaw::EqualPower => [
                math::sin_turns((1.0 - pan) * 0.125),
                math::sin_turns((1.0 + pan) * 0.125),
            ],
            // As Duck Forge's `sound::pan_gains`.
            PanLaw::OneSideDown => [(1.0 - pan).min(1.0), (1.0 + pan).min(1.0)],
        }
    }
}

/// One cue, whichever form it came from, placed in samples.
#[derive(Debug, Clone, PartialEq)]
struct Cue {
    /// Where it is in the cue list, for errors: `sounds[3]`, `[3]`.
    at: String,
    name: String,
    file: String,
    /// The sample it starts at.
    start: usize,
    /// The sample a loop stops at.
    stop: Option<usize>,
    looping: bool,
    /// (sample, level) points, in order: each level holds until the next
    /// point; before the first, the first level holds. Never empty.
    volume: Vec<(usize, f32)>,
    pan: f32,
}

impl Cue {
    /// Its level at `sample`.
    fn level(&self, sample: usize) -> f32 {
        self.volume
            .iter()
            .take_while(|(from, _)| *from <= sample)
            .last()
            .or(self.volume.first())
            .map_or(1.0, |p| p.1)
    }

    /// Where it starts, seconds.
    fn start_seconds(&self) -> f64 {
        self.start as f64 / f64::from(MIX_RATE)
    }

    /// Its volume as one number: the first level (a one-shot has only
    /// one).
    fn volume_now(&self) -> f32 {
        self.volume.first().map_or(1.0, |p| p.1)
    }
}

/// A cue list, read.
struct CueList {
    /// `duck_forge_log` or `plain_list`.
    form: &'static str,
    cues: Vec<Cue>,
    not_mixed: Vec<NotMixed>,
    /// How long the game ran, in samples (a Duck Forge log's `frames`).
    ran: Option<usize>,
    pan: PanLaw,
    /// Problems found while reading, reported with the rest.
    report: Report,
}

/// Read a cue list's text: Duck Forge's log (an object) or a plain list.
fn parse_cues(text: &str) -> Result<CueList, ToolError> {
    // Checked for syntax with exact lines first.
    let tree = duck_echo_core::tree::parse_json(text);
    let is_list = matches!(
        &tree,
        Ok(duck_echo_core::tree::Node {
            value: duck_echo_core::tree::Value::Array(_),
            ..
        })
    );
    let mut list = if is_list {
        parse_plain(&tree.map_err(|issue| ToolError::Invalid(Report::from_error(*issue)))?)?
    } else {
        // Duck Forge writes a synthesizer sound's own `volume` and the
        // play's `volume` on one line, so a key can be there twice. This
        // reader takes the last, as serde_json does.
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| {
            ToolError::issue(Issue::error(
                Code::Syntax,
                "",
                format!("the cue list is not JSON: {e}"),
            ))
        })?;
        parse_forge(&value)?
    };
    let mut report = std::mem::take(&mut list.report);
    for c in &list.cues {
        if !(-1.0..=1.0).contains(&c.pan) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    format!("{}.pan", c.at),
                    format!("pan is -1..=1, got {}", c.pan),
                )
                .got(c.pan)
                .range(-1.0, 1.0),
            );
        }
        if c.volume.iter().any(|p| !(0.0..=MAX_VOLUME).contains(&p.1)) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    format!("{}.volume", c.at),
                    "a volume is 0..=16 (1 plays the file as it is)",
                )
                .range(0.0, f64::from(MAX_VOLUME)),
            );
        }
    }
    if list.cues.len() > MAX_CUES {
        report.push(Issue::error(
            Code::Invalid,
            "",
            format!(
                "the cue list holds {} cues; one check mixes at most {MAX_CUES}",
                list.cues.len()
            ),
        ));
    }
    // (Only when nothing else is wrong: cues with mistakes are left out.)
    if list.cues.is_empty() && list.not_mixed.is_empty() && report.is_ok() {
        report.push(Issue::error(Code::Invalid, "", "the cue list has no cues"));
    }
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    list.cues.sort_by_key(|c| c.start);
    Ok(list)
}

/// A plain list: every field checked, unknown ones are errors.
fn parse_plain(tree: &duck_echo_core::tree::Node) -> Result<CueList, ToolError> {
    let schema = serde_json::to_value(schemars::schema_for!(Vec<PlainCue>)).expect("schema");
    let checked = duck_echo_core::structure::check_against(tree, &schema);
    let mut report = Report::new();
    for issue in checked.issues {
        report.push(issue);
    }
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    let list: Vec<PlainCue> =
        serde_json::from_value(checked.pruned.unwrap_or_default()).map_err(|e| {
            ToolError::failed(Code::Invalid, format!("the cue list does not decode: {e}"))
        })?;
    let sr = f64::from(MIX_RATE);
    let mut cues = Vec::new();
    for (i, c) in list.into_iter().enumerate() {
        let at = format!("[{i}]");
        let looping = c.looping.unwrap_or(false);
        if c.stop.is_some() && !looping {
            report.push(
                Issue::error(
                    Code::Invalid,
                    format!("{at}.stop"),
                    "`stop` is for a loop; a one-shot sound plays to its end",
                )
                .suggest("\"loop\": true"),
            );
        }
        if !(c.time.is_finite() && c.time >= 0.0 && c.time <= MAX_MIX_SECONDS) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    format!("{at}.time"),
                    format!(
                        "a cue starts at 0 seconds or later (at most {MAX_MIX_SECONDS}), got {}",
                        c.time
                    ),
                )
                .got(c.time),
            );
            continue;
        }
        if let Some(stop) = c.stop {
            // (A stop that is not a number counts as before the start.)
            if stop.is_nan() || stop < c.time {
                report.push(
                    Issue::error(
                        Code::OutOfRange,
                        format!("{at}.stop"),
                        format!(
                            "a loop stops after it starts ({:.3} s), got {stop:.3} s",
                            c.time
                        ),
                    )
                    .got(stop),
                );
            }
        }
        let start = (c.time * sr).round() as usize;
        cues.push(Cue {
            name: c.name.unwrap_or_else(|| name_of(&c.file)),
            file: c.file,
            start,
            stop: c
                .stop
                .map(|s| (s.min(MAX_MIX_SECONDS) * sr).round() as usize),
            looping,
            volume: vec![(start, c.volume.unwrap_or(1.0))],
            pan: c.pan.unwrap_or(0.0),
            at,
        });
    }
    Ok(CueList {
        form: "plain_list",
        cues,
        not_mixed: Vec::new(),
        ran: None,
        pan: PanLaw::EqualPower,
        report,
    })
}

/// The name of a cue from its file: the file name without its ending.
fn name_of(file: &str) -> String {
    Path::new(file)
        .file_stem()
        .map_or_else(|| file.to_string(), |s| s.to_string_lossy().into_owned())
}

/// Duck Forge's log: only `updates_per_second`, `sounds` and `loops` are
/// read, field by field, with a path for every problem.
fn parse_forge(log: &serde_json::Value) -> Result<CueList, ToolError> {
    use serde_json::Value;
    let mut report = Report::new();
    let Some(obj) = log.as_object() else {
        return Err(ToolError::issue(Issue::error(
            Code::TypeMismatch,
            "",
            "a cue list is Duck Forge's log.json (an object with updates_per_second) or a list of cues",
        )));
    };
    let ups =
        match obj.get("updates_per_second").and_then(Value::as_f64) {
            Some(u) if u.is_finite() && u > 0.0 => u,
            Some(u) => {
                return Err(ToolError::issue(
                    Issue::error(
                        Code::OutOfRange,
                        "updates_per_second",
                        format!("updates_per_second must be above 0, got {u}"),
                    )
                    .got(u)
                    .suggest("60"),
                ))
            }
            None => return Err(ToolError::issue(
                Issue::error(
                    Code::MissingField,
                    "updates_per_second",
                    "a Duck Forge log says its updates_per_second; a plain cue list is a JSON list",
                )
                .suggest("60"),
            )),
        };
    // Where frame `f` starts, as Duck Forge's `Simulation::sample_at`:
    // whole-number math when the rate is a whole number, so it never drifts.
    let sample_of = |frame: f64| -> usize {
        let before = (frame - 1.0).max(0.0);
        if ups.fract() == 0.0 && before.fract() == 0.0 {
            (before as u64 * u64::from(MIX_RATE) / ups as u64) as usize
        } else {
            (before * f64::from(MIX_RATE) / ups).floor() as usize
        }
    };
    let frame_at = |v: Option<&Value>, path: String, report: &mut Report| -> Option<f64> {
        match v.and_then(Value::as_f64) {
            Some(f) if f.is_finite() && f >= 1.0 && f.fract() == 0.0 => Some(f),
            Some(f) => {
                report.push(
                    Issue::error(
                        Code::OutOfRange,
                        path,
                        format!("a frame is a whole number from 1, got {f}"),
                    )
                    .got(f),
                );
                None
            }
            None => {
                report.push(Issue::error(
                    Code::MissingField,
                    path,
                    "a frame number is needed here",
                ));
                None
            }
        }
    };
    let number = |v: Option<&Value>, default: f32, path: String, report: &mut Report| -> f32 {
        match v {
            None | Some(Value::Null) => default,
            Some(v) => match v.as_f64() {
                Some(n) => n as f32,
                None => {
                    report.push(
                        Issue::error(Code::TypeMismatch, path, "expected a number").got(v.clone()),
                    );
                    default
                }
            },
        }
    };
    let list = |key: &str, report: &mut Report| -> Vec<Value> {
        match obj.get(key) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(other) => {
                report.push(
                    Issue::error(Code::TypeMismatch, key, "expected a list").got(other.clone()),
                );
                Vec::new()
            }
        }
    };
    let synth = "Duck Forge's own synthesizer sound (it has a wave, not a file): not in this mix";
    let mut cues = Vec::new();
    let mut not_mixed = Vec::new();
    for (i, s) in list("sounds", &mut report).iter().enumerate() {
        let at = format!("sounds[{i}]");
        let Some(entry) = s.as_object() else {
            report.push(
                Issue::error(Code::TypeMismatch, &at, "expected a sound's line").got(s.clone()),
            );
            continue;
        };
        let name = entry
            .get("sound")
            .and_then(Value::as_str)
            .unwrap_or("sound")
            .to_string();
        let Some(frame) = frame_at(entry.get("frame"), format!("{at}.frame"), &mut report) else {
            continue;
        };
        let start = sample_of(frame);
        match entry.get("file").and_then(Value::as_str) {
            Some(file) => cues.push(Cue {
                name,
                file: file.to_string(),
                start,
                stop: None,
                looping: false,
                volume: vec![(
                    start,
                    number(
                        entry.get("volume"),
                        1.0,
                        format!("{at}.volume"),
                        &mut report,
                    ),
                )],
                pan: number(entry.get("pan"), 0.0, format!("{at}.pan"), &mut report),
                at,
            }),
            None if entry.contains_key("wave") => not_mixed.push(NotMixed {
                name,
                start_seconds: ms(start as f64 / f64::from(MIX_RATE)),
                why: synth.into(),
            }),
            None => report.push(Issue::error(
                Code::MissingField,
                format!("{at}.file"),
                "a sound's line has a `file` (a WAV) or a `wave` (Duck Forge's synthesizer)",
            )),
        }
    }
    for (i, l) in list("loops", &mut report).iter().enumerate() {
        let at = format!("loops[{i}]");
        let Some(entry) = l.as_object() else {
            report.push(
                Issue::error(Code::TypeMismatch, &at, "expected a loop's line").got(l.clone()),
            );
            continue;
        };
        let name = entry
            .get("loop")
            .and_then(Value::as_str)
            .unwrap_or("loop")
            .to_string();
        let Some(start_frame) = frame_at(entry.get("start"), format!("{at}.start"), &mut report)
        else {
            continue;
        };
        let start = sample_of(start_frame);
        let stop = match entry.get("stop") {
            None | Some(Value::Null) => None,
            v => {
                let f = frame_at(v, format!("{at}.stop"), &mut report);
                if f.is_some_and(|f| f < start_frame) {
                    report.push(Issue::error(
                        Code::OutOfRange,
                        format!("{at}.stop"),
                        format!("a loop stops on or after the frame it starts on ({start_frame})"),
                    ));
                }
                f.map(sample_of)
            }
        };
        let mut volume = Vec::new();
        match entry.get("volume") {
            None | Some(Value::Null) => volume.push((start, 1.0)),
            Some(Value::Array(points)) => {
                for (j, p) in points.iter().enumerate() {
                    let path = format!("{at}.volume[{j}]");
                    let pair = p.as_array().filter(|a| a.len() == 2);
                    let Some(pair) = pair else {
                        report.push(
                            Issue::error(Code::TypeMismatch, path, "expected [frame, level]")
                                .got(p.clone())
                                .suggest("[1, 0.6]"),
                        );
                        continue;
                    };
                    let Some(f) = frame_at(pair.first(), format!("{path}[0]"), &mut report) else {
                        continue;
                    };
                    let level = number(pair.get(1), 1.0, format!("{path}[1]"), &mut report);
                    let sample = sample_of(f);
                    if volume.last().is_some_and(|(s, _)| *s > sample) {
                        report.push(Issue::error(
                            Code::Invalid,
                            &path,
                            "the volume points must be in frame order",
                        ));
                    }
                    volume.push((sample, level));
                }
                if volume.is_empty() {
                    volume.push((start, 1.0));
                }
            }
            Some(n) => volume.push((
                start,
                number(Some(n), 1.0, format!("{at}.volume"), &mut report),
            )),
        }
        match entry.get("file").and_then(Value::as_str) {
            Some(file) => cues.push(Cue {
                name,
                file: file.to_string(),
                start,
                stop,
                looping: true,
                volume,
                pan: 0.0,
                at,
            }),
            None if entry.contains_key("wave") => not_mixed.push(NotMixed {
                name,
                start_seconds: ms(start as f64 / f64::from(MIX_RATE)),
                why: synth.into(),
            }),
            None => report.push(Issue::error(
                Code::MissingField,
                format!("{at}.file"),
                "a loop's line has a `file` (a WAV) or a `wave` (Duck Forge's synthesizer)",
            )),
        }
    }
    // How long the game ran: up to where the frame after its last starts.
    let ran = obj
        .get("frames")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && *f >= 0.0)
        .map(|f| sample_of(f + 1.0));
    Ok(CueList {
        form: "duck_forge_log",
        cues,
        not_mixed,
        ran,
        pan: PanLaw::OneSideDown,
        report,
    })
}

// ---------------------------------------------------------------------
// The report.
// ---------------------------------------------------------------------

/// One sound in a moment over full volume.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverPart {
    /// The cue's name.
    pub name: String,
    /// Where it starts, seconds.
    pub start_seconds: f64,
    /// Its volume.
    pub volume: f32,
    /// Its own largest sample in that moment, dBFS.
    pub peak_dbfs: Option<f32>,
}

/// A change of one cue's volume.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VolumeChange {
    /// The cue's name.
    pub name: String,
    /// Its volume now.
    pub from: f32,
    /// The volume that fixes it.
    pub to: f32,
}

/// A moment over full volume.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Over {
    /// Where it starts, seconds.
    pub start_seconds: f64,
    /// Where it ends, seconds.
    pub end_seconds: f64,
    /// Its largest sample, dBFS (above 0).
    pub peak_dbfs: f32,
    /// The sounds playing in it, loudest first.
    pub sounds: Vec<OverPart>,
    /// Turn every one of them down this much, dB, so the peak is -1 dBFS.
    pub all_down_db: f32,
    /// Their volumes after that.
    pub all_down: Vec<VolumeChange>,
    /// Or turn only the loudest down, to this (`null` when that alone
    /// cannot fix it).
    pub loudest_alone: Option<VolumeChange>,
}

/// One cue, as the report shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CueReport {
    /// Its name.
    pub name: String,
    /// Its file, as the list names it.
    pub file: String,
    /// Where it starts, seconds.
    pub start_seconds: f64,
    /// Where it ends, seconds.
    pub end_seconds: f64,
    /// True for a loop.
    pub looping: bool,
    /// Its volume (a loop's first level).
    pub volume: f32,
    /// Its loudness as it plays in the mix, LUFS (K-weighted over its
    /// sounding part).
    pub loudness_lufs: Option<f32>,
    /// The rest of the mix over the same part, LUFS (`null` when nothing
    /// else plays).
    pub rest_lufs: Option<f32>,
    /// True when it is more than 15 dB under the rest.
    pub buried: bool,
    /// For a buried cue: the sounds it is under.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub under: Vec<String>,
    /// For a buried cue: the volume that puts it 10 dB under the rest;
    /// `null` when that is above 16: lower the others instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lift_to: Option<f32>,
}

/// The loudness of one section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionReport {
    /// Its name.
    pub name: String,
    /// Where it starts, seconds.
    pub start_seconds: f64,
    /// Where it ends, seconds.
    pub end_seconds: f64,
    /// Its loudness, LUFS (gated BS.1770; `null` when silent).
    pub loudness_lufs: Option<f32>,
    /// Its largest sample, dBFS.
    pub peak_dbfs: Option<f32>,
}

/// What `mix_check` answers (and writes as `<name>.mix.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MixChecked {
    /// Always true; failures are errors instead.
    pub ok: bool,
    /// Always `duck-echo-mix-check`.
    pub kind: &'static str,
    /// The mix's name (the cue list's file name).
    pub name: String,
    /// `duck_forge_log` or `plain_list`.
    pub form: &'static str,
    /// Length, seconds.
    pub length_seconds: f64,
    /// Always 44100.
    pub sample_rate: u32,
    /// The mix, WAV.
    pub mix_wav: String,
    /// The picture, PNG.
    pub picture_png: String,
    /// This report, JSON.
    pub report_json: String,
    /// Loudness of the whole mix, LUFS.
    pub loudness_lufs: Option<f32>,
    /// Largest sample of the mix, dBFS (above 0 is over full volume).
    pub peak_dbfs: Option<f32>,
    /// One line: the length, loudness and peak, and every problem.
    pub summary: String,
    /// Every moment over full volume.
    pub overs: Vec<Over>,
    /// Every cue, in time order, the buried ones marked.
    pub cues: Vec<CueReport>,
    /// Sounds in the cue list that could not be mixed (Duck Forge's own
    /// synthesizer sounds), so the mix is without them.
    pub not_mixed: Vec<NotMixed>,
    /// Short-term loudness (the 3 s up to the end of each second), LUFS;
    /// `null` for a silent second.
    pub short_term_lufs: Vec<Option<f32>>,
    /// The loudness of each section.
    pub sections: Vec<SectionReport>,
}

// ---------------------------------------------------------------------
// Mixing.
// ---------------------------------------------------------------------

/// A cue's audio: stereo at the mix rate.
struct Audio {
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Audio {
    fn len(&self) -> usize {
        self.left.len()
    }
}

/// The audio of a file: a WAV read, anything else rendered.
fn load_audio(root: &Root, folder: &str, cue: &Cue) -> Result<Audio, ToolError> {
    let files = root.song_files(folder.to_string());
    let path = if cue.file.starts_with(crate::PRESET_PREFIX) {
        root.read("", &cue.file)?
    } else {
        files(&cue.file)?
    };
    let buffer: AudioBuffer = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        duck_echo_audio::read_wav(&path)
            .map_err(|e| ToolError::failed(Code::Io, format!("cannot read {}: {e}", cue.file)))?
    } else {
        let doc = crate::load_document(&path, &files)?;
        crate::measure::measure_document(&doc, None, &crate::measure::Options::default(), "")?
            .buffer
    };
    Ok(to_mix_rate(&buffer))
}

/// Stereo at [`MIX_RATE`]: mono is copied to both sides, and another rate
/// is read with straight lines between its samples.
fn to_mix_rate(buffer: &AudioBuffer) -> Audio {
    let ch = usize::from(buffer.channels.max(1));
    let frames = buffer.samples.len() / ch;
    let side = |c: usize| -> Vec<f32> {
        let c = c.min(ch - 1);
        buffer.samples.chunks_exact(ch).map(|f| f[c]).collect()
    };
    let (l, r) = (side(0), side(1));
    if buffer.sample_rate == MIX_RATE || frames == 0 {
        return Audio { left: l, right: r };
    }
    let ratio = f64::from(buffer.sample_rate) / f64::from(MIX_RATE);
    let out_frames = ((frames as f64) / ratio).floor() as usize;
    let resample = |x: &[f32]| -> Vec<f32> {
        (0..out_frames)
            .map(|i| {
                let pos = i as f64 * ratio;
                let k = pos.floor() as usize;
                let f = (pos - k as f64) as f32;
                let a = x[k.min(frames - 1)];
                let b = x[(k + 1).min(frames - 1)];
                a + (b - a) * f
            })
            .collect()
    };
    Audio {
        left: resample(&l),
        right: resample(&r),
    }
}

/// One cue placed in the mix: what it adds to each side at a frame.
struct Placed<'a> {
    cue: &'a Cue,
    audio: &'a Audio,
    start: usize,
    end: usize,
    gains: [f32; 2],
}

impl Placed<'_> {
    /// What it adds to the mix at `frame` (between `start` and `end`).
    fn at(&self, frame: usize) -> [f32; 2] {
        if frame < self.start || frame >= self.end || self.audio.len() == 0 {
            return [0.0; 2];
        }
        let mut k = frame - self.start;
        if self.cue.looping {
            k %= self.audio.len();
        } else if k >= self.audio.len() {
            return [0.0; 2];
        }
        let level = self.cue.level(frame);
        [
            self.audio.left[k] * level * self.gains[0],
            self.audio.right[k] * level * self.gains[1],
        ]
    }
}

/// What to mix and how: the arguments of `mix_check`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Request<'a> {
    /// The cue list, relative to the project folder.
    pub cues: &'a str,
    /// Parts to measure on their own, in seconds (default every 10 s).
    pub sections: Option<&'a [Section]>,
    /// The mix's length in seconds (default: the end of the last sound,
    /// the last loop's stop, or how long a Duck Forge game ran).
    pub length: Option<f64>,
    /// The folder the cues' files are named from, relative to the project
    /// folder (default: see the module's notes).
    pub base: Option<&'a str>,
    /// The folder for the mix, the report and the picture (default the
    /// cue list's own).
    pub out: Option<&'a str>,
}

/// The Duck Forge folder a log is in: the nearest folder above it, inside
/// the project folder, whose `Cargo.toml` says `name = "duckforge"`.
fn duck_forge_folder_of(root: &Root, log: &Path) -> Option<PathBuf> {
    log.ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root.dir()))
        .find(|d| {
            std::fs::read_to_string(d.join("Cargo.toml")).is_ok_and(|t| {
                t.lines()
                    .any(|l| l.replace(' ', "") == "name=\"duckforge\"")
            })
        })
        .map(Path::to_path_buf)
}

/// Mix a cue list and check it (see [`Request`]). Arguments in errors are
/// `<prefix>cues`, `<prefix>base` and so on.
pub fn check(root: &Root, request: &Request, prefix: &str) -> Result<MixChecked, ToolError> {
    let Request {
        cues: cues_file,
        sections,
        length,
        base,
        out,
    } = *request;
    let key = format!("{prefix}cues");
    let path = root.read(&key, cues_file)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ToolError::failed(Code::Io, format!("cannot read {cues_file}: {e}")))?;
    let list = parse_cues(&text)?;
    let (form, cues, not_mixed, ran, pan_law) =
        (list.form, list.cues, list.not_mixed, list.ran, list.pan);
    // The folder the cues' files are named from.
    let folder = match base {
        Some(b) => {
            let dir = root.folder(&format!("{prefix}base"), Some(b))?;
            root.show(&dir)
        }
        None if form == "duck_forge_log" => {
            duck_forge_folder_of(root, &path).map_or_else(String::new, |d| root.show(&d))
        }
        None => root.folder_of(&path),
    };
    let name = file_safe(
        &path
            .file_stem()
            .map_or("mix".into(), |s| s.to_string_lossy().into_owned()),
    );
    let out_dir: PathBuf = match out {
        Some(o) => root.write(&format!("{prefix}out"), o)?,
        None => path
            .parent()
            .map_or(root.dir().to_path_buf(), Path::to_path_buf),
    };

    // Every file once, in the order first named.
    let mut audio: BTreeMap<String, Audio> = BTreeMap::new();
    let mut report = Report::new();
    for cue in &cues {
        if audio.contains_key(&cue.file) {
            continue;
        }
        match load_audio(root, &folder, cue) {
            Ok(a) => {
                audio.insert(cue.file.clone(), a);
            }
            Err(e) => {
                for mut issue in e.into_report().errors {
                    let inner = if issue.path.is_empty() {
                        String::new()
                    } else {
                        format!(" ({}: {})", issue.path, issue.message)
                    };
                    issue.message =
                        format!("`{}` cannot be played: {}{inner}", cue.file, issue.message);
                    issue.path = format!("{}.file", cue.at);
                    report.push(issue);
                }
            }
        }
    }
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }

    // The length: given, or the end of the last one-shot, the last stop,
    // or how long a Duck Forge game ran (its audio.wav is that long, and
    // longer if a sound rings on).
    let sr = f64::from(MIX_RATE);
    let natural = cues
        .iter()
        .map(|c| match (c.looping, c.stop) {
            (false, _) => c.start + audio[&c.file].len(),
            (true, Some(stop)) => stop,
            (true, None) => c.start,
        })
        .chain(ran)
        .max()
        .unwrap_or(0) as f64
        / sr;
    let seconds = match length {
        Some(l) if l.is_finite() && l > 0.0 && l <= MAX_MIX_SECONDS => l,
        Some(l) => {
            return Err(ToolError::issue(
                Issue::error(
                    Code::OutOfRange,
                    format!("{prefix}length_seconds"),
                    format!("length_seconds is above 0 and at most {MAX_MIX_SECONDS}, got {l}"),
                )
                .got(l)
                .range(0.0, MAX_MIX_SECONDS),
            ))
        }
        None if natural > 0.0 => natural.min(MAX_MIX_SECONDS),
        None => {
            return Err(ToolError::issue(
                Issue::error(
                    Code::MissingField,
                    format!("{prefix}length_seconds"),
                    "every cue is a loop with no stop, so the mix has no end: give its length",
                )
                .suggest("60"),
            ))
        }
    };
    let frames = (seconds * sr).round() as usize;
    let placed: Vec<Placed> = cues
        .iter()
        .map(|c| {
            let a = &audio[&c.file];
            let start = c.start;
            let end = match (c.looping, c.stop) {
                (false, _) => start + a.len(),
                (true, Some(stop)) => stop,
                (true, None) => frames,
            }
            .min(frames);
            Placed {
                cue: c,
                audio: a,
                start: start.min(frames),
                end,
                gains: pan_law.gains(c.pan),
            }
        })
        .collect();
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for p in &placed {
        for f in p.start..p.end {
            let [l, r] = p.at(f);
            left[f] += l;
            right[f] += r;
        }
    }

    let overs = find_overs(&left, &right, &placed);
    let kw_left = duck_echo_analyze::k_weighted(&left, MIX_RATE);
    let kw_right = duck_echo_analyze::k_weighted(&right, MIX_RATE);
    let cue_reports = cue_reports(&placed, &kw_left, &kw_right);
    let short_term = short_term(&kw_left, &kw_right);
    let sections = section_reports(
        &left,
        &right,
        seconds,
        sections,
        &format!("{prefix}sections"),
    )?;
    let whole = duck_echo_analyze::loudness(&[&left, &right], MIX_RATE).map(|(l, _)| l);
    let peak = left
        .iter()
        .chain(&right)
        .fold(0.0f32, |m, s| m.max(s.abs()));
    let peak_dbfs = (peak > 0.0).then(|| round1(20.0 * peak.log10()));

    std::fs::create_dir_all(&out_dir).map_err(|e| {
        ToolError::failed(
            Code::Io,
            format!("cannot make {}: {e}", root.show(&out_dir)),
        )
    })?;
    let wav = out_dir.join(format!("{name}.mix.wav"));
    let png = out_dir.join(format!("{name}.mix.png"));
    let json = out_dir.join(format!("{name}.mix.json"));
    let interleaved: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(l, r)| [*l, *r])
        .collect();
    // Float samples, so a mix over full volume is written as it is.
    crate::write_wav_file(
        &wav,
        &AudioBuffer {
            sample_rate: MIX_RATE,
            channels: 2,
            samples: interleaved,
        },
        duck_echo_audio::WavFormat::Float32,
    )?;
    let summary = summary_line(seconds, whole, peak_dbfs, &overs, &cue_reports, &not_mixed);
    crate::save::write_atomic(
        &png,
        &picture(
            &name,
            &summary,
            &left,
            &right,
            &short_term,
            &placed,
            &overs,
            &cue_reports,
            &sections,
        ),
    )?;
    let checked = MixChecked {
        ok: true,
        kind: "duck-echo-mix-check",
        name,
        form,
        length_seconds: seconds,
        sample_rate: MIX_RATE,
        mix_wav: root.show(&wav),
        picture_png: root.show(&png),
        report_json: root.show(&json),
        loudness_lufs: whole.map(round1),
        peak_dbfs,
        summary,
        overs,
        cues: cue_reports,
        not_mixed,
        short_term_lufs: short_term,
        sections,
    };
    let text = serde_json::to_string_pretty(&checked).expect("plain JSON") + "\n";
    crate::save::write_atomic(&json, text.as_bytes())?;
    Ok(checked)
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Seconds rounded to the millisecond.
fn ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn db(level: f32) -> Option<f32> {
    (level > 0.0).then(|| round1(20.0 * level.log10()))
}

/// Every moment over full volume, with what is behind it and the fixes.
fn find_overs(left: &[f32], right: &[f32], placed: &[Placed]) -> Vec<Over> {
    let sr = f64::from(MIX_RATE);
    let join = (OVER_JOIN_SECONDS * sr) as usize;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (f, (l, r)) in left.iter().zip(right).enumerate() {
        if l.abs() > 1.0 || r.abs() > 1.0 {
            match spans.last_mut() {
                Some(last) if f <= last.1 + join => last.1 = f + 1,
                _ => spans.push((f, f + 1)),
            }
        }
    }
    let mut overs = Vec::new();
    for (a, b) in spans {
        let sides = [left, right];
        let peak = (a..b)
            .flat_map(|f| [left[f].abs(), right[f].abs()])
            .fold(0.0f32, f32::max);
        let mut parts: Vec<(&Placed, f32)> = placed
            .iter()
            .filter(|p| p.start < b && p.end > a)
            .map(|p| {
                let own = (a..b)
                    .flat_map(|f| p.at(f).map(f32::abs))
                    .fold(0.0f32, f32::max);
                (p, own)
            })
            .filter(|(_, own)| *own > 0.0)
            .collect();
        parts.sort_by(|x, y| y.1.total_cmp(&x.1));
        let factor = FIX_PEAK / peak;
        // A loop's volume can move: its level where the moment starts.
        let at = a;
        let all_down = parts
            .iter()
            .map(|(p, _)| VolumeChange {
                name: p.cue.name.clone(),
                from: round3(p.cue.level(at)),
                to: round3(p.cue.level(at) * factor),
            })
            .collect();
        // The loudest alone: the smallest cut of it that keeps every
        // sample of the moment under the fix peak, if there is one.
        let loudest_alone = parts.first().and_then(|(p, _)| {
            let worst = |cut: f32| {
                (a..b)
                    .map(|f| {
                        let own = p.at(f);
                        sides
                            .iter()
                            .zip(own)
                            .map(|(s, o)| (s[f] - o * (1.0 - cut)).abs())
                            .fold(0.0f32, f32::max)
                    })
                    .fold(0.0f32, f32::max)
            };
            if worst(0.0) > FIX_PEAK {
                return None;
            }
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                if worst(mid) <= FIX_PEAK {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            Some(VolumeChange {
                name: p.cue.name.clone(),
                from: round3(p.cue.level(at)),
                to: round3(p.cue.level(at) * lo),
            })
        });
        overs.push(Over {
            start_seconds: ms(a as f64 / sr),
            end_seconds: ms(b as f64 / sr),
            peak_dbfs: round1(20.0 * peak.log10()),
            sounds: parts
                .iter()
                .map(|(p, own)| OverPart {
                    name: p.cue.name.clone(),
                    start_seconds: ms(p.cue.start_seconds()),
                    volume: round3(p.cue.level(at)),
                    peak_dbfs: db(*own),
                })
                .collect(),
            all_down_db: round1(-20.0 * factor.log10()),
            all_down,
            loudest_alone,
        });
    }
    overs
}

/// Mean square of K-weighted sides over a frame range, summed (BS.1770).
fn mean_square(l: &[f64], r: &[f64]) -> f64 {
    let n = l.len().max(1) as f64;
    (l.iter().map(|x| x * x).sum::<f64>() + r.iter().map(|x| x * x).sum::<f64>()) / n
}

fn lufs(ms: f64) -> Option<f32> {
    (ms > 1e-12).then(|| round1((-0.691 + 10.0 * ms.log10()) as f32))
}

/// Every cue's own loudness against the rest of the mix where it sounds.
fn cue_reports(placed: &[Placed], kw_l: &[f64], kw_r: &[f64]) -> Vec<CueReport> {
    let sr = f64::from(MIX_RATE);
    placed
        .iter()
        .map(|p| {
            let n = p.end.saturating_sub(p.start);
            let own: Vec<[f32; 2]> = (p.start..p.end).map(|f| p.at(f)).collect();
            let ol: Vec<f32> = own.iter().map(|s| s[0]).collect();
            let or: Vec<f32> = own.iter().map(|s| s[1]).collect();
            let kl = duck_echo_analyze::k_weighted(&ol, MIX_RATE);
            let kr = duck_echo_analyze::k_weighted(&or, MIX_RATE);
            // Its sounding part: from its first to its last sample above
            // -60 dBFS.
            let loud = |i: &usize| own[*i][0].abs().max(own[*i][1].abs()) > 0.001;
            let first = (0..n).find(loud);
            let last = (0..n).rev().find(loud);
            let (mut cue_lufs, mut rest_lufs) = (None, None);
            if let (Some(a), Some(b)) = (first, last) {
                let range = a..b + 1;
                cue_lufs = lufs(mean_square(&kl[range.clone()], &kr[range.clone()]));
                let rest_l: Vec<f64> = range.clone().map(|i| kw_l[p.start + i] - kl[i]).collect();
                let rest_r: Vec<f64> = range.clone().map(|i| kw_r[p.start + i] - kr[i]).collect();
                rest_lufs = lufs(mean_square(&rest_l, &rest_r));
            }
            let buried = !p.cue.looping
                && matches!((cue_lufs, rest_lufs), (Some(c), Some(r)) if c < r - BURIED_DB);
            let (under, lift_to) = if buried {
                let others = placed
                    .iter()
                    .filter(|o| !std::ptr::eq(*o, p) && o.start < p.end && o.end > p.start)
                    .map(|o| o.cue.name.clone())
                    .collect();
                let (c, r) = (cue_lufs.unwrap_or(0.0), rest_lufs.unwrap_or(0.0));
                let lift = p.cue.volume_now() * 10f32.powf((r - 10.0 - c) / 20.0);
                (others, (lift <= MAX_VOLUME).then(|| round3(lift)))
            } else {
                (Vec::new(), None)
            };
            CueReport {
                name: p.cue.name.clone(),
                file: p.cue.file.clone(),
                start_seconds: ms(p.cue.start_seconds()),
                end_seconds: ms(p.end as f64 / sr),
                looping: p.cue.looping,
                volume: p.cue.volume_now(),
                loudness_lufs: cue_lufs,
                rest_lufs,
                buried,
                under,
                lift_to,
            }
        })
        .collect()
}

/// Short-term loudness at the end of each second: the 3 s before it.
fn short_term(kw_l: &[f64], kw_r: &[f64]) -> Vec<Option<f32>> {
    let sr = MIX_RATE as usize;
    let seconds = kw_l.len().div_ceil(sr);
    let window = (SHORT_TERM_SECONDS * sr as f64) as usize;
    (1..=seconds)
        .map(|s| {
            let end = (s * sr).min(kw_l.len());
            let start = end.saturating_sub(window);
            lufs(mean_square(&kw_l[start..end], &kw_r[start..end]))
        })
        .collect()
}

/// The loudness of each section: those given, or every 10 s.
fn section_reports(
    left: &[f32],
    right: &[f32],
    seconds: f64,
    given: Option<&[Section]>,
    key: &str,
) -> Result<Vec<SectionReport>, ToolError> {
    let sections: Vec<Section> = match given {
        Some(list) => {
            let mut report = Report::new();
            for (i, s) in list.iter().enumerate() {
                if !(s.start >= 0.0 && s.end > s.start && s.start < seconds) {
                    report.push(
                        Issue::error(
                            Code::OutOfRange,
                            format!("{key}[{i}]"),
                            format!(
                                "a section starts at 0 or later, before the mix ends ({seconds:.2} s), and ends after it starts; got {} to {}",
                                s.start, s.end
                            ),
                        ),
                    );
                }
            }
            if !report.is_ok() {
                return Err(ToolError::Invalid(report));
            }
            list.to_vec()
        }
        None => {
            let count = (seconds / DEFAULT_SECTION_SECONDS).ceil().max(1.0) as usize;
            (0..count)
                .map(|i| {
                    let start = i as f64 * DEFAULT_SECTION_SECONDS;
                    let end = (start + DEFAULT_SECTION_SECONDS).min(seconds);
                    Section {
                        name: format!("{start:.0}-{end:.0} s"),
                        start,
                        end,
                    }
                })
                .collect()
        }
    };
    let sr = f64::from(MIX_RATE);
    Ok(sections
        .into_iter()
        .map(|s| {
            let a = ((s.start * sr) as usize).min(left.len());
            let b = ((s.end * sr) as usize).min(left.len()).max(a);
            let peak = left[a..b]
                .iter()
                .chain(&right[a..b])
                .fold(0.0f32, |m, x| m.max(x.abs()));
            SectionReport {
                loudness_lufs: duck_echo_analyze::loudness(&[&left[a..b], &right[a..b]], MIX_RATE)
                    .map(|(l, _)| round1(l)),
                peak_dbfs: db(peak),
                name: s.name,
                start_seconds: s.start,
                end_seconds: s.end,
            }
        })
        .collect())
}

fn summary_line(
    seconds: f64,
    loudness: Option<f32>,
    peak: Option<f32>,
    overs: &[Over],
    cues: &[CueReport],
    not_mixed: &[NotMixed],
) -> String {
    let mut parts = vec![format!(
        "{seconds:.1} s, {}, peak {}",
        loudness.map_or("silent".into(), |l| format!("{l:.1} LUFS")),
        peak.map_or("none".into(), |p| format!("{p:+.1} dBFS"))
    )];
    if overs.is_empty() {
        parts.push("never over full volume".into());
    } else {
        let list: Vec<String> = overs
            .iter()
            .take(5)
            .map(|o| {
                let names: Vec<&str> = o.sounds.iter().take(3).map(|s| s.name.as_str()).collect();
                format!("at {:.2} s ({})", o.start_seconds, names.join(" + "))
            })
            .collect();
        let more = if overs.len() > 5 {
            format!(" and {} more", overs.len() - 5)
        } else {
            String::new()
        };
        parts.push(format!(
            "{} moment{} over full volume: {}{more}",
            overs.len(),
            if overs.len() == 1 { "" } else { "s" },
            list.join(", ")
        ));
    }
    let buried: Vec<String> = cues
        .iter()
        .filter(|c| c.buried)
        .map(|c| {
            let under = match (c.loudness_lufs, c.rest_lufs) {
                (Some(a), Some(b)) => format!(", {:.0} dB under", b - a),
                _ => String::new(),
            };
            format!("{} at {:.2} s{under}", c.name, c.start_seconds)
        })
        .collect();
    if buried.is_empty() {
        parts.push("no cue buried".into());
    } else {
        parts.push(format!("buried: {}", buried.join(", ")));
    }
    if !not_mixed.is_empty() {
        // Each name once, in the order first played.
        let mut names: Vec<&str> = Vec::new();
        for n in not_mixed {
            if !names.contains(&n.name.as_str()) {
                names.push(&n.name);
            }
        }
        parts.push(format!(
            "not in the mix: {} of Duck Forge's own synthesizer sounds ({})",
            not_mixed.len(),
            names.join(", ")
        ));
    }
    parts.join("; ")
}

// ---------------------------------------------------------------------
// The picture.
// ---------------------------------------------------------------------

const PIC_W: usize = 1600;
const PIC_LEFT: usize = 44;
const PIC_RIGHT: usize = 16;
const PIC_TITLE: usize = 34;
const LEVEL_H: usize = 150;
const LANE_H: usize = 14;
const MAX_LANES: usize = 16;
const AXIS_H: usize = 34;

/// The mix: its level over time (peak light blue, short-term loudness
/// dark blue, over full volume in red) and below it every cue on its own
/// lane, a tick and its name at its start, a bar for a loop; buried cues
/// in red. Sections and seconds along the bottom.
#[allow(clippy::too_many_arguments)]
fn picture(
    name: &str,
    summary: &str,
    left: &[f32],
    right: &[f32],
    short_term: &[Option<f32>],
    placed: &[Placed],
    overs: &[Over],
    cues: &[CueReport],
    sections: &[SectionReport],
) -> Vec<u8> {
    let plot_w = PIC_W - PIC_LEFT - PIC_RIGHT;
    let frames = left.len().max(1);
    let seconds = frames as f64 / f64::from(MIX_RATE);
    let x_of = |t: f64| {
        PIC_LEFT + ((t / seconds) * plot_w as f64).clamp(0.0, (plot_w - 1) as f64) as usize
    };

    // Lanes: each cue goes in the first lane where its name fits.
    let mut lane_ends: Vec<usize> = Vec::new();
    let mut lanes = Vec::with_capacity(placed.len());
    for (p, c) in placed.iter().zip(cues) {
        let x = x_of(p.cue.start_seconds());
        let label_end = x + 4 + text_width(&c.name, 1);
        let bar_end = if p.cue.looping {
            x_of(p.end as f64 / f64::from(MIX_RATE))
        } else {
            x
        };
        let end = label_end.max(bar_end) + 6;
        let lane = lane_ends.iter().position(|&e| e <= x).unwrap_or_else(|| {
            if lane_ends.len() < MAX_LANES {
                lane_ends.push(0);
                lane_ends.len() - 1
            } else {
                // Full: the lane that frees up first.
                (0..lane_ends.len())
                    .min_by_key(|&i| lane_ends[i])
                    .unwrap_or(0)
            }
        });
        lane_ends[lane] = end;
        lanes.push(lane);
    }
    let lanes_h = lane_ends.len().max(1) * LANE_H + 8;
    let level_top = PIC_TITLE;
    let lanes_top = level_top + LEVEL_H + 8;
    let axis_top = lanes_top + lanes_h;
    let height = axis_top + AXIS_H;
    let mut b = Board::new(PIC_W, height);

    b.text(PIC_LEFT, 6, name, 2, Ink::Text, plot_w / (CHAR_W * 2));
    b.text(
        PIC_LEFT,
        6 + CHAR_H * 2 + 4,
        summary,
        1,
        Ink::Text,
        plot_w / CHAR_W,
    );

    // Level panel: -60 dBFS at the bottom, +6 at the top.
    let (floor, top) = (-60.0f32, 6.0f32);
    let y_of = |db: f32| {
        let t = ((top - db) / (top - floor)).clamp(0.0, 1.0);
        level_top + (t * (LEVEL_H - 1) as f32) as usize
    };
    b.fill(PIC_LEFT, level_top, plot_w, LEVEL_H, Ink::Panel);
    for db in [-20.0, -40.0] {
        b.fill(PIC_LEFT, y_of(db), plot_w, 1, Ink::Grid);
    }
    for x in 0..plot_w {
        let a = x * frames / plot_w;
        let z = ((x + 1) * frames / plot_w).max(a + 1).min(left.len());
        if a >= z {
            continue;
        }
        let peak = left[a..z]
            .iter()
            .chain(&right[a..z])
            .fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > 0.0 {
            let y = y_of(20.0 * peak.log10());
            let ink = if peak > 1.0 { Ink::Problem } else { Ink::Peak };
            b.fill(PIC_LEFT + x, y, 1, level_top + LEVEL_H - y, ink);
        }
    }
    // Full volume.
    b.fill(PIC_LEFT, y_of(0.0), plot_w, 1, Ink::Problem);
    // Short-term loudness, a step a second.
    for (s, l) in short_term.iter().enumerate() {
        if let Some(l) = l {
            let (x0, x1) = (x_of(s as f64), x_of((s + 1) as f64));
            b.fill(x0, y_of(*l), (x1 - x0).max(1), 2, Ink::Rms);
        }
    }
    for o in overs {
        let (x0, x1) = (x_of(o.start_seconds), x_of(o.end_seconds));
        b.fill(x0, level_top, (x1 - x0).max(2), 6, Ink::Problem);
    }
    b.frame(
        PIC_LEFT - 1,
        level_top - 1,
        plot_w + 2,
        LEVEL_H + 2,
        Ink::Text,
    );
    for (db, label) in [(0.0, "0"), (-20.0, "-20"), (-40.0, "-40")] {
        let w = text_width(label, 1);
        b.text(
            PIC_LEFT - 4 - w,
            y_of(db).saturating_sub(3),
            label,
            1,
            Ink::Text,
            4,
        );
    }

    // Sections: a line at each start, the name under the axis.
    for s in sections {
        let x = x_of(s.start_seconds);
        b.fill(x, level_top, 1, axis_top - level_top, Ink::Grid);
    }
    // Cues.
    for ((p, c), lane) in placed.iter().zip(cues).zip(&lanes) {
        let x = x_of(p.cue.start_seconds());
        let y = lanes_top + lane * LANE_H;
        let ink = if c.buried { Ink::Problem } else { Ink::Rms };
        b.fill(x, level_top + LEVEL_H - 10, 1, 10, ink);
        b.fill(x, y, 2, LANE_H - 3, ink);
        if p.cue.looping {
            let end = x_of(p.end as f64 / f64::from(MIX_RATE));
            b.fill(
                x,
                y + LANE_H - 5,
                end.saturating_sub(x).max(2),
                2,
                Ink::Peak,
            );
        }
        b.text(
            x + 4,
            y + 2,
            &c.name,
            1,
            if c.buried { Ink::Problem } else { Ink::Text },
            40,
        );
    }

    // Seconds and sections along the bottom.
    let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0]
        .into_iter()
        .find(|s| seconds / s <= 24.0)
        .unwrap_or(300.0);
    let mut t = 0.0;
    while t <= seconds + 1e-9 {
        let x = x_of(t);
        b.fill(x, axis_top, 1, 4, Ink::Text);
        // The unit once, on the first tick.
        let (label, left) = if t == 0.0 {
            ("0 s".to_string(), x)
        } else {
            let label = format!("{t:.0}");
            let left = x.saturating_sub(text_width(&label, 1) / 2);
            (label, left)
        };
        b.text(left, axis_top + 6, &label, 1, Ink::Text, 6);
        t += step;
    }
    for s in sections {
        let x = x_of(s.start_seconds) + 2;
        let room = x_of(s.end_seconds).saturating_sub(x) / CHAR_W;
        let words = match s.loudness_lufs {
            Some(l) => format!("{} {l:.1}", s.name),
            None => s.name.clone(),
        };
        b.text(x, axis_top + 18, &words, 1, Ink::Rms, room.max(1));
    }
    b.into_png()
}
