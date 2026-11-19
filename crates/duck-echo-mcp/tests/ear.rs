//! Phase 4, step 2: the MCP server's Duck Ear (docs/phase4-design.md,
//! section 6): `--ear`, `analyze` with `ear: true`, `listen` and
//! `ear_status`, through the real `duck-echo-mcp` binary over stdio.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    /// Start the server on `root` with the Ear `ear`, its folder in `home`.
    fn start(root: &Path, ear: &Path, home: &Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_duck-echo-mcp"))
            .arg("--root")
            .arg(root)
            .arg("--ear")
            .arg(ear)
            .env("DUCK_ECHO_EAR_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start duck-echo-mcp");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut server = Server {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        server.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        server
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    /// Call a tool: its answer, and whether it is an error.
    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        (
            result["structuredContent"].clone(),
            result["isError"].as_bool().unwrap(),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Closing its stdin ends the server, which stops its Ear.
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fresh(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stub_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ear/stub/duck_ear_stub.py")
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
fn the_tools_list_has_listen_ear_status_and_analyze_with_ear() {
    let dir = fresh("ear-tools-list");
    let mut server = Server::start(&dir, &dir.join("no-ear"), &dir.join("ear-home"));
    let tools = server.request("tools/list", json!({}))["result"]["tools"].clone();
    let tool = |name: &str| {
        tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
            .clone()
    };
    assert!(tool("listen")["description"]
        .as_str()
        .unwrap()
        .contains("advice"));
    assert!(tool("listen")["inputSchema"]["properties"]["like"].is_object());
    assert!(tool("ear_status")["description"]
        .as_str()
        .unwrap()
        .contains("loads no model"));
    assert_eq!(
        tool("analyze")["inputSchema"]["properties"]["ear"]["type"],
        "boolean"
    );
}

#[test]
fn without_an_ear_analyze_still_measures_and_says_why() {
    let dir = fresh("ear-missing");
    let mut server = Server::start(&dir, &dir.join("no-ear"), &dir.join("ear-home"));
    let (plain, error) = server.call("analyze", json!({"path": "preset:coin"}));
    assert!(!error, "{plain}");
    assert!(
        plain.get("ear").is_none(),
        "analyze without ear never asks the Ear"
    );
    let (heard, error) = server.call("analyze", json!({"path": "preset:coin", "ear": true}));
    assert!(!error, "{heard}");
    assert_eq!(heard["ear"]["available"], false);
    assert_eq!(heard["ear"]["reason"], "not_installed");
    assert_eq!(heard["loudness_lufs"], plain["loudness_lufs"]);
    let (status, _) = server.call("ear_status", json!({}));
    assert_eq!(status["available"], false);
}

#[test]
fn the_server_keeps_one_ear_for_every_question() {
    if !python_found() {
        return;
    }
    let dir = fresh("ear-stub");
    let mut server = Server::start(&dir, &stub_file(), &dir.join("ear-home"));
    let (heard, error) = server.call("analyze", json!({"path": "preset:coin", "ear": true}));
    assert!(!error, "{heard}");
    assert_eq!(heard["ear"]["available"], true);
    assert_eq!(heard["ear"]["stub"], true);
    assert!(heard["summary"]
        .as_str()
        .unwrap()
        .contains("Ear (stub, not listening)"));

    let (listened, error) = server.call(
        "listen",
        json!({"path": "preset:coin", "like": ["a coin pickup"]}),
    );
    assert!(!error, "{listened}");
    assert_eq!(listened["like"][0]["verdict"], "yes");

    // The same Ear answered both: one hello in its log.
    let (status, _) = server.call("ear_status", json!({}));
    assert_eq!(status["running"], true);
    assert_eq!(status["found_by"], "named");
    assert!(
        std::fs::read_to_string(dir.join("ear-home/ear.log"))
            .unwrap()
            .matches("hello; describe_idle_seconds = 120")
            .count()
            == 1
    );
}
