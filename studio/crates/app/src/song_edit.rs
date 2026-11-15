//! Editing a song in the timeline, as named commands.
//!
//! Every change the timeline makes is one of the song commands an AI
//! assistant uses (`song_add_note`, `song_move_note`, `song_resize_note`,
//! `song_remove_note`, `song_place_pattern`, `song_move_pattern`,
//! `song_remove_pattern`, `song_set`), so each is one step of the shared
//! Undo history, and a song made in the timeline is the same file as the
//! same song written by hand.
//!
//! The functions here only work out a command's name and arguments from
//! what the person did; the timeline runs them. Beats here are as the
//! file writes them (from 0); only the screen counts them from 1.

use duck_echo_core::{NotePitch, NoteValue, Song, note};
use serde_json::{Value, json};

use crate::song_view::{Clip, Lane};

/// A command: its name and its arguments.
pub type Edit = (&'static str, Value);

/// `beat` (from the start of the song) moved to the nearest step of the
/// grid: `step` is 1 for whole beats, 0.5 for half beats...
pub fn snap(beat: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return beat;
    }
    ((beat / step).round() * step).max(0.0)
}

/// `beat` (from the start of the song) as the file writes it: the bar,
/// counted from 1, and the beat in that bar, counted from 0.
pub fn bar_and_beat(song: &Song, beat: f64) -> (u32, f64) {
    let per_bar = f64::from(song.beats_per_bar);
    let bar = (beat / per_bar).floor().max(0.0);
    let in_bar = beat - bar * per_bar;
    // Tidy float noise: 1.9999999 is 2.
    let in_bar = (in_bar * 1_000_000.0).round() / 1_000_000.0;
    (bar as u32 + 1, in_bar)
}

/// A new note on `lane` (track `track`'s name in `file`) at `beat`, on
/// row `row`: one beat long.
pub fn add_note(file: &str, song: &Song, lane: &Lane, beat: f64, row: i32) -> Option<Edit> {
    let what = lane.note_at(row)?;
    let (bar, in_bar) = bar_and_beat(song, beat);
    let mut args = json!({
        "file": file,
        "track": lane.name,
        "bar": bar,
        "beat": in_bar,
        "note": what,
    });
    if !lane.kit {
        args["length"] = json!(1);
    }
    Some(("song_add_note", args))
}

/// A note as written, moved up or down by `semitones`: every pitch of a
/// chord moves together. A note written in Hz stays in Hz.
pub fn transpose(value: &NoteValue, semitones: i32) -> Option<Value> {
    let one = |p: &NotePitch| -> Option<Value> {
        match p {
            NotePitch::Name(name) => {
                let n = note::note_number(name).ok()?;
                note::note_name(n + semitones).map(Value::from)
            }
            NotePitch::Hz(hz) => {
                let moved = hz * 2f64.powf(f64::from(semitones) / 12.0);
                Some(json!((moved * 100.0).round() / 100.0))
            }
        }
    };
    match value {
        NoteValue::One(p) => one(p),
        NoteValue::Chord(list) => list
            .iter()
            .map(one)
            .collect::<Option<Vec<_>>>()
            .map(Value::from),
    }
}

/// Note `index` of track `track` moved to start at `beat`, and up by
/// `semitones` (or, on a kit, to `piece`).
pub fn move_note(
    file: &str,
    song: &Song,
    track: usize,
    index: usize,
    beat: f64,
    semitones: i32,
    piece: Option<String>,
) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    let written = t.notes.get(index)?;
    let (bar, in_bar) = bar_and_beat(song, beat);
    let mut args = json!({
        "file": file,
        "track": t.name,
        "index": index,
        "bar": bar,
        "beat": in_bar,
    });
    if let Some(piece) = piece {
        args["note"] = json!(piece);
    } else if semitones != 0 {
        args["note"] = transpose(&written.note, semitones)?;
    }
    Some(("song_move_note", args))
}

/// Note `index` of track `track` made `length` beats long.
pub fn resize_note(
    file: &str,
    song: &Song,
    track: usize,
    index: usize,
    length: f64,
) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    Some((
        "song_resize_note",
        json!({"file": file, "track": t.name, "index": index, "length": length}),
    ))
}

/// Note `index` of track `track` taken out.
pub fn remove_note(file: &str, song: &Song, track: usize, index: usize) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    Some((
        "song_remove_note",
        json!({"file": file, "track": t.name, "index": index}),
    ))
}

/// Pattern `pattern` placed on track `track` at `bar`.
pub fn place_pattern(
    file: &str,
    song: &Song,
    track: usize,
    pattern: &str,
    bar: u32,
) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    Some((
        "song_place_pattern",
        json!({"file": file, "track": t.name, "pattern": pattern, "bars": [bar]}),
    ))
}

