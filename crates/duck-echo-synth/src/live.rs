//! Live playback: changing a [`Voice`] while it plays, and repeating it.
//!
//! When a person drags a slider while a sound plays, three numbers can
//! change without making a new voice: a layer's volume, a filter's cutoff,
//! and the master gain. A sudden jump in any of them is heard as a click.
//! So a change does not jump: the value **slides** to its target over
//! [`SMOOTH_SECONDS`] (10 ms), a little each sample.
//!
//! How it stays out of the renderer's way: [`Voice::process`] and the code
//! under it are not changed at all. [`Voice::process_live`] is a wrapper.
//! While nothing is sliding it just calls `process`. While something
//! slides, it moves each sliding value one small step and then calls
//! `process` for **one frame**, again and again. `process` already works
//! for any block length and gives the same samples however the blocks are
//! cut (a test checks this), so one frame at a time is the same sound.
//!
//! So an offline render, which never sends a [`ParamUpdate`], runs the
//! very same code on the very same numbers as before this file existed.
//!
//! Everything here is allocation-free and lock-free, like `process`: the
//! sliding state is a few plain numbers that live inside the voice.

use crate::Voice;

/// How long a live change takes to arrive: 10 ms. Short enough to feel
/// instant, long enough that the step from one sample to the next is far
/// smaller than the sound's own.
pub const SMOOTH_SECONDS: f32 = 0.01;

/// A change to a playing voice, as plain numbers. These are the fields a
/// slider can change without building a new voice. Any other change (a new
/// layer, another wave, an envelope) needs a new [`Voice`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamUpdate {
    /// `layers[layer].volume` in the recipe.
    LayerVolume {
        /// The layer's position in the recipe's `layers`.
        layer: usize,
        /// The new volume.
        volume: f32,
    },
    /// `layers[layer].filter.cutoff` and `cutoff_end` in the recipe.
    FilterCutoff {
        /// The layer's position in the recipe's `layers`.
        layer: usize,
        /// The new `cutoff`, in Hz.
        cutoff: f32,
        /// The new `cutoff_end`, in Hz (`None`: the cutoff does not move).
        cutoff_end: Option<f32>,
    },
    /// `master.gain` in the recipe.
    MasterGain {
        /// The new gain.
        gain: f32,
    },
}

/// One number sliding to a target in equal steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ramp {
    target: f32,
    step: f32,
    /// Frames still to go. 0 means idle.
    left: u32,
}

impl Ramp {
    pub(crate) const IDLE: Ramp = Ramp {
        target: 0.0,
        step: 0.0,
        left: 0,
    };

    /// Start sliding from `current` to `target` over `frames` frames.
    fn to(current: f32, target: f32, frames: u32) -> Ramp {
        Ramp {
            target,
            step: (target - current) / frames as f32,
            left: frames,
        }
    }

    /// The value one frame on from `current`. The last step lands exactly
    /// on the target, so no rounding error is left behind.
    fn next(&mut self, current: f32) -> f32 {
        self.left -= 1;
        if self.left == 0 {
            self.target
        } else {
            current + self.step
        }
    }
}

/// A filter's cutoff sweep sliding to another: both ends of the sweep move
/// in a straight line from the old values to the new.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CutoffRamp {
    from: (f32, f32),
    to: (f32, f32),
    /// The target's `cutoff_end` as written (`None`: constant).
    to_end: Option<f32>,
    total: u32,
    left: u32,
}

/// A layer's live changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayerLive {
    volume: Ramp,
    cutoff: CutoffRamp,
}

impl LayerLive {
    pub(crate) const IDLE: LayerLive = LayerLive {
        volume: Ramp::IDLE,
        cutoff: CutoffRamp {
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            to_end: None,
            total: 0,
            left: 0,
        },
    };
}

/// A voice's live changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VoiceLive {
    gain: Ramp,
    /// Frames a change takes. 0 turns sliding off (a change jumps).
    smooth_frames: u32,
    /// True while any value in the voice is sliding.
    sliding: bool,
}

