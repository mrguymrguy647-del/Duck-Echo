//! The song timeline, without a window: what each lane holds, mute and
//! solo, the note check, and watching the file.
//!
//! The timeline draws a song as it is played: from its **arrangement**
//! (`duck_echo_core::arrange`), where patterns are placed, chords are
//! split and bars are turned into beats, the very list the renderer
//! plays. So what you see is what you hear.
//!
//! * A **track** is a lane.
//! * A **note** is a block, at its beat and as long as it is held, higher
//!   in the lane for a higher pitch (a small piano roll). On a drum kit
//!   each piece has its own row.
//! * A **placed pattern** is a clip: a frame around the notes it adds,
//!   with the pattern's name and its transpose.
//! * The **note check** (the same one `analyze` runs for agents) listens
//!   to each track alone, on a background thread, and marks every note
//!   that is missing, too quiet or at the wrong pitch.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use duck_echo_core::{Arrangement, Song, Sound};
use duck_echo_tools::root::Root;

use crate::library;

/// How often the timeline looks at the file for changes made elsewhere.
pub const LOOK_EVERY: Duration = Duration::from_millis(500);

/// One note block.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Where it starts, in beats from the start of the song.
    pub beat: f64,
    /// How long it is held, in beats (a drum hit is drawn a quarter beat
    /// long).
    pub length: f64,
    /// Where it sits in its lane, from 0 (bottom) to 1 (top).
    pub height: f32,
    /// What it plays: `E5`, `440 Hz` or `kick`.
    pub label: String,
    /// The field it is written in: `tracks[2].notes[0]`, or
    /// `patterns.walk.notes[1]` for a note a pattern adds.
    pub path: String,
    /// The bar it starts in (from 1) and the beat in that bar (from 0),
    /// as a person reads the file.
    pub bar: u32,
    /// The beat in that bar.
    pub beat_in_bar: f64,
    /// How hard it is played, 0 to 1.
    pub velocity: f32,
}

/// One placed pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// The pattern's name.
    pub pattern: String,
    /// Where it starts and how long it lasts, in beats.
    pub beat: f64,
    /// Its length in beats: whole bars, enough for its notes.
    pub length: f64,
    /// Semitones it is moved by.
    pub transpose: i32,
}

/// One track's lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// The track's name.
    pub name: String,
    /// Its instrument, as written.
    pub instrument: String,
    /// Its volume.
    pub volume: f32,
    /// True for a drum kit (rows are pieces, not pitches).
    pub kit: bool,
    /// The notes.
    pub blocks: Vec<Block>,
    /// The placed patterns.
    pub clips: Vec<Clip>,
}

/// Every lane of a song, from its arrangement.
pub fn lanes(song: &Song, arrangement: &Arrangement) -> Vec<Lane> {
    let per_bar = f64::from(song.beats_per_bar);
    song.tracks
        .iter()
        .zip(&arrangement.tracks)
        .map(|(track, plan)| {
            let kit = matches!(plan.instrument, duck_echo_core::Instrument::Kit(_));
            // Rows: kit pieces in name order, or the pitch range.
            let pieces: BTreeSet<&str> = plan
                .events
                .iter()
                .filter_map(|e| match &e.sound {
                    Sound::Piece(name) => Some(name.as_str()),
                    Sound::Pitch { .. } => None,
                })
                .collect();
            let pieces: Vec<&str> = pieces.into_iter().collect();
            let hz: Vec<f32> = plan
                .events
                .iter()
                .filter_map(|e| match &e.sound {
                    Sound::Pitch { hz, .. } => Some(*hz),
                    Sound::Piece(_) => None,
                })
                .collect();
            let low = hz.iter().copied().fold(f32::INFINITY, f32::min);
            let high = hz.iter().copied().fold(0.0f32, f32::max);
            let blocks = plan
                .events
                .iter()
                .map(|e| {
                    let (height, label, length) = match &e.sound {
                        Sound::Pitch { hz, name } => {
                            let height = if high > low {
                                ((hz / low).ln() / (high / low).ln()).clamp(0.0, 1.0)
                            } else {
                                0.5
                            };
                            let label = name.clone().unwrap_or_else(|| format!("{hz:.0} Hz"));
                            (height, label, e.length)
                        }
                        Sound::Piece(piece) => {
                            let row = pieces.iter().position(|p| p == piece).unwrap_or(0);
                            let height = if pieces.len() > 1 {
                                row as f32 / (pieces.len() - 1) as f32
                            } else {
                                0.5
                            };
                            (height, piece.clone(), 0.25)
                        }
                    };
                    Block {
                        beat: e.beat,
                        length,
                        height,
                        label,
                        path: e.path.clone(),
                        bar: e.bar,
                        beat_in_bar: e.beat_in_bar,
                        velocity: e.velocity,
                    }
                })
                .collect();
            let clips = track
                .play
                .iter()
                .flat_map(|placement| {
                    let notes = song
                        .patterns
                        .get(&placement.pattern)
                        .map(|p| p.notes.as_slice())
                        .unwrap_or_default();
                    let end = notes
                        .iter()
                        .map(|n| n.beat + n.length.unwrap_or(1.0).max(0.25))
                        .fold(0.0f64, f64::max);
                    let length = ((end / per_bar).ceil().max(1.0)) * per_bar;
                    placement.bars.iter().map(move |bar| Clip {
                        pattern: placement.pattern.clone(),
                        beat: f64::from(bar.saturating_sub(1)) * per_bar,
                        length,
                        transpose: placement.transpose,
                    })
                })
                .collect();
            Lane {
                name: track.name.clone(),
                instrument: track.instrument.clone(),
                volume: track.volume,
                kit,
                blocks,
                clips,
            }
        })
        .collect()
}

