//! Phase 4, step 2: what agents get from the Duck Ear (docs/phase4-design.md,
//! section 6), with the stub Ear in `ear/stub/` standing in for the real
//! one. The stub needs Python: these tests skip without it, unless
//! `DUCK_ECHO_REQUIRE_PYTHON=1` (as in CI).

use duck_echo_tools::ear::{self, EarConfig, EarKeeper};
use duck_echo_tools::{measure, Options};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A new, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("duck-echo-ear-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stub_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ear/stub/duck_ear_stub.py")
}

/// Is there a Python to run the stub? Without one the test is skipped,
/// unless CI says Python must be there.
fn python_found() -> bool {
    let program = duck_echo_ear::Program::new(stub_file());
    let found = std::process::Command::new(&program.command)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        if std::env::var("DUCK_ECHO_REQUIRE_PYTHON").as_deref() == Ok("1") {
            panic!(
                "no Python at {}, and DUCK_ECHO_REQUIRE_PYTHON=1",
                program.command.display()
            );
        }
        eprintln!("skipped: no Python at {}", program.command.display());
    }
    found
}

/// Settings for the stub Ear (`program`), with the Ear's folder in `home`.
fn config(program: &Path, home: &Path) -> EarConfig {
    EarConfig {
        program: Some(program.to_path_buf()),
        home: home.to_path_buf(),
        times: duck_echo_ear::Times::default(),
        ear_idle: ear::EAR_IDLE,
        describe_idle_seconds: duck_echo_ear::DESCRIBE_IDLE_SECONDS,
    }
}

/// A project folder whose root keeps `keeper`, with a coin and a chime.
fn project(name: &str, keeper: &Arc<EarKeeper>) -> (PathBuf, duck_echo_tools::root::Root) {
    let dir = folder(name);
    std::fs::create_dir_all(dir.join("sfx")).unwrap();
    std::fs::write(
        dir.join("sfx/coin.toml"),
        duck_echo_tools::presets::get("coin").unwrap().source,
    )
    .unwrap();
    std::fs::write(
        dir.join("sfx/chime.toml"),
        duck_echo_tools::presets::get("ui_confirm").unwrap().source,
    )
    .unwrap();
    let root = duck_echo_tools::root::Root::new(&dir)
        .unwrap()
        .with_ear(Arc::clone(keeper));
    (dir, root)
}

/// Run a command as an agent does.
fn run(
    root: &duck_echo_tools::root::Root,
    name: &str,
    args: Value,
) -> Result<Value, duck_echo_core::Report> {
    let project = duck_echo_tools::project::Project::new(root);
    duck_echo_tools::commands::parse(name, &args, "arguments.")
        .and_then(|c| {
            duck_echo_tools::commands::run(
                &project,
                c,
                duck_echo_tools::history::Who::Agent,
                None,
                "arguments.",
            )
        })
        .map_err(|e| e.into_report())
}

/// Measure a recipe or song text, as `analyze` does.
fn measured(text: &str, dir: &Path, name: &str) -> measure::RecipeMeasurement {
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    let doc = duck_echo_tools::load_document(&file, &duck_echo_tools::song::files_next_to(&file))
        .unwrap();
    measure::measure_document(&doc, None, &Options::default(), "").unwrap()
}

/// The coin preset with an `expect` line added.
fn coin_expecting(expect: &str) -> String {
    let source = duck_echo_tools::presets::get("coin").unwrap().source;
    source.replacen(
        "version = 0\n",
        &format!("version = 0\nexpect = {expect}\n"),
        1,
    )
}