impl VoiceLive {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            gain: Ramp::IDLE,
            smooth_frames: smooth_frames(sample_rate, SMOOTH_SECONDS),
            sliding: false,
        }
    }
}

/// `seconds` as a whole number of frames.
fn smooth_frames(sample_rate: u32, seconds: f32) -> u32 {
    (sample_rate as f32 * seconds.max(0.0)).round() as u32
}

impl Voice {
    /// Set how long a live change takes to arrive, in seconds. The default
    /// is [`SMOOTH_SECONDS`]. With 0 a change jumps at once, which clicks;
    /// only the click test does that, to show what sliding is for.
    pub fn set_smoothing(&mut self, seconds: f32) {
        self.live.smooth_frames = smooth_frames(self.sample_rate, seconds);
    }

    /// Change a playing voice. The value slides to its target over the
    /// next 10 ms of [`Self::process_live`]. A second change on the way
    /// starts from wherever the value is now, so it never jumps.
    ///
    /// Returns false, changing nothing, when the voice has no such layer
    /// or the layer has no filter (the caller needs a new voice then).
    /// Allocation-free and lock-free.
    pub fn update(&mut self, update: ParamUpdate) -> bool {
        let frames = self.live.smooth_frames;
        match update {
            ParamUpdate::LayerVolume { layer, volume } => {
                let Some(l) = self.layers.iter_mut().find(|l| l.index == layer) else {
                    return false;
                };
                if frames == 0 {
                    l.volume = volume;
                    l.live.volume = Ramp::IDLE;
                } else {
                    l.live.volume = Ramp::to(l.volume, volume, frames);
                }
            }
            ParamUpdate::FilterCutoff {
                layer,
                cutoff,
                cutoff_end,
            } => {
                let Some(l) = self.layers.iter_mut().find(|l| l.index == layer) else {
                    return false;
                };
                let Some((_, sweep)) = &mut l.filter else {
                    return false;
                };
                if frames == 0 {
                    *sweep = sweep.with_ends(cutoff, cutoff_end);
                    l.live.cutoff.left = 0;
                } else {
                    l.live.cutoff = CutoffRamp {
                        from: (sweep.start(), sweep.end()),
                        to: (cutoff, cutoff_end.unwrap_or(cutoff)),
                        to_end: cutoff_end,
                        total: frames,
                        left: frames,
                    };
                }
            }
            ParamUpdate::MasterGain { gain } => {
                if frames == 0 {
                    self.gain = gain;
                    self.live.gain = Ramp::IDLE;
                } else {
                    self.live.gain = Ramp::to(self.gain, gain, frames);
                }
            }
        }
        self.live.sliding = frames != 0 || self.any_sliding();
        true
    }

    fn any_sliding(&self) -> bool {
        self.live.gain.left > 0
            || self
                .layers
                .iter()
                .any(|l| l.live.volume.left > 0 || l.live.cutoff.left > 0)
    }

    /// True while a live change is still sliding to its target.
    pub fn sliding(&self) -> bool {
        self.live.sliding
    }

    /// Move every sliding value one frame on.
    fn slide_one_frame(&mut self) {
        let mut still = false;
        if self.live.gain.left > 0 {
            self.gain = self.live.gain.next(self.gain);
            still |= self.live.gain.left > 0;
        }
        for l in &mut self.layers {
            if l.live.volume.left > 0 {
                l.volume = l.live.volume.next(l.volume);
                still |= l.live.volume.left > 0;
            }
            let c = &mut l.live.cutoff;
            if c.left > 0 {
                c.left -= 1;
                if let Some((_, sweep)) = &mut l.filter {
                    *sweep = if c.left == 0 {
                        // Exactly the sweep a new voice would have.
                        sweep.with_ends(c.to.0, c.to_end)
                    } else {
                        let done = 1.0 - c.left as f32 / c.total as f32;
                        sweep.with_ends(
                            c.from.0 + (c.to.0 - c.from.0) * done,
                            Some(c.from.1 + (c.to.1 - c.from.1) * done),
                        )
                    };
                }
                still |= c.left > 0;
            }
        }
        self.live.sliding = still;
    }

