//! Duck Echo live: play sounds through the speakers while they are edited.
//!
//! This crate has no window and no sound card. It is the part between
//! them, in two halves joined by two one-way queues:
//!
//! ```text
//!  control thread (the window)               audio thread (the sound card)
//!  ---------------------------               -----------------------------
//!  Player                                     Mixer
//!    play / replace / update / stop  ------>    process(block), every ~23 ms:
//!    (sounds are made here, where                 take messages (never waits)
//!     allocating is fine)                         mix the sounds in the slots
//!                                                 slide changes, fade, clip
//!    collect()  <-------------------------      send finished sounds back
//!    (finished sounds are dropped here)
//! ```
//!
//! * [`live`] makes the pair. The [`Player`] stays with the window; the
//!   [`Mixer`] is moved into the sound card's callback, which calls
//!   [`Mixer::process`].
//! * A [`Sound`] is made from a recipe ([`Sound::from_recipe`]) or from
//!   finished samples ([`Sound::from_samples`]). A recipe plays exactly
//!   what `render` writes to a file.
//! * A slider drag is a [`ParamUpdate`]: the value slides to its target in
//!   10 ms. Any other edit is a new `Sound` given to [`Player::replace`],
//!   which takes over at the same place with a 10 ms cross-fade.
//! * [`SongPlayback`] plays a song from a render made on a background
//!   thread, and swaps in a new render after each edit.
//!
//! The rule of the audio thread, kept by [`Mixer::process`] and everything
//! under it: **no allocation, no freeing, no lock.** `tests/realtime.rs`
//! proves the first two with `assert_no_alloc`.

mod mixer;
mod player;
mod song;
mod source;
mod status;

pub use duck_echo_synth::live::{ParamUpdate, SMOOTH_SECONDS};
pub use mixer::{Mixer, FADE_SECONDS, SLOTS};
pub use player::{live, Config, PlayError, Player};
pub use song::{SongPlayback, SongState};
pub use source::{Sound, Source};
pub use status::Status;
