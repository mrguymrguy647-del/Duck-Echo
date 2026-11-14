//! The agent tools: definitions for `tools/list` and dispatch for
//! `tools/call`. All the work happens in `duck-echo-tools`; this file only
//! parses arguments, resolves paths against the root and shapes results.
//!
//! Every result is a JSON object, returned both as `structuredContent` and
//! as text; `picture: true` adds a PNG image block after the text. A failed
//! call sets `isError` and returns the validation report shape
//! (`{"ok": false, "errors": [...], "warnings": [...]}`), so an agent reads
//! a bad recipe and a bad argument the same way. Every error has a specific
//! code and a path: a recipe field (`layers[0].volume`) or the argument at
//! fault (`arguments.output`). Messages show paths relative to the root.

use duck_echo_audio::WavFormat;
use duck_echo_core::{load::did_you_mean, Code, Issue, Report};
use duck_echo_tools::commands;
use duck_echo_tools::edit::{self, EditOp};
use duck_echo_tools::history::Who;
use duck_echo_tools::project::{Plan, Project};
use duck_echo_tools::root::Root;
use duck_echo_tools::{instruments, measure, presets, Document, Options, RecipeRef, ToolError};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// What a tool call produced.
pub struct Outcome {
    value: Value,
    images: Vec<Vec<u8>>,
    is_error: bool,
}

impl Outcome {
    /// The MCP `CallToolResult`.
    pub fn into_result(self) -> Value {
        let mut content = vec![json!({ "type": "text", "text": self.value.to_string() })];
        for png in &self.images {
            content.push(json!({
                "type": "image",
                "data": base64(png),
                "mimeType": "image/png"
            }));
        }
        json!({
            "content": content,
            "structuredContent": self.value,
            "isError": self.is_error
        })
    }
}

/// Standard base64 with padding, for image content.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The report for a failure: paths relative to the root in every message,
/// and no issue without a path.
fn report_value(root: &Root, e: ToolError) -> Value {
    let mut report = e.at("arguments").into_report();
    for issue in report.errors.iter_mut().chain(report.warnings.iter_mut()) {
        issue.message = root.scrub(&issue.message);
        if let Some(Value::String(s)) = &mut issue.got {
            *s = root.scrub(s);
        }
    }
    to_value(&report)
}

fn arg_error(name: &str, code: Code, msg: impl Into<String>) -> ToolError {
    ToolError::issue(Issue::error(code, format!("arguments.{name}"), msg))
}

const RECIPE_SOURCE: &str = "Give the recipe (or song) either inline as `recipe` (a JSON object) or as `path` (a .toml/.json file relative to the server root, preset:NAME or instrument:NAME).";

const SONG_NOTE: &str = "A song works the same way as a recipe here: a document with `tracks` is a song (get_schema with kind \"song\" documents it), one with `layers` is a recipe.";

const NOTE_CHECK_NOTE: &str = "For a song, `note_check` compares the notes written with the notes heard: each track is rendered alone and every written note is listened for at its place. `notes_heard` of `notes_written` is in the summary; `tracks[].problems` lists each note that is `missing`, `quiet`, at a `wrong_pitch` (with the written and heard pitch and `cents_off`) or `uncertain` (too short to read), with its `path`, `bar` and `beat`.";

const LOUDNESS_NOTE: &str = "Loudness (`loudness_lufs`) is BS.1770 gated for sounds of 400 ms or more and an ungated K-weighted approximation for shorter ones (`loudness_method`, and `~` in the summary); compare short sounds with short sounds.";

const EXTRAS_NOTE: &str = "Options: `picture: true` adds a PNG (level over time above a spectrogram) as an image block; `per_layer: true` (recipes) adds `layers`, each layer analyzed alone as heard in the mix; `per_track: true` (songs) adds `tracks`, each track analyzed alone the same way; `solo`/`mute` hear only some layers (by number) or tracks (by name).";

fn recipe_props() -> Value {
    json!({
        "recipe": {
            "type": "object",
            "description": "A recipe as a JSON object, for example {\"version\": 0, \"layers\": [{\"source\": \"square\"}]}, or a song (an object with `tracks`). get_schema documents every field."
        },
        "path": {
            "type": "string",
            "description": "A recipe or song file (.toml or .json) relative to the server root, or preset:NAME. list_files shows the files."
        }
    })
}

