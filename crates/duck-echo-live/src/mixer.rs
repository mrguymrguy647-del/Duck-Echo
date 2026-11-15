//! The audio thread's half: the mixer.
//!
//! [`Mixer::process`] is called by the sound card's callback, about every
//! 23 ms, to fill the next block of samples. It must never be late, so
//! inside it there is:
//!
//! * **no allocation and no freeing.** Every buffer is made in
//!   [`Mixer::new`]. A sound that is finished is not dropped here: it is
//!   sent back to the control thread, which drops it. (Freeing memory can
//!   take a lock inside the system allocator.)
//! * **no lock and no waiting.** Messages come and go through two
//!   lock-free ring buffers. If the ring back to the control thread is
//!   full, finished sounds wait in a parking area made in advance; if that
//!   is nearly full too, the mixer stops taking new messages until the
//!   control thread catches up. It never blocks and never drops a sound.
//!
//! The tests in `tests/realtime.rs` run this under `assert_no_alloc`,
//! which fails on any allocation or free.

use crate::source::{Sound, Source};
use crate::status::Status;
use duck_echo_synth::live::ParamUpdate;
use rtrb::{Consumer, Producer, PushError};
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::sync::Arc;

/// How many sounds can play at once: the voice pool. Each slot holds one
/// sound (and, for 10 ms at a time, the sound it is taking over from).
pub const SLOTS: usize = 32;

/// How long a fade takes: a stopped sound fades out and a replacement
/// fades in over this, 10 ms. The same time as a live change takes to
/// arrive ([`duck_echo_synth::live::SMOOTH_SECONDS`]).
pub const FADE_SECONDS: f32 = 0.01;

/// Messages each ring buffer can hold.
pub(crate) const QUEUE: usize = 256;

/// Frames of scratch memory: sounds are made in pieces of at most this.
const SCRATCH_FRAMES: usize = 1024;

/// The most frames a replacement sound is moved forward on the audio
/// thread to line up with the sound it replaces (about 0.1 s). The control
/// thread has already moved it to where the old sound was a moment ago;
/// this covers only the blocks that played since. It is capped so one
/// callback never does unbounded work.
const MAX_CATCH_UP: usize = 4096;

/// Finished sounds that can wait here when the ring to the control thread
/// is full. Every sound the mixer holds has a place: each slot holds at
/// most three (playing, fading out, waiting), plus a little room.
const PARKING: usize = SLOTS * 3 + 8;

/// A message from the control thread.
#[derive(Debug)]
pub(crate) enum ToAudio {
    /// Start `sound` in `slot`. What was playing there fades out.
    Play {
        slot: usize,
        sound: Sound,
        looping: bool,
    },
    /// Take over from the sound in `slot` at the place it has reached,
    /// with a cross-fade. Does nothing if the slot is silent.
    Replace { slot: usize, sound: Sound },
    /// A live change to the voice in `slot`.
    Update { slot: usize, update: ParamUpdate },
    /// Slide the level of the sound in `slot` to `level`.
    Level { slot: usize, level: f32 },
    /// Repeat the sound in `slot`, or let it end.
    Looping { slot: usize, looping: bool },
    /// Fade out the sound in `slot` and stop it.
    Stop { slot: usize },
}

/// A message to the control thread.
#[derive(Debug)]
pub(crate) enum ToControl {
    /// A sound the mixer is done with. The control thread drops it.
    Retired(Source),
}

/// A number that slides to a target in equal steps, one per frame.
#[derive(Debug, Clone, Copy)]
struct Slide {
    value: f32,
    target: f32,
    step: f32,
    left: u32,
}

impl Slide {
    fn at(value: f32) -> Self {
        Slide {
            value,
            target: value,
            step: 0.0,
            left: 0,
        }
    }

    /// Slide to `target` over `frames` frames (at once when 0).
    fn to(&mut self, target: f32, frames: u32) {
        self.target = target;
        if frames == 0 {
            self.value = target;
            self.left = 0;
        } else {
            self.step = (target - self.value) / frames as f32;
            self.left = frames;
        }
    }

    /// The value for the next frame. The last step lands exactly on the
    /// target.
    #[inline]
    fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            self.value = if self.left == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }
}

/// A sound in a slot.
#[derive(Debug)]
struct Playing {
    source: Source,
    /// Frames to play before it ends (or repeats).
    length: usize,
    looping: bool,
    /// How many times it has repeated.
    loops: u64,
    /// The gain it plays at (see [`Sound`]).
    level: Slide,
    /// 1 when fully heard; slides to 0 when fading out, from 0 when
    /// fading in.
    fade: Slide,
    fading_out: bool,
}

