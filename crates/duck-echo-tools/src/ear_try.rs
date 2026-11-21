//! Trying a bigger describe model on this PC: download it, measure it on
//! the whole test set, show the result. The command line (`duck-echo ear
//! try large`) and the Studio (Settings > Duck Ear > "Try a bigger
//! model") share this.
//!
//! The Ear ships with scores only: no describe model has passed Duck
//! Echo's listening test (docs/ear-check.md). A model that might pass on
//! a big graphics card can be tried here. After a person says yes, this:
//!
//! 1. downloads llama.cpp's newest build for this PC (the one for any
//!    graphics card, Vulkan), if there is no `llama-server` yet;
//! 2. downloads the model's two files from Hugging Face, each checked by
//!    its SHA-256;
//! 3. runs the full describe check (`ear check --part describe --fresh`)
//!    with that model, and reports every yes and no and the times.
//!
//! **It never makes the model the Ear's.** Only when the model passes
//! the bars is the person offered to use it ([`use_model`]), and only
//! they decide.

use crate::ear::{EarConfig, EarKeeper};
use crate::ear_cards::Card;
use crate::ear_check::{self, CheckReport, Part, Watch};
use crate::ear_install::{self, Describe, Event, InstallError, Places};
use crate::ear_net::{self, megabytes, Cancel, Expected, NetError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The environment variable that points at another list of llama.cpp's
/// releases (a test server) instead of GitHub's.
pub const LLAMA_RELEASES_VARIABLE: &str = "DUCK_ECHO_LLAMA_RELEASES";

/// The environment variable that points the describe models' downloads
/// at another site (a test server, or a mirror) instead of Hugging Face.
pub const MODELS_URL_VARIABLE: &str = "DUCK_ECHO_EAR_MODELS_URL";

/// The test set, built in: each file's place under the Ear's `testset`
/// folder, and its text. The same files as `ear/testset` and the example
/// recipes it names.
const TESTSET: &[(&str, &str)] = &[
    (
        "ear/testset/testset.toml",
        include_str!("../../../ear/testset/testset.toml"),
    ),
    (
        "ear/testset/quick.toml",
        include_str!("../../../ear/testset/quick.toml"),
    ),
    (
        "ear/testset/coin_soft.toml",
        include_str!("../../../ear/testset/coin_soft.toml"),
    ),
    (
        "ear/testset/coin_harsh.toml",
        include_str!("../../../ear/testset/coin_harsh.toml"),
    ),
    (
        "ear/testset/noise.toml",
        include_str!("../../../ear/testset/noise.toml"),
    ),
    (
        "ear/testset/calm_loop.toml",
        include_str!("../../../ear/testset/calm_loop.toml"),
    ),
    (
        "ear/testset/tense_loop.toml",
        include_str!("../../../ear/testset/tense_loop.toml"),
    ),
    (
        "examples/recipes/chime.toml",
        include_str!("../../../examples/recipes/chime.toml"),
    ),
    (
        "examples/recipes/coin.toml",
        include_str!("../../../examples/recipes/coin.toml"),
    ),
    (
        "examples/recipes/explosion.toml",
        include_str!("../../../examples/recipes/explosion.toml"),
    ),
    (
        "examples/recipes/fanfare.toml",
        include_str!("../../../examples/recipes/fanfare.toml"),
    ),
    (
        "examples/recipes/jump.toml",
        include_str!("../../../examples/recipes/jump.toml"),
    ),
    (
        "examples/recipes/laser.toml",
        include_str!("../../../examples/recipes/laser.toml"),
    ),
    (
        "examples/recipes/siren.toml",
        include_str!("../../../examples/recipes/siren.toml"),
    ),
];

fn error(code: &str, message: impl Into<String>) -> InstallError {
    InstallError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// The describe model `which` names: a size (`full`, `large`) or a
/// model's own name.
pub fn model_named(which: &str) -> Result<Describe, InstallError> {
    let models = ear_install::manifest().describe;
    let names: Vec<String> = models
        .iter()
        .map(|m| format!("{} ({})", m.size, m.name))
        .collect();
    models
        .into_iter()
        .find(|m| m.size == which || m.name == which)
        .ok_or_else(|| {
            error(
                "unknown_model",
                format!(
                    "There is no describe model `{which}`. There are: {}.",
                    names.join(", ")
                ),
            )
        })
}

/// May `model` be offered on a PC with these graphics cards? `Err` says
/// why not, in words.
pub fn offered(model: &Describe, cards: &[Card]) -> Result<(), String> {
    let Some(needs) = model.needs_card_mb else {
        return Ok(());
    };
    match cards.first() {
        Some(card) if card.memory_mb >= needs => Ok(()),
        Some(card) => Err(format!(
            "{} needs a graphics card with {} GB or more; this PC's is {}",
            model.name,
            needs / 1000,
            card.words()
        )),
        None => Err(format!(
            "{} needs a graphics card with {} GB or more; none was found on this PC",
            model.name,
            needs / 1000
        )),
    }
}

/// The `llama-server` the Ear will use: the program
/// `DUCK_EAR_LLAMA_SERVER` names, else one in the Ear's `llama` folder
/// (or a folder inside it). The same rule as the Ear's own.
pub fn llama_server(home: &Path) -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("DUCK_EAR_LLAMA_SERVER") {
        let named = PathBuf::from(named);
        return named.is_file().then_some(named);
    }
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    fn find(folder: &Path, name: &str, deep: u32) -> Option<PathBuf> {
        let direct = folder.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        if deep == 0 {
            return None;
        }
        let mut inside: Vec<PathBuf> = std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        inside.sort();
        inside
            .iter()
            .find_map(|folder| find(folder, name, deep - 1))
    }
    find(&home.join("llama"), name, 4)
}

/// The folder a describe model is kept in.
pub fn model_folder(home: &Path, model: &Describe) -> PathBuf {
    home.join("models").join(&model.name)
}

/// What trying a model will download, to show before asking.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TryPlan {
    /// The model's name.
    pub model: String,
    /// How much there is to download, in bytes.
    pub download_bytes: u64,
    /// The parts, each with how much there is to download.
    pub parts: Vec<(String, u64)>,
    /// In words, for the question.
    pub summary: String,
}

