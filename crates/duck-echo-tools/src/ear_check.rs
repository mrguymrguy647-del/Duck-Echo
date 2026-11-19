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

/// Run the test set at `testset` with the Ear `keeper` keeps.
pub fn check(keeper: &EarKeeper, testset: &Path) -> Result<CheckReport, ToolError> {
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
    let mut matches: Vec<BTreeMap<String, f64>> = Vec::new();
    let mut results = Vec::new();
    let mut seconds = 0.0;
    for sound in &set.sound {
        let buffer = render(folder, &sound.file)?;
        let heard = keeper.sound(&buffer)?;
        let answer = listening
            .ask(&heard, "score", json!({ "texts": texts }))
            .map_err(|missed| match missed {
                Missed::Unavailable(u) => failed(crate::ear::unavailable_words(&u)),
                Missed::Failed(f) => failed(format!("the Ear failed on {}: {f}", sound.file)),
            })?;
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
        results,
        table,
        summary,
    })
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
