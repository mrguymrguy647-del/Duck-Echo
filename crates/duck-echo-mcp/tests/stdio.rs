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
        let (structured, err, _) = self.call_content(tool, args);
        (structured, err)
    }

    /// Call a tool; returns (structuredContent, isError, content blocks).
    fn call_content(&mut self, tool: &str, args: Value) -> (Value, bool, Vec<Value>) {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        assert_eq!(result["content"][0]["type"], "text");
        let text = result["content"][0]["text"].as_str().expect("text content");
        let structured = result["structuredContent"].clone();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), structured);
        (
            structured,
            result["isError"].as_bool().unwrap(),
            result["content"].as_array().unwrap().clone(),
        )
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
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("preset:NAME"));
    // The instructions point an agent at the guide before anything else.
    assert!(instructions.contains("Call get_guide first"));
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
        names[..14],
        [
            "get_guide",
            "get_schema",
            "list_presets",
            "get_preset",
            "list_instruments",
            "get_instrument",
            "list_files",
            "validate",
            "create",
            "edit",
            "render",
            "analyze",
            "compare",
            "variations"
        ]
    );
    // Then one tool per command, with the command's own name.
    let command_names: Vec<String> = duck_echo_tools::commands::names();
    assert_eq!(
        names[14..],
        command_names.iter().map(String::as_str).collect::<Vec<_>>()
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
            {"path": "master.normalize_peak_db", "value": -25},
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
    assert!((rendered["analysis"]["peak_dbfs"].as_f64().unwrap() + 25.0).abs() < 0.05);

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
        .contains("`..` is not allowed"));
    assert!(!root.parent().unwrap().join("escape.wav").exists());

    let (e, err) = s.call("get_preset", json!({"name": "gemm"}));
    assert!(err);
    assert_eq!(e["errors"][0]["code"], "not_found");
    assert_eq!(e["errors"][0]["path"], "arguments.name");
    assert_eq!(e["errors"][0]["suggestion"], "gem");
    assert!(e["errors"][0]["allowed"]["options"]
        .as_array()
        .unwrap()
        .contains(&json!("gem")));
    // MCP errors never point at the CLI.
    assert!(!e.to_string().contains("duck-echo presets"), "{e}");

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

/// A sandbox with `root/` (served) and `outside/` (must never be touched),
/// plus an unrelated file inside the root.
fn sandbox(name: &str) -> (PathBuf, PathBuf) {
    let base = fresh_root(name);
    let root = base.join("root");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(root.join("important.txt"), "keep me").unwrap();
    std::fs::write(outside.join("secret.wav"), "not audio").unwrap();
    (root, outside)
}

/// Every file under `dir`, relative, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let meta = std::fs::symlink_metadata(&p).unwrap();
            if meta.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn assert_refused(s: &mut Session, tool: &str, args: Value, why: &str) {
    let (report, err) = s.call(tool, args.clone());
    assert!(err, "{tool} {args} was not refused: {report}");
    assert_eq!(report["ok"], false);
    let msg = report["errors"][0]["message"].as_str().unwrap();
    assert!(
        msg.contains(why),
        "{tool} {args}: expected `{why}` in: {msg}"
    );
}

/// Security: no write path can leave the root, through `preset:`, `..`,
/// absolute paths or symbolic links, and render never clobbers non-WAV
/// files.
#[test]
fn write_paths_cannot_leave_the_root() {
    let (root, outside) = sandbox("mcp_security_writes");
    let base = root.parent().unwrap().to_path_buf();
    let outside_abs = outside.join("x.wav").display().to_string();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("out_link")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.wav"), root.join("file_link.wav")).unwrap();
        std::os::unix::fs::symlink(outside.join("missing.wav"), root.join("dangling.wav")).unwrap();
    }
    let before_root = files_under(&root);
    let before_outside = files_under(&outside);
    let mut s = started(&root);

    let bad_wavs: Vec<(String, &str)> = vec![
        ("preset:coin".into(), "read-only"),
        ("preset:/../../x.wav".into(), "read-only"),
        ("../x.wav".into(), "`..` is not allowed"),
        ("sfx/../../x.wav".into(), "`..` is not allowed"),
        (outside_abs.clone(), "absolute paths are not allowed"),
        #[cfg(unix)]
        (
            "out_link/x.wav".into(),
            "outside the server root through a symbolic link",
        ),
        #[cfg(unix)]
        (
            "file_link.wav".into(),
            "outside the server root through a symbolic link",
        ),
        #[cfg(unix)]
        ("dangling.wav".into(), "broken symbolic link"),
        ("important.txt".into(), "must end in .wav"),
        ("notes".into(), "must end in .wav"),
    ];
    for (out, why) in &bad_wavs {
        assert_refused(
            &mut s,
            "render",
            json!({"path": "preset:coin", "output": out}),
            why,
        );
    }

    let recipe = json!({"version": 0, "layers": [{"source": "sine"}]});
    for (path, why) in [
        ("preset:coin.toml", "read-only"),
        ("preset:/../../x.toml", "read-only"),
        ("../x.toml", "`..` is not allowed"),
        (
            &outside.join("x.toml").display().to_string()[..],
            "absolute paths are not allowed",
        ),
        #[cfg(unix)]
        (
            "out_link/x.toml",
            "outside the server root through a symbolic link",
        ),
    ] {
        assert_refused(
            &mut s,
            "create",
            json!({"path": path, "recipe": recipe}),
            why,
        );
    }

    for (dir, why) in [
        ("preset:coin", "read-only"),
        ("preset:/../..", "read-only"),
        ("..", "`..` is not allowed"),
        (
            &outside.display().to_string()[..],
            "absolute paths are not allowed",
        ),
        #[cfg(unix)]
        (
            "out_link",
            "outside the server root through a symbolic link",
        ),
    ] {
        assert_refused(
            &mut s,
            "variations",
            json!({"path": "preset:footstep", "count": 1, "output_dir": dir}),
            why,
        );
    }

    // Nothing was written or changed anywhere.
    assert_eq!(files_under(&root), before_root);
    assert_eq!(files_under(&outside), before_outside);
    assert_eq!(
        std::fs::read_to_string(root.join("important.txt")).unwrap(),
        "keep me"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.wav")).unwrap(),
        "not audio"
    );
    assert!(!base.join("x.wav").exists() && !base.parent().unwrap().join("x.wav").exists());

    // A recipe name is never trusted as a file name.
    let (vars, err) = s.call(
        "variations",
        json!({"recipe": {"version": 0, "name": "../../evil", "layers": [{"source": "noise"}]},
               "count": 1, "output_dir": "vars"}),
    );
    assert!(!err, "{vars}");
    assert_eq!(vars["renders"][0]["output"], "vars/______evil_seed1.wav");
    assert_eq!(files_under(&outside), before_outside);

    // Legitimate writes still work, including into new folders.
    let (ok, err) = s.call(
        "render",
        json!({"path": "preset:coin", "output": "a/b/coin.wav"}),
    );
    assert!(!err, "{ok}");
    assert!(root.join("a/b/coin.wav").exists());
}

