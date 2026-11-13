//! Song types: music written as text.
//!
//! A song is a second kind of document, next to the recipe. It holds a
//! tempo, some tracks, and notes on those tracks. Every note plays an
//! **instrument**, and an instrument is just a [`Recipe`].
//!
//! This module has two jobs:
//!
//! * The types ([`Song`], [`Track`], [`Note`], ...). TOML files and JSON
//!   tool calls both turn into these, so there is one schema.
//! * [`arrange`]: check a song and flatten it into a plain list of
//!   [`Event`]s per track (patterns placed, chords split, bars turned into
//!   beats). The renderer and the analyzer both work from that list, so
//!   they always agree on what was written.
//!
//! Nothing here makes sound. See `duck_echo_synth::song` for that.

use crate::load::did_you_mean;
use crate::note;
use crate::recipe::{Effect, Master, Recipe, CURRENT_VERSION, DEFAULT_SAMPLE_RATE};
use crate::report::{Code, Issue, Report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest song, in seconds (10 minutes).
pub const MAX_SONG_SECONDS: f64 = 600.0;
/// Most notes in one song, after patterns are placed and chords split.
pub const MAX_SONG_NOTES: usize = 20_000;
/// Most tracks in one song.
pub const MAX_TRACKS: usize = 64;

fn default_beats_per_bar() -> u32 {
    4
}
fn default_sample_rate() -> u32 {
    DEFAULT_SAMPLE_RATE
}
fn default_channels() -> u16 {
    2
}
fn default_one() -> f32 {
    1.0
}

/// A song: tracks of notes played by instruments. This is the root document
/// of a song `.toml` file or JSON tool call. A document with `tracks` is a
/// song; a document with `layers` is a recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Song {
    /// Schema version. Must be `0` for this version of Duck Echo. Required.
    pub version: u32,

    /// Optional name for humans and agents. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Optional one-sentence description. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional tags for search. Not used by the renderer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Random seed. Same song + same seed = byte-identical audio. Each
    /// track and each note gets its own stream from it. Default `0`.
    #[serde(default)]
    pub seed: u64,

    /// Speed in beats per minute, range 20..=400. Required.
    pub tempo: f64,

    /// Beats in one bar, range 1..=16. Default `4`.
    #[serde(default = "default_beats_per_bar")]
    pub beats_per_bar: u32,

    /// Length of the song in bars. When omitted, the song ends at the bar
    /// line after its last note. Sound still ringing (releases, reverb)
    /// plays on after the last bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<u32>,

    /// Output sample rate in Hz. Range 8000..=192000. Default `44100`.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Output channels: `2` (stereo, default) or `1` (mono).
    #[serde(default = "default_channels")]
    pub channels: u16,

    /// Master output stage for the whole song, the same table as in a
    /// recipe. Applied after the song `effects`.
    #[serde(default)]
    pub master: Master,

    /// Song-wide effect chain, applied in order to the mix of all tracks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,

    /// The song's own instruments, by name. Each is a full recipe. A track
    /// uses one by writing its name as `instrument`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub instruments: BTreeMap<String, Recipe>,

    /// The song's own drum kits, by name. A kit maps piece names (such as
    /// `kick`) to instruments: a name from `instruments`, a built-in
    /// instrument, `preset:NAME`, or a recipe file.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kits: BTreeMap<String, BTreeMap<String, String>>,

    /// Named groups of notes, placed on tracks with `play`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub patterns: BTreeMap<String, Pattern>,

    /// The tracks, mixed together. At least one is required.
    #[serde(default)]
    pub tracks: Vec<Track>,
}

/// One track: an instrument and the notes it plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// A short unique name, such as `"bass"`. Used in reports, by `solo`
    /// and `mute`, and to pick the track's seed stream. Required.
    pub name: String,

    /// The instrument: a name from the song's `instruments` or `kits`, a
    /// built-in instrument, `preset:NAME`, or a recipe file (a path ending
    /// in `.toml` or `.json`, relative to the song file). Required.
    pub instrument: String,

    /// Track level in the mix, range 0..=2. Default `1`.
    #[serde(default = "default_one")]
    pub volume: f32,

    /// Stereo position, -1 (left) ..= 1 (right). Default `0`.
    #[serde(default)]
    pub pan: f32,

    /// Notes written straight on the track.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,

    /// Patterns placed at bars. Added to `notes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub play: Vec<Placement>,
}