impl Playing {
    fn new(sound: Sound, looping: bool) -> Self {
        Playing {
            source: sound.source,
            length: sound.length,
            looping,
            loops: 0,
            level: Slide::at(sound.level),
            fade: Slide::at(1.0),
            fading_out: false,
        }
    }

    /// True once a fade-out has reached silence.
    fn faded_out(&self) -> bool {
        self.fading_out && self.fade.left == 0
    }

    /// True once there is nothing more to hear from it: it faded out, or
    /// it reached its end and does not repeat.
    fn done(&self) -> bool {
        self.faded_out() || (!self.looping && self.source.position() >= self.length)
    }
}

/// What waits for the slot's fading-out seat to be free. Only the newest
/// request is kept: a sound that was never heard is simply retired.
#[derive(Debug)]
enum Waiting {
    Play(Playing),
    Replace(Playing),
    Stop,
}

#[derive(Debug, Default)]
struct Slot {
    /// The sound being heard.
    current: Option<Playing>,
    /// The sound it took over from, fading out.
    outgoing: Option<Playing>,
    /// A request that arrived while `outgoing` was still fading.
    waiting: Option<Waiting>,
}

/// The mixer. Lives on the audio thread; talk to it through its
/// [`crate::Player`].
pub struct Mixer {
    sample_rate: u32,
    channels: usize,
    fade_frames: u32,
    slots: [Slot; SLOTS],
    /// One piece of one sound, before it is added to the mix.
    scratch: Vec<f32>,
    /// Finished sounds waiting for room in `to_control`.
    parked: Vec<Source>,
    /// Sounds held in slots (playing, fading out or waiting).
    held: usize,
    from_control: Consumer<ToAudio>,
    to_control: Producer<ToControl>,
    status: Arc<Status>,
}

impl Mixer {
    /// A mixer that fills blocks of `channels` (1 or 2) channels at
    /// `sample_rate`. Everything it will ever need is allocated here.
    pub(crate) fn new(
        sample_rate: u32,
        channels: usize,
        fade_seconds: f32,
        from_control: Consumer<ToAudio>,
        to_control: Producer<ToControl>,
        status: Arc<Status>,
    ) -> Self {
        Mixer {
            sample_rate,
            channels: channels.clamp(1, 2),
            fade_frames: (sample_rate as f32 * fade_seconds.max(0.0)).round() as u32,
            slots: std::array::from_fn(|_| Slot::default()),
            scratch: vec![0.0; SCRATCH_FRAMES * 2],
            parked: Vec::with_capacity(PARKING),
            held: 0,
            from_control,
            to_control,
            status,
        }
    }