fn extras_props() -> Value {
    json!({
        "picture": { "type": "boolean", "description": "Add a PNG picture of the sound as an image block: level over time (peak and RMS, onset ticks) above a log-frequency spectrogram. `picture` in the result describes its axes. Default false." },
        "per_layer": { "type": "boolean", "description": "Also analyze each heard layer on its own, at the gain, timing and length it has in the mix, in `layers`. Recipes only. Default false." },
        "per_track": { "type": "boolean", "description": "Also analyze each heard track on its own, at the gain, timing and length it has in the mix, in `tracks`. Songs only. Default false." },
        "notes": { "type": "boolean", "description": "List every written note of a song with what was heard for it, in `note_check.tracks[].notes`, not only the problems. Songs only. Default false." },
        "solo": { "type": "array", "items": { "type": ["integer", "string"] }, "description": "Hear only these, at their level in the full mix: layer numbers (indices into `layers`) for a recipe, track names for a song." },
        "mute": { "type": "array", "items": { "type": ["integer", "string"] }, "description": "Silence these: layer numbers for a recipe, track names for a song." }
    })
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    props: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

/// The `tools/list` entries: the original tools, then one tool per
/// command (see `duck_echo_tools::commands`), with the command's name and
/// arguments.
pub fn definitions() -> Vec<Value> {
    let mut list = original_definitions();
    list.extend(commands::TABLE.iter().map(command_definition));
    list
}

/// A command as an MCP tool.
fn command_definition(entry: &commands::Entry) -> Value {
    let read_only = !entry.changes_files && entry.name == "edit_history";
    json!({
        "name": entry.name,
        "title": entry.name.replace('_', " "),
        "description": commands::description(entry),
        "inputSchema": (entry.schema)(),
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": false
        }
    })
}

