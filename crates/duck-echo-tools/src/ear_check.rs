//! `duck-echo ear check`: how often the Duck Ear gets a test set right.
//!
//! A test set (`ear/testset/testset.toml`) lists sounds made by Duck Echo,
//! each with texts that are true for it and texts that are wrong for it.
//! The check renders every sound, asks the Ear to score every text of the
//! set against it (one `score` question per sound, through the answer
//! cache), and measures what docs/phase4-design.md, section 8, asks for:
//!
//! | Measure | What it means |
//! |---|---|
//! | Kind of sound | its true texts all score above all its wrong ones |
//! | Pairs | for one text, the sound it is true for scores above a sound it is wrong for |
//! | Soft and harsh | inside the group `coin`: each sound beats the other on its own texts |
//! | Calm and tense | the same, inside the group `mood` |
//! | Yes on true texts | the score alone says yes (the model's own line) |
//! | No on wrong texts | the score alone says no |
//! | Silence | it gets no yes for anything but silence |
//!
//! It only reads the Ear's answers; it changes nothing but the files it is
//! told to write (`ear-check.json` and `ear-check.md`).

use crate::ear::{lines_of, round, word_for_match, EarKeeper, Lines, Listening, Missed};
use crate::measure::{measure_document, Options};
use crate::ToolError;
use duck_echo_audio::AudioBuffer;
use duck_echo_core::Code;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// A test set, as `testset.toml` holds it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestSet {
    version: u32,
    sound: Vec<TestSound>,
}

/// One sound of a test set and what is true and wrong for it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestSound {
    file: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(rename = "true")]
    truths: Vec<String>,
    wrong: Vec<String>,
}

/// One measure of the check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Measure {
    /// Its name.
    pub name: String,
    /// What it counts, in words.
    pub what: String,
    /// How many were right.
    pub right: usize,
    /// Out of how many.
    pub of: usize,
    /// right / of.
    pub share: f64,
    /// The share it must reach before agents are told to rely on it.
    pub bar: f64,
    /// True when it reaches the bar.
    pub passed: bool,
}

/// One sound's answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundResult {
    /// The sound, as the test set names it.
    pub file: String,
    /// Each of its true texts and its match.
    pub true_matches: BTreeMap<String, f64>,
    /// Each of its wrong texts and its match.
    pub wrong_matches: BTreeMap<String, f64>,
    /// The words the Ear found closest.
    pub closest: Vec<String>,
    /// Wrong texts that scored at least as high as a true one.
    pub beaten_by: Vec<String>,
}

/// Which models `ear check` measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The score model only.
    Score,
    /// The describe model too: an error when the Ear has none.
    Describe,
    /// The score model, and the describe model when the Ear has one.
    All,
}

/// The describe model's answer about one text, and the verdict of both
/// models together.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextAnswer {
    /// `yes`, `no` or `partly`.
    pub answer: String,
    /// Its reason.
    pub reason: String,
    /// The score model's word and this answer together: `yes`, `no` or
    /// `unsure`.
    pub verdict: String,
}

/// What the describe model said about one sound.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Described {
    /// The sound, as the test set names it.
    pub file: String,
    /// What it heard, asked before any question.
    pub description: String,
    /// Each true text, answered.
    pub true_answers: BTreeMap<String, TextAnswer>,
    /// Each wrong text, answered.
    pub wrong_answers: BTreeMap<String, TextAnswer>,
}

/// A count of mistakes: the fewer the better.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mistakes {
    /// Its name.
    pub name: String,
    /// How many.
    pub count: usize,
    /// Out of how many.
    pub of: usize,
    /// count / of.
    pub share: f64,
}

/// A question asked a second time, of an Ear started afresh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskedTwice {
    /// The sound.
    pub file: String,
    /// `describe`, or the text asked about.
    pub question: String,
    /// The first answer.
    pub first: String,
    /// The second answer.
    pub second: String,
    /// True when both are the same words.
    pub same: bool,
}

