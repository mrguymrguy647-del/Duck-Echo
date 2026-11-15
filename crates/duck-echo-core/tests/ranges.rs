//! The checks and the ranges table agree: every number in a recipe and a
//! song is refused just outside its range, with the table's limits in the
//! error, and accepted at both ends of it.

use duck_echo_core::ranges::{self, Range, ALL};
use duck_echo_core::{load_song_str, load_str, Format, NoInstruments, Report};
use serde_json::{json, Value};

/// A recipe that uses every field that has a range.
fn recipe() -> Value {
    json!({
        "version": 0, "sample_rate": 44100, "channels": 2, "variation": 0.5, "duration": 1.0,
        "master": {"gain": 1, "normalize_peak_db": -1, "normalize_lufs": -16},
        "layers": [{
            "source": "square", "volume": 0.8, "offset": 0, "duty": 0.5, "pan": 0,
            "lfo": {"rate": 5, "delay": 0, "depth": 0.5, "target": "cutoff"},
            "envelope": {"attack": 0.01, "hold": 0, "decay": 0.1, "sustain": 0.5, "sustain_time": 0.1, "release": 0.1},
            "pitch": {
                "start": 440, "end": 880, "sweep_time": 0.1,
                "vibrato": {"rate": 5, "depth": 1, "delay": 0},
                "arpeggio": {"steps": [0, 4], "step_time": 0.05}
            },
            "filter": {"cutoff": 1000, "cutoff_end": 2000, "sweep_time": 0.1, "resonance": 1},
            "effects": [
                {"type": "echo", "delay": 0.1, "feedback": 0.3, "mix": 0.3},
                {"type": "reverb", "size": 0.5, "decay": 1, "damping": 0.5, "mix": 0.3},
                {"type": "distortion", "drive": 2, "mix": 0.5},
                {"type": "bitcrush", "bits": 8, "rate": 8000, "mix": 0.5}
            ]
        }]
    })
}

/// A song that uses every song field that has a range.
fn song() -> Value {
    json!({
        "version": 0, "tempo": 120, "beats_per_bar": 4, "sample_rate": 44100, "channels": 2,
        "instruments": {"lead": {"version": 0, "layers": [{"source": "sine"}]}},
        "patterns": {"p": {"notes": [{"beat": 0, "note": "C4", "length": 1, "velocity": 0.8}]}},
        "tracks": [{
            "name": "lead", "instrument": "lead", "volume": 0.8, "pan": 0,
            "notes": [{"beat": 0, "note": 440, "length": 1, "velocity": 0.8}],
            "play": [{"pattern": "p", "bars": [2], "transpose": 0}]
        }]
    })
}

/// Where a range's field is in the documents above: (is a song, path).
fn place(r: &Range) -> (bool, String) {
    let layer = "layers[0]";
    let effect = |i: usize| format!("{layer}.effects[{i}].{}", r.field);
    let path = match r.owner {
        "Recipe" => r.field.to_string(),
        "Master" => format!("master.{}", r.field),
        "Layer" => format!("{layer}.{}", r.field),
        "Lfo" => format!("{layer}.lfo.{}", r.field),
        "Envelope" => format!("{layer}.envelope.{}", r.field),
        "Frequency" => format!("{layer}.pitch.start"),
        "Pitch" => format!("{layer}.pitch.{}", r.field),
        "Vibrato" => format!("{layer}.pitch.vibrato.{}", r.field),
        "Arpeggio" if r.field == "steps[]" => format!("{layer}.pitch.arpeggio.steps[0]"),
        "Arpeggio" => format!("{layer}.pitch.arpeggio.{}", r.field),
        "Filter" => format!("{layer}.filter.{}", r.field),
        "Echo" => effect(0),
        "Reverb" => effect(1),
        "Distortion" => effect(2),
        "Bitcrush" => effect(3),
        "Song" => return (true, r.field.to_string()),
        "Track" => return (true, format!("tracks[0].{}", r.field)),
        "Placement" => return (true, format!("tracks[0].play[0].{}", r.field)),
        "Note" => return (true, format!("tracks[0].notes[0].{}", r.field)),
        "PatternNote" => return (true, format!("patterns.p.notes[0].{}", r.field)),
        "NotePitch" => return (true, "tracks[0].notes[0].note".to_string()),
        other => panic!("no place for {other}.{}: add it to this test", r.field),
    };
    (false, path)
}

/// Set `path` (such as `layers[0].lfo.rate`) in `doc`.
fn set(doc: &mut Value, path: &str, value: Value) {
    let mut at = doc;
    for part in path.split('.') {
        let (key, index) = match part.split_once('[') {
            Some((k, i)) => (k, Some(i.trim_end_matches(']').parse::<usize>().unwrap())),
            None => (part, None),
        };
        at = &mut at[key];
        if let Some(i) = index {
            at = &mut at[i];
        }
    }
    *at = value;
}

