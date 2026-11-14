//! Sound out loud: the step 3 mixer, connected to the sound card.
//!
//! [`duck_echo_live`] has no sound card of its own. This file gives it
//! one, through the `tinyaudio` crate: DirectSound on Windows, and ALSA on
//! Linux (linked in the normal way, so the system's ALSA reaches PipeWire
//! and PulseAudio too). `tinyaudio` calls the mixer from its own thread
//! whenever the sound card wants the next block, and the mixer never
//! allocates, frees or waits there (proven by the tests in
//! `crates/duck-echo-live`).
//!
//! The speakers are opened the first time something plays, not when the
//! Studio starts. So a computer with no sound card still opens the Studio,
//! and says "No speakers were found" only when Play is pressed.

use std::time::{Duration, Instant};

use duck_echo_live::{Config, Player, Sound, live};

/// The slot of the mixer the Studio plays its preview in. Playing another
/// sound there fades the last one out.
pub const PREVIEW: usize = 0;

/// Frames in each block the sound card asks for.
///
/// On Windows, DirectSound plays one buffer while the other is filled; its
/// "fill now" signal can itself come 10 ms or so late, and a busy computer
/// later still, so Duck Forge found 1024 (about 23 ms) too small there and
/// uses 2048 (about 46 ms). Linux uses 1024.
pub fn block_frames() -> usize {
    if cfg!(windows) { 2048 } else { 1024 }
}

/// An open sound card with the mixer playing into it.
pub struct Speaker {
    player: Player,
    sample_rate: u32,
    // Dropping it stops the sound card's thread (and drops the mixer
    // there, after the last block).
    _device: tinyaudio::OutputDevice,
}

impl Speaker {
    /// Open the sound card at `sample_rate`, in stereo.
    pub fn open(sample_rate: u32) -> Result<Speaker, String> {
        let (player, mut mixer) = live(Config {
            sample_rate,
            channels: 2,
            ..Config::default()
        });
        let params = tinyaudio::OutputDeviceParameters {
            sample_rate: sample_rate as usize,
            channels_count: 2,
            channel_sample_count: block_frames(),
        };
        let device = tinyaudio::run_output_device(params, move |block: &mut [f32]| {
            mixer.process(block);
        })
        .map_err(|error| error.to_string())?;
        Ok(Speaker {
            player,
            sample_rate,
            _device: device,
        })
    }

    /// The control thread's handle on the mixer.
    pub fn player(&mut self) -> &mut Player {
        &mut self.player
    }

    /// The sound card's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// The Studio's speakers: opened when first needed, opened again at
/// another sample rate when a sound needs it (the mixer does not
/// resample), and a plain sentence when there are none.
#[derive(Default)]
pub struct Speakers {
    speaker: Option<Speaker>,
    problem: Option<String>,
}

impl Speakers {
    /// Play `sound` in the preview slot, opening the speakers if needed.
    pub fn play(&mut self, sound: Sound, looping: bool) -> Result<(), String> {
        let rate = sound.sample_rate();
        if self
            .speaker
            .as_ref()
            .is_none_or(|s| s.sample_rate() != rate)
        {
            // The old speaker (if any) stops before the new one opens.
            self.speaker = None;
            match Speaker::open(rate) {
                Ok(speaker) => {
                    self.speaker = Some(speaker);
                    self.problem = None;
                }
                Err(why) => {
                    let said = format!(
                        "No speakers were found, so nothing can play ({why}). Check that \
                         your speakers or headphones are plugged in and switched on, then \
                         press Play again."
                    );
                    self.problem = Some(said.clone());
                    return Err(said);
                }
            }
        }
        let speaker = self.speaker.as_mut().expect("opened above");
        speaker
            .player()
            .play(PREVIEW, sound, looping)
            .map_err(|error| error.to_string())
    }

    /// Fade out whatever plays.
    pub fn stop(&mut self) {
        if let Some(speaker) = &mut self.speaker {
            let _ = speaker.player().stop(PREVIEW);
        }
    }

    /// Drop the sounds the mixer has finished with. Call every frame.
    pub fn collect(&mut self) {
        if let Some(speaker) = &mut self.speaker {
            speaker.player().collect();
        }
    }

    /// True while something plays in the preview slot.
    pub fn is_playing(&self) -> bool {
        self.speaker
            .as_ref()
            .is_some_and(|s| s.player.status().is_playing(PREVIEW))
    }

    /// How many sounds have started in the preview slot.
    pub fn starts(&self) -> u64 {
        self.speaker
            .as_ref()
            .map_or(0, |s| s.player.status().starts(PREVIEW))
    }

    /// Frames the sound card has asked for since the speakers opened.
    pub fn frames_played(&self) -> u64 {
        self.speaker
            .as_ref()
            .map_or(0, |s| s.player.status().frames())
    }

    /// Why nothing can play, if the speakers could not be opened.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

/// What [`play_to_the_end`] heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Played {
    /// How long the sound is.
    pub seconds: f64,
    /// Frames the sound card asked for while it played.
    pub frames: u64,
}

/// Play `sound` through the speakers and wait until it has finished (for
/// `DuckEchoStudio --play`, which CI uses to play into ALSA's "null"
/// device). Gives up after `timeout`.
pub fn play_to_the_end(sound: Sound, timeout: Duration) -> Result<Played, String> {
    let seconds = sound.length() as f64 / sound.sample_rate() as f64;
    let mut speakers = Speakers::default();
    // The null device of a test machine does not keep time: a short sound
    // can start and end between two looks. So wait for the count of
    // started sounds to go up, and then for the slot to be silent.
    let before = speakers.starts();
    speakers.play(sound, false)?;
    let started = Instant::now();
    loop {
        speakers.collect();
        if speakers.starts() > before && !speakers.is_playing() {
            break;
        }
        if started.elapsed() > timeout {
            return Err(format!(
                "the sound did not finish within {} seconds; the sound card asked for {} frames",
                timeout.as_secs(),
                speakers.frames_played()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let frames = speakers.frames_played();
    Ok(Played { seconds, frames })
}