/// The describe model's times, in seconds (the Ear's own).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DescribeTimes {
    /// Starting `llama-server` and loading the model, the first time.
    pub load: f64,
    /// Hearing a sound for the first time (`describe`): the mean.
    pub describe_mean: f64,
    /// The longest.
    pub describe_longest: f64,
    /// A yes or no question about a sound already heard: the mean.
    pub ask_mean: f64,
    /// The longest.
    pub ask_longest: f64,
}

/// The describe model's part of the check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DescribeReport {
    /// The describe model, `name@revision`.
    pub model: String,
    /// Where it ran: `cpu`, `vulkan` ...
    pub device: String,
    /// The device in words, such as the graphics card's name and memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// Its measures, with their bars.
    pub measures: Vec<Measure>,
    /// The verdict's mistakes: a true text given a no, a wrong one a yes.
    pub mistakes: Vec<Mistakes>,
    /// Its times.
    pub times: DescribeTimes,
    /// The same questions asked again of an Ear started afresh, with no
    /// answer cache: the words must be the same.
    pub asked_twice: Vec<AskedTwice>,
    /// Every sound's answers.
    pub results: Vec<Described>,
    /// The same, as Markdown.
    pub table: String,
}

/// What `ear check` answers (and writes as `ear-check.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckReport {
    /// Always true.
    pub ok: bool,
    /// The Ear's version.
    pub ear_version: String,
    /// The score model, `name@revision`.
    pub score_model: String,
    /// Where it ran.
    pub device: String,
    /// The model's yes and no lines.
    pub lines: Lines,
    /// The lines that serve both yes and no best on this test set (see
    /// `best_lines`), to copy into ear/convert/convert_clap.py's MODELS.
    pub suggested_lines: Lines,
    /// How many sounds and texts.
    pub sounds: usize,
    /// How many different texts.
    pub texts: usize,
    /// The measures, in the design's order.
    pub measures: Vec<Measure>,
    /// The Ear's own time per sound, in seconds.
    pub seconds_per_sound: f64,
    /// The describe model's part, when it was measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub describe: Option<DescribeReport>,
    /// Every sound's answers.
    pub results: Vec<SoundResult>,
    /// The same, as a Markdown table, for `docs/ear-check.md`.
    pub table: String,
    /// In one line.
    pub summary: String,
}

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::failed(Code::Invalid, message)
}

/// The sound a test set names, rendered (or one second of silence).
fn render(folder: &Path, file: &str) -> Result<AudioBuffer, ToolError> {
    if file == "silence" {
        return Ok(AudioBuffer {
            sample_rate: 44100,
            channels: 1,
            samples: vec![0.0; 44100],
        });
    }
    let at = format!("sound `{file}`");
    if let Some(name) = file.strip_prefix(crate::examples::EXAMPLE_PREFIX) {
        let example = crate::examples::get(name).map_err(|e| e.at(&at))?;
        let tree = duck_echo_core::tree::parse_toml(example.source)
            .map_err(|e| failed(format!("{at}: {e:?}")))?;
        let value = tree
            .to_json()
            .ok_or_else(|| failed(format!("{at}: not a document")))?;
        let doc = crate::load_document_value(&value, &|_: &str| {
            Err(failed("an example song names no files"))
        })
        .map_err(|e| e.at(&at))?;
        return Ok(measure_document(&doc, None, &Options::default(), "")?.buffer);
    }
    let path = if file.starts_with(crate::PRESET_PREFIX) {
        Path::new(file).to_path_buf()
    } else {
        folder.join(file)
    };
    crate::load_audio(&path).map_err(|e| e.at(&at))
}

fn measure(name: &str, what: &str, right: usize, of: usize, bar: f64) -> Measure {
    let share = if of == 0 {
        0.0
    } else {
        right as f64 / of as f64
    };
    Measure {
        name: name.into(),
        what: what.into(),
        right,
        of,
        share: round(share, 3),
        bar,
        passed: of > 0 && share >= bar,
    }
}