    /// Samples per second of the blocks it fills.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channels of the blocks it fills (1 or 2).
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Fill `out` (interleaved, any whole number of frames) with the next
    /// block: every playing sound mixed together and clipped to -1..=1.
    ///
    /// This is the audio callback. It allocates nothing, frees nothing and
    /// takes no lock.
    pub fn process(&mut self, out: &mut [f32]) {
        self.send_parked();
        self.take_messages();
        out.fill(0.0);
        let frames = out.len() / self.channels;
        let out = &mut out[..frames * self.channels];
        for i in 0..SLOTS {
            self.mix_slot(i, out);
        }
        // The speakers get nothing outside -1..=1, and nothing that is not
        // a number. This is the same clip a rendered file gets.
        let mut clipped = 0u64;
        for s in out.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
                clipped += 1;
            } else if *s > 1.0 || *s < -1.0 {
                *s = s.clamp(-1.0, 1.0);
                clipped += 1;
            }
        }
        if clipped > 0 {
            self.status.clipped.fetch_add(clipped, Relaxed);
        }
        self.status.frames.fetch_add(frames as u64, Relaxed);
        self.after_block();
    }

    /// Hand `source` back to the control thread to be dropped there. If
    /// the ring is full it waits in the parking area. It is never dropped
    /// here.
    fn retire(&mut self, source: Source) {
        match self.to_control.push(ToControl::Retired(source)) {
            Ok(()) => {}
            // `take_messages` keeps a parking place free for every sound
            // the mixer holds, so this push never grows the Vec.
            Err(PushError::Full(ToControl::Retired(source))) => self.parked.push(source),
        }
    }

    /// Send parked sounds back, as many as the ring takes.
    fn send_parked(&mut self) {
        while let Some(source) = self.parked.pop() {
            if let Err(PushError::Full(ToControl::Retired(source))) =
                self.to_control.push(ToControl::Retired(source))
            {
                self.parked.push(source);
                break;
            }
        }
    }

    /// Take the control thread's messages. A message can bring one more
    /// sound, so one is taken only while a parking place is free for every
    /// sound held; otherwise it stays in the ring until the control thread
    /// has collected some.
    fn take_messages(&mut self) {
        while self.held + self.parked.len() < PARKING {
            let Ok(message) = self.from_control.pop() else {
                break;
            };
            self.handle(message);
        }
    }

    fn refuse(&mut self, sound: Option<Sound>) {
        self.status.refused.fetch_add(1, Relaxed);
        if let Some(sound) = sound {
            self.retire(sound.source);
        }
    }

    fn handle(&mut self, message: ToAudio) {
        match message {
            ToAudio::Play {
                slot,
                sound,
                looping,
            } => {
                if !self.fits(slot, &sound) {
                    return self.refuse(Some(sound));
                }
                self.held += 1;
                self.request(slot, Waiting::Play(Playing::new(sound, looping)));
            }
            ToAudio::Replace { slot, sound } => {
                if !self.fits(slot, &sound) {
                    return self.refuse(Some(sound));
                }
                self.held += 1;
                self.request(slot, Waiting::Replace(Playing::new(sound, false)));
            }
            ToAudio::Stop { slot } => {
                if slot >= SLOTS {
                    return self.refuse(None);
                }
                self.request(slot, Waiting::Stop);
            }
            ToAudio::Update { slot, update } => {
                let Some(s) = self.slots.get_mut(slot) else {
                    return self.refuse(None);
                };
                // The sound being heard, and one that is about to take
                // over from it: both must end up with the change.
                let mut taken = false;
                if let Some(p) = &mut s.current {
                    taken |= p.source.update(update);
                }
                if let Some(Waiting::Play(p) | Waiting::Replace(p)) = &mut s.waiting {
                    taken |= p.source.update(update);
                }
                if !taken {
                    self.refuse(None);
                }
            }
            ToAudio::Level { slot, level } => {
                let frames = self.fade_frames;
                let Some(s) = self.slots.get_mut(slot) else {
                    return self.refuse(None);
                };
                if let Some(p) = &mut s.current {
                    p.level.to(level, frames);
                }
                if let Some(Waiting::Play(p) | Waiting::Replace(p)) = &mut s.waiting {
                    p.level = Slide::at(level);
                }
            }
            ToAudio::Looping { slot, looping } => {
                let Some(s) = self.slots.get_mut(slot) else {
                    return self.refuse(None);
                };
                if let Some(p) = &mut s.current {
                    p.looping = looping;
                }
                if let Some(Waiting::Play(p)) = &mut s.waiting {
                    p.looping = looping;
                }
            }
        }
    }

    /// True when `sound` can play in `slot`: the slot exists, and the
    /// sound has the mixer's sample rate (the mixer does not resample).
    fn fits(&self, slot: usize, sound: &Sound) -> bool {
        slot < SLOTS && sound.sample_rate() == self.sample_rate
    }

    /// Start, replace or stop the sound in `slot`. If the slot's last
    /// sound is still fading out, the request waits for it (at most 10 ms
    /// and one block), so two requests in a row can never cut a fade
    /// short, which would click.
    fn request(&mut self, slot: usize, request: Waiting) {
        if self.slots[slot].outgoing.is_some() {
            let before = self.slots[slot].waiting.replace(request);
            // An older request that never got its turn: its sound was
            // never heard.
            if let Some(Waiting::Play(p) | Waiting::Replace(p)) = before {
                self.held -= 1;
                self.retire(p.source);
            }
        } else {
            self.apply(slot, request);
        }
    }

    /// Carry out a request now. The slot's fading-out seat is free.
    fn apply(&mut self, slot: usize, request: Waiting) {
        let fade_frames = self.fade_frames;
        let old = self.slots[slot].current.take();
        let new = match request {
            Waiting::Stop => None,
            Waiting::Play(new) => Some(new),
            Waiting::Replace(mut new) => match &old {
                // Nothing is playing, so nothing starts: an edit made
                // while the sound is stopped stays silent.
                None => {
                    self.held -= 1;
                    self.retire(new.source);
                    None
                }
                Some(old) => {
                    // Take over at the place the old sound has reached.
                    new.looping = old.looping;
                    new.loops = old.loops;
                    let behind = frames_behind(
                        old.source.position(),
                        new.source.position(),
                        old.looping,
                        old.length,
                    );
                    catch_up(&mut new, behind, &mut self.scratch);
                    if fade_frames > 0 {
                        new.fade = Slide::at(0.0);
                        new.fade.to(1.0, fade_frames);
                    }
                    Some(new)
                }
            },
        };
        if let Some(mut old) = old {
            if fade_frames == 0 {
                self.held -= 1;
                self.retire(old.source);
            } else {
                old.fade.to(0.0, fade_frames);
                old.fading_out = true;
                self.slots[slot].outgoing = Some(old);
            }
        }
        if new.is_some() {
            // In this order, so a reader that sees the new start also
            // sees the slot as playing.
            let status = &self.status.slots[slot];
            status.playing.store(true, SeqCst);
            status.starts.fetch_add(1, SeqCst);
        }
        self.slots[slot].current = new;
    }

    /// Add the sounds of one slot to `out`.
    fn mix_slot(&mut self, slot: usize, out: &mut [f32]) {
        let channels = self.channels;
        let scratch = &mut self.scratch;
        let s = &mut self.slots[slot];
        if let Some(p) = &mut s.outgoing {
            mix_one(p, out, channels, scratch);
        }
        if let Some(p) = &mut s.current {
            mix_one(p, out, channels, scratch);
        }
    }

    /// After a block: retire what has ended, give waiting requests their
    /// turn, and tell the control thread where each slot is.
    fn after_block(&mut self) {
        for i in 0..SLOTS {
            if self.slots[i].outgoing.as_ref().is_some_and(Playing::done) {
                if let Some(p) = self.slots[i].outgoing.take() {
                    self.held -= 1;
                    self.retire(p.source);
                }
            }
            if self.slots[i].current.as_ref().is_some_and(Playing::done) {
                if let Some(p) = self.slots[i].current.take() {
                    self.held -= 1;
                    self.retire(p.source);
                }
            }
            if self.slots[i].outgoing.is_none() {
                if let Some(request) = self.slots[i].waiting.take() {
                    self.apply(i, request);
                }
            }
            let status = &self.status.slots[i];
            match &self.slots[i].current {
                Some(p) => {
                    status.position.store(p.source.position() as u64, Relaxed);
                    status.loops.store(p.loops, Relaxed);
                    status.playing.store(true, SeqCst);
                }
                None => status.playing.store(false, SeqCst),
            }
        }
    }
}

