//! The Duck Ear, for agents: `analyze` with `ear: true`, `listen` and
//! `ear_status`.
//!
//! The Duck Ear is an optional program that listens to a sound with
//! machine-learning models and says what it hears (`docs/phase4-design.md`).
//! `duck-echo-ear` starts it and speaks its protocol; this module decides
//! what to ask, and turns the answers into what agents read:
//!
//! * **Where the Ear is** ([`EarConfig`]): `--ear-program PROGRAM` (the
//!   MCP server and the CLI), else `DUCK_ECHO_EAR`, else the installed Ear
//!   in the Ear's folder.
//! * **One Ear, kept** ([`EarKeeper`]): started on the first question,
//!   stopped after 10 minutes with none. The Ear itself lets go of its
//!   describe model after 2 minutes with no describe question (Duck Echo
//!   tells it so in `hello`), so Duck Echo sends `load` before every
//!   describe or ask question.
//! * **The answer cache**: every answer is kept in the Ear's folder,
//!   named by the sound's SHA-256, the question, and the Ear's and its
//!   models' versions. The same question about the same sound gets the
//!   identical answer, at once, without starting the Ear; the answer says
//!   `cached: true`.
//! * **Temporary WAVs**: the sound to hear is written as a WAV into a
//!   folder of the keeper's own, and deleted as soon as it was heard.
//!
//! The Ear's answers are advice: they are never written into a sound or its
//! facts, and every answer says so.

use crate::measure::{measure_document, Measured, Options};
use crate::root::Root;
use crate::ToolError;
use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Code, Issue};
use duck_echo_ear::{Ear, Failure, Hello, Program, Settings, Times, Unavailable};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The environment variable naming the Ear program.
pub const PROGRAM_VARIABLE: &str = "DUCK_ECHO_EAR";
/// The environment variable naming the Ear's folder (its installed files,
/// its log and the answer cache).
pub const HOME_VARIABLE: &str = "DUCK_ECHO_EAR_HOME";
/// What every answer from the Ear says about itself.
pub const ADVICE: &str =
    "The Ear's answers come from listening models and can be wrong. Check them against the numbers.";
/// From this `match` up, the score model says yes, when its Ear does not
/// say its own line in `hello` (a real Ear does: the test set sets it).
pub const YES_FROM: f64 = 0.6;
/// Below this `match`, the score model says no; in between, it is unsure.
/// Again only when the Ear does not say its own.
pub const NO_BELOW: f64 = 0.3;
/// Two matches closer than this are "about the same".
pub const SAME_WITHIN: f64 = 0.05;
/// The longest stretch of sound the Ear hears, from the start, in seconds.
pub const LONGEST_HEARD: f64 = 30.0;
/// The most texts `listen` scores in one call.
pub const MAX_LIKE: usize = 8;
/// The longest text to match or question to ask, in characters.
pub const MAX_TEXT: usize = 500;
/// How long the Ear (and its score model) stays with no question.
pub const EAR_IDLE: Duration = Duration::from_secs(10 * 60);

// ---------------------------------------------------------------------
// Where the Ear is.
// ---------------------------------------------------------------------

/// How to find and run the Ear.
#[derive(Debug, Clone)]
pub struct EarConfig {
    /// The Ear program named on the command line (`--ear-program`), if any.
    pub program: Option<PathBuf>,
    /// The Ear's folder: the installed Ear, its log, the answer cache.
    pub home: PathBuf,
    /// How long each request may take.
    pub times: Times,
    /// How long the Ear stays with no question before it is stopped.
    pub ear_idle: Duration,
    /// How long the Ear keeps its describe model with no describe
    /// question (sent to it in `hello`).
    pub describe_idle_seconds: u64,
}

impl EarConfig {
    /// The usual settings: the program named (or `DUCK_ECHO_EAR`, or the
    /// installed Ear), the Ear's folder (`DUCK_ECHO_EAR_HOME`, or the usual
    /// place), the timeouts (scaled by `DUCK_ECHO_EAR_TIMEOUT_SCALE`), 10
    /// minutes for the Ear and 2 for its describe model.
    pub fn from_environment(program: Option<PathBuf>) -> Result<EarConfig, String> {
        Ok(EarConfig {
            program: program.or_else(|| std::env::var_os(PROGRAM_VARIABLE).map(PathBuf::from)),
            home: default_home(),
            times: Times::from_environment()?,
            ear_idle: EAR_IDLE,
            describe_idle_seconds: duck_echo_ear::DESCRIBE_IDLE_SECONDS,
        })
    }

    /// The Ear that the installer puts in the Ear's folder (step 5 of the
    /// design): `env/Scripts/duck-ear.exe` on Windows,
    /// `env/bin/duck-ear` elsewhere.
    pub fn installed_program(&self) -> PathBuf {
        if cfg!(windows) {
            self.home.join("env").join("Scripts").join("duck-ear.exe")
        } else {
            self.home.join("env").join("bin").join("duck-ear")
        }
    }

    /// The Ear program, and how it was found: `named` (on the command line
    /// or in `DUCK_ECHO_EAR`) or `installed`.
    pub fn locate(&self) -> (PathBuf, &'static str) {
        match &self.program {
            Some(program) => (program.clone(), "named"),
            None => (self.installed_program(), "installed"),
        }
    }

    /// The folder of the answer cache.
    pub fn answers(&self) -> PathBuf {
        self.home.join("answers")
    }
}