#[test]
fn without_the_ear_sounds_like_is_listed_but_never_fails() {
    let dir = folder("unchecked");
    let m = measured(
        &coin_expecting(r#"{ sounds_like = "a coin pickup", duration_seconds = [0.01, 5] }"#),
        &dir,
        "coin.toml",
    )
    .measured;
    let expect = m.expect.as_ref().unwrap();
    assert!(expect.pass);
    let line = expect
        .lines
        .iter()
        .find(|l| l.what == "sounds_like")
        .unwrap();
    assert_eq!(line.pass, None);
    assert_eq!(
        line.note.as_deref(),
        Some(duck_echo_tools::expect::NEEDS_THE_EAR)
    );
    assert!(
        m.analysis
            .summary
            .ends_with("expect: all 1 met (sounds_like not checked)"),
        "{}",
        m.analysis.summary
    );
    assert!(m.ear.is_none());
    let value = serde_json::to_value(&m).unwrap();
    assert!(value.get("ear").is_none());
    assert_eq!(value["expect"]["lines"][1]["pass"], Value::Null);
}

#[test]
fn analyze_with_the_ear_checks_sounds_like_and_marks_the_answer_as_advice() {
    if !python_found() {
        return;
    }
    let dir = folder("analyze");
    let keeper = EarKeeper::new(config(&stub_file(), &dir.join("ear-home")));

    // The stub says yes to a coin: the line passes.
    let mut m = measured(
        &coin_expecting(r#"{ sounds_like = "a soft coin pickup" }"#),
        &dir,
        "coin.toml",
    );
    let numbers_before = serde_json::to_value(m.measured.analysis.loudness_lufs).unwrap();
    ear::add_to_analysis(&keeper, &m.buffer, &mut m.measured).unwrap();
    let a = &m.measured;
    let section = serde_json::to_value(a.ear.as_ref().unwrap()).unwrap();
    assert_eq!(section["available"], true);
    assert_eq!(section["ok"], true);
    assert_eq!(section["stub"], true);
    assert_eq!(section["advice"], ear::ADVICE);
    assert_eq!(section["models"]["score"], "stub@0");
    assert_eq!(section["closest"][0]["text"], "coin pickup");
    assert_eq!(
        section["description"],
        "A stub Ear: no model listened to this."
    );
    assert_eq!(section["sounds_like"]["verdict"], "yes");
    assert_eq!(section["sounds_like"]["match"], 0.9);
    assert_eq!(section["sounds_like"]["answer"], "yes");
    assert_eq!(section["sounds_like"]["from"], json!(["score", "describe"]));
    assert_eq!(section["heard"]["from_seconds"], 0.0);
    assert!(section["heard"]["to_seconds"].as_f64().unwrap() > 0.0);
    let line = a.expect.as_ref().unwrap().lines.last().unwrap();
    assert_eq!(line.pass, Some(true));
    assert!(a.analysis.summary.contains("expect: all 1 met; Ear (stub, not listening): sounds like \"a soft coin pickup\": yes (0.90)"), "{}", a.analysis.summary);
    // The numbers are the same as without the Ear.
    assert_eq!(
        serde_json::to_value(a.analysis.loudness_lufs).unwrap(),
        numbers_before
    );

    // It says no to anything else: the line fails, and says why.
    let mut m = measured(
        &coin_expecting(r#"{ sounds_like = "an explosion" }"#),
        &dir,
        "boom.toml",
    );
    ear::add_to_analysis(&keeper, &m.buffer, &mut m.measured).unwrap();
    let line = m.measured.expect.as_ref().unwrap().lines.last().unwrap();
    assert_eq!(line.pass, Some(false));
    assert!(
        m.measured
            .analysis
            .summary
            .contains("EXPECT: 0 of 1 met (the Ear says no to sounds like \"an explosion\")"),
        "{}",
        m.measured.analysis.summary
    );

    // With no sounds_like, the section still has the closest words.
    let mut m = measured(
        duck_echo_tools::presets::get("coin").unwrap().source,
        &dir,
        "plain.toml",
    );
    ear::add_to_analysis(&keeper, &m.buffer, &mut m.measured).unwrap();
    assert!(
        m.measured
            .analysis
            .summary
            .ends_with("Ear (stub, not listening): closest \"coin pickup\" (0.50)"),
        "{}",
        m.measured.analysis.summary
    );
}

#[test]
fn a_song_has_an_expect_table_too() {
    let dir = folder("song");
    let song = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs/four_bars.toml"),
    )
    .unwrap();
    let song = song.replacen(
        "version = 0\n",
        "version = 0\nexpect = { sounds_like = \"a cheerful little tune\", loudness_lufs = [-40, 0] }\n",
        1,
    );
    let m = measured(&song, &dir, "tune.toml").measured;
    let expect = m.expect.as_ref().unwrap();
    assert_eq!(expect.lines.len(), 2);
    assert_eq!(expect.lines[0].what, "loudness_lufs");
    assert_eq!(expect.lines[0].pass, Some(true));
    assert_eq!(expect.lines[1].pass, None);
    assert!(
        m.analysis
            .summary
            .ends_with("; expect: all 1 met (sounds_like not checked)"),
        "{}",
        m.analysis.summary
    );

    // Its mistakes are found like a recipe's.
    let bad = song.replace("a cheerful little tune", "");
    std::fs::write(dir.join("bad.toml"), &bad).unwrap();
    let report = duck_echo_tools::load_document(
        &dir.join("bad.toml"),
        &duck_echo_tools::song::files_next_to(&dir.join("bad.toml")),
    )
    .err()
    .unwrap()
    .into_report();
    assert_eq!(report.errors[0].path, "expect.sounds_like", "{report:?}");
}

#[test]
fn sounds_like_is_a_few_words() {
    let dir = folder("words");
    for (text, ok) in [
        ("\"\"", false),
        ("\"   \"", false),
        (&format!("\"{}\"", "a".repeat(201)), false),
        (&format!("\"{}\"", "a".repeat(200)), true),
    ] {
        let file = dir.join("coin.toml");
        std::fs::write(
            &file,
            coin_expecting(&format!("{{ sounds_like = {text} }}")),
        )
        .unwrap();
        let loaded =
            duck_echo_tools::load_document(&file, &duck_echo_tools::song::files_next_to(&file));
        match (loaded, ok) {
            (Ok(_), true) => {}
            (Err(e), false) => {
                let report = e.into_report();
                assert_eq!(report.errors[0].path, "expect.sounds_like");
                assert!(
                    report.errors[0].message.contains("1 to 200 characters"),
                    "{report:?}"
                );
            }
            (Ok(_), false) => panic!("{text} should be refused"),
            (Err(e), true) => panic!("{text} should be fine: {:?}", e.into_report()),
        }
    }
}

#[test]
fn a_missing_ear_is_said_in_the_answer_and_the_numbers_stay() {
    let dir = folder("missing");
    let keeper = EarKeeper::new(config(&dir.join("no-ear-here"), &dir.join("ear-home")));
    let (_dir, root) = project("missing-project", &keeper);

    let listened = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "like": ["a coin"]}),
    )
    .unwrap();
    assert_eq!(listened["ok"], true);
    assert_eq!(listened["ear"]["available"], false);
    assert_eq!(listened["ear"]["reason"], "not_installed");
    assert!(
        listened["ear"]["message"]
            .as_str()
            .unwrap()
            .contains("duck-echo ear install"),
        "{listened}"
    );
    assert!(listened["summary"]
        .as_str()
        .unwrap()
        .starts_with("Ear: not available (not_installed)"));
    assert!(listened.get("like").is_none());

    let mut m = measured(
        &coin_expecting(r#"{ sounds_like = "a coin pickup" }"#),
        &dir,
        "coin.toml",
    );
    let before = m.measured.analysis.summary.clone();
    ear::add_to_analysis(&keeper, &m.buffer, &mut m.measured).unwrap();
    let line = m.measured.expect.as_ref().unwrap().lines.last().unwrap();
    assert_eq!(line.pass, None);
    assert!(
        line.note.as_deref().unwrap().contains("not available"),
        "{line:?}"
    );
    assert_eq!(
        m.measured.analysis.summary,
        format!("{before}; Ear: not available (not_installed)")
    );

    let status = run(&root, "ear_status", json!({})).unwrap();
    assert_eq!(status["available"], false);
    assert_eq!(status["reason"], "not_installed");
    assert_eq!(status["found_by"], "named");
}

