//! The song player: turns a checked song into samples.
//!
//! How the sound flows, for every block of a few hundred samples:
//!
//! ```text
//! note voices -> summed -> instrument effects -> instrument level
//!                                                    |
//!                              track volume and pan  v
//! all tracks summed -> song effects -> song master -> file
//! ```
//!
//! * Each note is one [`Voice`], made by [`crate::prepare_note`] and
//!   started on its exact sample.
//! * An instrument's own `effects` run **once**, on the sum of all its
//!   notes, not once per note. So a reverb is one room, echoes carry on
//!   over the next notes, and a fast melody does not run dozens of
//!   reverbs. (A drum kit has one such group per piece.)
//! * An instrument is levelled once ([`instrument_level`]), not per note,
//!   so `velocity` still means something.
//!
//! Seeds: every track gets a stream picked by its **name**, and every note
//! a stream picked by its **start sample**. Adding or moving one thing
//! never changes the random parts of another.
//!
//! The player works block by block like [`Voice`] does, which is the shape
//! live playback needs. It is not allocation-free yet: a new [`Voice`] is
//! made when each note starts. `Voice::process` itself still never
//! allocates.

use crate::effects::EffectState;
use crate::math::Rng;
use crate::{master, prepare_note, Mastering, NotePlay, Voice, RENDER_BLOCK};
use duck_echo_audio::AudioBuffer;
use duck_echo_core::song::home_hz;
use duck_echo_core::validate::chain_tail_seconds;
use duck_echo_core::{Arrangement, Instrument, Recipe, Song, Sound};

/// Stream tag for tracks. Changing it changes every song render.
const STREAM_TRACK: u64 = 0x5452_4143;

/// FNV-1a, a tiny fixed hash. It turns a track name into a number the
/// same way on every computer, so a track's seed depends on its name and
/// not on where it sits in the file.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The one fixed gain that makes an instrument as loud in a song as it is
/// when its recipe is rendered alone: the recipe's `master.gain` times
/// whatever its `normalize` or `normalize_lufs` would apply.
///
/// Normalizing every note on its own would make all notes equally loud
/// and undo `velocity`, so the gain is worked out once per instrument.
pub fn instrument_level(recipe: &Recipe, sample_rate: u32, channels: u16) -> f32 {
    if !recipe.master.normalize && recipe.master.normalize_lufs.is_none() {
        return recipe.master.gain;
    }
    let mut alone = recipe.clone();
    alone.sample_rate = sample_rate;
    alone.channels = channels;
    match crate::render_mastered(&alone) {
        Ok((_, mastering)) => recipe.master.gain * mastering.gain,
        Err(_) => recipe.master.gain,
    }
}

/// One note, planned.
#[derive(Debug, Clone)]
struct PlannedNote {
    /// The sample frame it starts on.
    start: usize,
    play: NotePlay,
}

/// One instrument recipe on a track, with its notes. A plain instrument is
/// one group; a drum kit is one group per piece.
#[derive(Debug, Clone)]
struct Group {
    recipe: Recipe,
    /// Notes in start order.
    notes: Vec<PlannedNote>,
    /// The next note to start.
    next: usize,
    /// Notes sounding now.
    active: Vec<Voice>,
    /// The recipe's master effects, one copy per channel.
    effects: Vec<Vec<EffectState>>,
    /// The frame after which this group is silent.
    end: usize,
}

#[derive(Debug, Clone)]
struct TrackPlayer {
    /// Position in the song's `tracks`.
    index: usize,
    groups: Vec<Group>,
    /// Volume and pan as one gain per channel.
    gains: [f32; 2],
}

/// A prepared, playable song.
#[derive(Debug, Clone)]
pub struct SongPlayer {
    sample_rate: u32,
    channels: usize,
    total_frames: usize,
    /// Frames in the song's bars, before any tail.
    song_frames: usize,
    position: usize,
    gain: f32,
    tracks: Vec<TrackPlayer>,
    /// Song-wide effects, one copy per channel.
    effects: Vec<Vec<EffectState>>,
    /// Block buffers, one `Vec` per channel: the whole mix, one track, one
    /// group.
    mix: Vec<Vec<f32>>,
    track: Vec<Vec<f32>>,
    group: Vec<Vec<f32>>,
    /// One voice's interleaved output for the current block.
    scratch: Vec<f32>,
}

