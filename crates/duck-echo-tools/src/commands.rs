//! Every action as one named command.
//!
//! A button in Duck Echo Studio, a line in a command script
//! (`duck-echo run`), one command typed in a terminal (`duck-echo do`) and
//! an agent's MCP tool all do the same thing: build a [`Command`] and
//! [`run`] it. Each command that changes a file becomes one step of the
//! project's shared Undo history ([`crate::project`]), so a person and an
//! agent can undo each other's steps.
//!
//! A command is written as JSON whose `"do"` is its name:
//!
//! ```json
//! {"do": "sound_set", "file": "sfx/coin.toml", "path": "layers[0].volume", "value": 0.6}
//! ```
//!
//! [`TABLE`] lists every command with its description and argument schema.
//! The MCP server makes one tool per entry, with the same name and
//! arguments, and `duck-echo commands` prints it. A test checks that the
//! table and the `Command` type list the same commands.
//!
//! Names use `_`, not `.`: Claude's tool names must match
//! `^[a-zA-Z0-9_-]{1,128}$`, and one name for the button, the script and
//! the tool matters more than looking like Duck Forge's `level.paint`.

use crate::edit::{apply_edits, document_text, parse_document, EditOp};
use crate::history::Who;
use crate::instruments::{self, INSTRUMENT_PREFIX};
use crate::project::{Done, Plan, Project};
use crate::{presets, ToolError, PRESET_PREFIX};
use duck_echo_core::load::{did_you_mean, document_kind, DocumentKind};
use duck_echo_core::{Code, Format, Issue, Report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::Path;

// ---------------------------------------------------------------------
// Arguments. Each struct is one command's arguments; its doc comments
// become the descriptions an agent reads in the tool's schema.
// ---------------------------------------------------------------------

/// One file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileArgs {
    /// The file, relative to the project folder, ending in `.toml` or
    /// `.json`: `sfx/coin.toml`.
    pub file: String,
}

/// A file and a new name for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileToArgs {
    /// The file, relative to the project folder.
    pub file: String,
    /// The new file, relative to the project folder, ending in `.toml` or
    /// `.json`. It must not exist yet.
    pub to: String,
}

/// Where a copy comes from, and where it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyFromArgs {
    /// What to copy: `preset:NAME`, `instrument:NAME`, or a file in the
    /// project.
    pub from: String,
    /// The new file, relative to the project folder, ending in `.toml` or
    /// `.json`. It must not exist yet.
    pub to: String,
}

/// A whole new text for a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTextArgs {
    /// The file, relative to the project folder. It must exist.
    pub file: String,
    /// Its whole new text, kept exactly as written (comments too). It must
    /// be a valid recipe or song, of the same kind as the file is now.
    pub text: String,
}

/// One field by its path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetArgs {
    /// The file, relative to the project folder.
    pub file: String,
    /// The field, as in error messages: `layers[0].volume`,
    /// `master.normalize_lufs`, `tracks[1].notes[3].note`. Missing tables
    /// on the way are made; an index equal to a list's length appends.
    pub path: String,
    /// The new value: a number, a string, a list or an object.
    pub value: Value,
}

/// One field to take out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveArgs {
    /// The file, relative to the project folder.
    pub file: String,
    /// The field or list item to remove: `master.normalize`,
    /// `layers[0].filter`, `effects[1]`.
    pub path: String,
}

/// A new layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddLayerArgs {
    /// The sound file, relative to the project folder.
    pub file: String,
    /// The layer, as a recipe writes it: `{"source": "sine", "pitch":
    /// {"start": "C5"}}`. Default: a sine wave at A4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<Value>,
    /// Where to put it among the layers, from 0. Default: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// One layer by its number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerArgs {
    /// The sound file, relative to the project folder.
    pub file: String,
    /// The layer's number, from 0.
    pub index: usize,
}

/// A layer to move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveLayerArgs {
    /// The sound file, relative to the project folder.
    pub file: String,
    /// The layer's number now, from 0.
    pub from: usize,
    /// Its number afterwards, from 0.
    pub to: usize,
}

/// A new effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddEffectArgs {
    /// The sound file, relative to the project folder.
    pub file: String,
    /// The layer it goes on, from 0. Leave it out for the master effects,
    /// which act on the whole sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<usize>,
    /// The effect, as a recipe writes it: `{"type": "reverb", "mix": 0.3}`.
    pub effect: Value,
    /// Where to put it in the chain, from 0. Default: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// One effect by its number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EffectArgs {
    /// The sound file, relative to the project folder.
    pub file: String,
    /// The layer whose effect it is, from 0. Leave it out for a master
    /// effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<usize>,
    /// The effect's number in its chain, from 0.
    pub index: usize,
}

/// A new track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddTrackArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// A short name, different from the other tracks': `bass`.
    pub name: String,
    /// What plays it: a built-in instrument (`piano`, `pad`, `bass`,
    /// `chip_lead`, `drums`), one from the song's `instruments`,
    /// `preset:NAME`, or a recipe file next to the song.
    pub instrument: String,
    /// Where to put it among the tracks, from 0. Default: last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// One track by its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
}

/// A new note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddNoteArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The bar `beat` counts from. Bars count from 1. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    /// When it starts, in beats after the bar line, from 0.
    pub beat: f64,
    /// What it plays: a note name (`"C5"`), a number in Hz, a list of
    /// these for a chord, or on a drum track a piece (`"kick"`).
    pub note: Value,
    /// How long it is held, in beats. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// How hard it is played, 0 to 1. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<f64>,
}

/// One note by its number on its track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The note's number in the track's `notes`, from 0.
    pub index: usize,
}

/// A note to move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveNoteArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The note's number in the track's `notes`, from 0.
    pub index: usize,
    /// Its new bar. Left out: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    /// Its new beat. Left out: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<f64>,
    /// Its new pitch (or chord, or drum piece). Left out: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Value>,
}

/// A note's new length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResizeNoteArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The note's number in the track's `notes`, from 0.
    pub index: usize,
    /// Its new length, in beats.
    pub length: f64,
}

