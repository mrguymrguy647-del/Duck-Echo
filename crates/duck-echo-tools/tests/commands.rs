//! Every command that changes files, undone and redone, gives back the
//! exact text of every file in the project.

use duck_echo_tools::commands::{self, Command};
use duck_echo_tools::history::Who;
use duck_echo_tools::project::Project;
use duck_echo_tools::root::Root;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn project_folder(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("duck-echo-commands-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every file in the project and its text, leaving out Duck Echo's own
/// `.duck-echo` folder.
fn snapshot(dir: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == ".duck-echo" {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.insert(rel, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn command(name: &str, args: Value) -> Command {
    commands::parse(name, &args, "").unwrap_or_else(|e| panic!("{name}: {:?}", e.into_report()))
}

#[test]
fn every_command_undone_and_redone_gives_back_the_exact_files() {
    let dir = project_folder("all");
    std::fs::create_dir_all(dir.join("music")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs/four_bars.toml"),
        dir.join("music/tune.toml"),
    )
    .unwrap();
    let root = Root::new(&dir).unwrap();
    let project = Project::new(&root);

    let script: Vec<(&str, Value)> = vec![
        ("file_new_sound", json!({"file": "sfx/beep.toml"})),
        ("file_new_sound", json!({"file": "sfx/beep.json"})),
        ("file_new_song", json!({"file": "music/new.toml"})),
        (
            "file_copy_from",
            json!({"from": "preset:coin", "to": "sfx/coin.toml"}),
        ),
        (
            "file_copy_from",
            json!({"from": "instrument:piano", "to": "sfx/piano.json"}),
        ),
        (
            "file_copy_from",
            json!({"from": "sfx/coin.toml", "to": "sfx/coin2.toml"}),
        ),
        (
            "file_rename",
            json!({"file": "sfx/coin2.toml", "to": "sfx/other/coin3.toml"}),
        ),
        (
            "file_duplicate",
            json!({"file": "sfx/beep.toml", "to": "sfx/beep2.toml"}),
        ),
        ("file_delete", json!({"file": "sfx/beep2.toml"})),
        ("file_restore", json!({"file": "sfx/beep2.toml"})),
        (
            "file_set_text",
            json!({"file": "sfx/beep2.toml", "text": "# Mine.\nversion = 0\nname = \"beep2\"\n\n[[layers]]\nsource = \"square\"\n"}),
        ),
        (
            "sound_set",
            json!({"file": "sfx/coin.toml", "path": "layers[0].volume", "value": 0.4}),
        ),
        (
            "sound_set",
            json!({"file": "sfx/beep.json", "path": "master.normalize_lufs", "value": -18}),
        ),
        (
            "sound_remove",
            json!({"file": "sfx/beep.json", "path": "master"}),
        ),
        ("sound_add_layer", json!({"file": "sfx/beep.toml"})),
        (
            "sound_add_layer",
            json!({"file": "sfx/beep.toml", "layer": {"source": "noise", "volume": 0.2}, "at": 0}),
        ),
        (
            "sound_move_layer",
            json!({"file": "sfx/beep.toml", "from": 0, "to": 2}),
        ),
        (
            "sound_remove_layer",
            json!({"file": "sfx/beep.toml", "index": 1}),
        ),
        (
            "sound_add_effect",
            json!({"file": "sfx/beep.toml", "effect": {"type": "reverb", "mix": 0.3}}),
        ),
        (
            "sound_add_effect",
            json!({"file": "sfx/beep.toml", "layer": 0, "effect": {"type": "bitcrush", "bits": 6}}),
        ),
        (
            "sound_remove_effect",
            json!({"file": "sfx/beep.toml", "layer": 0, "index": 0}),
        ),
        (
            "sound_remove_effect",
            json!({"file": "sfx/beep.toml", "index": 0}),
        ),
        (
            "song_set",
            json!({"file": "music/tune.toml", "path": "tempo", "value": 100}),
        ),
        (
            "song_remove",
            json!({"file": "music/tune.toml", "path": "tracks[1].pan"}),
        ),
        (
            "song_add_track",
            json!({"file": "music/tune.toml", "name": "keys", "instrument": "piano"}),
        ),
        (
            "song_add_note",
            json!({"file": "music/tune.toml", "track": "keys", "bar": 2, "beat": 1, "note": ["C4", "E4"], "length": 2}),
        ),
        (
            "song_move_note",
            json!({"file": "music/tune.toml", "track": "lead", "index": 0, "beat": 0.5, "note": "D5"}),
        ),
        (
            "song_resize_note",
            json!({"file": "music/tune.toml", "track": "lead", "index": 1, "length": 0.5}),
        ),
        (
            "song_remove_note",
            json!({"file": "music/tune.toml", "track": "lead", "index": 2}),
        ),
        (
            "song_place_pattern",
            json!({"file": "music/tune.toml", "track": "keys", "pattern": "walk", "bars": [3], "transpose": 12}),
        ),
        (
            "song_move_pattern",
            json!({"file": "music/tune.toml", "track": "bass", "index": 1, "bars": [3, 4]}),
        ),
        (
            "song_remove_pattern",
            json!({"file": "music/tune.toml", "track": "bass", "index": 0}),
        ),
        (
            "song_set_loop",
            json!({"file": "music/tune.toml", "loop": true, "bars": 4}),
        ),
        (
            "song_remove_track",
            json!({"file": "music/tune.toml", "track": "keys"}),
        ),
    ];

    // Every command that changes files is in the script.
    for entry in commands::TABLE.iter().filter(|e| e.changes_files) {
        assert!(
            script.iter().any(|(name, _)| *name == entry.name),
            "{} is not tested",
            entry.name
        );
    }

    let mut snapshots = vec![snapshot(&dir)];
    for (name, args) in &script {
        let answer = commands::run(&project, command(name, args.clone()), Who::Person, None, "")
            .unwrap_or_else(|e| panic!("{name} {args}: {:?}", e.into_report()));
        assert_eq!(answer["ok"], true, "{name}");
        let now = snapshot(&dir);
        assert_ne!(&now, snapshots.last().unwrap(), "{name} changed nothing");
        snapshots.push(now);
    }
    // The comments of the preset copy and of the song are still there.
    let coin = std::fs::read_to_string(dir.join("sfx/coin.toml")).unwrap();
    assert!(coin.contains("name = \"coin\""), "{coin}");
    assert!(
        coin.contains("description = \"Copy of the coin preset.\""),
        "{coin}"
    );
    let tune = std::fs::read_to_string(dir.join("music/tune.toml")).unwrap();
    assert!(tune.contains("loop = true"), "{tune}");

    let history = commands::run(
        &project,
        command("edit_history", json!({"limit": 100})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    assert_eq!(history["steps"].as_array().unwrap().len(), script.len());

    // Undo everything, checking each step on the way back.
    for i in (0..script.len()).rev() {
        commands::run(
            &project,
            Command::EditUndo(Default::default()),
            Who::Agent,
            None,
            "",
        )
        .unwrap_or_else(|e| panic!("undo {}: {:?}", script[i].0, e.into_report()));
        assert_eq!(snapshot(&dir), snapshots[i], "undo of {}", script[i].0);
    }
    assert!(commands::run(
        &project,
        Command::EditUndo(Default::default()),
        Who::Agent,
        None,
        ""
    )
    .is_err());

    // And redo it all.
    for (i, (name, _)) in script.iter().enumerate() {
        commands::run(
            &project,
            Command::EditRedo(Default::default()),
            Who::Person,
            None,
            "",
        )
        .unwrap_or_else(|e| panic!("redo {name}: {:?}", e.into_report()));
        assert_eq!(snapshot(&dir), snapshots[i + 1], "redo of {name}");
    }
    assert!(commands::run(
        &project,
        Command::EditRedo(Default::default()),
        Who::Person,
        None,
        ""
    )
    .is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_command_that_would_break_the_file_changes_nothing() {
    let dir = project_folder("broken");
    let root = Root::new(&dir).unwrap();
    let project = Project::new(&root);
    commands::run(
        &project,
        command("file_new_sound", json!({"file": "a.toml"})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    let before = snapshot(&dir);
    let err = commands::run(
        &project,
        command(
            "sound_set",
            json!({"file": "a.toml", "path": "layers[0].volume", "value": 7}),
        ),
        Who::Agent,
        None,
        "arguments.",
    )
    .unwrap_err()
    .into_report();
    assert_eq!(err.errors[0].path, "layers[0].volume", "{err:?}");
    assert_eq!(snapshot(&dir), before);
    // A sound command on a song is refused with the argument's path.
    commands::run(
        &project,
        command("file_new_song", json!({"file": "s.toml"})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    let err = commands::run(
        &project,
        command("sound_add_layer", json!({"file": "s.toml"})),
        Who::Agent,
        None,
        "arguments.",
    )
    .unwrap_err()
    .into_report();
    assert_eq!(err.errors[0].path, "arguments.file", "{err:?}");
    // Making a file that exists is refused.
    let err = commands::run(
        &project,
        command("file_new_sound", json!({"file": "a.toml"})),
        Who::Person,
        None,
        "",
    )
    .unwrap_err()
    .into_report();
    assert!(err.errors[0].message.contains("already exists"), "{err:?}");
    // The project's own folder cannot be touched.
    let err = commands::run(
        &project,
        command(
            "file_new_sound",
            json!({"file": ".duck-echo/history/x.toml"}),
        ),
        Who::Agent,
        None,
        "",
    )
    .unwrap_err()
    .into_report();
    assert!(err.errors[0].message.contains("edit_undo"), "{err:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn slider_strokes_join_one_step() {
    let dir = project_folder("stroke");
    let root = Root::new(&dir).unwrap();
    let project = Project::new(&root);
    commands::run(
        &project,
        command("file_new_sound", json!({"file": "a.toml"})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    let start = snapshot(&dir);
    for v in [0.1, 0.2, 0.3, 0.4] {
        commands::run(
            &project,
            command(
                "sound_set",
                json!({"file": "a.toml", "path": "layers[0].volume", "value": v}),
            ),
            Who::Person,
            Some(7),
            "",
        )
        .unwrap();
    }
    let history = commands::run(
        &project,
        command("edit_history", json!({})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    assert_eq!(history["steps"].as_array().unwrap().len(), 2);
    commands::run(
        &project,
        command("edit_undo", json!({})),
        Who::Person,
        None,
        "",
    )
    .unwrap();
    assert_eq!(snapshot(&dir), start);
    std::fs::remove_dir_all(&dir).ok();
}