/// Track gains from volume and pan. A centred track passes through at
/// `volume`. Panning uses the equal-power law of layers, scaled so the
/// centre is exactly 1: the far side fades out and the near side rises by
/// up to 3 dB, which keeps a mono sound equally loud across the stereo
/// field.
fn track_gains(volume: f32, pan: f32, channels: usize) -> [f32; 2] {
    if channels == 1 || pan == 0.0 {
        return [volume, volume];
    }
    let [l, r] = crate::pan_gains(pan);
    [
        volume * l * std::f32::consts::SQRT_2,
        volume * r * std::f32::consts::SQRT_2,
    ]
}

/// Plan a checked song. `arrangement` must come from
/// [`duck_echo_core::arrange`] for this same `song`.
pub fn prepare_song(song: &Song, arrangement: &Arrangement) -> SongPlayer {
    let sr = song.sample_rate as f32;
    let channels = song.channels.clamp(1, 2) as usize;
    let root = Rng::new(song.seed);
    let seconds_per_beat = song.seconds_per_beat();
    let song_frames = song
        .frame_at(arrangement.bars as f64 * song.beats_per_bar as f64)
        .max(1);

    let mut tracks = Vec::with_capacity(arrangement.tracks.len());
    let mut last_end = 0usize;
    for (index, plan) in arrangement.tracks.iter().enumerate() {
        let written = &song.tracks[index];
        let track_rng = root.fork(STREAM_TRACK ^ fnv1a64(plan.name.as_bytes()));

        // The recipes this track plays: one, or one per kit piece (in
        // name order, which a `BTreeMap` always gives).
        let recipes: Vec<(Option<&str>, &Recipe)> = match &plan.instrument {
            Instrument::Sound(recipe) => vec![(None, recipe.as_ref())],
            Instrument::Kit(pieces) => pieces.iter().map(|(n, r)| (Some(n.as_str()), r)).collect(),
        };
        let mut groups: Vec<Group> = recipes
            .iter()
            .map(|(_, recipe)| Group {
                recipe: (*recipe).clone(),
                notes: Vec::new(),
                next: 0,
                active: Vec::new(),
                effects: (0..channels)
                    .map(|_| {
                        recipe
                            .effects
                            .iter()
                            .map(|e| EffectState::new(e, sr))
                            .collect()
                    })
                    .collect(),
                end: 0,
            })
            .collect();
        let levels: Vec<f32> = recipes
            .iter()
            .map(|(_, r)| instrument_level(r, song.sample_rate, song.channels))
            .collect();
        let homes: Vec<Option<f32>> = recipes.iter().map(|(_, r)| home_hz(r)).collect();

        // Notes in a fixed order: by start sample, then low to high, then
        // as written. The place of a note among those that start together
        // is part of its seed.
        let mut order: Vec<(usize, f32, usize)> = plan
            .events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let hz = match &e.sound {
                    Sound::Pitch { hz, .. } => *hz,
                    Sound::Piece(_) => 0.0,
                };
                (song.frame_at(e.beat), hz, i)
            })
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));

        let mut same_start = 0u64;
        let mut previous_start = usize::MAX;
        for (start, _, i) in order {
            same_start = if start == previous_start {
                same_start + 1
            } else {
                0
            };
            previous_start = start;
            let event = &plan.events[i];
            let (g, ratio, gate) = match &event.sound {
                Sound::Pitch { hz, .. } => (
                    0,
                    homes[0].map_or(1.0, |home| hz / home),
                    Some((event.length * seconds_per_beat) as f32),
                ),
                Sound::Piece(name) => (
                    recipes
                        .iter()
                        .position(|(n, _)| *n == Some(name.as_str()))
                        .unwrap_or(0),
                    1.0,
                    None,
                ),
            };
            let seed = track_rng
                .fork(start as u64)
                .fork(same_start)
                .fork(groups[g].recipe.seed)
                .next_u64();
            let play = NotePlay {
                ratio,
                gate,
                gain: event.velocity * levels[g],
                seed,
                sample_rate: song.sample_rate,
                channels,
            };
            // Make the voice once now, only to learn how long it rings.
            let frames = prepare_note(&groups[g].recipe, &play).total_frames();
            let group = &mut groups[g];
            group.end = group.end.max(start + frames);
            group.notes.push(PlannedNote { start, play });
        }
        for group in &mut groups {
            if !group.notes.is_empty() {
                let tail = chain_tail_seconds(&group.recipe.effects);
                group.end += (tail * sr).round() as usize;
            }
            last_end = last_end.max(group.end);
        }
        tracks.push(TrackPlayer {
            index,
            groups,
            gains: track_gains(written.volume, written.pan, channels),
        });
    }

    let song_tail = (chain_tail_seconds(&song.effects) * sr).round() as usize;
    let total_frames = song_frames.max(last_end + song_tail);
    let block = || (0..channels).map(|_| vec![0.0; RENDER_BLOCK]).collect();
    SongPlayer {
        sample_rate: song.sample_rate,
        channels,
        total_frames,
        song_frames,
        position: 0,
        gain: song.master.gain,
        tracks,
        effects: (0..channels)
            .map(|_| {
                song.effects
                    .iter()
                    .map(|e| EffectState::new(e, sr))
                    .collect()
            })
            .collect(),
        mix: block(),
        track: block(),
        group: block(),
        scratch: vec![0.0; RENDER_BLOCK * channels],
    }
}