/// How many frames `new` is behind the sound it replaces. When the old
/// sound repeats and is at an earlier place than `new`, it has gone round
/// since the control thread looked, so `new` goes round too.
fn frames_behind(old: usize, new: usize, looping: bool, length: usize) -> usize {
    if old >= new {
        old - new
    } else if looping {
        old + length.saturating_sub(new)
    } else {
        0
    }
}

/// Move `p` on by `frames` (at most [`MAX_CATCH_UP`]) without a sound,
/// going round when it repeats.
fn catch_up(p: &mut Playing, frames: usize, scratch: &mut [f32]) {
    let mut frames = frames.min(MAX_CATCH_UP);
    while frames > 0 {
        let left = p.length.saturating_sub(p.source.position());
        if left == 0 {
            if !p.looping {
                break;
            }
            p.source.rewind();
            continue;
        }
        let n = frames.min(left);
        let before = p.source.position();
        p.source.skip(n, scratch);
        if p.source.position() == before {
            break;
        }
        frames -= n;
    }
}

/// Add `p`'s next frames to `out` (interleaved, `out_channels` channels),
/// each frame at its level and fade. Repeats the sound when it loops.
fn mix_one(p: &mut Playing, out: &mut [f32], out_channels: usize, scratch: &mut [f32]) {
    let channels = p.source.channels();
    let total = out.len() / out_channels;
    let mut done = 0;
    while done < total {
        if p.faded_out() {
            return;
        }
        let left = p.length.saturating_sub(p.source.position());
        if left == 0 {
            if !p.looping {
                return;
            }
            p.source.rewind();
            p.loops += 1;
            continue;
        }
        let want = (total - done).min(left).min(SCRATCH_FRAMES);
        let piece = &mut scratch[..want * channels];
        let got = p.source.process(piece);
        if got == 0 {
            // The source ended before `length`: treat it as the end.
            p.length = p.source.position();
            if p.length == 0 {
                return;
            }
            continue;
        }
        let target = &mut out[done * out_channels..(done + got) * out_channels];
        for (frame, to) in piece[..got * channels]
            .chunks_exact(channels)
            .zip(target.chunks_exact_mut(out_channels))
        {
            let gain = p.level.next() * p.fade.next();
            match (channels, out_channels) {
                (1, 1) => to[0] += frame[0] * gain,
                // A mono sound plays from both speakers.
                (1, _) => {
                    let s = frame[0] * gain;
                    to[0] += s;
                    to[1] += s;
                }
                // A stereo sound on one speaker: the two sides, halved.
                (_, 1) => to[0] += (frame[0] + frame[1]) * 0.5 * gain,
                _ => {
                    to[0] += frame[0] * gain;
                    to[1] += frame[1] * gain;
                }
            }
        }
        done += got;
    }
}