#[test]
fn listen_answers_likes_a_question_a_comparison_and_quickly() {
    if !python_found() {
        return;
    }
    let dir = folder("listen");
    let keeper = EarKeeper::new(config(&stub_file(), &dir.join("ear-home")));
    let (_dir, root) = project("listen-project", &keeper);

    let listened = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "like": ["a soft coin pickup", "a harsh alarm"], "ask": "Is it gentle?"}),
    )
    .unwrap();
    assert_eq!(listened["file"], "sfx/coin.toml");
    assert_eq!(listened["like"][0]["verdict"], "yes");
    assert_eq!(listened["like"][1]["verdict"], "no");
    assert_eq!(listened["like"][1]["rank"], 5);
    assert_eq!(listened["answer"]["question"], "Is it gentle?");
    assert!(listened["answer"]["text"]
        .as_str()
        .unwrap()
        .contains("stub"));
    assert_eq!(
        listened["summary"],
        "Ear (stub, not listening): \"a soft coin pickup\": yes (0.90), \"a harsh alarm\": no (0.10)"
    );

    // Two sounds: the stub hears no difference.
    let compared = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "like": ["a coin"], "compare_with": "sfx/chime.toml"}),
    )
    .unwrap();
    assert_eq!(
        compared["compared"][0]["more_like"], "about the same",
        "{compared}"
    );
    assert_eq!(compared["compared"][0]["by"], 0.0);

    // Quick: the score only.
    let quick = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "like": ["a coin"], "quick": true}),
    )
    .unwrap();
    assert!(quick["ear"].get("description").is_none(), "{quick}");
    assert!(quick["like"][0].get("answer").is_none());
    assert_eq!(quick["like"][0]["from"], json!(["score"]));
    assert_eq!(quick["like"][0]["verdict"], "yes");

    // A preset and an inline recipe work too, and solo hears one layer.
    let solo = run(
        &root,
        "listen",
        json!({"path": "preset:coin", "solo": [0], "quick": true}),
    )
    .unwrap();
    assert_eq!(solo["ear"]["available"], true);
    let inline = run(
        &root,
        "listen",
        json!({"recipe": {"version": 0, "layers": [{"source": "sine"}]}, "quick": true}),
    )
    .unwrap();
    assert_eq!(inline["file"], "the recipe");

    // Its mistakes are named.
    let nine: Vec<String> = (0..9).map(|i| format!("text {i}")).collect();
    for (args, path) in [
        (
            json!({"path": "sfx/coin.toml", "like": nine}),
            "arguments.like",
        ),
        (
            json!({"path": "sfx/coin.toml", "like": [""]}),
            "arguments.like[0]",
        ),
        (
            json!({"path": "sfx/coin.toml", "compare_with": "sfx/chime.toml"}),
            "arguments.compare_with",
        ),
        (json!({"like": ["a coin"]}), "arguments.path"),
        (
            json!({"path": "sfx/coin.toml", "loud": true}),
            "arguments.loud",
        ),
    ] {
        let report = run(&root, "listen", args.clone()).unwrap_err();
        assert_eq!(report.errors[0].path, path, "{args}: {report:?}");
    }

    // ear_status says what the stub is.
    let status = run(&root, "ear_status", json!({})).unwrap();
    assert_eq!(status["available"], true);
    assert_eq!(status["stub"], true);
    assert_eq!(status["running"], true);
    assert!(status["answers_kept"].as_u64().unwrap() > 0);
    assert!(status["summary"].as_str().unwrap().contains("a stub"));
}