/// The Ear's folder: `DUCK_ECHO_EAR_HOME`, else `%LOCALAPPDATA%\DuckEcho\ear`
/// on Windows, `$XDG_DATA_HOME/duck-echo/ear` (usually
/// `~/.local/share/duck-echo/ear`) elsewhere.
pub fn default_home() -> PathBuf {
    if let Some(home) = std::env::var_os(HOME_VARIABLE) {
        return PathBuf::from(home);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("DuckEcho"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|d| d.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .map(|d| d.join("duck-echo"))
    };
    base.unwrap_or_else(|| std::env::temp_dir().join("duck-echo"))
        .join("ear")
}

// ---------------------------------------------------------------------
// One Ear, kept.
// ---------------------------------------------------------------------

/// Why one question got no answer.
#[derive(Debug, Clone)]
pub(crate) enum Missed {
    /// The Ear cannot be asked at all.
    Unavailable(Unavailable),
    /// It was asked, and the question failed.
    Failed(Failure),
}

/// One Ear, started when first needed and stopped after [`EarConfig::ear_idle`]
/// with no question. The MCP server keeps one for as long as it runs; the
/// command line makes one per command.
pub struct EarKeeper {
    config: EarConfig,
    inner: Mutex<Inner>,
    /// The keeper's own folder for the WAVs the Ear hears.
    temp: PathBuf,
    next_sound: AtomicU64,
}

struct Inner {
    ear: Option<Ear>,
    last_used: Instant,
    last_error: Option<String>,
}

/// Each keeper gets its own folder for sounds, even in one program.
static KEEPERS: AtomicU64 = AtomicU64::new(0);

impl EarKeeper {
    /// A keeper for the Ear `config` names. It starts no Ear yet. A
    /// background thread stops the Ear after `config.ear_idle` with no
    /// question; it ends when the keeper is dropped.
    pub fn new(config: EarConfig) -> Arc<EarKeeper> {
        let number = KEEPERS.fetch_add(1, Ordering::Relaxed);
        let temp =
            std::env::temp_dir().join(format!("duck-echo-ear-{}-{number}", std::process::id()));
        let keeper = Arc::new(EarKeeper {
            config,
            inner: Mutex::new(Inner {
                ear: None,
                last_used: Instant::now(),
                last_error: None,
            }),
            temp,
            next_sound: AtomicU64::new(0),
        });
        watch_for_idle(Arc::downgrade(&keeper), keeper.config.ear_idle);
        keeper
    }

    /// Its settings.
    pub fn config(&self) -> &EarConfig {
        &self.config
    }

    /// The folder where the sounds for the Ear are written. Each is
    /// deleted once heard, and the folder when the keeper goes.
    pub fn sounds_folder(&self) -> &Path {
        &self.temp
    }

    /// Is its Ear running now?
    pub fn is_running(&self) -> bool {
        self.lock().ear.as_mut().is_some_and(|ear| ear.is_running())
    }

    /// The last lines the running Ear wrote to its log.
    pub fn log_tail(&self) -> String {
        self.lock()
            .ear
            .as_ref()
            .map(Ear::log_tail)
            .unwrap_or_default()
    }

    /// Stop the Ear politely, if it runs.
    pub fn stop(&self) {
        if let Some(ear) = self.lock().ear.take() {
            ear.stop();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A thread that panicked while holding the lock leaves the Ear in
        // a state that is still safe to use or stop.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The running Ear's hello, starting the Ear if needed.
    fn hello(&self) -> Result<Hello, Unavailable> {
        let mut inner = self.lock();
        self.running(&mut inner).map(|ear| ear.hello().clone())
    }

    /// The running Ear, started if it is not.
    fn running<'i>(&self, inner: &'i mut Inner) -> Result<&'i mut Ear, Unavailable> {
        if !inner.ear.as_mut().is_some_and(|ear| ear.is_running()) {
            inner.ear = None;
            let (program, _) = self.config.locate();
            let _ = std::fs::create_dir_all(&self.config.home);
            let settings = Settings {
                program: Program::new(&program),
                times: self.config.times,
                describe_idle_seconds: self.config.describe_idle_seconds,
                log_file: self
                    .config
                    .home
                    .is_dir()
                    .then(|| self.config.home.join("ear.log")),
            };
            match Ear::start(&settings) {
                Ok(ear) => inner.ear = Some(ear),
                Err(unavailable) => {
                    inner.last_error = Some(unavailable.message.clone());
                    return Err(unavailable);
                }
            }
        }
        inner.last_used = Instant::now();
        Ok(inner.ear.as_mut().expect("just started"))
    }

    /// Ask the Ear `method`, starting it if needed. Before a describe or
    /// ask question, `load` the describe model: the Ear may have let it go.
    fn call(&self, method: &str, params: Value) -> Result<(Value, Hello), Missed> {
        let mut inner = self.lock();
        let ear = self.running(&mut inner).map_err(Missed::Unavailable)?;
        let hello = ear.hello().clone();
        let mut answer = Ok(Value::Null);
        if matches!(method, "describe" | "ask") {
            answer = ear.call("load", json!({"roles": ["describe"]}), &mut |_| {});
        }
        if answer.is_ok() {
            answer = ear.call(method, params, &mut |_| {});
        }
        inner.last_used = Instant::now();
        match answer {
            Ok(value) => Ok((value, hello)),
            Err(failure) => {
                inner.last_error = Some(failure.to_string());
                if !matches!(failure, Failure::EarError { .. }) {
                    inner.ear = None; // it was stopped
                }
                Err(Missed::Failed(failure))
            }
        }
    }

    /// Write `buffer` (its first [`LONGEST_HEARD`] seconds) as a WAV for the
    /// Ear to hear. The file is deleted when the returned [`Sound`] is
    /// dropped.
    pub(crate) fn sound(&self, buffer: &AudioBuffer) -> Result<Sound, ToolError> {
        let channels = usize::from(buffer.channels.max(1));
        let frames = buffer.samples.len() / channels;
        let longest = (LONGEST_HEARD * f64::from(buffer.sample_rate)) as usize;
        let heard_frames = frames.min(longest);
        let heard = AudioBuffer {
            sample_rate: buffer.sample_rate,
            channels: buffer.channels,
            samples: buffer.samples[..heard_frames * channels].to_vec(),
        };
        let bytes = duck_echo_audio::encode_wav(&heard, duck_echo_audio::WavFormat::Pcm16)
            .map_err(|e| {
                ToolError::failed(
                    Code::Io,
                    format!("could not make the sound for the Ear: {e}"),
                )
            })?;
        std::fs::create_dir_all(&self.temp).map_err(|e| {
            ToolError::failed(
                Code::Io,
                format!("could not make {}: {e}", self.temp.display()),
            )
        })?;
        let number = self.next_sound.fetch_add(1, Ordering::Relaxed);
        let path = self.temp.join(format!("sound-{number}.wav"));
        std::fs::write(&path, &bytes).map_err(|e| {
            ToolError::failed(Code::Io, format!("could not write {}: {e}", path.display()))
        })?;
        Ok(Sound {
            sha256: crate::asset::sha256_hex(&bytes),
            path,
            heard: Heard {
                from_seconds: 0.0,
                to_seconds: round(
                    heard_frames as f64 / f64::from(buffer.sample_rate.max(1)),
                    3,
                ),
            },
        })
    }
}

impl Drop for EarKeeper {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

/// Stop the keeper's Ear after `idle` with no question. Checks a few times
/// a second; ends when the keeper is gone.
fn watch_for_idle(keeper: Weak<EarKeeper>, idle: Duration) {
    let tick = (idle / 4).clamp(Duration::from_millis(20), Duration::from_secs(1));
    std::thread::spawn(move || loop {
        std::thread::sleep(tick);
        let Some(keeper) = keeper.upgrade() else {
            return;
        };
        // A question in progress holds the lock; then it is not idle.
        let Ok(mut inner) = keeper.inner.try_lock() else {
            continue;
        };
        if inner.ear.is_some() && inner.last_used.elapsed() >= idle {
            if let Some(ear) = inner.ear.take() {
                ear.stop();
            }
        }
    });
}

/// A sound written for the Ear to hear. Deleted when dropped.
pub(crate) struct Sound {
    path: PathBuf,
    sha256: String,
    heard: Heard,
}

impl Drop for Sound {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------
// The answer cache.
// ---------------------------------------------------------------------

/// The cache key of a question: the sound, the question without the
/// sound's path, and the Ear's and its models' versions. `serde_json`
/// keeps an object's keys in order, so the same question always gives the
/// same text.
fn cache_key(sound: &Sound, method: &str, params: &Value, hello: &Hello) -> Value {
    let models: Vec<Value> = hello
        .models
        .iter()
        .map(|m| json!({"role": m.role, "name": m.name, "revision": m.revision}))
        .collect();
    json!({
        "sound": format!("sha256:{}", sound.sha256),
        "method": method,
        "params": params,
        "ear_version": hello.ear_version,
        "models": models,
    })
}

fn cache_file(config: &EarConfig, key: &Value) -> PathBuf {
    let name = crate::asset::sha256_hex(key.to_string().as_bytes());
    config
        .answers()
        .join(&name[..2])
        .join(format!("{name}.json"))
}

fn cached(config: &EarConfig, key: &Value) -> Option<Value> {
    let text = std::fs::read_to_string(cache_file(config, key)).ok()?;
    let record: Value = serde_json::from_str(&text).ok()?;
    // The key is kept in the file too, so a (very unlikely) clash of two
    // keys' hashes is never mistaken for an answer.
    (record["key"] == *key).then(|| record["answer"].clone())
}

fn keep(config: &EarConfig, key: &Value, answer: &Value) {
    let record = json!({"key": key, "answer": answer});
    let _ = write_whole(&cache_file(config, key), &record.to_string());
}

/// Write a file whole: into a new file next to it, then renamed over it,
/// so a reader (another Duck Echo program) never sees half of it.
fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let part = path.with_extension(format!("part{}", std::process::id()));
    std::fs::write(&part, text)?;
    std::fs::rename(&part, path)
}

/// How many answers the cache holds.
fn answers_kept(config: &EarConfig) -> usize {
    let Ok(dirs) = std::fs::read_dir(config.answers()) else {
        return 0;
    };
    dirs.flatten()
        .filter(|d| d.path().is_dir())
        .filter_map(|d| std::fs::read_dir(d.path()).ok())
        .flat_map(|files| files.flatten())
        .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
        .count()
}

// ---------------------------------------------------------------------
// Asking, through the cache.
// ---------------------------------------------------------------------

/// The questions of one call: each goes to the cache first, then to the Ear.
pub(crate) struct Listening<'k> {
    keeper: &'k EarKeeper,
    pub(crate) hello: Hello,
    /// True while every answer so far came from the cache.
    all_cached: bool,
    /// The Ear's own time for the answers (from the answers, so a cached
    /// answer gives the same number).
    seconds: f64,
    /// True to ask the Ear everything, whatever the cache holds, and keep
    /// nothing: for measuring (`ear check --fresh`).
    pub(crate) fresh: bool,
}

impl<'k> Listening<'k> {
    /// Begin: the running Ear's hello, starting the Ear if needed. Its
    /// hello names its models' exact revisions, which every cache key
    /// needs; greeting it loads no model, so a cached answer still costs
    /// no listening. (Step 2 kept the last hello in a file instead, to
    /// answer from the cache without starting the Ear; but a program can
    /// load another model without changing, and step 3's check then got
    /// one model's answers for another.)
    pub(crate) fn begin(keeper: &'k EarKeeper) -> Result<Listening<'k>, Unavailable> {
        let hello = keeper.hello()?;
        Ok(Listening {
            keeper,
            hello,
            all_cached: true,
            seconds: 0.0,
            fresh: false,
        })
    }

    pub(crate) fn ask(
        &mut self,
        sound: &Sound,
        method: &str,
        params: Value,
    ) -> Result<Value, Missed> {
        if self.fresh {
            self.all_cached = false;
            let answer = self.ask_again(sound, method, params)?;
            self.seconds += answer["seconds"].as_f64().unwrap_or(0.0);
            return Ok(answer);
        }
        let key = cache_key(sound, method, &params, &self.hello);
        let answer = match cached(&self.keeper.config, &key) {
            Some(answer) => answer,
            None => {
                self.all_cached = false;
                let mut full = params.clone();
                full["wav"] = json!(sound.path.to_string_lossy());
                let (answer, hello) = self.keeper.call(method, full)?;
                // The running Ear may be newer than a remembered hello.
                self.hello = hello;
                let key = cache_key(sound, method, &params, &self.hello);
                keep(&self.keeper.config, &key, &answer);
                answer
            }
        };
        self.seconds += answer["seconds"].as_f64().unwrap_or(0.0);
        Ok(answer)
    }

    /// Ask the Ear itself, whatever the cache holds, and keep nothing:
    /// for `ear check`, which asks twice to see the same words come back.
    pub(crate) fn ask_again(
        &mut self,
        sound: &Sound,
        method: &str,
        params: Value,
    ) -> Result<Value, Missed> {
        let mut full = params;
        full["wav"] = json!(sound.path.to_string_lossy());
        let (answer, hello) = self.keeper.call(method, full)?;
        self.hello = hello;
        Ok(answer)
    }
}

// ---------------------------------------------------------------------
// What agents read.
// ---------------------------------------------------------------------

/// What part of the sound the Ear heard, in seconds from its start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Heard {
    /// Where it started: always 0.
    pub from_seconds: f64,
    /// Where it ended: the sound's end, or [`LONGEST_HEARD`].
    pub to_seconds: f64,
}

/// One of the closest words, with its match.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Closest {
    /// The words.
    pub text: String,
    /// How well they fit, 0 to 1.
    #[serde(rename = "match")]
    pub matched: f64,
}