/// Run the test set at `testset` with the Ear `keeper` keeps: the score
/// model, and with `part` the describe model too. With `fresh`, the Ear
/// is asked everything, whatever the answer cache holds, and nothing is
/// kept: the times are then this run's own.
pub fn check(
    keeper: &EarKeeper,
    testset: &Path,
    part: Part,
    fresh: bool,
) -> Result<CheckReport, ToolError> {
    let text = std::fs::read_to_string(testset).map_err(|e| {
        ToolError::failed(
            Code::NotFound,
            format!("cannot read {}: {e}", testset.display()),
        )
    })?;
    let set: TestSet = toml::from_str(&text)
        .map_err(|e| failed(format!("{} is not a test set: {e}", testset.display())))?;
    if set.version != 1 {
        return Err(failed(format!(
            "test set version {} (this Duck Echo reads 1)",
            set.version
        )));
    }
    let folder = testset.parent().unwrap_or(Path::new("."));
    let mut texts: Vec<String> = set
        .sound
        .iter()
        .flat_map(|s| s.truths.iter().chain(&s.wrong).cloned())
        .collect();
    texts.sort();
    texts.dedup();

    let mut listening =
        Listening::begin(keeper).map_err(|u| failed(crate::ear::unavailable_words(&u)))?;
    listening.fresh = fresh;
    let with_describe = match part {
        Part::Score => false,
        Part::All => listening.hello.can("ask") && listening.hello.can("describe"),
        Part::Describe => {
            if !(listening.hello.can("ask") && listening.hello.can("describe")) {
                return Err(failed(format!(
                    "this Ear has no describe model to check: {}",
                    crate::ear::not_there(&listening.hello, "describe")
                )));
            }
            true
        }
    };
    let mut matches: Vec<BTreeMap<String, f64>> = Vec::new();
    let mut results = Vec::new();
    let mut seconds = 0.0;
    let mut heard_sounds = Vec::new();
    for sound in &set.sound {
        let buffer = render(folder, &sound.file)?;
        let heard = keeper.sound(&buffer)?;
        let answer = listening
            .ask(&heard, "score", json!({ "texts": texts }))
            .map_err(|missed| missed_error(missed, &sound.file))?;
        heard_sounds.push(heard);
        seconds += answer["seconds"].as_f64().unwrap_or(0.0);
        let by_text: BTreeMap<String, f64> = answer["scores"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| Some((s["text"].as_str()?.to_string(), s["match"].as_f64()?)))
            .collect();
        let get = |t: &String| by_text.get(t).copied().unwrap_or(0.0);
        let lowest_true = sound.truths.iter().map(get).fold(f64::INFINITY, f64::min);
        results.push(SoundResult {
            file: sound.file.clone(),
            true_matches: sound.truths.iter().map(|t| (t.clone(), get(t))).collect(),
            wrong_matches: sound.wrong.iter().map(|t| (t.clone(), get(t))).collect(),
            closest: answer["closest"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["text"].as_str().map(str::to_string))
                .collect(),
            beaten_by: sound
                .wrong
                .iter()
                .filter(|t| get(t) >= lowest_true)
                .cloned()
                .collect(),
        });
        matches.push(by_text);
    }
    let lines = lines_of(&listening.hello);
    let at = |i: usize, t: &str| matches[i].get(t).copied().unwrap_or(0.0);

    // Kind of sound.
    let kind_right = results.iter().filter(|r| r.beaten_by.is_empty()).count();
    // Pairs: a text true for one sound and wrong for another.
    let mut pairs = (0, 0);
    for (i, a) in set.sound.iter().enumerate() {
        for t in &a.truths {
            for (j, b) in set.sound.iter().enumerate() {
                if b.wrong.contains(t) {
                    pairs.1 += 1;
                    if at(i, t) > at(j, t) {
                        pairs.0 += 1;
                    }
                }
            }
        }
    }
    // Groups: each sound beats the others in its group on its own texts.
    let group = |name: &str| {
        let members: Vec<usize> = (0..set.sound.len())
            .filter(|&i| set.sound[i].group.as_deref() == Some(name))
            .collect();
        let mut count = (0, 0);
        for &i in &members {
            for t in &set.sound[i].truths {
                for &j in members.iter().filter(|&&j| j != i) {
                    count.1 += 1;
                    if at(i, t) > at(j, t) {
                        count.0 += 1;
                    }
                }
            }
        }
        count
    };
    let soft_harsh = group("coin");
    let calm_tense = group("mood");
    // Yes and no, from the score alone.
    let words = |pick: fn(&TestSound) -> &Vec<String>, wanted: &str| {
        let mut count = (0, 0);
        for (i, s) in set.sound.iter().enumerate() {
            for t in pick(s) {
                count.1 += 1;
                if word_for_match(at(i, t), lines) == wanted {
                    count.0 += 1;
                }
            }
        }
        count
    };
    let true_yes = words(|s| &s.truths, "yes");
    let wrong_no = words(|s| &s.wrong, "no");
    // Silence: no yes for anything but silence.
    let silence = set.sound.iter().position(|s| s.file == "silence");
    let silence_right = silence.map(|i| {
        let wrong_yes = set.sound[i]
            .wrong
            .iter()
            .filter(|t| word_for_match(at(i, t), lines) == "yes")
            .count();
        (usize::from(wrong_yes == 0), 1)
    });

    let mut measures = vec![
        measure(
            "Kind of sound",
            "every true text scores above every wrong one",
            kind_right,
            results.len(),
            0.9,
        ),
        measure(
            "Pairs",
            "for one text, the sound it is true for scores above a sound it is wrong for",
            pairs.0,
            pairs.1,
            0.9,
        ),
        measure(
            "Soft and harsh",
            "a soft and a harsh coin, each above the other on its own words",
            soft_harsh.0,
            soft_harsh.1,
            0.75,
        ),
        measure(
            "Calm and tense",
            "a calm and a tense loop, each above the other on its own words",
            calm_tense.0,
            calm_tense.1,
            0.8,
        ),
        measure(
            "Yes on true texts",
            "the score alone says yes to a true text",
            true_yes.0,
            true_yes.1,
            0.8,
        ),
        measure(
            "No on wrong texts",
            "the score alone says no to a wrong text",
            wrong_no.0,
            wrong_no.1,
            0.8,
        ),
    ];
    if let Some((right, of)) = silence_right {
        measures.push(measure(
            "Silence",
            "no yes for anything but silence",
            right,
            of,
            1.0,
        ));
    }
    let seconds_per_sound = round(seconds / set.sound.len().max(1) as f64, 3);
    let score_model = listening
        .hello
        .models
        .iter()
        .find(|m| m.role == "score")
        .map(|m| format!("{}@{}", m.name, m.revision))
        .unwrap_or_default();
    let mut said: Vec<(f64, bool)> = Vec::new(); // (match, true text?)
    for (i, s) in set.sound.iter().enumerate() {
        said.extend(s.truths.iter().map(|t| (at(i, t), true)));
        said.extend(s.wrong.iter().map(|t| (at(i, t), false)));
    }
    let suggested_lines = best_lines(&said);
    let passed = measures.iter().filter(|m| m.passed).count();
    let summary = format!(
        "{}: {passed} of {} measures reach their bar, on {} sounds and {} texts; {:.2} s a sound",
        score_model,
        measures.len(),
        results.len(),
        texts.len(),
        seconds_per_sound
    );
    let table = table_of(&measures, &results);
    let describe = if with_describe {
        Some(check_describe(
            keeper,
            &mut listening,
            &set,
            &heard_sounds,
            &matches,
            lines,
        )?)
    } else {
        None
    };
    let summary = match &describe {
        None => summary,
        Some(d) => format!(
            "{summary}. {}: {} of {} measures reach their bar; {:.1} s to hear a sound, {:.1} s a question",
            d.model,
            d.measures.iter().filter(|m| m.passed).count(),
            d.measures.len(),
            d.times.describe_mean,
            d.times.ask_mean
        ),
    };
    Ok(CheckReport {
        ok: true,
        ear_version: listening.hello.ear_version.clone(),
        score_model,
        device: listening.hello.device.clone(),
        lines,
        suggested_lines,
        sounds: results.len(),
        texts: texts.len(),
        measures,
        seconds_per_sound,
        describe,
        results,
        table,
        summary,
    })
}