#[test]
fn the_same_question_gets_the_identical_answer_from_the_cache_without_asking_the_ear() {
    if !python_found() {
        return;
    }
    let dir = folder("cache");
    let home = dir.join("ear-home");
    let keeper = EarKeeper::new(config(&stub_file(), &home));
    let (_dir, root) = project("cache-project", &keeper);
    let question =
        json!({"path": "sfx/coin.toml", "like": ["a soft coin pickup"], "ask": "Is it gentle?"});

    let first = run(&root, "listen", question.clone()).unwrap();
    assert_eq!(first["ear"]["cached"], false);
    keeper.stop();
    assert!(!keeper.is_running());

    let second = run(&root, "listen", question.clone()).unwrap();
    assert_eq!(second["ear"]["cached"], true);
    // The Ear was greeted (its hello names its models), never asked.
    let log = keeper.log_tail();
    assert!(log.contains("hello"), "{log}");
    assert!(
        !log.contains("answered score"),
        "the cache answered, yet: {log}"
    );
    let without_cached = |mut v: Value| {
        v["ear"].as_object_mut().unwrap().remove("cached");
        v
    };
    assert_eq!(without_cached(first.clone()), without_cached(second));

    // Another sound is a new question.
    let other = run(
        &root,
        "listen",
        json!({"path": "sfx/chime.toml", "like": ["a soft coin pickup"], "ask": "Is it gentle?"}),
    )
    .unwrap();
    assert_eq!(other["ear"]["cached"], false);

    // A new model revision is a new question too, and its answer is kept
    // beside the old one.
    let newer = dir.join("newer_stub.py");
    let text = std::fs::read_to_string(stub_file()).unwrap();
    std::fs::write(
        &newer,
        text.replace("SCORE_REVISION = \"0\"", "SCORE_REVISION = \"1\""),
    )
    .unwrap();
    let newer_keeper = EarKeeper::new(config(&newer, &home));
    let (_dir, newer_root) = project("cache-newer", &newer_keeper);
    let newer_answer = run(&newer_root, "listen", question.clone()).unwrap();
    assert_eq!(newer_answer["ear"]["cached"], false);
    assert_eq!(newer_answer["ear"]["models"]["score"], "stub@1");
    let again = run(&newer_root, "listen", question).unwrap();
    assert_eq!(again["ear"]["cached"], true);
}

