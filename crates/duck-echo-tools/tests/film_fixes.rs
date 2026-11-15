//! What making the sound for "Pick a Door: Blocks" (a 58.6 s horror film
//! made with Duck Forge) taught us, item by item.

use duck_echo_core::{load_str, Format};

fn recipe(text: &str) -> duck_echo_core::Recipe {
    load_str(text, Format::Toml)
        .unwrap_or_else(|r| panic!("{r}"))
        .recipe
}

/// Item 1: a layer longer than 30 s used to be cut at 30 s without a word.
#[test]
fn a_45_second_layer_plays_for_45_seconds() {
    let r = recipe(
        "version = 0\nsample_rate = 8000\n[[layers]]\nsource = \"noise\"\nvolume = 0.3\nenvelope = { attack = 2, hold = 25, decay = 18 }\n",
    );
    let buffer = duck_echo_tools::render_buffer(&r).unwrap();
    let seconds = buffer.duration_seconds();
    assert!((seconds - 45.0).abs() < 0.01, "{seconds} s");
    // It fades out by itself, so the end is quiet, not cut.
    let tail = &buffer.samples[buffer.samples.len() - 80..];
    assert!(tail.iter().all(|s| s.abs() < 0.01), "{tail:?}");
}
