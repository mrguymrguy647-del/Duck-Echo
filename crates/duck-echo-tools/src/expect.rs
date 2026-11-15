//! A recipe's `expect`: what the sound should measure as, checked line by
//! line against its analysis.

use duck_echo_analyze::{Analysis, Level};
use duck_echo_core::{Amount, Expect};
use serde::Serialize;

/// One line of `expect`, checked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExpectLine {
    /// The measure: `roughness`, `loudness_lufs`...
    pub what: String,
    /// What the recipe wants: `high`, or `-20..-16`.
    pub wanted: String,
    /// What was measured: `medium`, or `-17.3`. `null` when it could not
    /// be measured (a silent sound).
    pub got: Option<String>,
    /// True when it is what was wanted.
    pub pass: bool,
}

/// Every line of `expect`, checked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Expected {
    /// True when every line passes.
    pub pass: bool,
    /// How many lines pass.
    pub met: usize,
    /// The lines, in the order of the fields.
    pub lines: Vec<ExpectLine>,
}

fn level_word(level: Level) -> &'static str {
    level.word()
}

/// One feel line: its name, what was wanted, the level measured and the
/// measure itself.
type AmountLine = (&'static str, Option<Amount>, Option<Level>, Option<f32>);
/// One range line: its name, the `[low, high]` wanted and the value.
type RangeLine<'e> = (&'static str, &'e Option<Vec<f32>>, Option<f64>);

/// Check `expect` against `a`.
pub fn check(expect: &Expect, a: &Analysis) -> Expected {
    let mut lines = Vec::new();
    let feel = a.feel;
    let amounts: [AmountLine; 4] = [
        (
            "roughness",
            expect.roughness,
            feel.map(|f| f.roughness),
            a.roughness,
        ),
        (
            "sharpness",
            expect.sharpness,
            feel.map(|f| f.sharpness),
            a.sharpness,
        ),
        ("boom", expect.boom, feel.map(|f| f.boom), a.boom),
        (
            "movement",
            expect.movement,
            feel.map(|f| f.movement),
            a.movement,
        ),
    ];
    for (what, wanted, got, value) in amounts {
        let Some(wanted) = wanted else { continue };
        lines.push(ExpectLine {
            what: what.into(),
            wanted: wanted.word().into(),
            got: got
                .zip(value)
                .map(|(l, v)| format!("{} ({v})", level_word(l))),
            pass: got.is_some_and(|l| l.word() == wanted.word()),
        });
    }
    let ranges: [RangeLine; 3] = [
        (
            "loudness_lufs",
            &expect.loudness_lufs,
            a.loudness_lufs.map(f64::from),
        ),
        ("peak_dbfs", &expect.peak_dbfs, a.peak_dbfs.map(f64::from)),
        (
            "duration_seconds",
            &expect.duration_seconds,
            Some(a.duration_seconds),
        ),
    ];
    for (what, wanted, got) in ranges {
        let Some(pair) = wanted.as_ref().filter(|p| p.len() == 2) else {
            continue;
        };
        let (lo, hi) = (f64::from(pair[0]), f64::from(pair[1]));
        lines.push(ExpectLine {
            what: what.into(),
            wanted: format!("{lo}..{hi}"),
            got: got.map(|g| format!("{:.2}", g)),
            pass: got.is_some_and(|g| g >= lo - 1e-6 && g <= hi + 1e-6),
        });
    }
    let met = lines.iter().filter(|l| l.pass).count();
    Expected {
        pass: met == lines.len(),
        met,
        lines,
    }
}

/// What the summary says about `expect`: "expect: all 3 met", or
/// "expect: 2 of 3 met (roughness is medium, wanted high)".
pub fn summary_words(e: &Expected) -> String {
    if e.pass {
        return format!("expect: all {} met", e.lines.len());
    }
    let missed: Vec<String> = e
        .lines
        .iter()
        .filter(|l| !l.pass)
        .map(|l| {
            let got = l.got.as_deref().unwrap_or("nothing");
            let got = got.split(' ').next().unwrap_or(got);
            format!("{} is {got}, wanted {}", l.what, l.wanted)
        })
        .collect();
    format!(
        "EXPECT: {} of {} met ({})",
        e.met,
        e.lines.len(),
        missed.join("; ")
    )
}
