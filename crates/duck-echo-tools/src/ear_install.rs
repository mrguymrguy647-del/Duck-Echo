//! Installing, updating and removing the Duck Ear (docs/phase4-design.md,
//! section 5). The command line (`duck-echo ear install`) and the Studio
//! (Settings > Duck Ear) share this.
//!
//! Only a person starts it, after seeing the size: never an AI assistant,
//! and never Duck Echo by itself. What is installed is the Ear with
//! **scores only**: no describe model passed Duck Echo's listening test
//! (docs/ear-check.md), so none comes with it.
//!
//! The steps:
//!
//! 1. **uv**, a small program that installs Python and packages: its
//!    wheel from PyPI (a zip file), checked by SHA-256, with the `uv`
//!    program unpacked from it.
//! 2. **The Ear's own code**, which is built into Duck Echo (the files of
//!    `ear/duck_ear/`, with `pyproject.toml` and `uv.lock`), written into
//!    the Ear's folder. So the Ear always matches the Duck Echo that
//!    installed it.
//! 3. **A private Python and the Ear's packages**: `uv sync --frozen`
//!    installs Python into the Ear's folder and every package of
//!    `uv.lock`, each checked by its hash. No Python already on the PC is
//!    used or changed.
//! 4. **The score model**, from a Duck Echo release, each file checked
//!    against the release's `SHA256SUMS`.
//! 5. **A self-test**: the Ear is started and must find that a coin sounds
//!    more like "a coin pickup" than like "an explosion". Only then is
//!    `ear.json` written, which is what "installed" means.
//!
//! Installing again is how the Ear is updated: every file that is already
//! there and right is kept, so the models are not downloaded again.

use crate::ear::{EarConfig, EarKeeper, Listening, Missed};
use crate::ear_net::{self, megabytes, Cancel, Expected, NetError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The list of everything the installer downloads (`ear/manifest.toml`).
const MANIFEST: &str = include_str!("../../../ear/manifest.toml");

/// The Ear's own code, built in: each file's place in the Ear's `package`
/// folder, and its text.
const PACKAGE: &[(&str, &str)] = &[
    (
        "pyproject.toml",
        include_str!("../../../ear/pyproject.toml"),
    ),
    ("uv.lock", include_str!("../../../ear/uv.lock")),
    ("README.md", include_str!("../../../ear/README.md")),
    (
        "duck_ear/__init__.py",
        include_str!("../../../ear/duck_ear/__init__.py"),
    ),
    (
        "duck_ear/audio.py",
        include_str!("../../../ear/duck_ear/audio.py"),
    ),
    (
        "duck_ear/catalog.py",
        include_str!("../../../ear/duck_ear/catalog.py"),
    ),
    (
        "duck_ear/describe.py",
        include_str!("../../../ear/duck_ear/describe.py"),
    ),
    (
        "duck_ear/guard.py",
        include_str!("../../../ear/duck_ear/guard.py"),
    ),
    (
        "duck_ear/score.py",
        include_str!("../../../ear/duck_ear/score.py"),
    ),
    (
        "duck_ear/server.py",
        include_str!("../../../ear/duck_ear/server.py"),
    ),
    (
        "duck_ear/words.py",
        include_str!("../../../ear/duck_ear/words.py"),
    ),
    (LAUNCHER, "from duck_ear.server import main\n\nmain()\n"),
];

/// The file that starts the installed Ear (run by the Ear's own Python).
pub const LAUNCHER: &str = "duck_ear_run.py";

/// The environment variable that points the score model's download at
/// another address (a test server, or a mirror) instead of the release in
/// the manifest.
pub const SCORE_URL_VARIABLE: &str = "DUCK_ECHO_EAR_SCORE_URL";

/// The environment variable naming another address for uv's file (a test
/// server, or a test that must not reach the internet). Its SHA-256 is
/// still checked against the manifest's.
pub const UV_URL_VARIABLE: &str = "DUCK_ECHO_EAR_UV_URL";

/// The environment variable naming a Python to use instead of installing
/// a private one. For building and testing where the private Python
/// cannot be downloaded; an Ear installed this way stops working if that
/// Python is removed.
pub const PYTHON_VARIABLE: &str = "DUCK_ECHO_EAR_INSTALL_PYTHON";

// ---------------------------------------------------------------------
// The manifest.
// ---------------------------------------------------------------------

/// `ear/manifest.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The manifest's own version.
    pub version: u32,
    /// The private Python's version, such as `3.12`.
    pub python: String,
    /// uv.
    pub uv: Uv,
    /// The score model.
    pub score: Score,
    /// llama.cpp.
    pub llama: Llama,
    /// The describe models that can be tried.
    pub describe: Vec<Describe>,
}

