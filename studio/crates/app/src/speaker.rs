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

use duck_echo_core::{Arrangement, Song};
use duck_echo_live::{Config, Player, SongPlayback, SongState, Sound, live};

/// The slot of the mixer the Studio plays its preview in. Playing another
/// sound there fades the last one out.
pub const PREVIEW: usize = 0;

/// The slot a song plays in, from its background render.
pub const SONG: usize = 1;

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
    /// The song playing, rendered in the background.
    song: Option<SongPlayback>,
}

impl Speakers {
    /// Open the speakers at `rate` if they are not open at it already.
    fn open_at(&mut self, rate: u32) -> Result<(), String> {
        if self
            .speaker
            .as_ref()
            .is_some_and(|s| s.sample_rate() == rate)
        {
            return Ok(());
        }
        // The old speaker (if any) stops before the new one opens.
        self.song = None;
        self.speaker = None;
        match Speaker::open(rate) {
            Ok(speaker) => {
                self.speaker = Some(speaker);
                self.problem = None;
                Ok(())
            }
            Err(why) => {
                let said = format!(
                    "No speakers were found, so nothing can play ({why}). Check that your \
                     speakers or headphones are plugged in and switched on, then press Play \
                     again."
                );
                self.problem = Some(said.clone());
                Err(said)
            }
        }
    }

    /// Play `sound` in the preview slot, opening the speakers if needed.
    /// A song that plays stops.
    pub fn play(&mut self, sound: Sound, looping: bool) -> Result<(), String> {
        self.stop_song();
        self.open_at(sound.sample_rate())?;
        let speaker = self.speaker.as_mut().expect("opened above");
        speaker
            .player()
            .play(PREVIEW, sound, looping)
            .map_err(|error| error.to_string())
    }

    /// Play a song: it is rendered in the background (the same render as
    /// its file) and starts when that is ready. A sound that plays stops.
    pub fn play_song(&mut self, song: Song, arrangement: Arrangement) -> Result<(), String> {
        self.open_at(song.sample_rate)?;
        let speaker = self.speaker.as_mut().expect("opened above");
        let _ = speaker.player().stop(PREVIEW);
        let playback = self.song.get_or_insert_with(|| SongPlayback::new(SONG));
        playback.play(song, arrangement);
        Ok(())
    }

    fn stop_song(&mut self) {
        if let (Some(playback), Some(speaker)) = (&mut self.song, &mut self.speaker) {
            let _ = playback.stop(speaker.player());
        }
    }

    /// Fade out whatever plays.
    pub fn stop(&mut self) {
        self.stop_song();
        if let Some(speaker) = &mut self.speaker {
            let _ = speaker.player().stop(PREVIEW);
        }
    }

    /// Hand a song's render to the mixer when it is ready, and drop the
    /// sounds the mixer has finished with. Call every frame.
    pub fn collect(&mut self) {
        if let Some(speaker) = &mut self.speaker {
            if let Some(playback) = &mut self.song
                && let Err(why) = playback.poll(speaker.player())
                && why != duck_echo_live::PlayError::Busy
            {
                self.problem = Some(why.to_string());
            }
            speaker.player().collect();
        }
    }

    /// True while something plays (or a song is getting ready).
    pub fn is_playing(&self) -> bool {
        let song = self
            .song
            .as_ref()
            .is_some_and(|p| p.state() != SongState::Stopped);
        song || self
            .speaker
            .as_ref()
            .is_some_and(|s| s.player.status().is_playing(PREVIEW))
    }

    /// True while a song's render is being made.
    pub fn song_getting_ready(&self) -> bool {
        self.song
            .as_ref()
            .is_some_and(|p| p.state() == SongState::GettingReady)
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

/// Play a song through the speakers (rendered in the background first, as
/// Home does) and wait until it has finished, or for a loop, until it has
/// gone round once. For `DuckEchoStudio --play` with a song.
pub fn play_song_to_the_end(
    song: Song,
    arrangement: Arrangement,
    timeout: Duration,
) -> Result<Played, String> {
    let mut speakers = Speakers::default();
    speakers.play_song(song, arrangement)?;
    let started = Instant::now();
    loop {
        speakers.collect();
        if let Some(problem) = speakers.problem() {
            return Err(problem.to_string());
        }
        let (began, went_round) = speakers.speaker.as_ref().map_or((false, false), |s| {
            let status = s.player.status();
            (status.starts(SONG) > 0, status.loops(SONG) > 0)
        });
        if began && !speakers.is_playing() {
            break;
        }
        // A loop never ends by itself: once round is enough.
        if went_round {
            speakers.stop();
            break;
        }
        if started.elapsed() > timeout {
            return Err(format!(
                "the song did not finish within {} seconds; the sound card asked for {} frames",
                timeout.as_secs(),
                speakers.frames_played()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let frames = speakers.frames_played();
    let rate = speakers
        .speaker
        .as_ref()
        .map_or(44_100, |s| s.sample_rate());
    Ok(Played {
        seconds: frames as f64 / rate as f64,
        frames,
    })
}
