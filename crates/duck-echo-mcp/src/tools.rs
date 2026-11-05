//! The agent tools: definitions for `tools/list` and dispatch for
//! `tools/call`. All the work happens in `duck-echo-tools`; this file only
//! parses arguments, resolves paths against the root and shapes results.
//!
//! Every result is a JSON object, returned both as `structuredContent` and
//! as text. Failures set `isError` and return the validation report shape
//! (`{"ok": false, "errors": [...], "warnings": [...]}`), so an agent reads
//! a bad recipe and a bad argument the same way.

use crate::root::Root;
use duck_echo_audio::WavFormat;
use duck_echo_core::{load::did_you_mean, Code, Issue, Loaded, Report};
use duck_echo_tools::edit::{self, EditOp};
use duck_echo_tools::{presets, RecipeRef, ToolError};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

/// What a tool call produced.
pub struct Outcome {
    value: Value,
    is_error: bool,
}

impl Outcome {
    /// The MCP `CallToolResult`.
    pub fn into_result(self) -> Value {
        json!({
            "content": [{ "type": "text", "text": self.value.to_string() }],
            "structuredContent": self.value,
            "isError": self.is_error
        })
    }
}

fn report_value(e: ToolError) -> Value {
    let report = match e {
        ToolError::Invalid(r) => r,
        ToolError::Io(msg) => Report::from_error(Issue::error(Code::Other, "", msg)),
    };
    to_value(&report)
}

fn arg_error(name: &str, msg: impl Into<String>) -> ToolError {
    ToolError::Invalid(Report::from_error(Issue::error(
        Code::Other,
        format!("arguments.{name}"),
        msg,
    )))
}

const RECIPE_SOURCE: &str = "Give the recipe either inline as `recipe` (a JSON object) or as `path` (a .toml/.json file relative to the server root, or preset:NAME).";

