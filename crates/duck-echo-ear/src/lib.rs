//! The bridge to the Duck Ear.
//!
//! The Duck Ear is a separate program (in Python) that listens to a sound
//! with machine-learning models and says what it hears. It is optional:
//! nothing in Duck Echo needs it, and a render never waits for it.
//!
//! This crate only starts the Ear, speaks its protocol (JSON Lines over
//! stdin and stdout, see [`protocol`]), keeps time and stops it. It knows
//! nothing about recipes or sounds; `duck-echo-tools` decides what to ask.
//! The plan is `docs/phase4-design.md`, section 3.
//!
//! ```text
//!   Ear::call ──▶ writer thread ──▶ the Ear's stdin
//!             ◀── reader thread ◀── the Ear's stdout (answers, by id)
//!                 stderr thread ◀── the Ear's stderr (its log)
//! ```
//!
//! Three threads, so the Ear can never stop Duck Echo by filling a pipe
//! nobody reads, and every wait has a deadline. When something goes wrong
//! (no answer in time, a broken line, the Ear falls over), the Ear and
//! every program it started are stopped, and the next question starts it
//! again.

pub mod protocol;
pub mod times;
mod tree;
#[cfg(windows)]
mod windows_job;

pub use protocol::{Hello, Model, Progress, PROTOCOL};
pub use times::Times;
pub use tree::{Program, PYTHON_VARIABLE};

use protocol::Message;
use serde_json::{json, Value};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long `llama-server` (the describe model) stays with no describe
/// question before the Ear stops it and frees the graphics card. Sent to
/// the Ear in `hello`, so this number lives in Duck Echo.
pub const DESCRIBE_IDLE_SECONDS: u64 = 120;

/// How much of the Ear's log (its stderr) is kept in memory: the end of it.
pub const LOG_KEPT: usize = 64 * 1024;

/// How big `ear.log` may grow before it is started again (the old one is
/// kept as `ear.log.old`).
pub const LOG_FILE_LIMIT: u64 = 1024 * 1024;

/// How to start the Ear.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The Ear program.
    pub program: Program,
    /// How long each request may take.
    pub times: Times,
    /// Sent in `hello`: see [`DESCRIBE_IDLE_SECONDS`].
    pub describe_idle_seconds: u64,
    /// Where to keep the Ear's log, if anywhere.
    pub log_file: Option<PathBuf>,
}

impl Settings {
    /// The Ear at `program`, with the usual times and no log file.
    pub fn new(program: Program) -> Settings {
        Settings {
            program,
            times: Times::default(),
            describe_idle_seconds: DESCRIBE_IDLE_SECONDS,
            log_file: None,
        }
    }
}

/// Why the Ear cannot be asked at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// There is no Ear program where Duck Echo looked.
    NotInstalled,
    /// The program is there but would not start, or exited at once.
    WillNotStart,
    /// It started but did not answer `hello` in time.
    NoHello,
    /// Its answer to `hello` was broken or missed something needed.
    BrokenHello,
    /// It speaks another protocol version.
    Protocol,
}

impl Reason {
    /// The reason as a short code, for JSON: `not_installed` ...
    pub fn code(self) -> &'static str {
        match self {
            Reason::NotInstalled => "not_installed",
            Reason::WillNotStart => "will_not_start",
            Reason::NoHello => "no_hello",
            Reason::BrokenHello => "broken_hello",
            Reason::Protocol => "protocol",
        }
    }
}

/// The Ear cannot be asked: why, in a code and in words, and the end of its
/// log when it said anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    /// Why, as a code.
    pub reason: Reason,
    /// Why, in words, with what to do.
    pub message: String,
    /// The last lines the Ear wrote to its log, if any.
    pub log: String,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// One question to the Ear failed. The Ear is still usable only after
