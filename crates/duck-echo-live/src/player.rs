//! The control thread's half: the [`Player`].
//!
//! Everything here may allocate, free and take its time. The player makes
//! messages, sends them to the [`Mixer`], and drops the sounds the mixer
//! hands back.

use crate::mixer::{Mixer, ToAudio, ToControl, FADE_SECONDS, QUEUE, SLOTS};
use crate::source::Sound;
use crate::status::Status;
use duck_echo_synth::live::ParamUpdate;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;

/// How the mixer is set up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    /// Samples per second of the sound card. A sound must have the same
    /// rate to play: the mixer does not resample.
    pub sample_rate: u32,
    /// Channels of the sound card: 1 or 2.
    pub channels: usize,
    /// How long a stopped sound takes to fade out and a replacement to
    /// fade in. The default is [`FADE_SECONDS`] (10 ms). With 0 a sound
    /// is cut at once, which clicks; only the click test does that.
    pub fade_seconds: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sample_rate: 44_100,
            channels: 2,
            fade_seconds: FADE_SECONDS,
        }
    }
}

/// Make the two halves: the [`Player`] for the control thread and the
/// [`Mixer`] for the audio thread. All the mixer's memory is made here.
pub fn live(config: Config) -> (Player, Mixer) {
    let (to_audio, from_control) = RingBuffer::new(QUEUE);
    let (to_control, from_audio) = RingBuffer::new(QUEUE);
    let status = Arc::new(Status::new());
    let mixer = Mixer::new(
        config.sample_rate,
        config.channels,
        config.fade_seconds,
        from_control,
        to_control,
        status.clone(),
    );
    let player = Player {
        sample_rate: config.sample_rate,
        channels: mixer.channels(),
        to_audio,
        from_audio,
        status,
    };
    (player, mixer)
}

/// Why a request could not be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayError {
    /// There is no slot with this number (slots are 0 to 31).
    NoSuchSlot {
        /// The slot asked for.
        slot: usize,
    },
    /// The sound has another sample rate than the speakers.
    SampleRate {
        /// The sound's samples per second.
        sound: u32,
        /// The speakers' samples per second.
        speakers: u32,
    },
    /// The mixer has not taken the earlier messages yet (the sound card
    /// is stopped or very busy). Nothing was sent; try again shortly.
    Busy,
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::NoSuchSlot { slot } => write!(
                f,
                "there is no slot {slot}; slots are numbered 0 to {}",
                SLOTS - 1
            ),
            PlayError::SampleRate { sound, speakers } => write!(
                f,
                "the sound is {sound} Hz but the speakers run at {speakers} Hz; open the speakers at {sound} Hz to play it"
            ),
            PlayError::Busy => write!(
                f,
                "the speakers are not taking messages right now; try again in a moment"
            ),
        }
    }
}

impl std::error::Error for PlayError {}

/// The control thread's handle on the mixer.
pub struct Player {
    sample_rate: u32,
    channels: usize,
    to_audio: Producer<ToAudio>,
    from_audio: Consumer<ToControl>,
    status: Arc<Status>,
}

impl Player {
    /// Samples per second of the mixer.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channels of the mixer.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// What the mixer reports: play heads, clipping, refused messages.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// How many more messages the mixer's queue can take right now.
    pub fn room(&self) -> usize {
        self.to_audio.slots()
    }

    fn send(&mut self, message: ToAudio) -> Result<(), PlayError> {
        // A message that does not fit is dropped here, on the control
        // thread, where dropping a sound is fine.
        self.to_audio.push(message).map_err(|_| PlayError::Busy)
    }

    fn check(&self, slot: usize, sound: Option<&Sound>) -> Result<(), PlayError> {
        if slot >= SLOTS {
            return Err(PlayError::NoSuchSlot { slot });
        }
        match sound {
            Some(s) if s.sample_rate() != self.sample_rate => Err(PlayError::SampleRate {
                sound: s.sample_rate(),
                speakers: self.sample_rate,
            }),
            _ => Ok(()),
        }
    }

    /// Play `sound` in `slot` from its start, repeating it when `looping`.
    /// A sound already playing there fades out over 10 ms.
    pub fn play(&mut self, slot: usize, sound: Sound, looping: bool) -> Result<(), PlayError> {
        self.check(slot, Some(&sound))?;
        self.send(ToAudio::Play {
            slot,
            sound,
            looping,
        })
    }

    /// Swap `sound` in for the sound playing in `slot`: it takes over at
    /// the place the old sound has reached, with a 10 ms cross-fade, and
    /// keeps repeating if the old one did. Use this after an edit that a
    /// [`ParamUpdate`] cannot express. If nothing is playing in the slot,
    /// nothing starts.
    ///
    /// The new sound is first moved to the old sound's place here, on the
    /// control thread; the mixer only covers the last few blocks.
    pub fn replace(&mut self, slot: usize, mut sound: Sound) -> Result<(), PlayError> {
        self.check(slot, Some(&sound))?;
        let place = self.status.position(slot).unwrap_or(0);
        sound.advance_to(place as usize);
        self.send(ToAudio::Replace { slot, sound })
    }

    /// A live change to the voice in `slot`. It slides to the new value
    /// over 10 ms.
    pub fn update(&mut self, slot: usize, update: ParamUpdate) -> Result<(), PlayError> {
        self.check(slot, None)?;
        self.send(ToAudio::Update { slot, update })
    }

    /// Slide the level of the sound in `slot` to `level` (see
    /// [`Sound::level`]). After a live change to a recipe that is
    /// normalized, render it again and pass the new mastering gain here,
    /// so the speakers stay as loud as the file.
    pub fn set_level(&mut self, slot: usize, level: f32) -> Result<(), PlayError> {
        self.check(slot, None)?;
        self.send(ToAudio::Level { slot, level })
    }

    /// Repeat the sound in `slot`, or let it play to its end.
    pub fn set_looping(&mut self, slot: usize, looping: bool) -> Result<(), PlayError> {
        self.check(slot, None)?;
        self.send(ToAudio::Looping { slot, looping })
    }

    /// Stop the sound in `slot`. It fades out over 10 ms.
    pub fn stop(&mut self, slot: usize) -> Result<(), PlayError> {
        self.check(slot, None)?;
        self.send(ToAudio::Stop { slot })
    }

    /// Stop every slot.
    pub fn stop_all(&mut self) -> Result<(), PlayError> {
        (0..SLOTS).try_for_each(|slot| self.stop(slot))
    }

    /// Drop the sounds the mixer has finished with, and return how many
    /// there were. Call this often (every frame of the window): the mixer
    /// never frees a sound itself, it hands each one back here.
    pub fn collect(&mut self) -> usize {
        let mut count = 0;
        while let Ok(ToControl::Retired(source)) = self.from_audio.pop() {
            drop(source);
            count += 1;
        }
        count
    }
}
