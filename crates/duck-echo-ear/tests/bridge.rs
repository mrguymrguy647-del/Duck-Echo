//! The bridge, against a pretend Ear that misbehaves on purpose
//! (`fake-ear`, built with this crate, so no Python is needed for these),
//! and against the stub Ear in `ear/stub/` (which needs Python).
//!
//! Set `DUCK_ECHO_REQUIRE_PYTHON=1` (CI does) to make the stub's tests
//! fail, rather than skip, when no Python is found.

use duck_echo_ear::{Ear, Failure, Program, Reason, Settings, Times};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Short times, so a test that waits for a timeout is quick.
fn quick() -> Times {
    let seconds = Duration::from_secs;
    Times {
        hello: seconds(10),
        load_score: seconds(2),
        load_describe: seconds(2),
        score: seconds(2),
        describe_on_card: seconds(2),
        describe_on_cpu: seconds(2),
        bye: seconds(1),
    }
}

/// The pretend Ear, behaving as `how` says.
fn fake(how: &str) -> Settings {
    let mut settings = Settings::new(Program::new(env!("CARGO_BIN_EXE_fake-ear")).arg(how));
    settings.times = quick();
    settings
}

fn start(how: &str) -> Ear {
    Ear::start(&fake(how)).unwrap_or_else(|e| panic!("{how}: {e:?}"))
}

/// Ask with no progress wanted.
fn ask(ear: &mut Ear, method: &str, params: Value) -> Result<Value, Failure> {
    ear.call(method, params, &mut |_| {})
}

/// A new, empty folder for one test.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-ear-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_good_ear_says_hello_answers_and_stops_politely() {
    let mut ear = start("well");
    let hello = ear.hello().clone();
    assert_eq!(hello.protocol, 1);
    assert_eq!(hello.ear_version, "0.0.1-fake");
    assert!(hello.can("score") && hello.can("ask"));
    assert_eq!(hello.device, "cpu");
    assert_eq!(hello.models[0].revision, "1");
    let answer = ask(&mut ear, "score", json!({"texts": ["a coin"]})).unwrap();
    assert_eq!(
        answer,
        json!({"method": "score", "params": {"texts": ["a coin"]}})
    );
    // Each request gets its own id, so many in a row work.
    for n in 0..50 {
        assert_eq!(
            ask(&mut ear, "load", json!({"n": n})).unwrap()["params"]["n"],
            n
        );
    }
    assert!(ear.is_running());
    assert_eq!(ear.stop(), Some(0));
}

#[test]
fn a_missing_ear_is_not_installed() {
    let dir = folder("missing");
    for file in [dir.join("no-such-ear"), dir.join("no_such_ear.py")] {
        let error = Ear::start(&Settings::new(Program::new(&file)))
            .err()
            .unwrap();
        assert_eq!(error.reason, Reason::NotInstalled, "{error:?}");
        assert_eq!(error.reason.code(), "not_installed");
        assert!(error.message.contains("no-such-ear") || error.message.contains("no_such_ear"));
    }
}

#[test]
fn a_file_that_is_not_a_program_will_not_start() {
    let dir = folder("not-a-program");
    let file = dir.join("notes.txt");
    std::fs::write(&file, "just some words").unwrap();
    let error = Ear::start(&Settings::new(Program::new(&file)))
        .err()
        .unwrap();
    assert_eq!(error.reason, Reason::WillNotStart, "{error:?}");
}