/// A pattern to place on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacePatternArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The pattern's name, from the song's `patterns`.
    pub pattern: String,
    /// The bars it starts at, counted from 1: `[1, 2, 4]`.
    pub bars: Vec<u32>,
    /// Semitones up (or down, when negative). Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transpose: Option<i32>,
}

/// A placed pattern to move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovePatternArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The placement's number in the track's `play`, from 0.
    pub index: usize,
    /// The bars it starts at now, counted from 1.
    pub bars: Vec<u32>,
}

/// One placed pattern by its number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlacementArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// The track's name.
    pub track: String,
    /// The placement's number in the track's `play`, from 0.
    pub index: usize,
}

/// Looping, and the song's length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLoopArgs {
    /// The song file, relative to the project folder.
    pub file: String,
    /// True for a seamless loop, exactly `bars` long.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// The song's length in bars. Left out: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<u32>,
}

/// No arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// How much of the history to show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct HistoryArgs {
    /// At most this many steps of each list. Default 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------
// The commands.
// ---------------------------------------------------------------------

/// Everything a person or an agent can do to a project. As JSON, each is
/// an object whose `"do"` is the command's name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do")]
#[allow(missing_docs)]
pub enum Command {
    #[serde(rename = "file_new_sound")]
    FileNewSound(FileArgs),
    #[serde(rename = "file_new_song")]
    FileNewSong(FileArgs),
    #[serde(rename = "file_copy_from")]
    FileCopyFrom(CopyFromArgs),
    #[serde(rename = "file_rename")]
    FileRename(FileToArgs),
    #[serde(rename = "file_duplicate")]
    FileDuplicate(FileToArgs),
    #[serde(rename = "file_delete")]
    FileDelete(FileArgs),
    #[serde(rename = "file_restore")]
    FileRestore(FileArgs),
    #[serde(rename = "file_set_text")]
    FileSetText(SetTextArgs),
    #[serde(rename = "sound_set")]
    SoundSet(SetArgs),
    #[serde(rename = "sound_remove")]
    SoundRemove(RemoveArgs),
    #[serde(rename = "sound_add_layer")]
    SoundAddLayer(AddLayerArgs),
    #[serde(rename = "sound_remove_layer")]
    SoundRemoveLayer(LayerArgs),
    #[serde(rename = "sound_move_layer")]
    SoundMoveLayer(MoveLayerArgs),
    #[serde(rename = "sound_add_effect")]
    SoundAddEffect(AddEffectArgs),
    #[serde(rename = "sound_remove_effect")]
    SoundRemoveEffect(EffectArgs),
    #[serde(rename = "song_set")]
    SongSet(SetArgs),
    #[serde(rename = "song_remove")]
    SongRemove(RemoveArgs),
    #[serde(rename = "song_add_track")]
    SongAddTrack(AddTrackArgs),
    #[serde(rename = "song_remove_track")]
    SongRemoveTrack(TrackArgs),
    #[serde(rename = "song_add_note")]
    SongAddNote(AddNoteArgs),
    #[serde(rename = "song_move_note")]
    SongMoveNote(MoveNoteArgs),
    #[serde(rename = "song_resize_note")]
    SongResizeNote(ResizeNoteArgs),
    #[serde(rename = "song_remove_note")]
    SongRemoveNote(NoteArgs),
    #[serde(rename = "song_place_pattern")]
    SongPlacePattern(PlacePatternArgs),
    #[serde(rename = "song_move_pattern")]
    SongMovePattern(MovePatternArgs),
    #[serde(rename = "song_remove_pattern")]
    SongRemovePattern(PlacementArgs),
    #[serde(rename = "song_set_loop")]
    SongSetLoop(SetLoopArgs),
    #[serde(rename = "edit_undo")]
    EditUndo(NoArgs),
    #[serde(rename = "edit_redo")]
    EditRedo(NoArgs),
    #[serde(rename = "edit_history")]
    EditHistory(HistoryArgs),
}

/// One row of the command table.
pub struct Entry {
    /// The command's name: its `"do"`, its MCP tool name.
    pub name: &'static str,
    /// What it does, for people and agents.
    pub description: &'static str,
    /// True when it changes files (and so can be undone).
    pub changes_files: bool,
    /// The JSON Schema of its arguments.
    pub schema: fn() -> Value,
}

/// The arguments' schema, as an MCP tool's `inputSchema`.
fn schema_of<T: JsonSchema>() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(T)).expect("schema");
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
        obj.remove("description");
        obj.entry("properties").or_insert_with(|| json!({}));
    }
    v
}

const UNDO_NOTE: &str =
    "It becomes one step of the project's shared Undo history (edit_undo takes it back).";

