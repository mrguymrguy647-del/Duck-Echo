//! Playtest 3, as a test: a person in the Studio and an AI assistant
//! through the real MCP server, on the same sound at the same time.
//!
//! The Studio is the real app, drawn with no window (`egui_kittest`).
//! The assistant is the real `duck-echo-mcp` program, started as a child
//! process and spoken to over stdin and stdout, exactly as Claude Desktop
//! does. The two share nothing but the project folder.
//!
//! CI builds `duck-echo-mcp` before the Studio's tests. On your computer,
//! build it first: `cargo build -p duck-echo-mcp` in the folder above
//! `studio/`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use studio_app::settings::Settings;
use studio_app::{AppMenu, StudioApp, theme};

/// The newest `duck-echo-mcp` built in the main workspace.
fn mcp_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "duck-echo-mcp.exe"
    } else {
        "duck-echo-mcp"
    };
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target");
    ["release", "debug"]
        .iter()
        .map(|profile| target.join(profile).join(name))
        .filter(|p| p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .unwrap_or_else(|| {
            panic!(
                "this test needs Duck Echo's MCP server: run `cargo build -p duck-echo-mcp` in {}",
                target.parent().unwrap().display()
            )
        })
}

/// The AI assistant: the MCP server and the messages to it.
struct Assistant {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    next: u64,
}

impl Assistant {
    fn start(root: &Path) -> Assistant {
        let mut child = Command::new(mcp_binary())
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start duck-echo-mcp");
        let to = child.stdin.take().unwrap();
        let from = BufReader::new(child.stdout.take().unwrap());
        let mut a = Assistant {
            child,
            to,
            from,
            next: 1,
        };
        a.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "playtest 3", "version": "0"}}),
        );
        a.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        a
    }

    fn send(&mut self, message: Value) {
        writeln!(self.to, "{message}").unwrap();
        self.to.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            self.from.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == id {
                return reply;
            }
        }
    }

    /// Call a tool; its answer, or a panic with its error.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &reply["result"];
        assert_eq!(result["isError"], false, "{tool}: {result}");
        result["structuredContent"].clone()
    }
}

impl Drop for Assistant {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn folder(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("duck-echo-together-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Duck Echo")).unwrap();
    dir
}

fn harness(dir: &Path) -> Harness<'static, StudioApp> {
    let settings = Settings {
        project_folder: Some(dir.join("Duck Echo")),
        check_for_updates: false,
        ..Settings::default()
    };
    let app = StudioApp::new(settings, AppMenu::none());
    let render = std::env::var_os("STUDIO_SCREENSHOTS").is_some();
    let builder = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(theme::apply_brand);
    let builder = if render { builder.wgpu() } else { builder };
    let harness = builder.build_eframe(|_| app);
    harness.ctx.set_theme(egui::ThemePreference::Dark);
    harness
}

fn shoot(harness: &mut Harness<'static, StudioApp>, name: &str) {
    let Some(folder) = std::env::var_os("STUDIO_SCREENSHOTS") else {
        return;
    };
    harness.run_steps(3);
    let picture = harness.render().expect("the screen can be drawn");
    std::fs::create_dir_all(&folder).unwrap();
    picture
        .save(Path::new(&folder).join(format!("{name}.png")))
        .unwrap();
}

/// Draw frames (in real time, so the Studio's half-second look at the
/// file happens) until `done`.
fn until(harness: &mut Harness<'static, StudioApp>, what: &str, done: impl Fn(&StudioApp) -> bool) {
    let started = Instant::now();
    while !done(harness.state()) {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "waited too long for {what}; the status line says {:?}",
            harness.state().note
        );
        std::thread::sleep(Duration::from_millis(20));
        harness.run_steps(1);
    }
    harness.run_steps(2);
}

fn coin(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("Duck Echo").join("coin.toml")).unwrap()
}

