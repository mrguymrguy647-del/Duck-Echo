//! Roadmap task 10: every example in docs/AGENT_GUIDE.md works exactly
//! as the guide says, so the guide cannot quietly go out of date.

use duck_echo_core::Report;
use duck_echo_tools::edit::{apply_edits, load_value, EditOp};
use duck_echo_tools::measure::{measure_document, Options};
use duck_echo_tools::song::files_next_to;
use duck_echo_tools::{presets, Document};
use std::path::Path;

fn guide() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/AGENT_GUIDE.md"))
        .unwrap()
}

/// The code blocks of one language in the guide, in order.
fn blocks(language: &str) -> Vec<String> {
    let text = guide();
    let fence = format!("```{language}\n");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find(&fence) {
        let body = &rest[start + fence.len()..];
        let end = body.find("```").expect("closed block");
        out.push(body[..end].to_string());
        rest = &body[end..];
    }
    out
}

fn no_problems(report: &Report, what: &str) {
    assert!(report.errors.is_empty(), "{what}: {report}");
    assert!(report.warnings.is_empty(), "{what}: {report}");
}

#[test]
fn every_toml_example_is_valid_with_no_warnings() {
    let examples = blocks("toml");
    assert_eq!(
        examples.len(),
        4,
        "a recipe with ranges, a growl with expect, a recipe and a song"
    );
    let files = files_next_to(Path::new("guide.toml"));
    for (i, src) in examples.iter().enumerate() {
        let dir = std::env::temp_dir().join(format!("duck-echo-guide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("example{i}.toml"));
        std::fs::write(&path, src).unwrap();
        let doc = duck_echo_tools::load_document(&path, &files)
            .unwrap_or_else(|e| panic!("example {i}: {e}"));
        no_problems(doc.report(), &format!("example {i}"));
        let options = Options {
            check_notes: true,
            ..Options::default()
        };
        let m = measure_document(&doc, None, &options, "").unwrap().measured;
        let a = &m.analysis;
        assert!(!a.clipping.clipped, "example {i}");
        // An example that says what it should measure measures it.
        if let Some(expected) = &m.expect {
            assert!(expected.pass, "example {i}: {}", a.summary);
            assert!(a.summary.contains("expect: all 2 met"), "{}", a.summary);
        }
        match &doc {
            Document::Recipe(r) => {
                // The step and the coin: at the level they ask for (their
                // category's target).
                let target = r.loaded.recipe.master.normalize_lufs.unwrap();
                assert!(
                    (a.loudness_lufs.unwrap() - target).abs() < 0.1,
                    "{}",
                    a.summary
                );
            }
            Document::Song(_) => {
                // The song example: a clean loop, every note heard, at
                // the music target, and nothing reported as extra.
                assert!(a.summary.contains("loop joins cleanly"), "{}", a.summary);
                let check = m.note_check.unwrap();
                assert_eq!(check.notes_heard, check.notes_written, "{check:?}");
                assert!(
                    check.tracks.iter().all(|t| t.extra_onsets.is_empty()),
                    "{check:?}"
                );
                assert!((a.loudness_lufs.unwrap() + 18.0).abs() < 0.1);
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn the_levelling_edit_works_on_a_preset_as_the_guide_says() {
    let json_blocks = blocks("json");
    assert_eq!(
        json_blocks.len(),
        3,
        "the levelling edit, the game_send examples and the listen examples"
    );
    // The game_send examples: each line is a valid call.
    for line in json_blocks[1].lines().filter(|l| !l.trim().is_empty()) {
        let args: serde_json::Value = serde_json::from_str(line).unwrap();
        duck_echo_tools::commands::parse("game_send", &args, "")
            .unwrap_or_else(|e| panic!("{line}: {:?}", e.into_report()));
    }
    let ops: Vec<EditOp> = serde_json::from_str(&json_blocks[0]).unwrap();
    let coin = presets::get("coin").unwrap();
    let mut doc = duck_echo_core::tree::parse_toml(coin.source)
        .unwrap()
        .to_json()
        .unwrap();
    apply_edits(&mut doc, &ops).unwrap();
    let loaded = load_value(&doc).unwrap();
    no_problems(&loaded.report, "levelled coin");
    assert_eq!(loaded.recipe.master.normalize_lufs, Some(-19.0));

    // From then on, a change to the sound keeps the level.
    let mut louder = doc.clone();
    let op: Vec<EditOp> =
        serde_json::from_str(r#"[{"path": "layers[0].source", "value": "saw"}]"#).unwrap();
    apply_edits(&mut louder, &op).unwrap();
    for d in [&doc, &louder] {
        let r = load_value(d).unwrap().recipe;
        let buf = duck_echo_synth::render(&r).unwrap();
        let a = duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels);
        assert!(
            (a.loudness_lufs.unwrap() + 19.0).abs() < 0.1,
            "{}",
            a.summary
        );
    }
}

#[test]
fn the_tables_in_the_guide_match_the_built_ins() {
    let text = guide();
    // Every preset and every category target is in the guide.
    for p in presets::list(None).unwrap() {
        assert!(text.contains(&p.name), "preset {} is missing", p.name);
        let target = format!("| {} | {} LUFS |", p.category, p.target_lufs.unwrap());
        assert!(text.contains(&target), "{target}");
    }
    // Every instrument with its range.
    for i in duck_echo_tools::instruments::list().unwrap() {
        if let Some([lo, hi]) = &i.range {
            let row = format!("| `{}` |", i.name);
            let line = text
                .lines()
                .find(|l| l.starts_with(&row))
                .unwrap_or_else(|| panic!("{row}"));
            assert!(line.contains(&format!("{lo} to {hi}")), "{line}");
        }
    }
    for kit in duck_echo_tools::instruments::KITS {
        for (piece, _) in kit.pieces {
            assert!(text.contains(&format!("`{piece}`")), "{piece}");
        }
    }
}

/// Run a named command in `project` with the Duck Forge folder `forge`
/// set, as the MCP server started with --duck-forge does.
fn run_with_forge(
    project: &Path,
    forge: &Path,
    name: &str,
    args: &serde_json::Value,
) -> serde_json::Value {
    let root = duck_echo_tools::root::Root::new(project)
        .unwrap()
        .with_duck_forge(duck_echo_tools::forge::DuckForge::open(forge).unwrap());
    let project = duck_echo_tools::project::Project::new(&root);
    duck_echo_tools::commands::parse(name, args, "arguments.")
        .and_then(|c| {
            duck_echo_tools::commands::run(
                &project,
                c,
                duck_echo_tools::history::Who::Agent,
                None,
                "arguments.",
            )
        })
        .unwrap_or_else(|e| panic!("{name} {args}: {}", e.into_report()))
}

#[test]
fn the_send_to_a_game_chapter_works_as_it_says() {
    // A Duck Forge folder with the guide's game, and a project with the
    // files its examples send.
    let dir = std::env::temp_dir().join(format!("duck-echo-guide-send-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (project, forge) = (dir.join("project"), dir.join("Duck Forge"));
    std::fs::create_dir_all(forge.join("examples/mygame")).unwrap();
    std::fs::write(
        forge.join("Cargo.toml"),
        "[package]\nname = \"duckforge\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(forge.join("examples/mygame/main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(project.join("sfx")).unwrap();
    std::fs::create_dir_all(project.join("music")).unwrap();
    for (file, preset) in [("sfx/thud.toml", "thud"), ("sfx/step.toml", "footstep")] {
        std::fs::write(project.join(file), presets::get(preset).unwrap().source).unwrap();
    }
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs/four_bars.toml"),
        project.join("music/theme.toml"),
    )
    .unwrap();

    // Each game_send line of the chapter writes its files...
    let sends: Vec<serde_json::Value> = blocks("json")[1]
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(sends.len(), 3, "a sound, a song and variants");
    let game = forge.join("examples/mygame");
    for args in &sends {
        let sent = run_with_forge(&project, &forge, "game_send", args);
        assert_eq!(sent["unchanged"], false, "{sent}");
        assert!(
            game.join(sent["facts"].as_str().unwrap()).is_file(),
            "{sent}"
        );
        assert!(
            game.join(sent["recipe"].as_str().unwrap()).is_file(),
            "{sent}"
        );
    }
    // ...the song as OGG...
    assert!(game.join("assets/theme_day.ogg").is_file());
    // ...and every file the chapter's Rust loads is there.
    let rust = &blocks("rust")[0];
    let loaded: Vec<&str> = rust
        .split('"')
        .filter(|part| part.starts_with("examples/mygame/"))
        .collect();
    assert_eq!(loaded.len(), 3, "{rust}");
    for file in loaded {
        assert!(forge.join(file).is_file(), "{file} was not written");
    }
    // Sending again with no change writes nothing.
    for args in &sends {
        let again = run_with_forge(&project, &forge, "game_send", args);
        assert_eq!(again["unchanged"], true, "{again}");
        assert_eq!(again["written"], serde_json::json!([]), "{again}");
    }
    // game_list names what the game has now.
    let games = run_with_forge(&project, &forge, "game_list", &serde_json::json!({}));
    let text = games.to_string();
    for name in ["thud", "theme_day", "step"] {
        assert!(text.contains(&format!("\"{name}\"")), "{name}: {text}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The text of one chapter (from its `## ` heading to the next).
fn chapter(title: &str) -> String {
    let text = guide();
    let start = text
        .find(&format!("## {title}\n"))
        .unwrap_or_else(|| panic!("no chapter {title}"));
    let rest = &text[start + 3..];
    let end = rest.find("\n## ").map(|e| e + 1).unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn the_listening_chapter_says_what_was_measured_and_its_calls_work() {
    use duck_echo_tools::ear::{EarConfig, EarKeeper};
    let text = chapter("Listening with the Ear");
    // What the chapter must say, in its own words.
    for words in [
        "compass, not a judge",
        "only a\nperson installs the Ear",
        "`ear_status`",
        "`compare_with`",
        "`ear: true`",
        "`expect.sounds_like`",
        "Scores only, for now.",
        "Naming short, plain sounds.",
        "Calm against tense music.",
        "Silence",
    ] {
        assert!(text.contains(words), "the chapter does not say: {words}");
    }
    // Its numbers are the measured ones.
    let check = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ear-check.md"),
    )
    .unwrap();
    for (guide_says, check_says) in [
        ("209 times in 214", "209 of 214"),
        (
            "(3 of 3)",
            "| Soft and harsh | soft and harsh versions told apart | 75% | 3 of 3 |",
        ),
        (
            "\"calm music\" above\n  \"fast exciting music\"",
            "gives the tense loop \"calm music\" (0.57) above\n  \"fast exciting music\" (0.56)",
        ),
    ] {
        assert!(text.contains(guide_says), "{guide_says}");
        assert!(check.contains(check_says), "ear-check.md: {check_says}");
    }

    // Its listen calls work as it says, with the stub Ear.
    let stub = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ear/stub/duck_ear_stub.py");
    let program = duck_echo_ear::Program::new(&stub);
    let python = std::process::Command::new(&program.command)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !python {
        assert!(
            std::env::var("DUCK_ECHO_REQUIRE_PYTHON").as_deref() != Ok("1"),
            "no Python, and DUCK_ECHO_REQUIRE_PYTHON=1"
        );
        eprintln!("skipped the listen calls: no Python for the stub Ear");
        return;
    }
    let dir = std::env::temp_dir().join(format!("duck-echo-guide-ear-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sfx")).unwrap();
    std::fs::write(
        dir.join("sfx/jump.toml"),
        presets::get("jump").unwrap().source,
    )
    .unwrap();
    let keeper = EarKeeper::new(EarConfig {
        program: Some(stub),
        home: dir.join("ear-home"),
        ..EarConfig::from_environment(None).unwrap()
    });
    let root = duck_echo_tools::root::Root::new(&dir)
        .unwrap()
        .with_ear(keeper.clone());
    let project = duck_echo_tools::project::Project::new(&root);
    let run = |name: &str, args: &serde_json::Value| {
        duck_echo_tools::commands::parse(name, args, "arguments.")
            .and_then(|c| {
                duck_echo_tools::commands::run(
                    &project,
                    c,
                    duck_echo_tools::history::Who::Agent,
                    None,
                    "arguments.",
                )
            })
            .unwrap_or_else(|e| panic!("{name} {args}: {}", e.into_report()))
    };
    // "Render the old one to a second file first".
    let before = duck_echo_tools::load_audio(&dir.join("sfx/jump.toml")).unwrap();
    let wav = duck_echo_audio::encode_wav(&before, duck_echo_audio::WavFormat::Pcm16).unwrap();
    std::fs::write(dir.join("sfx/jump_before.wav"), wav).unwrap();
    let calls: Vec<serde_json::Value> = blocks("json")[2]
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls.len(), 2, "a listen and a comparison");
    let listened = run("listen", &calls[0]);
    for like in listened["like"].as_array().unwrap() {
        for field in ["match", "verdict", "rank"] {
            assert!(!like[field].is_null(), "{field}: {listened}");
        }
    }
    assert!(listened["summary"].is_string());
    let compared = run("listen", &calls[1]);
    let first = &compared["compared"][0];
    assert!(
        first["more_like"].is_string() && first["by"].is_number(),
        "{compared}"
    );
    // The same question again is answered from the kept answers.
    let again = run("listen", &calls[0]);
    assert_eq!(again["ear"]["cached"], true, "{again}");
    keeper.stop();
    std::fs::remove_dir_all(&dir).ok();
}