/// uv: its version, and its file for each kind of PC.
#[derive(Debug, Clone, Deserialize)]
pub struct Uv {
    /// Its version.
    pub version: String,
    /// The file for each kind of PC (`windows-x86_64`, `linux-x86_64`).
    #[serde(flatten)]
    pub files: BTreeMap<String, UvFile>,
}

/// uv's file for one kind of PC.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UvFile {
    /// Where to download it.
    pub url: String,
    /// Its size.
    pub bytes: u64,
    /// Its SHA-256.
    pub sha256: String,
    /// The uv program's name inside it.
    pub inside: String,
}

/// The score model: where its release is and which files it has.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Score {
    /// Its name.
    pub name: String,
    /// The original model's revision.
    pub revision: String,
    /// The folder of its release (each file's address is this plus its
    /// name).
    pub release: String,
    /// The name of the release's list of SHA-256s.
    pub sums: String,
    /// Its files.
    pub files: Vec<String>,
    /// About how much there is to download.
    pub bytes: u64,
}

/// llama.cpp: where its releases are listed, and how its files are named.
#[derive(Debug, Clone, Deserialize)]
pub struct Llama {
    /// The address of the newest release's description.
    pub releases: String,
    /// About how much there is to download.
    pub bytes: u64,
    /// For each kind of PC, how its files' names end.
    #[serde(flatten)]
    pub builds: BTreeMap<String, LlamaBuilds>,
}

/// How llama.cpp's files for one kind of PC are named.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlamaBuilds {
    /// The build for any graphics card (Vulkan).
    pub vulkan: String,
    /// The build for the CPU only.
    pub cpu: String,
}

/// A describe model.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Describe {
    /// Its name, which is also its folder's name.
    pub name: String,
    /// The Ear's size it belongs to: `full`, `large`.
    pub size: String,
    /// The graphics card memory (MB) it needs before it is offered.
    #[serde(default)]
    pub needs_card_mb: Option<u64>,
    /// Its Hugging Face repository.
    pub repo: String,
    /// The pinned revision.
    pub revision: String,
    /// Its licence.
    pub licence: String,
    /// How it did in the listening test, in one line.
    pub listening_test: String,
    /// The context size for llama-server.
    pub context: u32,
    /// The sample rate it hears at.
    pub sample_rate: u32,
    /// The model's own file.
    pub model: ModelFile,
    /// The part that hears.
    pub mmproj: ModelFile,
}

/// One file of a model.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Its SHA-256.
    pub sha256: String,
}

impl Describe {
    /// How much there is to download.
    pub fn bytes(&self) -> u64 {
        self.model.bytes + self.mmproj.bytes
    }
}

/// The manifest built into this Duck Echo.
pub fn manifest() -> Manifest {
    toml::from_str(MANIFEST).expect("ear/manifest.toml is checked by a test")
}

/// The kind of PC this is, as the manifest names it, or why the Ear
/// cannot be installed here.
pub fn platform() -> Result<&'static str, InstallError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x86_64"),
        ("linux", "x86_64") => Ok("linux-x86_64"),
        (os, arch) => Err(InstallError::new(
            "unsupported",
            format!("The Duck Ear's installer knows Windows and Linux on 64-bit Intel or AMD PCs; this is {os} on {arch}."),
        )),
    }
}

// ---------------------------------------------------------------------
// Errors, in words.
// ---------------------------------------------------------------------

/// Why the Ear was not installed (or removed), for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallError {
    /// A short code for programs: `no_internet`, `no_room`,
    /// `wrong_sha256`, `not_found`, `cancelled`, `unsupported`,
    /// `will_not_run`, `python`, `self_test`, `in_use`, `failed`.
    pub code: String,
    /// What happened and what to do, in words.
    pub message: String,
}

