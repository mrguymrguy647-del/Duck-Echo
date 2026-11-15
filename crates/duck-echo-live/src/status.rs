//! What the audio thread tells the control thread, as plain numbers that
//! either thread can read or write without a lock (atomics).

use crate::mixer::SLOTS;
use std::sync::atomic::{
    AtomicBool, AtomicU64,
    Ordering::{Relaxed, SeqCst},
};

/// Where one slot is.
#[derive(Debug, Default)]
pub(crate) struct SlotStatus {
    /// True while a sound is playing in the slot.
    pub(crate) playing: AtomicBool,
    /// The next frame of that sound.
    pub(crate) position: AtomicU64,
    /// How many times it has repeated.
    pub(crate) loops: AtomicU64,
    /// How many sounds have started in this slot since the mixer was
    /// made. With `playing`, it tells "has ended" from "has not begun".
    pub(crate) starts: AtomicU64,
}

/// The mixer's state, shared by both threads. The mixer writes it at the
/// end of every block; the [`crate::Player`] reads it.
#[derive(Debug)]
pub struct Status {
    pub(crate) slots: [SlotStatus; SLOTS],
    /// Frames mixed so far.
    pub(crate) frames: AtomicU64,
    /// Samples that were clipped to -1..=1.
    pub(crate) clipped: AtomicU64,
    /// Messages the mixer could not carry out (no such slot, another
    /// sample rate, a live change the sound could not take).
    pub(crate) refused: AtomicU64,
}

impl Status {
    pub(crate) fn new() -> Self {
        Status {
            slots: std::array::from_fn(|_| SlotStatus::default()),
            frames: AtomicU64::new(0),
            clipped: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        }
    }

    /// True while a sound is playing in `slot`.
    pub fn is_playing(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|s| s.playing.load(SeqCst))
    }

    /// The frame the sound in `slot` has reached (the play head), or
    /// `None` when the slot is silent.
    pub fn position(&self, slot: usize) -> Option<u64> {
        let s = self.slots.get(slot)?;
        s.playing.load(SeqCst).then(|| s.position.load(Relaxed))
    }

    /// How many sounds have started playing in `slot` since the mixer was
    /// made. A sound that starts and ends between two looks at
    /// [`Status::is_playing`] still counts here.
    pub fn starts(&self, slot: usize) -> u64 {
        self.slots.get(slot).map_or(0, |s| s.starts.load(SeqCst))
    }

    /// How many times the sound in `slot` has repeated.
    pub fn loops(&self, slot: usize) -> u64 {
        self.slots.get(slot).map_or(0, |s| s.loops.load(Relaxed))
    }

    /// Frames mixed since the mixer was made.
    pub fn frames(&self) -> u64 {
        self.frames.load(Relaxed)
    }

    /// Samples that were louder than the speakers can go and were clipped.
    pub fn clipped_samples(&self) -> u64 {
        self.clipped.load(Relaxed)
    }

    /// Messages the mixer could not carry out.
    pub fn refused(&self) -> u64 {
        self.refused.load(Relaxed)
    }
}