/// The Ear's answer about one text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LikeAnswer {
    /// The text.
    pub text: String,
    /// How much more this text fits the sound than the Ear's other words
    /// do, 0 to 1.
    #[serde(rename = "match")]
    pub matched: f64,
    /// Where the text came among the Ear's words (1 is best).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u64>,
    /// Out of how many.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub of: Option<u64>,
    /// The describe model's yes, no or partly, when it was asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// Its reason, in one sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The two put together: `yes`, `no` or `unsure`.
    pub verdict: String,
    /// Which models the verdict comes from: `score`, `describe`.
    pub from: Vec<String>,
}

/// Why something was not asked or answered.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NotAsked {
    /// Always false.
    pub available: bool,
    /// Why, in words.
    pub reason: String,
}

/// A text, or why there is none.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Said {
    /// The words.
    Text(String),
    /// Why there are none.
    Not(NotAsked),
}

/// A failed question: its code and words.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EarError {
    /// Such as `timeout`.
    pub code: String,
    /// What happened, in words.
    pub message: String,
}

/// The `ear` section of an answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EarSection {
    /// False when the Ear cannot be asked on this PC now.
    pub available: bool,
    /// When not available: why, as a code (`not_installed` ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When not available: why, in words, with what to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// When not available: the last lines of the Ear's log, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// When available: false when a question failed (see `error`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// What failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EarError>,
    /// True for the stub Ear, which does not listen.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stub: bool,
    /// What the Ear's answers are worth.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advice: Option<&'static str>,
    /// Its models, by role: `name@revision`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// Where its models run: `cpu`, `vulkan` ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// What part of the sound it heard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard: Option<Heard>,
    /// The describe model's words about the sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Said>,
    /// The Ear's own words that fit the sound best. Only when asked for
    /// (`closest: true` on `listen`): they are often wrong for short
    /// sounds (docs/ear-check.md), so they are not shown by default.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub closest: Vec<Closest>,
    /// For `analyze`: the answer about `expect.sounds_like`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sounds_like: Option<LikeAnswer>,
    /// The Ear's own time for its answers, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// True when every answer came from the answer cache: the Ear was not
    /// asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
}

