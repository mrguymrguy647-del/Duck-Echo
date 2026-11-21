//! Test the Ear: let it really listen to two sounds Duck Echo knows, and
//! say what it heard and how long it took.
//!
//! The coin preset with "a coin pickup", and the Duck Craft night song (its
//! first 30 seconds) with "calm music". Both are heard afresh: the kept
//! answers are not used and nothing is kept, so Test shows how this Ear
//! listens on this PC now. Settings > Duck Ear > Test and `duck-echo ear
//! test` both use this.

use crate::ear::{lines_of, unavailable_words, word_for_match, EarKeeper, Listening, Missed};
use crate::ToolError;
use duck_echo_audio::AudioBuffer;
use duck_echo_core::Format;
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::time::Instant;

/// One sound of the test, and what the Ear said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestLine {
    /// The sound, in words: `coin`, `night song`.
    pub sound: String,
    /// The text it was asked about.
    pub text: String,
    /// The score model's word: `yes`, `no` or `unsure`.
    pub word: String,
    /// The score model's match, 0 to 1.
    pub matched: f64,
    /// How long hearing it and answering took, in seconds.
    pub seconds: f64,
}

/// What a test found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tested {
    /// The Ear's version.
    pub ear_version: String,
    /// Where its models run, in words.
    pub device: String,
    /// What it can do.
    pub capabilities: Vec<String>,
    /// One line for each sound.
    pub lines: Vec<TestLine>,
    /// True when the Ear said yes to both, as it should.
    pub passed: bool,
    /// All of it, in one or two sentences.
    pub summary: String,
}

/// Why the test could not be done.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestError {
    /// `not_installed`, `will_not_start` ... from the Ear, or `failed`.
    pub code: String,
    /// What happened and what to do, in words.
    pub message: String,
}

/// The sounds of the test: their names, the text each should fit, and a
/// way to make each.
const SOUNDS: &[(&str, &str)] = &[("coin", "a coin pickup"), ("night song", "calm music")];

fn sound_of(name: &str) -> Result<AudioBuffer, ToolError> {
    match name {
        "coin" => crate::load_audio(Path::new("preset:coin")),
        _ => {
            let example = crate::examples::get("duck_craft_night")?;
            let files = crate::song::files_next_to(Path::new("."));
            let loaded = duck_echo_core::load_song_str(
                example.source,
                Format::Toml,
                &crate::song::Library::new(&files),
            )?;
            let doc = crate::song::Document::Song(crate::song::SongRef {
                loaded,
                stem: example.name.to_string(),
            });
            Ok(crate::measure::measure_document(
                &doc,
                None,
                &crate::measure::Options::default(),
                "",
            )?
            .buffer)
        }
    }
}

/// Let the Ear `keeper` keeps listen to the two test sounds.
pub fn test(keeper: &EarKeeper) -> Result<Tested, TestError> {
    let failed = |message: String| TestError {
        code: "failed".to_string(),
        message,
    };
    let mut listening = Listening::begin(keeper).map_err(|u| TestError {
        code: u.reason.code().to_string(),
        message: unavailable_words(&u),
    })?;
    let lines_at = lines_of(&listening.hello);
    let mut lines = Vec::new();
    for (name, text) in SOUNDS {
        let buffer =
            sound_of(name).map_err(|e| failed(format!("the {name} would not render: {e}")))?;
        let sound = keeper
            .sound(&buffer)
            .map_err(|e| failed(format!("the {name} could not be written: {e}")))?;
        let started = Instant::now();
        let answer = listening
            .ask_again(&sound, "score", json!({ "texts": [text] }))
            .map_err(|missed| match missed {
                Missed::Unavailable(u) => TestError {
                    code: u.reason.code().to_string(),
                    message: unavailable_words(&u),
                },
                Missed::Failed(f) => TestError {
                    code: f.code().to_string(),
                    message: format!("The Ear could not answer about the {name}: {f}"),
                },
            })?;
        let matched = answer["scores"][0]["match"].as_f64().unwrap_or(0.0);
        lines.push(TestLine {
            sound: name.to_string(),
            text: text.to_string(),
            word: word_for_match(matched, lines_at).to_string(),
            matched: crate::ear::round(matched, 2),
            seconds: crate::ear::round(started.elapsed().as_secs_f64(), 1),
        });
    }
    let hello = &listening.hello;
    let device = hello
        .device_name
        .clone()
        .unwrap_or_else(|| hello.device.clone());
    let passed = lines.iter().all(|line| line.word == "yes");
    let heard: Vec<String> = lines
        .iter()
        .map(|l| {
            format!(
                "{}: \"{}\": {} ({:.2}), {:.1} s",
                l.sound, l.text, l.word, l.matched, l.seconds
            )
        })
        .collect();
    let summary = format!(
        "The Duck Ear {} listened on {}. {}.{}",
        hello.ear_version,
        device,
        heard.join("; "),
        if passed {
            ""
        } else if hello.stub {
            " (This is the stub Ear: it does not listen.)"
        } else {
            " It should say yes to both: if it does not, Repair installs it again."
        }
    );
    Ok(Tested {
        ear_version: hello.ear_version.clone(),
        device,
        capabilities: hello.capabilities.clone(),
        lines,
        passed,
        summary,
    })
}