/// [`Failure::EarError`]; after every other failure it has been stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// No answer in time. The Ear was stopped.
    Timeout {
        /// The request.
        method: String,
        /// How long Duck Echo waited.
        seconds: f64,
    },
    /// A line that is not the protocol, or an answer to another question.
    /// The Ear was stopped.
    BrokenAnswer {
        /// What was wrong.
        why: String,
        /// The first 200 characters of the line.
        line: String,
    },
    /// The Ear exited while Duck Echo waited.
    EarStopped {
        /// Its exit code, if it gave one.
        exit_code: Option<i32>,
        /// The last lines of its log.
        log: String,
    },
    /// The Ear answered with an error. It still runs.
    EarError {
        /// Its code, such as `unreadable_audio`.
        code: String,
        /// Its words.
        message: String,
    },
    /// The Ear was already stopped (by an earlier failure, or by `stop`).
    NotRunning,
}

impl Failure {
    /// The failure as a short code, for JSON: `timeout`, `broken_answer`,
    /// `ear_stopped`, `not_running`, or the Ear's own code.
    pub fn code(&self) -> &str {
        match self {
            Failure::Timeout { .. } => "timeout",
            Failure::BrokenAnswer { .. } => "broken_answer",
            Failure::EarStopped { .. } => "ear_stopped",
            Failure::EarError { code, .. } => code,
            Failure::NotRunning => "not_running",
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Timeout { method, seconds } => write!(
                f,
                "the Ear took longer than {seconds:.0} s to answer {method}; it was stopped. It starts again on the next question."
            ),
            Failure::BrokenAnswer { why, line } => write!(
                f,
                "the Ear sent a line Duck Echo cannot read ({why}): `{line}`; it was stopped"
            ),
            Failure::EarStopped { exit_code, log } => {
                match exit_code {
                    Some(code) => write!(f, "the Ear stopped (exit code {code})")?,
                    None => write!(f, "the Ear stopped")?,
                }
                if log.is_empty() {
                    write!(f, " without saying why")
                } else {
                    write!(f, "; it said: {log}")
                }
            }
            Failure::EarError { code, message } => write!(f, "the Ear said {code}: {message}"),
            Failure::NotRunning => write!(f, "the Ear is not running"),
        }
    }
}

/// What the reader thread passes on.
enum FromEar {
    /// One line, read (or why it is broken, with its start).
    Line(Result<Message, (String, String)>),
    /// The Ear's stdout closed: it has exited (or is exiting).
    Closed,
}

/// A running Ear.
pub struct Ear {
    tree: tree::Tree,
    /// Lines for the writer thread. Dropping it closes the Ear's stdin.
    to_ear: Option<Sender<String>>,
    from_ear: Receiver<FromEar>,
    log: Arc<Mutex<Log>>,
    times: Times,
    next_id: u64,
    hello: Hello,
    running: bool,
}

