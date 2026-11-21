//! A recipe's (or a song's) `expect`: what the sound should measure as,
//! checked line by line against its analysis.
//!
//! `sounds_like` is the one line the numbers cannot check: only the Duck
//! Ear can (see [`crate::ear`]). Until it does, the line is listed as not
//! checked, with `pass: null`, and never counts as a failure.

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
    /// be measured (a silent sound), or for `sounds_like` before the Ear
    /// listened.
    pub got: Option<String>,
    /// True when it is what was wanted, false when not. `null` when it was
    /// not checked: `sounds_like` without the Ear, or when the Ear is
    /// unsure. A line that was not checked never fails.
    pub pass: Option<bool>,
    /// Why a line was not checked, or where its answer came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The note on a `sounds_like` line the Ear has not listened to.
pub const NEEDS_THE_EAR: &str = "needs the Ear: analyze with ear: true";

/// Every line of `expect`, checked.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Expected {
    /// True when every line that was checked passes.
    pub pass: bool,
    /// How many lines pass.
    pub met: usize,
    /// The lines, in the order of the fields.
    pub lines: Vec<ExpectLine>,
}

impl Expected {
    /// Work `pass` and `met` out again from the lines (after the Ear
    /// changed one).
    fn count(&mut self) {
        self.met = self.lines.iter().filter(|l| l.pass == Some(true)).count();
        self.pass = self.lines.iter().all(|l| l.pass != Some(false));
    }

    /// Put the Ear's answer on the `sounds_like` line: its verdict (`yes`,
    /// `no` or `unsure`) and its match. Does nothing without that line.
    pub fn set_sounds_like(&mut self, verdict: &str, matched: f64, note: &str) {
        let Some(line) = self.lines.iter_mut().find(|l| l.what == "sounds_like") else {
            return;
        };
        line.got = Some(format!("{verdict} ({matched:.2})"));
        line.pass = match verdict {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        };
        line.note = Some(note.to_string());
        self.count();
    }

    /// Put why the Ear could not answer on the `sounds_like` line.
    pub fn set_sounds_like_unheard(&mut self, why: &str) {
        if let Some(line) = self.lines.iter_mut().find(|l| l.what == "sounds_like") {
            line.note = Some(why.to_string());
        }
    }
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
            pass: Some(got.is_some_and(|l| l.word() == wanted.word())),
            note: None,
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
            pass: Some(got.is_some_and(|g| g >= lo - 1e-6 && g <= hi + 1e-6)),
            note: None,
        });
    }
    if let Some(text) = &expect.sounds_like {
        lines.push(ExpectLine {
            what: "sounds_like".into(),
            wanted: text.trim().to_string(),
            got: None,
            pass: None,
            note: Some(NEEDS_THE_EAR.into()),
        });
    }
    let mut expected = Expected {
        pass: true,
        met: 0,
        lines,
    };
    expected.count();
    expected
}

/// What the summary says about `expect`: "expect: all 3 met", or
/// "EXPECT: 2 of 3 met (roughness is medium, wanted high)". Lines that were
/// not checked are named after it: "expect: all 2 met (sounds_like not
/// checked)", or "(sounds_like: the Ear is unsure)".
pub fn summary_words(e: &Expected) -> String {
    let checked = e.lines.iter().filter(|l| l.pass.is_some()).count();
    let unchecked: Vec<String> = e
        .lines
        .iter()
        .filter(|l| l.pass.is_none())
        .map(|l| match l.got {
            None => format!("{} not checked", l.what),
            Some(_) => format!("{}: the Ear is unsure", l.what),
        })
        .collect();
    let after = if unchecked.is_empty() {
        String::new()
    } else {
        format!(" ({})", unchecked.join("; "))
    };
    if checked == 0 {
        return format!("expect: {}", unchecked.join("; "));
    }
    if e.pass {
        return format!("expect: all {checked} met{after}");
    }
    let missed: Vec<String> = e
        .lines
        .iter()
        .filter(|l| l.pass == Some(false))
        .map(|l| {
            let got = l.got.as_deref().unwrap_or("nothing");
            let got = got.split(' ').next().unwrap_or(got);
            if l.what == "sounds_like" {
                format!("the Ear says {got} to sounds like \"{}\"", l.wanted)
            } else {
                format!("{} is {got}, wanted {}", l.what, l.wanted)
            }
        })
        .collect();
    format!(
        "EXPECT: {} of {checked} met ({}){after}",
        e.met,
        missed.join("; ")
    )
}
