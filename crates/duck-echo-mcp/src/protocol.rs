//! The MCP stdio protocol: JSON-RPC 2.0 messages, one per line.
//!
//! Duck Echo only needs the tools part of MCP: `initialize`, `ping`,
//! `tools/list` and `tools/call`, plus ignoring notifications. That is
//! small enough to implement directly on `serde_json`, which keeps the
//! server synchronous and free of an async runtime. Everything
//! protocol-specific lives in this file, so moving to an SDK later only
//! replaces it.

use crate::tools;
use duck_echo_tools::root::Root;
use serde_json::{json, Value};

/// Protocol versions this server speaks, newest first. A client asking for
/// one of these gets it; otherwise the newest is offered.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Duck Echo makes sound effects and music from text recipes. Call get_guide first (no arguments): it returns the agent guide, which shows the whole workflow for sounds and music in one short page (the same text as docs/AGENT_GUIDE.md in the Duck Echo repository). get_guide also returns the reference pages: recipe_spec, song_spec, analysis, instruments and presets. A recipe is a JSON object (or a TOML file) with `version: 0` and a list of `layers`; get_schema documents every field. Typical loop: list_presets, get_preset to start from something that works, create to save it as a file, edit to change fields by path, render to make a WAV, analyze or compare to check it by numbers (loudness, brightness, attack, pitch track, energy over time, note onsets, and a one-line summary). analyze and render take `picture: true` for a spectrogram image, and `per_layer`, `solo` or `mute` to hear layers on their own. To hit a loudness target (list_presets gives one per category) set `master.normalize_lufs`. Loudness is gated BS.1770 for sounds of 400 ms or more and an ungated approximation for shorter ones (`~` in the summary), so compare short sounds with short sounds. validate reports every problem in one call with exact paths and suggestions. Anywhere a recipe path is accepted, `preset:NAME` names a built-in preset. Duck Echo also makes music: a song is a document with `tempo` and `tracks` instead of `layers`, where each track plays notes on an instrument, and an instrument is a recipe: a built-in one from list_instruments (piano, pad, bass, chip_lead, and the kit drums with notes kick, snare, hat, clap), one written in the song under `instruments`, a preset:NAME, or a recipe file next to the song. get_schema with kind \"song\" documents it. validate, create, edit, render, analyze and compare take a song wherever they take a recipe; `per_track: true` analyzes each track alone, and `solo`/`mute` take track names. File paths are relative to the server root; list_files shows what is there. A result with isError true is a failed call and holds a report: {ok: false, errors: [{code, path, message, allowed, suggestion}]}, where path is a recipe field or the argument at fault (arguments.output). validate is the exception: an invalid recipe is its answer, so it returns isError false with ok false. Every change to a file (create, edit, and the command tools such as sound_set, sound_add_layer, song_add_note, file_copy_from) is one step of the project's Undo history, shared with a person working in Duck Echo Studio at the same time: edit_undo takes back the newest step whoever made it, so read edit_history first, and file_copy_from names a copy of a preset after its new file.";

/// JSON-RPC error codes used here.
mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

/// Server state.
pub struct Server {
    root: Root,
}

impl Server {
    /// A server for `root`.
    pub fn new(root: Root) -> Self {
        Self { root }
    }

    /// Handle one line of input; returns the line to send back, if any.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(
                Value::Null,
                code::PARSE_ERROR,
                &format!("parse error: {e}"),
            )),
        };
        reply.map(|v| v.to_string())
    }

    fn handle(&mut self, msg: Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response to a request we never send, or junk.
            return match id {
                Some(id) if msg.get("result").is_none() && msg.get("error").is_none() => Some(
                    error(id, code::INVALID_REQUEST, "a request needs a `method`"),
                ),
                _ => None,
            };
        };
        // Notifications (no id) never get a reply.
        let id = id?;
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => success(id, initialize(&params)),
            "ping" => success(id, json!({})),
            "tools/list" => success(id, json!({ "tools": tools::definitions() })),
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(error(
                        id,
                        code::INVALID_PARAMS,
                        "tools/call needs a tool `name`",
                    ));
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if !args.is_object() {
                    return Some(error(
                        id,
                        code::INVALID_PARAMS,
                        "`arguments` must be an object",
                    ));
                }
                match tools::call(&self.root, name, &args) {
                    Some(outcome) => success(id, outcome.into_result()),
                    None => error(
                        id,
                        code::INVALID_PARAMS,
                        &format!("unknown tool `{name}`; tools/list shows the tools"),
                    ),
                }
            }
            other => error(
                id,
                code::METHOD_NOT_FOUND,
                &format!("method `{other}` is not supported"),
            ),
        })
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "duck-echo",
            "title": "Duck Echo",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": INSTRUCTIONS
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
