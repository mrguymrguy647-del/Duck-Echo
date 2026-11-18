//! Sending sounds into a Duck Forge game (Phase 3, step 3:
//! docs/phase3-design.md, section 3).
//!
//! `game_send` renders a sound, a song or a set of variants straight into a
//! game of a Duck Forge folder; `game_list` says which games there are and
//! which Duck Echo sounds each already has. The same commands serve the
//! Studio, `duck-echo send` and the MCP server.
//!
//! **Where it may write.** An agent must not write anywhere it likes, so:
//!
//! * The Duck Forge folder is given, never guessed: Studio Settings, the
//!   MCP server's `--duck-forge DIR`, or `DUCK_ECHO_DUCK_FORGE` for the
//!   command line. It must be a Duck Forge folder: its `Cargo.toml` names
//!   the package `duckforge`, and it has an `examples` folder (as Duck Forge
//!   Studio checks).
//! * A game is named, never given as a path: `duck_craft` is the folder
//!   `examples/duck_craft/` with a `main.rs` in it.
//! * Files go only into the game's `assets/` (the sound and its facts) and
//!   `art/` (the recipe), with names made safe ([`crate::file_safe`]). A
//!   folder or file there that is a symbolic link is refused.
//! * Every file is written whole and renamed into place
//!   ([`crate::save::write_atomic`]), so a running game never reads half a
//!   file. The order is the sound, then its facts, then the recipe.
//!
//! **Sending again with no change writes nothing.** That is decided from
//! the files in the game alone, with no rendering:
//!
//! * the facts beside the sound have the same `recipe_hash` (everything
//!   that changes the sound: the recipe's values, its seed, its random
//!   ranges), the same format and the same OGG quality;
//! * the sound file on disk still has the `file_sha256` its facts say, so
//!   a sound changed by hand is sent again.
//!
//! Both are checks of the game folder against itself. They never compare
//! the sound with what this computer would make: an OGG's bytes differ
//! from one computer to another (CI showed it), so a game folder committed
//! on Windows and sent again from Linux with the same recipe is
//! "unchanged". The recipe copy in `art/` is compared as text, with line
//! endings ignored (Git on Windows checks text out with `\r\n`); if only
//! its text changed (a comment), only it is written.
//!
//! Sending is not a step of Duck Echo's Undo history: it writes into the
//! game, not into the Duck Echo project. Duck Forge Studio's own History
//! sees the game's files change.

use std::path::{Path, PathBuf};

use duck_echo_core::{load::did_you_mean, Code, Issue};
use serde::Serialize;

use crate::instruments::INSTRUMENT_PREFIX;
use crate::root::Root;
use crate::{asset, file_safe, presets, Document, ToolError, PRESET_PREFIX};

/// The environment variable that names the Duck Forge folder for the
/// command line.
pub const ENV_VARIABLE: &str = "DUCK_ECHO_DUCK_FORGE";

/// Most variants one send makes.
pub const MAX_VARIANTS: u32 = crate::MAX_VARIATIONS;

/// A Duck Forge folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuckForge {
    dir: PathBuf,
}

impl DuckForge {
    /// Open `dir` as a Duck Forge folder, or say why it is not one.
    pub fn open(dir: &Path) -> Result<DuckForge, String> {
        let dir = dir
            .canonicalize()
            .map_err(|e| format!("cannot use {} as the Duck Forge folder: {e}", dir.display()))?;
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        let named = manifest
            .lines()
            .any(|line| line.replace(' ', "") == "name=\"duckforge\"");
        if !named || !dir.join("examples").is_dir() {
            return Err(format!(
                "{} is not a Duck Forge folder: choose the folder that holds Duck Forge (its Cargo.toml names the package duckforge, and it has an examples folder)",
                dir.display()
            ));
        }
        Ok(DuckForge { dir })
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The names of its games, in order: `examples/<name>/` folders with a
    /// `main.rs` (links are not followed).
    pub fn games(&self) -> Vec<String> {
        let mut games: Vec<String> = std::fs::read_dir(self.dir.join("examples"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| e.path().join("main.rs").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| is_game_name(n))
            .collect();
        games.sort();
        games
    }

    /// The folder of the game `name`, or an error at `key` that lists the
    /// games and suggests the nearest.
    fn game(&self, name: &str, key: &str) -> Result<PathBuf, ToolError> {
        let games = self.games();
        if is_game_name(name) && games.iter().any(|g| g == name) {
            return Ok(self.dir.join("examples").join(name));
        }
        let mut issue = Issue::error(
            Code::NotFound,
            key,
            format!(
                "there is no game `{name}` in the Duck Forge folder (a game is a folder examples/<name> with a main.rs; game_list lists them)"
            ),
        )
        .got(name)
        .one_of(games.iter().cloned());
        if let Some(s) = did_you_mean(name, &games) {
            issue = issue.suggest(s);
        }
        Err(ToolError::issue(issue))
    }
}

/// A game's name: letters, digits, `_` and `-`, so it can never be a path.
fn is_game_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The Duck Forge folder of `root`, or an error at `key` saying how to
/// set one.
fn duck_forge_of<'r>(root: &'r Root, key: &str) -> Result<&'r DuckForge, ToolError> {
    root.duck_forge().ok_or_else(|| {
        ToolError::issue(Issue::error(
            Code::InvalidArgument,
            key,
            format!(
                "no Duck Forge folder is set: choose it in Duck Echo Studio's Settings, start the MCP server with --duck-forge DIR, or set {ENV_VARIABLE} for the command line"
            ),
        ))
    })
}