/// Every command, in the order a person meets them.
pub const TABLE: &[Entry] = &[
    Entry { name: "file_new_sound", changes_files: true, schema: schema_of::<FileArgs>,
        description: "Make a new sound (a recipe) with one sine layer, named after the file. Refuses a file that exists." },
    Entry { name: "file_new_song", changes_files: true, schema: schema_of::<FileArgs>,
        description: "Make a new song: 120 BPM, 4 bars, one piano track with one note, named after the file. Refuses a file that exists." },
    Entry { name: "file_copy_from", changes_files: true, schema: schema_of::<CopyFromArgs>,
        description: "Make a new file as a copy of a preset (`preset:coin`), a built-in instrument (`instrument:piano`) or a file in the project. The copy is named after its new file and its description says what it was copied from, so nothing stale is left. A TOML copy keeps its comments." },
    Entry { name: "file_rename", changes_files: true, schema: schema_of::<FileToArgs>,
        description: "Give a file a new name (or folder). Its `name` field follows." },
    Entry { name: "file_duplicate", changes_files: true, schema: schema_of::<FileToArgs>,
        description: "Make a copy of a file under a new name, named after it." },
    Entry { name: "file_delete", changes_files: true, schema: schema_of::<FileArgs>,
        description: "Delete a file. A copy goes to the project's recycle bin, so edit_undo or file_restore brings it back." },
    Entry { name: "file_restore", changes_files: true, schema: schema_of::<FileArgs>,
        description: "Bring back the newest deleted copy of a file from the recycle bin. Refuses when the file exists." },
    Entry { name: "file_set_text", changes_files: true, schema: schema_of::<SetTextArgs>,
        description: "Replace a recipe's or song's whole text, kept exactly as written (comments too). The text must be valid. This is what a text box saves." },
    Entry { name: "sound_set", changes_files: true, schema: schema_of::<SetArgs>,
        description: "Set one field of a sound by its path, such as `layers[0].volume` or `master.normalize_lufs`. Every slider uses this. The result must be a valid recipe; if not, nothing is changed and the report says why." },
    Entry { name: "sound_remove", changes_files: true, schema: schema_of::<RemoveArgs>,
        description: "Take one field or list item out of a sound, such as `master.normalize` or `layers[0].filter`." },
    Entry { name: "sound_add_layer", changes_files: true, schema: schema_of::<AddLayerArgs>,
        description: "Add a layer to a sound." },
    Entry { name: "sound_remove_layer", changes_files: true, schema: schema_of::<LayerArgs>,
        description: "Remove one layer of a sound." },
    Entry { name: "sound_move_layer", changes_files: true, schema: schema_of::<MoveLayerArgs>,
        description: "Move a layer to another place among the layers." },
    Entry { name: "sound_add_effect", changes_files: true, schema: schema_of::<AddEffectArgs>,
        description: "Add an effect (echo, reverb, distortion, bitcrush) to a layer, or to the whole sound." },
    Entry { name: "sound_remove_effect", changes_files: true, schema: schema_of::<EffectArgs>,
        description: "Remove one effect from a layer, or from the whole sound." },
    Entry { name: "song_set", changes_files: true, schema: schema_of::<SetArgs>,
        description: "Set one field of a song by its path, such as `tempo`, `tracks[1].volume` or `tracks[0].notes[3].note`. The result must be a valid song." },
    Entry { name: "song_remove", changes_files: true, schema: schema_of::<RemoveArgs>,
        description: "Take one field or list item out of a song." },
    Entry { name: "song_add_track", changes_files: true, schema: schema_of::<AddTrackArgs>,
        description: "Add an empty track that plays an instrument." },
    Entry { name: "song_remove_track", changes_files: true, schema: schema_of::<TrackArgs>,
        description: "Remove a track and its notes." },
    Entry { name: "song_add_note", changes_files: true, schema: schema_of::<AddNoteArgs>,
        description: "Add a note (or a chord, or a drum hit) to a track." },
    Entry { name: "song_move_note", changes_files: true, schema: schema_of::<MoveNoteArgs>,
        description: "Move a note to another bar, beat or pitch." },
    Entry { name: "song_resize_note", changes_files: true, schema: schema_of::<ResizeNoteArgs>,
        description: "Make a note longer or shorter." },
    Entry { name: "song_remove_note", changes_files: true, schema: schema_of::<NoteArgs>,
        description: "Remove one note from a track." },
    Entry { name: "song_place_pattern", changes_files: true, schema: schema_of::<PlacePatternArgs>,
        description: "Place a pattern on a track at one or more bars, optionally moved up or down." },
    Entry { name: "song_move_pattern", changes_files: true, schema: schema_of::<MovePatternArgs>,
        description: "Change the bars a placed pattern starts at." },
    Entry { name: "song_remove_pattern", changes_files: true, schema: schema_of::<PlacementArgs>,
        description: "Take a placed pattern off a track (the pattern itself stays in the song)." },
    Entry { name: "song_set_loop", changes_files: true, schema: schema_of::<SetLoopArgs>,
        description: "Make a song a seamless loop (or not), and set its length in bars." },
    Entry { name: "edit_undo", changes_files: false, schema: schema_of::<NoArgs>,
        description: "Undo the newest step of the project's history, whoever made it (a person in the Studio or an agent). It refuses, changing nothing, if a file was changed outside Duck Echo since; that change is then the newest step." },
    Entry { name: "edit_redo", changes_files: false, schema: schema_of::<NoArgs>,
        description: "Redo the step undone last, whoever undid it. Refuses, changing nothing, if a file it would change was changed since." },
    Entry { name: "edit_history", changes_files: false, schema: schema_of::<HistoryArgs>,
        description: "List the project's history, newest first: each step's number, title, who made it and the files it changed, and the steps waiting for Redo." },
];

/// The table entry for a command name.
pub fn entry(name: &str) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.name == name)
}

/// A command's description, with the Undo note for those that change
/// files.
pub fn description(entry: &Entry) -> String {
    if entry.changes_files {
        format!("{} {UNDO_NOTE}", entry.description)
    } else {
        entry.description.to_string()
    }
}

/// Every command name.
pub fn names() -> Vec<String> {
    TABLE.iter().map(|e| e.name.to_string()).collect()
}