/// What trying `which` in `home` will download: less for what is already
/// there. An error when the Ear is not installed, or the model is not
/// offered on a PC with these graphics `cards` ([`crate::ear_cards::cards`]).
pub fn try_plan(
    home: &Path,
    which: &str,
    quick: bool,
    cards: &[Card],
) -> Result<TryPlan, InstallError> {
    let model = model_named(which)?;
    if ear_install::installed(home).is_none() {
        return Err(error(
            "not_installed",
            "Install the Duck Ear first (Settings > Duck Ear > Install, or `duck-echo ear install`): a describe model is tried beside its score model.",
        ));
    }
    offered(&model, cards).map_err(|why| error("no_card", format!("{why}.")))?;
    let folder = model_folder(home, &model);
    let mut parts = Vec::new();
    if llama_server(home).is_none() {
        parts.push((
            "llama.cpp, which runs the model".to_string(),
            ear_install::manifest().llama.bytes,
        ));
    }
    for file in [&model.model, &model.mmproj] {
        let there = std::fs::metadata(folder.join(&file.name)).is_ok_and(|m| m.len() == file.bytes);
        if !there {
            parts.push((file.name.clone(), file.bytes));
        }
    }
    let download_bytes: u64 = parts.iter().map(|(_, bytes)| bytes).sum();
    let questions = if quick { "10 sounds" } else { "all 41 sounds" };
    let summary = format!(
        "Try {} ({}, {}): {} to download into {}. Then it is asked about {questions} of Duck Echo's test set (minutes on a graphics card, far longer on a CPU), and the results are shown. It does not change which model the Ear uses. Its listening test so far: {}.",
        model.name,
        model.size,
        model.licence,
        if download_bytes == 0 { "nothing".to_string() } else { megabytes(download_bytes) },
        home.display(),
        model.listening_test
    );
    Ok(TryPlan {
        model: model.name,
        download_bytes,
        parts,
        summary,
    })
}