fn missed_error(missed: Missed, file: &str) -> ToolError {
    match missed {
        Missed::Unavailable(u) => failed(crate::ear::unavailable_words(&u)),
        Missed::Failed(f) => failed(format!("the Ear failed on {file}: {f}")),
    }
}

/// How many sounds are asked about a second time.
const ASKED_TWICE: usize = 4;

/// The describe model's part: what it hears in each sound, its yes or no
/// to every true and wrong text, the verdict of both models, and its
/// times. The Ear's silence guard is off (`guard: false`), so silence and
/// noise go to the model like any sound: a model that says "yes, a coin"
/// to silence is not listening.
fn check_describe(
    keeper: &EarKeeper,
    listening: &mut Listening,
    set: &TestSet,
    sounds: &[crate::ear::Sound],
    matches: &[BTreeMap<String, f64>],
    lines: Lines,
) -> Result<DescribeReport, ToolError> {
    let ask_params = |text: &str| json!({"question": crate::ear::question_for(text), "answer": "yes_no", "guard": false});
    let mut results = Vec::new();
    let mut describe_times = Vec::new();
    let mut ask_times = Vec::new();
    let mut load = 0.0;
    for (i, sound) in set.sound.iter().enumerate() {
        let started = std::time::Instant::now();
        let described = listening
            .ask(&sounds[i], "describe", json!({"guard": false}))
            .map_err(|missed| missed_error(missed, &sound.file))?;
        let took = described["seconds"].as_f64().unwrap_or(0.0);
        if i == 0 {
            // The first question also starts llama-server.
            load = (started.elapsed().as_secs_f64() - took).max(0.0);
        }
        describe_times.push(took);
        let mut answers = |texts: &[String]| -> Result<BTreeMap<String, TextAnswer>, ToolError> {
            let mut out = BTreeMap::new();
            for text in texts {
                let asked = listening
                    .ask(&sounds[i], "ask", ask_params(text))
                    .map_err(|missed| missed_error(missed, &sound.file))?;
                ask_times.push(asked["seconds"].as_f64().unwrap_or(0.0));
                let answer = asked["answer"].as_str().unwrap_or_default().to_string();
                let matched = matches[i].get(text).copied().unwrap_or(0.0);
                let (verdict, _) = crate::ear::verdict(matched, Some(&answer), lines);
                out.insert(
                    text.clone(),
                    TextAnswer {
                        answer,
                        reason: asked["reason"].as_str().unwrap_or_default().to_string(),
                        verdict,
                    },
                );
            }
            Ok(out)
        };
        let true_answers = answers(&sound.truths)?;
        let wrong_answers = answers(&sound.wrong)?;
        results.push(Described {
            file: sound.file.clone(),
            description: described["text"].as_str().unwrap_or_default().to_string(),
            true_answers,
            wrong_answers,
        });
    }

    let count = |pick: fn(&Described) -> &BTreeMap<String, TextAnswer>,
                 word: fn(&TextAnswer) -> &String,
                 wanted: &str| {
        let all = results.iter().flat_map(|r| pick(r).values());
        (
            all.clone().filter(|a| word(a) == wanted).count(),
            all.count(),
        )
    };
    fn truths(r: &Described) -> &BTreeMap<String, TextAnswer> {
        &r.true_answers
    }
    fn wrongs(r: &Described) -> &BTreeMap<String, TextAnswer> {
        &r.wrong_answers
    }
    fn answer(a: &TextAnswer) -> &String {
        &a.answer
    }
    fn verdict(a: &TextAnswer) -> &String {
        &a.verdict
    }
    let (true_yes, true_all) = count(truths, answer, "yes");
    let (wrong_no, wrong_all) = count(wrongs, answer, "no");
    let mut measures = vec![
        measure(
            "Describe: yes on true texts",
            "the describe model says yes to a true text",
            true_yes,
            true_all,
            0.8,
        ),
        measure(
            "Describe: no on wrong texts",
            "the describe model says no to a wrong text",
            wrong_no,
            wrong_all,
            0.8,
        ),
    ];
    // Silence and noise: no yes to a wrong text (a coin, music ...).
    let not_listening = |file: &str| {
        results.iter().find(|r| r.file == file).map(|r| {
            let yes = r
                .wrong_answers
                .values()
                .filter(|a| a.answer == "yes")
                .count();
            (r.wrong_answers.len() - yes, r.wrong_answers.len())
        })
    };
    for (name, file, what) in [
        (
            "Describe: silence",
            "silence",
            "no yes for a sound when there is only silence",
        ),
        (
            "Describe: noise",
            "noise.toml",
            "no yes for a sound when there is only noise",
        ),
    ] {
        if let Some((right, of)) = not_listening(file) {
            measures.push(measure(name, what, right, of, 1.0));
        }
    }
    let (verdict_yes, _) = count(truths, verdict, "yes");
    let (verdict_no, _) = count(wrongs, verdict, "no");
    measures.push(measure(
        "Verdict: yes on true texts",
        "both models say yes to a true text",
        verdict_yes,
        true_all,
        0.8,
    ));
    measures.push(measure(
        "Verdict: no on wrong texts",
        "both models say no to a wrong text",
        verdict_no,
        wrong_all,
        0.8,
    ));
    let mistakes = |name: &str, count: usize, of: usize| Mistakes {
        name: name.into(),
        count,
        of,
        share: round(count as f64 / of.max(1) as f64, 3),
    };
    let mistakes = vec![
        mistakes(
            "Describe: a true text said no",
            count(truths, answer, "no").0,
            true_all,
        ),
        mistakes(
            "Describe: a wrong text said yes",
            count(wrongs, answer, "yes").0,
            wrong_all,
        ),
        mistakes(
            "Verdict: a true text said no",
            count(truths, verdict, "no").0,
            true_all,
        ),
        mistakes(
            "Verdict: a wrong text said yes",
            count(wrongs, verdict, "yes").0,
            wrong_all,
        ),
    ];

    // The same questions again, of an Ear started afresh and with no
    // answer cache: the words must be the same.
    keeper.stop();
    let mut asked_twice = Vec::new();
    for (i, sound) in set.sound.iter().enumerate().take(ASKED_TWICE) {
        let again = listening
            .ask_again(&sounds[i], "describe", json!({"guard": false}))
            .map_err(|missed| missed_error(missed, &sound.file))?;
        let second = again["text"].as_str().unwrap_or_default().to_string();
        asked_twice.push(AskedTwice {
            file: sound.file.clone(),
            question: "describe".into(),
            same: second == results[i].description,
            first: results[i].description.clone(),
            second,
        });
        if let Some(text) = sound.truths.first() {
            let again = listening
                .ask_again(&sounds[i], "ask", ask_params(text))
                .map_err(|missed| missed_error(missed, &sound.file))?;
            let words = |answer: &str, reason: &str| format!("{answer}: {reason}");
            let first = &results[i].true_answers[text];
            let first = words(&first.answer, &first.reason);
            let second = words(
                again["answer"].as_str().unwrap_or_default(),
                again["reason"].as_str().unwrap_or_default(),
            );
            asked_twice.push(AskedTwice {
                file: sound.file.clone(),
                question: text.clone(),
                same: first == second,
                first,
                second,
            });
        }
    }
    let same = asked_twice.iter().filter(|a| a.same).count();
    measures.push(measure(
        "Same words twice",
        "a question asked again of an Ear started afresh gets the same words",
        same,
        asked_twice.len(),
        1.0,
    ));

    let mean = |times: &[f64]| times.iter().sum::<f64>() / times.len().max(1) as f64;
    let longest = |times: &[f64]| times.iter().copied().fold(0.0, f64::max);
    let times = DescribeTimes {
        load: round(load, 1),
        describe_mean: round(mean(&describe_times), 2),
        describe_longest: round(longest(&describe_times), 2),
        ask_mean: round(mean(&ask_times), 2),
        ask_longest: round(longest(&ask_times), 2),
    };
    let model = listening
        .hello
        .models
        .iter()
        .find(|m| m.role == "describe")
        .map(|m| format!("{}@{}", m.name, m.revision))
        .unwrap_or_default();
    let table = describe_table(&measures, &mistakes, &times, &asked_twice, &results);
    Ok(DescribeReport {
        model,
        device: listening.hello.device.clone(),
        device_name: listening.hello.device_name.clone(),
        measures,
        mistakes,
        times,
        asked_twice,
        results,
        table,
    })
}