/// Read a command from its name and its arguments (a JSON object). Every
/// unknown or missing argument is reported at once, at
/// `<prefix><argument>` (`arguments.file` for MCP), with a suggestion.
pub fn parse(name: &str, args: &Value, prefix: &str) -> Result<Command, ToolError> {
    let Some(entry) = entry(name) else {
        let names = names();
        let mut issue = Issue::error(
            Code::UnknownVariant,
            format!("{prefix}do"),
            format!("there is no command `{name}`"),
        )
        .got(name)
        .one_of(names.iter().cloned());
        if let Some(s) = did_you_mean(name, &names) {
            issue = issue.suggest(s);
        }
        return Err(ToolError::issue(issue));
    };
    let empty = Map::new();
    let obj = match args {
        Value::Null => &empty,
        Value::Object(obj) => obj,
        other => {
            return Err(ToolError::issue(
                Issue::error(
                    Code::TypeMismatch,
                    prefix.trim_end_matches('.'),
                    "a command's arguments must be a JSON object",
                )
                .got(other.clone()),
            ))
        }
    };
    let schema = (entry.schema)();
    let props: Vec<String> = schema["properties"]
        .as_object()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    let mut report = Report::new();
    for key in obj.keys().filter(|k| k.as_str() != "do") {
        if !props.contains(key) {
            let mut issue = Issue::error(
                Code::UnknownField,
                format!("{prefix}{key}"),
                format!("`{name}` has no argument `{key}`"),
            )
            .got(key.clone())
            .one_of(props.iter().cloned());
            if let Some(s) = did_you_mean(key, &props) {
                issue = issue.suggest(s);
            }
            report.push(issue);
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let required = required.as_str().unwrap_or_default();
        if !obj.contains_key(required) {
            report.push(Issue::error(
                Code::MissingField,
                format!("{prefix}{required}"),
                format!("`{name}` needs the argument `{required}`"),
            ));
        }
    }
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    let mut full = obj.clone();
    full.insert("do".into(), Value::String(name.into()));
    serde_json::from_value(Value::Object(full)).map_err(|e| {
        let text = e.to_string();
        // serde names the field in backticks when it can.
        let field = text
            .split('`')
            .nth(1)
            .filter(|f| props.iter().any(|p| p == f))
            .unwrap_or("");
        ToolError::issue(Issue::error(
            Code::TypeMismatch,
            format!("{prefix}{field}").trim_end_matches('.').to_string(),
            format!("`{name}`: {text}"),
        ))
    })
}

impl Command {
    /// The command's name, its `"do"`.
    pub fn name(&self) -> String {
        serde_json::to_value(self).expect("serializes")["do"]
            .as_str()
            .expect("tagged")
            .to_string()
    }
}

// ---------------------------------------------------------------------
// Running.
// ---------------------------------------------------------------------

/// Run a command in `project`. `by` says who asked (it is kept in the
/// history); `stroke` joins this change to the newest step when they share
/// it (a slider drag). Argument errors are reported at `<prefix><name>`.
/// The answer is what [`Done`] or the history list holds, as JSON.
pub fn run(
    project: &Project,
    command: Command,
    by: Who,
    stroke: Option<u64>,
    prefix: &str,
) -> Result<Value, ToolError> {
    let r = Runner {
        project,
        by,
        stroke,
        prefix,
    };
    let done = match command {
        Command::FileNewSound(a) => r.new_file(&a.file, Kind::Recipe)?,
        Command::FileNewSong(a) => r.new_file(&a.file, Kind::Song)?,
        Command::FileCopyFrom(a) => r.copy_from(&a.from, &a.to)?,
        Command::FileRename(a) => r.rename(&a.file, &a.to, false)?,
        Command::FileDuplicate(a) => r.rename(&a.file, &a.to, true)?,
        Command::FileDelete(a) => r.delete(&a.file)?,
        Command::FileRestore(a) => r.restore(&a.file)?,
        Command::FileSetText(a) => r.set_text(&a.file, &a.text)?,
        Command::SoundSet(a) => r.edit_doc(&a.file, Kind::Recipe, set_title(&a), |doc| {
            set(doc, &a.path, a.value.clone())
        })?,
        Command::SoundRemove(a) => {
            r.edit_doc(&a.file, Kind::Recipe, format!("Remove {}", a.path), |doc| {
                remove(doc, &a.path)
            })?
        }
        Command::SoundAddLayer(a) => {
            let layer = a.layer.clone().unwrap_or_else(|| json!({"source": "sine"}));
            r.edit_doc(&a.file, Kind::Recipe, "Add a layer".into(), |doc| {
                insert(
                    list_at(doc, &["layers"], "layers")?,
                    a.at,
                    layer.clone(),
                    "at",
                )
            })?
        }
        Command::SoundRemoveLayer(a) => r.edit_doc(
            &a.file,
            Kind::Recipe,
            format!("Remove layer {}", a.index),
            |doc| take(list_at(doc, &["layers"], "layers")?, a.index, "index").map(|_| ()),
        )?,
        Command::SoundMoveLayer(a) => r.edit_doc(
            &a.file,
            Kind::Recipe,
            format!("Move layer {} to {}", a.from, a.to),
            |doc| {
                let layers = list_at(doc, &["layers"], "layers")?;
                let layer = take(layers, a.from, "from")?;
                insert(layers, Some(a.to), layer, "to")
            },
        )?,
        Command::SoundAddEffect(a) => {
            let title = match a.layer {
                Some(l) => format!("Add an effect to layer {l}"),
                None => "Add a master effect".into(),
            };
            r.edit_doc(&a.file, Kind::Recipe, title, |doc| {
                insert(effects_of(doc, a.layer)?, a.at, a.effect.clone(), "at")
            })?
        }
        Command::SoundRemoveEffect(a) => {
            let title = match a.layer {
                Some(l) => format!("Remove effect {} of layer {l}", a.index),
                None => format!("Remove master effect {}", a.index),
            };
            r.edit_doc(&a.file, Kind::Recipe, title, |doc| {
                take(effects_of(doc, a.layer)?, a.index, "index").map(|_| ())
            })?
        }
        Command::SongSet(a) => r.edit_doc(&a.file, Kind::Song, set_title(&a), |doc| {
            set(doc, &a.path, a.value.clone())
        })?,
        Command::SongRemove(a) => {
            r.edit_doc(&a.file, Kind::Song, format!("Remove {}", a.path), |doc| {
                remove(doc, &a.path)
            })?
        }
        Command::SongAddTrack(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!("Add track {} ({})", a.name, a.instrument),
            |doc| {
                let track = json!({"name": a.name, "instrument": a.instrument});
                insert(list_at(doc, &["tracks"], "tracks")?, a.at, track, "at")
            },
        )?,
        Command::SongRemoveTrack(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!("Remove track {}", a.track),
            |doc| {
                let i = track_index(doc, &a.track)?;
                take(list_at(doc, &["tracks"], "tracks")?, i, "track").map(|_| ())
            },
        )?,
        Command::SongAddNote(a) => {
            let mut note = Map::new();
            if let Some(bar) = a.bar {
                note.insert("bar".into(), json!(bar));
            }
            note.insert("beat".into(), json!(a.beat));
            note.insert("note".into(), a.note.clone());
            if let Some(length) = a.length {
                note.insert("length".into(), json!(length));
            }
            if let Some(velocity) = a.velocity {
                note.insert("velocity".into(), json!(velocity));
            }
            let title = format!(
                "Add note {} to {}, bar {} beat {}",
                compact(&a.note),
                a.track,
                a.bar.unwrap_or(1),
                a.beat
            );
            r.edit_doc(&a.file, Kind::Song, title, |doc| {
                let notes = track_list(doc, &a.track, "notes")?;
                notes.push(Value::Object(note.clone()));
                Ok(())
            })?
        }
        Command::SongMoveNote(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!("Move note {} of {}", a.index, a.track),
            |doc| {
                let note = item(track_list(doc, &a.track, "notes")?, a.index)?;
                let obj = note
                    .as_object_mut()
                    .ok_or_else(|| bad(r.prefix, "index", "that note is not an object"))?;
                if let Some(bar) = a.bar {
                    obj.insert("bar".into(), json!(bar));
                }
                if let Some(beat) = a.beat {
                    obj.insert("beat".into(), json!(beat));
                }
                if let Some(n) = &a.note {
                    obj.insert("note".into(), n.clone());
                }
                Ok(())
            },
        )?,
        Command::SongResizeNote(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!(
                "Make note {} of {} {} beats long",
                a.index, a.track, a.length
            ),
            |doc| {
                let note = item(track_list(doc, &a.track, "notes")?, a.index)?;
                note.as_object_mut()
                    .ok_or_else(|| bad(r.prefix, "index", "that note is not an object"))?
                    .insert("length".into(), json!(a.length));
                Ok(())
            },
        )?,
        Command::SongRemoveNote(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!("Remove note {} of {}", a.index, a.track),
            |doc| take(track_list(doc, &a.track, "notes")?, a.index, "index").map(|_| ()),
        )?,
        Command::SongPlacePattern(a) => {
            let mut placement = json!({"pattern": a.pattern, "bars": a.bars});
            if let Some(t) = a.transpose.filter(|t| *t != 0) {
                placement["transpose"] = json!(t);
            }
            r.edit_doc(
                &a.file,
                Kind::Song,
                format!(
                    "Place pattern {} on {} at bars {:?}",
                    a.pattern, a.track, a.bars
                ),
                |doc| {
                    track_list(doc, &a.track, "play")?.push(placement.clone());
                    Ok(())
                },
            )?
        }
        Command::SongMovePattern(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!(
                "Move pattern {} of {} to bars {:?}",
                a.index, a.track, a.bars
            ),
            |doc| {
                let placement = item(track_list(doc, &a.track, "play")?, a.index)?;
                placement["bars"] = json!(a.bars);
                Ok(())
            },
        )?,
        Command::SongRemovePattern(a) => r.edit_doc(
            &a.file,
            Kind::Song,
            format!("Take pattern {} off {}", a.index, a.track),
            |doc| take(track_list(doc, &a.track, "play")?, a.index, "index").map(|_| ()),
        )?,
        Command::SongSetLoop(a) => {
            let title = match (a.looping, a.bars) {
                (true, Some(b)) => format!("Loop the song, {b} bars"),
                (true, None) => "Loop the song".into(),
                (false, Some(b)) => format!("Stop looping, {b} bars"),
                (false, None) => "Stop looping".into(),
            };
            r.edit_doc(&a.file, Kind::Song, title, |doc| {
                let obj = doc.as_object_mut().expect("a song is an object");
                if a.looping {
                    obj.insert("loop".into(), json!(true));
                } else {
                    obj.remove("loop");
                }
                if let Some(bars) = a.bars {
                    obj.insert("bars".into(), json!(bars));
                }
                Ok(())
            })?
        }
        Command::EditUndo(_) => project.undo(by)?,
        Command::EditRedo(_) => project.redo(by)?,
        Command::EditHistory(a) => {
            return Ok(
                serde_json::to_value(project.history_list(a.limit.unwrap_or(20))?)
                    .expect("serializes"),
            )
        }
    };
    Ok(serde_json::to_value(done).expect("serializes"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Recipe,
    Song,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Recipe => "sound (a recipe)",
            Kind::Song => "song",
        }
    }
}

struct Runner<'a, 'r> {
    project: &'a Project<'r>,
    by: Who,
    stroke: Option<u64>,
    prefix: &'a str,
}

/// A problem with one argument.
fn bad(prefix: &str, arg: &str, message: impl Into<String>) -> ToolError {
    ToolError::issue(Issue::error(
        Code::InvalidArgument,
        format!("{prefix}{arg}"),
        message,
    ))
}

/// A value in a few characters, for a step's title.
fn compact(v: &Value) -> String {
    let text = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 40 {
        format!("{}...", text.chars().take(37).collect::<String>())
    } else {
        text
    }
}

fn set_title(a: &SetArgs) -> String {
    format!("Set {} to {}", a.path, compact(&a.value))
}

fn set(doc: &mut Value, path: &str, value: Value) -> Result<(), ToolError> {
    apply_edits(
        doc,
        &[EditOp {
            path: path.to_string(),
            value: Some(value),
            remove: false,
        }],
    )
    .map_err(|e| e.at("path"))
}

fn remove(doc: &mut Value, path: &str) -> Result<(), ToolError> {
    apply_edits(
        doc,
        &[EditOp {
            path: path.to_string(),
            value: None,
            remove: true,
        }],
    )
    .map_err(|e| e.at("path"))
}

/// The list at `keys` in `doc`, made empty if it is missing.
fn list_at<'d>(
    doc: &'d mut Value,
    keys: &[&str],
    name: &str,
) -> Result<&'d mut Vec<Value>, ToolError> {
    let mut v = doc;
    for k in keys {
        v = v
            .as_object_mut()
            .ok_or_else(|| {
                ToolError::failed(Code::Invalid, format!("`{name}` is not inside a table"))
            })?
            .entry(k.to_string())
            .or_insert_with(|| json!([]));
    }
    v.as_array_mut()
        .ok_or_else(|| ToolError::failed(Code::TypeMismatch, format!("`{name}` is not a list")))
}

