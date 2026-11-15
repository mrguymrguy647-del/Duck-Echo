//! Playing a song from a render made in the background.
//!
//! A song's notes each make a new voice, which allocates, so a song cannot
//! be made note by note on the audio thread yet. Instead the whole song is
//! rendered on a worker thread (the same render that writes the file) and
//! played as finished samples. After an edit it is rendered again, and the
//! new render takes over at the play head with a 10 ms cross-fade, so the
//! change is heard without stopping.

use crate::player::{PlayError, Player};
use crate::source::Sound;
use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Arrangement, Song};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::mpsc::{channel, Receiver, Sender};

/// What a [`SongPlayback`] is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SongState {
    /// Nothing is playing.
    Stopped,
    /// Play was pressed and the first render is being made ("Getting the
    /// song ready...").
    GettingReady,
    /// The song is playing.
    Playing,
}

struct Request {
    number: u64,
    song: Song,
    arrangement: Arrangement,
    /// Only these tracks, when some are muted or one is solo.
    heard: Option<Vec<usize>>,
}

struct Rendered {
    number: u64,
    buffer: AudioBuffer,
}

/// Plays one song in one slot of a [`Player`], rendering in the
/// background. Lives on the control thread.
pub struct SongPlayback {
    slot: usize,
    requests: Sender<Request>,
    results: Receiver<Rendered>,
    /// The number of the newest render asked for.
    newest: u64,
    /// The first render number that belongs to the current Play. Results
    /// with a lower number were asked for before it and are thrown away.
    first_of_this_play: u64,
    /// True once the current Play has started in the mixer.
    started: bool,
    /// The slot's count of started sounds when ours was sent, so "it
    /// ended" is not mistaken for "it has not begun".
    starts_before: u64,
    looping: bool,
    state: SongState,
    /// A finished render that has not been handed to the player yet, and
    /// its number.
    ready: Option<(u64, Sound)>,
    /// The number of the newest render handed to the player.
    handed: u64,
    /// Repeat (or not) whatever the song's `loop` says. `None`: follow it.
    looping_override: Option<bool>,
    /// Play only these tracks. `None`: all of them.
    heard: Option<Vec<usize>>,
}

impl SongPlayback {
    /// A song player for `slot`. Starts the worker thread, which sleeps
    /// until there is a song to render.
    pub fn new(slot: usize) -> Self {
        let (requests, todo) = channel::<Request>();
        let (done, results) = channel::<Rendered>();
        // The worker ends by itself when this SongPlayback is dropped
        // (its queue closes).
        std::thread::Builder::new()
            .name("duck-echo-song-render".into())
            .spawn(move || {
                while let Ok(mut request) = todo.recv() {
                    // Only the newest edit matters.
                    while let Ok(newer) = todo.try_recv() {
                        request = newer;
                    }
                    let (full, mastering) =
                        duck_echo_synth::song::render_song(&request.song, &request.arrangement);
                    // Muted or solo tracks: the stem of the tracks heard,
                    // at the full mix's level, as `render --solo` writes.
                    let buffer = match &request.heard {
                        Some(heard) => duck_echo_synth::song::render_song_tracks(
                            &request.song,
                            &request.arrangement,
                            heard,
                            &mastering,
                        ),
                        None => full,
                    };
                    let rendered = Rendered {
                        number: request.number,
                        buffer,
                    };
                    if done.send(rendered).is_err() {
                        break;
                    }
                }
            })
            .expect("start the song render thread");
        SongPlayback {
            slot,
            requests,
            results,
            newest: 0,
            first_of_this_play: 0,
            started: false,
            starts_before: 0,
            looping: false,
            state: SongState::Stopped,
            ready: None,
            handed: 0,
            looping_override: None,
            heard: None,
        }
    }

    /// Repeat the song (`Some(true)`), play it once (`Some(false)`), or
    /// follow its `loop` (`None`). Takes effect at the next [`Self::play`]
    /// or [`Self::changed`].
    pub fn set_looping(&mut self, looping: Option<bool>) {
        self.looping_override = looping;
    }

