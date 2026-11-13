//! Checking a song without ears: the notes written, next to the notes
//! heard.
//!
//! A note can go missing in ways the song text does not show. It can be
//! out of its instrument's range, buried, too short to sound, or played
//! by an instrument whose loudest part is not the note itself. So for
//! every written note this module listens at the place the note should
//! be, on its track rendered alone, and says what is there.

use duck_echo_analyze::notes::{cents, read_hit, read_pitched, NoteStatus};
use duck_echo_analyze::Onset;
use duck_echo_audio::AudioBuffer;
use duck_echo_core::note::note_name;
use duck_echo_core::{Song, Sound, TrackPlan};
use serde::Serialize;

/// A note this far under the usual level of its track is `quiet`, dB.
pub const QUIET_BELOW_DB: f64 = 24.0;

/// The longest stretch of a note that is listened to, seconds.
const LISTEN_SECONDS: f64 = 0.8;
/// The stretch listened to for a drum hit, seconds.
const HIT_SECONDS: f64 = 0.05;
/// An onset this close to a written note belongs to it, seconds.
const ONSET_REACH: f64 = 0.06;
/// How long a note may ring on after it ends, when deciding which other
/// notes could be sounding, seconds.
const RING_SECONDS: f64 = 1.5;

/// One written note and what was heard for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteLine {
    /// `heard`, `missing` (nothing there), `quiet` (far below the rest of
    /// the track), `wrong_pitch` (something else is there) or `uncertain`
    /// (too short to read).
    pub status: String,
    /// Where the note is written, such as `tracks[1].notes[3]` or
    /// `patterns.walk.notes[2]`.
    pub path: String,
    /// The bar the note starts in, counted from 1.
    pub bar: u32,
    /// The beat in that bar, counted from 0.
    pub beat: f64,
    /// The note as written: a note name, a frequency, or a kit piece.
    pub written: String,
    /// The written pitch in Hz. Absent for kit pieces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_hz: Option<f32>,
    /// The nearest note name to what was heard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard: Option<String>,
    /// The pitch heard, Hz.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_hz: Option<f32>,
    /// How far the pitch heard is from the pitch written, in cents (100
    /// is a semitone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cents_off: Option<f32>,
    /// Level of what was heard, dBFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_dbfs: Option<f32>,
}

/// A clear start of sound where no note was written.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtraOnset {
    /// Seconds from the start of the file.
    pub time_seconds: f64,
    /// The bar it falls in, counted from 1.
    pub bar: u32,
    /// The beat in that bar, counted from 0.
    pub beat: f64,
    /// Its pitch, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_hz: Option<f32>,
}

/// The notes of one track, written and heard.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackNoteCheck {
    /// The track's name.
    pub name: String,
    /// The instrument as written on the track.
    pub instrument: String,
    /// Notes written on the track (patterns placed, chords split).
    pub notes_written: usize,
    /// How many of them were heard at their pitch.
    pub notes_heard: usize,
    /// Notes too short to read; they are neither heard nor a problem.
    #[serde(skip_serializing_if = "is_zero")]
    pub notes_uncertain: usize,
    /// Every note that was not simply heard.
    pub problems: Vec<NoteLine>,
    /// Starts of sound with no written note (an echo's repeats, for
    /// example). Information, not a problem.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_onsets: Vec<ExtraOnset>,
    /// Every note, when asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<NoteLine>>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The notes of a whole song, written and heard.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteCheck {
    /// Notes written in the tracks that were checked.
    pub notes_written: usize,
    /// How many of them were heard at their pitch.
    pub notes_heard: usize,
    /// Notes too short to read.
    #[serde(skip_serializing_if = "is_zero")]
    pub notes_uncertain: usize,
    /// One entry per checked track.
    pub tracks: Vec<TrackNoteCheck>,
}

impl NoteCheck {
    /// Gather the tracks' counts.
    pub fn of(tracks: Vec<TrackNoteCheck>) -> Self {
        Self {
            notes_written: tracks.iter().map(|t| t.notes_written).sum(),
            notes_heard: tracks.iter().map(|t| t.notes_heard).sum(),
            notes_uncertain: tracks.iter().map(|t| t.notes_uncertain).sum(),
            tracks,
        }
    }

    /// The part of a summary line, such as `61 of 62 notes heard`.
    pub fn summary(&self) -> String {
        let mut text = format!("{} of {} notes heard", self.notes_heard, self.notes_written);
        if self.notes_uncertain > 0 {
            text.push_str(&format!(" ({} too short to read)", self.notes_uncertain));
        }
        text
    }
}

