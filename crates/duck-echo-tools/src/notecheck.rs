//! Checking a song without ears: the notes written, next to the notes
//! heard.
//!
//! A note can go missing in ways the song text does not show. It can be
//! out of its instrument's range, buried, too short to sound, or played
//! by an instrument whose loudest part is not the note itself. So for
//! every written note this module listens at the place the note should
//! be, on its track rendered alone, and says what is there.

use duck_echo_analyze::notes::{cents, read_hit, read_pitched, NoteStatus, SAME_NOTE_CENTS};
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
/// An onset up to this long after a held note ends still counts as
/// inside it (the note's release is still sounding), seconds.
const HELD_GRACE: f64 = 0.15;
/// The stretch measured just before and just after an onset, to see
/// whether the level jumps, seconds. Long enough for several cycles of a
/// low bass note, so the wobble of one cycle does not count as a jump.
const JUMP_WINDOW: f64 = 0.08;
/// A rise in level this big, from just before an onset to just after it,
/// is a new sound even at the pitch of a held note, dB.
const JUMP_DB: f64 = 6.0;
/// The highest pitch read from an onset that counts as a note, Hz. The
/// top note of a piano (C8) is 4186 Hz. Above this, a "pitch" read from a
/// short stretch is hiss or a fold-back tone, so it counts as no clear
/// pitch.
const NOTE_PITCH_LIMIT: f64 = 5000.0;

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
    /// The beat in that bar, counted from 0, rounded to the nearest eighth
    /// of a beat (the place you would write a note there).
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
    // For a loop, notes held across the end also ring at the start of
    // the file, so they are counted there too, one loop length earlier.
    let loop_seconds = song.loop_.then(|| mono.len() as f64 / sr);
    let mut held = pitched.clone();
    if let Some(length) = loop_seconds {
        held.extend(
            pitched
                .iter()
                .map(|(s, e, hz)| (s - length, e - length, *hz)),
        );
    }
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
        .filter(|o| !inside_held_note(o, &held, &mono, sr))
        .map(|o| {
            let beats = o.time_seconds / spb;
            ExtraOnset {
                time_seconds: o.time_seconds,
                bar: (beats / bpb).floor() as u32 + 1,
                // Rounded to the nearest eighth of a beat: the place you
                // would write the note. The exact time is in time_seconds.
                beat: ((beats - (beats / bpb).floor() * bpb) * 8.0).round() / 8.0,
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

/// True when `onset` is only a ripple inside a note that is still held on
/// the same track, so it is not an extra note.
///
/// The onset finder looks for sudden changes. A long, low note is never
/// perfectly steady: its filter closes, its layers drift against each
/// other, and its waves beat. Each of those can look like a new start.
/// So an onset inside a held note is explained by that note when both
/// are true:
///
/// * it brings no new pitch: it has no clear pitch (none, or one above
///   5 kHz, where a short reading is hiss), or its pitch is the held
///   note's pitch or one of its overtones (2, 3, ... 8 times it), within
///   half a semitone; and
/// * the level does not jump: the 80 ms after it are less than 6 dB
///   louder than the 80 ms before it.
///
/// `held` is (start, end, Hz) in seconds for every pitched note of the
/// track. Drum tracks have none, so their extras are found by time alone.
fn inside_held_note(onset: &Onset, held: &[(f64, f64, f64)], mono: &[f32], sr: f64) -> bool {
    let t = onset.time_seconds;
    let sounding: Vec<&(f64, f64, f64)> = held
        .iter()
        .filter(|(start, end, _)| *start < t - ONSET_REACH && t <= end + HELD_GRACE)
        .collect();
    if sounding.is_empty() {
        return false;
    }
    let same_pitch = match onset.note_hz.filter(|hz| (*hz as f64) <= NOTE_PITCH_LIMIT) {
        None => true,
        Some(hz) => sounding.iter().any(|(_, _, note)| {
            (1..=8).any(|n| cents(hz as f64, note * n as f64).abs() < SAME_NOTE_CENTS)
        }),
    };
    if !same_pitch {
        return false;
    }
    // The level before is measured only after the held note started, so
    // its own attack is never part of the comparison.
    let latest_start = sounding
        .iter()
        .map(|(start, _, _)| *start)
        .fold(f64::MIN, f64::max);
    let before_from = (t - JUMP_WINDOW).max(latest_start + 0.02);
    let frame = |s: f64| ((s * sr).round().max(0.0) as usize).min(mono.len());
    let rms = |from: f64, to: f64| {
        let part = &mono[frame(from)..frame(to).max(frame(from))];
        if part.is_empty() {
            return None;
        }
        Some((part.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / part.len() as f64).sqrt())
    };
    match (rms(before_from, t - 0.005), rms(t, t + JUMP_WINDOW)) {
        (Some(before), Some(after)) => to_db(after) - to_db(before) < JUMP_DB,
        // Too little before the onset to compare: the held note explains
        // it, as it did for the pitch.
        _ => true,
    }
}