    /// [`Self::process`] for live playback: the same samples, with live
    /// changes sliding in. While nothing slides this is `process` itself.
    /// While something slides, each frame is made on its own, after every
    /// sliding value has moved one step. Allocation-free and lock-free.
    pub fn process_live(&mut self, out: &mut [f32]) -> usize {
        let mut rest = out;
        let mut written = 0;
        while !rest.is_empty() {
            if !self.live.sliding {
                return written + self.process(rest);
            }
            let take = self.channels.min(rest.len());
            let (frame, tail) = std::mem::take(&mut rest).split_at_mut(take);
            self.slide_one_frame();
            written += self.process(frame);
            rest = tail;
        }
        written
    }

    /// Go back to the first frame, to play the sound again: the same
    /// samples as a new voice would make, without building one. Live
    /// changes stay as they are (a volume that was turned down stays
    /// down). Allocates nothing: delay lines are cleared in place.
    ///
    /// The offline renderer never calls this.
    pub fn rewind(&mut self) {
        self.position = 0;
        for l in &mut self.layers {
            l.local = 0;
            l.control_counter = 0;
            l.osc.rewind(l.duty);
            if let Some((filter, _)) = &mut l.filter {
                filter.reset();
            }
            for fx in &mut l.effects {
                fx.reset();
            }
        }
        for chain in &mut self.master_effects {
            for fx in chain {
                fx.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepare, RENDER_BLOCK};
    use duck_echo_core::{load_str, Format, Recipe};

    /// A stereo recipe with everything that keeps state: noise, an LFO on
    /// the pulse width, a swept filter with an LFO, and all four effects.
    const BUSY: &str = r#"
version = 0
channels = 2
seed = 7
variation = 0.2
effects = [{ type = "reverb", decay = 0.3, mix = 0.2 }]

[master]
gain = 0.8

[[layers]]
source = "square"
duty = 0.3
pan = -0.4
volume = 0.5
envelope = { attack = 0.01, decay = 0.2, sustain = 0.4, sustain_time = 0.1, release = 0.1 }
pitch = { start = "A3", end = "A4", vibrato = { rate = 6, depth = 0.3 } }
lfo = { target = "duty", rate = 3, depth = 0.2 }
filter = { cutoff = 800, cutoff_end = 3000, resonance = 2 }
effects = [{ type = "echo", delay = 0.05, feedback = 0.4 }, { type = "bitcrush", bits = 6, rate = 9000 }]

[[layers]]
source = "noise"
noise_mode = "pitched"
pan = 0.5
volume = 0.3
offset = 0.05
envelope = { attack = 0.005, decay = 0.25 }
pitch = { start = 900 }
filter = { kind = "bandpass", cutoff = 1200 }
lfo = { target = "cutoff", rate = 4, depth = 1 }
effects = [{ type = "distortion", drive = 3 }]

[[layers]]
source = "noise"
volume = 0.2
envelope = { attack = 0.0, decay = 0.1 }
"#;

    /// A plain sine with no filter and no effects, so nothing remembers
    /// the past: after a change has arrived, the samples must be exactly
    /// those of a new voice.
    const PLAIN: &str = r#"
version = 0
[[layers]]
source = "sine"
volume = 0.4
envelope = { attack = 0.01, sustain = 1.0, sustain_time = 0.5, release = 0.05 }
pitch = { start = 220 }
"#;

    fn recipe(src: &str) -> Recipe {
        load_str(src, Format::Toml).unwrap().recipe
    }

    /// Run a voice to its end with `step`, in blocks of `block` samples.
    fn run(
        mut voice: Voice,
        block: usize,
        step: impl Fn(&mut Voice, &mut [f32]) -> usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; voice.total_frames() * voice.channels()];
        for chunk in out.chunks_mut(block) {
            step(&mut voice, chunk);
        }
        out
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|s| s.to_bits()).collect()
    }