impl Ear {
    /// Start the Ear and greet it. On success the Ear has answered `hello`
    /// with protocol version 1.
    pub fn start(settings: &Settings) -> Result<Ear, Unavailable> {
        let program = &settings.program;
        let unavailable = |reason, message: String, log: String| Unavailable {
            reason,
            message,
            log,
        };
        // A Python Ear's file must be there (Python itself is checked by
        // starting it).
        if program.command != program.file && !program.file.is_file() {
            return Err(unavailable(
                Reason::NotInstalled,
                format!("there is no Duck Ear at {}", program.file.display()),
                String::new(),
            ));
        }
        let (tree, pipes) = tree::start(program).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound && program.command == program.file {
                unavailable(
                    Reason::NotInstalled,
                    format!("there is no Duck Ear at {}", program.file.display()),
                    String::new(),
                )
            } else {
                unavailable(
                    Reason::WillNotStart,
                    format!(
                        "the Duck Ear at {} would not start: could not run {}: {error}",
                        program.file.display(),
                        program.command.display()
                    ),
                    String::new(),
                )
            }
        })?;

        let log = Arc::new(Mutex::new(Log::new(settings.log_file.clone())));
        let (to_ear, for_writer) = mpsc::channel::<String>();
        let (from_reader, from_ear) = mpsc::channel::<FromEar>();
        spawn_writer(pipes.stdin, for_writer);
        spawn_reader(pipes.stdout, from_reader);
        spawn_log_reader(pipes.stderr, Arc::clone(&log));

        let mut ear = Ear {
            tree,
            to_ear: Some(to_ear),
            from_ear,
            log,
            times: settings.times,
            next_id: 0,
            hello: Hello {
                protocol: PROTOCOL,
                ear_version: String::new(),
                capabilities: Vec::new(),
                device: String::new(),
                device_name: None,
                models: Vec::new(),
                stub: false,
                not_available: Default::default(),
            },
            running: true,
        };
        let params = json!({
            "protocol": PROTOCOL,
            "describe_idle_seconds": settings.describe_idle_seconds,
        });
        let answer = ear.call_within("hello", params, settings.times.hello, &mut |_| {});
        let file = program.file.display();
        let result = match answer {
            Ok(result) => result,
            Err(failure) => {
                let log = ear.log_tail();
                let (reason, message) = match failure {
                    Failure::Timeout { seconds, .. } => (
                        Reason::NoHello,
                        format!("the Duck Ear at {file} did not say hello within {seconds:.0} s"),
                    ),
                    Failure::EarError { code, message } if code == "protocol" => (
                        Reason::Protocol,
                        format!(
                            "the Duck Ear at {file} speaks another protocol version ({message}); update the Ear"
                        ),
                    ),
                    Failure::EarStopped { .. } => (
                        Reason::WillNotStart,
                        format!("the Duck Ear at {file} stopped before it said hello"),
                    ),
                    other => (
                        Reason::BrokenHello,
                        format!("the Duck Ear at {file} did not say hello properly: {other}"),
                    ),
                };
                ear.stop_now();
                return Err(unavailable(reason, message, log));
            }
        };
        if result["protocol"] != json!(PROTOCOL) {
            ear.stop_now();
            return Err(unavailable(
                Reason::Protocol,
                format!(
                    "the Duck Ear at {file} speaks protocol {}, and this Duck Echo speaks {PROTOCOL}; update the Ear",
                    result["protocol"]
                ),
                String::new(),
            ));
        }
        match serde_json::from_value::<Hello>(result) {
            Ok(hello) => {
                ear.hello = hello;
                Ok(ear)
            }
            Err(error) => {
                ear.stop_now();
                Err(unavailable(
                    Reason::BrokenHello,
                    format!(
                        "the Duck Ear at {file} said hello without what Duck Echo needs: {error}"
                    ),
                    String::new(),
                ))
            }
        }
    }

    /// What the Ear said about itself.
    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// Is the Ear still running? (It may have exited by itself.)
    pub fn is_running(&mut self) -> bool {
        if self.running && self.tree.exited().is_some() {
            self.stop_now();
        }
        self.running
    }

    /// The Ear's process number, while it runs.
    pub fn process_id(&self) -> u32 {
        self.tree.id()
    }

    /// The last lines the Ear wrote to its log.
    pub fn log_tail(&self) -> String {
        self.log.lock().map(|log| log.tail()).unwrap_or_default()
    }

    /// Ask the Ear, with the time from the [`Times`] table.
    /// `on_progress` is told about each progress line.
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<Value, Failure> {
        let time = self.times.for_request(method, &params, &self.hello.device);
        self.call_within(method, params, time, on_progress)
    }

    /// Ask the Ear, and wait for its answer no longer than `time`.
    pub fn call_within(
        &mut self,
        method: &str,
        params: Value,
        time: Duration,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<Value, Failure> {
        if !self.running {
            return Err(Failure::NotRunning);
        }
        let deadline = Instant::now() + time;
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let sent = self
            .to_ear
            .as_ref()
            .is_some_and(|to_ear| to_ear.send(line).is_ok());
        if !sent {
            return Err(self.stopped_by_itself());
        }
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.from_ear.recv_timeout(left) {
                Ok(FromEar::Line(Ok(message))) if message.id() != id => {
                    let failure = Failure::BrokenAnswer {
                        why: format!(
                            "an answer to request {}, while Duck Echo waited for {id}",
                            message.id()
                        ),
                        line: String::new(),
                    };
                    self.stop_now();
                    return Err(failure);
                }
                Ok(FromEar::Line(Ok(Message::Result { result, .. }))) => return Ok(result),
                Ok(FromEar::Line(Ok(Message::Error { code, message, .. }))) => {
                    return Err(Failure::EarError { code, message })
                }
                Ok(FromEar::Line(Ok(Message::Progress { progress, .. }))) => on_progress(&progress),
                Ok(FromEar::Line(Err((why, line)))) => {
                    self.stop_now();
                    return Err(Failure::BrokenAnswer { why, line });
                }
                Ok(FromEar::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.stopped_by_itself());
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.stop_now();
                    return Err(Failure::Timeout {
                        method: method.to_string(),
                        seconds: time.as_secs_f64(),
                    });
                }
            }
        }
    }

    /// Stop the Ear politely: say `bye`, close its stdin, wait for it to
    /// exit (no longer than the `bye` time), then stop everything it
    /// started. Returns its exit code when it gave one.
    pub fn stop(mut self) -> Option<i32> {
        if !self.running {
            return self.tree.exited().flatten();
        }
        let deadline = Instant::now() + self.times.bye;
        let _ = self.call_within("bye", json!({}), self.times.bye, &mut |_| {});
        self.to_ear = None; // closes its stdin
        let code = self.tree.wait_until(deadline).flatten();
        self.stop_now();
        code
    }

    /// Stop the Ear and everything it started, at once.
    fn stop_now(&mut self) {
        self.running = false;
        self.to_ear = None;
        self.tree.stop_all();
    }

    /// The Ear went away by itself: wait a moment for its exit code and
    /// its last words, then make sure nothing it started is left.
    fn stopped_by_itself(&mut self) -> Failure {
        let exit_code = self
            .tree
            .wait_until(Instant::now() + Duration::from_secs(2))
            .flatten();
        self.stop_now();
        // The log reader may still be reading the Ear's last lines.
        std::thread::sleep(Duration::from_millis(100));
        Failure::EarStopped {
            exit_code,
            log: self.log_tail(),
        }
    }
}