impl InstallError {
    fn new(code: &str, message: impl Into<String>) -> InstallError {
        InstallError {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<NetError> for InstallError {
    fn from(error: NetError) -> InstallError {
        InstallError::new(error.code(), error.to_string())
    }
}

impl From<InstallError> for crate::ToolError {
    fn from(error: InstallError) -> crate::ToolError {
        crate::ToolError::failed(duck_echo_core::Code::Io, error.message)
    }
}

// ---------------------------------------------------------------------
// What is said while it works.
// ---------------------------------------------------------------------

/// What the installer is doing, for a progress line or bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Event<'a> {
    /// A new step began.
    Step(&'a str),
    /// A file is arriving.
    Download {
        /// The file, in words.
        what: &'a str,
        /// Bytes so far.
        done: u64,
        /// Of how many, when known.
        of: Option<u64>,
    },
    /// A model is being asked the test set's questions.
    Question {
        /// Questions asked so far.
        done: usize,
        /// Of how many.
        of: usize,
    },
}

/// Where each part of the Ear is, inside its folder.
#[derive(Debug, Clone)]
pub struct Places {
    /// The Ear's folder.
    pub home: PathBuf,
}

impl Places {
    /// The places inside `home`.
    pub fn new(home: impl Into<PathBuf>) -> Places {
        Places { home: home.into() }
    }

    /// The uv program.
    pub fn uv(&self) -> PathBuf {
        self.home
            .join("uv")
            .join(if cfg!(windows) { "uv.exe" } else { "uv" })
    }

    /// The Ear's own code.
    pub fn package(&self) -> PathBuf {
        self.home.join("package")
    }

    /// The file that starts the Ear.
    pub fn launcher(&self) -> PathBuf {
        self.package().join(LAUNCHER)
    }

    /// The Ear's private Python, in its environment.
    pub fn python(&self) -> PathBuf {
        let env = self.home.join("env");
        if cfg!(windows) {
            env.join("Scripts").join("python.exe")
        } else {
            env.join("bin").join("python")
        }
    }

    /// The score model's folder.
    pub fn score_model(&self) -> PathBuf {
        self.home.join("models").join("score")
    }

    /// `ear.json`: written when the self-test passed.
    pub fn record(&self) -> PathBuf {
        self.home.join("ear.json")
    }
}

/// What `ear.json` says about an installed Ear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    /// The record's own version.
    pub version: u32,
    /// The Duck Echo that installed it.
    pub duck_echo: String,
    /// The Ear's version.
    pub ear_version: String,
    /// uv's version.
    pub uv: String,
    /// The Python it runs in.
    pub python: String,
    /// The score model, `name@revision`.
    pub score_model: String,
    /// What it can do: `score` (and `describe`, `ask` with a describe
    /// model).
    pub capabilities: Vec<String>,
}

/// The installed Ear in `home`, if there is one: its `ear.json`, and the
/// program and Python it names still there.
pub fn installed(home: &Path) -> Option<Installed> {
    let places = Places::new(home);
    let record = std::fs::read_to_string(places.record()).ok()?;
    let record: Installed = serde_json::from_str(&record).ok()?;
    (places.launcher().is_file() && places.python().is_file()).then_some(record)
}

/// Is the Ear's own code in `home` older or newer than this Duck Echo's?
/// Then installing again brings it up to date.
pub fn needs_update(home: &Path) -> bool {
    let places = Places::new(home);
    installed(home).is_some()
        && PACKAGE.iter().any(|(name, text)| {
            std::fs::read_to_string(places.package().join(name))
                .ok()
                .as_deref()
                != Some(*text)
        })
}

/// What an install will download and use, to show before asking.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// The Ear's folder.
    pub home: String,
    /// About how much there is to download, in bytes.
    pub download_bytes: u64,
    /// About how much it takes on disk once installed, in bytes.
    pub disk_bytes: u64,
    /// The parts, each with about how much there is to download.
    pub parts: Vec<(String, u64)>,
    /// In words, for the question "Install?".
    pub summary: String,
}

/// About how much the private Python and the Ear's packages are to
/// download, and how much the programs take once unpacked (measured:
/// docs/phase4-design.md, section 5).
const PYTHON_AND_PACKAGES_DOWNLOAD: u64 = 110_000_000;
const PROGRAMS_ON_DISK: u64 = 350_000_000;

/// What installing into `home` will download: less when the score model
/// is already there.
pub fn plan(home: &Path) -> Result<Plan, InstallError> {
    let manifest = manifest();
    let uv = &manifest.uv.files[platform()?];
    let places = Places::new(home);
    let have_model = manifest
        .score
        .files
        .iter()
        .all(|file| places.score_model().join(file).is_file());
    let mut parts = vec![
        (format!("uv {}", manifest.uv.version), uv.bytes),
        (
            format!("Python {} and the Ear's packages", manifest.python),
            PYTHON_AND_PACKAGES_DOWNLOAD,
        ),
    ];
    if !have_model {
        parts.push((
            format!("the score model ({})", manifest.score.name),
            manifest.score.bytes,
        ));
    }
    let download_bytes = parts.iter().map(|(_, bytes)| bytes).sum();
    let disk_bytes = PROGRAMS_ON_DISK + manifest.score.bytes;
    let summary = format!(
        "The Duck Ear (scores only): about {} to download, about {} on disk, into {}. It listens on this PC only; nothing is sent anywhere.",
        megabytes(download_bytes),
        megabytes(disk_bytes),
        home.display()
    );
    Ok(Plan {
        home: home.display().to_string(),
        download_bytes,
        disk_bytes,
        parts,
        summary,
    })
}

// ---------------------------------------------------------------------
// Installing.
// ---------------------------------------------------------------------

fn io_error(what: &str, path: &Path, error: std::io::Error) -> InstallError {
    InstallError::new(
        "failed",
        format!("Cannot {what} {}: {error}", path.display()),
    )
}

// ---------------------------------------------------------------------
// A stopped install, and a broken one.
// ---------------------------------------------------------------------

/// Who started an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Starter {
    /// `duck-echo ear install`.
    CommandLine,
    /// Duck Echo Studio, Settings > Duck Ear.
    Studio,
}