/// One note on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// The bar `beat` is counted from. Bars count from 1. Default `1`, so
    /// that `beat` alone counts from the start of the song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,

    /// When the note starts, in beats after the bar line. `0` is on the
    /// bar line, `0.5` half a beat later. Required.
    pub beat: f64,

    /// What to play: a note name such as `"C5"`, a number in Hz, a list of
    /// these for a chord, or on a kit track the name of a piece such as
    /// `"kick"`. Required.
    pub note: NoteValue,

    /// How long the note is held, in beats. Default `1`. Not used by kit
    /// pieces, which always play their whole sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,

    /// How hard the note is played, 0..=1. It multiplies the volume.
    /// Default `1`.
    #[serde(default = "default_one")]
    pub velocity: f32,
}

/// One note in a pattern. Like [`Note`], without `bar`: a pattern's beats
/// count from the bar it is placed at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatternNote {
    /// When the note starts, in beats after the start of the pattern.
    /// Beat `beats_per_bar` is the first beat of the next bar. Required.
    pub beat: f64,

    /// What to play: a note name, a number in Hz, a list of these for a
    /// chord, or on a kit track the name of a piece. Required.
    pub note: NoteValue,

    /// How long the note is held, in beats. Default `1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,

    /// How hard the note is played, 0..=1. Default `1`.
    #[serde(default = "default_one")]
    pub velocity: f32,
}

/// What a note plays: one pitch, or several at once (a chord).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NoteValue {
    /// One pitch or kit piece.
    One(NotePitch),
    /// Several at the same time.
    Chord(Vec<NotePitch>),
}

impl NoteValue {
    /// The pitches, one for a single note.
    pub fn pitches(&self) -> &[NotePitch] {
        match self {
            NoteValue::One(p) => std::slice::from_ref(p),
            NoteValue::Chord(list) => list,
        }
    }
}

/// One pitch: a frequency, or a name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NotePitch {
    /// A frequency in Hz, range 1..=20000.
    Hz(f64),
    /// A note name such as `"C5"`, or on a kit track a piece name such as
    /// `"kick"`.
    Name(String),
}

/// A named group of notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    /// The notes. At least one.
    pub notes: Vec<PatternNote>,
}

/// A pattern placed on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// The name of a pattern in `patterns`. Required.
    pub pattern: String,

    /// The bars the pattern starts at, such as `[1, 2, 4]`. Bars count
    /// from 1. Required.
    pub bars: Vec<u32>,

    /// Move every note up (or down, when negative) by this many semitones,
    /// range -48..=48. Default `0`. Not used by kit pieces.
    #[serde(default)]
    pub transpose: i32,
}

impl Song {
    /// Seconds in one beat.
    pub fn seconds_per_beat(&self) -> f64 {
        60.0 / self.tempo
    }

    /// The sample frame at `beat` beats from the start of the song.
    ///
    /// Only `*`, `/`, `+` and `floor` on 64-bit floats are used. Those give
    /// the same answer on every computer, so a note starts on the same
    /// sample everywhere.
    pub fn frame_at(&self, beat: f64) -> usize {
        let frames = beat * 60.0 * self.sample_rate as f64 / self.tempo;
        (frames + 0.5).floor().max(0.0) as usize
    }
}

/// What a track's `instrument` turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Instrument {
    /// One recipe, played at the pitch of each note.
    Sound(Box<Recipe>),
    /// A drum kit: piece name to recipe. Each piece plays as written.
    Kit(BTreeMap<String, Recipe>),
}

/// Where instruments that are not written in the song itself come from:
/// built-in instruments, presets and recipe files. The front ends (CLI, MCP
/// server) supply this, because only they know where files may be read.
pub trait InstrumentSource {
    /// Find the instrument `reference` names. The returned issue's path is
    /// left empty; [`arrange`] fills in the field that named it.
    fn find(&self, reference: &str) -> Result<Instrument, Box<Issue>>;
}