/// The describe model's measures, times and wrong answers, as Markdown.
fn describe_table(
    measures: &[Measure],
    mistakes: &[Mistakes],
    times: &DescribeTimes,
    asked_twice: &[AskedTwice],
    results: &[Described],
) -> String {
    let mut out = measures_table(measures);
    out.push_str("\n| Mistakes (fewer is better) | Count | Share |\n|---|---|---|\n");
    for m in mistakes {
        out.push_str(&format!(
            "| {} | {} of {} | {:.0}% |\n",
            m.name,
            m.count,
            m.of,
            m.share * 100.0
        ));
    }
    out.push_str(&format!(
        "\n| Time | Seconds |\n|---|---|\n| Starting llama-server and loading the model | {:.1} |\n\
         | Hearing a sound the first time (mean, longest) | {:.1}, {:.1} |\n\
         | A yes or no question about a sound already heard (mean, longest) | {:.1}, {:.1} |\n",
        times.load, times.describe_mean, times.describe_longest, times.ask_mean, times.ask_longest
    ));
    for a in asked_twice.iter().filter(|a| !a.same) {
        out.push_str(&format!(
            "\nNot the same twice: `{}`, {}: first \"{}\", then \"{}\".\n",
            a.file, a.question, a.first, a.second
        ));
    }
    out.push_str(
        "\n| Sound | What the describe model heard | True texts not given a yes | Wrong texts given a yes |\n|---|---|---|---|\n",
    );
    for r in results {
        let list = |answers: &BTreeMap<String, TextAnswer>, bad: fn(&str) -> bool| {
            answers
                .iter()
                .filter(|(_, a)| bad(&a.answer))
                .map(|(t, a)| format!("{t} ({})", a.answer))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            r.file,
            r.description.replace('|', "/"),
            list(&r.true_answers, |a| a != "yes"),
            list(&r.wrong_answers, |a| a == "yes"),
        ));
    }
    out
}