    #[test]
    fn with_no_live_change_process_live_is_process_bit_for_bit() {
        for src in [BUSY, PLAIN] {
            let voice = prepare(&recipe(src)).unwrap();
            let block = RENDER_BLOCK * voice.channels();
            let offline = run(voice.clone(), block, |v, c| v.process(c));
            let live = run(voice.clone(), block, |v, c| v.process_live(c));
            assert_eq!(bits(&offline), bits(&live));
            // An odd block size too.
            let odd = run(voice, 334, |v, c| v.process_live(c));
            assert_eq!(bits(&offline), bits(&odd));
        }
    }

    #[test]
    fn one_frame_at_a_time_gives_the_same_samples_as_big_blocks() {
        // Sliding makes the voice one frame at a time. That is only safe
        // because the cut of the blocks never changes a sample.
        for src in [BUSY, PLAIN] {
            let voice = prepare(&recipe(src)).unwrap();
            let ch = voice.channels();
            let big = run(voice.clone(), RENDER_BLOCK * ch, |v, c| v.process(c));
            let single = run(voice, ch, |v, c| v.process(c));
            assert_eq!(bits(&big), bits(&single));
        }
    }

    #[test]
    fn a_rewound_voice_makes_the_same_samples_as_a_new_one() {
        let voice = prepare(&recipe(BUSY)).unwrap();
        let block = RENDER_BLOCK * voice.channels();
        let fresh = run(voice.clone(), block, |v, c| v.process(c));
        let mut used = voice;
        let mut sink = vec![0.0f32; fresh.len()];
        used.process(&mut sink);
        assert!(used.finished());
        used.rewind();
        assert_eq!(used.position(), 0);
        let again = run(used.clone(), block, |v, c| v.process(c));
        assert_eq!(bits(&fresh), bits(&again));
        // Rewinding half way through works too.
        let mut half = used;
        half.rewind();
        half.process(&mut sink[..fresh.len() / 2]);
        half.rewind();
        let third = run(half, block, |v, c| v.process(c));
        assert_eq!(bits(&fresh), bits(&third));
    }

    #[test]
    fn a_volume_change_slides_and_then_matches_a_new_voice_exactly() {
        let old = recipe(PLAIN);
        let mut new = old.clone();
        new.layers[0].volume = 1.0;
        let mut voice = prepare(&old).unwrap();
        let reference = run(prepare(&new).unwrap(), RENDER_BLOCK, |v, c| v.process(c));
        let frames = voice.total_frames();
        let mut out = vec![0.0f32; frames];
        // Play 0.1 s, change the volume, play the rest.
        let at = 4410;
        voice.process_live(&mut out[..at]);
        assert!(voice.update(ParamUpdate::LayerVolume {
            layer: 0,
            volume: 1.0
        }));
        assert!(voice.sliding());
        voice.process_live(&mut out[at..]);
        assert!(!voice.sliding());
        // 10 ms later the voice is the new recipe's voice, to the bit.
        let arrived = at + 441;
        assert_eq!(bits(&out[arrived..]), bits(&reference[arrived..]));
        // On the way, the level rises steadily from the old sound's to the
        // new one's: sample / reference goes from 0.4 to 1.0.
        let mut last = 0.39;
        for i in (at..arrived).step_by(40) {
            if reference[i].abs() > 0.05 {
                let share = out[i] / reference[i];
                assert!(
                    share > last && share <= 1.0,
                    "frame {i}: {share} after {last}"
                );
                last = share;
            }
        }
        assert!(last > 0.9);
    }