/// A clip moved to start at bar `to`. A placement can start at several
/// bars; only this clip's bar changes. `None` when nothing would change,
/// or another clip of the same placement is already there.
pub fn move_clip(file: &str, song: &Song, track: usize, clip: &Clip, to: u32) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    let placement = t.play.get(clip.placement)?;
    if to == clip.bar || to == 0 || placement.bars.contains(&to) {
        return None;
    }
    let bars: Vec<u32> = placement
        .bars
        .iter()
        .map(|b| if *b == clip.bar { to } else { *b })
        .collect();
    Some((
        "song_move_pattern",
        json!({"file": file, "track": t.name, "index": clip.placement, "bars": bars}),
    ))
}

/// A clip taken off its track: its bar leaves the placement, or the
/// whole placement goes when it was its only bar.
pub fn remove_clip(file: &str, song: &Song, track: usize, clip: &Clip) -> Option<Edit> {
    let t = song.tracks.get(track)?;
    let placement = t.play.get(clip.placement)?;
    if placement.bars.len() <= 1 {
        return Some((
            "song_remove_pattern",
            json!({"file": file, "track": t.name, "index": clip.placement}),
        ));
    }
    let bars: Vec<u32> = placement
        .bars
        .iter()
        .copied()
        .filter(|b| *b != clip.bar)
        .collect();
    Some((
        "song_move_pattern",
        json!({"file": file, "track": t.name, "index": clip.placement, "bars": bars}),
    ))
}

/// Track `track`'s volume set to `volume`.
pub fn set_volume(file: &str, track: usize, volume: f32) -> Edit {
    let v = (f64::from(volume) * 100.0).round() / 100.0;
    (
        "song_set",
        json!({"file": file, "path": format!("tracks[{track}].volume"), "value": v}),
    )
}

/// Track `track` played by `instrument`.
pub fn set_instrument(file: &str, track: usize, instrument: &str) -> Edit {
    (
        "song_set",
        json!({"file": file, "path": format!("tracks[{track}].instrument"), "value": instrument}),
    )
}

/// The instruments a track can be played by: the built-in ones, and the
/// song's own instruments and kits.
pub fn instruments(song: &Song) -> Vec<String> {
    let mut names: Vec<String> = duck_echo_tools::instruments::list()
        .map(|list| list.into_iter().map(|i| i.name).collect())
        .unwrap_or_default();
    names.extend(song.instruments.keys().cloned());
    names.extend(song.kits.keys().cloned());
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Song {
        let text = "version = 0\ntempo = 120\n[[tracks]]\nname = \"keys\"\ninstrument = \"piano\"\nnotes = [{ bar = 2, beat = 1, note = \"E5\" }, { beat = 0, note = [\"C4\", \"E4\"] }]\nplay = [{ pattern = \"p\", bars = [1, 3] }]\n[patterns.p]\nnotes = [{ beat = 0, note = \"C4\" }]\n";
        let dir = std::env::temp_dir().join(format!("duck-echo-song-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = duck_echo_tools::root::Root::new(&dir).unwrap();
        crate::library::load_song(&root, "", text).unwrap().0
    }

    #[test]
    fn beats_snap_and_split_into_bars() {
        assert_eq!(snap(5.3, 0.5), 5.5);
        assert_eq!(snap(-0.2, 1.0), 0.0);
        let s = song();
        assert_eq!(bar_and_beat(&s, 0.0), (1, 0.0));
        assert_eq!(bar_and_beat(&s, 5.5), (2, 1.5));
        assert_eq!(bar_and_beat(&s, 8.0), (3, 0.0));
    }

    #[test]
    fn a_chord_moves_together_and_a_kit_note_changes_piece() {
        let s = song();
        let (name, args) = move_note("s.toml", &s, 0, 1, 4.0, 2, None).unwrap();
        assert_eq!(name, "song_move_note");
        assert_eq!(args["note"], json!(["D4", "F#4"]));
        assert_eq!(
            (args["bar"].clone(), args["beat"].clone()),
            (json!(2), json!(0.0))
        );
        let (_, args) = move_note("s.toml", &s, 0, 0, 5.0, 0, None).unwrap();
        assert!(
            args.get("note").is_none(),
            "a sideways move keeps the pitch"
        );
        let (_, args) = move_note("s.toml", &s, 0, 0, 5.0, 0, Some("kick".into())).unwrap();
        assert_eq!(args["note"], json!("kick"));
        assert_eq!(
            transpose(&NoteValue::One(NotePitch::Hz(440.0)), 12),
            Some(json!(880.0))
        );
    }

    #[test]
    fn a_clip_moves_or_leaves_its_placement() {
        let s = song();
        let clip = Clip {
            pattern: "p".into(),
            beat: 8.0,
            length: 4.0,
            transpose: 0,
            placement: 0,
            bar: 3,
        };
        let (_, args) = move_clip("s.toml", &s, 0, &clip, 4).unwrap();
        assert_eq!(args["bars"], json!([1, 4]));
        assert!(
            move_clip("s.toml", &s, 0, &clip, 1).is_none(),
            "bar 1 has one"
        );
        assert!(move_clip("s.toml", &s, 0, &clip, 3).is_none());
        let (name, args) = remove_clip("s.toml", &s, 0, &clip).unwrap();
        assert_eq!(name, "song_move_pattern");
        assert_eq!(args["bars"], json!([1]));
    }
}