impl Starter {
    fn word(self) -> &'static str {
        match self {
            Starter::CommandLine => "command_line",
            Starter::Studio => "studio",
        }
    }
}

/// `installing.json`: there while an install runs, and after one that did
/// not finish. A running install writes the time into it every few
/// seconds, so another program can tell it still runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Marker {
    version: u32,
    /// `studio` or `command_line`.
    by: String,
    /// When it began and when it last said it was running, in seconds
    /// since 1970.
    started: u64,
    beat: u64,
    /// How it ended without finishing: `person` (Stop, Ctrl+C) or `error`.
    /// None while it runs, or when its program was closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stopped: Option<String>,
    /// What went wrong, when `stopped` is `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// After this long with no word from a running install, it was closed.
const BEAT_EVERY: Duration = Duration::from_secs(3);
const CLOSED_AFTER: u64 = 20;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn marker_file(home: &Path) -> PathBuf {
    home.join("installing.json")
}

fn read_marker(home: &Path) -> Option<Marker> {
    serde_json::from_str(&std::fs::read_to_string(marker_file(home)).ok()?).ok()
}

fn write_marker(home: &Path, marker: &Marker) {
    let text = serde_json::to_string_pretty(marker).expect("serializes") + "\n";
    let part = marker_file(home).with_extension(format!("part{}", std::process::id()));
    if std::fs::write(&part, text).is_ok() {
        let _ = std::fs::rename(&part, marker_file(home));
    }
}

/// How an install that did not finish ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Its program was closed (the Studio, or the terminal) while it ran.
    Closed,
    /// A person pressed Stop. (Ctrl+C on the command line closes the
    /// program, so it reads as [`Ended::Closed`].)
    ByPerson,
    /// It failed: why, in words.
    Failed(String),
}

/// An install that began and did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// Who started it.
    pub by: Starter,
    /// How it ended.
    pub ended: Ended,
    /// How much is already in the Ear's folder, in bytes: an install
    /// carries on from there.
    pub bytes_here: u64,
}

/// What is in the Ear's folder.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// No Ear, and no install begun.
    NotInstalled,
    /// An install runs now, in another program (or this one).
    Installing(Starter),
    /// An install began and did not finish.
    Stopped(Stopped),
    /// It was installed and something is missing now: what, in words.
    Broken(String),
    /// Installed.
    Installed(Installed),
}

/// What is in the Ear's folder `home`: installed, broken, stopped half
/// way, being installed, or nothing.
pub fn state(home: &Path) -> State {
    let places = Places::new(home);
    if let Some(marker) = read_marker(home) {
        let by = if marker.by == "studio" {
            Starter::Studio
        } else {
            Starter::CommandLine
        };
        let ended = match marker.stopped.as_deref() {
            Some("person") => Ended::ByPerson,
            Some(_) => Ended::Failed(marker.error.unwrap_or_default()),
            None if now().saturating_sub(marker.beat) < CLOSED_AFTER => {
                return State::Installing(by)
            }
            None => Ended::Closed,
        };
        return State::Stopped(Stopped {
            by,
            ended,
            bytes_here: disk_use(home),
        });
    }
    let Some(record) = std::fs::read_to_string(places.record())
        .ok()
        .and_then(|text| serde_json::from_str::<Installed>(&text).ok())
    else {
        return State::NotInstalled;
    };
    let missing = |what: &str, path: &Path| {
        State::Broken(format!(
            "{what} is missing ({}). An antivirus may have removed it: allow the Duck Ear's folder, then Repair (it installs again, and keeps what is still there).",
            path.display()
        ))
    };
    if !places.python().is_file() {
        return missing("The Ear's private Python", &places.python());
    }
    if !places.launcher().is_file() {
        return missing("The Ear's own code", &places.launcher());
    }
    if let Some(file) = manifest()
        .score
        .files
        .iter()
        .map(|file| places.score_model().join(file))
        .find(|path| !path.is_file())
    {
        return missing("A file of the score model", &file);
    }
    State::Installed(record)
}