impl EarSection {
    fn unavailable(u: &Unavailable) -> EarSection {
        EarSection {
            available: false,
            reason: Some(u.reason.code().to_string()),
            message: Some(unavailable_words(u)),
            log: (!u.log.is_empty()).then(|| u.log.clone()),
            ..EarSection::empty()
        }
    }

    fn empty() -> EarSection {
        EarSection {
            available: true,
            reason: None,
            message: None,
            log: None,
            ok: None,
            error: None,
            stub: false,
            advice: None,
            models: BTreeMap::new(),
            device: None,
            heard: None,
            description: None,
            closest: Vec::new(),
            sounds_like: None,
            seconds: None,
            cached: None,
        }
    }

    fn heard_by(hello: &Hello, heard: Heard) -> EarSection {
        EarSection {
            ok: Some(true),
            stub: hello.stub,
            advice: Some(ADVICE),
            models: hello
                .models
                .iter()
                .map(|m| (m.role.clone(), format!("{}@{}", m.name, m.revision)))
                .collect(),
            device: Some(hello.device.clone()),
            heard: Some(heard),
            ..EarSection::empty()
        }
    }

    fn failed(&mut self, failure: &Failure) {
        self.ok = Some(false);
        self.error = Some(EarError {
            code: failure.code().to_string(),
            message: failure.to_string(),
        });
    }