/// An [`InstrumentSource`] that knows no instruments. A song checked with
/// it must define every instrument itself.
pub struct NoInstruments;

impl InstrumentSource for NoInstruments {
    fn find(&self, reference: &str) -> Result<Instrument, Box<Issue>> {
        Err(Box::new(
            Issue::error(
                Code::NotFound,
                "",
                format!(
                    "instrument `{reference}` is not defined in the song's `instruments` or `kits`"
                ),
            )
            .got(reference),
        ))
    }
}

/// What one note plays, after checking.
#[derive(Debug, Clone, PartialEq)]
pub enum Sound {
    /// A pitch for a [`Instrument::Sound`].
    Pitch {
        /// Frequency in Hz, after `transpose`.
        hz: f32,
        /// The note name, when there is one (after `transpose`).
        name: Option<String>,
    },
    /// A piece of an [`Instrument::Kit`].
    Piece(String),
}

/// One note to play: a song's notes with patterns placed, chords split and
/// bars turned into beats.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Start, in beats from the start of the song.
    pub beat: f64,
    /// How long the note is held, in beats. Not used for kit pieces.
    pub length: f64,
    /// What it plays.
    pub sound: Sound,
    /// 0..=1.
    pub velocity: f32,
    /// The field the note was written in, such as `tracks[1].notes[3]` or
    /// `patterns.walk.notes[2]`.
    pub path: String,
    /// The bar the note starts in, counted from 1.
    pub bar: u32,
    /// The beat inside that bar, counted from 0.
    pub beat_in_bar: f64,
}

/// One track, ready to play.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPlan {
    /// The track's name.
    pub name: String,
    /// Its instrument.
    pub instrument: Instrument,
    /// Its notes, sorted by start time (ties keep the order written).
    pub events: Vec<Event>,
}

/// A checked song, flattened for the renderer and the analyzer.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement {
    /// Length in bars: the song's `bars`, or computed from the last note.
    pub bars: u32,
    /// One plan per track, in file order.
    pub tracks: Vec<TrackPlan>,
}

impl Arrangement {
    /// Notes in the whole song.
    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.events.len()).sum()
    }
}

/// The pitch an instrument recipe plays as written: the `pitch.start` of
/// its first layer that has a pitch (plain white noise has none; pitched
/// noise does). Notes move every
/// layer by `note / home`. `None` when no layer has a pitch; such an
/// instrument sounds the same for every note.
pub fn home_hz(recipe: &Recipe) -> Option<f32> {
    recipe
        .layers
        .iter()
        .find(|l| l.has_pitch())
        .map(|l| l.pitch.start.hz())
}

/// True for strings that name a recipe file rather than an instrument.
pub fn is_file_reference(reference: &str) -> bool {
    let lower = reference.to_ascii_lowercase();
    lower.ends_with(".toml") || lower.ends_with(".json")
}

/// Check a song and flatten it. Reports every problem at once. The
/// arrangement is `Some` exactly when the report has no errors.
pub fn arrange(song: &Song, source: &dyn InstrumentSource) -> (Report, Option<Arrangement>) {
    let mut a = Arranger {
        song,
        source,
        report: Report::new(),
        found: BTreeMap::new(),
    };
    let plan = a.run();
    let ok = a.report.is_ok();
    (a.report, plan.filter(|_| ok))
}

struct Arranger<'a> {
    song: &'a Song,
    source: &'a dyn InstrumentSource,
    report: Report,
    /// Instruments already looked up in `source`, so a name used by many
    /// tracks is found (and, for a file, read) once.
    found: BTreeMap<String, Result<Instrument, Box<Issue>>>,
}

