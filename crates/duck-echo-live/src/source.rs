//! What the mixer can play: a live [`Voice`], or samples that were already
//! rendered (a song, a WAV file).
//!
//! Everything a [`Source`] does while playing is allocation-free and
//! lock-free. Making one, and dropping one, happens on the control thread.

use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Recipe, Report};
use duck_echo_synth::live::ParamUpdate;
use duck_echo_synth::{Mastering, Voice};

/// Something that makes samples, frame after frame.
#[derive(Debug)]
pub enum Source {
    /// A sound made as it plays, so sliders can change it live. Boxed so a
    /// message that carries one is small.
    Voice(Box<Voice>),
    /// Samples made earlier: a song rendered in the background, or a file.
    Samples(Samples),
}

/// Rendered samples and how far through them playback is.
#[derive(Debug)]
pub struct Samples {
    /// Interleaved samples.
    data: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    /// The next frame to play.
    position: usize,
}

impl Source {
    /// Samples per second.
    pub fn sample_rate(&self) -> u32 {
        match self {
            Source::Voice(v) => v.sample_rate(),
            Source::Samples(s) => s.sample_rate,
        }
    }

    /// Channels (1 or 2).
    pub fn channels(&self) -> usize {
        match self {
            Source::Voice(v) => v.channels(),
            Source::Samples(s) => s.channels,
        }
    }

    /// Length in frames.
    pub fn total_frames(&self) -> usize {
        match self {
            Source::Voice(v) => v.total_frames(),
            Source::Samples(s) => s.data.len() / s.channels,
        }
    }

    /// The next frame to play.
    pub fn position(&self) -> usize {
        match self {
            Source::Voice(v) => v.position(),
            Source::Samples(s) => s.position,
        }
    }

    /// Fill `out` (interleaved, a whole number of frames) with the next
    /// frames and return how many were made. Fewer than asked means the
    /// end was reached; the rest of `out` is zero.
    pub(crate) fn process(&mut self, out: &mut [f32]) -> usize {
        match self {
            Source::Voice(v) => v.process_live(out),
            Source::Samples(s) => {
                let from = s.position * s.channels;
                let n = out.len().min(s.data.len() - from);
                out[..n].copy_from_slice(&s.data[from..from + n]);
                out[n..].fill(0.0);
                s.position += n / s.channels;
                n / s.channels
            }
        }
    }

    /// Go back to the first frame.
    pub(crate) fn rewind(&mut self) {
        match self {
            Source::Voice(v) => v.rewind(),
            Source::Samples(s) => s.position = 0,
        }
    }

    /// A live change. False when this source cannot take it.
    pub(crate) fn update(&mut self, update: ParamUpdate) -> bool {
        match self {
            Source::Voice(v) => v.update(update),
            Source::Samples(_) => false,
        }
    }

    /// Move `frames` further on without anything being heard, using
    /// `scratch` to make (and throw away) a voice's samples.
    pub(crate) fn skip(&mut self, frames: usize, scratch: &mut [f32]) {
        match self {
            Source::Voice(v) => {
                let ch = v.channels();
                let mut left = frames;
                while left > 0 {
                    let n = left.min(scratch.len() / ch);
                    if v.process_live(&mut scratch[..n * ch]) == 0 {
                        break;
                    }
                    left -= n;
                }
            }
            Source::Samples(s) => {
                s.position = (s.position + frames).min(s.data.len() / s.channels);
            }
        }
    }
}

/// A sound ready to play: a [`Source`], how loud to play it, and where it
/// ends. Made on the control thread, where allocating is fine.
#[derive(Debug)]
pub struct Sound {
    pub(crate) source: Source,
    /// The gain the mixer plays it at. For a recipe this is the gain that
    /// `master.normalize` or `master.normalize_lufs` gives the rendered
    /// file, so the speakers and the file are equally loud.
    pub(crate) level: f32,
    /// Frames to play. For a recipe this is the rendered file's length
    /// (after `master.trim_silence`).
    pub(crate) length: usize,
}

impl Sound {
    /// A recipe as a live voice that plays exactly what `render` writes to
    /// a file: the same samples at the same level and length.
    ///
    /// The file's level comes from the offline master steps, which need
    /// the whole sound. So this renders the recipe once to learn them. If
    /// you already rendered it, use [`Sound::from_recipe_mastered`].
    pub fn from_recipe(recipe: &Recipe) -> Result<Sound, Report> {
        let (_, mastering) = duck_echo_synth::render_mastered(recipe)?;
        Sound::from_recipe_mastered(recipe, &mastering)
    }

    /// [`Sound::from_recipe`] with the `mastering` of a render of the same
    /// recipe (from [`duck_echo_synth::render_mastered`]).
    pub fn from_recipe_mastered(recipe: &Recipe, mastering: &Mastering) -> Result<Sound, Report> {
        let voice = duck_echo_synth::prepare(recipe)?;
        let length = mastering.frames.min(voice.total_frames()).max(1);
        Ok(Sound {
            source: Source::Voice(Box::new(voice)),
            level: mastering.gain,
            length,
        })
    }

    /// Samples that are already finished (a rendered song, a WAV file),
    /// played as they are.
    pub fn from_samples(buffer: AudioBuffer) -> Sound {
        let channels = (buffer.channels as usize).max(1);
        let mut data = buffer.samples;
        data.truncate(data.len() / channels * channels);
        if data.is_empty() {
            data.resize(channels, 0.0);
        }
        let length = data.len() / channels;
        Sound {
            source: Source::Samples(Samples {
                data,
                channels,
                sample_rate: buffer.sample_rate,
                position: 0,
            }),
            level: 1.0,
            length,
        }
    }

    /// Samples per second.
    pub fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    /// Channels (1 or 2).
    pub fn channels(&self) -> usize {
        self.source.channels()
    }

    /// Frames that will play.
    pub fn length(&self) -> usize {
        self.length
    }

    /// The gain it plays at.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Set how long the voice's live changes take to arrive (see
    /// [`Voice::set_smoothing`]). Only the click test passes 0, to show
    /// what the default 10 ms is for.
    pub fn set_smoothing(&mut self, seconds: f32) {
        if let Source::Voice(v) = &mut self.source {
            v.set_smoothing(seconds);
        }
    }

    /// Move to `frame` without a sound, so this can take over from a sound
    /// that is already that far in. Done on the control thread: it makes
    /// (and throws away) the samples before `frame`.
    pub(crate) fn advance_to(&mut self, frame: usize) {
        let frame = frame.min(self.length);
        let mut scratch = vec![0.0f32; 4096 * self.source.channels()];
        let behind = frame.saturating_sub(self.source.position());
        self.source.skip(behind, &mut scratch);
    }
}