    /// The summary's words for this section, after the numbers.
    fn summary_words(&self) -> String {
        if !self.available {
            return format!(
                "Ear: not available ({})",
                self.reason.as_deref().unwrap_or("unknown")
            );
        }
        if let Some(error) = &self.error {
            return format!("Ear: failed ({})", error.code);
        }
        let who = if self.stub {
            "Ear (stub, not listening)"
        } else {
            "Ear (advice)"
        };
        match &self.sounds_like {
            Some(like) => format!(
                "{who}: sounds like \"{}\": {} ({:.2})",
                like.text, like.verdict, like.matched
            ),
            None => format!("{who}: heard"),
        }
    }
}

/// Words for an Ear that cannot be asked, with what to do.
pub(crate) fn unavailable_words(u: &Unavailable) -> String {
    match u.reason {
        duck_echo_ear::Reason::NotInstalled => format!(
            "The Duck Ear is not installed ({}). Ask the person to install it: Duck Echo Studio > Settings > Duck Ear, or duck-echo ear install.",
            u.message
        ),
        _ => u.message.clone(),
    }
}

pub(crate) fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Where yes starts and no ends, for a score model's `match`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Lines {
    /// From this match up: yes.
    pub yes_from: f64,
    /// Below this match: no. In between: unsure.
    pub no_below: f64,
}

/// The lines of this Ear's score model: its own (from `hello`), else
/// [`YES_FROM`] and [`NO_BELOW`].
pub(crate) fn lines_of(hello: &Hello) -> Lines {
    let model = hello.models.iter().find(|m| m.role == "score");
    Lines {
        yes_from: model.and_then(|m| m.yes_from).unwrap_or(YES_FROM),
        no_below: model.and_then(|m| m.no_below).unwrap_or(NO_BELOW),
    }
}

/// Why this Ear cannot do `method` (`describe`, `ask`): its own words from
/// `hello`, or a plain reason.
pub(crate) fn not_there(hello: &Hello, method: &str) -> String {
    hello
        .not_available
        .get(method)
        .cloned()
        .unwrap_or_else(|| "this Ear has no describe model".to_string())
}

/// The score model's own word for a match.
pub(crate) fn word_for_match(matched: f64, lines: Lines) -> &'static str {
    if matched >= lines.yes_from {
        "yes"
    } else if matched < lines.no_below {
        "no"
    } else {
        "unsure"
    }
}

/// The verdict on one text: the score model's word and the describe
/// model's answer must agree for a yes or a no.
pub(crate) fn verdict(matched: f64, answer: Option<&str>, lines: Lines) -> (String, Vec<String>) {
    let by_match = word_for_match(matched, lines);
    match answer {
        None => (by_match.to_string(), vec!["score".to_string()]),
        Some(answer) => {
            let both = match (by_match, answer) {
                ("yes", "yes") => "yes",
                ("no", "no") => "no",
                _ => "unsure",
            };
            (both.to_string(), vec!["score".into(), "describe".into()])
        }
    }
}

/// The question asked about one text.
pub(crate) fn question_for(text: &str) -> String {
    format!("Does this sound like {text}? Answer yes, no or partly, then say why in one sentence.")
}

/// What one round of listening found.
struct Found {
    section: EarSection,
    likes: Vec<LikeAnswer>,
    free: Option<Said>,
}

