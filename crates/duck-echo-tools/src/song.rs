//! Songs for the front ends: finding instruments, and loading a document
//! that may be a recipe or a song.
//!
//! The CLI and the MCP server both load documents through
//! [`load_document`], so `render`, `validate`, `analyze` and `compare` work
//! on songs and recipes alike in both.

use crate::{presets, RecipeRef, ToolError, PRESET_PREFIX};
use duck_echo_core::load::{document_kind, read_text, DocumentKind};
use duck_echo_core::song::is_file_reference;
use duck_echo_core::{
    load_song_str, Code, Format, Instrument, InstrumentSource, Issue, LoadedSong, Report,
};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Turns an instrument file reference written in a song (such as
/// `"lead.toml"`) into the real path to read. Each front end gives its
/// own: the CLI reads next to the song file, the MCP server goes through
/// its root guard.
pub type FileResolver<'a> = dyn Fn(&str) -> Result<PathBuf, ToolError> + 'a;

/// A [`FileResolver`] that reads instrument files from the folder the
/// song file `song` is in. For the CLI.
pub fn files_next_to(song: &Path) -> impl Fn(&str) -> Result<PathBuf, ToolError> {
    let folder = song
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    move |reference| Ok(folder.join(reference))
}

/// Finds the instruments a song does not define itself: `preset:NAME` and
/// recipe files.
pub struct Library<'a> {
    files: &'a FileResolver<'a>,
}

impl<'a> Library<'a> {
    /// A library that reads instrument files through `files`.
    pub fn new(files: &'a FileResolver<'a>) -> Self {
        Self { files }
    }
}

/// One issue that stands for a failed tool call, to report at the song
/// field that caused it.
fn as_issue(e: ToolError, what: &str) -> Box<Issue> {
    Box::new(match e {
        ToolError::Failed(issue) => *issue,
        ToolError::Invalid(report) => {
            let first = report
                .errors
                .first()
                .map(|i| {
                    if i.path.is_empty() {
                        i.message.clone()
                    } else {
                        format!("{}: {}", i.path, i.message)
                    }
                })
                .unwrap_or_default();
            Issue::error(
                Code::Invalid,
                "",
                format!(
                    "{what} is not a valid recipe ({} problem(s)); the first is: {first}",
                    report.errors.len()
                ),
            )
        }
    })
}

impl InstrumentSource for Library<'_> {
    fn find(&self, reference: &str) -> Result<Instrument, Box<Issue>> {
        if let Some(name) = reference.strip_prefix(PRESET_PREFIX) {
            let preset = presets::get(name).map_err(|e| as_issue(e, reference))?;
            let loaded = presets::load(preset).map_err(|e| as_issue(e, reference))?;
            return Ok(Instrument::Sound(Box::new(loaded.recipe)));
        }
        if is_file_reference(reference) {
            let path = (self.files)(reference).map_err(|e| as_issue(e, reference))?;
            let text = read_text(&path).map_err(|_| {
                Box::new(
                    Issue::error(
                        Code::NotFound,
                        "",
                        format!("cannot read the instrument file `{reference}` (paths are relative to the song file's folder)"),
                    )
                    .got(reference),
                )
            })?;
            let format = Format::from_path(&path);
            if document_kind(&text, format) == DocumentKind::Song {
                return Err(Box::new(
                    Issue::error(
                        Code::Invalid,
                        "",
                        format!("`{reference}` is a song; an instrument must be a recipe (a document with `layers`)"),
                    )
                    .got(reference),
                ));
            }
            let loaded = duck_echo_core::load_str(&text, format)
                .map_err(|r| as_issue(ToolError::Invalid(r), reference))?;
            return Ok(Instrument::Sound(Box::new(loaded.recipe)));
        }
        Err(Box::new(
            Issue::error(
                Code::NotFound,
                "",
                format!(
                    "there is no instrument named `{reference}`; use a name from the song's `instruments` or `kits`, `preset:NAME`, or a recipe file ending in .toml or .json"
                ),
            )
            .got(reference),
        ))
    }
}

/// A loaded song plus a file-safe stem for output names.
#[derive(Debug, Clone)]
pub struct SongRef {
    /// The validated song with its arrangement and warnings.
    pub loaded: LoadedSong,
    /// Name used for output files.
    pub stem: String,
}

/// A loaded document: a sound or a song.
#[derive(Debug, Clone)]
pub enum Document {
    /// A recipe (a document with `layers`).
    Recipe(RecipeRef),
    /// A song (a document with `tracks`).
    Song(SongRef),
}

