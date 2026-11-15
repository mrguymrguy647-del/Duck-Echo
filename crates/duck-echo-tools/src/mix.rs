//! Check a mix from its cue list (`mix_check`, `duck-echo mix`).
//!
//! A cue list says which sound plays when, how loud and where. It is read
//! in either of two forms:
//!
//! * Duck Forge's `log.json`, as Duck Forge writes it:
//!   `{"updates_per_second": 60, "sounds": [{"frame", "sound", "file",
//!   "volume", "pan"}], "loops": [{"loop", "file", "start", "stop",
//!   "volume": [[frame, level], ...]}]}`, times in frames;
//! * a plain list, times in seconds:
//!   `[{"time": 1.25, "file": "sfx/door.wav", "name": "door",
//!   "volume": 0.8, "pan": 0}]`, where `"loop": true` (with `"stop"`)
//!   plays the file over and over.
//!
//! A cue's `file` is a WAV, a recipe or a song (rendered first) or
//! `preset:NAME`, relative to the cue list's folder. The check mixes them
//! at 44.1 kHz in stereo (equal-power pan, as Duck Echo's own stereo) and
//! reports:
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

/// Duck Forge's `log.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgeLog {
    /// Frames per second of the game: times are frame numbers.
    pub updates_per_second: f64,
    /// One-shot sounds.
    #[serde(default)]
    pub sounds: Vec<ForgeSound>,
    /// Loops.
    #[serde(default)]
    pub loops: Vec<ForgeLoop>,
}

/// One sound in Duck Forge's log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgeSound {
    /// The frame it starts on.
    pub frame: f64,
    /// Its name.
    pub sound: String,
    /// The file it plays.
    pub file: String,
    /// Its volume, linear (1 is as the file is). Default 1.
    #[serde(default = "one")]
    pub volume: f32,
    /// Its pan, -1 (left) to 1 (right). Default 0.
    #[serde(default)]
    pub pan: f32,
}

/// One loop in Duck Forge's log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgeLoop {
    /// Its name.
    #[serde(rename = "loop")]
    pub name: String,
    /// The file it plays over and over.
    pub file: String,
    /// The frame it starts on.
    pub start: f64,
    /// The frame it stops on; `null` or left out: the end of the mix.
    #[serde(default)]
    pub stop: Option<f64>,
    /// Its volume over time, `[[frame, level], ...]` in frame order, the
    /// level linear and straight lines between the points; before the
    /// first point it is at the first level. Left out: 1.
    #[serde(default)]
    pub volume: Vec<[f64; 2]>,
}

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

fn one() -> f32 {
    1.0
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

/// One cue, whichever form it came from.
#[derive(Debug, Clone, PartialEq)]
struct Cue {
    /// Where it is in the cue list, for errors: `sounds[3]`, `[3]`.
    at: String,
    name: String,
    file: String,
    start: f64,
    stop: Option<f64>,
    looping: bool,
    /// (seconds, level) points; empty is 1.
    volume: Vec<(f64, f32)>,
    pan: f32,
}

impl Cue {
    /// Its level at `t` seconds.
    fn level(&self, t: f64) -> f32 {
        let v = &self.volume;
        match v.len() {
            0 => 1.0,
            _ if t <= v[0].0 => v[0].1,
            _ => {
                for w in v.windows(2) {
                    if t <= w[1].0 {
                        let span = w[1].0 - w[0].0;
                        let f = if span > 0.0 { (t - w[0].0) / span } else { 1.0 };
                        return w[0].1 + (w[1].1 - w[0].1) * f as f32;
                    }
                }
                v[v.len() - 1].1
            }
        }
    }

    /// Its volume as one number: the first level (a one-shot has only
    /// one).
    fn volume_now(&self) -> f32 {
        self.volume.first().map_or(1.0, |p| p.1)
    }
}

/// Read a cue list's text: Duck Forge's log (an object) or a plain list.
fn parse_cues(text: &str) -> Result<Vec<Cue>, ToolError> {
    let tree = duck_echo_core::tree::parse_json(text)
        .map_err(|issue| ToolError::Invalid(Report::from_error(*issue)))?;
    let is_list = matches!(tree.value, duck_echo_core::tree::Value::Array(_));
    let schema = if is_list {
        serde_json::to_value(schemars::schema_for!(Vec<PlainCue>)).expect("schema")
    } else {
        serde_json::to_value(schemars::schema_for!(ForgeLog)).expect("schema")
    };
    let checked = duck_echo_core::structure::check_against(&tree, &schema);
    let mut report = Report::new();
    for issue in checked.issues {
        report.push(issue);
    }
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    let value = checked.pruned.unwrap_or_default();
    let name_of = |file: &str| {
        Path::new(file)
            .file_stem()
            .map_or_else(|| file.to_string(), |s| s.to_string_lossy().into_owned())
    };
    let mut cues = Vec::new();
    if is_list {
        let list: Vec<PlainCue> = serde_json::from_value(value).map_err(|e| {
            ToolError::failed(Code::Invalid, format!("the cue list does not decode: {e}"))
        })?;
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
            cues.push(Cue {
                name: c.name.unwrap_or_else(|| name_of(&c.file)),
                file: c.file,
                start: c.time,
                stop: c.stop,
                looping,
                volume: vec![(c.time, c.volume.unwrap_or(1.0))],
                pan: c.pan.unwrap_or(0.0),
                at,
            });
        }
    } else {
        let log: ForgeLog = serde_json::from_value(value).map_err(|e| {
            ToolError::failed(Code::Invalid, format!("the log does not decode: {e}"))
        })?;
        let fps = log.updates_per_second;
        if !(fps.is_finite() && fps > 0.0) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    "updates_per_second",
                    format!("updates_per_second must be above 0, got {fps}"),
                )
                .got(fps)
                .suggest("60"),
            );
            return Err(ToolError::Invalid(report));
        }
        for (i, s) in log.sounds.into_iter().enumerate() {
            cues.push(Cue {
                at: format!("sounds[{i}]"),
                name: s.sound,
                file: s.file,
                start: s.frame / fps,
                stop: None,
                looping: false,
                volume: vec![(s.frame / fps, s.volume)],
                pan: s.pan,
            });
        }
        for (i, l) in log.loops.into_iter().enumerate() {
            let at = format!("loops[{i}]");
            for (j, w) in l.volume.windows(2).enumerate() {
                if w[1][0] < w[0][0] {
                    report.push(
                        Issue::error(
                            Code::Invalid,
                            format!("{at}.volume[{}]", j + 1),
                            "the volume points must be in frame order",
                        )
                        .got(w[1][0]),
                    );
                }
            }
            cues.push(Cue {
                at,
                name: l.name,
                file: l.file,
                start: l.start / fps,
                stop: l.stop.map(|f| f / fps),
                looping: true,
                volume: l.volume.iter().map(|p| (p[0] / fps, p[1] as f32)).collect(),
                pan: 0.0,
            });
        }
    }
    for c in &cues {
        if !(c.start.is_finite() && c.start >= 0.0) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    format!("{}.time", c.at),
                    "a cue starts at 0 seconds or later",
                )
                .got(c.start),
            );
        }
        if let Some(stop) = c.stop {
            if stop < c.start {
                report.push(
                    Issue::error(
                        Code::OutOfRange,
                        format!("{}.stop", c.at),
                        format!(
                            "a loop stops after it starts ({:.3} s), got {stop:.3} s",
                            c.start
                        ),
                    )
                    .got(stop),
                );
            }
        }
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
    if cues.len() > MAX_CUES {
        report.push(Issue::error(
            Code::Invalid,
            "",
            format!(
                "the cue list holds {} cues; one check mixes at most {MAX_CUES}",
                cues.len()
            ),
        ));
    }
    if cues.is_empty() {
        report.push(Issue::error(Code::Invalid, "", "the cue list has no cues"));
    }
    if report.is_ok() {
        Ok(cues)
    } else {
        Err(ToolError::Invalid(report))
    }
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