/// Listen to `buffer`: a description (unless `quick`), each of `texts`
/// scored and (unless `quick`) asked about, then the free `question`, if
/// any; and with `closest`, the Ear's own words that fit best. A failure
/// stops the listening and is reported in the section.
fn listen_to(
    keeper: &EarKeeper,
    buffer: &AudioBuffer,
    texts: &[String],
    question: Option<&str>,
    quick: bool,
    closest: bool,
) -> Result<Found, ToolError> {
    let sound = keeper.sound(buffer)?;
    let mut listening = match Listening::begin(keeper) {
        Ok(listening) => listening,
        Err(u) => {
            return Ok(Found {
                section: EarSection::unavailable(&u),
                likes: Vec::new(),
                free: None,
            })
        }
    };
    let mut section = EarSection::heard_by(&listening.hello, sound.heard);
    let mut likes = Vec::new();
    let mut free = None;
    let outcome = (|| -> Result<(), Missed> {
        let scored = listening.ask(&sound, "score", json!({"texts": texts}))?;
        if closest {
            section.closest = scored["closest"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some(Closest {
                        text: c["text"].as_str()?.to_string(),
                        matched: round(c["match"].as_f64()?, 3),
                    })
                })
                .collect();
        }
        let can_answer = !quick && listening.hello.can("ask");
        let can_describe = !quick && listening.hello.can("describe");
        if quick {
            // Not asked: no description.
        } else if can_describe {
            let described = listening.ask(&sound, "describe", json!({}))?;
            section.description = described["text"].as_str().map(|t| Said::Text(t.into()));
        } else {
            section.description = Some(Said::Not(NotAsked {
                available: false,
                reason: not_there(&listening.hello, "describe"),
            }));
        }
        for (i, text) in texts.iter().enumerate() {
            let score = &scored["scores"][i];
            let matched = round(score["match"].as_f64().unwrap_or(0.0), 3);
            let (answer, reason) = if can_answer {
                let asked = listening.ask(
                    &sound,
                    "ask",
                    json!({"question": question_for(text), "answer": "yes_no"}),
                )?;
                (
                    asked["answer"].as_str().map(str::to_string),
                    asked["reason"].as_str().map(str::to_string),
                )
            } else {
                (None, None)
            };
            let (verdict, from) = verdict(matched, answer.as_deref(), lines_of(&listening.hello));
            likes.push(LikeAnswer {
                text: text.clone(),
                matched,
                rank: score["rank"].as_u64(),
                of: score["of"].as_u64(),
                answer,
                reason,
                verdict,
                from,
            });
        }
        if let Some(question) = question {
            free = Some(if can_answer {
                let asked = listening.ask(
                    &sound,
                    "ask",
                    json!({"question": question, "answer": "free"}),
                )?;
                Said::Text(asked["text"].as_str().unwrap_or_default().to_string())
            } else {
                Said::Not(NotAsked {
                    available: false,
                    reason: if quick {
                        "quick: only the score model listened".into()
                    } else {
                        not_there(&listening.hello, "ask")
                    },
                })
            });
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => {}
        Err(Missed::Failed(failure)) => section.failed(&failure),
        Err(Missed::Unavailable(u)) => section = EarSection::unavailable(&u),
    }
    if section.available && section.error.is_none() {
        section.seconds = Some(round(listening.seconds, 3));
        section.cached = Some(listening.all_cached);
    }
    Ok(Found {
        section,
        likes,
        free,
    })
}

/// The keeper to use: the root's (the MCP server keeps one), or a new one
/// for this one command, from the environment.
pub fn keeper_of(root: &Root) -> Result<Arc<EarKeeper>, ToolError> {
    match root.ear() {
        Some(keeper) => Ok(Arc::clone(keeper)),
        None => keeper_from_environment(None),
    }
}

/// A new keeper for the Ear `program` names (or `DUCK_ECHO_EAR`, or the
/// installed Ear), with the usual settings. A bad
/// `DUCK_ECHO_EAR_TIMEOUT_SCALE` is an error that names it.
pub fn keeper_from_environment(program: Option<PathBuf>) -> Result<Arc<EarKeeper>, ToolError> {
    EarConfig::from_environment(program)
        .map(EarKeeper::new)
        .map_err(|why| {
            ToolError::issue(Issue::error(
                Code::Invalid,
                duck_echo_ear::times::SCALE_VARIABLE,
                why,
            ))
        })
}

// ---------------------------------------------------------------------
// analyze with ear: true.
// ---------------------------------------------------------------------

/// Add the Ear's `ear` section to an analysis of `buffer` (what `analyze`
/// with `ear: true` does): a description, and the
/// answer about `expect.sounds_like`, which then passes, fails or stays
/// unsure. The numbers stay as they were. The summary gets the Ear's
/// words at its end, marked as advice.
pub fn add_to_analysis(
    keeper: &EarKeeper,
    buffer: &AudioBuffer,
    measured: &mut Measured,
) -> Result<(), ToolError> {
    let sounds_like = measured.expect.as_ref().and_then(|e| {
        e.lines
            .iter()
            .find(|l| l.what == "sounds_like")
            .map(|l| l.wanted.clone())
    });
    let texts: Vec<String> = sounds_like.iter().cloned().collect();
    let found = listen_to(keeper, buffer, &texts, None, false, false)?;
    let mut section = found.section;
    section.sounds_like = found.likes.into_iter().next();

    if let Some(expect) = measured.expect.as_mut() {
        let before = crate::expect::summary_words(expect);
        match &section.sounds_like {
            Some(like) => {
                let note = if section.stub {
                    "the stub Ear's answer: it does not listen"
                } else {
                    "the Ear's advice"
                };
                expect.set_sounds_like(&like.verdict, like.matched, note);
            }
            None if sounds_like.is_some() => {
                let why = section.summary_words();
                expect.set_sounds_like_unheard(&why);
            }
            None => {}
        }
        let after = crate::expect::summary_words(expect);
        measured.analysis.summary = measured.analysis.summary.replacen(&before, &after, 1);
    }
    measured.analysis.summary.push_str("; ");
    measured.analysis.summary.push_str(&section.summary_words());
    measured.ear = Some(section);
    Ok(())
}

// ---------------------------------------------------------------------
// listen.
// ---------------------------------------------------------------------

/// What to listen to and what to ask: the arguments of `listen`.
pub struct ListenRequest<'a> {
    /// A WAV, recipe or song file, or `preset:NAME` (or `None` with
    /// `recipe`).
    pub path: Option<&'a str>,
    /// A recipe or song, inline.
    pub recipe: Option<&'a Value>,
    /// Another seed.
    pub seed: Option<u64>,
    /// Hear only these layers or tracks.
    pub solo: Vec<String>,
    /// Hear all but these.
    pub mute: Vec<String>,
    /// Texts to match.
    pub like: &'a [String],
    /// A free question.
    pub ask: Option<&'a str>,
    /// A second sound to score against the same texts.
    pub compare_with: Option<&'a str>,
    /// The score model only.
    pub quick: bool,
    /// Also give the Ear's own words that fit best (`ear.closest`).
    pub closest: bool,
}

/// One text, scored for two sounds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Compared {
    /// The text.
    pub text: String,
    /// Its match for the sound listened to.
    pub this: f64,
    /// Its match for `compare_with`.
    pub other: f64,
    /// Which sound it fits more: the file's name, or `about the same`.
    pub more_like: String,
    /// By how much (0 when about the same).
    pub by: f64,
}

/// The answer of `listen`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listened {
    /// Always true: a missing or failed Ear is said in `ear`.
    pub ok: bool,
    /// What was listened to.
    pub file: String,
    /// The Ear: available or not, its models, what it heard, the
    /// description, and the closest words when asked for.
    pub ear: EarSection,
    /// Each text of `like`, answered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub like: Vec<LikeAnswer>,
    /// The answer to `ask`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<FreeAnswer>,
    /// With `compare_with`: each text, scored for both sounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compared: Option<Vec<Compared>>,
    /// In one line.
    pub summary: String,
}

/// A free question and its answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FreeAnswer {
    /// The question.
    pub question: String,
    /// The answer, or why there is none.
    pub text: Said,
}