    #[test]
    fn the_master_gain_slides_and_lands_exactly() {
        let old = recipe(PLAIN);
        let mut new = old.clone();
        new.master.gain = 0.25;
        let mut voice = prepare(&old).unwrap();
        let reference = run(prepare(&new).unwrap(), RENDER_BLOCK, |v, c| v.process(c));
        let mut out = vec![0.0f32; voice.total_frames()];
        voice.process_live(&mut out[..1000]);
        voice.update(ParamUpdate::MasterGain { gain: 0.25 });
        voice.process_live(&mut out[1000..]);
        assert_eq!(bits(&out[1441..]), bits(&reference[1441..]));
        assert_ne!(bits(&out[1000..1441]), bits(&reference[1000..1441]));
    }

    #[test]
    fn a_cutoff_change_slides_to_the_new_sweep() {
        let src = PLAIN.replace(
            "source = \"sine\"",
            "source = \"saw\"\nfilter = { cutoff = 400 }",
        );
        let old = recipe(&src);
        let mut new = old.clone();
        new.layers[0].filter.as_mut().unwrap().cutoff = 4000.0;
        new.layers[0].filter.as_mut().unwrap().cutoff_end = Some(2000.0);
        let mut voice = prepare(&old).unwrap();
        let reference = run(prepare(&new).unwrap(), RENDER_BLOCK, |v, c| v.process(c));
        let mut out = vec![0.0f32; voice.total_frames()];
        voice.process_live(&mut out[..2000]);
        assert!(voice.update(ParamUpdate::FilterCutoff {
            layer: 0,
            cutoff: 4000.0,
            cutoff_end: Some(2000.0),
        }));
        voice.process_live(&mut out[2000..]);
        assert!(!voice.sliding());
        // The filter remembers a little of the old sound, so the match is
        // close at first and then exact to rounding.
        let later = 2000 + 441 + 2000;
        let worst = out[later..]
            .iter()
            .zip(&reference[later..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
        // Before the change it was the old sound: darker than the new.
        assert_ne!(bits(&out[..2000]), bits(&reference[..2000]));
    }

    #[test]
    fn a_change_to_something_that_is_not_there_is_refused() {
        let mut voice = prepare(&recipe(PLAIN)).unwrap();
        assert!(!voice.update(ParamUpdate::LayerVolume {
            layer: 3,
            volume: 1.0
        }));
        // The sine has no filter.
        assert!(!voice.update(ParamUpdate::FilterCutoff {
            layer: 0,
            cutoff: 500.0,
            cutoff_end: None,
        }));
        assert!(!voice.sliding());
    }

    #[test]
    fn with_smoothing_off_a_change_jumps_at_once() {
        let mut voice = prepare(&recipe(PLAIN)).unwrap();
        voice.set_smoothing(0.0);
        let mut new = recipe(PLAIN);
        new.layers[0].volume = 1.0;
        let reference = run(prepare(&new).unwrap(), RENDER_BLOCK, |v, c| v.process(c));
        let mut out = vec![0.0f32; voice.total_frames()];
        voice.process_live(&mut out[..3000]);
        voice.update(ParamUpdate::LayerVolume {
            layer: 0,
            volume: 1.0,
        });
        assert!(!voice.sliding());
        voice.process_live(&mut out[3000..]);
        assert_eq!(bits(&out[3000..]), bits(&reference[3000..]));
    }

    #[test]
    fn a_second_change_starts_from_where_the_first_had_got_to() {
        let mut voice = prepare(&recipe(PLAIN)).unwrap();
        let mut out = vec![0.0f32; 8000];
        voice.process_live(&mut out[..4000]);
        voice.update(ParamUpdate::LayerVolume {
            layer: 0,
            volume: 1.0,
        });
        voice.process_live(&mut out[4000..4200]);
        // Half way up, turn it down again.
        voice.update(ParamUpdate::LayerVolume {
            layer: 0,
            volume: 0.0,
        });
        voice.process_live(&mut out[4200..]);
        // No step is bigger than the plain sine's own biggest step at full
        // volume (2 pi 220 / 44100 = 0.031) plus a hair.
        let worst = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.032, "{worst}");
        // And it ends silent: the volume arrived at 0.
        assert!(out[7000..].iter().all(|s| *s == 0.0));
    }
}