fn recipe_props() -> Value {
    json!({
        "recipe": {
            "type": "object",
            "description": "A recipe as a JSON object, for example {\"version\": 0, \"layers\": [{\"source\": \"square\"}]}. get_schema documents every field."
        },
        "path": {
            "type": "string",
            "description": "A recipe file (.toml or .json) relative to the server root, or preset:NAME."
        }
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

/// The `tools/list` entries.
pub fn definitions() -> Vec<Value> {
    vec![
        tool(
            "get_schema",
            "Recipe schema",
            "The JSON Schema of a Duck Echo recipe: every field with its description, default and allowed range. Read it before writing recipes by hand.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "list_presets",
            "List presets",
            "List built-in presets: name, category (ui, pickup, weapon, impact, movement, alert, ambient), description and tags. Presets are good starting points; all peak at -3 dBFS.",
            json!({ "tag": { "type": "string", "description": "Only presets with this tag, for example `ui` or `explosion`." } }),
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
            "validate",
            "Validate a recipe",
            &format!("Check a recipe and report every problem in one call: unknown fields, bad values, wrong types and out-of-range numbers, each with its exact path (such as layers[0].pitch.start), the allowed range or options, and a suggestion. {RECIPE_SOURCE}"),
            recipe_props(),
            &[],
            true,
        ),
        tool(
            "create",
            "Create a recipe file",
            "Validate a recipe and save it as a file (.toml or .json by extension) relative to the server root. Refuses an invalid recipe (returning its report) and refuses to replace an existing file unless `overwrite` is true.",
            json!({
                "path": { "type": "string", "description": "File to write, ending in .toml or .json." },
                "recipe": { "type": "object", "description": "The recipe as a JSON object." },
                "overwrite": { "type": "boolean", "description": "Replace an existing file. Default false." }
            }),
            &["path", "recipe"],
            false,
        ),
        tool(
            "edit",
            "Edit a recipe",
            &format!("Change a recipe by path and validate the result. Each op is {{\"path\": \"layers[0].pitch.start\", \"value\": \"C5\"}} to set (an index equal to the array length appends; missing tables are created) or {{\"path\": \"effects[1]\", \"remove\": true}} to remove. Returns the edited recipe. A file is rewritten only if the result is valid (comments are not kept); an invalid result returns the report and changes nothing. {RECIPE_SOURCE}"),
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
            &format!("Render a recipe to a WAV file and return the analysis of the result (including a one-line `summary`). {RECIPE_SOURCE}"),
            with(recipe_props(), json!({
                "output": { "type": "string", "description": "WAV file to write, relative to the server root." },
                "seed": { "type": "integer", "minimum": 0, "description": "Override the recipe seed." },
                "format": { "type": "string", "enum": ["pcm16", "pcm24", "float32"], "description": "Sample format. Default pcm16." }
            })),
            &["output"],
            false,
        ),
        tool(
            "analyze",
            "Analyze a sound",
            "Measure a sound without listening: a one-line `summary`, duration, peak, loudness (LUFS with its method), clipping, silence, attack, decay, crest factor, spectral centroid (brightness), tonality and a pitch track. Give `path` (a WAV file, recipe file or preset:NAME) or an inline `recipe`, which is rendered in memory.",
            with(recipe_props(), json!({
                "path": { "type": "string", "description": "A WAV file, recipe file or preset:NAME, relative to the server root." }
            })),
            &[],
            true,
        ),
        tool(
            "compare",
            "Compare two sounds",
            "What changed from `a` to `b`: a one-line summary (for example \"b is 3.0 LU louder, 20% brighter\"), b-minus-a deltas, waveform correlation, and both analyses. Each side is a path (WAV, recipe file or preset:NAME) or an inline recipe object.",
            json!({
                "a": { "description": "First sound: a path string or a recipe object." },
                "b": { "description": "Second sound: a path string or a recipe object." }
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
    let defs = definitions();
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
        Some(_) => Err(arg_error(key, format!("`{key}` must be a string"))),
    }
}

fn req_str<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ToolError> {
    opt_str(args, key)?.ok_or_else(|| arg_error(key, format!("`{key}` is required")))
}

fn opt_bool(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(arg_error(key, format!("`{key}` must be true or false"))),
    }
}

/// Where a recipe comes from.
enum Source {
    Inline(Value),
    Path(PathBuf),
}

fn source(root: &Root, args: &Map<String, Value>) -> Result<Source, ToolError> {
    match (args.get("recipe"), opt_str(args, "path")?) {
        (Some(Value::Object(_)), Some(_)) => Err(arg_error(
            "recipe",
            "give either `recipe` or `path`, not both",
        )),
        (Some(v @ Value::Object(_)), None) => Ok(Source::Inline(v.clone())),
        (Some(_), _) => Err(arg_error("recipe", "`recipe` must be a JSON object")),
        (None, Some(p)) => Ok(Source::Path(root.read(p)?)),
        (None, None) => Err(arg_error("recipe", RECIPE_SOURCE)),
    }
}

fn load_source(src: &Source) -> Result<RecipeRef, ToolError> {
    match src {
        Source::Inline(v) => {
            let loaded = edit::load_value(v)?;
            let stem = loaded.recipe.name.clone().unwrap_or_else(|| "sound".into());
            Ok(RecipeRef { loaded, stem })
        }
        Source::Path(p) => duck_echo_tools::load_recipe(p),
    }
}

fn audio_of(root: &Root, v: &Value, key: &str) -> Result<duck_echo_audio::AudioBuffer, ToolError> {
    match v {
        Value::String(p) => duck_echo_tools::load_audio(&root.read(p)?),
        Value::Object(_) => {
            let loaded: Loaded = edit::load_value(v)?;
            duck_echo_tools::render_buffer(&loaded.recipe)
        }
        _ => Err(arg_error(
            key,
            format!("`{key}` must be a path string or a recipe object"),
        )),
    }
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
    if !definitions().iter().any(|d| d["name"] == name) {
        return None;
    }
    let result = check_args(name, args).and_then(|()| run(root, name, args));
    Some(match result {
        Ok(value) => Outcome {
            value,
            is_error: false,
        },
        Err(e) => Outcome {
            value: report_value(e),
            is_error: true,
        },
    })
}

fn run(root: &Root, name: &str, args: &Map<String, Value>) -> Result<Value, ToolError> {
    match name {
        "get_schema" => Ok(duck_echo_core::schema::recipe_schema()),
        "list_presets" => Ok(json!({ "presets": presets::list(opt_str(args, "tag")?)? })),
        "get_preset" => {
            let p = presets::get(req_str(args, "name")?)?;
            let info = presets::info(p)?;
            let doc = duck_echo_core::tree::parse_toml(p.source)
                .ok()
                .and_then(|t| t.to_json())
                .unwrap_or(Value::Null);
            Ok(json!({
                "name": info.name,
                "category": info.category,
                "description": info.description,
                "tags": info.tags,
                "recipe": doc,
                "toml": p.source
            }))
        }
        "validate" => match load_source(&source(root, args)?) {
            Ok(r) => Ok(to_value(&r.loaded.report)),
            // An invalid recipe is the answer here, not a failed call.
            Err(ToolError::Invalid(report)) => Ok(to_value(&report)),
            Err(e) => Err(e),
        },
        "create" => {
            let path_arg = req_str(args, "path")?;
            let path = root.write(path_arg)?;
            if !duck_echo_tools::is_recipe_path(&path) {
                return Err(arg_error("path", "`path` must end in .toml or .json"));
            }
            let Some(doc @ Value::Object(_)) = args.get("recipe") else {
                return Err(arg_error("recipe", "`recipe` must be a JSON object"));
            };
            if path.exists() && !opt_bool(args, "overwrite")?.unwrap_or(false) {
                return Err(arg_error(
                    "overwrite",
                    format!("{path_arg} already exists; pass \"overwrite\": true to replace it"),
                ));
            }
            let loaded = edit::load_value(doc)?;
            edit::write_document(&path, doc)?;
            Ok(json!({ "ok": true, "path": root.show(&path), "warnings": loaded.report.warnings }))
        }
        "edit" => {
            let src = source(root, args)?;
            let ops: Vec<EditOp> = serde_json::from_value(
                args.get("ops").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| {
                arg_error(
                    "ops",
                    format!(
                        "`ops` must be a list of {{path, value}} or {{path, remove: true}}: {e}"
                    ),
                )
            })?;
            let (mut doc, file) = match &src {
                Source::Inline(v) => (v.clone(), None),
                Source::Path(p)
                    if p.to_string_lossy()
                        .starts_with(duck_echo_tools::PRESET_PREFIX) =>
                {
                    let name =
                        p.to_string_lossy()[duck_echo_tools::PRESET_PREFIX.len()..].to_string();
                    let preset = presets::get(&name)?;
                    let doc = duck_echo_core::tree::parse_toml(preset.source)
                        .map_err(|i| ToolError::Invalid(Report::from_error(*i)))?
                        .to_json()
                        .unwrap_or(Value::Null);
                    (doc, None)
                }
                Source::Path(p) => (edit::read_document(p)?, Some(p.clone())),
            };
            let write = opt_bool(args, "write")?.unwrap_or(file.is_some());
            if write && file.is_none() {
                return Err(arg_error(
                    "write",
                    "only a recipe file can be written; use create to save an inline recipe or preset",
                ));
            }
            if let (true, Some(p)) = (write, &file) {
                if !duck_echo_tools::is_recipe_path(p) {
                    return Err(arg_error(
                        "path",
                        "only a .toml or .json recipe file can be rewritten; pass \"write\": false to edit without saving",
                    ));
                }
            }
            edit::apply_edits(&mut doc, &ops)?;
            let loaded = edit::load_value(&doc)?;
            if let (true, Some(p)) = (write, &file) {
                edit::write_document(p, &doc)?;
            }
            Ok(json!({
                "ok": true,
                "recipe": doc,
                "warnings": loaded.report.warnings,
                "written": write,
                "path": file.as_ref().map(|p| root.show(p))
            }))
        }
        "render" => {
            let r = load_source(&source(root, args)?)?;
            let out_arg = req_str(args, "output")?;
            let out = root.write(out_arg)?;
            if !duck_echo_tools::has_wav_extension(&out) {
                return Err(arg_error(
                    "output",
                    format!("`{out_arg}` must end in .wav; render never writes audio over another kind of file"),
                ));
            }
            let seed =
                match args.get("seed") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_u64().ok_or_else(|| {
                        arg_error("seed", "`seed` must be a whole number 0 or more")
                    })?),
                };
            let format = match opt_str(args, "format")? {
                None | Some("pcm16") => WavFormat::Pcm16,
                Some("pcm24") => WavFormat::Pcm24,
                Some("float32") => WavFormat::Float32,
                Some(other) => {
                    return Err(arg_error(
                        "format",
                        format!("unknown format `{other}`; use pcm16, pcm24 or float32"),
                    ))
                }
            };
            let done = duck_echo_tools::render_to_file(&r.loaded, &out, seed, format)?;
            let mut v = to_value(&done);
            v["output"] = json!(root.show(&out));
            Ok(v)
        }
        "analyze" => {
            let buffer = match (args.get("recipe"), opt_str(args, "path")?) {
                (Some(_), Some(_)) => {
                    return Err(arg_error(
                        "recipe",
                        "give either `recipe` or `path`, not both",
                    ))
                }
                (Some(v), None) => audio_of(root, v, "recipe")?,
                (None, Some(p)) => duck_echo_tools::load_audio(&root.read(p)?)?,
                (None, None) => {
                    return Err(arg_error(
                        "path",
                        "give `path` (WAV, recipe file or preset:NAME) or `recipe`",
                    ))
                }
            };
            Ok(to_value(&duck_echo_tools::analyze_buffer(&buffer)))
        }
        "compare" => {
            let a = audio_of(root, &args["a"], "a")?;
            let b = audio_of(root, &args["b"], "b")?;
            Ok(to_value(&duck_echo_tools::compare_buffers(&a, &b)))
        }
        "variations" => {
            let r = load_source(&source(root, args)?)?;
            let count = match args.get("count") {
                None | Some(Value::Null) => 8,
                Some(v) => v
                    .as_u64()
                    .and_then(|c| u32::try_from(c).ok())
                    .ok_or_else(|| arg_error("count", "`count` must be a whole number"))?,
            };
            let variation = match args.get("variation") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_f64()
                        .ok_or_else(|| arg_error("variation", "`variation` must be a number"))?
                        as f32,
                ),
            };
            let dir = opt_str(args, "output_dir")?
                .map(|d| root.write(d))
                .transpose()?;
            let v = duck_echo_tools::variations(&r, count, variation, dir.as_deref())?;
            let mut value = to_value(&v);
            if let Some(renders) = value["renders"].as_array_mut() {
                for item in renders {
                    if let Some(p) = item["output"].as_str() {
                        item["output"] = json!(root.show(std::path::Path::new(p)));
                    }
                }
            }
            Ok(value)
        }
        _ => unreachable!("checked by call"),
    }
}