    /// Play only the tracks numbered in `heard` (others are muted), or
    /// every track (`None`). They play at the level they have in the full
    /// mix, as `render --solo` writes them. Takes effect at the next
    /// [`Self::play`] or [`Self::changed`].
    pub fn set_heard(&mut self, heard: Option<Vec<usize>>) {
        self.heard = heard;
    }

    /// What it is doing, as of the last [`Self::poll`].
    pub fn state(&self) -> SongState {
        self.state
    }

    /// True while a render is being made that has not reached the
    /// speakers yet: the first one after Play, or a new one after an
    /// edit. False when stopped.
    pub fn is_rendering(&self) -> bool {
        self.state != SongState::Stopped && self.handed < self.newest
    }

    fn ask(&mut self, song: Song, arrangement: Arrangement) {
        self.newest += 1;
        // If the worker is gone the song simply never gets ready; `poll`
        // has nothing to report then.
        let _ = self.requests.send(Request {
            number: self.newest,
            song,
            arrangement,
            heard: self.heard.clone(),
        });
    }

    /// Play the song from its start. The render begins now; the sound
    /// begins when it is ready (see [`Self::poll`]). A song with
    /// `loop = true` repeats.
    pub fn play(&mut self, song: Song, arrangement: Arrangement) {
        self.looping = self.looping_override.unwrap_or(song.loop_);
        self.started = false;
        self.state = SongState::GettingReady;
        self.ask(song, arrangement);
        self.first_of_this_play = self.newest;
    }

    /// The song was edited. If it is playing (or getting ready), render
    /// it again; the new render takes over at the play head. If it is
    /// stopped, nothing happens.
    pub fn changed(&mut self, song: Song, arrangement: Arrangement) {
        if self.state == SongState::Stopped {
            return;
        }
        self.looping = self.looping_override.unwrap_or(song.loop_);
        self.ask(song, arrangement);
    }

    /// Stop the song (it fades out over 10 ms).
    pub fn stop(&mut self, player: &mut Player) -> Result<(), PlayError> {
        self.state = SongState::Stopped;
        self.started = false;
        self.ready = None;
        player.stop(self.slot)
    }

    /// Hand finished renders to the player, and notice when the song has
    /// played to its end. Call this often (every frame of the window).
    pub fn poll(&mut self, player: &mut Player) -> Result<SongState, PlayError> {
        // Keep only the newest finished render.
        while let Ok(rendered) = self.results.try_recv() {
            let wanted =
                self.state != SongState::Stopped && rendered.number >= self.first_of_this_play;
            if wanted {
                self.ready = Some((rendered.number, Sound::from_samples(rendered.buffer)));
            }
        }
        if self.ready.is_some() {
            // Two messages may be needed. If the mixer's queue has no room
            // the render is kept, and handed over on a later poll.
            if player.room() < 2 {
                return Err(PlayError::Busy);
            }
            if let Some((number, sound)) = self.ready.take() {
                let sent = if self.started {
                    player
                        .set_looping(self.slot, self.looping)
                        .and_then(|()| player.replace(self.slot, sound))
                } else {
                    self.starts_before = player.status().slots[self.slot].starts.load(SeqCst);
                    player.play(self.slot, sound, self.looping)
                };
                if let Err(e) = sent {
                    // Not a full queue (that was checked): the song cannot
                    // play here at all, for example at this sample rate.
                    self.state = SongState::Stopped;
                    self.started = false;
                    return Err(e);
                }
                self.started = true;
                self.handed = number;
                self.state = SongState::Playing;
            }
        }
        // It has played to its end when the mixer started it and the slot
        // is silent again.
        if self.state == SongState::Playing {
            let slot = &player.status().slots[self.slot];
            let began = slot.starts.load(SeqCst) > self.starts_before;
            if began && !slot.playing.load(SeqCst) {
                self.state = SongState::Stopped;
                self.started = false;
            }
        }
        Ok(self.state)
    }
}
