//! A song made in the timeline is the same song as one written by hand:
//! the timeline's edits (the commands of `song_edit`, run as the Studio
//! runs them) give a file that renders the very same bytes as the same
//! song written as text.

use duck_echo_tools::commands;
use duck_echo_tools::history::Who;
use duck_echo_tools::project::Project;
use duck_echo_tools::root::Root;
use studio_app::library;
use studio_app::song_edit::{self, Edit};
use studio_app::song_view;

/// The start: two tracks and a pattern, no notes yet.
const START: &str = r#"# Made in the timeline.
version = 0
tempo = 120
bars = 4

[patterns.beat]
notes = [
  { beat = 0, note = "kick" },
  { beat = 2, note = "snare" },
]

[[tracks]]
name = "lead"
instrument = "chip_lead"

[[tracks]]
name = "drums"
instrument = "drums"
"#;

/// The same song, with what the edits below add, written by hand.
const BY_HAND: &str = r#"version = 0
tempo = 120
bars = 4

[patterns.beat]
notes = [
  { beat = 0, note = "kick" },
  { beat = 2, note = "snare" },
]

[[tracks]]
name = "lead"
instrument = "chip_lead"
volume = 0.6
notes = [
  { bar = 1, beat = 1, note = "G4", length = 2 },
  { bar = 2, beat = 2, note = "E4", length = 1 },
]

[[tracks]]
name = "drums"
instrument = "drums"
play = [
  { pattern = "beat", bars = [1] },
  { pattern = "beat", bars = [3] },
]
"#;

fn run(root: &Root, (name, args): Edit) {
    let command = commands::parse(name, &args, "").unwrap();
    commands::run(&Project::new(root), command, Who::Person, None, "")
        .unwrap_or_else(|e| panic!("{name} {args}: {:?}", e.into_report()));
}

#[test]
fn a_song_made_in_the_timeline_renders_the_same_bytes_as_the_same_song_written_by_hand() {
    let dir = std::env::temp_dir().join(format!("duck-echo-timeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("made.toml"), START).unwrap();
    let root = Root::new(&dir).unwrap();
    let file = "made.toml";
    let now = |root: &Root| {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        library::load_song(root, "", &text).unwrap()
    };

    // Double-click the lead lane at bar 1, beat 1 on E4's row (an empty
    // lane spans F#3 to F#4), twice more at other spots, drag a note,
    // stretch it, delete one.
    let (song, arrangement) = now(&root);
    let lanes = song_view::lanes(&song, &arrangement);
    let lead = &lanes[0];
    let e4 = (0..lead.rows as i32)
        .find(|r| lead.note_at(*r).as_deref() == Some("E4"))
        .unwrap();
    run(
        &root,
        song_edit::add_note(file, &song, lead, 0.0, e4).unwrap(),
    );
    run(
        &root,
        song_edit::add_note(file, &song, lead, 6.0, e4).unwrap(),
    );
    run(
        &root,
        song_edit::add_note(file, &song, lead, 12.0, e4).unwrap(),
    );
    let (song, _) = now(&root);
    // The first note: a beat later and three semitones up (E4 to G4).
    run(
        &root,
        song_edit::move_note(file, &song, 0, 0, 1.0, 3, None).unwrap(),
    );
    let (song, _) = now(&root);
    run(
        &root,
        song_edit::resize_note(file, &song, 0, 0, 2.0).unwrap(),
    );
    let (song, _) = now(&root);
    run(&root, song_edit::remove_note(file, &song, 0, 2).unwrap());

    // Place the beat on the drums at bar 1 and at bar 2, then drag the
    // second clip to bar 3.
    let (song, _) = now(&root);
    run(
        &root,
        song_edit::place_pattern(file, &song, 1, "beat", 1).unwrap(),
    );
    let (song, _) = now(&root);
    run(
        &root,
        song_edit::place_pattern(file, &song, 1, "beat", 2).unwrap(),
    );
    let (song, arrangement) = now(&root);
    let clip = song_view::lanes(&song, &arrangement)[1].clips[1].clone();
    assert_eq!(clip.bar, 2);
    run(
        &root,
        song_edit::move_clip(file, &song, 1, &clip, 3).unwrap(),
    );
    run(&root, song_edit::set_volume(file, 0, 0.6));

    let made = std::fs::read_to_string(dir.join(file)).unwrap();
    // The comment written at the top is still there.
    assert!(made.starts_with("# Made in the timeline."), "{made}");

    let (song, arrangement) = now(&root);
    let (hand, hand_arrangement) = library::load_song(&root, "", BY_HAND).unwrap();
    let from_timeline = duck_echo_synth::song::render_song(&song, &arrangement).0;
    let from_text = duck_echo_synth::song::render_song(&hand, &hand_arrangement).0;
    let wav = |b| duck_echo_audio::encode_wav(b, duck_echo_audio::WavFormat::default()).unwrap();
    assert!(
        wav(&from_timeline) == wav(&from_text),
        "the timeline's song and the hand-written song sound different:\n{made}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