impl Document {
    /// Validation warnings.
    pub fn report(&self) -> &Report {
        match self {
            Document::Recipe(r) => &r.loaded.report,
            Document::Song(s) => &s.loaded.report,
        }
    }

    /// Name used for output files.
    pub fn stem(&self) -> &str {
        match self {
            Document::Recipe(r) => &r.stem,
            Document::Song(s) => &s.stem,
        }
    }

    /// `"recipe"` or `"song"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Document::Recipe(_) => "recipe",
            Document::Song(_) => "song",
        }
    }
}

fn stem_of(path: &Path, fallback: &str) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| fallback.into())
}

/// Load a recipe or a song from a file, or a preset from `preset:NAME`.
/// A song's instrument files are read through `files`.
pub fn load_document(reference: &Path, files: &FileResolver) -> Result<Document, ToolError> {
    let text = reference.to_string_lossy();
    if text.starts_with(PRESET_PREFIX) {
        return crate::load_recipe(reference).map(Document::Recipe);
    }
    let src = read_text(reference)?;
    let format = Format::from_path(reference);
    match document_kind(&src, format) {
        DocumentKind::Recipe => Ok(Document::Recipe(RecipeRef {
            loaded: duck_echo_core::load_str(&src, format)?,
            stem: stem_of(reference, "sound"),
        })),
        DocumentKind::Song => Ok(Document::Song(SongRef {
            loaded: load_song_str(&src, format, &Library::new(files))?,
            stem: stem_of(reference, "song"),
        })),
    }
}

/// Load a recipe or a song given as a JSON value. Line and column numbers
/// are dropped from the report, since they would refer to re-serialized
/// text the caller never saw; paths are exact.
pub fn load_document_value(doc: &Value, files: &FileResolver) -> Result<Document, ToolError> {
    let text = serde_json::to_string(doc).expect("json");
    let strip = |mut r: Report| {
        for i in r.errors.iter_mut().chain(r.warnings.iter_mut()) {
            i.line = None;
            i.column = None;
        }
        r
    };
    match document_kind(&text, Format::Json) {
        DocumentKind::Recipe => {
            let loaded = crate::edit::load_value(doc)?;
            let stem = loaded.recipe.name.clone().unwrap_or_else(|| "sound".into());
            Ok(Document::Recipe(RecipeRef { loaded, stem }))
        }
        DocumentKind::Song => match load_song_str(&text, Format::Json, &Library::new(files)) {
            Ok(mut loaded) => {
                loaded.report = strip(loaded.report);
                let stem = loaded.song.name.clone().unwrap_or_else(|| "song".into());
                Ok(Document::Song(SongRef { loaded, stem }))
            }
            Err(r) => Err(ToolError::Invalid(strip(r))),
        },
    }
}

/// The facts about a song a game engine or an agent needs next to the
/// audio: enough to switch music on a bar line.
#[derive(Debug, Clone, Serialize)]
pub struct SongFacts {
    /// Beats per minute.
    pub tempo: f64,
    /// Beats in one bar.
    pub beats_per_bar: u32,
    /// Length in bars.
    pub bars: u32,
    /// Frames in those bars, exactly. The file is longer when sound rings
    /// on after the last bar.
    pub song_frames: usize,
    /// Seconds in one bar.
    pub seconds_per_bar: f64,
    /// Tracks in the song.
    pub tracks: usize,
    /// Notes in the song, with patterns placed and chords split.
    pub notes: usize,
}

impl SongFacts {
    /// The facts of a loaded song.
    pub fn of(loaded: &LoadedSong) -> Self {
        let song = &loaded.song;
        let bars = loaded.arrangement.bars;
        Self {
            tempo: song.tempo,
            beats_per_bar: song.beats_per_bar,
            bars,
            song_frames: song.frame_at(bars as f64 * song.beats_per_bar as f64),
            seconds_per_bar: song.beats_per_bar as f64 * song.seconds_per_beat(),
            tracks: song.tracks.len(),
            notes: loaded.arrangement.note_count(),
        }
    }

    /// The start of a song's summary line, such as
    /// `4 bars at 110 BPM, 2 tracks`.
    pub fn summary(&self) -> String {
        format!(
            "{} bar{} at {} BPM, {} track{}",
            self.bars,
            if self.bars == 1 { "" } else { "s" },
            self.tempo,
            self.tracks,
            if self.tracks == 1 { "" } else { "s" },
        )
    }
}