/// Load the document with `path` set to `value`, and return the errors at
/// `path` as JSON.
fn errors_at(is_song: bool, path: &str, value: Value) -> Vec<Value> {
    let mut doc = if is_song { song() } else { recipe() };
    set(&mut doc, path, value);
    let text = doc.to_string();
    let report: Report = if is_song {
        match load_song_str(&text, Format::Json, &NoInstruments) {
            Ok(l) => l.report,
            Err(r) => r,
        }
    } else {
        match load_str(&text, Format::Json) {
            Ok(l) => l.report,
            Err(r) => r,
        }
    };
    report
        .errors
        .iter()
        .filter(|i| i.path == path)
        .map(|i| serde_json::to_value(i).unwrap())
        .collect()
}

/// A whole number for a field that only takes whole numbers.
fn number(r: &Range, v: f64) -> Value {
    let whole = [
        "sample_rate",
        "channels",
        "bits",
        "beats_per_bar",
        "transpose",
    ];
    if whole.contains(&r.field) {
        json!(v.round() as i64)
    } else {
        json!(v)
    }
}

#[test]
fn the_base_documents_are_valid() {
    assert!(load_str(&recipe().to_string(), Format::Json).is_ok());
    let loaded = load_song_str(&song().to_string(), Format::Json, &NoInstruments);
    assert!(loaded.is_ok(), "{:?}", loaded.err());
}

#[test]
fn every_range_is_enforced_with_the_tables_own_limits() {
    for r in ALL {
        let (is_song, path) = place(r);
        let what = format!("{}.{} at {path}", r.owner, r.field);
        // A step outside: a whole number for whole fields, otherwise 1%.
        let step = ((r.max - r.min) * 0.01).max(0.0005);
        let whole = number(r, r.min) == json!(r.min as i64) && number(r, 0.4) == json!(0);
        let (below, above) = if whole {
            (r.min - 1.0, r.max + 1.0)
        } else {
            (r.min - step, r.max + step)
        };

        // Above the largest value: refused, naming the table's limits.
        let errors = errors_at(is_song, &path, number(r, above));
        assert_eq!(errors.len(), 1, "{what} above: {errors:?}");
        assert_eq!(errors[0]["code"], "out_of_range", "{what}");
        if !r.above_min {
            assert_eq!(errors[0]["allowed"]["min"], json!(r.min), "{what}");
            assert_eq!(errors[0]["allowed"]["max"], json!(r.max), "{what}");
        }

        // Below the smallest: refused. (A field that cannot be negative is
        // refused by its type instead, which is still a refusal.)
        let errors = errors_at(is_song, &path, number(r, below));
        assert_eq!(errors.len(), 1, "{what} below: {errors:?}");

        // The largest value is allowed.
        let errors = errors_at(is_song, &path, number(r, r.max));
        assert_eq!(errors, Vec::<Value>::new(), "{what} at its largest");

        // The smallest is allowed, unless the range leaves it out.
        let errors = errors_at(is_song, &path, number(r, r.min));
        assert_eq!(
            errors.is_empty(),
            !r.above_min,
            "{what} at its smallest: {errors:?}"
        );
    }
}

#[test]
fn an_lfos_depth_depends_on_its_target() {
    for (target, max) in [("volume", 1.0), ("cutoff", 4.0)] {
        let mut doc = recipe();
        set(&mut doc, "layers[0].lfo.target", json!(target));
        set(&mut doc, "layers[0].lfo.depth", json!(max));
        assert!(
            load_str(&doc.to_string(), Format::Json).is_ok(),
            "{target} at {max}"
        );
        set(&mut doc, "layers[0].lfo.depth", json!(max + 0.01));
        let report = load_str(&doc.to_string(), Format::Json).unwrap_err();
        let issue = serde_json::to_value(&report.errors[0]).unwrap();
        assert_eq!(issue["path"], "layers[0].lfo.depth");
        assert_eq!(issue["allowed"]["max"], json!(max));
    }
    assert_eq!(ranges::lfo_depth(duck_echo_core::LfoTarget::Duty).max, 0.5);
}

#[test]
fn the_old_named_limits_are_the_tables() {
    use duck_echo_core::validate::{MAX_DURATION, MAX_TAIL, MIN_NORMALIZE_LUFS};
    assert_eq!(ranges::DURATION.max, MAX_DURATION as f64);
    assert_eq!(ranges::REVERB_DECAY.max, MAX_TAIL as f64);
    assert_eq!(ranges::MASTER_NORMALIZE_LUFS.min, MIN_NORMALIZE_LUFS as f64);
}