/// A note the note check found a problem with.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The track's number.
    pub track: usize,
    /// Where the note is written.
    pub path: String,
    /// Its bar and beat in the bar.
    pub bar: u32,
    /// The beat in that bar.
    pub beat: f64,
    /// What is wrong, in a sentence.
    pub why: String,
}

impl Problem {
    /// True when this is the problem of `block` on track `track`.
    pub fn is_about(&self, track: usize, block: &Block) -> bool {
        self.track == track
            && self.path == block.path
            && self.bar == block.bar
            && (self.beat - block.beat_in_bar).abs() < 1e-6
    }
}

/// What the note check found, from the background thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    /// Which version of the file it is the check of.
    pub generation: u64,
    /// The summary line agents read: bars, tempo, loudness, how the loop
    /// joins, how many notes were heard.
    pub summary: String,
    /// The notes with a problem.
    pub problems: Vec<Problem>,
}

/// Check a song's notes: render each track alone and listen for every
/// note, as `analyze` does for agents.
fn check(generation: u64, root_dir: &Path, rel: &str, text: &str) -> Result<Checked, String> {
    let root = Root::new(root_dir)?;
    let files = root.song_files(library::folder_of(rel).to_string());
    let loaded = duck_echo_core::load_song_str(
        text,
        duck_echo_core::Format::Toml,
        &duck_echo_tools::song::Library::new(&files),
    )
    .map_err(|r| r.to_string())?;
    let names: Vec<String> = loaded.song.tracks.iter().map(|t| t.name.clone()).collect();
    let options = duck_echo_tools::Options {
        check_notes: true,
        ..Default::default()
    };
    let extras = options
        .for_song(&names, "")
        .map_err(|e| e.into_report().to_string())?;
    let facts = duck_echo_tools::SongFacts::of(&loaded);
    let measured = duck_echo_tools::measure::measure_song(
        &loaded.song,
        &loaded.arrangement,
        &facts,
        &loaded.report.warnings,
        &extras,
    )
    .map_err(|e| e.into_report().to_string())?
    .measured;
    let mut problems = Vec::new();
    if let Some(check) = &measured.note_check {
        for (track, t) in check.tracks.iter().enumerate() {
            for line in &t.problems {
                let why = match line.status.as_str() {
                    "missing" => "Nothing is heard here.".to_string(),
                    "quiet" => "It is much quieter than the rest of the track.".to_string(),
                    "wrong_pitch" => format!(
                        "It sounds like {} instead.",
                        line.heard.clone().unwrap_or_else(|| "another note".into())
                    ),
                    other => format!("The note check says: {other}."),
                };
                problems.push(Problem {
                    track,
                    path: line.path.clone(),
                    bar: line.bar,
                    beat: line.beat,
                    why,
                });
            }
        }
    }
    Ok(Checked {
        generation,
        summary: measured.analysis.summary,
        problems,
    })
}