/// The tools from before commands existed.
fn original_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_schema",
            "Recipe or song schema",
            "The JSON Schema of a Duck Echo recipe (a sound) or song (music): every field with its description, default and allowed range. Read it before writing one by hand.",
            json!({ "kind": { "type": "string", "enum": ["recipe", "song"], "description": "Which schema. Default `recipe`." } }),
            &[],
            true,
        ),
        tool(
            "list_presets",
            "List presets",
            "List built-in presets: name, category (ui, pickup, weapon, impact, movement, alert, ambient), description and tags. Presets are good starting points, each levelled to its category's loudness target (`target_lufs`: impact -16, weapon -17, alert -18, pickup -19, movement -20, ui -22, ambient -24 LUFS). To keep a target through edits, set `master.normalize_lufs` to it.",
            json!({ "tag": { "type": "string", "description": "Only presets with this tag, for example `ui` or `explosion`. A tag no preset has is a `not_found` error that suggests the closest real tag and lists them all in `allowed`." } }),
            &[],
            true,
        ),
        tool(
            "get_preset",
            "Get a preset",
            "A preset's recipe as a JSON object (ready to pass to create, edit or render) and as its TOML source with comments.",
            json!({ "name": { "type": "string", "description": "Preset name from list_presets." } }),
            &["name"],
            true,
        ),
        tool(
            "list_instruments",
            "List instruments",
            "List the built-in instruments a song's track can name as `instrument`: piano, pad, bass, chip_lead, and the drum kit `drums` (whose notes are `kick`, `snare`, `hat`, `clap`; each piece is also an instrument of its own). Each entry has its description, `home` (the note its recipe is written at), `range` (the lowest and highest notes it sounds right at) and `target_lufs` (how loud it is alone). Anywhere a recipe path is accepted, `instrument:NAME` names one, so analyze or render can hear it alone.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "get_instrument",
            "Get an instrument",
            "A built-in instrument's recipe as a JSON object (ready to copy into a song's `instruments` table and change) and as its TOML source with comments, plus its home note, range and loudness target.",
            json!({ "name": { "type": "string", "description": "Instrument name from list_instruments. A kit is not one recipe; ask for its pieces." } }),
            &["name"],
            true,
        ),
        tool(
            "list_files",
            "List files",
            "List files and folders under the server root, or under one folder in it: each entry's path (relative to the root, ready to pass to other tools), kind (folder, recipe, audio, image, link, other), size in bytes and, for WAV files, length in seconds. Hidden files are skipped; at most 500 entries (`truncated` says when more exist).",
            json!({
                "path": { "type": "string", "description": "Folder to list, relative to the server root. Default: the root." },
                "recursive": { "type": "boolean", "description": "Include subfolders. Default true." }
            }),
            &[],
            true,
        ),
        tool(
            "validate",
            "Validate a recipe or song",
            &format!("Check a recipe or a song and report every problem in one call: unknown fields, bad values, wrong types and out-of-range numbers, each with its exact path (such as layers[0].pitch.start or tracks[1].notes[3].note), the allowed range or options, and a suggestion. The report is the answer, so an invalid document returns isError false with `ok: false`; isError is true only when the call itself fails (a missing file, a bad argument). {RECIPE_SOURCE} {SONG_NOTE}"),
            recipe_props(),
            &[],
            true,
        ),
        tool(
            "create",
            "Create a recipe file",
            "Validate a recipe or a song and save it as a file (.toml or .json by extension) relative to the server root, creating folders as needed. Refuses an invalid document (returning its report) and refuses to replace an existing file unless `overwrite` is true.",
            json!({
                "path": { "type": "string", "description": "File to write, ending in .toml or .json." },
                "recipe": { "type": "object", "description": "The recipe or song as a JSON object." },
                "overwrite": { "type": "boolean", "description": "Replace an existing file. Default false." }
            }),
            &["path", "recipe"],
            false,
        ),
        tool(
            "edit",
            "Edit a recipe",
            &format!("Change a recipe by path and validate the result. Each op is {{\"path\": \"layers[0].pitch.start\", \"value\": \"C5\"}} to set (an index equal to the array length appends; missing tables are created) or {{\"path\": \"effects[1]\", \"remove\": true}} to remove. Returns the edited recipe. A file is rewritten only if the result is valid (comments are not kept); an invalid result returns the report and changes nothing. Songs are edited the same way, with paths such as tracks[1].notes[3].note. {RECIPE_SOURCE}"),
            with(recipe_props(), json!({
                "ops": {
                    "type": "array",
                    "description": "Edits, applied in order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "value": {},
                            "remove": { "type": "boolean" }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }
                },
                "write": { "type": "boolean", "description": "Save the result back to `path`. Default true for files, false for inline recipes and presets." }
            })),
            &["ops"],
            false,
        ),
        tool(
            "render",
            "Render to WAV",
            &format!("Render a recipe to a WAV file and return the analysis of the result (including a one-line `summary`, `energy_over_time` and note `onsets`) and any render warnings. {LOUDNESS_NOTE} To hit a loudness target in one render, set `master.normalize_lufs`; it measures the same way. {EXTRAS_NOTE} With solo or mute the WAV holds only the heard layers or tracks. A song's result also has `song` (tempo, bars, and `song_frames`, the exact length of its bars in samples); a song with `loop: true` renders to exactly that length with its tail folded onto its start, and `loop_seam.clean` says the end joins the start without a click. With `per_track` or `notes`, render also returns `note_check` as analyze does. {RECIPE_SOURCE} {SONG_NOTE}"),
            with(with(recipe_props(), json!({
                "output": { "type": "string", "description": "WAV file to write, relative to the server root." },
                "seed": { "type": "integer", "minimum": 0, "description": "Override the recipe seed." },
                "format": { "type": "string", "enum": ["pcm16", "pcm24", "float32"], "description": "Sample format. Default pcm16." }
            })), extras_props()),
            &["output"],
            false,
        ),
        tool(
            "analyze",
            "Analyze a sound",
            &format!("Measure a sound without listening: a one-line `summary`, duration, peak, loudness, clipping, silence, attack, decay, crest factor, spectral centroid (brightness), tonality, a pitch track, `energy_over_time` (RMS in 20 slices) and note `onsets` (time and pitch of each new note or burst). {LOUDNESS_NOTE} Give `path` (a WAV file, recipe or song file, or preset:NAME) or an inline `recipe`, which is rendered in memory. {EXTRAS_NOTE} {SONG_NOTE} {NOTE_CHECK_NOTE}"),
            with(with(recipe_props(), json!({
                "path": { "type": "string", "description": "A WAV file, recipe or song file, or preset:NAME, relative to the server root. list_files shows the files." }
            })), extras_props()),
            &[],
            true,
        ),
        tool(
            "compare",
            "Compare two sounds",
            &format!("What changed from `a` to `b`: a one-line summary (for example \"b is 3.0 LU louder, 20% brighter\"), b-minus-a deltas, waveform correlation, and both analyses. Each side is a path (WAV, recipe or song file, or preset:NAME) or an inline recipe or song object. The loudness delta is given only when both sides use the same loudness method. {LOUDNESS_NOTE}"),
            json!({
                "a": { "description": "First sound: a path string, or a recipe or song object." },
                "b": { "description": "Second sound: a path string, or a recipe or song object." }
            }),
            &["a", "b"],
            true,
        ),
        tool(
            "variations",
            "Render variations",
            &format!("Render seeds 1..count of a recipe and analyze each, to pick a variant by its numbers. Seeds change noise and, when `variation` is above 0, jitter pitch and timing; `seeds_matter: false` means they would all sound the same. {RECIPE_SOURCE}"),
            with(recipe_props(), json!({
                "count": { "type": "integer", "minimum": 1, "maximum": duck_echo_tools::MAX_VARIATIONS, "description": "How many seeds. Default 8." },
                "variation": { "type": "number", "minimum": 0, "maximum": 1, "description": "Override the recipe's variation amount." },
                "output_dir": { "type": "string", "description": "Write <name>_seed<N>.wav files here. Without it nothing is written." }
            })),
            &[],
            false,
        ),
    ]
}

