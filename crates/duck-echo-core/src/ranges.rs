//! The allowed range of every number in a recipe or a song, in one table.
//!
//! Three readers use it, so they can never disagree:
//!
//! * [`crate::validate`] and the song checks refuse a value outside its
//!   range;
//! * [`crate::schema`] writes each range into the JSON Schema as `minimum`
//!   and `maximum`, where agents read them;
//! * Duck Echo Studio makes each slider from the schema, so a slider can
//!   never be dragged to a value that validation would refuse.
//!
//! Before this table the numbers were written by hand inside the checks,
//! and the schema only mentioned them in words.

use crate::recipe::LfoTarget;

/// The allowed range of one number field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// The schema type the field belongs to: `Layer`, `Envelope`, `Echo`...
    /// `Recipe` and `Song` are the top level of their documents.
    pub owner: &'static str,
    /// The field's name. `steps[]` means each item of the list `steps`.
    /// Empty when the range belongs to the type itself (`Frequency`, a
    /// number of Hz wherever a pitch is written).
    pub field: &'static str,
    /// The smallest allowed value.
    pub min: f64,
    /// The largest allowed value.
    pub max: f64,
    /// True when `min` itself is not allowed (a note's length must be
    /// more than 0).
    pub above_min: bool,
}

impl Range {
    /// True when `value` is allowed.
    pub fn contains(&self, value: f64) -> bool {
        let above = if self.above_min {
            value > self.min
        } else {
            value >= self.min
        };
        above && value <= self.max
    }
}