fn effects_of(doc: &mut Value, layer: Option<usize>) -> Result<&mut Vec<Value>, ToolError> {
    match layer {
        None => list_at(doc, &["effects"], "effects"),
        Some(i) => {
            let layers = list_at(doc, &["layers"], "layers")?;
            let count = layers.len();
            let layer = layers.get_mut(i).ok_or_else(|| {
                ToolError::issue(
                    Issue::error(
                        Code::OutOfRange,
                        "layer",
                        format!("there is no layer {i}; the sound has {count}, numbered from 0"),
                    )
                    .got(i)
                    .range(0.0, count.saturating_sub(1) as f64),
                )
            })?;
            list_at(layer, &["effects"], "effects")
        }
    }
}

/// Put `value` into `list` at `at` (default: last).
fn insert(
    list: &mut Vec<Value>,
    at: Option<usize>,
    value: Value,
    arg: &str,
) -> Result<(), ToolError> {
    let at = at.unwrap_or(list.len());
    if at > list.len() {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                arg,
                format!(
                    "there are {} items, so the place can be 0 to {}",
                    list.len(),
                    list.len()
                ),
            )
            .got(at)
            .range(0.0, list.len() as f64),
        ));
    }
    list.insert(at, value);
    Ok(())
}

/// Take item `i` out of `list`.
fn take(list: &mut Vec<Value>, i: usize, arg: &str) -> Result<Value, ToolError> {
    if i >= list.len() {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                arg,
                format!(
                    "there is no item {i}; there are {}, numbered from 0",
                    list.len()
                ),
            )
            .got(i)
            .range(0.0, list.len().saturating_sub(1) as f64),
        ));
    }
    Ok(list.remove(i))
}