/// A folder of a game (`assets` or `art`), made if missing; refused when
/// it is a link or not a folder.
fn game_folder(game: &Path, sub: &str) -> Result<PathBuf, ToolError> {
    let folder = game.join(sub);
    match std::fs::symlink_metadata(&folder) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => Err(ToolError::failed(
            Code::PathRefused,
            format!("the game's {sub} is a link or not a folder, so nothing is written into it"),
        )),
        Ok(_) => Ok(folder),
        Err(_) => {
            std::fs::create_dir(&folder).map_err(|e| {
                ToolError::failed(
                    Code::Io,
                    format!("cannot make the game's {sub} folder: {e}"),
                )
            })?;
            Ok(folder)
        }
    }
}

/// A file in a game folder: refused when it is a link or a folder.
fn game_file(folder: &Path, name: &str) -> Result<PathBuf, ToolError> {
    let path = folder.join(name);
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() || meta.is_dir() {
            return Err(ToolError::failed(
                Code::PathRefused,
                format!("{name} in the game is a link or a folder, so it is not written over"),
            ));
        }
    }
    Ok(path)
}

/// What to send.
#[derive(Debug, Clone, Copy, Default)]
pub struct Request<'a> {
    /// The sound or song: a file of the Duck Echo project, or
    /// `preset:NAME`.
    pub file: &'a str,
    /// The game's name.
    pub game: &'a str,
    /// The name in the game (default: the file's name).
    pub name: Option<&'a str>,
    /// Send this many numbered variants and their list instead.
    pub variants: Option<u32>,
    /// `wav` or `ogg` (default: `wav` for a sound, `ogg` for a song).
    pub format: Option<&'a str>,
    /// OGG quality, 0 to 10 (default 4).
    pub quality: Option<f32>,
}

/// What a send did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sent {
    /// Always true; failures are errors instead.
    pub ok: bool,
    /// The game.
    pub game: String,
    /// `sound`, `song` or `variants`.
    pub kind: &'static str,
    /// `wav` or `ogg`.
    pub format: &'static str,
    /// True when the sound in the game is already this one, so it was not
    /// written (its recipe copy may still have been, if only its text
    /// changed).
    pub unchanged: bool,
    /// The files written, from the game's folder, in the order written.
    /// Empty when nothing changed.
    pub written: Vec<String>,
    /// The sound (for variants, the first one), from the game's folder.
    pub audio: String,
    /// Its facts file.
    pub facts: String,
    /// The recipe copy.
    pub recipe: String,
    /// For variants: the list a game picks from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    /// One sentence saying what happened.
    pub summary: String,
}