/// Check argument names against the tool's input schema.
fn check_args(name: &str, args: &Map<String, Value>) -> Result<(), ToolError> {
    let defs = original_definitions();
    let def = defs.iter().find(|d| d["name"] == name).expect("known tool");
    let props = def["inputSchema"]["properties"].as_object().expect("props");
    let known: Vec<String> = props.keys().cloned().collect();
    let mut report = Report::new();
    for k in args.keys() {
        if !props.contains_key(k) {
            let mut issue = Issue::error(
                Code::UnknownField,
                format!("arguments.{k}"),
                format!("`{name}` has no argument `{k}`"),
            )
            .got(k.clone())
            .one_of(known.clone());
            if let Some(s) = did_you_mean(k, &known) {
                issue = issue.suggest(s);
            }
            report.push(issue);
        }
    }
    for r in def["inputSchema"]["required"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let r = r.as_str().unwrap_or_default();
        if !args.contains_key(r) {
            report.push(Issue::error(
                Code::MissingField,
                format!("arguments.{r}"),
                format!("`{name}` needs the argument `{r}`"),
            ));
        }
    }
    if report.is_ok() {
        Ok(())
    } else {
        Err(ToolError::Invalid(report))
    }
}

fn opt_str<'a>(args: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(arg_error(
            key,
            Code::TypeMismatch,
            format!("`{key}` must be a string"),
        )),
    }
}

fn req_str<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ToolError> {
    opt_str(args, key)?
        .ok_or_else(|| arg_error(key, Code::MissingField, format!("`{key}` is required")))
}

fn opt_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(arg_error(
            key,
            Code::TypeMismatch,
            format!("`{key}` must be true or false"),
        )),
    }
}

/// A `solo` or `mute` list: layer numbers or track names, kept as text
/// until the document's kind is known.
fn opt_selectors(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Value::String(name) => Ok(name.clone()),
                Value::Number(n) if n.is_u64() => Ok(n.to_string()),
                _ => Err(ToolError::issue(
                    Issue::error(
                        Code::TypeMismatch,
                        format!("arguments.{key}[{i}]"),
                        "give a layer number from 0 (for a recipe) or a track name (for a song)",
                    )
                    .got(v.clone()),
                )),
            })
            .collect(),
        Some(v) => Err(ToolError::issue(
            Issue::error(
                Code::TypeMismatch,
                format!("arguments.{key}"),
                format!("`{key}` must be a list of layer numbers, such as [0, 2], or of track names, such as [\"bass\"]"),
            )
            .got(v.clone()),
        )),
    }
}

/// The measuring options of a call. `always_check` is true for `analyze`,
/// which listens for every written note of a song; `render` does so only
/// with `per_track`, because the check renders every track once more.
fn options_of(args: &Map<String, Value>, always_check: bool) -> Result<Options, ToolError> {
    let per_track = opt_bool(args, "per_track")?.unwrap_or(false);
    Ok(Options {
        per_layer: opt_bool(args, "per_layer")?.unwrap_or(false),
        per_track,
        solo: opt_selectors(args, "solo")?,
        mute: opt_selectors(args, "mute")?,
        picture: opt_bool(args, "picture")?.unwrap_or(false),
        check_notes: always_check || per_track,
        list_notes: opt_bool(args, "notes")?.unwrap_or(false),
    })
}

/// Where a recipe comes from.
enum Source {
    Inline(Value),
    Path(PathBuf),
}

impl Source {
    /// The argument the recipe came from, for error paths.
    fn arg(&self) -> &'static str {
        match self {
            Source::Inline(_) => "arguments.recipe",
            Source::Path(_) => "arguments.path",
        }
    }
}

