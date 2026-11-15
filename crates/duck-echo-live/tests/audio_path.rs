//! A tripwire for the other half of the audio thread's rule: no lock, no
//! waiting, no file or screen output.
//!
//! Nothing can count locks the way `assert_no_alloc` counts allocations,
//! so this reads the source of every file the audio thread runs and fails
//! if one of them names a lock or anything that can wait. It cannot prove
//! the code is right; it stops such a thing being added without anyone
//! noticing. (The stall test in `realtime.rs` is the running proof: the
//! mixer keeps making blocks while the control thread does nothing.)

use std::path::Path;

/// Every file with code that runs inside `Mixer::process`.
const AUDIO_PATH: &[&str] = &[
    "crates/duck-echo-live/src/mixer.rs",
    "crates/duck-echo-live/src/source.rs",
    "crates/duck-echo-live/src/status.rs",
    "crates/duck-echo-synth/src/lib.rs",
    "crates/duck-echo-synth/src/live.rs",
    "crates/duck-echo-synth/src/dsp.rs",
    "crates/duck-echo-synth/src/effects.rs",
    "crates/duck-echo-synth/src/math.rs",
];

/// Things that lock, wait, or talk to the system.
const FORBIDDEN: &[&str] = &[
    "Mutex",
    "RwLock",
    "Condvar",
    "Barrier",
    "OnceLock",
    "LazyLock",
    "mpsc",
    ".lock()",
    "std::thread",
    "thread::sleep",
    "park()",
    "std::fs",
    "std::io",
    "std::net",
    "std::process",
    "println!",
    "eprintln!",
    "print!",
    "dbg!",
    "Instant::",
    "SystemTime",
];

#[test]
fn nothing_on_the_audio_path_names_a_lock_or_a_wait() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in AUDIO_PATH {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for (number, line) in text.lines().enumerate() {
            // Comments may say "no lock"; only code counts. Tests at the
            // end of a file are not on the audio path.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.trim() == "mod tests {" {
                break;
            }
            for word in FORBIDDEN {
                assert!(
                    !line.contains(word),
                    "{file}:{}: `{word}` on the audio path: {line}",
                    number + 1
                );
            }
        }
    }
}

#[test]
fn the_two_queues_are_the_lock_free_ring_buffer() {
    // The mixer's only ways in and out are rtrb's ring buffer ends and
    // atomics. If this ever changes, the reviewer must look again.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mixer = std::fs::read_to_string(root.join("crates/duck-echo-live/src/mixer.rs")).unwrap();
    assert!(mixer.contains("from_control: Consumer<ToAudio>"));
    assert!(mixer.contains("to_control: Producer<ToControl>"));
    assert!(mixer.contains("use rtrb::{Consumer, Producer, PushError};"));
    assert!(mixer.contains("status: Arc<Status>"));
    let status = std::fs::read_to_string(root.join("crates/duck-echo-live/src/status.rs")).unwrap();
    for line in status
        .lines()
        .filter(|l| l.contains("pub(crate)") && l.contains(": "))
    {
        assert!(
            line.contains("Atomic") || line.contains("[SlotStatus; SLOTS]"),
            "a field of the shared status is not an atomic: {line}"
        );
    }
}