/// Send a sound, a song or variants into a game. Arguments in errors are
/// `<prefix>file`, `<prefix>game` and so on.
pub fn send(root: &Root, request: &Request, prefix: &str) -> Result<Sent, ToolError> {
    let arg = |name: &str| format!("{prefix}{name}");
    let forge = duck_forge_of(root, &arg("game"))?;
    let game_dir = forge.game(request.game, &arg("game"))?;

    // The document, and its text for the recipe copy.
    let file = request.file;
    if file.starts_with(INSTRUMENT_PREFIX) {
        return Err(ToolError::issue(
            Issue::error(
                Code::InvalidArgument,
                arg("file"),
                "an instrument plays notes in a song; send a sound, a song or a preset",
            )
            .got(file),
        ));
    }
    let path = root.read(&arg("file"), file)?;
    let files = root.song_files(root.folder_of(&path));
    let doc = crate::load_document(&path, &files).map_err(|e| e.at(&arg("file")))?;
    let (text, extension) = match file.strip_prefix(PRESET_PREFIX) {
        Some(name) => (presets::get(name)?.source.to_string(), "toml".to_string()),
        None => {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                ToolError::failed(crate::io_code(&e), format!("cannot read {file}: {e}"))
                    .at(&arg("file"))
            })?;
            let ext = path
                .extension()
                .map_or("toml".into(), |e| e.to_string_lossy().to_ascii_lowercase());
            (text, ext)
        }
    };
    let is_song = matches!(doc, Document::Song(_));

    // What to make.
    let format: &'static str = match request.format {
        None if is_song => "ogg",
        None => "wav",
        Some("wav") => "wav",
        Some("ogg") => "ogg",
        Some(other) => {
            return Err(ToolError::issue(
                Issue::error(
                    Code::UnknownVariant,
                    arg("format"),
                    format!("unknown format `{other}`"),
                )
                .got(other)
                .one_of(["wav", "ogg"]),
            ))
        }
    };
    if request.quality.is_some() && format != "ogg" {
        return Err(ToolError::issue(Issue::error(
            Code::InvalidArgument,
            arg("quality"),
            "quality is for an OGG; a WAV is written exactly",
        )));
    }
    let quality = (format == "ogg").then(|| request.quality.unwrap_or(crate::ogg::DEFAULT_QUALITY));
    if let Some(count) = request.variants {
        let why = if is_song {
            Some("variants are for a sound effect; a song is sent as one file")
        } else if format == "ogg" {
            Some("variants are sound effects and go as WAV")
        } else if !(1..=MAX_VARIANTS).contains(&count) {
            Some("variants is 1..=64")
        } else {
            None
        };
        if let Some(why) = why {
            return Err(ToolError::issue(
                Issue::error(Code::InvalidArgument, arg("variants"), why)
                    .got(count)
                    .range(1.0, f64::from(MAX_VARIANTS)),
            ));
        }
    }
    let name = file_safe(request.name.unwrap_or_else(|| doc.stem()));
    let assets = game_folder(&game_dir, "assets")?;
    let art = game_folder(&game_dir, "art")?;
    let recipe_name = format!("{name}.{extension}");
    let recipe_path = game_file(&art, &recipe_name)?;
    let recipe_rel = format!("art/{recipe_name}");
    let game = request.game.to_string();

    // Has the recipe copy's text changed? (Line endings do not count.)
    let same_text = |a: &str, b: &str| a.replace("\r\n", "\n") == b.replace("\r\n", "\n");
    let recipe_current = std::fs::read_to_string(&recipe_path).is_ok_and(|t| same_text(&t, &text));

    if let Some(count) = request.variants {
        let Document::Recipe(recipe) = &doc else {
            unreachable!("checked above: variants are for a sound")
        };
        let list_name = format!("{name}.variants.json");
        let list_path = game_file(&assets, &list_name)?;
        for i in 1..=count {
            game_file(&assets, &format!("{name}_{i}.wav"))?;
        }
        let unchanged = variants_unchanged(&list_path, &assets, &recipe.loaded.recipe, count);
        let mut written = Vec::new();
        if !unchanged {
            let request = crate::variants::Request {
                count,
                name: Some(&name),
                variation: None,
                recipe_file: Some(&recipe_rel),
            };
            crate::variants::export(recipe, &recipe_rel, &request, &assets, prefix, &|p| {
                show(&game_dir, p)
            })?;
            written.extend((1..=count).flat_map(|i| {
                [
                    format!("assets/{name}_{i}.wav"),
                    format!("assets/{name}_{i}.wav.json"),
                ]
            }));
            written.push(format!("assets/{list_name}"));
        }
        if !recipe_current {
            crate::save::write_atomic(&recipe_path, text.as_bytes())?;
            written.push(recipe_rel.clone());
        }
        let summary = match (unchanged, written.is_empty()) {
            (true, true) => format!("{name} ({count} variants) is already in {game} and has not changed: nothing written."),
            (true, false) => format!("{name} ({count} variants) is already in {game} and sounds the same; its recipe text changed, so only {recipe_rel} was written."),
            _ => format!("Sent {count} variants of {name} to {game}: assets/{name}_1.wav ... assets/{name}_{count}.wav, listed in assets/{list_name}."),
        };
        return Ok(Sent {
            ok: true,
            game,
            kind: "variants",
            format: "wav",
            unchanged,
            written,
            audio: format!("assets/{name}_1.wav"),
            facts: format!("assets/{name}_1.wav.json"),
            recipe: recipe_rel,
            list: Some(format!("assets/{list_name}")),
            summary,
        });
    }

    // One sound or song.
    let audio_name = format!("{name}.{format}");
    let audio = game_file(&assets, &audio_name)?;
    let facts_name = format!("{audio_name}.json");
    game_file(&assets, &facts_name)?;
    let hash = asset::recipe_hash(&match &doc {
        Document::Recipe(r) => asset::recipe_data(&r.loaded.recipe),
        Document::Song(s) => asset::song_data(&s.loaded.song),
    });
    let unchanged = sound_unchanged(&audio, &hash, format, quality);
    let mut written = Vec::new();
    let mut said = String::new();
    if !unchanged {
        let format_settings = crate::ogg::RenderFormat {
            wav: duck_echo_audio::WavFormat::Pcm16,
            quality,
        };
        let done = crate::render_document_to_file(
            &doc,
            &audio,
            None,
            format_settings,
            &crate::measure::Options::default(),
            prefix,
        )
        .map_err(|e| e.at(&arg("file")))?;
        // The facts again, now naming the recipe copy.
        let mut info = done.asset;
        info.recipe = Some(recipe_rel.clone());
        asset::write_sidecar(&audio, &mut info)?;
        written.push(format!("assets/{audio_name}"));
        written.push(format!("assets/{facts_name}"));
        said = match info.loudness_lufs {
            Some(l) => format!(" ({:.2} s, {l:.1} LUFS)", info.duration_seconds),
            None => format!(" ({:.2} s, silent)", info.duration_seconds),
        };
    }
    if !recipe_current {
        crate::save::write_atomic(&recipe_path, text.as_bytes())?;
        written.push(recipe_rel.clone());
    }
    let summary = match (unchanged, written.is_empty()) {
        (true, true) => format!("{name} is already in {game} and has not changed: nothing written."),
        (true, false) => format!("{name} is already in {game} and sounds the same; its recipe text changed, so only {recipe_rel} was written."),
        _ => format!("Sent {name} to {game}: assets/{audio_name}{said}."),
    };
    Ok(Sent {
        ok: true,
        game,
        kind: if is_song { "song" } else { "sound" },
        format,
        unchanged,
        written,
        audio: format!("assets/{audio_name}"),
        facts: format!("assets/{facts_name}"),
        recipe: recipe_rel,
        list: None,
        summary,
    })
}