#[test]
fn the_same_program_with_another_model_misses_the_cache() {
    // Step 3 found this: the real Ear is one program for every model (it
    // reads which from its folder or DUCK_EAR_SCORE_MODEL). So the cache
    // keys must come from the running Ear's hello, never from the program.
    if !python_found() {
        return;
    }
    let dir = folder("cache-model");
    let stub = dir.join("stub").join("duck_ear_stub.py");
    std::fs::create_dir_all(stub.parent().unwrap()).unwrap();
    std::fs::copy(stub_file(), &stub).unwrap();
    std::fs::write(dir.join("stub/stub_revision.txt"), "a").unwrap();
    let keeper = EarKeeper::new(config(&stub, &dir.join("ear-home")));
    let (_dir, root) = project("cache-model-project", &keeper);
    let question = json!({"path": "sfx/coin.toml", "like": ["a coin"], "quick": true});
    let first = run(&root, "listen", question.clone()).unwrap();
    assert_eq!(first["ear"]["models"]["score"], "stub@a");
    keeper.stop();
    std::fs::write(dir.join("stub/stub_revision.txt"), "b").unwrap();
    let second = run(&root, "listen", question).unwrap();
    assert_eq!(second["ear"]["models"]["score"], "stub@b", "{second}");
    assert_eq!(
        second["ear"]["cached"], false,
        "another model gave the old model's answer"
    );
}

#[test]
fn the_ear_is_stopped_after_its_idle_time() {
    if !python_found() {
        return;
    }
    let dir = folder("idle");
    let mut config = config(&stub_file(), &dir.join("ear-home"));
    config.ear_idle = Duration::from_millis(500);
    let keeper = EarKeeper::new(config);
    let (_dir, root) = project("idle-project", &keeper);
    run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "quick": true}),
    )
    .unwrap();
    assert!(keeper.is_running());
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !keeper.is_running(),
        "the Ear was not stopped after 0.5 s with no question"
    );
    // The next question starts it again.
    run(
        &root,
        "listen",
        json!({"path": "sfx/chime.toml", "quick": true}),
    )
    .unwrap();
    assert!(keeper.is_running());
}

#[test]
fn the_describe_model_is_let_go_and_loaded_again_when_needed() {
    if !python_found() {
        return;
    }
    let dir = folder("describe-idle");
    let mut config = config(&stub_file(), &dir.join("ear-home"));
    config.describe_idle_seconds = 1;
    let keeper = EarKeeper::new(config);
    let (_dir, root) = project("describe-idle-project", &keeper);
    let first = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "ask": "first question"}),
    )
    .unwrap();
    assert!(first["ear"]["ok"] == true, "{first}");
    std::thread::sleep(Duration::from_millis(1500));
    // A score question alone does not need the describe model.
    let scored = run(
        &root,
        "listen",
        json!({"path": "sfx/chime.toml", "quick": true}),
    )
    .unwrap();
    assert!(scored["ear"]["ok"] == true, "{scored}");
    // A describe question loads it again (the stub refuses one without it).
    let second = run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "ask": "second question"}),
    )
    .unwrap();
    assert!(second["ear"]["ok"] == true, "{second}");
    let log = keeper.log_tail();
    assert!(
        log.contains("stopped the describe model after 1 s"),
        "{log}"
    );
    assert_eq!(log.matches("loaded the describe model").count(), 2, "{log}");
}