type Job = (u64, std::path::PathBuf, String, String);

/// The background thread that checks the notes.
struct Checker {
    jobs: Sender<Job>,
    done: Receiver<(u64, Result<Checked, String>)>,
    asked: u64,
    answered: u64,
}

impl Checker {
    fn new() -> Checker {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("song note check".into())
            .spawn(move || {
                while let Ok(mut job) = inbox.recv() {
                    while let Ok(newer) = inbox.try_recv() {
                        job = newer;
                    }
                    let (generation, dir, rel, text) = job;
                    let result = check(generation, &dir, &rel, &text);
                    if outbox.send((generation, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("a thread for the note check");
        Checker {
            jobs,
            done,
            asked: 0,
            answered: 0,
        }
    }
}

/// One song open in the timeline.
pub struct SongView {
    /// The file, relative to the project folder.
    pub rel: String,
    /// The text the file held when last read.
    text: String,
    /// The song and its arrangement, when the text works.
    pub loaded: Option<(Song, Arrangement)>,
    /// Why the song cannot be shown, when it cannot.
    pub problem: Option<String>,
    /// The lanes of [`SongView::loaded`].
    pub lanes: Vec<Lane>,
    /// Muted tracks, by number.
    pub muted: BTreeSet<usize>,
    /// Solo tracks, by number. When any is solo, only those are heard.
    pub solo: BTreeSet<usize>,
    /// Play repeats the song.
    pub looping: bool,
    /// The newest note check.
    pub checked: Option<Checked>,
    /// Why the note check failed, if it did.
    pub check_problem: Option<String>,
    checker: Checker,
    looked_at: Instant,
    /// True when the file is no longer there.
    pub gone: bool,
}

impl SongView {
    /// Open the song `rel` of the project.
    pub fn open(root: &Root, rel: &str) -> Result<SongView, String> {
        let text = read(root, rel)?;
        let mut view = SongView {
            rel: rel.to_string(),
            text: String::new(),
            loaded: None,
            problem: None,
            lanes: Vec::new(),
            muted: BTreeSet::new(),
            solo: BTreeSet::new(),
            looping: false,
            checked: None,
            check_problem: None,
            checker: Checker::new(),
            looked_at: Instant::now(),
            gone: false,
        };
        view.take(root, text);
        Ok(view)
    }

    /// The file's name without its folder or extension.
    pub fn name(&self) -> String {
        Path::new(&self.rel)
            .file_stem()
            .map_or(self.rel.clone(), |s| s.to_string_lossy().into_owned())
    }

    /// Show `text`, the file's text now, and check its notes.
    fn take(&mut self, root: &Root, text: String) {
        self.text = text;
        match library::load_song(root, library::folder_of(&self.rel), &self.text) {
            Ok((song, arrangement)) => {
                self.lanes = lanes(&song, &arrangement);
                let count = song.tracks.len();
                self.muted.retain(|t| *t < count);
                self.solo.retain(|t| *t < count);
                self.loaded = Some((song, arrangement));
                self.problem = None;
                self.checker.asked += 1;
                let _ = self.checker.jobs.send((
                    self.checker.asked,
                    root.dir().to_path_buf(),
                    self.rel.clone(),
                    self.text.clone(),
                ));
            }
            Err(why) => {
                self.problem = Some(why);
            }
        }
    }

    /// Call every frame (and from the tests, with the time): pick up the
    /// note check, and look at the file for changes made elsewhere.
    /// Returns true when the song changed (the player must hear it).
    pub fn tick(&mut self, root: &Root, now: Instant) -> bool {
        while let Ok((generation, result)) = self.checker.done.try_recv() {
            self.checker.answered = generation;
            if generation == self.checker.asked {
                match result {
                    Ok(checked) => {
                        self.checked = Some(checked);
                        self.check_problem = None;
                    }
                    Err(why) => self.check_problem = Some(why),
                }
            }
        }
        if now.duration_since(self.looked_at) < LOOK_EVERY {
            return false;
        }
        self.looked_at = now;
        match read(root, &self.rel) {
            Ok(text) => {
                self.gone = false;
                if text != self.text {
                    self.take(root, text);
                    return self.loaded.is_some();
                }
                false
            }
            Err(_) => {
                self.gone = true;
                false
            }
        }
    }

    /// True while the notes are being checked.
    pub fn checking(&self) -> bool {
        self.checker.answered < self.checker.asked
    }

    /// Wait (up to `timeout`) for the newest note check. For the tests.
    pub fn wait_for_check(&mut self, root: &Root, timeout: Duration) -> bool {
        let start = Instant::now();
        while self.checking() && start.elapsed() < timeout {
            self.tick(root, self.looked_at);
            std::thread::sleep(Duration::from_millis(10));
        }
        !self.checking()
    }

    /// The tracks heard, or `None` for all of them: the solo tracks if
    /// any is solo, else every track that is not muted.
    pub fn heard(&self) -> Option<Vec<usize>> {
        let count = self.lanes.len();
        if !self.solo.is_empty() {
            return Some(self.solo.iter().copied().collect());
        }
        if self.muted.is_empty() {
            return None;
        }
        Some((0..count).filter(|t| !self.muted.contains(t)).collect())
    }

    /// True when track `t` is heard now.
    pub fn is_heard(&self, t: usize) -> bool {
        self.heard().is_none_or(|h| h.contains(&t))
    }

    /// Mute or unmute track `t`.
    pub fn toggle_mute(&mut self, t: usize) {
        if !self.muted.remove(&t) {
            self.muted.insert(t);
        }
    }

    /// Solo track `t`, or take its solo away.
    pub fn toggle_solo(&mut self, t: usize) {
        if !self.solo.remove(&t) {
            self.solo.insert(t);
        }
    }

    /// The problem the note check found with `block` of track `t`.
    pub fn problem_of(&self, t: usize, block: &Block) -> Option<&Problem> {
        self.checked
            .as_ref()?
            .problems
            .iter()
            .find(|p| p.is_about(t, block))
    }
}

fn read(root: &Root, rel: &str) -> Result<String, String> {
    let path = root
        .read("file", rel)
        .map_err(|e| e.into_report().to_string())?;
    std::fs::read_to_string(&path).map_err(|e| format!("{rel} cannot be read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> (Song, Arrangement) {
        let dir = std::env::temp_dir().join(format!("duck-echo-lanes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = Root::new(&dir).unwrap();
        let example = duck_echo_tools::examples::get("duck_craft_day").unwrap();
        library::load_song(&root, "", example.source).unwrap()
    }

    #[test]
    fn the_day_theme_has_a_lane_per_track_with_its_notes_and_clips() {
        let (song, arrangement) = day();
        let lanes = lanes(&song, &arrangement);
        let names: Vec<&str> = lanes.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            song.tracks
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
        );
        // Every note the renderer plays is a block.
        let blocks: usize = lanes.iter().map(|l| l.blocks.len()).sum();
        assert_eq!(blocks, arrangement.note_count());
        // The pad's first chord: three notes at the start, a bar long.
        let pad = &lanes[0];
        let first: Vec<&Block> = pad.blocks.iter().filter(|b| b.beat == 0.0).collect();
        assert_eq!(first.len(), 3);
        assert!(first.iter().all(|b| b.length == 4.0 && b.bar == 1));
        // Higher notes sit higher.
        let c4 = first.iter().find(|b| b.label == "C4").unwrap();
        let g4 = first.iter().find(|b| b.label == "G4").unwrap();
        assert!(g4.height > c4.height);
        // The bass is clips of its one-bar pattern: bars 1 to 8.
        let bass = &lanes[1];
        assert!(bass.blocks.iter().all(|b| b.path.starts_with("patterns.")));
        let mut starts: Vec<f64> = bass.clips.iter().map(|c| c.beat).collect();
        starts.sort_by(f64::total_cmp);
        assert_eq!(
            starts,
            (0..8).map(|b| f64::from(b) * 4.0).collect::<Vec<_>>()
        );
        assert!(bass.clips.iter().all(|c| c.length == 4.0));
        assert!(
            bass.clips
                .iter()
                .any(|c| c.transpose == 7 && c.pattern == "root_and_fifth")
        );
        // The drums are a kit: rows are pieces.
        let drums = lanes.iter().find(|l| l.kit).expect("a kit lane");
        assert!(drums.blocks.iter().any(|b| b.label == "kick"));
    }
}