/// The nearest note name to a frequency.
fn nearest_note(hz: f64) -> Option<String> {
    (hz > 0.0)
        .then(|| (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32)
        .and_then(note_name)
}

fn to_db(level: f64) -> f64 {
    20.0 * level.max(1e-12).log10()
}

/// Mono samples for frames `from..to` of `mono`. A loop is read round the
/// join; anything else stops at the end.
fn stretch(mono: &[f32], from: usize, to: usize, looped: bool) -> Vec<f32> {
    if mono.is_empty() || to <= from {
        return Vec::new();
    }
    if looped {
        (from..to).map(|i| mono[i % mono.len()]).collect()
    } else {
        mono[from.min(mono.len())..to.min(mono.len())].to_vec()
    }
}

/// Listen for every note of `plan` in `stem`, that track rendered alone.
/// `onsets` are the stem's analysis onsets, used to find starts of sound
/// that no note explains. `list_all` also returns every note.
pub fn check_track(
    song: &Song,
    plan: &TrackPlan,
    instrument: &str,
    stem: &AudioBuffer,
    onsets: &[Onset],
    list_all: bool,
) -> TrackNoteCheck {
    let mono = stem.to_mono();
    let sr = stem.sample_rate as f64;
    let spb = song.seconds_per_beat();
    let seconds = |beats: f64| beats * spb;

    // (start, end) of every pitched note in seconds, with its pitch.
    let pitched: Vec<(f64, f64, f64)> = plan
        .events
        .iter()
        .filter_map(|e| match &e.sound {
            Sound::Pitch { hz, .. } => {
                Some((seconds(e.beat), seconds(e.beat + e.length), *hz as f64))
            }
            Sound::Piece(_) => None,
        })
        .collect();

    let mut lines = Vec::with_capacity(plan.events.len());
    let mut levels = Vec::new();
    for event in &plan.events {
        let start = seconds(event.beat);
        let frame = |t: f64| (t * sr).round().max(0.0) as usize;
        let (written, written_hz, reading) = match &event.sound {
            Sound::Pitch { hz, name } => {
                let held = seconds(event.length);
                // Skip the very start (the attack), then listen to the
                // held part, up to 0.8 s of it.
                let from = start + (0.25 * held).min(0.08);
                let to = start + held.min(LISTEN_SECONDS);
                let window = stretch(&mono, frame(from), frame(to), song.loop_);
                // Other notes of the track that could be sounding here.
                let others: Vec<f64> = pitched
                    .iter()
                    .filter(|(s, e, other)| {
                        *s < to && e + RING_SECONDS > from && (*other - *hz as f64).abs() > 1e-6
                    })
                    .map(|(_, _, other)| *other)
                    .collect();
                (
                    name.clone().unwrap_or_else(|| format!("{hz} Hz")),
                    Some(*hz),
                    read_pitched(&window, stem.sample_rate, *hz as f64, &others),
                )
            }
            Sound::Piece(piece) => {
                let window = stretch(&mono, frame(start), frame(start + HIT_SECONDS), song.loop_);
                (piece.clone(), None, read_hit(&window))
            }
        };
        if reading.status == NoteStatus::Heard {
            if let Some(level) = reading.level {
                levels.push(to_db(level));
            }
        }
        lines.push((
            reading.status,
            reading.level,
            NoteLine {
                status: String::new(),
                path: event.path.clone(),
                bar: event.bar,
                beat: event.beat_in_bar,
                written,
                written_hz,
                heard: reading.heard_hz.and_then(nearest_note),
                heard_hz: reading.heard_hz.map(|h| h as f32),
                cents_off: written_hz
                    .zip(reading.heard_hz)
                    .map(|(w, h)| cents(h, w as f64) as f32),
                level_dbfs: reading.level.map(|l| to_db(l) as f32),
            },
        ));
    }

    // The usual level of the track: the middle one of its heard notes.
    levels.sort_by(|a, b| a.total_cmp(b));
    let usual = levels.get(levels.len() / 2).copied();

    let mut heard = 0;
    let mut uncertain = 0;
    let mut problems = Vec::new();
    let mut all = Vec::new();
    for (status, level, mut line) in lines {
        let quiet = status == NoteStatus::Heard
            && usual
                .zip(level)
                .is_some_and(|(usual, l)| to_db(l) < usual - QUIET_BELOW_DB);
        line.status = match status {
            NoteStatus::Heard if quiet => "quiet",
            NoteStatus::Heard => "heard",
            NoteStatus::Missing => "missing",
            NoteStatus::WrongPitch => "wrong_pitch",
            NoteStatus::Uncertain => "uncertain",
        }
        .to_string();
        match line.status.as_str() {
            "heard" => heard += 1,
            "uncertain" => {
                uncertain += 1;
                problems.push(line.clone());
            }
            _ => problems.push(line.clone()),
        }
        if list_all {
            all.push(line);
        }
    }

    // Starts of sound that no written note explains.
    let starts: Vec<f64> = plan.events.iter().map(|e| seconds(e.beat)).collect();
    let bpb = song.beats_per_bar as f64;
    let extra_onsets = onsets
        .iter()
        // A loop file begins mid-ring, so its first sample is an onset.
        .filter(|o| !(song.loop_ && o.time_seconds < 0.03))
        .filter(|o| {
            !starts
                .iter()
                .any(|s| (s - o.time_seconds).abs() <= ONSET_REACH)
        })
        .map(|o| {
            let beats = o.time_seconds / spb;
            ExtraOnset {
                time_seconds: o.time_seconds,
                bar: (beats / bpb).floor() as u32 + 1,
                beat: ((beats - (beats / bpb).floor() * bpb) * 100.0).round() / 100.0,
                note_hz: o.note_hz,
            }
        })
        .collect();

    TrackNoteCheck {
        name: plan.name.clone(),
        instrument: instrument.to_string(),
        notes_written: plan.events.len(),
        notes_heard: heard,
        notes_uncertain: uncertain,
        problems,
        extra_onsets,
        notes: list_all.then_some(all),
    }
}