/// Declare the ranges: a constant for each (the checks use these), and
/// [`ALL`], the list of every one (the schema uses this).
macro_rules! ranges {
    ($( $(#[$doc:meta])* $name:ident: $owner:literal . $field:literal = $min:literal ..= $max:literal $(, $above:ident)?; )*) => {
        $(
            $(#[$doc])*
            pub const $name: Range = Range {
                owner: $owner,
                field: $field,
                min: $min,
                max: $max,
                above_min: ranges!(@above $($above)?),
            };
        )*
        /// Every range, in the order of the documents.
        pub const ALL: &[Range] = &[$($name),*];
    };
    (@above) => { false };
    (@above above_min) => { true };
}

ranges! {
    // The top level of a recipe.
    /// Samples per second.
    SAMPLE_RATE: "Recipe"."sample_rate" = 8000.0 ..= 192000.0;
    /// 1 is mono, 2 is stereo.
    CHANNELS: "Recipe"."channels" = 1.0 ..= 2.0;
    /// How much a seed changes pitch and timing.
    VARIATION: "Recipe"."variation" = 0.0 ..= 1.0;
    /// A fixed length in seconds.
    DURATION: "Recipe"."duration" = 0.001 ..= 60.0;

    // `master`.
    /// Linear output gain.
    MASTER_GAIN: "Master"."gain" = 0.0 ..= 4.0;
    /// Peak target in dBFS for `normalize`.
    MASTER_NORMALIZE_PEAK_DB: "Master"."normalize_peak_db" = -40.0 ..= 0.0;
    /// Loudness target in LUFS.
    MASTER_NORMALIZE_LUFS: "Master"."normalize_lufs" = -60.0 ..= 0.0;

    // A layer.
    /// Linear volume.
    LAYER_VOLUME: "Layer"."volume" = 0.0 ..= 2.0;
    /// Start time in seconds.
    LAYER_OFFSET: "Layer"."offset" = 0.0 ..= 30.0;
    /// Pulse width of a square wave.
    LAYER_DUTY: "Layer"."duty" = 0.01 ..= 0.99;
    /// Left (-1) to right (1).
    LAYER_PAN: "Layer"."pan" = -1.0 ..= 1.0;

    // A layer's LFO. `depth` depends on the target: see [`lfo_depth`].
    /// Cycles per second.
    LFO_RATE: "Lfo"."rate" = 0.01 ..= 50.0;
    /// Seconds before the LFO starts.
    LFO_DELAY: "Lfo"."delay" = 0.0 ..= 30.0;
    /// The widest `depth` any target allows (the schema's range).
    LFO_DEPTH: "Lfo"."depth" = 0.0 ..= 4.0;

    // The envelope.
    /// Seconds to rise to full level.
    ENVELOPE_ATTACK: "Envelope"."attack" = 0.0 ..= 30.0;
    /// Seconds held at full level.
    ENVELOPE_HOLD: "Envelope"."hold" = 0.0 ..= 30.0;
    /// Seconds to fall to the sustain level.
    ENVELOPE_DECAY: "Envelope"."decay" = 0.0 ..= 30.0;
    /// The level held after the decay.
    ENVELOPE_SUSTAIN: "Envelope"."sustain" = 0.0 ..= 1.0;
    /// Seconds held at the sustain level.
    ENVELOPE_SUSTAIN_TIME: "Envelope"."sustain_time" = 0.0 ..= 30.0;
    /// Seconds to fall to silence.
    ENVELOPE_RELEASE: "Envelope"."release" = 0.0 ..= 30.0;

    // Pitch.
    /// A pitch written as a number, in Hz.
    FREQUENCY: "Frequency"."" = 1.0 ..= 20000.0;
    /// Seconds a pitch sweep takes.
    PITCH_SWEEP_TIME: "Pitch"."sweep_time" = 0.001 ..= 30.0;
    /// Vibrato cycles per second.
    VIBRATO_RATE: "Vibrato"."rate" = 0.0 ..= 100.0;
    /// Vibrato depth in semitones.
    VIBRATO_DEPTH: "Vibrato"."depth" = 0.0 ..= 48.0;
    /// Seconds before the vibrato starts.
    VIBRATO_DELAY: "Vibrato"."delay" = 0.0 ..= 30.0;
    /// One arpeggio step, in semitones.
    ARPEGGIO_STEP: "Arpeggio"."steps[]" = -96.0 ..= 96.0;
    /// Seconds each arpeggio step lasts.
    ARPEGGIO_STEP_TIME: "Arpeggio"."step_time" = 0.001 ..= 30.0;

    // The filter.
    /// Cutoff in Hz.
    FILTER_CUTOFF: "Filter"."cutoff" = 10.0 ..= 20000.0;
    /// Cutoff at the end of a sweep, in Hz.
    FILTER_CUTOFF_END: "Filter"."cutoff_end" = 10.0 ..= 20000.0;
    /// Seconds a cutoff sweep takes.
    FILTER_SWEEP_TIME: "Filter"."sweep_time" = 0.001 ..= 30.0;
    /// Resonance (Q).
    FILTER_RESONANCE: "Filter"."resonance" = 0.1 ..= 30.0;

    // The effects.
    /// Seconds between echoes.
    ECHO_DELAY: "Echo"."delay" = 0.001 ..= 5.0;
    /// How much of each echo feeds the next.
    ECHO_FEEDBACK: "Echo"."feedback" = 0.0 ..= 0.99;
    /// Dry (0) to wet (1).
    ECHO_MIX: "Echo"."mix" = 0.0 ..= 1.0;
    /// Room size.
    REVERB_SIZE: "Reverb"."size" = 0.0 ..= 1.0;
    /// Seconds for the tail to fall 60 dB.
    REVERB_DECAY: "Reverb"."decay" = 0.05 ..= 10.0;
    /// How fast the highs die away.
    REVERB_DAMPING: "Reverb"."damping" = 0.0 ..= 1.0;
    /// Dry (0) to wet (1).
    REVERB_MIX: "Reverb"."mix" = 0.0 ..= 1.0;
    /// Gain before the shaper.
    DISTORTION_DRIVE: "Distortion"."drive" = 1.0 ..= 100.0;
    /// Dry (0) to wet (1).
    DISTORTION_MIX: "Distortion"."mix" = 0.0 ..= 1.0;
    /// Bit depth.
    BITCRUSH_BITS: "Bitcrush"."bits" = 1.0 ..= 16.0;
    /// Sample-and-hold rate in Hz.
    BITCRUSH_RATE: "Bitcrush"."rate" = 100.0 ..= 192000.0;
    /// Dry (0) to wet (1).
    BITCRUSH_MIX: "Bitcrush"."mix" = 0.0 ..= 1.0;

    // The top level of a song.
    /// Beats per minute.
    SONG_TEMPO: "Song"."tempo" = 20.0 ..= 400.0;
    /// Beats in a bar.
    SONG_BEATS_PER_BAR: "Song"."beats_per_bar" = 1.0 ..= 16.0;
    /// Samples per second.
    SONG_SAMPLE_RATE: "Song"."sample_rate" = 8000.0 ..= 192000.0;
    /// 1 is mono, 2 is stereo.
    SONG_CHANNELS: "Song"."channels" = 1.0 ..= 2.0;

    // A track and its notes.
    /// Linear volume.
    TRACK_VOLUME: "Track"."volume" = 0.0 ..= 2.0;
    /// Left (-1) to right (1).
    TRACK_PAN: "Track"."pan" = -1.0 ..= 1.0;
    /// Semitones a placed pattern is moved.
    PLACEMENT_TRANSPOSE: "Placement"."transpose" = -48.0 ..= 48.0;
    /// When a note starts, in beats.
    NOTE_BEAT: "Note"."beat" = 0.0 ..= 100000.0;
    /// How long a note is held, in beats: more than 0.
    NOTE_LENGTH: "Note"."length" = 0.0 ..= 100000.0, above_min;
    /// How hard a note is played.
    NOTE_VELOCITY: "Note"."velocity" = 0.0 ..= 1.0;
    /// When a pattern's note starts, in beats.
    PATTERN_NOTE_BEAT: "PatternNote"."beat" = 0.0 ..= 100000.0;
    /// How long a pattern's note is held, in beats: more than 0.
    PATTERN_NOTE_LENGTH: "PatternNote"."length" = 0.0 ..= 100000.0, above_min;
    /// How hard a pattern's note is played.
    PATTERN_NOTE_VELOCITY: "PatternNote"."velocity" = 0.0 ..= 1.0;
    /// A note written as a number, in Hz.
    NOTE_PITCH_HZ: "NotePitch"."" = 1.0 ..= 20000.0;
}

/// The range of an LFO's `depth`, which depends on what the LFO moves: a
/// part of the volume (0 to 1), octaves of cutoff (0 to 4), or an amount of
/// duty (0 to 0.5).
pub fn lfo_depth(target: LfoTarget) -> Range {
    let max = match target {
        LfoTarget::Volume => 1.0,
        LfoTarget::Cutoff => 4.0,
        LfoTarget::Duty => 0.5,
    };
    Range { max, ..LFO_DEPTH }
}

/// The range of `owner.field`, if the table has one.
pub fn find(owner: &str, field: &str) -> Option<Range> {
    ALL.iter()
        .copied()
        .find(|r| r.owner == owner && r.field == field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_field_is_listed_twice_and_every_range_is_in_order() {
        for (i, r) in ALL.iter().enumerate() {
            assert!(r.min < r.max, "{}.{}", r.owner, r.field);
            assert!(
                !ALL[..i]
                    .iter()
                    .any(|o| o.owner == r.owner && o.field == r.field),
                "{}.{} twice",
                r.owner,
                r.field
            );
        }
        assert_eq!(find("Layer", "volume"), Some(LAYER_VOLUME));
        assert_eq!(find("Layer", "nope"), None);
    }

    #[test]
    fn the_widest_lfo_depth_is_the_table_entry() {
        assert_eq!(lfo_depth(LfoTarget::Cutoff), LFO_DEPTH);
        assert_eq!(lfo_depth(LfoTarget::Volume).max, 1.0);
        assert_eq!(lfo_depth(LfoTarget::Duty).max, 0.5);
    }

    #[test]
    fn a_range_can_leave_out_its_smallest_value() {
        assert!(!NOTE_LENGTH.contains(0.0));
        assert!(NOTE_LENGTH.contains(0.001));
        assert!(NOTE_BEAT.contains(0.0));
        assert!(!NOTE_BEAT.contains(100000.5));
    }
}
