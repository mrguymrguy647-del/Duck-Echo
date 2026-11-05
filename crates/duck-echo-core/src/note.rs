//! Note names and the [`Frequency`] type.
//!
//! A frequency in a recipe is either a number in Hz or a note name in
//! scientific pitch notation: a letter `A`..`G` (either case), an optional
//! `#` (sharp) or `b` (flat), and an octave from `-1` to `9`. `A4` is
//! 440 Hz and `C4` is middle C. The text the author wrote is kept, so a
//! recipe written with notes serializes back with notes.
//!
//! Conversion uses only IEEE-exact operations (a table of the twelve
//! semitone ratios and power-of-two octave scaling), so every platform
//! computes the same frequency from the same note.

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;

/// JSON Schema `format` value for note-name strings. The structural checker
/// in [`crate::structure`] validates strings with this format using
/// [`parse_note`].
pub const NOTE_FORMAT: &str = "note-name";

/// Regular expression for note names, published in the schema for external
/// validators. [`parse_note`] is the authoritative check.
pub const NOTE_PATTERN: &str = "^[A-Ga-g][#b]?(-1|[0-9])$";

/// `2^(k/12)` for `k` in `0..12`, correctly rounded.
const SEMITONE_RATIOS: [f64; 12] = [
    1.0,
    1.059_463_094_359_295_3,
    1.122_462_048_309_373,
    1.189_207_115_002_721,
    1.259_921_049_894_873_2,
    1.334_839_854_170_034_4,
    core::f64::consts::SQRT_2,
    1.498_307_076_876_681_5,
    1.587_401_051_968_199_6,
    1.681_792_830_507_429,
    1.781_797_436_280_678_5,
    1.887_748_625_363_387,
];

/// Why a note name was rejected, with a likely intended name when there is
/// an obvious one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteError {
    /// Human-readable explanation, including an example of a valid name.
    pub message: String,
    /// A likely intended note name.
    pub suggestion: Option<String>,
}

impl fmt::Display for NoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(s) = &self.suggestion {
            write!(f, " Did you mean \"{s}\"?")?;
        }
        Ok(())
    }
}

fn note_error(text: &str, why: &str, suggestion: Option<String>) -> NoteError {
    NoteError {
        message: format!(
            "invalid note name \"{text}\": {why}; write a letter A-G, an optional # or b, then an octave -1..9, for example \"C5\", \"F#3\" or \"Bb2\""
        ),
        suggestion,
    }
}

/// If `text` is a number written as a string, optionally with an `Hz`
/// unit (`"440"`, `"440hz"`, `"261.6 Hz"`), the number as written.
pub fn quoted_number(text: &str) -> Option<&str> {
    let t = text.trim();
    let t = if t.to_ascii_lowercase().ends_with("hz") {
        t[..t.len() - 2].trim_end()
    } else {
        t
    };
    (!t.is_empty() && t.parse::<f64>().is_ok_and(f64::is_finite)).then_some(t)
}

/// MIDI note number (C-1 = 0, A4 = 69) of a note name.
pub fn note_number(text: &str) -> Result<i32, NoteError> {
    if let Some(number) = quoted_number(text) {
        return Err(NoteError {
            message: format!(
                "\"{text}\" is a number in quotes; write {number} without quotes (frequencies are in Hz), or use a note name such as \"C5\""
            ),
            suggestion: Some(number.to_string()),
        });
    }
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let Some(letter) = chars.next() else {
        return Err(note_error(text, "it is empty", None));
    };
    let base = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        'H' => {
            // German spelling: H is B.
            let fixed = format!("B{}", &trimmed[1..]);
            return Err(note_error(
                text,
                "`H` is not a note letter here",
                Some(fixed),
            ));
        }
        _ => return Err(note_error(text, "it must start with a letter A-G", None)),
    };
    let rest: &str = chars.as_str();
    let (accidental, octave_text) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        Some('♯') => {
            let fixed = format!("{letter}#{}", &rest['♯'.len_utf8()..]);
            return Err(note_error(text, "use `#` for sharp", Some(fixed)));
        }
        Some('♭') => {
            let fixed = format!("{letter}b{}", &rest['♭'.len_utf8()..]);
            return Err(note_error(text, "use `b` for flat", Some(fixed)));
        }
        _ => (0, rest),
    };
    if octave_text.is_empty() {
        let fixed = format!("{letter}{}4", &rest[..rest.len() - octave_text.len()]);
        return Err(note_error(text, "it has no octave number", Some(fixed)));
    }
    let octave: i32 = octave_text.parse().map_err(|_| {
        note_error(
            text,
            &format!("`{octave_text}` is not an octave number"),
            None,
        )
    })?;
    if !(-1..=9).contains(&octave) {
        return Err(note_error(
            text,
            &format!("octave {octave} is outside -1..9"),
            None,
        ));
    }
    if trimmed != text {
        return Err(note_error(
            text,
            "it has surrounding spaces",
            Some(trimmed.to_string()),
        ));
    }
    Ok((octave + 1) * 12 + base + accidental)
}

/// Frequency in Hz of a MIDI note number, with A4 (69) = 440 Hz.
pub fn note_number_to_hz(number: i32) -> f32 {
    let diff = number - 69;
    let mut hz = 440.0 * SEMITONE_RATIOS[diff.rem_euclid(12) as usize];
    let octaves = diff.div_euclid(12);
    // Power-of-two scaling is exact.
    for _ in 0..octaves.max(0) {
        hz *= 2.0;
    }
    for _ in 0..(-octaves).max(0) {
        hz *= 0.5;
    }
    hz as f32
}