impl SongPlayer {
    /// Output sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Output channels (1 or 2).
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Total length in frames: the song's bars plus whatever rings on.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Frames in the song's bars alone, to the sample.
    pub fn song_frames(&self) -> usize {
        self.song_frames
    }

    /// True once every frame has been produced.
    pub fn finished(&self) -> bool {
        self.position >= self.total_frames
    }

    /// Silence every track whose index (its position in the song's
    /// `tracks`) is not in `heard`. The tracks kept sound exactly as they
    /// do in the full song, because a track's seed depends only on its
    /// name. Call before the first [`Self::process`].
    pub fn keep_tracks(&mut self, heard: &[usize]) {
        self.tracks.retain(|t| heard.contains(&t.index));
    }

    /// Fill `out` with the next frames, interleaved when stereo, and
    /// return how many frames were written. Writes zeros past the end.
    /// Output is not clipped; the caller clips or normalizes.
    pub fn process(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        for chunk in out.chunks_mut(RENDER_BLOCK * self.channels) {
            written += self.process_block(chunk);
        }
        written
    }

    fn process_block(&mut self, out: &mut [f32]) -> usize {
        out.iter_mut().for_each(|s| *s = 0.0);
        if self.finished() {
            return 0;
        }
        let ch = self.channels;
        let n = (out.len() / ch).min(self.total_frames - self.position);
        let block_start = self.position;
        let block_end = block_start + n;
        for buf in &mut self.mix {
            buf[..n].iter_mut().for_each(|s| *s = 0.0);
        }

        for track in &mut self.tracks {
            for buf in &mut self.track {
                buf[..n].iter_mut().for_each(|s| *s = 0.0);
            }
            for group in &mut track.groups {
                if block_start >= group.end {
                    continue;
                }
                for buf in &mut self.group {
                    buf[..n].iter_mut().for_each(|s| *s = 0.0);
                }
                // Notes already sounding fill the whole block.
                for voice in &mut group.active {
                    add_voice(voice, 0, n, &mut self.scratch, &mut self.group);
                }
                // Notes that start inside this block begin part way in.
                while group.next < group.notes.len() && group.notes[group.next].start < block_end {
                    let note = &group.notes[group.next];
                    let offset = note.start.saturating_sub(block_start);
                    let mut voice = prepare_note(&group.recipe, &note.play);
                    add_voice(&mut voice, offset, n, &mut self.scratch, &mut self.group);
                    group.active.push(voice);
                    group.next += 1;
                }
                group.active.retain(|v| !v.finished());
                // The instrument's effects, once for all its notes.
                for (buf, chain) in self.group.iter_mut().zip(group.effects.iter_mut()) {
                    for fx in chain {
                        fx.process(&mut buf[..n]);
                    }
                }
                for (t, g) in self.track.iter_mut().zip(self.group.iter()) {
                    for (t, g) in t[..n].iter_mut().zip(g[..n].iter()) {
                        *t += *g;
                    }
                }
            }
            for (c, (m, t)) in self.mix.iter_mut().zip(self.track.iter()).enumerate() {
                let gain = track.gains[c];
                for (m, t) in m[..n].iter_mut().zip(t[..n].iter()) {
                    *m += *t * gain;
                }
            }
        }

        for (buf, chain) in self.mix.iter_mut().zip(self.effects.iter_mut()) {
            for fx in chain {
                fx.process(&mut buf[..n]);
            }
        }
        for (i, frame) in out[..n * ch].chunks_exact_mut(ch).enumerate() {
            for (c, s) in frame.iter_mut().enumerate() {
                *s = self.mix[c][i] * self.gain;
            }
        }
        self.position += n;
        n
    }
}