fn item(list: &mut [Value], i: usize) -> Result<&mut Value, ToolError> {
    let count = list.len();
    list.get_mut(i).ok_or_else(|| {
        ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                "index",
                format!("there is no item {i}; there are {count}, numbered from 0"),
            )
            .got(i)
            .range(0.0, count.saturating_sub(1) as f64),
        )
    })
}

fn track_index(doc: &Value, name: &str) -> Result<usize, ToolError> {
    let tracks: Vec<String> = doc["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect();
    tracks.iter().position(|t| t == name).ok_or_else(|| {
        let mut issue = Issue::error(
            Code::NotFound,
            "track",
            format!("the song has no track named `{name}`"),
        )
        .got(name)
        .one_of(tracks.iter().cloned());
        if let Some(s) = did_you_mean(name, &tracks) {
            issue = issue.suggest(s);
        }
        ToolError::issue(issue)
    })
}

/// A track's `notes` or `play` list, made empty if missing.
fn track_list<'d>(
    doc: &'d mut Value,
    track: &str,
    list: &str,
) -> Result<&'d mut Vec<Value>, ToolError> {
    let i = track_index(doc, track)?;
    let track = &mut doc["tracks"][i];
    list_at(track, &[list], list)
}

/// The kind of document in `text`.
fn kind_of(text: &str, format: Format) -> Kind {
    match document_kind(text, format) {
        DocumentKind::Recipe => Kind::Recipe,
        DocumentKind::Song => Kind::Song,
    }
}