fn measures_table(measures: &[Measure]) -> String {
    let mut out =
        String::from("| Measure | Right | Share | Bar | Reached |\n|---|---|---|---|---|\n");
    for m in measures {
        out.push_str(&format!(
            "| {} | {} of {} | {:.0}% | {:.0}% | {} |\n",
            m.name,
            m.right,
            m.of,
            m.share * 100.0,
            m.bar * 100.0,
            if m.passed { "yes" } else { "no" }
        ));
    }
    out
}

/// The lines that serve both bars best on `said` (each text's match, and
/// whether it was true): the highest of "yes on true texts" and "no on
/// wrong texts" at their lower, with each kind of mistake (a true text
/// said no, a wrong one said yes) at most 10%. Ties keep the first found.
fn best_lines(said: &[(f64, bool)]) -> Lines {
    let truths = said.iter().filter(|s| s.1).count().max(1) as f64;
    let wrongs = said.iter().filter(|s| !s.1).count().max(1) as f64;
    let mut best = (
        f64::NEG_INFINITY,
        Lines {
            yes_from: crate::ear::YES_FROM,
            no_below: crate::ear::NO_BELOW,
        },
    );
    for yes in 30..=95 {
        for no in 5..=yes {
            let lines = Lines {
                yes_from: f64::from(yes) / 100.0,
                no_below: f64::from(no) / 100.0,
            };
            let count = |truth: bool, word: &str| {
                said.iter()
                    .filter(|&&(m, t)| t == truth && word_for_match(m, lines) == word)
                    .count() as f64
            };
            let (true_yes, true_no) = (count(true, "yes") / truths, count(true, "no") / truths);
            let (wrong_no, wrong_yes) = (count(false, "no") / wrongs, count(false, "yes") / wrongs);
            if true_no > 0.1 || wrong_yes > 0.1 {
                continue;
            }
            let worse = true_yes.min(wrong_no);
            if worse > best.0 + 1e-9 {
                best = (worse, lines);
            }
        }
    }
    best.1
}