/// Security: reads cannot leave the root either, and `preset:` only names
/// real presets.
#[test]
fn read_paths_cannot_leave_the_root() {
    let (root, outside) = sandbox("mcp_security_reads");
    std::fs::write(outside.join("recipe.toml"), "version = 0\n[[layers]]\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("out_link")).unwrap();
    let mut s = started(&root);

    for (path, why) in [
        ("../outside/recipe.toml", "`..` is not allowed"),
        (
            &outside.join("recipe.toml").display().to_string()[..],
            "absolute paths are not allowed",
        ),
        #[cfg(unix)]
        (
            "out_link/recipe.toml",
            "outside the server root through a symbolic link",
        ),
        ("preset:../coin", "preset name"),
        ("preset:/etc/passwd", "preset name"),
        ("preset:Coin", "preset name"),
        ("preset:nope", "no preset named"),
    ] {
        assert_refused(&mut s, "validate", json!({"path": path}), why);
        assert_refused(&mut s, "analyze", json!({"path": path}), why);
    }
    #[cfg(unix)]
    assert_refused(
        &mut s,
        "compare",
        json!({"a": "preset:coin", "b": "out_link/secret.wav"}),
        "outside the server root through a symbolic link",
    );
}

/// Playtest 1: an agent can see what it made, picture a sound, hear layers
/// on their own and level a sound to a loudness target in one render.
#[test]
fn files_pictures_layers_and_loudness_targets() {
    let root = fresh_root("mcp_playtest_fixes");
    std::fs::write(root.join(".gitkeep"), "").unwrap();
    let mut s = started(&root);

    let (coin, _) = s.call("get_preset", json!({"name": "coin"}));
    let (_, err) = s.call(
        "create",
        json!({"path": "sfx/coin.toml", "recipe": coin["recipe"]}),
    );
    assert!(!err);
    // Level it to a louder target in one edit and one render.
    let (edited, err) = s.call(
        "edit",
        json!({"path": "sfx/coin.toml", "ops": [
            {"path": "master.normalize", "remove": true},
            {"path": "master.normalize_lufs", "value": -16}
        ]}),
    );
    assert!(!err, "{edited}");
    assert_eq!(edited["warnings"], json!([]));
    let (r, err, content) = s.call_content(
        "render",
        json!({"path": "sfx/coin.toml", "output": "sfx/coin.wav"}),
    );
    assert!(!err, "{r}");
    assert_eq!(content.len(), 1, "no picture unless asked");
    let lufs = r["analysis"]["loudness_lufs"].as_f64().unwrap();
    assert!((lufs + 16.0).abs() <= 0.1, "{lufs}");
    assert_eq!(r["warnings"], json!([]));

    // A target the peak ceiling does not allow renders quieter and warns.
    let (r, err) = s.call(
        "render",
        json!({"recipe": {"version": 0, "master": {"normalize_lufs": -2},
                          "layers": [{"source": "sine"}]},
               "output": "loud.wav"}),
    );
    assert!(!err, "{r}");
    assert_eq!(r["warnings"][0]["path"], "master.normalize_lufs");
    assert_eq!(r["warnings"][0]["code"], "suspicious");

    // list_files shows what was made, relative to the root.
    let (files, err) = s.call("list_files", json!({}));
    assert!(!err, "{files}");
    assert_eq!(files["folder"], ".");
    let entries: Vec<(String, String)> = files["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["path"].as_str().unwrap().to_string(),
                e["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let want = [
        ("loud.wav", "audio"),
        ("sfx", "folder"),
        ("sfx/coin.toml", "recipe"),
        ("sfx/coin.wav", "audio"),
    ];
    assert_eq!(
        entries,
        want.map(|(p, k)| (p.to_string(), k.to_string())),
        "{files}"
    );
    let wav = &files["entries"][3];
    assert!(
        (wav["seconds"].as_f64().unwrap() - r["analysis"]["duration_seconds"].as_f64().unwrap())
            .abs()
            < 1.0
    );
    let (flat, err) = s.call("list_files", json!({"path": "sfx", "recursive": false}));
    assert!(!err);
    assert_eq!(flat["entries"].as_array().unwrap().len(), 2);
    #[cfg(unix)]
    {
        let outside = fresh_root("mcp_playtest_outside");
        std::fs::write(outside.join("secret.wav"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let (files, _) = s.call("list_files", json!({}));
        let link = files["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == "link")
            .unwrap();
        assert_eq!(link["kind"], "link");
        assert!(
            !files.to_string().contains("secret"),
            "links are not followed"
        );
    }

    // A picture comes back as an image block, with its axes described.
    let (a, err, content) =
        s.call_content("analyze", json!({"path": "sfx/coin.wav", "picture": true}));
    assert!(!err, "{a}");
    assert_eq!(content.len(), 2);
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["mimeType"], "image/png");
    // Base64 of the PNG signature.
    assert!(content[1]["data"]
        .as_str()
        .unwrap()
        .starts_with("iVBORw0KGgo"));
    assert_eq!(a["picture"]["width"], 480);
    assert!(a["energy_over_time"]["rms_dbfs"].as_array().unwrap().len() == 20);
    assert_eq!(a["onsets"].as_array().unwrap().len(), 2);
    let (_, err, content) = s.call_content(
        "render",
        json!({"path": "preset:explosion", "output": "boom.wav", "picture": true}),
    );
    assert!(!err);
    assert_eq!(content[1]["type"], "image");

    // Layers on their own, at their level in the mix.
    let layered = json!({"version": 0, "layers": [
        {"name": "lead", "source": "square", "pitch": {"start": "C5", "arpeggio": {"steps": [0, 7], "step_time": 0.1, "repeat": false}},
         "envelope": {"hold": 0.2, "decay": 0.05}, "volume": 0.4},
        {"name": "bass", "source": "triangle", "pitch": {"start": "C3"}, "envelope": {"hold": 0.2, "decay": 0.05}}
    ]});
    let (a, err) = s.call("analyze", json!({"recipe": layered, "per_layer": true}));
    assert!(!err, "{a}");
    assert_eq!(a["layers"][0]["name"], "lead");
    let lead_notes: Vec<f64> = a["layers"][0]["analysis"]["onsets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["note_hz"].as_f64().unwrap())
        .collect();
    assert_eq!(lead_notes.len(), 2, "{lead_notes:?}");
    assert!((lead_notes[0] - 523.3).abs() < 5.0 && (lead_notes[1] - 784.0).abs() < 5.0);
    let (solo, err) = s.call("analyze", json!({"recipe": layered, "mute": [0]}));
    assert!(!err);
    assert_eq!(solo["heard_layers"], json!([1]));
    // The muted mix reads like layer 1 alone, plus the note on what was heard.
    assert_eq!(
        solo["summary"].as_str().unwrap(),
        format!(
            "{}, 1 of 2 layers heard",
            a["layers"][1]["analysis"]["summary"].as_str().unwrap()
        )
    );
    let (r, err) = s.call(
        "render",
        json!({"recipe": layered, "output": "bass.wav", "solo": [1], "per_layer": true}),
    );
    assert!(!err, "{r}");
    assert_eq!(r["heard_layers"], json!([1]));
    assert_eq!(r["layers"].as_array().unwrap().len(), 1);
}

/// Playtest 1: every failure has a specific code and a path, messages show
/// paths relative to the root, and hints point at MCP tools.
#[test]
fn every_error_has_a_code_a_path_and_no_absolute_path() {
    let root = fresh_root("mcp_error_polish");
    let mut s = started(&root);
    let (_, err) = s.call(
        "render",
        json!({"path": "preset:coin", "output": "sfx/coin.wav"}),
    );
    assert!(!err);
    std::fs::write(root.join("notes.txt"), "not audio").unwrap();
    std::fs::write(
        root.join("broken.toml"),
        "version = 0\n[[layers]\nsource = 1\n",
    )
    .unwrap();
    let (_, err) = s.call(
        "create",
        json!({"path": "a.toml", "recipe": {"version": 0, "layers": [{}]}}),
    );
    assert!(!err);
    let abs = root.canonicalize().unwrap().display().to_string();

    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "analyze",
            json!({"path": "sfx/con.wav"}),
            "not_found",
            "arguments.path",
        ),
        (
            "analyze",
            json!({"path": "notes.txt"}),
            "io",
            "arguments.path",
        ),
        ("analyze", json!({}), "missing_field", "arguments.path"),
        (
            "analyze",
            json!({"path": "sfx/coin.wav", "per_layer": true}),
            "invalid_argument",
            "arguments.per_layer",
        ),
        (
            "analyze",
            json!({"path": "preset:coin", "solo": [3]}),
            "out_of_range",
            "arguments.solo[0]",
        ),
        (
            "analyze",
            json!({"path": "preset:coin", "solo": "0"}),
            "type_mismatch",
            "arguments.solo",
        ),
        (
            "analyze",
            json!({"path": "preset:coin", "picture": "yes"}),
            "type_mismatch",
            "arguments.picture",
        ),
        (
            "render",
            json!({"output": "x.wav"}),
            "missing_field",
            "arguments.recipe",
        ),
        (
            "render",
            json!({"path": "preset:coin", "output": "x.wav", "format": "mp3"}),
            "unknown_variant",
            "arguments.format",
        ),
        (
            "render",
            json!({"path": "preset:coin", "output": "x.wav", "seed": -1}),
            "type_mismatch",
            "arguments.seed",
        ),
        (
            "render",
            json!({"path": "preset:coin", "output": "x.txt"}),
            "path_refused",
            "arguments.output",
        ),
        (
            "render",
            json!({"path": "preset:coin", "output": "../x.wav"}),
            "path_refused",
            "arguments.output",
        ),
        (
            "render",
            json!({"path": "preset:coin", "recipe": {}, "output": "x.wav"}),
            "invalid_argument",
            "arguments.recipe",
        ),
        (
            "render",
            json!({"path": "broken.toml", "output": "x.wav"}),
            "syntax",
            "arguments.path",
        ),
        (
            "create",
            json!({"path": "a.toml", "recipe": {"version": 0, "layers": [{}]}}),
            "already_exists",
            "arguments.overwrite",
        ),
        (
            "create",
            json!({"path": "a.txt", "recipe": {"version": 0, "layers": [{}]}}),
            "path_refused",
            "arguments.path",
        ),
        (
            "edit",
            json!({"path": "a.toml", "ops": [{"path": "", "value": 1}]}),
            "invalid",
            "arguments.ops",
        ),
        (
            "edit",
            json!({"path": "a.toml", "ops": "nope"}),
            "type_mismatch",
            "arguments.ops",
        ),
        (
            "edit",
            json!({"recipe": {"version": 0, "layers": [{}]}, "ops": [], "write": true}),
            "invalid_argument",
            "arguments.write",
        ),
        (
            "variations",
            json!({"path": "preset:coin", "count": 0}),
            "out_of_range",
            "arguments.count",
        ),
        (
            "variations",
            json!({"path": "preset:coin", "variation": 2}),
            "out_of_range",
            "arguments.variation",
        ),
        (
            "compare",
            json!({"a": 5, "b": "preset:coin"}),
            "type_mismatch",
            "arguments.a",
        ),
        (
            "compare",
            json!({"a": "preset:coin", "b": "gone.wav"}),
            "not_found",
            "arguments.b",
        ),
        (
            "get_preset",
            json!({"name": "gemm"}),
            "not_found",
            "arguments.name",
        ),
        (
            "list_files",
            json!({"path": "nothere"}),
            "not_found",
            "arguments.path",
        ),
        (
            "list_files",
            json!({"path": ".."}),
            "path_refused",
            "arguments.path",
        ),
        (
            "list_files",
            json!({"path": "preset:coin"}),
            "path_refused",
            "arguments.path",
        ),
        (
            "list_files",
            json!({"recursive": "yes"}),
            "type_mismatch",
            "arguments.recursive",
        ),
        (
            "validate",
            json!({"path": "preset:nope"}),
            "not_found",
            "arguments.path",
        ),
    ];
    for (tool, args, code, path) in cases {
        let (report, err) = s.call(tool, args.clone());
        assert!(err, "{tool} {args}: not an error: {report}");
        let text = report.to_string();
        assert!(!text.contains(&abs), "{tool} {args} leaks the root: {text}");
        assert!(
            !text.contains("duck-echo "),
            "{tool} {args} names the CLI: {text}"
        );
        for e in report["errors"].as_array().unwrap() {
            assert_ne!(e["code"], "other", "{tool} {args}");
            assert!(
                !e["path"].as_str().unwrap().is_empty(),
                "{tool} {args}: {e}"
            );
        }
        assert_eq!(report["errors"][0]["code"], code, "{tool} {args}: {report}");
        assert_eq!(report["errors"][0]["path"], path, "{tool} {args}: {report}");
    }

    // A missing file names the closest file and the tool that lists them.
    let (e, _) = s.call("analyze", json!({"path": "sfx/con.wav"}));
    assert_eq!(e["errors"][0]["suggestion"], "sfx/coin.wav");
    assert!(e["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("list_files"));
    let (e, _) = s.call("analyze", json!({"path": "notes.txt"}));
    assert!(e["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("notes.txt"));

    // validate answers a bad recipe with isError false; the report says ok false.
    let (v, err) = s.call("validate", json!({"path": "broken.toml"}));
    assert!(!err);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["path"], "arguments.path");
    assert!(v["errors"][0]["line"].is_number());

    // The descriptions explain loudness methods and isError.
    let tools = s.request("tools/list", json!({}));
    let desc = |name: &str| {
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .to_string()
    };
    for name in ["analyze", "render", "compare"] {
        assert!(desc(name).contains("400 ms"), "{name}");
    }
    assert!(desc("validate").contains("isError"));
}

/// An unknown tag is an error with the closest real tag, not an empty list;
/// a layered recipe's summary says how many layers it has.
#[test]
fn tag_typos_and_layer_counts_help_the_agent() {
    let root = fresh_root("mcp_agent_hints");
    let mut s = started(&root);

    let (e, err) = s.call("list_presets", json!({"tag": "explsion"}));
    assert!(err, "{e}");
    assert_eq!(e["errors"][0]["code"], "not_found");
    assert_eq!(e["errors"][0]["path"], "arguments.tag");
    assert_eq!(e["errors"][0]["suggestion"], "explosion");

    let (ok, err) = s.call("list_presets", json!({"tag": "explosion"}));
    assert!(!err);
    assert!(!ok["presets"].as_array().unwrap().is_empty());

    let (a, err) = s.call("analyze", json!({"path": "preset:powerup"}));
    assert!(!err, "{a}");
    let summary = a["summary"].as_str().unwrap();
    assert!(
        summary.ends_with("2 layers; per_layer describes each"),
        "{summary}"
    );
}

/// A small song with its own instruments, as an agent would send it.
fn song_json(lead: &str) -> Value {
    json!({
        "version": 0,
        "name": "tune",
        "tempo": 120,
        "bars": 2,
        "instruments": {
            "tone": {
                "version": 0,
                "layers": [{
                    "source": "sine",
                    "envelope": {"attack": 0.01, "decay": 0.05, "sustain": 0.7, "release": 0.05},
                    "pitch": {"start": "A4"}
                }]
            }
        },
        "patterns": {"up": {"notes": [
            {"beat": 0, "note": "A3"}, {"beat": 1, "note": "C4"},
            {"beat": 2, "note": "E4"}, {"beat": 3, "note": "A4"}
        ]}},
        "tracks": [
            {"name": "melody", "instrument": lead, "notes": [
                {"beat": 0, "note": "A4", "length": 2},
                {"bar": 2, "beat": 0, "note": ["C5", "E5"], "length": 2}
            ]},
            {"name": "bass", "instrument": "tone", "volume": 0.8,
             "play": [{"pattern": "up", "bars": [1, 2]}]}
        ]
    })
}

/// Roadmap task 16: every tool that takes a recipe takes a song.
#[test]
fn songs_work_through_every_tool() {
    let root = fresh_root("mcp_songs");
    let mut s = started(&root);

    let (schema, err) = s.call("get_schema", json!({"kind": "song"}));
    assert!(!err);
    assert_eq!(schema["title"], "Duck Echo song v0");
    assert!(schema["properties"]["tracks"].is_object());
    let (schema, _) = s.call("get_schema", json!({}));
    assert_eq!(schema["title"], "Duck Echo recipe v0");
    let (report, err) = s.call("get_schema", json!({"kind": "tune"}));
    assert!(err);
    assert_eq!(report["errors"][0]["path"], "arguments.kind");

    // A recipe file next to the song is its lead instrument.
    let (made, err) = s.call(
        "create",
        json!({"path": "songs/lead.toml", "recipe": {
            "version": 0,
            "layers": [{"source": "square", "volume": 0.4,
                "envelope": {"attack": 0.01, "decay": 0.05, "sustain": 0.7, "release": 0.05},
                "pitch": {"start": "C4"}}]
        }}),
    );
    assert!(!err, "{made}");
    assert_eq!(made["kind"], "recipe");
    let (made, err) = s.call(
        "create",
        json!({"path": "songs/tune.toml", "recipe": song_json("lead.toml")}),
    );
    assert!(!err, "{made}");
    assert_eq!(made["kind"], "song");
    assert_eq!(made["path"], "songs/tune.toml");

    let (report, err) = s.call("validate", json!({"path": "songs/tune.toml"}));
    assert!(!err);
    assert_eq!(report["ok"], true, "{report}");

    let (done, err) = s.call(
        "render",
        json!({"path": "songs/tune.toml", "output": "out/tune.wav", "per_track": true}),
    );
    assert!(!err, "{done}");
    assert_eq!(done["output"], "out/tune.wav");
    assert_eq!(done["song"]["bars"], 2);
    assert_eq!(done["song"]["tempo"], 120.0);
    // Two bars at 120 BPM are exactly four seconds of 44.1 kHz stereo.
    assert_eq!(done["song"]["song_frames"], 176_400);
    assert_eq!(done["analysis"]["per_channel"].as_array().unwrap().len(), 2);
    let tracks = done["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["name"], "melody");
    assert_eq!(tracks[0]["notes"], 3);
    assert_eq!(tracks[1]["notes"], 8);
    assert!(root.join("out/tune.wav").exists());

    let (heard, err) = s.call(
        "analyze",
        json!({"path": "songs/tune.toml", "solo": ["bass"]}),
    );
    assert!(!err, "{heard}");
    assert_eq!(heard["heard_tracks"], json!(["bass"]));
    assert!(heard["summary"]
        .as_str()
        .unwrap()
        .starts_with("2 bars at 120 BPM, 2 tracks (1 heard); "));
    assert!(heard["onsets"].as_array().unwrap().len() >= 6, "{heard}");

    // analyze listens for every written note; render does with per_track.
    assert_eq!(heard["note_check"]["notes_written"], 8);
    assert_eq!(heard["note_check"]["notes_heard"], 8);
    assert_eq!(heard["note_check"]["tracks"][0]["name"], "bass");
    assert!(heard["summary"]
        .as_str()
        .unwrap()
        .contains("8 of 8 notes heard"));
    assert_eq!(done["note_check"]["notes_written"], 11);
    let (listed, err) = s.call(
        "analyze",
        json!({"path": "songs/tune.toml", "notes": true, "picture": true}),
    );
    assert!(!err, "{listed}");
    let notes = listed["note_check"]["tracks"][0]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(notes.len(), 3);
    assert_eq!(notes[1]["written"], "C5");
    assert_eq!(notes[1]["status"], "heard");
    assert_eq!(listed["picture"]["bar_lines"].as_array().unwrap().len(), 2);
    let (plain, _) = s.call(
        "render",
        json!({"path": "songs/tune.toml", "output": "out/plain.wav"}),
    );
    assert!(plain.get("note_check").is_none());

    // An inline song, and comparing a song with a WAV of itself.
    let (inline, err) = s.call("analyze", json!({"recipe": song_json("tone")}));
    assert!(!err, "{inline}");
    assert_eq!(inline["song"]["notes"], 11);
    let (same, err) = s.call(
        "compare",
        json!({"a": "songs/tune.toml", "b": "out/tune.wav"}),
    );
    assert!(!err, "{same}");
    assert!(
        same["correlation"]["value"].as_f64().unwrap_or(0.0) > 0.99
            || same["summary"].as_str().unwrap().contains("same"),
        "{same}"
    );

    // Edits use song paths, and a bad note is reported at its path.
    let (edited, err) = s.call(
        "edit",
        json!({"path": "songs/tune.toml", "ops": [{"path": "tracks[0].notes[0].note", "value": "B4"}]}),
    );
    assert!(!err, "{edited}");
    assert_eq!(edited["recipe"]["tracks"][0]["notes"][0]["note"], "B4");
    assert_eq!(edited["written"], true);
    let (report, err) = s.call(
        "edit",
        json!({"path": "songs/tune.toml", "ops": [{"path": "tracks[0].notes[0].note", "value": "H4"}]}),
    );
    assert!(err);
    assert_eq!(report["errors"][0]["path"], "tracks[0].notes[0].note");
    assert_eq!(report["errors"][0]["suggestion"], "B4");

    // Wrong kind of option or tool for a song.
    for (tool, args, path) in [
        (
            "analyze",
            json!({"path": "songs/tune.toml", "per_layer": true}),
            "arguments.per_layer",
        ),
        (
            "analyze",
            json!({"path": "songs/tune.toml", "solo": ["drums"]}),
            "arguments.solo[0]",
        ),
        (
            "analyze",
            json!({"path": "preset:coin", "per_track": true}),
            "arguments.per_track",
        ),
        (
            "analyze",
            json!({"path": "out/tune.wav", "per_track": true}),
            "arguments.per_track",
        ),
        (
            "analyze",
            json!({"path": "preset:coin", "notes": true}),
            "arguments.notes",
        ),
        (
            "analyze",
            json!({"path": "out/tune.wav", "notes": true}),
            "arguments.notes",
        ),
        (
            "variations",
            json!({"path": "songs/tune.toml"}),
            "arguments.path",
        ),
    ] {
        let (report, err) = s.call(tool, args.clone());
        assert!(err, "{tool} {args}: {report}");
        assert_eq!(report["errors"][0]["path"], path, "{tool} {args}: {report}");
    }
}

/// Security: a song's instrument files go through the root guard like
/// every other path.
#[test]
fn song_instrument_files_cannot_leave_the_root() {
    let (root, outside) = sandbox("mcp_security_songs");
    let recipe = "version = 0\n[[layers]]\nsource = \"sine\"\n";
    std::fs::write(outside.join("recipe.toml"), recipe).unwrap();
    std::fs::create_dir_all(root.join("shared")).unwrap();
    std::fs::create_dir_all(root.join("songs")).unwrap();
    std::fs::write(root.join("shared/lead.toml"), recipe).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("out_link")).unwrap();
    let mut s = started(&root);

    let absolute = outside.join("recipe.toml").display().to_string();
    for (reference, why) in [
        ("../outside/recipe.toml", "`..` is not allowed"),
        ("../../outside/recipe.toml", "`..` is not allowed"),
        // Even a `..` that would stay inside the root: paths never go up.
        ("../shared/lead.toml", "`..` is not allowed"),
        (&absolute[..], "absolute paths are not allowed"),
        #[cfg(unix)]
        (
            "out_link/recipe.toml",
            "outside the server root through a symbolic link",
        ),
        ("missing.toml", "does not exist under the server root"),
    ] {
        // Inline (read from the root) and as a file in a folder.
        let song = song_json(reference);
        let (made, err) = s.call(
            "create",
            json!({"path": "songs/bad.toml", "recipe": song.clone(), "overwrite": true}),
        );
        assert!(err, "{reference}: {made}");
        for (tool, args) in [
            ("validate", json!({"recipe": song.clone()})),
            ("render", json!({"recipe": song.clone(), "output": "x.wav"})),
            ("analyze", json!({"recipe": song.clone()})),
        ] {
            let (report, err) = s.call(tool, args);
            // validate answers with the report; the others fail.
            assert_eq!(err, tool != "validate", "{tool} {reference}: {report}");
            assert_eq!(report["ok"], false);
            let e = &report["errors"][0];
            assert_eq!(
                e["path"], "tracks[0].instrument",
                "{tool} {reference}: {report}"
            );
            let msg = e["message"].as_str().unwrap();
            assert!(
                msg.contains(why),
                "{tool} {reference}: expected `{why}` in: {msg}"
            );
            assert!(!report.to_string().contains(&root.display().to_string()));
        }
    }
    assert!(!root.join("songs/bad.toml").exists());
    assert!(!root.join("x.wav").exists());

    // A file inside the root, next to the song, is fine.
    let (made, err) = s.call(
        "create",
        json!({"path": "shared/ok.toml", "recipe": song_json("lead.toml")}),
    );
    assert!(!err, "{made}");
    // The same reference from the root does not find it: paths are
    // relative to the song's folder.
    let (report, _) = s.call("validate", json!({"recipe": song_json("lead.toml")}));
    assert_eq!(report["errors"][0]["code"], "not_found");
    let (report, _) = s.call("validate", json!({"recipe": song_json("shared/lead.toml")}));
    assert_eq!(report["ok"], true, "{report}");
}

/// Roadmap task 19: built-in instruments through the tools, and
/// `instrument:NAME` held to the same rules as `preset:NAME`: read-only,
/// a known name only, never a path.
#[test]
fn built_in_instruments_are_listed_played_and_read_only() {
    let (root, _outside) = sandbox("mcp_instruments");
    let mut s = started(&root);

    let (list, err) = s.call("list_instruments", json!({}));
    assert!(!err, "{list}");
    let names: Vec<&str> = list["instruments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "bass",
            "chip_lead",
            "clap",
            "hat",
            "kick",
            "pad",
            "piano",
            "snare",
            "drums"
        ]
    );
    assert_eq!(list["instruments"][6]["range"], json!(["C2", "C7"]));
    assert_eq!(
        list["instruments"][8]["pieces"],
        json!(["clap", "hat", "kick", "snare"])
    );

    let (piano, err) = s.call("get_instrument", json!({"name": "piano"}));
    assert!(!err, "{piano}");
    assert_eq!(piano["home"], "C4");
    assert_eq!(piano["recipe"]["layers"].as_array().unwrap().len(), 7);
    assert!(piano["toml"].as_str().unwrap().contains("hammer"));
    let (report, err) = s.call("get_instrument", json!({"name": "pianoo"}));
    assert!(err);
    assert_eq!(report["errors"][0]["path"], "arguments.name");
    assert_eq!(report["errors"][0]["suggestion"], "piano");

    // An instrument can be heard alone, like a preset.
    let (a, err) = s.call("analyze", json!({"path": "instrument:piano"}));
    assert!(!err, "{a}");
    assert!((a["loudness_lufs"].as_f64().unwrap() + 18.0).abs() < 0.1);
    let (done, err) = s.call(
        "render",
        json!({"path": "instrument:kick", "output": "kick.wav"}),
    );
    assert!(!err, "{done}");
    assert!(root.join("kick.wav").exists());
    // Editing one gives a changed copy and writes nothing.
    let (edited, err) = s.call(
        "edit",
        json!({"path": "instrument:bass", "ops": [{"path": "layers[0].volume", "value": 0.3}]}),
    );
    assert!(!err, "{edited}");
    assert_eq!(edited["written"], false);

    // A song names instruments plainly; a whole tune needs no other file.
    let song = json!({
        "version": 0, "tempo": 120, "bars": 1,
        "tracks": [
            {"name": "keys", "instrument": "piano", "notes": [{"beat": 0, "note": ["C4", "E4", "G4"], "length": 2}]},
            {"name": "beat", "instrument": "drums", "notes": [{"beat": 0, "note": "kick"}, {"beat": 1, "note": "snare"}]}
        ]
    });
    let (a, err) = s.call("analyze", json!({"recipe": song, "per_track": true}));
    assert!(!err, "{a}");
    assert_eq!(a["tracks"][1]["instrument"], "drums");

    // Only known names, only for reading, never a path.
    let before = files_under(&root);
    for (path, why) in [
        ("instrument:../piano", "instrument name"),
        ("instrument:/etc/passwd", "instrument name"),
        ("instrument:Piano", "instrument name"),
        ("instrument:piano.toml", "instrument name"),
        ("instrument:nope", "no built-in instrument named"),
        ("instrument:drums", "is a kit"),
    ] {
        assert_refused(&mut s, "validate", json!({"path": path}), why);
        assert_refused(&mut s, "analyze", json!({"path": path}), why);
        assert_refused(
            &mut s,
            "compare",
            json!({"a": path, "b": "preset:coin"}),
            why,
        );
    }
    for (tool, args) in [
        (
            "render",
            json!({"path": "preset:coin", "output": "instrument:piano"}),
        ),
        (
            "render",
            json!({"path": "preset:coin", "output": "instrument:../x.wav"}),
        ),
        (
            "create",
            json!({"path": "instrument:piano", "recipe": {"version": 0, "layers": [{}]}}),
        ),
        (
            "variations",
            json!({"path": "preset:coin", "output_dir": "instrument:piano"}),
        ),
    ] {
        assert_refused(&mut s, tool, args, "instruments are built in and read-only");
    }
    assert_refused(
        &mut s,
        "list_files",
        json!({"path": "instrument:piano"}),
        "instruments are built in, not files",
    );
    assert_eq!(files_under(&root), before, "a refused call wrote something");
}

/// Security: a drive letter in the middle of a path. On Windows, joining
/// `C:\x` onto a folder replaces the folder, so `sub/C:\Windows\x.wav`
/// could leave the root once `sub` exists. `:` is refused in every path,
/// on every platform, and nothing is ever written or read outside.
#[test]
fn drive_letters_inside_a_path_cannot_leave_the_root() {
    let (root, outside) = sandbox("mcp_security_drives");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(
        outside.join("recipe.toml"),
        "version = 0\n[[layers]]\nsource = \"sine\"\n",
    )
    .unwrap();
    let mut s = started(&root);
    let before_root = files_under(&root);
    let before_outside = files_under(&outside);
    let why = "`:` is not allowed in a path";

    // The real thing on this machine: an existing folder, then the full
    // path of a place outside the root.
    let real_out = format!("sub/{}", outside.join("evil.wav").display());
    let real_in = format!("sub/{}", outside.join("recipe.toml").display());

    for path in [
        "sub/C:\\Windows\\evil.wav",
        "sub/C:/Windows/evil.wav",
        "x:y.wav",
    ] {
        assert_refused(
            &mut s,
            "render",
            json!({"path": "preset:coin", "output": path}),
            why,
        );
        assert_refused(
            &mut s,
            "variations",
            json!({"path": "preset:coin", "output_dir": path}),
            why,
        );
    }
    for path in ["sub/C:\\Windows\\x.toml", "sub/C:/x.toml", "a:b.toml"] {
        assert_refused(
            &mut s,
            "create",
            json!({"path": path, "recipe": {"version": 0, "layers": [{}]}}),
            why,
        );
        assert_refused(&mut s, "validate", json!({"path": path}), why);
        assert_refused(&mut s, "analyze", json!({"path": path}), why);
        assert_refused(&mut s, "list_files", json!({"path": path}), why);
        assert_refused(
            &mut s,
            "compare",
            json!({"a": path, "b": "preset:coin"}),
            why,
        );
        // As a song's instrument file, inline and next to a song file.
        let (report, _) = s.call("validate", json!({"recipe": song_json(path)}));
        assert_eq!(report["ok"], false, "{path}");
        assert_eq!(report["errors"][0]["path"], "tracks[0].instrument");
        assert!(report["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains(why));
        let (made, err) = s.call(
            "create",
            json!({"path": "sub/song.toml", "recipe": song_json(path), "overwrite": true}),
        );
        assert!(err, "{path}: {made}");
    }

    // Whatever this platform makes of the real paths, the calls may
    // fail or stay inside the root, but nothing outside is touched or
    // read: the outside recipe is never accepted as an instrument.
    let _ = s.call("render", json!({"path": "preset:coin", "output": real_out}));
    let (report, _) = s.call("validate", json!({"recipe": song_json(&real_in)}));
    assert_eq!(report["ok"], false, "{report}");
    let (made, err) = s.call(
        "create",
        json!({"path": "sub/song.toml", "recipe": song_json(&real_in), "overwrite": true}),
    );
    assert!(err, "{made}");
    let (report, err) = s.call("analyze", json!({"path": real_in}));
    assert!(err, "{report}");

    assert_eq!(
        files_under(&outside),
        before_outside,
        "something outside changed"
    );
    let after: Vec<String> = files_under(&root)
        .into_iter()
        .filter(|f| !f.ends_with("evil.wav"))
        .collect();
    assert_eq!(after, before_root, "a refused call wrote something");
    assert!(!outside.join("evil.wav").exists());
}

/// Phase 2 step 2: the guide is built in, and `create` copies a preset.
#[test]
fn get_guide_and_create_from_a_preset() {
    let root = fresh_root("mcp_guide_from");
    let mut s = started(&root);

    // The guide, then a reference page, then a typo.
    let (guide, err) = s.call("get_guide", json!({}));
    assert!(!err);
    assert_eq!(guide["topic"], "guide");
    assert!(guide["text"]
        .as_str()
        .unwrap()
        .starts_with("# Duck Echo agent guide"));
    let (spec, err) = s.call("get_guide", json!({"topic": "song_spec"}));
    assert!(!err);
    assert!(spec["text"].as_str().unwrap().starts_with("## Songs"));
    let (bad, err) = s.call("get_guide", json!({"topic": "songs"}));
    assert!(err);
    assert_eq!(bad["errors"][0]["path"], "arguments.topic");
    assert!(bad["errors"][0]["allowed"]["options"]
        .as_array()
        .unwrap()
        .contains(&json!("song_spec")));

    // A copy of a preset, named after its file, comments kept.
    let (made, err) = s.call(
        "create",
        json!({"path": "sfx/my_coin.toml", "from": "preset:coin"}),
    );
    assert!(!err, "{made}");
    assert_eq!(made["path"], "sfx/my_coin.toml");
    assert_eq!(made["kind"], "recipe");
    let text = std::fs::read_to_string(root.join("sfx/my_coin.toml")).unwrap();
    assert!(text.contains("name = \"my_coin\""), "{text}");
    assert!(
        text.contains("description = \"Copy of the coin preset.\""),
        "{text}"
    );
    assert!(text.contains('#'), "comments are kept: {text}");

    // It exists now: refused at `overwrite`, then replaced when asked.
    let (again, err) = s.call(
        "create",
        json!({"path": "sfx/my_coin.toml", "from": "preset:laser"}),
    );
    assert!(err);
    assert_eq!(again["errors"][0]["path"], "arguments.overwrite");
    let (_, err) = s.call(
        "create",
        json!({"path": "sfx/my_coin.toml", "from": "preset:laser", "overwrite": true}),
    );
    assert!(!err);
    assert!(std::fs::read_to_string(root.join("sfx/my_coin.toml"))
        .unwrap()
        .contains("Copy of the laser preset."));

    // A bad source is reported at `from`; `recipe` and `from` together, or
    // neither, are refused.
    let (bad, err) = s.call(
        "create",
        json!({"path": "sfx/x.toml", "from": "preset:coinn"}),
    );
    assert!(err);
    assert_eq!(bad["errors"][0]["path"], "arguments.from");
    assert_eq!(bad["errors"][0]["suggestion"], "coin");
    let (bad, err) = s.call(
        "create",
        json!({"path": "sfx/x.toml", "from": "preset:coin", "recipe": {"version": 0}}),
    );
    assert!(err);
    assert_eq!(bad["errors"][0]["path"], "arguments.from");
    let (bad, err) = s.call("create", json!({"path": "sfx/x.toml"}));
    assert!(err);
    assert_eq!(bad["errors"][0]["path"], "arguments.recipe");

    // Both copies are Undo steps: undoing twice leaves no file.
    for _ in 0..2 {
        let (_, err) = s.call("edit_undo", json!({}));
        assert!(!err);
    }
    assert!(!root.join("sfx/my_coin.toml").exists());
}