/// Run `voice` for frames `offset..n` of the block and add it into the
/// per-channel `bus`.
fn add_voice(
    voice: &mut Voice,
    offset: usize,
    n: usize,
    scratch: &mut [f32],
    bus: &mut [Vec<f32>],
) {
    let ch = bus.len();
    let frames = n - offset;
    let out = &mut scratch[..frames * ch];
    voice.process(out);
    for (c, buf) in bus.iter_mut().enumerate() {
        for (b, frame) in buf[offset..n].iter_mut().zip(out.chunks_exact(ch)) {
            *b += frame[c];
        }
    }
}

/// Run a player to the end, unclipped.
fn mix(mut player: SongPlayer) -> Vec<f32> {
    let channels = player.channels();
    let mut samples = vec![0.0f32; player.total_frames() * channels];
    let mut pos = 0;
    while pos < samples.len() {
        let end = (pos + RENDER_BLOCK * channels).min(samples.len());
        let n = player.process(&mut samples[pos..end]);
        if n == 0 {
            break;
        }
        pos += n * channels;
    }
    samples
}

/// Render a checked song offline: mix, then the song's `master` steps
/// (normalize, clip, trim). Also returns what the master steps did, for
/// [`render_song_tracks`].
pub fn render_song(song: &Song, arrangement: &Arrangement) -> (AudioBuffer, Mastering) {
    let player = prepare_song(song, arrangement);
    let channels = player.channels();
    let (samples, mastering) = master(
        &song.master,
        song.sample_rate,
        song.master.trim_silence,
        mix(player),
        channels,
    );
    (
        AudioBuffer {
            sample_rate: song.sample_rate,
            channels: channels as u16,
            samples,
        },
        mastering,
    )
}

/// Render only the tracks whose indices are in `heard`, finished with the
/// full song's `mastering` (from [`render_song`]): the same gain, then
/// clipping, then cut or padded to the same length.
///
/// The result is a **stem**: what those tracks add to the mix, lined up
/// with it in time and level. The stems of all tracks added together give
/// the full mix (to within float rounding), as long as nothing clips and
/// the song's own effects are linear (echo and reverb are; distortion and
/// bitcrush are not).
pub fn render_song_tracks(
    song: &Song,
    arrangement: &Arrangement,
    heard: &[usize],
    mastering: &Mastering,
) -> AudioBuffer {
    let mut player = prepare_song(song, arrangement);
    player.keep_tracks(heard);
    let channels = player.channels();
    let mut samples = mix(player);
    for s in &mut samples {
        *s = (*s * mastering.gain).clamp(-1.0, 1.0);
    }
    samples.resize(mastering.frames * channels, 0.0);
    AudioBuffer {
        sample_rate: song.sample_rate,
        channels: channels as u16,
        samples,
    }
}