/// Install the Ear into `home` (or bring it up to date, or repair it),
/// for `by`. `events` hears what is being done; `cancel` stops it at the
/// next download or step. Files already there and checked are kept, so
/// an install that stopped carries on where it was.
///
/// While it runs, `installing.json` in `home` says so; it goes when the
/// install finishes, and stays when it does not, saying how it ended.
pub fn install(
    home: &Path,
    by: Starter,
    events: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<Installed, InstallError> {
    let uv_url = std::env::var(UV_URL_VARIABLE).ok();
    install_from(home, by, uv_url.as_deref(), events, cancel)
}

/// [`install`], with uv's file from `uv_url` instead of the manifest's
/// address (its SHA-256 is still the manifest's). For tests that must not
/// reach the internet.
pub fn install_from(
    home: &Path,
    by: Starter,
    uv_url: Option<&str>,
    events: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<Installed, InstallError> {
    std::fs::create_dir_all(home).map_err(|e| io_error("make the folder", home, e))?;
    let mut marker = Marker {
        version: 1,
        by: by.word().to_string(),
        started: now(),
        beat: now(),
        stopped: None,
        error: None,
    };
    write_marker(home, &marker);
    let running = std::sync::atomic::AtomicBool::new(true);
    let done = std::thread::scope(|scope| {
        let beat = marker.clone();
        let running = &running;
        scope.spawn(move || {
            let mut beat = beat;
            while running.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                if now() >= beat.beat + BEAT_EVERY.as_secs() {
                    beat.beat = now();
                    if running.load(std::sync::atomic::Ordering::Relaxed) {
                        write_marker(home, &beat);
                    }
                }
            }
        });
        let done = install_steps(home, uv_url, events, cancel);
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        done
    });
    match &done {
        Ok(_) => {
            let _ = std::fs::remove_file(marker_file(home));
            crate::ear_home::forget_wanted(home);
        }
        Err(error) => {
            marker.beat = now();
            if error.code == "cancelled" {
                marker.stopped = Some("person".into());
            } else {
                marker.stopped = Some("error".into());
                marker.error = Some(error.message.clone());
            }
            write_marker(home, &marker);
        }
    }
    done
}

fn install_steps(
    home: &Path,
    uv_url: Option<&str>,
    events: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<Installed, InstallError> {
    let manifest = manifest();
    let places = Places::new(home);
    let uv_file = &manifest.uv.files[platform()?];
    std::fs::create_dir_all(home).map_err(|e| io_error("make the folder", home, e))?;
    // Not installed until the self-test passes again.
    let _ = std::fs::remove_file(places.record());
    let stopped = |cancel: &Cancel| {
        if cancel.is_cancelled() {
            Err(InstallError::from(NetError::Cancelled))
        } else {
            Ok(())
        }
    };

    // 1. uv.
    events(Event::Step(
        "Getting uv, which installs Python and packages",
    ));
    let downloads = home.join("downloads");
    let wheel = downloads.join("uv.zip");
    let have_uv = uv_says(&places.uv()).is_some_and(|said| said.contains(&manifest.uv.version));
    if !have_uv {
        ear_net::download(
            uv_url.unwrap_or(&uv_file.url),
            &wheel,
            &Expected {
                bytes: Some(uv_file.bytes),
                sha256: Some(uv_file.sha256.clone()),
            },
            &mut |done, of| {
                events(Event::Download {
                    what: "uv",
                    done,
                    of,
                })
            },
            cancel,
        )?;
        let folder = places.uv();
        let folder = folder.parent().expect("uv has a folder");
        ear_net::unzip(&wheel, folder, Some(&uv_file.inside))
            .map_err(|why| InstallError::new("failed", why))?;
        if uv_says(&places.uv()).is_none() {
            return Err(InstallError::new(
                "will_not_run",
                format!(
                    "uv was unpacked to {} but will not start. An antivirus may have blocked or removed it: allow that file (or the Duck Ear's folder), then try again.",
                    places.uv().display()
                ),
            ));
        }
    }
    stopped(cancel)?;

    // 2. The Ear's own code.
    events(Event::Step("Writing the Ear's own code"));
    let package = places.package();
    let _ = std::fs::remove_dir_all(package.join("duck_ear"));
    for (name, text) in PACKAGE {
        let file = package.join(name);
        let folder = file.parent().expect("a file has a folder");
        std::fs::create_dir_all(folder).map_err(|e| io_error("make the folder", folder, e))?;
        std::fs::write(&file, text).map_err(|e| io_error("write", &file, e))?;
    }

    // 3. A private Python, and the packages of uv.lock.
    events(Event::Step(
        "Installing a private Python and the Ear's packages (a minute or two)",
    ));
    let mut uv = Command::new(places.uv());
    uv.current_dir(&package)
        .args(["sync", "--frozen", "--no-dev", "--no-install-project"])
        .env("UV_PROJECT_ENVIRONMENT", home.join("env"))
        .env("UV_PYTHON_INSTALL_DIR", home.join("python"))
        .env("UV_CACHE_DIR", home.join("uv").join("cache"))
        .env("UV_NO_CONFIG", "1")
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .env_remove("VIRTUAL_ENV")
        .stdin(Stdio::null());
    match std::env::var_os(PYTHON_VARIABLE) {
        Some(python) => {
            uv.arg("--python")
                .arg(python)
                .env("UV_PYTHON_PREFERENCE", "system");
        }
        None => {
            uv.arg("--python")
                .arg(&manifest.python)
                .env("UV_PYTHON_PREFERENCE", "only-managed");
        }
    }
    let out = uv
        .output()
        .map_err(|e| io_error("start", &places.uv(), e))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let last: Vec<&str> = said.trim().lines().rev().take(6).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        let lower = said.to_lowercase();
        let (code, lead) = if ["dns", "connect", "timed out", "network", "request failed"]
            .iter()
            .any(|word| lower.contains(word))
        {
            ("no_internet", "Could not download Python or a package. Check the internet connection and try again: what was already downloaded is kept.")
        } else if lower.contains("no space") {
            ("no_room", "There is not enough room on the disk for Python and the Ear's packages. Free some space and try again.")
        } else {
            (
                "python",
                "uv could not install Python and the Ear's packages.",
            )
        };
        return Err(InstallError::new(
            code,
            format!("{lead} uv said: {}", last.join(" / ")),
        ));
    }
    if !places.python().is_file() {
        return Err(InstallError::new(
            "python",
            format!(
                "uv finished, but there is no Python at {}. An antivirus may have removed it: allow the Duck Ear's folder, then try again.",
                places.python().display()
            ),
        ));
    }
    stopped(cancel)?;

    // 4. The score model.
    events(Event::Step("Getting the score model"));
    let base = std::env::var(SCORE_URL_VARIABLE).unwrap_or_else(|_| manifest.score.release.clone());
    let base = base.trim_end_matches('/');
    let sums = ear_net::fetch_text(&format!("{base}/{}", manifest.score.sums)).map_err(|error| {
        match error {
            NetError::NotFound { url } => InstallError::new(
                "not_found",
                format!("The score model is not published yet: {url} is not there. It is made by the \"Ear\" workflow of Duck Echo's repository (docs/phase4-design.md, step 5)."),
            ),
            other => other.into(),
        }
    })?;
    let sums = read_sums(&sums);
    let folder = places.score_model();
    for file in &manifest.score.files {
        let Some(sha256) = sums.get(file) else {
            return Err(InstallError::new(
                "failed",
                format!(
                    "The score model's {} does not list {file}.",
                    manifest.score.sums
                ),
            ));
        };
        let what = format!("the score model: {file}");
        ear_net::download(
            &format!("{base}/{file}"),
            &folder.join(file),
            &Expected {
                bytes: None,
                sha256: Some(sha256.clone()),
            },
            &mut |done, of| {
                events(Event::Download {
                    what: &what,
                    done,
                    of,
                })
            },
            cancel,
        )?;
    }
    stopped(cancel)?;

    // 5. The self-test.
    events(Event::Step("Trying the Ear on a coin sound"));
    let hello = self_test(home)?;
    let record = Installed {
        version: 1,
        duck_echo: env!("CARGO_PKG_VERSION").to_string(),
        ear_version: hello.ear_version.clone(),
        uv: manifest.uv.version.clone(),
        python: manifest.python.clone(),
        score_model: hello
            .models
            .iter()
            .find(|m| m.role == "score")
            .map(|m| format!("{}@{}", m.name, m.revision))
            .unwrap_or_default(),
        capabilities: hello.capabilities.clone(),
    };
    let text = serde_json::to_string_pretty(&record).expect("serializes") + "\n";
    std::fs::write(places.record(), text).map_err(|e| io_error("write", &places.record(), e))?;
    // The downloads and uv's cache are not needed again.
    let _ = std::fs::remove_dir_all(&downloads);
    let _ = std::fs::remove_dir_all(home.join("uv").join("cache"));
    events(Event::Step("The Duck Ear is installed"));
    Ok(record)
}

/// What `uv --version` says, if the program at `uv` starts.
fn uv_says(uv: &Path) -> Option<String> {
    let out = Command::new(uv)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// A `SHA256SUMS` file, read: each file's name and its SHA-256.
pub fn read_sums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let sha256 = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (sha256.len() == 64).then(|| (name.to_string(), sha256.to_lowercase()))
        })
        .collect()
}