/// What trying a model found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tried {
    /// The model's name.
    pub model: String,
    /// True when it was asked about the whole test set (not the quick 10).
    pub whole_set: bool,
    /// True when it reached every bar a describe model must reach, on the
    /// whole test set.
    pub passed: bool,
    /// The bars it missed, in words.
    pub missed: Vec<String>,
    /// In one or two sentences.
    pub summary: String,
    /// The whole check: what `ear-check.json` holds.
    pub report: CheckReport,
}

/// What is kept in a model's folder about its last check
/// (`last-check.json`): whether it may be offered for use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastCheck {
    /// The model, `name@revision`, as the Ear reported it.
    pub model: String,
    /// Asked about the whole test set?
    pub whole_set: bool,
    /// Reached every bar?
    pub passed: bool,
    /// The bars it missed.
    pub missed: Vec<String>,
}

/// The last check of the describe model `which` in `home`, if it was
/// tried here.
pub fn last_check(home: &Path, which: &str) -> Option<LastCheck> {
    let model = model_named(which).ok()?;
    let text = std::fs::read_to_string(model_folder(home, &model).join("last-check.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// The bars a describe model must reach before it is offered for use:
/// its own measures (yes on true texts, no on wrong texts, silence,
/// noise) and the same words twice. The verdict's measures are reported
/// but do not count here: they also depend on the score model.
fn bars_missed(report: &CheckReport) -> Vec<String> {
    let Some(describe) = &report.describe else {
        return vec!["the describe model was not measured".to_string()];
    };
    describe
        .measures
        .iter()
        .filter(|m| (m.name.starts_with("Describe:") || m.name == "Same words twice") && !m.passed)
        .map(|m| {
            format!(
                "{}: {} of {} ({:.0}%, the bar is {:.0}%)",
                m.name,
                m.right,
                m.of,
                m.share * 100.0,
                m.bar * 100.0
            )
        })
        .collect()
}

fn net(error: NetError) -> InstallError {
    error.into()
}

/// The releases in what the list of releases answered: GitHub gives a
/// list, newest first; one release alone is read as a list of one.
fn releases(listed: &Value) -> Vec<&Value> {
    match listed.as_array() {
        Some(list) => list.iter().collect(),
        None => vec![listed],
    }
}

/// The newest release that holds a build for this PC, with that file.
///
/// The newest release is not always one with programs in it: llama.cpp's
/// `v1.0.0` holds only a note naming the nightly build it was made from.
/// So this walks down the list to the first release that has the file.
fn newest_build<'a>(listed: &'a Value, wanted: &str) -> Option<(String, &'a Value)> {
    let zip = format!("{wanted}.zip");
    let tar = format!("{wanted}.tar.gz");
    releases(listed)
        .into_iter()
        .filter(|release| release["draft"].as_bool() != Some(true))
        .find_map(|release| {
            let asset = release["assets"].as_array()?.iter().find(|asset| {
                let name = asset["name"].as_str().unwrap_or_default();
                name.ends_with(&zip) || name.ends_with(&tar)
            })?;
            let tag = release["tag_name"].as_str().unwrap_or("?").to_string();
            Some((tag, asset))
        })
}

/// The names of the releases looked through, for an error message.
fn release_names(listed: &Value) -> String {
    let names: Vec<&str> = releases(listed)
        .into_iter()
        .filter_map(|release| release["tag_name"].as_str())
        .collect();
    if names.is_empty() {
        "the list is empty".to_string()
    } else {
        names.join(", ")
    }
}

/// Get llama.cpp's newest build for this PC into the Ear's `llama`
/// folder: the Vulkan build when there is a graphics card, else the CPU
/// build.
fn get_llama(
    home: &Path,
    with_card: bool,
    events: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<PathBuf, InstallError> {
    let manifest = ear_install::manifest();
    let builds = &manifest.llama.builds[ear_install::platform()?];
    let wanted = if with_card {
        &builds.vulkan
    } else {
        &builds.cpu
    };
    let address = std::env::var(LLAMA_RELEASES_VARIABLE).unwrap_or(manifest.llama.releases.clone());
    events(Event::Step("Looking up llama.cpp's newest build"));
    let listed = ear_net::fetch_text(&address).map_err(net)?;
    let listed: Value = serde_json::from_str(&listed).map_err(|_| {
        error(
            "failed",
            format!("{address} did not answer with a list of files."),
        )
    })?;
    let (tag, asset) = newest_build(&listed, wanted).ok_or_else(|| {
        error(
            "not_found",
            format!("None of llama.cpp's newest releases ({}) has a file whose name ends in {wanted}.zip or {wanted}.tar.gz. Download llama.cpp for this PC by hand from https://github.com/ggml-org/llama.cpp/releases and unpack it into {}.", release_names(&listed), home.join("llama").display()),
        )
    })?;
    let name = asset["name"].as_str().unwrap_or_default().to_string();
    let url = asset["browser_download_url"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let expected = Expected {
        bytes: asset["size"].as_u64(),
        // GitHub lists each file's SHA-256 as `sha256:<hex>`.
        sha256: asset["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_string),
    };
    let archive = home.join("downloads").join(&name);
    let what = format!("llama.cpp {tag}");
    ear_net::download(
        &url,
        &archive,
        &expected,
        &mut |done, of| {
            events(Event::Download {
                what: &what,
                done,
                of,
            })
        },
        cancel,
    )
    .map_err(net)?;
    events(Event::Step("Unpacking llama.cpp"));
    let folder = home.join("llama");
    let _ = std::fs::remove_dir_all(&folder);
    ear_net::unpack(&archive, &folder).map_err(|why| error("failed", why))?;
    let _ = std::fs::write(folder.join("release.txt"), format!("{tag}\n{name}\n"));
    let _ = std::fs::remove_file(&archive);
    let server = llama_server(home).ok_or_else(|| {
        error(
            "failed",
            format!(
                "{name} was unpacked into {}, but there is no llama-server in it.",
                folder.display()
            ),
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755));
    }
    let starts = Command::new(&server)
        .arg("--version")
        .env("LD_LIBRARY_PATH", server.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .output();
    if !starts.is_ok_and(|out| out.status.success()) {
        return Err(error(
            "will_not_run",
            format!(
                "llama-server was unpacked to {} but will not start. On Windows an antivirus may have blocked it; on Linux the Vulkan build needs the Vulkan library (the package libvulkan1).",
                server.display()
            ),
        ));
    }
    Ok(server)
}

/// The cards `llama-server --list-devices` names, with their memory.
fn llama_cards(server: &Path) -> Vec<Card> {
    let Ok(out) = Command::new(server)
        .arg("--list-devices")
        .env("LD_LIBRARY_PATH", server.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            // "  Vulkan0: NVIDIA GeForce RTX 4090 (24564 MiB, 24000 MiB free)"
            let (_, rest) = line.trim().split_once(": ")?;
            let (name, memory) = rest.rsplit_once(" (")?;
            let memory_mb = memory.split(" MiB").next()?.trim().parse().ok()?;
            Some(Card {
                name: name.to_string(),
                memory_mb,
            })
        })
        .collect()
}

/// Write the built-in test set into the Ear's folder, and return the test
/// set file to use.
fn write_testset(home: &Path, quick: bool) -> Result<PathBuf, InstallError> {
    let folder = home.join("testset");
    for (name, text) in TESTSET {
        let file = folder.join(name);
        let parent = file.parent().expect("a file has a folder");
        std::fs::create_dir_all(parent)
            .and_then(|()| std::fs::write(&file, text))
            .map_err(|e| error("failed", format!("Cannot write {}: {e}", file.display())))?;
    }
    Ok(folder
        .join("ear/testset")
        .join(if quick { "quick.toml" } else { "testset.toml" }))
}

/// Try the describe model `which`: download what is missing, then run
/// the describe check with it (the whole test set, or with `quick` the 10
/// sounds of the quick set). `cards` are this PC's graphics cards
/// ([`crate::ear_cards::cards`]). `events` hears what is being done; `cancel`
/// stops it. The Ear's own choice of model is not changed.
pub fn try_model(
    home: &Path,
    which: &str,
    quick: bool,
    cards: &[Card],
    events: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<Tried, InstallError> {
    let model = model_named(which)?;
    try_plan(home, which, quick, cards)?; // installed, and offered on this PC

    // 1. llama.cpp.
    let server = match llama_server(home) {
        Some(server) => server,
        None => get_llama(home, !cards.is_empty(), events, cancel)?,
    };
    if let Some(needs) = model.needs_card_mb {
        let seen = llama_cards(&server);
        if !seen.iter().any(|card| card.memory_mb >= needs) {
            let found = seen.iter().map(Card::words).collect::<Vec<_>>().join(", ");
            return Err(error(
                "no_card",
                format!(
                    "llama.cpp does not see a graphics card with {} GB or more (it sees: {}). Updating the graphics driver often helps. Nothing more was downloaded.",
                    needs / 1000,
                    if found.is_empty() { "none" } else { &found }
                ),
            ));
        }
    }

    // 2. The model's files.
    let folder = model_folder(home, &model);
    let site =
        std::env::var(MODELS_URL_VARIABLE).unwrap_or_else(|_| "https://huggingface.co".into());
    for file in [&model.model, &model.mmproj] {
        events(Event::Step(&format!("Getting {}", file.name)));
        ear_net::download(
            &format!(
                "{}/{}/resolve/{}/{}",
                site.trim_end_matches('/'),
                model.repo,
                model.revision,
                file.name
            ),
            &folder.join(&file.name),
            &Expected {
                bytes: Some(file.bytes),
                sha256: Some(file.sha256.clone()),
            },
            &mut |done, of| {
                events(Event::Download {
                    what: &file.name,
                    done,
                    of,
                })
            },
            cancel,
        )
        .map_err(net)?;
    }
    // What the Ear reads to use the model (the same as
    // ear/duck_ear/catalog.py's `model_json`).
    let about = json!({
        "name": model.name,
        "size": model.size,
        "source": format!("https://huggingface.co/{}", model.repo),
        "revision": model.revision,
        "licence": model.licence,
        "listening_test": model.listening_test,
        "files": {"model": model.model.name, "mmproj": model.mmproj.name},
        "sha256": {&model.model.name: model.model.sha256, &model.mmproj.name: model.mmproj.sha256},
        "bytes": model.bytes(),
        "settings": {"context": model.context, "sample_rate": model.sample_rate},
    });
    std::fs::write(
        folder.join("model.json"),
        serde_json::to_string_pretty(&about).expect("serializes") + "\n",
    )
    .map_err(|e| {
        error(
            "failed",
            format!("Cannot write into {}: {e}", folder.display()),
        )
    })?;

    // 3. The check, with this model beside the Ear's score model.
    events(Event::Step("Asking the model about the test set"));
    let testset = write_testset(home, quick)?;
    let mut config = EarConfig::from_environment(None).map_err(|why| error("failed", why))?;
    config.program = Some(Places::new(home).launcher());
    config.home = home.to_path_buf();
    config.describe_model = Some(folder.clone());
    if std::env::var_os(duck_echo_ear::times::SCALE_VARIABLE).is_none() {
        // A big model's first load from a slow disk takes its time.
        config.times = config.times.scaled(3.0);
        config
            .ear_env
            .push((duck_echo_ear::times::SCALE_VARIABLE.into(), "3".into()));
    }
    let keeper = EarKeeper::new(config);
    let mut progress = |done: usize, of: usize| events(Event::Question { done, of });
    let mut watch = Watch::new(&mut progress, cancel.clone());
    let checked = ear_check::check_watched(&keeper, &testset, Part::Describe, true, &mut watch);
    keeper.stop();
    let report = checked.map_err(|e| {
        if cancel.is_cancelled() {
            net(NetError::Cancelled)
        } else {
            let report = e.into_report();
            let why = report
                .errors
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "the check failed".to_string());
            error(
                "check_failed",
                format!(
                    "The check did not finish: {why}. The Ear's log is {} and llama-server's is {}.",
                    home.join("ear.log").display(),
                    home.join("llama-server.log").display()
                ),
            )
        }
    })?;

    let missed = if quick {
        vec!["only the quick set of 10 sounds was asked: too few to judge a model".to_string()]
    } else {
        bars_missed(&report)
    };
    let passed = !quick && missed.is_empty();
    let name = report
        .describe
        .as_ref()
        .map(|d| d.model.clone())
        .unwrap_or_else(|| model.name.clone());
    let summary = if passed {
        format!("{name} reached every bar. The Ear can use it for describing, if you say so.")
    } else if quick {
        format!("{name} answered the quick set: everything works. Run the whole set to judge it.")
    } else {
        format!(
            "{name} did not reach every bar, so the Ear keeps scores only: {}.",
            missed.join("; ")
        )
    };
    let last = LastCheck {
        model: name,
        whole_set: !quick,
        passed,
        missed: missed.clone(),
    };
    let _ = std::fs::write(
        folder.join("last-check.json"),
        serde_json::to_string_pretty(&last).expect("serializes") + "\n",
    );
    // The whole results stay beside the model, to show again and to save.
    let _ = ear_check::write(&report, &folder);
    Ok(Tried {
        model: model.name,
        whole_set: !quick,
        passed,
        missed,
        summary,
        report,
    })
}

/// The two files a try's results are kept in.
pub const RESULT_FILES: [&str; 2] = ["ear-check.json", "ear-check.md"];

/// Copy the results of the last try of `which` (`ear-check.json` and
/// `ear-check.md`, kept in the model's folder) into `to`, to keep or to
/// send. Returns the two files written.
pub fn save_results(home: &Path, which: &str, to: &Path) -> Result<Vec<PathBuf>, InstallError> {
    let model = model_named(which)?;
    let folder = model_folder(home, &model);
    std::fs::create_dir_all(to).map_err(|e| {
        error(
            "failed",
            format!("Cannot make the folder {}: {e}", to.display()),
        )
    })?;
    let mut written = Vec::new();
    for name in RESULT_FILES {
        let from = folder.join(name);
        if !from.is_file() {
            return Err(error(
                "not_tried",
                format!("{} has no results yet: try it first.", model.name),
            ));
        }
        std::fs::copy(&from, to.join(name)).map_err(|e| {
            error(
                "failed",
                format!("Cannot write {}: {e}", to.join(name).display()),
            )
        })?;
        written.push(to.join(name));
    }
    Ok(written)
}

/// The results of the last try of `which`, as `ear-check.json` holds
/// them, if it was tried here.
pub fn last_results(home: &Path, which: &str) -> Option<Value> {
    let model = model_named(which).ok()?;
    let text = std::fs::read_to_string(model_folder(home, &model).join(RESULT_FILES[0])).ok()?;
    serde_json::from_str(&text).ok()
}

/// The describe model the Ear uses now (the name in `models/describe.txt`),
/// if any.
pub fn model_in_use(home: &Path) -> Option<String> {
    let name = std::fs::read_to_string(home.join("models").join("describe.txt")).ok()?;
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Make the describe model `which` the one the Ear uses, or with `None`
/// go back to scores only. A model is only accepted after it reached
/// every bar on the whole test set, on this PC. Returns what was done, in
/// words.
pub fn use_model(home: &Path, which: Option<&str>) -> Result<String, InstallError> {
    let chosen = home.join("models").join("describe.txt");
    let Some(which) = which else {
        let _ = std::fs::remove_file(&chosen);
        return Ok("The Ear gives scores only again.".to_string());
    };
    let model = model_named(which)?;
    match last_check(home, which) {
        Some(last) if last.passed && last.whole_set => {
            std::fs::write(&chosen, format!("{}\n", model.name))
                .map_err(|e| error("failed", format!("Cannot write {}: {e}", chosen.display())))?;
            Ok(format!(
                "The Ear now describes with {}. `duck-echo ear use none` goes back to scores only.",
                model.name
            ))
        }
        Some(last) => Err(error(
            "not_passed",
            format!(
                "{} did not reach every bar on this PC ({}), so the Ear does not use it.",
                model.name,
                last.missed.join("; ")
            ),
        )),
        None => Err(error(
            "not_tried",
            format!(
                "{} has not been tried on this PC yet: `duck-echo ear try {}` measures it first.",
                model.name, model.size
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large() -> Describe {
        model_named("large").unwrap()
    }

    fn card(memory_mb: u64) -> Card {
        Card {
            name: "Pretend Card".into(),
            memory_mb,
        }
    }

    /// A list of releases as GitHub answers it, newest first: `v1.0.0`
    /// holds only a note (as llama.cpp's really does), a draft holds a
    /// build nobody may see yet, and two nightly releases hold builds.
    fn listed() -> Value {
        let file = |name: &str| json!({"name": name, "size": 10, "browser_download_url": format!("https://example.invalid/{name}")});
        json!([
            {"tag_name": "v1.0.0", "draft": false, "assets": [file("nightly-tag.txt")]},
            {"tag_name": "b12361", "draft": true, "assets": [file("llama-b12361-bin-ubuntu-x64.tar.gz")]},
            {"tag_name": "b12360", "draft": false, "assets": [
                file("llama-b12360-bin-ubuntu-vulkan-x64.tar.gz"),
                file("llama-b12360-bin-ubuntu-x64.tar.gz"),
                file("llama-b12360-bin-win-cpu-x64.zip"),
            ]},
            {"tag_name": "b12359", "draft": false, "assets": [file("llama-b12359-bin-ubuntu-x64.tar.gz")]},
        ])
    }

    #[test]
    fn the_build_comes_from_the_newest_release_that_has_one() {
        let listed = listed();
        let (tag, asset) = newest_build(&listed, "bin-ubuntu-x64").unwrap();
        assert_eq!(tag, "b12360");
        // Not the Vulkan build, whose name ends differently.
        assert_eq!(asset["name"], "llama-b12360-bin-ubuntu-x64.tar.gz");
        let (tag, asset) = newest_build(&listed, "bin-win-cpu-x64").unwrap();
        assert_eq!(tag, "b12360");
        assert_eq!(asset["name"], "llama-b12360-bin-win-cpu-x64.zip");
        // No release has a build for this one, and the error can say
        // which releases were looked through.
        assert!(newest_build(&listed, "bin-win-vulkan-x64").is_none());
        assert_eq!(release_names(&listed), "v1.0.0, b12361, b12360, b12359");
    }

    #[test]
    fn one_release_alone_is_read_as_a_list_of_one() {
        let one = listed()[3].clone();
        let (tag, _) = newest_build(&one, "bin-ubuntu-x64").unwrap();
        assert_eq!(tag, "b12359");
        assert!(newest_build(&json!({"message": "Not Found"}), "bin-ubuntu-x64").is_none());
        assert_eq!(release_names(&json!([])), "the list is empty");
    }

    #[test]
    fn large_is_offered_only_with_a_card_of_16_gb_or_more() {
        assert!(offered(&large(), &[card(24564)]).is_ok());
        assert!(offered(&large(), &[card(16376)]).is_ok());
        let small = offered(&large(), &[card(8192)]).unwrap_err();
        assert!(
            small.contains(
                "needs a graphics card with 16 GB or more; this PC's is Pretend Card (8 GB)"
            ),
            "{small}"
        );
        let none = offered(&large(), &[]).unwrap_err();
        assert!(none.contains("none was found"), "{none}");
        assert!(offered(&model_named("full").unwrap(), &[]).is_ok());
    }

    #[test]
    fn a_model_is_named_by_size_or_by_name() {
        assert_eq!(model_named("large").unwrap().name, "qwen3-omni-30b-a3b");
        assert_eq!(model_named("qwen2.5-omni-7b").unwrap().size, "full");
        let unknown = model_named("huge").unwrap_err();
        assert_eq!(unknown.code, "unknown_model");
        assert!(unknown.message.contains("large (qwen3-omni-30b-a3b)"));
    }

    #[test]
    fn every_file_the_test_set_names_is_built_in() {
        for set in ["ear/testset/testset.toml", "ear/testset/quick.toml"] {
            let text = TESTSET.iter().find(|(name, _)| *name == set).unwrap().1;
            for line in text.lines() {
                let Some(file) = line.strip_prefix("file = \"") else {
                    continue;
                };
                let file = file.trim_end_matches('"');
                if file == "silence" || file.contains(':') {
                    continue;
                }
                let place = match file.strip_prefix("../../") {
                    Some(outside) => outside.to_string(),
                    None => format!("ear/testset/{file}"),
                };
                assert!(
                    TESTSET.iter().any(|(name, _)| *name == place),
                    "{place} is not built in"
                );
            }
        }
    }

    #[test]
    fn a_model_is_used_only_after_it_passed_on_this_pc() {
        let home = std::env::temp_dir().join(format!("duck-echo-ear-use-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let folder = model_folder(&home, &large());
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            use_model(&home, Some("large")).unwrap_err().code,
            "not_tried"
        );
        let mut last = LastCheck {
            model: "qwen3-omni-30b-a3b@6e35".into(),
            whole_set: true,
            passed: false,
            missed: vec!["Describe: yes on true texts: 30 of 47 (64%, the bar is 80%)".into()],
        };
        let write = |last: &LastCheck| {
            std::fs::write(
                folder.join("last-check.json"),
                serde_json::to_string(last).unwrap(),
            )
            .unwrap()
        };
        write(&last);
        let refused = use_model(&home, Some("large")).unwrap_err();
        assert_eq!(refused.code, "not_passed");
        assert!(refused.message.contains("64%"));
        assert_eq!(model_in_use(&home), None);
        // A pass on the quick set is not a pass.
        last.passed = true;
        last.whole_set = false;
        write(&last);
        assert_eq!(
            use_model(&home, Some("large")).unwrap_err().code,
            "not_passed"
        );
        last.whole_set = true;
        last.missed.clear();
        write(&last);
        assert!(use_model(&home, Some("large"))
            .unwrap()
            .contains("now describes with qwen3-omni-30b-a3b"));
        assert_eq!(model_in_use(&home).as_deref(), Some("qwen3-omni-30b-a3b"));
        assert!(use_model(&home, None).unwrap().contains("scores only"));
        assert_eq!(model_in_use(&home), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn llama_server_is_found_in_the_ears_folder_or_a_folder_inside() {
        let home = std::env::temp_dir().join(format!("duck-echo-ear-llama-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let name = if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        if std::env::var_os("DUCK_EAR_LLAMA_SERVER").is_some() {
            return; // named from outside: nothing to find
        }
        assert_eq!(llama_server(&home), None);
        let inside = home.join("llama").join("build").join("bin");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join(name), b"").unwrap();
        assert_eq!(llama_server(&home), Some(inside.join(name)));
        let _ = std::fs::remove_dir_all(&home);
    }
}
