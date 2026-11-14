//! The command-line tool and the MCP server change the same file at the
//! same moment, many times. No change is ever lost, no reader ever sees
//! half a file, every change is one Undo step, and undoing them all gives
//! back the file as it was.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How many layers each program adds.
const EACH: usize = 40;

/// The `duck-echo` binary, which sits next to `duck-echo-mcp`.
/// `cargo test --workspace` builds both before running any test. A test
/// cannot run cargo itself (cargo holds its build lock while tests run),
/// so when the CLI is missing or older than a source file of the crates,
/// this says how to build it instead of testing old code.
fn cli_binary() -> PathBuf {
    let mcp = PathBuf::from(env!("CARGO_BIN_EXE_duck-echo-mcp"));
    let cli = mcp.with_file_name(format!("duck-echo{}", std::env::consts::EXE_SUFFIX));
    fn newest(dir: &Path) -> Option<std::time::SystemTime> {
        let mut out = None;
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            let time = if path.is_dir() {
                newest(&path)
            } else {
                entry.metadata().and_then(|m| m.modified()).ok()
            };
            out = out.max(time);
        }
        out
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let sources = std::fs::read_dir(&crates)
        .unwrap()
        .flatten()
        .filter_map(|c| newest(&c.path().join("src")))
        .max();
    let built = std::fs::metadata(&cli).and_then(|m| m.modified()).ok();
    assert!(
        built.is_some() && built >= sources,
        "this test needs an up-to-date duck-echo binary at {}: run `cargo test --workspace`, or `cargo build -p duck-echo-cli` first",
        cli.display()
    );
    cli
}

fn layer(name: String) -> Value {
    json!({"name": name, "source": "sine", "volume": 0.01})
}

/// The names of the layers in the file, if it parses.
fn layer_names(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let doc: toml::Value = toml::from_str(&text)
        .unwrap_or_else(|e| panic!("read half a file or a broken one: {e}\n{text}"));
    doc["layers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(String::from))
        .collect()
}

#[test]
fn the_cli_and_the_mcp_server_never_lose_a_change() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shared_undo");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("busy.toml");
    let start = "# One layer to start with.\nversion = 0\nname = \"busy\"\n\n[[layers]]\nname = \"first\"\nsource = \"sine\"\n";
    std::fs::write(&file, start).unwrap();

    // The CLI's script: EACH commands, run one after another by one process.
    let script: Vec<Value> = (0..EACH)
        .map(|i| json!({"do": "sound_add_layer", "file": "busy.toml", "layer": layer(format!("cli{i}"))}))
        .collect();
    std::fs::write(
        root.join("script.json"),
        serde_json::to_string(&script).unwrap(),
    )
    .unwrap();

    // The MCP server, started and ready before anything begins.
    let mut server = Command::new(env!("CARGO_BIN_EXE_duck-echo-mcp"))
        .arg("--root")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start duck-echo-mcp");
    let mut to_server = server.stdin.take().unwrap();
    let mut from_server = BufReader::new(server.stdout.take().unwrap());
    let mut send = |msg: Value| {
        writeln!(to_server, "{msg}").unwrap();
        to_server.flush().unwrap();
    };
    send(
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}),
    );
    let mut line = String::new();
    from_server.read_line(&mut line).unwrap();
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    // A reader that keeps checking the file is always whole.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let (file, stop) = (file.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                layer_names(&file);
                reads += 1;
            }
            reads
        })
    };

    // Go: the CLI runs its script while the MCP server gets its calls.
    let cli = Command::new(cli_binary())
        .args(["run", "script.json", "--root"])
        .arg(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start duck-echo");
    for i in 0..EACH {
        send(
            json!({"jsonrpc": "2.0", "id": i + 1, "method": "tools/call", "params": {
                "name": "sound_add_layer",
                "arguments": {"file": "busy.toml", "layer": layer(format!("mcp{i}"))}
            }}),
        );
    }
    for _ in 0..EACH {
        line.clear();
        from_server.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
    }
    let cli = cli.wait_with_output().unwrap();
    assert!(
        cli.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&cli.stdout),
        String::from_utf8_lossy(&cli.stderr)
    );
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.join().unwrap() > 0);

    // Every change is there.
    let names = layer_names(&file);
    assert_eq!(names.len(), 1 + 2 * EACH, "{names:?}");
    for i in 0..EACH {
        assert!(names.contains(&format!("cli{i}")), "cli{i} was lost");
        assert!(names.contains(&format!("mcp{i}")), "mcp{i} was lost");
    }
    // Each program's own changes are in the order it made them.
    for who in ["cli", "mcp"] {
        let mine: Vec<&String> = names.iter().filter(|n| n.starts_with(who)).collect();
        let wanted: Vec<String> = (0..EACH).map(|i| format!("{who}{i}")).collect();
        assert_eq!(mine, wanted.iter().collect::<Vec<_>>());
    }
    // No temporary file or lock is left behind.
    fn temporary_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                temporary_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "tmp") {
                out.push(path);
            }
        }
    }
    let mut leftovers = Vec::new();
    temporary_files(&root, &mut leftovers);
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(!root.join(".duck-echo/lock").exists());

    // One step per change, by the right one, and undoing them all (from
    // the server) gives back the file exactly as it was.
    let call = |send: &mut dyn FnMut(Value),
                from: &mut BufReader<_>,
                id: usize,
                name: &str,
                args: Value| {
        send(
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}}),
        );
        let mut line = String::new();
        from.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        reply["result"].clone()
    };
    let history = call(
        &mut send,
        &mut from_server,
        1000,
        "edit_history",
        json!({"limit": 500}),
    );
    let steps = history["structuredContent"]["steps"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(steps.len(), 2 * EACH);
    let by_person = steps.iter().filter(|s| s["by"] == "person").count();
    let by_agent = steps.iter().filter(|s| s["by"] == "agent").count();
    assert_eq!((by_person, by_agent), (EACH, EACH));
    for i in 0..2 * EACH {
        let r = call(
            &mut send,
            &mut from_server,
            1001 + i,
            "edit_undo",
            json!({}),
        );
        assert_eq!(r["isError"], false, "{r}");
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), start);
    let r = call(&mut send, &mut from_server, 2000, "edit_undo", json!({}));
    assert_eq!(r["isError"], true, "{r}");

    // Closing its input ends the server.
    drop(to_server);
    server.wait().unwrap();
    std::fs::remove_dir_all(&root).ok();
}