fn bad(prefix: &str, arg: &str, message: impl Into<String>) -> ToolError {
    ToolError::issue(Issue::error(
        Code::InvalidArgument,
        format!("{prefix}{arg}"),
        message,
    ))
}

/// Check a text to match or a question: 1 to [`MAX_TEXT`] characters.
fn check_text(prefix: &str, arg: &str, text: &str) -> Result<(), ToolError> {
    let length = text.trim().chars().count();
    if length == 0 || length > MAX_TEXT {
        return Err(ToolError::issue(
            Issue::error(
                Code::InvalidArgument,
                format!("{prefix}{arg}"),
                format!("a text of 1 to {MAX_TEXT} characters, such as \"a soft coin pickup\"; this is {length}"),
            )
            .got(text),
        ));
    }
    Ok(())
}

/// The sound a `listen` names, rendered or read, and its name.
fn sound_of(
    root: &Root,
    request: &ListenRequest<'_>,
    prefix: &str,
) -> Result<(AudioBuffer, String), ToolError> {
    let options = Options {
        solo: request.solo.clone(),
        mute: request.mute.clone(),
        ..Options::default()
    };
    match (request.path, request.recipe) {
        (Some(_), Some(_)) => Err(bad(
            prefix,
            "recipe",
            "give either `recipe` or `path`, not both",
        )),
        (None, None) => Err(ToolError::issue(Issue::error(
            Code::MissingField,
            format!("{prefix}path"),
            "give `path` (a WAV, recipe or song file, or preset:NAME) or `recipe`",
        ))),
        (None, Some(recipe)) => {
            let doc = crate::load_document_value(recipe, &root.song_files(String::new()))
                .map_err(|e| e.at(&format!("{prefix}recipe")))?;
            let measured = measure_document(&doc, request.seed, &options, prefix)?;
            Ok((measured.buffer, "the recipe".into()))
        }
        (Some(path), None) => {
            let at = format!("{prefix}path");
            let file = root.read(&at, path)?;
            if crate::is_recipe_ref(&file) {
                let doc = crate::load_document(&file, &root.song_files(root.folder_of(&file)))
                    .map_err(|e| e.at(&at))?;
                let measured = measure_document(&doc, request.seed, &options, prefix)?;
                Ok((measured.buffer, path.to_string()))
            } else {
                options.for_wav(prefix)?;
                if request.seed.is_some() {
                    return Err(bad(prefix, "seed", "a WAV or OGG file has no seed"));
                }
                let buffer = crate::load_audio(&file).map_err(|e| e.at(&at))?;
                Ok((buffer, path.to_string()))
            }
        }
    }
}

/// `listen`: ask the Ear about a sound. Never an error because of the Ear:
/// a missing or failed Ear is said in the answer's `ear`.
pub fn listen(
    root: &Root,
    request: &ListenRequest<'_>,
    prefix: &str,
) -> Result<Listened, ToolError> {
    if request.like.len() > MAX_LIKE {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                format!("{prefix}like"),
                format!(
                    "at most {MAX_LIKE} texts at once; this is {}",
                    request.like.len()
                ),
            )
            .range(0.0, MAX_LIKE as f64),
        ));
    }
    for (i, text) in request.like.iter().enumerate() {
        check_text(prefix, &format!("like[{i}]"), text)?;
    }
    if let Some(question) = request.ask {
        check_text(prefix, "ask", question)?;
    }
    if request.compare_with.is_some() && request.like.is_empty() {
        return Err(ToolError::issue(
            Issue::error(
                Code::InvalidArgument,
                format!("{prefix}compare_with"),
                "`compare_with` scores both sounds against the texts in `like`: give at least one",
            )
            .suggest("\"like\": [\"a soft coin pickup\"]"),
        ));
    }
    let (buffer, file) = sound_of(root, request, prefix)?;
    let other = match request.compare_with {
        Some(path) => {
            let other = ListenRequest {
                path: Some(path),
                recipe: None,
                seed: None,
                solo: Vec::new(),
                mute: Vec::new(),
                ..*request
            };
            let (buffer, _) = sound_of(root, &other, prefix)
                .map_err(|e| e.at(&format!("{prefix}compare_with")))?;
            Some((buffer, path.to_string()))
        }
        None => None,
    };

    let keeper = keeper_of(root)?;
    let found = listen_to(
        &keeper,
        &buffer,
        request.like,
        request.ask,
        request.quick,
        request.closest,
    )?;
    let mut section = found.section;
    let mut compared = None;
    if let (Some((other_buffer, other_file)), true) = (other, section.error.is_none()) {
        if section.available {
            let other_found = listen_to(&keeper, &other_buffer, request.like, None, true, false)?;
            match other_found.section.error {
                Some(error) => section.error = Some(error),
                None => {
                    if section.cached == Some(true) {
                        section.cached = other_found.section.cached;
                    }
                    compared = Some(
                        found
                            .likes
                            .iter()
                            .zip(&other_found.likes)
                            .map(|(this, that)| {
                                let by = round(this.matched - that.matched, 3);
                                let more_like = if by.abs() < SAME_WITHIN {
                                    "about the same".to_string()
                                } else if by > 0.0 {
                                    file.clone()
                                } else {
                                    other_file.clone()
                                };
                                Compared {
                                    text: this.text.clone(),
                                    this: this.matched,
                                    other: that.matched,
                                    more_like,
                                    by: if by.abs() < SAME_WITHIN {
                                        0.0
                                    } else {
                                        by.abs()
                                    },
                                }
                            })
                            .collect(),
                    );
                }
            }
            if section.error.is_some() {
                section.ok = Some(false);
            }
        }
    }
    let answer = request
        .ask
        .zip(found.free)
        .map(|(question, text)| FreeAnswer {
            question: question.to_string(),
            text,
        });
    let summary = listen_summary(&section, &found.likes, compared.as_deref());
    Ok(Listened {
        ok: true,
        file,
        ear: section,
        like: found.likes,
        answer,
        compared,
        summary,
    })
}