fn source(root: &Root, args: &Map<String, Value>) -> Result<Source, ToolError> {
    match (args.get("recipe"), opt_str(args, "path")?) {
        (Some(Value::Object(_)), Some(_)) => Err(arg_error(
            "recipe",
            Code::InvalidArgument,
            "give either `recipe` or `path`, not both",
        )),
        (Some(v @ Value::Object(_)), None) => Ok(Source::Inline(v.clone())),
        (Some(_), _) => Err(arg_error(
            "recipe",
            Code::TypeMismatch,
            "`recipe` must be a JSON object",
        )),
        (None, Some(p)) => Ok(Source::Path(root.read("arguments.path", p)?)),
        (None, None) => Err(arg_error("recipe", Code::MissingField, RECIPE_SOURCE)),
    }
}

/// How a song in `folder` reads its instrument files: see
/// [`Root::song_files`].
fn song_files(root: &Root, folder: String) -> impl Fn(&str) -> Result<PathBuf, ToolError> + '_ {
    root.song_files(folder)
}

/// The folder of a file inside the root, as a path relative to it.
fn folder_in_root(root: &Root, file: &std::path::Path) -> String {
    root.folder_of(file)
}

/// Load the recipe or song a tool call names.
fn load_doc(root: &Root, src: &Source) -> Result<Document, ToolError> {
    match src {
        Source::Inline(v) => {
            duck_echo_tools::load_document_value(v, &song_files(root, String::new()))
        }
        Source::Path(p) => {
            duck_echo_tools::load_document(p, &song_files(root, folder_in_root(root, p)))
        }
    }
    .map_err(|e| e.at(src.arg()))
}

/// Load a document that must be a recipe (for tools that make no sense on
/// a song).
fn load_source(root: &Root, src: &Source, tool: &str) -> Result<RecipeRef, ToolError> {
    match load_doc(root, src)? {
        Document::Recipe(r) => Ok(r),
        Document::Song(_) => Err(ToolError::issue(Issue::error(
            Code::InvalidArgument,
            src.arg(),
            format!("`{tool}` works on a recipe (one sound), and this is a song; to hear a song with another seed, use render with `seed`"),
        ))),
    }
}

fn audio_of(root: &Root, v: &Value, key: &str) -> Result<duck_echo_audio::AudioBuffer, ToolError> {
    let at = format!("arguments.{key}");
    match v {
        Value::String(p) => {
            let path = root.read(&at, p)?;
            let files = song_files(root, folder_in_root(root, &path));
            duck_echo_tools::load_audio_with(&path, &files).map_err(|e| e.at(&at))
        }
        Value::Object(_) => {
            let doc = duck_echo_tools::load_document_value(v, &song_files(root, String::new()))
                .map_err(|e| e.at(&at))?;
            measure::measure_document(&doc, None, &Options::default(), "arguments.")
                .map(|m| m.buffer)
                .map_err(|e| e.at(&at))
        }
        _ => Err(arg_error(
            key,
            Code::TypeMismatch,
            format!("`{key}` must be a path string, or a recipe or song object"),
        )),
    }
}

/// The TOML source of `preset:NAME` or `instrument:NAME`. `None` for
/// anything else (a real file).
fn built_in_source(path: &std::path::Path) -> Option<Result<&'static str, ToolError>> {
    let text = path.to_string_lossy();
    if let Some(name) = text.strip_prefix(duck_echo_tools::PRESET_PREFIX) {
        return Some(presets::get(name).map(|p| p.source));
    }
    if let Some(name) = text.strip_prefix(instruments::INSTRUMENT_PREFIX) {
        return Some(instruments::get(name).map(|i| i.source));
    }
    None
}

/// Serialize through text so `f32` fields keep their short form (`0.99`,
/// not `0.9900000095367432`), exactly as the CLI prints them.
fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::from_str(&serde_json::to_string(v).expect("serializes")).expect("valid json")
}

/// Run a tool. `None` for an unknown tool name.
pub fn call(root: &Root, name: &str, args: &Value) -> Option<Outcome> {
    let empty = Map::new();
    let args = args.as_object().unwrap_or(&empty);
    let result = if commands::entry(name).is_some() {
        // A command: the same arguments, checks and Undo step as the
        // Studio's buttons and the CLI's `duck-echo do`.
        commands::parse(name, &Value::Object(args.clone()), "arguments.")
            .and_then(|c| commands::run(&Project::new(root), c, Who::Agent, None, "arguments."))
            .map(|v| (v, Vec::new()))
    } else if original_definitions().iter().any(|d| d["name"] == name) {
        check_args(name, args).and_then(|()| run(root, name, args))
    } else {
        return None;
    };
    Some(match result {
        Ok((value, images)) => Outcome {
            value,
            images,
            is_error: false,
        },
        Err(e) => Outcome {
            value: report_value(root, e),
            images: Vec::new(),
            is_error: true,
        },
    })
}