/// The measures, and the sounds the Ear got wrong, as Markdown.
fn table_of(measures: &[Measure], results: &[SoundResult]) -> String {
    let mut out = measures_table(measures);
    let missed: Vec<&SoundResult> = results.iter().filter(|r| !r.beaten_by.is_empty()).collect();
    if !missed.is_empty() {
        out.push_str("\n| Sound | True text (match) | Wrong text at least as high (match) |\n|---|---|---|\n");
        for r in missed {
            let truths: Vec<String> = r
                .true_matches
                .iter()
                .map(|(t, m)| format!("{t} ({m:.2})"))
                .collect();
            let wrong: Vec<String> = r
                .beaten_by
                .iter()
                .map(|t| {
                    format!(
                        "{t} ({:.2})",
                        r.wrong_matches.get(t).copied().unwrap_or(0.0)
                    )
                })
                .collect();
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                r.file,
                truths.join(", "),
                wrong.join(", ")
            ));
        }
    }
    out
}

/// Write the report as `ear-check.json` and `ear-check.md` into `out`.
pub fn write(report: &CheckReport, out: &Path) -> Result<(), ToolError> {
    let io = |e: std::io::Error| {
        ToolError::failed(
            Code::Io,
            format!("cannot write into {}: {e}", out.display()),
        )
    };
    std::fs::create_dir_all(out).map_err(io)?;
    let json = serde_json::to_string_pretty(report).expect("serializes");
    std::fs::write(out.join("ear-check.json"), json + "\n").map_err(io)?;
    let md = format!(
        "# Duck Ear check\n\n{}\n\nModel: `{}` on `{}`; yes from {:.3}, no below {:.3} \
         (the best on this set: yes from {:.2}, no below {:.2}).\n\n{}",
        report.summary,
        report.score_model,
        report.device,
        report.lines.yes_from,
        report.lines.no_below,
        report.suggested_lines.yes_from,
        report.suggested_lines.no_below,
        report.table
    );
    let md = match &report.describe {
        None => md,
        Some(d) => format!(
            "{md}\n## The describe model\n\nModel: `{}` on `{}`{}.\n\n{}",
            d.model,
            d.device,
            d.device_name
                .as_deref()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default(),
            d.table
        ),
    };
    std::fs::write(out.join("ear-check.md"), md).map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suggested_lines_split_true_from_wrong() {
        // True texts score high, wrong ones low, with a gap between.
        let said = [
            (0.9, true),
            (0.8, true),
            (0.7, true),
            (0.2, false),
            (0.1, false),
            (0.15, false),
        ];
        let lines = best_lines(&said);
        assert!(lines.yes_from > 0.2 && lines.yes_from <= 0.7, "{lines:?}");
        assert!(lines.no_below > 0.2 && lines.no_below <= lines.yes_from);
        for (m, truth) in said {
            let word = word_for_match(m, lines);
            assert_eq!(word, if truth { "yes" } else { "no" }, "{m}");
        }
    }

    #[test]
    fn the_suggested_lines_keep_each_mistake_at_most_one_in_ten() {
        // One wrong text scores as high as the true ones: saying yes to
        // it would be a mistake in 1 of 4 wrong texts, so it must not be
        // yes, even if that costs some true yeses.
        let said = [
            (0.9, true),
            (0.8, true),
            (0.85, false),
            (0.1, false),
            (0.2, false),
            (0.3, false),
        ];
        let lines = best_lines(&said);
        assert_ne!(word_for_match(0.85, lines), "yes", "{lines:?}");
    }
}