/// Frequency in Hz of a note name such as `"C5"`.
pub fn parse_note(text: &str) -> Result<f32, NoteError> {
    note_number(text).map(note_number_to_hz)
}

/// A frequency written either as Hz or as a note name.
#[derive(Debug, Clone, PartialEq)]
pub struct Frequency {
    hz: f32,
    note: Option<String>,
}

impl Frequency {
    /// A frequency in Hz.
    pub fn hz_value(hz: f32) -> Self {
        Self { hz, note: None }
    }

    /// A frequency from a note name.
    pub fn from_note(text: &str) -> Result<Self, NoteError> {
        Ok(Self {
            hz: parse_note(text)?,
            note: Some(text.to_string()),
        })
    }

    /// The frequency in Hz.
    pub fn hz(&self) -> f32 {
        self.hz
    }

    /// The note name as written, if the frequency was given as one.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

impl From<f32> for Frequency {
    fn from(hz: f32) -> Self {
        Self::hz_value(hz)
    }
}

impl Serialize for Frequency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.note {
            Some(n) => serializer.serialize_str(n),
            None => serializer.serialize_f32(self.hz),
        }
    }
}

impl<'de> Deserialize<'de> for Frequency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Frequency;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a frequency in Hz or a note name such as \"C5\"")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Frequency, E> {
                Ok(Frequency::hz_value(v as f32))
            }
            fn visit_f32<E: de::Error>(self, v: f32) -> Result<Frequency, E> {
                Ok(Frequency::hz_value(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Frequency, E> {
                Ok(Frequency::hz_value(v as f32))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Frequency, E> {
                Ok(Frequency::hz_value(v as f32))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Frequency, E> {
                Frequency::from_note(v).map_err(|e| E::custom(e.to_string()))
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl JsonSchema for Frequency {
    fn schema_name() -> Cow<'static, str> {
        "Frequency".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A frequency: a number in Hz, or a note name in scientific pitch notation (letter A-G, optional # or b, octave -1..9; A4 = 440 Hz, C4 = middle C).",
            "anyOf": [
                { "type": "number", "format": "float" },
                {
                    "type": "string",
                    "format": NOTE_FORMAT,
                    "pattern": NOTE_PATTERN,
                    "examples": ["C5", "F#3", "Bb2"]
                }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_pitches() {
        assert_eq!(parse_note("A4").unwrap(), 440.0);
        assert!((parse_note("C5").unwrap() - 523.25).abs() < 0.005);
        assert!((parse_note("C4").unwrap() - 261.63).abs() < 0.005);
        assert_eq!(parse_note("A5").unwrap(), 880.0);
        assert_eq!(parse_note("A-1").unwrap(), 13.75);
        assert!((parse_note("C-1").unwrap() - 8.175_8).abs() < 0.001);
        assert!((parse_note("B9").unwrap() - 15_804.27).abs() < 0.01);
        // Lower-case letters are accepted.
        assert_eq!(parse_note("a4").unwrap(), 440.0);
    }

    #[test]
    fn sharps_flats_and_enharmonics() {
        assert_eq!(parse_note("A#3").unwrap(), parse_note("Bb3").unwrap());
        assert_eq!(parse_note("Cb4").unwrap(), parse_note("B3").unwrap());
        assert_eq!(parse_note("E#4").unwrap(), parse_note("F4").unwrap());
        assert!((parse_note("F#3").unwrap() - 185.0).abs() < 0.005);
        assert_eq!(note_number("C-1").unwrap(), 0);
        assert_eq!(note_number("G9").unwrap(), 127);
    }

    #[test]
    fn bad_names_explain_and_suggest() {
        let e = parse_note("H4").unwrap_err();
        assert_eq!(e.suggestion.as_deref(), Some("B4"));
        assert!(e.message.contains("\"C5\""));
        assert_eq!(
            parse_note("C#").unwrap_err().suggestion.as_deref(),
            Some("C#4")
        );
        assert_eq!(
            parse_note("F♯3").unwrap_err().suggestion.as_deref(),
            Some("F#3")
        );
        assert!(parse_note("C10").is_err());
        assert!(parse_note("C-2").is_err());
        assert!(parse_note("").is_err());
        assert!(parse_note("X4").unwrap_err().suggestion.is_none());
        assert_eq!(
            parse_note(" C4").unwrap_err().suggestion.as_deref(),
            Some("C4")
        );
    }

    #[test]
    fn quoted_numbers_suggest_the_bare_number() {
        for (text, want) in [
            ("440", "440"),
            ("440hz", "440"),
            ("261.6 Hz", "261.6"),
            (" 880HZ ", "880"),
        ] {
            let e = parse_note(text).unwrap_err();
            assert_eq!(e.suggestion.as_deref(), Some(want), "{text}");
            assert!(e.message.contains("without quotes"), "{}", e.message);
        }
        assert_eq!(quoted_number("hz"), None);
        assert_eq!(quoted_number("C4"), None);
        assert_eq!(quoted_number("inf"), None);
    }

    #[test]
    fn frequency_round_trips_its_spelling() {
        let f: Frequency = serde_json::from_str("\"Bb2\"").unwrap();
        assert_eq!(f.note(), Some("Bb2"));
        assert_eq!(serde_json::to_string(&f).unwrap(), "\"Bb2\"");
        let g: Frequency = serde_json::from_str("523.25").unwrap();
        assert_eq!(g.note(), None);
        assert_eq!(serde_json::to_string(&g).unwrap(), "523.25");
        let i: Frequency = serde_json::from_str("440").unwrap();
        assert_eq!(i.hz(), 440.0);
    }
}