/// A path inside a game, as `send` names it: from the game's folder.
fn show(game: &Path, path: &Path) -> String {
    path.strip_prefix(game)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// A JSON file, read; `None` if it is missing or is not JSON.
fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Is the sound at `audio` already the one this recipe makes? From the
/// game's files alone: its facts have this `recipe_hash`, format and
/// quality, and the file still has the `file_sha256` they say.
fn sound_unchanged(audio: &Path, hash: &str, format: &str, quality: Option<f32>) -> bool {
    let Some(facts) = read_json(&asset::sidecar_path(audio)) else {
        return false;
    };
    let text = |key: &str| facts.get(key).and_then(serde_json::Value::as_str);
    let same_quality = match (
        quality,
        facts.get("quality").and_then(serde_json::Value::as_f64),
    ) {
        (None, None) => true,
        (Some(q), Some(f)) => (f64::from(q) - f).abs() < 1e-6,
        _ => false,
    };
    text("recipe_hash") == Some(hash)
        && text("format").unwrap_or("wav") == format
        && same_quality
        && file_matches(audio, text("file_sha256"))
}

/// Does the file at `path` have the SHA-256 its facts say?
fn file_matches(path: &Path, said: Option<&str>) -> bool {
    match (said, asset::file_sha256(path)) {
        (Some(said), Ok(now)) => said == now,
        _ => false,
    }
}

/// Are the variants in the game already the ones this recipe makes, with
/// `count` of them? From the game's files alone, as [`sound_unchanged`].
fn variants_unchanged(
    list: &Path,
    assets: &Path,
    recipe: &duck_echo_core::Recipe,
    count: u32,
) -> bool {
    let Some(list) = read_json(list) else {
        return false;
    };
    let files = list.get("files").and_then(serde_json::Value::as_array);
    if list.get("recipe_hash").and_then(serde_json::Value::as_str)
        != Some(crate::variants::list_hash(recipe).as_str())
        || files.map_or(0, Vec::len) != count as usize
    {
        return false;
    }
    files.into_iter().flatten().enumerate().all(|(i, entry)| {
        let Some(file) = entry.get("file").and_then(serde_json::Value::as_str) else {
            return false;
        };
        if !is_plain_name(file) {
            return false;
        }
        let mut r = recipe.clone();
        r.seed = i as u64 + 1;
        let hash = asset::recipe_hash(&asset::recipe_data(&r));
        sound_unchanged(&assets.join(file), &hash, "wav", None)
    })
}

/// A file name with no folder in it.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != ".." && name != "."
}