fn run(
    root: &Root,
    name: &str,
    args: &Map<String, Value>,
) -> Result<(Value, Vec<Vec<u8>>), ToolError> {
    let plain = |v: Value| Ok((v, Vec::new()));
    match name {
        "get_schema" => match opt_str(args, "kind")? {
            None | Some("recipe") => plain(duck_echo_core::schema::recipe_schema()),
            Some("song") => plain(duck_echo_core::schema::song_schema()),
            Some(other) => Err(ToolError::issue(
                Issue::error(
                    Code::UnknownVariant,
                    "arguments.kind",
                    format!("unknown kind `{other}`"),
                )
                .got(other)
                .one_of(["recipe", "song"]),
            )),
        },
        "list_presets" => {
            let list = presets::list(opt_str(args, "tag")?).map_err(|e| e.at("arguments.tag"))?;
            plain(json!({ "presets": list }))
        }
        "get_preset" => {
            let p = presets::get(req_str(args, "name")?).map_err(|e| e.at("arguments.name"))?;
            let info = presets::info(p)?;
            let doc = duck_echo_core::tree::parse_toml(p.source)
                .ok()
                .and_then(|t| t.to_json())
                .unwrap_or(Value::Null);
            plain(json!({
                "name": info.name,
                "category": info.category,
                "description": info.description,
                "tags": info.tags,
                "recipe": doc,
                "toml": p.source
            }))
        }
        "list_instruments" => plain(json!({ "instruments": instruments::list()? })),
        "get_instrument" => {
            let i = instruments::get(req_str(args, "name")?).map_err(|e| e.at("arguments.name"))?;
            let info = instruments::info(i)?;
            let doc = duck_echo_core::tree::parse_toml(i.source)
                .ok()
                .and_then(|t| t.to_json())
                .unwrap_or(Value::Null);
            let mut out = to_value(&info);
            out["recipe"] = doc;
            out["toml"] = json!(i.source);
            plain(out)
        }
        "list_files" => {
            let dir = root.folder("arguments.path", opt_str(args, "path")?)?;
            let recursive = opt_bool(args, "recursive")?.unwrap_or(true);
            let listing = duck_echo_tools::files::list(&dir, root.dir(), recursive)
                .map_err(|e| e.at("arguments.path"))?;
            plain(to_value(&listing))
        }
        "validate" => {
            let src = source(root, args)?;
            match load_doc(root, &src) {
                Ok(doc) => plain(to_value(doc.report())),
                // An invalid recipe is the answer here, not a failed call.
                Err(ToolError::Invalid(report)) => plain(to_value(&report)),
                Err(e) => Err(e),
            }
        }
        "create" => {
            let path_arg = req_str(args, "path")?;
            let path = root.write("arguments.path", path_arg)?;
            if !duck_echo_tools::is_recipe_path(&path) {
                return Err(arg_error(
                    "path",
                    Code::PathRefused,
                    "`path` must end in .toml or .json",
                ));
            }
            let Some(doc @ Value::Object(_)) = args.get("recipe") else {
                return Err(arg_error(
                    "recipe",
                    Code::TypeMismatch,
                    "`recipe` must be a JSON object",
                ));
            };
            let overwrite = opt_bool(args, "overwrite")?.unwrap_or(false);
            let files = song_files(root, folder_in_root(root, &path));
            let loaded = duck_echo_tools::load_document_value(doc, &files)
                .map_err(|e| e.at("arguments.recipe"))?;
            let text = edit::document_text(&path, doc).map_err(|e| e.at("arguments.path"))?;
            // Saved through the project, so the change is a step of the
            // shared Undo history and can never overwrite another
            // program's change unseen.
            let project = Project::new(root);
            let rel = project.name_of(&path);
            project.apply(std::slice::from_ref(&rel), Who::Agent, None, |texts| {
                if texts[0].is_some() && !overwrite {
                    return Err(ToolError::issue(
                        Issue::error(
                            Code::AlreadyExists,
                            "arguments.overwrite",
                            format!(
                                "{path_arg} already exists; pass \"overwrite\": true to replace it"
                            ),
                        )
                        .suggest("true"),
                    ));
                }
                Ok(Plan {
                    title: format!("Create {rel}"),
                    writes: vec![(rel.clone(), Some(text.clone()))],
                })
            })?;
            plain(json!({
                "ok": true,
                "path": root.show(&path),
                "kind": loaded.kind(),
                "warnings": loaded.report().warnings
            }))
        }
        "edit" => {
            let src = source(root, args)?;
            let ops: Vec<EditOp> = serde_json::from_value(
                args.get("ops").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| {
                arg_error(
                    "ops",
                    Code::TypeMismatch,
                    format!(
                        "`ops` must be a list of {{path, value}} or {{path, remove: true}}: {e}"
                    ),
                )
            })?;
            let (mut doc, file) = match &src {
                Source::Inline(v) => (v.clone(), None),
                // A preset or built-in instrument: edit a copy of its
                // source. It is never written back.
                Source::Path(p) if built_in_source(p).is_some() => {
                    let source = built_in_source(p)
                        .expect("checked")
                        .map_err(|e| e.at("arguments.path"))?;
                    let doc = duck_echo_core::tree::parse_toml(source)
                        .map_err(|i| ToolError::Invalid(Report::from_error(*i)))?
                        .to_json()
                        .unwrap_or(Value::Null);
                    (doc, None)
                }
                Source::Path(p) => (
                    edit::read_document(p).map_err(|e| e.at("arguments.path"))?,
                    Some(p.clone()),
                ),
            };
            let write = opt_bool(args, "write")?.unwrap_or(file.is_some());
            if write && file.is_none() {
                return Err(arg_error(
                    "write",
                    Code::InvalidArgument,
                    "only a recipe file can be written; use create to save an inline recipe or preset",
                ));
            }
            if let (true, Some(p)) = (write, &file) {
                if !duck_echo_tools::is_recipe_path(p) {
                    return Err(arg_error(
                        "path",
                        Code::PathRefused,
                        "only a .toml or .json recipe file can be rewritten; pass \"write\": false to edit without saving",
                    ));
                }
            }
            edit::apply_edits(&mut doc, &ops).map_err(|e| e.at("arguments.ops"))?;
            let folder = file
                .as_ref()
                .map(|p| folder_in_root(root, p))
                .unwrap_or_default();
            let loaded = duck_echo_tools::load_document_value(&doc, &song_files(root, folder))
                .map_err(|e| e.at(src.arg()))?;
            if let (true, Some(p)) = (write, &file) {
                // Saved through the project: one Undo step. The edits are
                // made again on the file's text as it is under the lock, so
                // a change another program made meanwhile is kept.
                let project = Project::new(root);
                let rel = project.name_of(p);
                let format = duck_echo_core::Format::from_path(p);
                let folder = folder_in_root(root, p);
                let mut edited = doc.clone();
                project.apply(std::slice::from_ref(&rel), Who::Agent, None, |texts| {
                    let Some(text) = &texts[0] else {
                        return Err(arg_error(
                            "path",
                            Code::NotFound,
                            format!("{rel} no longer exists"),
                        ));
                    };
                    edited =
                        edit::parse_document(text, format).map_err(|e| e.at("arguments.path"))?;
                    edit::apply_edits(&mut edited, &ops).map_err(|e| e.at("arguments.ops"))?;
                    duck_echo_tools::load_document_value(
                        &edited,
                        &song_files(root, folder.clone()),
                    )
                    .map_err(|e| e.at("arguments.path"))?;
                    Ok(Plan {
                        title: format!(
                            "Edit {rel} ({} change{})",
                            ops.len(),
                            if ops.len() == 1 { "" } else { "s" }
                        ),
                        writes: vec![(rel.clone(), Some(edit::document_text(p, &edited)?))],
                    })
                })?;
                doc = edited;
            }
            plain(json!({
                "ok": true,
                "recipe": doc,
                "warnings": loaded.report().warnings,
                "written": write,
                "path": file.as_ref().map(|p| root.show(p))
            }))
        }
        "render" => {
            let src = source(root, args)?;
            let doc = load_doc(root, &src)?;
            let out_arg = req_str(args, "output")?;
            let out = root.write("arguments.output", out_arg)?;
            if !duck_echo_tools::has_wav_extension(&out) {
                return Err(arg_error(
                    "output",
                    Code::PathRefused,
                    format!("`{out_arg}` must end in .wav; render never writes audio over another kind of file"),
                ));
            }
            let seed = match args.get("seed") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or_else(|| {
                    arg_error(
                        "seed",
                        Code::TypeMismatch,
                        "`seed` must be a whole number 0 or more",
                    )
                })?),
            };
            let format = match opt_str(args, "format")? {
                None | Some("pcm16") => WavFormat::Pcm16,
                Some("pcm24") => WavFormat::Pcm24,
                Some("float32") => WavFormat::Float32,
                Some(other) => {
                    return Err(ToolError::issue(
                        Issue::error(
                            Code::UnknownVariant,
                            "arguments.format",
                            format!("unknown format `{other}`"),
                        )
                        .got(other)
                        .one_of(["pcm16", "pcm24", "float32"]),
                    ))
                }
            };
            let options = options_of(args, false)?;
            let done = duck_echo_tools::render_document_to_file(
                &doc,
                &out,
                seed,
                format,
                &options,
                "arguments.",
            )
            .map_err(|e| e.at("arguments.output"))?;
            let images = done.png.clone().into_iter().collect();
            let mut v = to_value(&done);
            v["output"] = json!(root.show(&out));
            Ok((v, images))
        }
        "analyze" => {
            let options = options_of(args, true)?;
            let measure_doc = |src: Source| -> Result<measure::Measured, ToolError> {
                let doc = load_doc(root, &src)?;
                Ok(
                    measure::measure_document(&doc, None, &options, "arguments.")
                        .map_err(|e| e.at(src.arg()))?
                        .measured,
                )
            };
            let measured = match (args.get("recipe"), opt_str(args, "path")?) {
                (Some(_), Some(_)) => {
                    return Err(arg_error(
                        "recipe",
                        Code::InvalidArgument,
                        "give either `recipe` or `path`, not both",
                    ))
                }
                (Some(v @ Value::Object(_)), None) => measure_doc(Source::Inline(v.clone()))?,
                (Some(_), None) => {
                    return Err(arg_error(
                        "recipe",
                        Code::TypeMismatch,
                        "`recipe` must be a JSON object",
                    ))
                }
                (None, Some(p)) => {
                    let path = root.read("arguments.path", p)?;
                    if duck_echo_tools::is_recipe_ref(&path) {
                        measure_doc(Source::Path(path))?
                    } else {
                        let extras = options.for_wav("arguments.")?;
                        let buffer = duck_echo_tools::load_audio(&path)
                            .map_err(|e| e.at("arguments.path"))?;
                        measure::measure_audio(&buffer, &extras)?
                    }
                }
                (None, None) => {
                    return Err(arg_error(
                        "path",
                        Code::MissingField,
                        "give `path` (WAV, recipe or song file, or preset:NAME) or `recipe`",
                    ))
                }
            };
            let images = measured.png.clone().into_iter().collect();
            Ok((to_value(&measured), images))
        }
        "compare" => {
            let a = audio_of(root, &args["a"], "a")?;
            let b = audio_of(root, &args["b"], "b")?;
            plain(to_value(&duck_echo_tools::compare_buffers(&a, &b)))
        }
        "variations" => {
            let src = source(root, args)?;
            let r = load_source(root, &src, "variations")?;
            let count = match args.get("count") {
                None | Some(Value::Null) => 8,
                Some(v) => v
                    .as_u64()
                    .and_then(|c| u32::try_from(c).ok())
                    .ok_or_else(|| {
                        arg_error(
                            "count",
                            Code::TypeMismatch,
                            "`count` must be a whole number",
                        )
                    })?,
            };
            if !(1..=duck_echo_tools::MAX_VARIATIONS).contains(&count) {
                return Err(ToolError::issue(
                    Issue::error(
                        Code::OutOfRange,
                        "arguments.count",
                        format!(
                            "count must be 1..={}, got {count}",
                            duck_echo_tools::MAX_VARIATIONS
                        ),
                    )
                    .got(count)
                    .range(1.0, duck_echo_tools::MAX_VARIATIONS as f64),
                ));
            }
            let variation = match args.get("variation") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let x = v.as_f64().ok_or_else(|| {
                        arg_error(
                            "variation",
                            Code::TypeMismatch,
                            "`variation` must be a number",
                        )
                    })?;
                    if !(0.0..=1.0).contains(&x) {
                        return Err(ToolError::issue(
                            Issue::error(
                                Code::OutOfRange,
                                "arguments.variation",
                                format!("variation must be 0..=1, got {x}"),
                            )
                            .got(x)
                            .range(0.0, 1.0),
                        ));
                    }
                    Some(x as f32)
                }
            };
            let dir = opt_str(args, "output_dir")?
                .map(|d| root.write("arguments.output_dir", d))
                .transpose()?;
            let v = duck_echo_tools::variations(&r, count, variation, dir.as_deref())
                .map_err(|e| e.at("arguments.output_dir"))?;
            let mut value = to_value(&v);
            if let Some(renders) = value["renders"].as_array_mut() {
                for item in renders {
                    if let Some(p) = item["output"].as_str() {
                        item["output"] = json!(root.show(std::path::Path::new(p)));
                    }
                }
            }
            plain(value)
        }
        _ => unreachable!("checked by call"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        for (raw, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), enc);
        }
    }
}
