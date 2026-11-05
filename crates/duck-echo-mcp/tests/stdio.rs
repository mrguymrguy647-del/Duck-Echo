//! Roadmap task 9: drive the real `duck-echo-mcp` binary over stdio.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Session {
    fn start(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_duck-echo-mcp"))
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start duck-echo-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad reply {line:?}: {e}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let reply = self.read();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["jsonrpc"], "2.0");
        reply
    }

    /// Call a tool; returns (structuredContent, isError).
    fn call(&mut self, tool: &str, args: Value) -> (Value, bool) {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text content");
        let structured = result["structuredContent"].clone();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), structured);
        (structured, result["isError"].as_bool().unwrap())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fresh_root(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn started(root: &Path) -> Session {
    let mut s = Session::start(root);
    let init = s.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("preset:NAME"));
    s.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    s
}

#[test]
fn create_edit_render_analyze_loop() {
    let root = fresh_root("mcp_loop");
    let mut s = started(&root);

    let tools = s.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "get_schema",
            "list_presets",
            "get_preset",
            "validate",
            "create",
            "edit",
            "render",
            "analyze",
            "compare",
            "variations"
        ]
    );
    for t in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(
            t["description"].as_str().unwrap().len() > 40,
            "{}",
            t["name"]
        );
    }

    // Start from a preset.
    let (presets, err) = s.call("list_presets", json!({"tag": "pickup"}));
    assert!(!err);
    assert!(presets["presets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "coin"));
    let (coin, err) = s.call("get_preset", json!({"name": "coin"}));
    assert!(!err);
    assert!(coin["toml"].as_str().unwrap().contains("[[layers]]"));

    // create -> edit -> render -> analyze.
    let (created, err) = s.call(
        "create",
        json!({"path": "sfx/coin.toml", "recipe": coin["recipe"]}),
    );
    assert!(!err, "{created}");
    assert_eq!(created["path"], "sfx/coin.toml");
    let (again, err) = s.call(
        "create",
        json!({"path": "sfx/coin.toml", "recipe": coin["recipe"]}),
    );
    assert!(
        err && again["errors"][0]["path"] == "arguments.overwrite",
        "{again}"
    );

    let (edited, err) = s.call(
        "edit",
        json!({"path": "sfx/coin.toml", "ops": [
            {"path": "master.normalize_peak_db", "value": -9},
            {"path": "layers[0].pitch.start", "value": "E6"}
        ]}),
    );
    assert!(!err, "{edited}");
    assert_eq!(edited["written"], true);
    assert_eq!(edited["recipe"]["layers"][0]["pitch"]["start"], "E6");
    let on_disk = std::fs::read_to_string(root.join("sfx/coin.toml")).unwrap();
    assert!(on_disk.contains("start = \"E6\""), "{on_disk}");

    let (rendered, err) = s.call(
        "render",
        json!({"path": "sfx/coin.toml", "output": "out/coin.wav"}),
    );
    assert!(!err, "{rendered}");
    assert_eq!(rendered["output"], "out/coin.wav");
    assert!(root.join("out/coin.wav").exists());
    assert!((rendered["analysis"]["peak_dbfs"].as_f64().unwrap() + 9.0).abs() < 0.05);

    let (analysis, err) = s.call("analyze", json!({"path": "out/coin.wav"}));
    assert!(!err);
    assert_eq!(analysis["summary"], rendered["analysis"]["summary"]);
    let first_hz = analysis["pitch_track"][0]["dominant_hz"].as_f64().unwrap();
    assert!(
        (first_hz - 1318.5).abs() < 5.0,
        "E6 expected, got {first_hz}"
    );

    let (cmp, err) = s.call("compare", json!({"a": "preset:coin", "b": "out/coin.wav"}));
    assert!(!err);
    assert!(
        cmp["summary"].as_str().unwrap().contains("quieter"),
        "{}",
        cmp["summary"]
    );

    let (vars, err) = s.call(
        "variations",
        json!({"path": "sfx/coin.toml", "count": 3, "variation": 0.4, "output_dir": "vars"}),
    );
    assert!(!err, "{vars}");
    assert_eq!(vars["renders"].as_array().unwrap().len(), 3);
    assert_eq!(vars["renders"][0]["output"], "vars/coin_seed1.wav");
    assert!(root.join("vars/coin_seed3.wav").exists());

    // Inline recipes work too.
    let (inline, err) = s.call(
        "analyze",
        json!({"recipe": {"version": 0, "layers": [{"source": "sine", "pitch": {"start": "A4"}}]}}),
    );
    assert!(!err);
    assert!(
        inline["summary"].as_str().unwrap().contains("440 Hz"),
        "{}",
        inline["summary"]
    );
}

#[test]
fn invalid_edits_return_the_report_and_change_nothing() {
    let root = fresh_root("mcp_invalid");
    let mut s = started(&root);
    let (_, err) = s.call(
        "create",
        json!({"path": "a.toml", "recipe": {"version": 0, "layers": [{"source": "sine"}]}}),
    );
    assert!(!err);
    let before = std::fs::read_to_string(root.join("a.toml")).unwrap();

    let (report, err) = s.call(
        "edit",
        json!({"path": "a.toml", "ops": [
            {"path": "layers[0].volume", "value": 5},
            {"path": "layers[0].sorce", "value": "noise"}
        ]}),
    );
    assert!(err);
    // The plain validation report shape, nothing else.
    let keys: Vec<&String> = report.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["errors", "ok", "warnings"]);
    assert_eq!(report["ok"], false);
    let paths: Vec<&str> = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.contains(&"layers[0].volume") && paths.contains(&"layers[0].sorce"),
        "{paths:?}"
    );
    assert!(report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e.get("line").is_none()));
    assert_eq!(
        std::fs::read_to_string(root.join("a.toml")).unwrap(),
        before
    );

    // validate answers with the report as a normal result.
    let (v, err) = s.call(
        "validate",
        json!({"recipe": {"version": 0, "layers": [{"volume": 9}]}}),
    );
    assert!(!err);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["path"], "layers[0].volume");

    // A bad edit path names the op.
    let (bad, err) = s.call(
        "edit",
        json!({"path": "a.toml", "ops": [{"path": "layers[7].volume", "value": 1}]}),
    );
    assert!(err);
    assert!(bad["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("past the end"));
}

#[test]
fn arguments_paths_and_protocol_edges() {
    let root = fresh_root("mcp_edges");
    let mut s = started(&root);

    let (e, err) = s.call("render", json!({"path": "preset:coin", "outptu": "x.wav"}));
    assert!(err);
    assert_eq!(e["errors"][0]["path"], "arguments.outptu");
    assert_eq!(e["errors"][0]["suggestion"], "output");

    let (e, err) = s.call(
        "render",
        json!({"path": "preset:coin", "output": "../escape.wav"}),
    );
    assert!(err);
    assert!(e["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("outside the server root"));
    assert!(!root.parent().unwrap().join("escape.wav").exists());

    let (e, err) = s.call("get_preset", json!({"name": "gemm"}));
    assert!(err);
    assert!(e["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("`gem`"));

    let unknown = s.request("tools/call", json!({"name": "explode", "arguments": {}}));
    assert_eq!(unknown["error"]["code"], -32602);
    let missing = s.request("resources/list", json!({}));
    assert_eq!(missing["error"]["code"], -32601);
    assert_eq!(s.request("ping", json!({}))["result"], json!({}));

    // Unknown protocol versions get the newest supported one.
    let mut s2 = Session::start(&root);
    let init = s2.request(
        "initialize",
        json!({"protocolVersion": "1999-01-01", "capabilities": {}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");

    // Junk input gets a parse error, and the server keeps going.
    writeln!(s.stdin, "not json").unwrap();
    s.stdin.flush().unwrap();
    assert_eq!(s.read()["error"]["code"], -32700);
    let (schema, err) = s.call("get_schema", json!({}));
    assert!(!err);
    assert_eq!(schema["title"], "Duck Echo recipe v0");
}
