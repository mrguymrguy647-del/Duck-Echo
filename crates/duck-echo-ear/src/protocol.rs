//! The Duck Ear's protocol, version 1: JSON Lines over stdin and stdout.
//!
//! Each line is one JSON object. Duck Echo writes requests:
//!
//! ```json
//! {"id": 2, "method": "score", "params": {"wav": "...", "texts": ["a coin"]}}
//! ```
//!
//! and the Ear writes back, with the same `id`, either a result, an error,
//! or (before either of them) any number of progress lines:
//!
//! ```json
//! {"id": 2, "result": {"scores": [...]}}
//! {"id": 2, "error": {"code": "unreadable_audio", "message": "..."}}
//! {"id": 2, "progress": {"message": "loading the score model", "done": 0.5}}
//! ```
//!
//! Inside version 1, a field Duck Echo does not know is skipped, so a newer
//! Ear may add fields. A field Duck Echo needs that is missing makes the
//! line broken. (Recipes and tool calls stay strict: this rule is only for
//! the line between the two programs, which are updated at different
//! times.) See `docs/phase4-design.md`, section 3.

use serde::Deserialize;
use serde_json::Value;

/// The protocol version this Duck Echo speaks.
pub const PROTOCOL: u32 = 1;

/// The longest line Duck Echo reads from the Ear: 1 MB. Nothing the Ear
/// needs to say is anywhere near that long, so a longer line means
/// something went wrong.
pub const MAX_LINE: usize = 1024 * 1024;

/// What the Ear says about itself, in its answer to `hello`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hello {
    /// Its protocol version. Always [`PROTOCOL`] here: another number is
    /// refused before a `Hello` is made.
    pub protocol: u32,
    /// The Ear's own version, such as `0.1.0`.
    pub ear_version: String,
    /// What it can do: `score`, `describe`, `ask`. Names Duck Echo does not
    /// know are kept but never used.
    pub capabilities: Vec<String>,
    /// Where it runs its models: `cpu`, `vulkan`, or `none` (the stub).
    pub device: String,
    /// The device in words, such as "NVIDIA GeForce RTX 4060 (8 GB)".
    #[serde(default)]
    pub device_name: Option<String>,
    /// The graphics card's memory in MB, when the describe model runs on
    /// one.
    #[serde(default)]
    pub device_memory_mb: Option<u64>,
    /// Its models, one per role.
    pub models: Vec<Model>,
    /// True for the stub Ear, which never really listens.
    #[serde(default)]
    pub stub: bool,
    /// What the Ear cannot do yet, and why, by method: `{"describe": "..."}`.
    #[serde(default)]
    pub not_available: std::collections::BTreeMap<String, String>,
}

impl Hello {
    /// Can this Ear do `capability` (such as `describe`)?
    pub fn can(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// One of the Ear's models.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Model {
    /// What it is for: `score` or `describe`.
    pub role: String,
    /// Its name.
    pub name: String,
    /// The exact version of its files (for example a commit), so that an
    /// answer can name exactly which model gave it.
    pub revision: String,
    /// Is it loaded into memory now?
    #[serde(default)]
    pub loaded: bool,
    /// For a score model: from this `match` up, the score says yes. Set
    /// from the test set (docs/ear-check.md).
    #[serde(default)]
    pub yes_from: Option<f64>,
    /// For a score model: below this `match`, the score says no.
    #[serde(default)]
    pub no_below: Option<f64>,
    /// For a describe model: the Ear's size it belongs to (`full`,
    /// `large` ...).
    #[serde(default)]
    pub size: Option<String>,
}

/// A progress line: the Ear is still working on a request. It never gives
/// the request more time.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Progress {
    /// What it is doing, in words.
    pub message: String,
    /// How far along, from 0 to 1, when the Ear knows.
    #[serde(default)]
    pub done: Option<f64>,
}

/// One line from the Ear, read.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The answer to request `id`.
    Result {
        /// Which request.
        id: u64,
        /// The answer.
        result: Value,
    },
    /// Request `id` failed in the Ear.
    Error {
        /// Which request.
        id: u64,
        /// A short code, such as `unreadable_audio`.
        code: String,
        /// What went wrong, in words.
        message: String,
    },
    /// Request `id` is still being worked on.
    Progress {
        /// Which request.
        id: u64,
        /// How far it is.
        progress: Progress,
    },
}