impl Arranger<'_> {
    /// Add an error, unless the same one is already there. A pattern used
    /// in four bars would otherwise report its bad note four times.
    fn error(&mut self, issue: Issue) {
        let same = |i: &Issue| i.path == issue.path && i.message == issue.message;
        if !self.report.errors.iter().any(same) {
            self.report.push(issue);
        }
    }

    fn warn(&mut self, code: Code, path: &str, message: impl Into<String>) {
        self.report.push(Issue::warning(code, path, message));
    }

    fn range(&mut self, path: &str, value: f64, min: f64, max: f64) -> bool {
        if value.is_nan() || value < min || value > max {
            self.error(
                Issue::error(
                    Code::OutOfRange,
                    path,
                    format!("value {value} is outside the allowed range {min}..={max}"),
                )
                .got(value)
                .range(min, max),
            );
            false
        } else {
            true
        }
    }

    fn range_f32(&mut self, path: &str, value: f32, min: f32, max: f32) -> bool {
        let clean = crate::validate::f64_clean;
        self.range(path, clean(value), clean(min), clean(max))
    }

    fn run(&mut self) -> Option<Arrangement> {
        let song = self.song;
        if song.version != CURRENT_VERSION {
            self.error(
                Issue::error(
                    Code::UnsupportedVersion,
                    "version",
                    format!(
                        "schema version {} is not supported by this build (supports {})",
                        song.version, CURRENT_VERSION
                    ),
                )
                .got(song.version)
                .one_of([CURRENT_VERSION.to_string()]),
            );
        }
        let tempo_ok = self.range("tempo", song.tempo, 20.0, 400.0);
        let bpb_ok = self.range("beats_per_bar", song.beats_per_bar as f64, 1.0, 16.0);
        self.range("sample_rate", song.sample_rate as f64, 8000.0, 192_000.0);
        self.range("channels", song.channels as f64, 1.0, 2.0);
        if let Some(bars) = song.bars {
            if bars == 0 {
                self.error(
                    Issue::error(Code::OutOfRange, "bars", "a song is at least 1 bar long")
                        .got(bars),
                );
            }
        }
        let shared = crate::validate::validate_master_and_effects(
            &song.master,
            &song.effects,
            song.sample_rate,
        );
        self.report.extend(shared);

        self.own_instruments();

        if song.tracks.is_empty() {
            self.error(Issue::error(
                Code::Invalid,
                "tracks",
                "song has no tracks, so it would render silence; add at least one [[tracks]] entry",
            ));
        }
        if song.tracks.len() > MAX_TRACKS {
            self.error(
                Issue::error(
                    Code::OutOfRange,
                    "tracks",
                    format!(
                        "a song has at most {MAX_TRACKS} tracks, this one has {}",
                        song.tracks.len()
                    ),
                )
                .got(song.tracks.len())
                .range(1.0, MAX_TRACKS as f64),
            );
        }
        for (name, pattern) in &song.patterns {
            if pattern.notes.is_empty() {
                self.error(Issue::error(
                    Code::Invalid,
                    format!("patterns.{name}.notes"),
                    format!("pattern `{name}` has no notes; add at least one"),
                ));
            }
        }

        let mut plans = Vec::with_capacity(song.tracks.len());
        let mut used_patterns: Vec<&str> = Vec::new();
        for (i, track) in song.tracks.iter().enumerate() {
            let tp = format!("tracks[{i}]");
            if track.name.trim().is_empty() {
                self.error(Issue::error(
                    Code::Invalid,
                    format!("{tp}.name"),
                    "a track needs a name, such as \"bass\"",
                ));
            } else if let Some(first) = song.tracks[..i].iter().position(|t| t.name == track.name) {
                self.error(
                    Issue::error(
                        Code::Invalid,
                        format!("{tp}.name"),
                        format!(
                            "two tracks are named `{}` (tracks[{first}] and tracks[{i}]); track names must be different",
                            track.name
                        ),
                    )
                    .got(track.name.clone()),
                );
            }
            self.range_f32(&format!("{tp}.volume"), track.volume, 0.0, 2.0);
            self.range_f32(&format!("{tp}.pan"), track.pan, -1.0, 1.0);
            if song.channels == 1 && track.pan != 0.0 {
                self.warn(
                    Code::Unused,
                    &format!("{tp}.pan"),
                    "pan only affects stereo output; set `channels = 2` at the top level",
                );
            }
            let instrument = self.instrument_of(&track.instrument, &format!("{tp}.instrument"));
            if track.notes.is_empty() && track.play.is_empty() {
                self.warn(
                    Code::Suspicious,
                    &tp,
                    format!("track `{}` has no notes; add `notes` or `play`", track.name),
                );
            }
            for p in &track.play {
                used_patterns.push(&p.pattern);
            }
            // Notes are placed in beats, so they can be checked even when
            // the tempo is wrong; only a bad bar length stops that.
            if !bpb_ok {
                continue;
            }
            let events = self.events(i, track, instrument.as_ref());
            if let Some(instrument) = instrument {
                plans.push(TrackPlan {
                    name: track.name.clone(),
                    instrument,
                    events,
                });
            }
        }
        for name in song.patterns.keys() {
            if !used_patterns.contains(&name.as_str()) {
                self.warn(
                    Code::Unused,
                    &format!("patterns.{name}"),
                    format!("pattern `{name}` is not played by any track; place it with `play`"),
                );
            }
        }
        if !(tempo_ok && bpb_ok) {
            return None;
        }

        // Length: the bars written, or up to the bar line after the last
        // note ends.
        let bpb = song.beats_per_bar as f64;
        let last_end = plans
            .iter()
            .flat_map(|t| t.events.iter())
            .map(|e| match e.sound {
                Sound::Pitch { .. } => e.beat + e.length,
                Sound::Piece(_) => e.beat,
            })
            .fold(0.0f64, f64::max);
        let computed = ((last_end / bpb).ceil() as u32).max(1);
        let bars = song.bars.filter(|b| *b > 0).unwrap_or(computed);
        let seconds = bars as f64 * bpb * song.seconds_per_beat();
        if seconds > MAX_SONG_SECONDS {
            self.error(
                Issue::error(
                    Code::OutOfRange,
                    if song.bars.is_some() { "bars" } else { "tracks" },
                    format!(
                        "the song is {bars} bars, which is {seconds:.0} s at this tempo; the limit is {MAX_SONG_SECONDS} s"
                    ),
                )
                .got(seconds)
                .range(0.0, MAX_SONG_SECONDS),
            );
        }
        let notes: usize = plans.iter().map(|t| t.events.len()).sum();
        if notes > MAX_SONG_NOTES {
            self.error(
                Issue::error(
                    Code::OutOfRange,
                    "tracks",
                    format!(
                        "the song has {notes} notes after patterns are placed; the limit is {MAX_SONG_NOTES}"
                    ),
                )
                .got(notes)
                .range(0.0, MAX_SONG_NOTES as f64),
            );
        }
        Some(Arrangement {
            bars,
            tracks: plans,
        })
    }

    /// Check the song's own `instruments` and `kits`.
    fn own_instruments(&mut self) {
        let song = self.song;
        for (name, recipe) in &song.instruments {
            let base = format!("instruments.{name}");
            self.own_name(name, &base);
            if song.kits.contains_key(name) {
                self.error(Issue::error(
                    Code::Invalid,
                    &base,
                    format!(
                        "`{name}` is both an instrument and a kit; give one of them another name"
                    ),
                ));
            }
            let inner = crate::validate::validate(recipe);
            for mut issue in inner.errors.into_iter().chain(inner.warnings) {
                issue.path = if issue.path.is_empty() {
                    base.clone()
                } else {
                    format!("{base}.{}", issue.path)
                };
                self.report.push(issue);
            }
        }
        for (name, pieces) in &song.kits {
            let base = format!("kits.{name}");
            self.own_name(name, &base);
            if pieces.is_empty() {
                self.error(Issue::error(
                    Code::Invalid,
                    &base,
                    format!("kit `{name}` has no pieces; add at least one, such as kick = \"preset:thud\""),
                ));
            }
        }
        let used = |name: &str| {
            song.tracks.iter().any(|t| t.instrument == name)
                || song
                    .kits
                    .values()
                    .any(|pieces| pieces.values().any(|p| p == name))
        };
        for name in song.instruments.keys() {
            if !used(name) {
                self.warn(
                    Code::Unused,
                    &format!("instruments.{name}"),
                    format!("instrument `{name}` is not used by any track or kit"),
                );
            }
        }
        for name in song.kits.keys() {
            if !song.tracks.iter().any(|t| &t.instrument == name) {
                self.warn(
                    Code::Unused,
                    &format!("kits.{name}"),
                    format!("kit `{name}` is not used by any track"),
                );
            }
        }
    }

    /// Names of the song's own instruments and kits must not look like a
    /// preset or a file, or a track could not tell them apart.
    fn own_name(&mut self, name: &str, path: &str) {
        if name.is_empty() || name.contains(':') || is_file_reference(name) {
            self.error(
                Issue::error(
                    Code::InvalidFormat,
                    path,
                    format!(
                        "`{name}` cannot be used as a name here; use letters, digits and underscores (names with `:` or ending in .toml/.json mean presets and files)"
                    ),
                )
                .got(name),
            );
        }
    }

    /// Look up a track's instrument: the song's own kits and instruments
    /// first, then the outside source.
    fn instrument_of(&mut self, reference: &str, path: &str) -> Option<Instrument> {
        let song = self.song;
        if let Some(pieces) = song.kits.get(reference) {
            let mut kit = BTreeMap::new();
            for (piece, target) in pieces {
                let piece_path = format!("kits.{reference}.{piece}");
                let recipe = self.sound_of(target, &piece_path)?;
                kit.insert(piece.clone(), recipe);
            }
            return Some(Instrument::Kit(kit));
        }
        if let Some(recipe) = song.instruments.get(reference) {
            return Some(Instrument::Sound(Box::new(recipe.clone())));
        }
        match self.lookup(reference) {
            Ok(instrument) => Some(instrument),
            Err(issue) => {
                let mut issue = *issue;
                if issue.path.is_empty() {
                    issue.path = path.to_string();
                }
                if issue.suggestion.is_none() {
                    let own: Vec<String> = song
                        .instruments
                        .keys()
                        .chain(song.kits.keys())
                        .cloned()
                        .collect();
                    if let Some(s) = did_you_mean(reference, &own) {
                        issue = issue.suggest(s);
                    }
                }
                self.error(issue);
                None
            }
        }
    }

    /// A kit piece's recipe. A piece cannot itself be a kit.
    fn sound_of(&mut self, reference: &str, path: &str) -> Option<Recipe> {
        if let Some(recipe) = self.song.instruments.get(reference) {
            return Some(recipe.clone());
        }
        match self.lookup(reference) {
            Ok(Instrument::Sound(recipe)) => Some(*recipe),
            Ok(Instrument::Kit(_)) => {
                self.error(
                    Issue::error(
                        Code::Invalid,
                        path,
                        format!("`{reference}` is a kit; a kit piece must be a single instrument"),
                    )
                    .got(reference),
                );
                None
            }
            Err(issue) => {
                let mut issue = *issue;
                if issue.path.is_empty() {
                    issue.path = path.to_string();
                }
                self.error(issue);
                None
            }
        }
    }

    fn lookup(&mut self, reference: &str) -> Result<Instrument, Box<Issue>> {
        if let Some(found) = self.found.get(reference) {
            return found.clone();
        }
        let found = self.source.find(reference);
        self.found.insert(reference.to_string(), found.clone());
        found
    }

    /// All notes of one track as events, sorted by start.
    fn events(
        &mut self,
        index: usize,
        track: &Track,
        instrument: Option<&Instrument>,
    ) -> Vec<Event> {
        let song = self.song;
        let bpb = song.beats_per_bar as f64;
        let tp = format!("tracks[{index}]");
        let mut out = Vec::new();

        for (n, note) in track.notes.iter().enumerate() {
            let np = format!("{tp}.notes[{n}]");
            let bar = note.bar.unwrap_or(1);
            if bar == 0 {
                self.error(
                    Issue::error(
                        Code::OutOfRange,
                        format!("{np}.bar"),
                        "bars count from 1; the first bar is `bar = 1`",
                    )
                    .got(0)
                    .suggest("1"),
                );
                continue;
            }
            let start = (bar - 1) as f64 * bpb;
            self.note(
                &mut out,
                &np,
                &track.name,
                instrument,
                start,
                note.beat,
                &note.note,
                note.length,
                note.velocity,
                0,
            );
        }

        for (p, placement) in track.play.iter().enumerate() {
            let pp = format!("{tp}.play[{p}]");
            let Some(pattern) = song.patterns.get(&placement.pattern) else {
                let names: Vec<String> = song.patterns.keys().cloned().collect();
                let mut issue = Issue::error(
                    Code::NotFound,
                    format!("{pp}.pattern"),
                    format!("there is no pattern named `{}`", placement.pattern),
                )
                .got(placement.pattern.clone())
                .one_of(names.iter().cloned());
                if let Some(s) = did_you_mean(&placement.pattern, &names) {
                    issue = issue.suggest(s);
                }
                self.error(issue);
                continue;
            };
            if placement.bars.is_empty() {
                self.error(Issue::error(
                    Code::Invalid,
                    format!("{pp}.bars"),
                    "`bars` is empty, so the pattern is never played; list the bars it starts at, such as [1, 2]",
                ));
            }
            self.range(
                &format!("{pp}.transpose"),
                placement.transpose as f64,
                -48.0,
                48.0,
            );
            if placement.transpose != 0 && matches!(instrument, Some(Instrument::Kit(_))) {
                self.warn(
                    Code::Unused,
                    &format!("{pp}.transpose"),
                    "transpose has no effect on a kit; kit pieces have no pitch to move",
                );
            }
            for (b, &bar) in placement.bars.iter().enumerate() {
                if bar == 0 {
                    self.error(
                        Issue::error(
                            Code::OutOfRange,
                            format!("{pp}.bars[{b}]"),
                            "bars count from 1; the first bar is 1",
                        )
                        .got(0)
                        .suggest("1"),
                    );
                    continue;
                }
                if let Some(total) = song.bars {
                    if bar > total {
                        self.error(
                            Issue::error(
                                Code::OutOfRange,
                                format!("{pp}.bars[{b}]"),
                                format!(
                                    "bar {bar} is past the end of the song, which has {total} bars"
                                ),
                            )
                            .got(bar)
                            .range(1.0, total as f64),
                        );
                        continue;
                    }
                }
                let start = (bar - 1) as f64 * bpb;
                for (n, note) in pattern.notes.iter().enumerate() {
                    let np = format!("patterns.{}.notes[{n}]", placement.pattern);
                    self.note(
                        &mut out,
                        &np,
                        &track.name,
                        instrument,
                        start,
                        note.beat,
                        &note.note,
                        note.length,
                        note.velocity,
                        placement.transpose,
                    );
                }
            }
        }
        // A stable sort keeps the written order for notes that start
        // together. `total_cmp` orders floats the same way everywhere.
        out.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        out
    }

    /// Check one written note and add its events (several for a chord).
    #[allow(clippy::too_many_arguments)]
    fn note(
        &mut self,
        out: &mut Vec<Event>,
        np: &str,
        track: &str,
        instrument: Option<&Instrument>,
        bar_start: f64,
        beat: f64,
        value: &NoteValue,
        length: Option<f64>,
        velocity: f32,
        transpose: i32,
    ) {
        let song = self.song;
        let bpb = song.beats_per_bar as f64;
        let mut ok = self.range(&format!("{np}.beat"), beat, 0.0, 100_000.0);
        let length = length.unwrap_or(1.0);
        if !(length > 0.0 && length <= 100_000.0) {
            self.error(
                Issue::error(
                    Code::OutOfRange,
                    format!("{np}.length"),
                    format!("a note's length must be more than 0 beats, got {length}"),
                )
                .got(length),
            );
            ok = false;
        }
        ok &= self.range_f32(&format!("{np}.velocity"), velocity, 0.0, 1.0);
        let start = bar_start + beat;
        if let (true, Some(total)) = (ok, song.bars) {
            if start >= total as f64 * bpb {
                self.error(
                    Issue::error(
                        Code::OutOfRange,
                        format!("{np}.beat"),
                        format!(
                            "the note starts at beat {start} of the song, after its end ({total} bars of {bpb} beats = {} beats); move the note or raise `bars`",
                            total as f64 * bpb
                        ),
                    )
                    .got(beat),
                );
                ok = false;
            }
        }
        let pitches = value.pitches();
        if pitches.is_empty() {
            self.error(Issue::error(
                Code::Invalid,
                format!("{np}.note"),
                "the chord is empty; list at least one note, such as [\"C4\", \"E4\", \"G4\"]",
            ));
        }
        let chord = matches!(value, NoteValue::Chord(_));
        for (k, pitch) in pitches.iter().enumerate() {
            let pp = if chord {
                format!("{np}.note[{k}]")
            } else {
                format!("{np}.note")
            };
            let Some(sound) = self.sound(&pp, track, instrument, pitch, transpose) else {
                continue;
            };
            if ok {
                out.push(Event {
                    beat: start,
                    length,
                    sound,
                    velocity,
                    path: np.to_string(),
                    bar: (start / bpb).floor() as u32 + 1,
                    beat_in_bar: start - (start / bpb).floor() * bpb,
                });
            }
        }
    }

    /// Check one pitch against the track's instrument.
    fn sound(
        &mut self,
        path: &str,
        track: &str,
        instrument: Option<&Instrument>,
        pitch: &NotePitch,
        transpose: i32,
    ) -> Option<Sound> {
        match (instrument?, pitch) {
            (Instrument::Kit(pieces), NotePitch::Name(name)) => {
                if pieces.contains_key(name) {
                    return Some(Sound::Piece(name.clone()));
                }
                let names: Vec<String> = pieces.keys().cloned().collect();
                let mut issue = Issue::error(
                    Code::UnknownVariant,
                    path,
                    format!("track `{track}` plays a kit, which has no piece named `{name}`"),
                )
                .got(name.clone())
                .one_of(names.iter().cloned());
                if let Some(s) = did_you_mean(name, &names) {
                    issue = issue.suggest(s);
                }
                self.error(issue);
                None
            }
            (Instrument::Kit(pieces), NotePitch::Hz(hz)) => {
                self.error(
                    Issue::error(
                        Code::TypeMismatch,
                        path,
                        format!("track `{track}` plays a kit; write the name of a piece instead of a frequency"),
                    )
                    .got(*hz)
                    .one_of(pieces.keys().cloned()),
                );
                None
            }
            (Instrument::Sound(_), NotePitch::Hz(hz)) => {
                let moved = note::transpose_hz(*hz as f32, transpose);
                if !self.range(path, *hz, 1.0, 20_000.0) {
                    return None;
                }
                if !(1.0..=20_000.0).contains(&moved) {
                    self.error(
                        Issue::error(
                            Code::OutOfRange,
                            path,
                            format!("{hz} Hz moved by {transpose} semitones is {moved} Hz, outside 1..=20000"),
                        )
                        .got(*hz)
                        .range(1.0, 20_000.0),
                    );
                    return None;
                }
                Some(Sound::Pitch {
                    hz: moved,
                    name: None,
                })
            }
            (Instrument::Sound(_), NotePitch::Name(text)) => match note::note_number(text) {
                Ok(number) => {
                    let moved = number + transpose;
                    match note::note_name(moved) {
                        Some(name) => Some(Sound::Pitch {
                            hz: note::note_number_to_hz(moved),
                            // Keep the spelling as written when nothing moved.
                            name: Some(if transpose == 0 { text.clone() } else { name }),
                        }),
                        None => {
                            self.error(
                                Issue::error(
                                    Code::OutOfRange,
                                    path,
                                    format!("`{text}` moved by {transpose} semitones is outside the note range C-1..B9"),
                                )
                                .got(text.clone()),
                            );
                            None
                        }
                    }
                }
                Err(e) => {
                    let mut message = e.message;
                    let drum_like = !text.is_empty()
                        && text
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit())
                        && text.len() > 2;
                    if drum_like {
                        message.push_str(&format!(
                            "; if `{text}` is a drum, the track `{track}` must use a kit as its instrument"
                        ));
                    }
                    let mut issue =
                        Issue::error(Code::InvalidFormat, path, message).got(text.clone());
                    if let Some(s) = e.suggestion {
                        issue = issue.suggest(s);
                    }
                    self.error(issue);
                    None
                }
            },
        }
    }
}