/// Equal-power gains for `pan`, as Duck Echo's own stereo: a centred
/// sound is -3 dB on each side, so it is as loud as in mono.
fn pan_gains(pan: f32) -> [f32; 2] {
    [
        math::sin_turns((1.0 - pan) * 0.125),
        math::sin_turns((1.0 + pan) * 0.125),
    ]
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
        let level = self.cue.level(frame as f64 / f64::from(MIX_RATE));
        [
            self.audio.left[k] * level * self.gains[0],
            self.audio.right[k] * level * self.gains[1],
        ]
    }
}

/// Mix `cues_file` (relative to the project folder) and check it.
/// `sections` are given in seconds; `length` sets the mix's length (else
/// the end of the last sound, or the last loop's stop). `out` is the
/// folder for the mix, the report and the picture (default the cue
/// list's own). Arguments in errors are `<prefix>cues` and so on.
pub fn check(
    root: &Root,
    cues_file: &str,
    sections: Option<&[Section]>,
    length: Option<f64>,
    out: Option<&str>,
    prefix: &str,
) -> Result<MixChecked, ToolError> {
    let key = format!("{prefix}cues");
    let path = root.read(&key, cues_file)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ToolError::failed(Code::Io, format!("cannot read {cues_file}: {e}")))?;
    let form = if text.trim_start().starts_with('[') {
        "plain_list"
    } else {
        "duck_forge_log"
    };
    let mut cues = parse_cues(&text)?;
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    let folder = root.folder_of(&path);
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

    // The length: given, or the end of the last one-shot, or the last stop.
    let sr = f64::from(MIX_RATE);
    let natural = cues
        .iter()
        .map(|c| match (c.looping, c.stop) {
            (false, _) => c.start + audio[&c.file].len() as f64 / sr,
            (true, Some(stop)) => stop,
            (true, None) => c.start,
        })
        .fold(0.0, f64::max);
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
            let start = (c.start * sr).round() as usize;
            let end = match (c.looping, c.stop) {
                (false, _) => start + a.len(),
                (true, Some(stop)) => (stop * sr).round() as usize,
                (true, None) => frames,
            }
            .min(frames);
            Placed {
                cue: c,
                audio: a,
                start: start.min(frames),
                end,
                gains: pan_gains(c.pan),
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
    let summary = summary_line(seconds, whole, peak_dbfs, &overs, &cue_reports);
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
        let at = a as f64 / sr;
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
                    start_seconds: ms(p.cue.start),
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
                start_seconds: ms(p.cue.start),
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
        let x = x_of(p.cue.start);
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
        let x = x_of(p.cue.start);
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