/// Start the installed Ear in `home` and let it listen to a coin: it must
/// say the coin is more like "a coin pickup" than like "an explosion".
/// Returns what the Ear says about itself.
pub fn self_test(home: &Path) -> Result<duck_echo_ear::Hello, InstallError> {
    let failed = |why: String| {
        InstallError::new(
            "self_test",
            format!(
                "The Ear was installed but failed its test: {why}. Its log is {}.",
                home.join("ear.log").display()
            ),
        )
    };
    // The installed Ear in this folder, whatever `DUCK_ECHO_EAR` names.
    let config = EarConfig {
        program: Some(Places::new(home).launcher()),
        home: home.to_path_buf(),
        ..EarConfig::from_environment(None).map_err(|why| failed(why.to_string()))?
    };
    let keeper = EarKeeper::new(config);
    let mut listening =
        Listening::begin(&keeper).map_err(|u| failed(crate::ear::unavailable_words(&u)))?;
    let coin = crate::load_audio(Path::new("preset:coin"))
        .map_err(|_| failed("the coin preset would not render".into()))?;
    let sound = keeper
        .sound(&coin)
        .map_err(|_| failed("the coin could not be written".into()))?;
    let texts = ["a coin pickup", "an explosion"];
    let answer = listening
        .ask_again(&sound, "score", json!({ "texts": texts }))
        .map_err(|missed| match missed {
            Missed::Unavailable(u) => failed(crate::ear::unavailable_words(&u)),
            Missed::Failed(f) => failed(f.to_string()),
        })?;
    let matched = |i: usize| answer["scores"][i]["match"].as_f64().unwrap_or(-1.0);
    keeper.stop();
    if matched(0) > matched(1) {
        Ok(listening.hello.clone())
    } else {
        Err(failed(format!(
            "a coin scored {:.2} for \"a coin pickup\" and {:.2} for \"an explosion\"",
            matched(0),
            matched(1)
        )))
    }
}