#[test]
fn the_sounds_written_for_the_ear_are_deleted() {
    if !python_found() {
        return;
    }
    let dir = folder("temp");
    let keeper = EarKeeper::new(config(&stub_file(), &dir.join("ear-home")));
    let (_dir, root) = project("temp-project", &keeper);
    run(
        &root,
        "listen",
        json!({"path": "sfx/coin.toml", "like": ["a coin"], "compare_with": "sfx/chime.toml"}),
    )
    .unwrap();
    let sounds = keeper.sounds_folder().to_path_buf();
    let left: Vec<_> = std::fs::read_dir(&sounds)
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    drop(root);
    drop(keeper);
    assert!(!sounds.exists(), "{} is still there", sounds.display());
}

/// Step 3: `ear check` on a small test set, with the stub (which says 0.9
/// to any text with "coin" in it and 0.1 to anything else, whatever the
/// sound).
#[test]
fn ear_check_measures_a_test_set_and_writes_its_report() {
    if !python_found() {
        return;
    }
    let dir = folder("check");
    let keeper = EarKeeper::new(config(&stub_file(), &dir.join("ear-home")));
    let testset = dir.join("testset.toml");
    std::fs::write(
        &testset,
        r#"
version = 1

[[sound]]
file = "preset:coin"
true = ["a coin pickup"]
wrong = ["an explosion"]

[[sound]]
file = "preset:explosion"
true = ["an explosion"]
wrong = ["a coin pickup"]

[[sound]]
file = "silence"
true = ["silence"]
wrong = ["a coin pickup"]
"#,
    )
    .unwrap();

    let report = duck_echo_tools::ear_check::check(&keeper, &testset).unwrap();
    assert_eq!((report.sounds, report.texts), (3, 3));
    let measure = |name: &str| {
        let m = report.measures.iter().find(|m| m.name == name).unwrap();
        (m.right, m.of)
    };
    // Only the coin has its true text above its wrong one.
    assert_eq!(measure("Kind of sound"), (1, 3));
    // Yes only for "a coin pickup" on the coin; no only for "an explosion"
    // on the coin.
    assert_eq!(measure("Yes on true texts"), (1, 3));
    assert_eq!(measure("No on wrong texts"), (1, 3));
    // The stub says yes to a coin in silence: not listening.
    assert_eq!(measure("Silence"), (0, 1));
    let explosion = &report.results[1];
    assert_eq!(explosion.file, "preset:explosion");
    assert_eq!(explosion.beaten_by, ["a coin pickup"]);
    // The stub reports no lines: the defaults.
    assert_eq!(report.lines.yes_from, ear::YES_FROM);
    assert!(report.table.contains("| Kind of sound | 1 of 3 |"));

    let out = dir.join("out");
    duck_echo_tools::ear_check::write(&report, &out).unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("ear-check.json")).unwrap())
            .unwrap();
    assert_eq!(written["sounds"], 3);
    assert!(std::fs::read_to_string(out.join("ear-check.md"))
        .unwrap()
        .contains("`preset:explosion`"));
}

#[test]
fn ear_check_refuses_a_test_set_with_an_unknown_field() {
    let dir = folder("check-bad");
    let keeper = EarKeeper::new(config(&stub_file(), &dir.join("ear-home")));
    let testset = dir.join("testset.toml");
    std::fs::write(
        &testset,
        "version = 1\n[[sound]]\nfile = \"preset:coin\"\ntrue = []\nwrong = []\nloud = true\n",
    )
    .unwrap();
    let error = duck_echo_tools::ear_check::check(&keeper, &testset)
        .unwrap_err()
        .into_report();
    let text = serde_json::to_string(&error).unwrap();
    assert!(text.contains("loud"), "{text}");
}