impl Message {
    /// The request this line is about.
    pub fn id(&self) -> u64 {
        match self {
            Message::Result { id, .. }
            | Message::Error { id, .. }
            | Message::Progress { id, .. } => *id,
        }
    }
}

/// Read one line from the Ear. `Err` says why it is broken.
pub fn read_line(line: &str) -> Result<Message, String> {
    // A line from Windows may end in \r\n; the \n is already gone.
    let line = line.strip_suffix('\r').unwrap_or(line);
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("it is not JSON ({error})"))?;
    let Value::Object(object) = value else {
        return Err("it is JSON, but not an object".into());
    };
    let id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("it has no `id` that is a whole number")?;
    let result = object.get("result");
    let error = object.get("error");
    let progress = object.get("progress");
    match (result, error, progress) {
        (Some(result), None, None) => Ok(Message::Result {
            id,
            result: result.clone(),
        }),
        (None, Some(error), None) => {
            let code = error.get("code").and_then(Value::as_str);
            let message = error.get("message").and_then(Value::as_str);
            match (code, message) {
                (Some(code), Some(message)) => Ok(Message::Error {
                    id,
                    code: code.to_string(),
                    message: message.to_string(),
                }),
                _ => Err("its `error` needs a `code` and a `message`, both text".into()),
            }
        }
        (None, None, Some(progress)) => serde_json::from_value(progress.clone())
            .map(|progress| Message::Progress { id, progress })
            .map_err(|error| format!("its `progress` is not right ({error})")),
        _ => Err("it needs exactly one of `result`, `error` and `progress`".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_three_kinds_of_line_are_read() {
        assert_eq!(
            read_line(r#"{"id":2,"result":{"a":1}}"#),
            Ok(Message::Result {
                id: 2,
                result: json!({"a": 1})
            })
        );
        assert_eq!(
            read_line(r#"{"id":3,"error":{"code":"internal","message":"oops"}}"#),
            Ok(Message::Error {
                id: 3,
                code: "internal".into(),
                message: "oops".into()
            })
        );
        assert_eq!(
            read_line(r#"{"id":4,"progress":{"message":"loading"}}"#),
            Ok(Message::Progress {
                id: 4,
                progress: Progress {
                    message: "loading".into(),
                    done: None
                }
            })
        );
    }

    #[test]
    fn a_windows_line_end_and_unknown_fields_are_fine() {
        let line = "{\"id\":1,\"result\":{},\"new_thing\":true}\r";
        assert_eq!(read_line(line).unwrap().id(), 1);
    }

    #[test]
    fn broken_lines_say_why() {
        for (line, why) in [
            ("hello there", "not JSON"),
            ("[1, 2]", "not an object"),
            (r#"{"result":{}}"#, "no `id`"),
            (r#"{"id":-1,"result":{}}"#, "no `id`"),
            (r#"{"id":1}"#, "exactly one"),
            (r#"{"id":1,"result":{},"error":{}}"#, "exactly one"),
            (r#"{"id":1,"error":{"code":"x"}}"#, "`code` and a `message`"),
            (
                r#"{"id":1,"progress":{"done":0.5}}"#,
                "`progress` is not right",
            ),
        ] {
            let error = read_line(line).unwrap_err();
            assert!(error.contains(why), "{line}: {error}");
        }
    }

    #[test]
    fn hello_skips_fields_it_does_not_know_but_needs_its_own() {
        let hello: Hello = serde_json::from_value(json!({
            "protocol": 1, "ear_version": "0.1.0", "capabilities": ["score"],
            "device": "cpu", "models": [{"role": "score", "name": "m", "revision": "r"}],
            "something_new": [1, 2, 3]
        }))
        .unwrap();
        assert!(hello.can("score"));
        assert!(!hello.can("describe"));
        assert!(!hello.stub);
        assert!(serde_json::from_value::<Hello>(json!({"protocol": 1})).is_err());
    }
}