#[test]
fn a_person_and_an_ai_assistant_edit_the_same_sound() {
    let dir = folder("sound");
    let project = dir.join("Duck Echo");
    let mut harness = harness(&dir);
    harness.run_steps(2);
    // The person makes a copy of the coin and opens it.
    harness.state_mut().make_copy("preset:coin", "coin");
    harness.state_mut().open_sound("coin.toml");
    harness.run_steps(3);
    let mut assistant = Assistant::start(&project);

    // "Make my coin's tail a little longer": the assistant reads the
    // file, then changes one number.
    let read = assistant.call("list_files", json!({}));
    assert!(read.to_string().contains("coin.toml"), "{read}");
    assistant.call(
        "sound_set",
        json!({"file": "coin.toml", "path": "layers[0].envelope.decay", "value": 0.5}),
    );
    // Within a second the Studio shows it, with the three-second note.
    until(&mut harness, "the assistant's change", |app| {
        app.editor
            .as_ref()
            .is_some_and(|e| e.text.contains("decay = 0.5"))
    });
    harness.get_by_label_contains("AI assistant: Set layers[0].envelope.decay to 0.5");
    shoot(&mut harness, "11-an-ai-assistant-changed-it");
    // The person's comment-free file kept the preset's comments.
    assert!(coin(&dir).starts_with("# Coin:"));

    // The person moves a slider meanwhile: one step, the person's.
    {
        let app = harness.state_mut();
        let root = duck_echo_tools::root::Root::new(&project).unwrap();
        let editor = app.editor.as_mut().unwrap();
        let now = Instant::now();
        editor
            .slide(&root, "layers[0].volume", json!(0.6), now)
            .unwrap();
        editor.release(&root, now).unwrap();
    }
    harness.run_steps(2);
    // Both changes are in the file.
    let text = coin(&dir);
    assert!(
        text.contains("decay = 0.5") && text.contains("volume = 0.6"),
        "{text}"
    );

    // The assistant sees the person's step in the shared history.
    let history = assistant.call("edit_history", json!({}));
    assert_eq!(history["steps"][0]["who"], "You", "{history}");
    assert_eq!(history["steps"][1]["who"], "AI assistant", "{history}");

    // History in the Studio lists the assistant's step.
    harness.get_by_label("History").click();
    harness.run_steps(2);
    harness.get_by_label("HISTORY");
    assert!(harness.query_all_by_label("AI assistant").count() >= 1);
    shoot(&mut harness, "12-history-with-the-assistant");

    // The assistant moves the pitch; the person presses Undo in the note:
    // the assistant's change goes, the person's stays.
    assistant.call(
        "sound_set",
        json!({"file": "coin.toml", "path": "layers[0].pitch.start", "value": "A5"}),
    );
    until(&mut harness, "the new pitch", |app| {
        app.editor
            .as_ref()
            .is_some_and(|e| e.text.contains("\"A5\""))
    });
    let note = harness.get_by_label_contains("AI assistant: Set layers[0].pitch.start");
    let undo = harness
        .query_all_by_label("Undo")
        .rfind(|n| (n.rect().center().y - note.rect().center().y).abs() < 15.0)
        .expect("Undo in the note");
    undo.click();
    harness.run_steps(3);
    let text = coin(&dir);
    assert!(
        text.contains("\"B5\"") && !text.contains("\"A5\""),
        "{text}"
    );
    assert!(text.contains("volume = 0.6"), "{text}");
    assert_eq!(harness.state().editor.as_ref().unwrap().text, text);

    // A text editor changes the file: it is noted, and becomes a step.
    std::fs::write(
        project.join("coin.toml"),
        text.replace("volume = 0.6", "volume = 0.7"),
    )
    .unwrap();
    until(&mut harness, "the outside change", |app| {
        app.editor
            .as_ref()
            .is_some_and(|e| e.text.contains("volume = 0.7"))
    });
    harness.get_by_label_contains("changed by another program");
    let history = assistant.call("edit_history", json!({}));
    assert_eq!(history["steps"][0]["who"], "Outside Duck Echo", "{history}");

    // Ctrl+Z takes the outside change back too.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert!(coin(&dir).contains("volume = 0.6"));
    drop(assistant);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_assistant_adds_a_note_to_the_open_song() {
    let dir = folder("song");
    let project = dir.join("Duck Echo");
    let mut harness = harness(&dir);
    harness.run_steps(2);
    harness
        .state_mut()
        .make_copy("example:duck_craft_day", "duck_craft_day");
    harness.state_mut().open_song("duck_craft_day.toml");
    harness.run_steps(3);
    let mut assistant = Assistant::start(&project);
    assistant.call(
        "song_add_note",
        json!({"file": "duck_craft_day.toml", "track": "keys", "bar": 2, "beat": 3, "note": "A5"}),
    );
    until(&mut harness, "the assistant's note", |app| {
        app.song_view.as_ref().is_some_and(|v| {
            v.lanes[2]
                .blocks
                .iter()
                .any(|b| b.label == "A5" && b.bar == 2 && b.beat_in_bar == 3.0)
        })
    });
    harness.get_by_label_contains("AI assistant: Add note A5 to keys in bar 2");
    harness.get_by_label("keys: A5, bar 2, beat 4");
    harness.get_by_label("History").click();
    harness.run_steps(2);
    assert!(harness.query_all_by_label("AI assistant").count() >= 1);
    drop(assistant);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn settings_offer_the_lines_to_connect_an_assistant() {
    let dir = folder("connect");
    let mut harness = harness(&dir);
    harness.run_steps(2);
    harness.get_by_label("Settings").click();
    // A window sizes and centres itself over its first frames (half a
    // second here): let it settle before clicking in it.
    harness.run_steps(30);
    harness.get_by_label("Connect an AI assistant");
    // Settings scrolls when it is taller than its box (longer folder paths
    // on another computer wrap onto more lines): bring Copy into view
    // first, as a person would.
    harness.get_by_label("Copy").scroll_to_me();
    harness.run_steps(3);
    shoot(&mut harness, "13-connect-an-ai-assistant");
    harness.get_by_label("Copy").click();
    harness.run_steps(2);
    assert!(
        harness.state().connect_copied,
        "the Copy button was pressed"
    );
    harness.get_by_label("Copied.");
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        });
    if let Some(text) = copied {
        let config: Value = serde_json::from_str(&text).unwrap();
        let args = &config["mcpServers"]["duck-echo"]["args"];
        assert_eq!(args[0], "--root");
        assert!(args[1].as_str().unwrap().ends_with("Duck Echo"));
    }
    std::fs::remove_dir_all(&dir).ok();
}