impl Drop for Ear {
    fn drop(&mut self) {
        self.stop_now();
    }
}

/// Writes each line it is given to the Ear's stdin. When the sender is
/// dropped, it ends, and the Ear's stdin closes with it.
fn spawn_writer(mut stdin: std::process::ChildStdin, lines: Receiver<String>) {
    std::thread::spawn(move || {
        for line in lines {
            let written = stdin
                .write_all(line.as_bytes())
                .and_then(|()| stdin.write_all(b"\n"))
                .and_then(|()| stdin.flush());
            if written.is_err() {
                break; // the Ear has gone; the reader will say so
            }
        }
    });
}

/// Reads the Ear's stdout line by line and passes each on.
fn spawn_reader(stdout: std::process::ChildStdout, to: Sender<FromEar>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_capped_line(&mut reader, protocol::MAX_LINE) {
                Ok(None) => {
                    let _ = to.send(FromEar::Closed);
                    return;
                }
                Ok(Some(Line::Whole(bytes))) => {
                    let line = String::from_utf8_lossy(&bytes);
                    let read = protocol::read_line(&line).map_err(|why| (why, start_of(&line)));
                    if to.send(FromEar::Line(read)).is_err() {
                        return; // nobody listens any more
                    }
                }
                Ok(Some(Line::TooLong(start))) => {
                    let why = format!(
                        "a line longer than {} MB",
                        protocol::MAX_LINE / (1024 * 1024)
                    );
                    let start = start_of(&String::from_utf8_lossy(&start));
                    let _ = to.send(FromEar::Line(Err((why, start))));
                    return;
                }
                Err(_) => {
                    let _ = to.send(FromEar::Closed);
                    return;
                }
            }
        }
    });
}

/// The first 200 characters of `line`, for a message.
fn start_of(line: &str) -> String {
    let line = line.trim_end();
    match line.char_indices().nth(200) {
        Some((at, _)) => format!("{}...", &line[..at]),
        None => line.to_string(),
    }
}

/// One line read by [`read_capped_line`].
enum Line {
    /// The whole line, without its `\n`.
    Whole(Vec<u8>),
    /// The line was longer than allowed; its start.
    TooLong(Vec<u8>),
}