/// Set (or add) the top-level `name` and `description` of a document's
/// text. In TOML only those two lines change, so comments and layout are
/// kept; JSON is written out again.
pub fn set_identity(
    text: &str,
    format: Format,
    name: &str,
    description: Option<&str>,
) -> Result<String, ToolError> {
    let quote = |s: &str| serde_json::to_string(s).expect("a string serializes");
    if format == Format::Json {
        let mut doc = parse_document(text, format)?;
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("name".into(), json!(name));
            if let Some(d) = description {
                obj.insert("description".into(), json!(d));
            }
        }
        return Ok(serde_json::to_string_pretty(&doc).expect("json") + "\n");
    }
    let is_key = |line: &str, key: &str| {
        let t = line.trim_start();
        t.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let mut out: Vec<String> = text.lines().map(str::to_string).collect();
    let mut wanted = vec![("name", name.to_string())];
    if let Some(d) = description {
        wanted.push(("description", d.to_string()));
    }
    for (key, value) in wanted {
        let line = format!("{key} = {}", quote(&value));
        let top_now = out
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .unwrap_or(out.len());
        match out[..top_now].iter().position(|l| is_key(l, key)) {
            Some(i) => out[i] = line,
            None => {
                // After `version`, or `name`, or at the start.
                let after = out[..top_now]
                    .iter()
                    .rposition(|l| is_key(l, "name") || is_key(l, "version"))
                    .map_or(0, |i| i + 1);
                out.insert(after, line);
            }
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

impl Runner<'_, '_> {
    /// A file argument, checked by the project's path rules, as the name
    /// the history uses. It must be a `.toml` or `.json` file.
    fn doc_path(&self, arg: &str, value: &str) -> Result<(String, Format), ToolError> {
        let key = format!("{}{arg}", self.prefix);
        let path = self.project.root().write(&key, value)?;
        if !crate::is_recipe_path(&path) {
            return Err(ToolError::issue(
                Issue::error(
                    Code::PathRefused,
                    key,
                    format!("`{value}` must end in .toml or .json"),
                )
                .got(value),
            ));
        }
        Ok((self.project.name_of(&path), Format::from_path(&path)))
    }

    /// Check a document's text as the file `rel` would hold it. Its
    /// warnings are returned; errors refuse the change.
    fn check(&self, rel: &str, text: &str) -> Result<(), ToolError> {
        let format = Format::from_path(Path::new(rel));
        match kind_of(text, format) {
            Kind::Recipe => duck_echo_core::load_str(text, format).map(|_| ())?,
            Kind::Song => {
                let root = self.project.root();
                let folder = rel
                    .rsplit_once('/')
                    .map_or(String::new(), |(f, _)| f.to_string());
                let files = root.song_files(folder);
                duck_echo_core::load_song_str(text, format, &crate::song::Library::new(&files))
                    .map(|_| ())?
            }
        }
        Ok(())
    }

    fn not_found(&self, arg: &str, rel: &str) -> ToolError {
        ToolError::issue(
            Issue::error(
                Code::NotFound,
                format!("{}{arg}", self.prefix),
                format!("`{rel}` does not exist in the project; list_files shows what does"),
            )
            .got(rel),
        )
    }

    fn exists(&self, arg: &str, rel: &str) -> ToolError {
        ToolError::issue(
            Issue::error(
                Code::AlreadyExists,
                format!("{}{arg}", self.prefix),
                format!("`{rel}` already exists; choose another name, or delete it first"),
            )
            .got(rel),
        )
    }

    fn stem(rel: &str) -> String {
        Path::new(rel)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sound".into())
    }

    fn new_file(&self, file: &str, kind: Kind) -> Result<Done, ToolError> {
        let (rel, format) = self.doc_path("file", file)?;
        let name = Self::stem(&rel);
        let toml_text = match kind {
            Kind::Recipe => format!(
                "version = 0\nname = {}\n\n[[layers]]\nsource = \"sine\"\nenvelope = {{ attack = 0.01, decay = 0.3 }}\npitch = {{ start = \"A4\" }}\n",
                serde_json::to_string(&name).expect("string")
            ),
            Kind::Song => format!(
                "version = 0\nname = {}\ntempo = 120\nbars = 4\n\n[[tracks]]\nname = \"melody\"\ninstrument = \"piano\"\nnotes = [{{ beat = 0, note = \"C4\" }}]\n",
                serde_json::to_string(&name).expect("string")
            ),
        };
        let text = match format {
            Format::Toml => toml_text,
            Format::Json => {
                document_text(Path::new(&rel), &parse_document(&toml_text, Format::Toml)?)?
            }
        };
        self.check(&rel, &text)?;
        let title = format!(
            "New {} {rel}",
            if kind == Kind::Song { "song" } else { "sound" }
        );
        self.project
            .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                if texts[0].is_some() {
                    return Err(self.exists("file", &rel));
                }
                Ok(Plan {
                    title: title.clone(),
                    writes: vec![(rel.clone(), Some(text.clone()))],
                })
            })
    }

    /// The text a `from` names, and words for "copy of ...".
    fn source(&self, from: &str) -> Result<(String, Format, String), ToolError> {
        let key = format!("{}from", self.prefix);
        if let Some(name) = from.strip_prefix(PRESET_PREFIX) {
            self.project.root().read(&key, from)?;
            let p = presets::get(name).map_err(|e| e.at(&key))?;
            return Ok((
                p.source.to_string(),
                Format::Toml,
                format!("the {name} preset"),
            ));
        }
        if let Some(name) = from.strip_prefix(INSTRUMENT_PREFIX) {
            self.project.root().read(&key, from)?;
            let i = instruments::get(name).map_err(|e| e.at(&key))?;
            return Ok((
                i.source.to_string(),
                Format::Toml,
                format!("the {name} instrument"),
            ));
        }
        let path = self.project.root().read(&key, from)?;
        let text = std::fs::read_to_string(&path).map_err(|e| {
            ToolError::failed(Code::Io, format!("cannot read {from}: {e}")).at(&key)
        })?;
        Ok((text, Format::from_path(&path), self.project.name_of(&path)))
    }

    /// `text` (in `from_format`) as a file `rel` would hold it, named
    /// after it.
    fn retarget(
        &self,
        text: &str,
        from_format: Format,
        rel: &str,
        description: Option<&str>,
    ) -> Result<String, ToolError> {
        let format = Format::from_path(Path::new(rel));
        let text = if format == from_format {
            text.to_string()
        } else {
            document_text(Path::new(rel), &parse_document(text, from_format)?)?
        };
        set_identity(&text, format, &Self::stem(rel), description)
    }

    fn copy_from(&self, from: &str, to: &str) -> Result<Done, ToolError> {
        let (rel, _) = self.doc_path("to", to)?;
        let (source, format, what) = self.source(from)?;
        let description = format!("Copy of {what}.");
        let text = self.retarget(&source, format, &rel, Some(&description))?;
        self.check(&rel, &text)?;
        self.project
            .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                if texts[0].is_some() {
                    return Err(self.exists("to", &rel));
                }
                Ok(Plan {
                    title: format!("Copy {what} to {rel}"),
                    writes: vec![(rel.clone(), Some(text.clone()))],
                })
            })
    }

    fn rename(&self, file: &str, to: &str, keep: bool) -> Result<Done, ToolError> {
        let (from_rel, from_format) = self.doc_path("file", file)?;
        let (to_rel, _) = self.doc_path("to", to)?;
        if from_rel == to_rel {
            return Err(bad(
                self.prefix,
                "to",
                "the new name is the same as the old one",
            ));
        }
        self.project.apply(
            &[from_rel.clone(), to_rel.clone()],
            self.by,
            self.stroke,
            |texts| {
                let Some(text) = &texts[0] else {
                    return Err(self.not_found("file", &from_rel));
                };
                if texts[1].is_some() {
                    return Err(self.exists("to", &to_rel));
                }
                let description = keep.then(|| format!("Copy of {from_rel}."));
                let new_text = self.retarget(text, from_format, &to_rel, description.as_deref())?;
                self.check(&to_rel, &new_text)?;
                let mut writes = vec![(to_rel.clone(), Some(new_text))];
                if !keep {
                    writes.insert(0, (from_rel.clone(), None));
                }
                Ok(Plan {
                    title: if keep {
                        format!("Duplicate {from_rel} as {to_rel}")
                    } else {
                        format!("Rename {from_rel} to {to_rel}")
                    },
                    writes,
                })
            },
        )
    }

    fn delete(&self, file: &str) -> Result<Done, ToolError> {
        let (rel, _) = self.doc_path("file", file)?;
        let mut deleted = String::new();
        let done =
            self.project
                .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                    let Some(text) = &texts[0] else {
                        return Err(self.not_found("file", &rel));
                    };
                    deleted = text.clone();
                    Ok(Plan {
                        title: format!("Delete {rel}"),
                        writes: vec![(rel.clone(), None)],
                    })
                })?;
        self.project.recycle(&rel, &deleted)?;
        Ok(done)
    }

    fn restore(&self, file: &str) -> Result<Done, ToolError> {
        let (rel, _) = self.doc_path("file", file)?;
        let Some(text) = self.project.recycled(&rel) else {
            return Err(ToolError::issue(
                Issue::error(
                    Code::NotFound,
                    format!("{}file", self.prefix),
                    format!("there is no deleted copy of `{rel}` in the recycle bin"),
                )
                .got(rel.clone()),
            ));
        };
        self.project
            .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                if texts[0].is_some() {
                    return Err(self.exists("file", &rel));
                }
                Ok(Plan {
                    title: format!("Restore {rel}"),
                    writes: vec![(rel.clone(), Some(text.clone()))],
                })
            })
    }

    fn set_text(&self, file: &str, text: &str) -> Result<Done, ToolError> {
        let (rel, format) = self.doc_path("file", file)?;
        self.check(&rel, text)
            .map_err(|e| e.at(&format!("{}text", self.prefix)))?;
        let new_kind = kind_of(text, format);
        self.project
            .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                let Some(old) = &texts[0] else {
                    return Err(self.not_found("file", &rel));
                };
                if kind_of(old, format) != new_kind {
                    return Err(bad(
                        self.prefix,
                        "text",
                        format!(
                            "`{rel}` is a {}; the new text is a {}",
                            kind_of(old, format).word(),
                            new_kind.word()
                        ),
                    ));
                }
                Ok(Plan {
                    title: format!("Edit the text of {rel}"),
                    writes: vec![(rel.clone(), Some(text.to_string()))],
                })
            })
    }

    /// Change a recipe or song through its data: read it, let `change`
    /// edit it, check the result, and write it.
    fn edit_doc(
        &self,
        file: &str,
        kind: Kind,
        title: String,
        mut change: impl FnMut(&mut Value) -> Result<(), ToolError>,
    ) -> Result<Done, ToolError> {
        let (rel, format) = self.doc_path("file", file)?;
        let prefix = self.prefix;
        self.project
            .apply(std::slice::from_ref(&rel), self.by, self.stroke, |texts| {
                let Some(text) = &texts[0] else {
                    return Err(self.not_found("file", &rel));
                };
                if kind_of(text, format) != kind {
                    return Err(bad(
                        prefix,
                        "file",
                        format!(
                            "`{rel}` is a {}, and this command works on a {}",
                            kind_of(text, format).word(),
                            kind.word()
                        ),
                    ));
                }
                let mut doc = parse_document(text, format)?;
                change(&mut doc).map_err(|mut e| {
                    // Argument names inside a command's own errors get the
                    // prefix; recipe paths stay as they are.
                    for issue in e.issues_mut() {
                        if matches!(
                            issue.path.as_str(),
                            "path" | "at" | "index" | "from" | "to" | "layer" | "track"
                        ) {
                            issue.path = format!("{prefix}{}", issue.path);
                        }
                    }
                    e
                })?;
                let new_text = document_text(Path::new(&rel), &doc)?;
                self.check(&rel, &new_text).map_err(|mut e| {
                    // Lines of a text nobody has seen would only confuse:
                    // the field path says where the problem is.
                    for issue in e.issues_mut() {
                        issue.line = None;
                        issue.column = None;
                    }
                    e
                })?;
                Ok(Plan {
                    title: format!("{title} in {rel}"),
                    writes: vec![(rel.clone(), Some(new_text))],
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_and_the_type_list_the_same_commands() {
        // Every table entry parses with minimal arguments into a command
        // of the same name, and no name is listed twice.
        let mut seen = Vec::new();
        for e in TABLE {
            assert!(!seen.contains(&e.name), "{} twice", e.name);
            seen.push(e.name);
            assert!(
                e.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{}",
                e.name
            );
            let schema = (e.schema)();
            assert_eq!(schema["type"], "object", "{}", e.name);
            assert_eq!(schema["additionalProperties"], false, "{}", e.name);
        }
        // The serde names of the enum: every variant's tag is in the table.
        let sample: Vec<Command> = vec![
            Command::FileNewSound(FileArgs {
                file: "a.toml".into(),
            }),
            Command::EditUndo(NoArgs {}),
            Command::EditHistory(HistoryArgs::default()),
        ];
        for c in sample {
            assert!(entry(&c.name()).is_some(), "{}", c.name());
        }
        assert_eq!(TABLE.len(), 30);
    }

    #[test]
    fn arguments_are_checked_with_paths_and_suggestions() {
        let e = parse(
            "sound_set",
            &json!({"file": "a.toml", "pth": "x"}),
            "arguments.",
        )
        .unwrap_err()
        .into_report();
        let paths: Vec<&str> = e.errors.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            ["arguments.pth", "arguments.path", "arguments.value"]
        );
        assert_eq!(e.errors[0].suggestion.as_deref(), Some("path"));
        let e = parse("sound_sett", &json!({}), "")
            .unwrap_err()
            .into_report();
        assert_eq!(e.errors[0].suggestion.as_deref(), Some("sound_set"));
        let e = parse(
            "song_add_note",
            &json!({"file": "s.toml", "track": "t", "beat": "one", "note": "C4"}),
            "arguments.",
        )
        .unwrap_err()
        .into_report();
        assert_eq!(e.errors[0].code, Code::TypeMismatch);
        assert!(parse("edit_undo", &Value::Null, "").is_ok());
    }

    #[test]
    fn identity_lines_change_and_comments_stay() {
        let text = "# A coin.\nversion = 0\nname = \"coin\" # old\ndescription = \"Classic coin pickup.\"\n\n[master]\nname = \"not this one\"\n";
        let out = set_identity(
            text,
            Format::Toml,
            "my coin",
            Some("Copy of the coin preset."),
        )
        .unwrap();
        assert_eq!(
            out,
            "# A coin.\nversion = 0\nname = \"my coin\"\ndescription = \"Copy of the coin preset.\"\n\n[master]\nname = \"not this one\"\n"
        );
        let bare = set_identity("version = 0\n[[layers]]\n", Format::Toml, "x", Some("y")).unwrap();
        assert_eq!(
            bare,
            "version = 0\nname = \"x\"\ndescription = \"y\"\n[[layers]]\n"
        );
    }
}