fn listen_summary(
    section: &EarSection,
    likes: &[LikeAnswer],
    compared: Option<&[Compared]>,
) -> String {
    if !section.available || section.error.is_some() {
        let mut words = section.summary_words();
        if let Some(message) = section
            .message
            .as_deref()
            .or(section.error.as_ref().map(|e| e.message.as_str()))
        {
            words.push_str(": ");
            words.push_str(message);
        }
        return words;
    }
    let who = if section.stub {
        "Ear (stub, not listening)"
    } else {
        "Ear (advice)"
    };
    let mut parts: Vec<String> = likes
        .iter()
        .map(|l| format!("\"{}\": {} ({:.2})", l.text, l.verdict, l.matched))
        .collect();
    // Only when asked for with `closest: true`.
    if let Some(closest) = section.closest.first() {
        parts.push(format!(
            "closest \"{}\" ({:.2})",
            closest.text, closest.matched
        ));
    }
    for c in compared.into_iter().flatten() {
        parts.push(if c.by == 0.0 {
            format!("both about the same for \"{}\"", c.text)
        } else {
            format!(
                "{} is more like \"{}\" (by {:.2})",
                c.more_like, c.text, c.by
            )
        });
    }
    if parts.is_empty() {
        parts.push("heard".to_string());
    }
    format!("{who}: {}", parts.join(", "))
}

// ---------------------------------------------------------------------
// ear_status.
// ---------------------------------------------------------------------

/// `ear_status`: is there an Ear, which models, which device, how many
/// answers are kept. Starts the Ear if it is not running (only to say
/// hello: it loads no model).
pub fn status(root: &Root) -> Result<Value, ToolError> {
    let keeper = keeper_of(root)?;
    let config = keeper.config();
    let (program, found_by) = config.locate();
    let hello = keeper.hello();
    let last_error = keeper.lock().last_error.clone();
    let mut answer = json!({
        "ok": true,
        "available": hello.is_ok(),
        "program": program.to_string_lossy(),
        "found_by": found_by,
        "home": config.home.to_string_lossy(),
        "running": keeper.is_running(),
        "answers_kept": answers_kept(config),
        "ear_idle_seconds": config.ear_idle.as_secs(),
        "describe_idle_seconds": config.describe_idle_seconds,
    });
    let summary = match &hello {
        Ok(hello) => {
            answer["ear_version"] = json!(hello.ear_version);
            answer["capabilities"] = json!(hello.capabilities);
            answer["device"] = json!(hello.device);
            answer["device_name"] = json!(hello.device_name);
            if let Some(memory) = hello.device_memory_mb {
                answer["device_memory_mb"] = json!(memory);
            }
            answer["models"] = hello
                .models
                .iter()
                .map(|m| {
                    let mut model = json!({
                        "role": m.role, "name": m.name, "revision": m.revision, "loaded": m.loaded,
                    });
                    if let Some(size) = &m.size {
                        model["size"] = json!(size);
                    }
                    model
                })
                .collect();
            if !hello.not_available.is_empty() {
                answer["not_available"] = json!(hello.not_available);
            }
            answer["stub"] = json!(hello.stub);
            format!(
                "The Duck Ear {} is ready{}, on {}: it can {}.",
                hello.ear_version,
                if hello.stub {
                    " (a stub: it does not listen)"
                } else {
                    ""
                },
                hello.device_name.as_deref().unwrap_or(&hello.device),
                hello.capabilities.join(", ")
            )
        }
        Err(u) => {
            answer["reason"] = json!(u.reason.code());
            answer["message"] = json!(unavailable_words(u));
            if !u.log.is_empty() {
                answer["log"] = json!(u.log);
            }
            format!(
                "The Duck Ear is not available ({}): {}",
                u.reason.code(),
                unavailable_words(u)
            )
        }
    };
    if let Some(error) = last_error {
        answer["last_error"] = json!(error);
    }
    answer["summary"] = json!(summary);
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_models_must_agree_for_a_yes_or_a_no() {
        let lines = Lines {
            yes_from: YES_FROM,
            no_below: NO_BELOW,
        };
        assert_eq!(verdict(0.8, Some("yes"), lines).0, "yes");
        assert_eq!(verdict(0.1, Some("no"), lines).0, "no");
        assert_eq!(verdict(0.8, Some("no"), lines).0, "unsure");
        assert_eq!(verdict(0.1, Some("yes"), lines).0, "unsure");
        assert_eq!(verdict(0.45, Some("yes"), lines).0, "unsure");
        assert_eq!(verdict(0.8, Some("partly"), lines).0, "unsure");
        // The score alone, when the describe model was not asked.
        assert_eq!(
            verdict(0.8, None, lines),
            ("yes".to_string(), vec!["score".to_string()])
        );
        assert_eq!(verdict(0.29, None, lines).0, "no");
        assert_eq!(verdict(0.3, None, lines).0, "unsure");
        assert_eq!(verdict(0.6, None, lines).0, "yes");
        // A model's own lines move them.
        let own = Lines {
            yes_from: 0.731,
            no_below: 0.269,
        };
        assert_eq!(verdict(0.7, None, own).0, "unsure");
        assert_eq!(verdict(0.28, None, own).0, "unsure");
        assert_eq!(verdict(0.75, None, own).0, "yes");
    }

    #[test]
    fn the_ears_folder_follows_its_variable() {
        // Only checks the shape: the variable itself is set by tests that
        // run in their own process (see tests/ear.rs).
        let home = default_home();
        assert!(home.ends_with("ear"), "{}", home.display());
    }
}
