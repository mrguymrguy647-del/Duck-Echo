//! A pretend Duck Ear for the bridge's tests. It speaks the protocol well,
//! or breaks it on purpose in the way its first argument names, so the
//! tests can check every way a real Ear could go wrong, without Python.
//!
//! `fake-ear <how>`, where `<how>` is one of:
//!
//! * `well`: answers everything properly (each answer echoes its request).
//! * `silent`: never answers anything, not even `hello`.
//! * `slow`: says hello, then takes 30 s to answer anything else.
//! * `progress_forever`: says hello, then only ever sends progress lines.
//! * `garbage`: says hello, then answers with a line that is not JSON.
//! * `wrong_id`: says hello, then answers with the wrong `id`.
//! * `exit_midway`: says hello, then writes a last word to its log and
//!   exits with code 3 when asked anything.
//! * `stderr_flood`: says hello, then writes 50 MB to its log before
//!   each answer.
//! * `huge_line`: says hello, then answers with a 2 MB line.
//! * `old_protocol`: answers `hello` with a protocol error.
//! * `broken_hello`: answers `hello` with a result missing its fields.
//! * `crlf`: answers properly, with Windows line ends.
//! * `extra_fields`: answers properly, with fields Duck Echo does not know.
//! * `ear_error`: says hello, then answers everything with an error.
//! * `ignores_bye`: answers properly, but never answers `bye` and never
//!   exits when its stdin closes.
//! * `child <file>`: answers properly, and starts a child of its own that
//!   writes a growing count into `<file>` every 20 ms, forever (as
//!   `llama-server` will run under the real Ear).
//! * `heartbeat <file>`: the child above.
//!
//! Like a real Ear, it exits when its stdin closes (except `ignores_bye`).

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let how = args.get(1).map(String::as_str).unwrap_or("well");
    if how == "heartbeat" {
        heartbeat(args.get(2).expect("heartbeat needs a file"));
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = request["id"].as_u64().unwrap_or(0);
        let method = request["method"].as_str().unwrap_or("").to_string();
        if method == "hello" {
            answer_hello(how, id, &args, &mut stdout);
            continue;
        }
        if method == "bye" && how != "ignores_bye" {
            send(&mut stdout, &json!({"id": id, "result": {}}), "\n");
            std::process::exit(0);
        }
        answer(how, id, &method, &request["params"], &mut stdout);
    }
    if how == "ignores_bye" {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

fn send(stdout: &mut std::io::Stdout, value: &Value, end: &str) {
    let _ = write!(stdout, "{value}{end}");
    let _ = stdout.flush();
}

fn answer_hello(how: &str, id: u64, args: &[String], stdout: &mut std::io::Stdout) {
    match how {
        "silent" => {}
        "old_protocol" => send(
            stdout,
            &json!({"id": id, "error": {"code": "protocol", "message": "this Ear speaks protocol 2"}}),
            "\n",
        ),
        "broken_hello" => send(stdout, &json!({"id": id, "result": {"protocol": 1}}), "\n"),
        _ => {
            if how == "child" {
                start_heartbeat(args.get(2).expect("child needs a file"));
            }
            let mut hello = json!({
                "protocol": 1,
                "ear_version": "0.0.1-fake",
                "capabilities": ["score", "describe", "ask"],
                "device": "cpu",
                "models": [{"role": "score", "name": "fake", "revision": "1"}],
                "stub": true,
            });
            if how == "extra_fields" {
                hello["something_new"] = json!({"from": "a newer Ear"});
            }
            let end = if how == "crlf" { "\r\n" } else { "\n" };
            send(stdout, &json!({"id": id, "result": hello}), end);
        }
    }
}

fn answer(how: &str, id: u64, method: &str, params: &Value, stdout: &mut std::io::Stdout) {
    let echo = json!({"id": id, "result": {"method": method, "params": params}});
    match how {
        "silent" => {}
        "slow" => {
            std::thread::sleep(Duration::from_secs(30));
            send(stdout, &echo, "\n");
        }
        "progress_forever" => loop {
            send(
                stdout,
                &json!({"id": id, "progress": {"message": "still going", "done": 0.5}}),
                "\n",
            );
            std::thread::sleep(Duration::from_millis(50));
        },
        "garbage" => {
            let _ = writeln!(stdout, "this is not JSON at all");
            let _ = stdout.flush();
        }
        "wrong_id" => send(stdout, &json!({"id": id + 100, "result": {}}), "\n"),
        "exit_midway" => {
            eprintln!("about to fall over");
            std::process::exit(3);
        }
        "stderr_flood" => {
            let mut stderr = std::io::stderr();
            let line = format!("{}\n", "log line ".repeat(10));
            for _ in 0..(50 * 1024 * 1024 / line.len()) {
                let _ = stderr.write_all(line.as_bytes());
            }
            let _ = stderr.flush();
            send(stdout, &echo, "\n");
        }
        "huge_line" => {
            let big = "x".repeat(2 * 1024 * 1024);
            send(stdout, &json!({"id": id, "result": {"big": big}}), "\n");
        }
        "crlf" => send(stdout, &echo, "\r\n"),
        "extra_fields" => {
            let mut answer = echo;
            answer["also"] = json!("something new");
            send(stdout, &answer, "\n");
        }
        "ear_error" => send(
            stdout,
            &json!({"id": id, "error": {"code": "unreadable_audio", "message": "that is not a WAV file"}}),
            "\n",
        ),
        _ => send(stdout, &echo, "\n"),
    }
}

/// Start a copy of this program as a child that beats forever. It is
/// never waited for on purpose: the test checks that stopping the Ear
/// stops this child too, even though its parent forgot about it.
#[allow(clippy::zombie_processes)]
fn start_heartbeat(file: &str) {
    let me = std::env::current_exe().expect("this program's own path");
    std::process::Command::new(me)
        .args(["heartbeat", file])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start the heartbeat");
}

/// Write a growing count into `file` every 20 ms, forever.
fn heartbeat(file: &str) -> ! {
    let mut count: u64 = 0;
    loop {
        count += 1;
        let _ = std::fs::write(file, count.to_string());
        std::thread::sleep(Duration::from_millis(20));
    }
}
