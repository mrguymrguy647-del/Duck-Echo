//! Phase 4, step 2: the Duck Ear on the command line (`analyze --ear`,
//! `listen`, `ear status`), run as a person or an agent would run them.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `duck-echo` in `dir`, with the Ear's folder inside it.
fn duck_echo(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_duck-echo"))
        .args(args)
        .current_dir(dir)
        .env("DUCK_ECHO_EAR_HOME", dir.join("ear-home"))
        .env_remove("DUCK_ECHO_EAR")
        .output()
        .unwrap()
}

fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn stub() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ear/stub/duck_ear_stub.py")
        .to_string_lossy()
        .into_owned()
}

fn python_found() -> bool {
    let python = if cfg!(windows) { "python" } else { "python3" };
    let python = std::env::var("DUCK_ECHO_EAR_PYTHON").unwrap_or_else(|_| python.into());
    let found = Command::new(&python)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        if std::env::var("DUCK_ECHO_REQUIRE_PYTHON").as_deref() == Ok("1") {
            panic!("no Python at {python}, and DUCK_ECHO_REQUIRE_PYTHON=1");
        }
        eprintln!("skipped: no Python at {python}");
    }
    found
}

#[test]
fn without_an_ear_the_commands_say_so_and_still_measure() {
    let dir = fresh("cli-ear-missing");
    let missing = dir.join("no-ear").to_string_lossy().into_owned();
    let out = duck_echo(
        &dir,
        &["analyze", "preset:coin", "--ear", "--ear-program", &missing],
    );
    assert!(out.status.success());
    let a = json_of(&out);
    assert_eq!(a["ear"]["available"], false);
    assert!(a["loudness_lufs"].is_number());
    // Without --ear, no Ear at all.
    let out = duck_echo(&dir, &["analyze", "preset:coin", "--ear-program", &missing]);
    assert!(json_of(&out).get("ear").is_none());

    let out = duck_echo(
        &dir,
        &[
            "listen",
            "preset:coin",
            "--like",
            "a coin",
            "--ear-program",
            &missing,
        ],
    );
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("Ear: not available (not_installed)"),
        "{text}"
    );

    let out = duck_echo(
        &dir,
        &["ear", "status", "--ear-program", &missing, "--json"],
    );
    assert!(out.status.success());
    assert_eq!(json_of(&out)["available"], false);

    // A bad timeout scale is an error that names it.
    let out = Command::new(env!("CARGO_BIN_EXE_duck-echo"))
        .args(["ear", "status", "--json"])
        .current_dir(&dir)
        .env("DUCK_ECHO_EAR_TIMEOUT_SCALE", "slow")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("DUCK_ECHO_EAR_TIMEOUT_SCALE"));
}

#[test]
fn listen_and_ear_status_with_the_stub() {
    if !python_found() {
        return;
    }
    let dir = fresh("cli-ear-stub");
    let stub = stub();
    let out = duck_echo(
        &dir,
        &[
            "listen",
            "preset:coin",
            "--like",
            "a soft coin pickup",
            "--like",
            "a harsh alarm",
            "--ear-program",
            &stub,
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("Ear (stub, not listening): \"a soft coin pickup\": yes (0.90), \"a harsh alarm\": no (0.10)"),
        "{text}"
    );
    assert!(text.contains("description: A stub Ear"), "{text}");

    let chime = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/recipes/chime.toml");
    let chime = chime.to_string_lossy();
    let out = duck_echo(&dir, &["analyze", &chime, "--ear", "--ear-program", &stub]);
    let a = json_of(&out);
    assert_eq!(a["ear"]["sounds_like"]["text"], "a soft, gentle menu chime");
    assert_eq!(a["expect"]["lines"][0]["what"], "sounds_like");

    let out = duck_echo(&dir, &["ear", "status", "--ear-program", &stub]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("is ready (a stub"), "{text}");
    assert!(text.contains("answers kept:"), "{text}");
}

#[test]
fn the_old_spelling_ear_program_is_accepted_with_a_note() {
    let dir = fresh("cli-ear-old-spelling");
    let missing = dir.join("no-ear").to_string_lossy().into_owned();
    for args in [
        vec![
            "listen",
            "preset:coin",
            "--quick",
            "--ear",
            missing.as_str(),
        ],
        vec!["ear", "status", "--ear", missing.as_str()],
    ] {
        let out = duck_echo(&dir, &args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let note = String::from_utf8_lossy(&out.stderr);
        assert!(
            note.contains("now spelled --ear-program"),
            "{args:?}: {note}"
        );
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            said.contains("not_installed") || said.contains("not available"),
            "{said}"
        );
        assert!(
            said.contains("no-ear"),
            "the old spelling named the program: {said}"
        );
    }
}