/// One Duck Echo sound already in a game.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GameSound {
    /// Its name.
    pub name: String,
    /// Its file, from the game's folder (for variants, the list).
    pub file: String,
    /// `wav` or `ogg`.
    pub format: String,
    /// Length in seconds (for variants, the first one's).
    pub duration_seconds: Option<f64>,
    /// Loudness, LUFS.
    pub loudness_lufs: Option<f64>,
    /// True for a loop.
    pub loops: bool,
    /// The recipe copy, from the game's folder, if its facts name one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// For variants: how many.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<usize>,
}

/// One game.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GameInfo {
    /// Its name, as `game_send` takes it.
    pub name: String,
    /// The Duck Echo sounds in its `assets` folder (those with a facts
    /// file), in name order.
    pub sounds: Vec<GameSound>,
}

/// What `game_list` answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Games {
    /// Always true.
    pub ok: bool,
    /// The Duck Forge folder.
    pub duck_forge: String,
    /// Its games.
    pub games: Vec<GameInfo>,
}

/// The games of the Duck Forge folder, and their Duck Echo sounds.
pub fn list(root: &Root, prefix: &str) -> Result<Games, ToolError> {
    let forge = duck_forge_of(root, &format!("{prefix}game"))?;
    let games = forge
        .games()
        .into_iter()
        .map(|name| {
            let assets = forge.dir.join("examples").join(&name).join("assets");
            GameInfo {
                sounds: sounds_in(&assets),
                name,
            }
        })
        .collect();
    Ok(Games {
        ok: true,
        duck_forge: forge.dir.display().to_string(),
        games,
    })
}

/// The Duck Echo sounds in a game's `assets` folder: each facts file, and
/// each variant list (its numbered sounds are not listed one by one).
fn sounds_in(assets: &Path) -> Vec<GameSound> {
    let mut names: Vec<String> = std::fs::read_dir(assets)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut sounds = Vec::new();
    let mut in_sets: Vec<String> = Vec::new();
    for list_name in names.iter().filter(|n| n.ends_with(".variants.json")) {
        let Some(list) = read_json(&assets.join(list_name)) else {
            continue;
        };
        let files: Vec<String> = list["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["file"].as_str().map(str::to_string))
            .collect();
        let first = files
            .first()
            .and_then(|f| read_json(&assets.join(format!("{f}.json"))));
        in_sets.extend(files.iter().map(|f| format!("{f}.json")));
        sounds.push(GameSound {
            name: list_name.trim_end_matches(".variants.json").to_string(),
            file: format!("assets/{list_name}"),
            format: "wav".into(),
            duration_seconds: first.as_ref().and_then(|f| f["duration_seconds"].as_f64()),
            loudness_lufs: first.as_ref().and_then(|f| f["loudness_lufs"].as_f64()),
            loops: false,
            recipe: first
                .as_ref()
                .and_then(|f| f["recipe"].as_str().map(str::to_string)),
            variants: Some(files.len()),
        });
    }
    for facts_name in names
        .iter()
        .filter(|n| n.ends_with(".wav.json") || n.ends_with(".ogg.json"))
        .filter(|n| !in_sets.contains(n))
    {
        let Some(facts) = read_json(&assets.join(facts_name)) else {
            continue;
        };
        let file = facts_name.trim_end_matches(".json");
        sounds.push(GameSound {
            name: file
                .trim_end_matches(".wav")
                .trim_end_matches(".ogg")
                .to_string(),
            file: format!("assets/{file}"),
            format: facts["format"].as_str().unwrap_or("wav").to_string(),
            duration_seconds: facts["duration_seconds"].as_f64(),
            loudness_lufs: facts["loudness_lufs"].as_f64(),
            loops: facts["loops"].as_bool().unwrap_or(false),
            recipe: facts["recipe"].as_str().map(str::to_string),
            variants: None,
        });
    }
    sounds.sort_by(|a, b| a.name.cmp(&b.name));
    sounds
}