// ---------------------------------------------------------------------
// Removing.
// ---------------------------------------------------------------------

/// Everything the Ear keeps in its folder. Only these are ever deleted.
const OWN: &[&str] = &[
    "ear.json",
    "ear.log",
    "llama-server.log",
    "uv",
    "python",
    "env",
    "package",
    "models",
    "llama",
    "answers",
    "downloads",
    "testset",
    "installing.json",
    "device.txt",
    "wanted.json",
];

/// How many bytes the Ear takes in `home` now.
pub fn disk_use(home: &Path) -> u64 {
    fn size(path: &Path) -> u64 {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return 0;
        };
        if meta.is_dir() {
            std::fs::read_dir(path)
                .map(|entries| entries.flatten().map(|e| size(&e.path())).sum())
                .unwrap_or(0)
        } else {
            meta.len()
        }
    }
    OWN.iter().map(|name| size(&home.join(name))).sum()
}

/// Remove the Ear from `home`: its programs, its models and its answers.
/// Only the Ear's own files and folders are deleted; anything else in the
/// folder stays. Returns how many bytes were freed.
pub fn remove(home: &Path) -> Result<u64, InstallError> {
    let freed = disk_use(home);
    for name in OWN {
        let path = home.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let done = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        done.map_err(|e| {
            InstallError::new(
                "in_use",
                format!(
                    "Cannot remove {}: {e}. Close what may be using the Ear (Duck Echo Studio, an AI assistant's Duck Echo server), then try again.",
                    path.display()
                ),
            )
        })?;
    }
    // The folder itself goes too, if nothing else is in it.
    let _ = std::fs::remove_dir(home);
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_reads_and_names_this_kind_of_pc() {
        let manifest = manifest();
        assert_eq!(manifest.version, 1);
        for kind in ["windows-x86_64", "linux-x86_64"] {
            let uv = &manifest.uv.files[kind];
            assert!(
                uv.url.starts_with("https://files.pythonhosted.org/"),
                "{kind}"
            );
            assert_eq!(uv.sha256.len(), 64);
            assert!(uv.inside.contains(&manifest.uv.version));
            assert!(manifest.llama.builds.contains_key(kind));
        }
        assert!(manifest.score.files.contains(&"model.json".to_string()));
        let large = manifest
            .describe
            .iter()
            .find(|m| m.size == "large")
            .unwrap();
        assert_eq!(large.needs_card_mb, Some(16000));
        assert_eq!(megabytes(large.bytes()), "19.9 GB");
    }

    #[test]
    fn every_file_of_the_ears_package_is_built_in() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ear/duck_ear");
        let mut on_disk: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".py"))
            .map(|name| format!("duck_ear/{name}"))
            .collect();
        on_disk.sort();
        let mut built_in: Vec<String> = PACKAGE
            .iter()
            .map(|(name, _)| name.to_string())
            .filter(|name| name.starts_with("duck_ear/"))
            .collect();
        built_in.sort();
        assert_eq!(built_in, on_disk, "add the new file to PACKAGE");
    }

    #[test]
    fn sums_are_read_with_and_without_a_star() {
        let a = "a".repeat(64);
        let sums = read_sums(&format!(
            "{a}  audio.onnx\n{}  *text.onnx\nnot a line\n",
            "B".repeat(64)
        ));
        assert_eq!(sums["audio.onnx"], a);
        assert_eq!(sums["text.onnx"], "b".repeat(64));
        assert_eq!(sums.len(), 2);
    }

    #[test]
    fn the_plan_says_the_size_before_anything_is_downloaded() {
        let home = std::env::temp_dir().join(format!("duck-echo-ear-plan-{}", std::process::id()));
        let plan = plan(&home).unwrap();
        assert!(plan.download_bytes > 800_000_000, "{plan:?}");
        assert!(plan.summary.contains("scores only"));
        assert!(plan.summary.contains("to download"));
        assert!(!home.exists(), "a plan writes nothing");
    }

    fn empty(name: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("duck-echo-ear-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        home
    }

    fn marker(by: &str, beat: u64, stopped: Option<&str>, error: Option<&str>) -> Marker {
        Marker {
            version: 1,
            by: by.into(),
            started: beat,
            beat,
            stopped: stopped.map(str::to_string),
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn a_stopped_install_says_how_it_ended() {
        let home = empty("state-stopped");
        assert_eq!(state(&home), State::NotInstalled);
        std::fs::create_dir_all(home.join("downloads")).unwrap();
        std::fs::write(home.join("downloads/uv.zip.part"), vec![0u8; 500]).unwrap();
        // Its program wrote the time a moment ago: it still runs.
        write_marker(&home, &marker("studio", now(), None, None));
        assert_eq!(state(&home), State::Installing(Starter::Studio));
        // No word for a minute: its program was closed.
        write_marker(&home, &marker("studio", now() - 60, None, None));
        let State::Stopped(stopped) = state(&home) else {
            panic!("{:?}", state(&home))
        };
        assert_eq!(stopped.by, Starter::Studio);
        assert_eq!(stopped.ended, Ended::Closed);
        assert!(stopped.bytes_here >= 500, "{stopped:?}");
        write_marker(
            &home,
            &marker("command_line", now() - 60, Some("person"), None),
        );
        assert!(matches!(
            state(&home),
            State::Stopped(Stopped {
                by: Starter::CommandLine,
                ended: Ended::ByPerson,
                ..
            })
        ));
        write_marker(
            &home,
            &marker("studio", now(), Some("error"), Some("The disk is full.")),
        );
        assert!(
            matches!(state(&home), State::Stopped(Stopped { ended: Ended::Failed(why), .. }) if why == "The disk is full.")
        );
        // Remove takes the marker too.
        remove(&home).unwrap();
        assert_eq!(state(&home), State::NotInstalled);
    }

    #[test]
    fn an_install_that_is_stopped_leaves_a_note_and_one_that_is_cancelled_says_so() {
        let home = empty("state-cancel");
        let cancel = Cancel::new();
        cancel.cancel();
        let error = install(&home, Starter::Studio, &mut |_| {}, &cancel).unwrap_err();
        assert_eq!(error.code, "cancelled");
        let marker = read_marker(&home).expect("the note stays");
        assert_eq!(marker.by, "studio");
        assert_eq!(marker.stopped.as_deref(), Some("person"));
        assert!(matches!(
            state(&home),
            State::Stopped(Stopped {
                ended: Ended::ByPerson,
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_broken_ear_says_what_is_missing() {
        let home = empty("state-broken");
        let places = Places::new(&home);
        let record = Installed {
            version: 1,
            duck_echo: "0.1.0".into(),
            ear_version: "0.4.0".into(),
            uv: "0.13.9".into(),
            python: "3.12".into(),
            score_model: "laion-clap-general@ada0c23a36c4".into(),
            capabilities: vec!["score".into()],
        };
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(places.record(), serde_json::to_string(&record).unwrap()).unwrap();
        let State::Broken(why) = state(&home) else {
            panic!("{:?}", state(&home))
        };
        assert!(why.contains("private Python is missing"), "{why}");
        assert!(why.contains("Repair"), "{why}");
        for file in [places.launcher(), places.python()] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"").unwrap();
        }
        let State::Broken(why) = state(&home) else {
            panic!("{:?}", state(&home))
        };
        assert!(why.contains("score model"), "{why}");
        std::fs::create_dir_all(places.score_model()).unwrap();
        for file in &manifest().score.files {
            std::fs::write(places.score_model().join(file), b"x").unwrap();
        }
        assert_eq!(state(&home), State::Installed(record));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn remove_deletes_the_ears_own_things_and_nothing_else() {
        let home =
            std::env::temp_dir().join(format!("duck-echo-ear-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("models/score")).unwrap();
        std::fs::write(home.join("models/score/audio.onnx"), vec![0u8; 1000]).unwrap();
        std::fs::write(home.join("ear.json"), "{}").unwrap();
        std::fs::write(home.join("my notes.txt"), "keep me").unwrap();
        assert!(
            installed(&home).is_none(),
            "an ear.json alone is not an Ear"
        );
        assert_eq!(disk_use(&home), 1002);
        assert_eq!(remove(&home).unwrap(), 1002);
        assert!(!home.join("models").exists());
        assert!(!home.join("ear.json").exists());
        assert_eq!(
            std::fs::read_to_string(home.join("my notes.txt")).unwrap(),
            "keep me"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