#[test]
fn an_ear_that_never_says_hello_is_given_up_on_in_time() {
    let mut settings = fake("silent");
    settings.times.hello = Duration::from_millis(500);
    let started = Instant::now();
    let error = Ear::start(&settings).err().unwrap();
    assert_eq!(error.reason, Reason::NoHello, "{error:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn another_protocol_or_a_broken_hello_is_refused() {
    let error = Ear::start(&fake("old_protocol")).err().unwrap();
    assert_eq!(error.reason, Reason::Protocol, "{error:?}");
    assert!(
        error.message.contains("update the Ear"),
        "{}",
        error.message
    );

    let error = Ear::start(&fake("broken_hello")).err().unwrap();
    assert_eq!(error.reason, Reason::BrokenHello, "{error:?}");
}

#[test]
fn a_slow_answer_times_out_and_the_ear_is_stopped() {
    let mut ear = start("slow");
    let started = Instant::now();
    let failure = ear
        .call_within("score", json!({}), Duration::from_millis(300), &mut |_| {})
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(failure.code(), "timeout");
    assert!(failure
        .to_string()
        .contains("starts again on the next question"));
    assert!(!ear.is_running());
    assert_eq!(
        ask(&mut ear, "score", json!({})).unwrap_err(),
        Failure::NotRunning
    );
}

#[test]
fn progress_never_gives_a_request_more_time() {
    let mut ear = start("progress_forever");
    let mut seen = 0;
    let started = Instant::now();
    let failure = ear
        .call_within("ask", json!({}), Duration::from_millis(500), &mut |p| {
            assert_eq!(p.message, "still going");
            seen += 1;
        })
        .unwrap_err();
    assert_eq!(failure.code(), "timeout");
    assert!(seen >= 2, "{seen} progress lines");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_line_that_is_not_json_is_a_broken_answer() {
    let mut ear = start("garbage");
    match ask(&mut ear, "score", json!({})).unwrap_err() {
        Failure::BrokenAnswer { why, line } => {
            assert!(why.contains("not JSON"), "{why}");
            assert_eq!(line, "this is not JSON at all");
        }
        other => panic!("{other:?}"),
    }
    assert!(!ear.is_running());
}

#[test]
fn an_answer_to_another_question_is_a_broken_answer() {
    let mut ear = start("wrong_id");
    match ask(&mut ear, "score", json!({})).unwrap_err() {
        Failure::BrokenAnswer { why, .. } => assert!(why.contains("request 101"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_ear_that_falls_over_gives_its_exit_code_and_last_words() {
    let mut ear = start("exit_midway");
    match ask(&mut ear, "score", json!({})).unwrap_err() {
        Failure::EarStopped { exit_code, log } => {
            assert_eq!(exit_code, Some(3));
            assert!(log.contains("about to fall over"), "{log}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!ear.is_running());
}

#[test]
fn fifty_megabytes_of_log_never_block_the_ear() {
    let mut ear = start("stderr_flood");
    let answer = ear
        .call_within(
            "score",
            json!({"n": 1}),
            Duration::from_secs(60),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(answer["params"]["n"], 1);
    // Only the end of the log is kept.
    let tail = ear.log_tail();
    assert!(tail.chars().count() <= 4001, "{}", tail.len());
    assert!(tail.contains("log line"));
}

#[test]
fn a_line_over_a_megabyte_is_a_broken_answer() {
    let mut ear = start("huge_line");
    match ask(&mut ear, "score", json!({})).unwrap_err() {
        Failure::BrokenAnswer { why, line } => {
            assert!(why.contains("1 MB"), "{why}");
            assert!(line.len() < 1000);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn windows_line_ends_and_fields_from_a_newer_ear_are_fine() {
    for how in ["crlf", "extra_fields"] {
        let mut ear = start(how);
        assert_eq!(
            ask(&mut ear, "score", json!({"a": 1})).unwrap()["params"]["a"],
            1
        );
        assert_eq!(ear.stop(), Some(0), "{how}");
    }
}

#[test]
fn an_error_answer_leaves_the_ear_running() {
    let mut ear = start("ear_error");
    let failure = ask(&mut ear, "score", json!({})).unwrap_err();
    assert_eq!(
        failure,
        Failure::EarError {
            code: "unreadable_audio".into(),
            message: "that is not a WAV file".into()
        }
    );
    assert_eq!(failure.code(), "unreadable_audio");
    assert!(ear.is_running());
    assert!(ask(&mut ear, "score", json!({})).is_err());
    assert!(ear.is_running());
}

#[test]
fn an_ear_that_ignores_bye_is_stopped_anyway() {
    let ear = start("ignores_bye");
    let started = Instant::now();
    ear.stop();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

/// Is something still writing to `file`? Reads it twice, apart.
fn still_beating(file: &Path) -> bool {
    std::thread::sleep(Duration::from_millis(300));
    let before = std::fs::read_to_string(file).unwrap_or_default();
    std::thread::sleep(Duration::from_millis(500));
    let after = std::fs::read_to_string(file).unwrap_or_default();
    before != after
}

/// Start the pretend Ear with a child of its own, and wait for the child.
fn start_with_child(file: &Path) -> Ear {
    let mut settings = fake("child");
    settings.program = settings.program.arg(file);
    let ear = Ear::start(&settings).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !file.exists() {
        assert!(Instant::now() < deadline, "the child never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(still_beating(file), "the child is not running");
    ear
}

#[test]
fn stopping_the_ear_stops_what_it_started_too() {
    let dir = folder("tree");
    // Politely: the Ear answers bye and exits, but its child would live on.
    let file = dir.join("heartbeat-stop");
    let ear = start_with_child(&file);
    ear.stop();
    assert!(!still_beating(&file), "the child outlived stop()");

    // Dropped without a word (as when a request fails).
    let file = dir.join("heartbeat-drop");
    let ear = start_with_child(&file);
    drop(ear);
    assert!(!still_beating(&file), "the child outlived drop");

    // After a timeout.
    let file = dir.join("heartbeat-timeout");
    let mut ear = start_with_child(&file);
    // A wait of zero always runs out before any answer can come.
    let failure = ear
        .call_within("score", json!({}), Duration::ZERO, &mut |_| {})
        .unwrap_err();
    assert_eq!(failure.code(), "timeout");
    assert!(!still_beating(&file), "the child outlived a timeout");
}

#[test]
fn an_ear_in_a_folder_with_spaces_and_accents_starts() {
    let dir = folder("accents").join("Dück Ëar folder");
    std::fs::create_dir_all(&dir).unwrap();
    let name = if cfg!(windows) {
        "fake ëar.exe"
    } else {
        "fake ëar"
    };
    let copy = dir.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_fake-ear"), &copy).unwrap();
    let mut settings = Settings::new(Program::new(&copy).arg("well"));
    settings.times = quick();
    let mut ear = Ear::start(&settings).unwrap();
    assert!(ask(&mut ear, "score", json!({})).is_ok());
    assert_eq!(ear.stop(), Some(0));
}

// The stub Ear, through Python.

fn stub_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ear/stub/duck_ear_stub.py")
}

/// The stub's settings, or `None` (with a note) when there is no Python.
fn stub() -> Option<Settings> {
    let program = Program::new(stub_file());
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
        return None;
    }
    let mut settings = Settings::new(program);
    settings.times = quick();
    Some(settings)
}

/// A tiny WAV file: 0.01 s of silence.
fn write_wav(path: &Path) {
    let samples = 441u32;
    let data = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&44100u32.to_le_bytes());
    bytes.extend_from_slice(&(44100u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    bytes.resize(bytes.len() + data as usize, 0);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn the_stub_answers_every_request_the_design_names() {
    let Some(settings) = stub() else { return };
    let mut ear = Ear::start(&settings).unwrap();
    let hello = ear.hello().clone();
    assert!(hello.stub);
    assert_eq!(hello.device, "none");
    assert!(hello.can("score") && hello.can("describe") && hello.can("ask"));

    let dir = folder("stub").join("Dück Ëar sounds");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("coin ü.wav");
    write_wav(&wav);
    let wav = wav.to_string_lossy().to_string();

    let loaded = ask(&mut ear, "load", json!({"roles": ["score", "describe"]})).unwrap();
    assert_eq!(loaded["loaded"], json!(["score", "describe"]));

    let scored = ask(
        &mut ear,
        "score",
        json!({"wav": wav, "texts": ["a soft coin pickup", "an explosion"]}),
    )
    .unwrap();
    assert_eq!(scored["stub"], true);
    assert_eq!(scored["scores"][0]["match"], 0.9);
    assert_eq!(scored["scores"][1]["match"], 0.1);
    assert_eq!(scored["scores"][0]["text"], "a soft coin pickup");

    let described = ask(&mut ear, "describe", json!({"wav": wav})).unwrap();
    assert!(described["text"].as_str().unwrap().contains("stub"));

    let answered = ask(
        &mut ear,
        "ask",
        json!({"wav": wav, "question": "Is this calm?"}),
    )
    .unwrap();
    assert_eq!(answered["answer"], "unsure");

    // Its errors leave it running.
    let not_wav = dir.join("notes.wav");
    std::fs::write(&not_wav, "words").unwrap();
    let failure = ask(&mut ear, "score", json!({"wav": not_wav, "texts": ["x"]})).unwrap_err();
    assert_eq!(failure.code(), "unreadable_audio", "{failure}");
    let failure = ask(&mut ear, "sing", json!({})).unwrap_err();
    assert_eq!(failure.code(), "unknown_method", "{failure}");
    let failure = ask(&mut ear, "score", json!({"wav": wav})).unwrap_err();
    assert_eq!(failure.code(), "bad_params", "{failure}");
    assert!(ear.is_running());
    assert!(
        ear.log_tail().contains("describe_idle_seconds = 120"),
        "{}",
        ear.log_tail()
    );

    assert_eq!(ear.stop(), Some(0));
}

#[test]
fn the_stub_exits_as_soon_as_its_stdin_closes() {
    let Some(settings) = stub() else { return };
    let program = settings.program;
    let mut child = std::process::Command::new(&program.command)
        .args(&program.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdin.take()); // as when Duck Echo stops, however it stops
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "the stub did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
}