/// Read up to and including the next `\n`, but never keep more than `cap`
/// bytes. `None` at the end of the stream.
fn read_capped_line(reader: &mut impl BufRead, cap: usize) -> std::io::Result<Option<Line>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if line.is_empty() {
                None
            } else {
                Some(Line::Whole(line))
            });
        }
        let (take, ends) = match available.iter().position(|&byte| byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (available.len(), false),
        };
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if line.len() > cap {
            line.truncate(1000);
            return Ok(Some(Line::TooLong(line)));
        }
        if ends {
            line.pop(); // the \n
            return Ok(Some(Line::Whole(line)));
        }
    }
}

/// Reads the Ear's stderr (its log) as fast as it comes, so the Ear never
/// waits on a full pipe. Keeps the end in memory and, when asked, writes
/// it all to a file.
fn spawn_log_reader(mut stderr: std::process::ChildStderr, log: Arc<Mutex<Log>>) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(read) => match log.lock() {
                    Ok(mut log) => log.add(&chunk[..read]),
                    Err(_) => return,
                },
            }
        }
    });
}

/// The end of the Ear's log, and its file.
struct Log {
    kept: Vec<u8>,
    file: Option<PathBuf>,
}

impl Log {
    fn new(file: Option<PathBuf>) -> Log {
        Log {
            kept: Vec::new(),
            file,
        }
    }

    fn add(&mut self, bytes: &[u8]) {
        self.kept.extend_from_slice(bytes);
        if self.kept.len() > 2 * LOG_KEPT {
            let cut = self.kept.len() - LOG_KEPT;
            self.kept.drain(..cut);
        }
        if let Some(file) = &self.file {
            // Writing the log must never stop the Ear: any failure here is
            // ignored.
            let too_big = std::fs::metadata(file).is_ok_and(|m| m.len() > LOG_FILE_LIMIT);
            if too_big {
                let mut old = file.clone().into_os_string();
                old.push(".old");
                let _ = std::fs::rename(file, old);
            }
            if let Ok(mut out) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
            {
                let _ = out.write_all(bytes);
            }
        }
    }

    /// The last 20 lines kept, at most 4,000 characters.
    fn tail(&self) -> String {
        let text = String::from_utf8_lossy(&self.kept);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let last = lines[lines.len().saturating_sub(20)..].join("\n");
        match last.char_indices().rev().nth(4000) {
            Some((at, _)) => last[at..].to_string(),
            None => last,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_longer_than_the_cap_is_cut() {
        let text = format!("{}\nnext\n", "x".repeat(5000));
        let mut reader = std::io::Cursor::new(text.into_bytes());
        match read_capped_line(&mut reader, 1000).unwrap() {
            Some(Line::TooLong(start)) => assert_eq!(start.len(), 1000),
            _ => panic!("expected a line that is too long"),
        }
    }

    #[test]
    fn lines_are_read_one_by_one_and_the_last_may_lack_its_newline() {
        let mut reader = std::io::Cursor::new(b"one\ntwo\r\nthree".to_vec());
        let mut lines = Vec::new();
        while let Some(line) = read_capped_line(&mut reader, 100).unwrap() {
            match line {
                Line::Whole(bytes) => lines.push(String::from_utf8(bytes).unwrap()),
                Line::TooLong(_) => panic!("not too long"),
            }
        }
        assert_eq!(lines, ["one", "two\r", "three"]);
    }

    #[test]
    fn the_log_keeps_its_end_and_its_last_lines() {
        let mut log = Log::new(None);
        for n in 0..100_000 {
            log.add(format!("line {n}\n").as_bytes());
        }
        assert!(log.kept.len() <= 2 * LOG_KEPT);
        let tail = log.tail();
        assert_eq!(tail.lines().count(), 20);
        assert!(tail.ends_with("line 99999"), "{tail}");
    }

    #[test]
    fn a_long_line_is_shortened_for_messages() {
        assert_eq!(start_of("short\n"), "short");
        let long = "é".repeat(300);
        assert_eq!(start_of(&long).chars().count(), 203);
    }
}
