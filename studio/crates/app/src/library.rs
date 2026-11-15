//! What Home shows, without a window: the person's sounds and songs in the
//! project folder, a small picture and the length of each, and the gallery
//! of built-in sounds, instruments and example songs to start from.
//!
//! The pictures are made on a background thread, with the same rendering
//! and analysis an agent's `analyze` uses: each card's picture is the
//! sound's level over time (`energy_over_time`), 20 bars from start to end.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use duck_echo_core::{DocumentKind, Format, document_kind};
use duck_echo_tools::measure::{Options, measure_document};
use duck_echo_tools::root::Root;
use duck_echo_tools::song::{Document, Library};
use duck_echo_tools::{examples, instruments, presets};

/// A sound or a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A recipe.
    Sound,
    /// A song.
    Song,
}

/// One of the person's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyFile {
    /// The file, relative to the project folder, with `/` between parts.
    pub rel: String,
    /// Its name: the file name without `.toml` or `.json`.
    pub name: String,
    /// A sound or a song.
    pub kind: Kind,
    /// Its text when it was read.
    pub text: String,
}

/// The project folder's sounds and songs, sorted by name. Hidden files
/// and folders (such as `.duck-echo`, the Undo history) are left out, and
/// so is anything that is not a recipe or a song.
pub fn scan(project: &Path) -> Vec<MyFile> {
    let Ok(listing) = duck_echo_tools::files::list(project, project, true) else {
        return Vec::new();
    };
    let mut files: Vec<MyFile> = listing
        .entries
        .into_iter()
        .filter(|e| e.kind == "recipe")
        .filter_map(|e| {
            let path = project.join(&e.path);
            let text = std::fs::read_to_string(&path).ok()?;
            let kind = match document_kind(&text, Format::from_path(&path)) {
                DocumentKind::Recipe => Kind::Sound,
                DocumentKind::Song => Kind::Song,
            };
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some(MyFile {
                rel: e.path.replace('\\', "/"),
                name,
                kind,
                text,
            })
        })
        .collect();
    files.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.rel.cmp(&b.rel))
    });
    files
}

/// A free file name in `folder` (relative to the project) for `stem`:
/// `coin.toml`, or `coin 2.toml` if that is taken, and so on.
pub fn free_name(project: &Path, folder: &str, stem: &str, extension: &str) -> String {
    let join = |name: String| {
        if folder.is_empty() {
            name
        } else {
            format!("{folder}/{name}")
        }
    };
    let taken = |rel: &str| project.join(rel).exists();
    let first = join(format!("{stem}.{extension}"));
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| join(format!("{stem} {n}.{extension}")))
        .find(|rel| !taken(rel))
        .expect("some number is free")
}

/// The folder part of a file's name in the project (`""` at the top).
pub fn folder_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The extension of a file's name (`toml` or `json`).
pub fn extension_of(rel: &str) -> &str {
    rel.rsplit_once('.').map_or("toml", |(_, ext)| ext)
}

/// What a card shows of a file once it has been rendered.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumb {
    /// "0.31 s", or for a song "8 bars" (with ", loops" for a loop).
    pub length: String,
    /// The level over time, 20 values from 0 (silence or -60 dBFS) to 1
    /// (full scale).
    pub levels: Vec<f32>,
    /// Why it could not be rendered, in a few words, if it could not.
    pub problem: Option<String>,
}

/// Render and measure one file for its card.
fn make_thumb(project: &Path, rel: &str) -> Thumb {
    let failed = |problem: String| Thumb {
        length: String::new(),
        levels: Vec::new(),
        problem: Some(problem),
    };
    let Ok(root) = Root::new(project) else {
        return failed("the project folder cannot be read".into());
    };
    let path = match root.read("file", rel) {
        Ok(path) => path,
        Err(e) => return failed(first_line(&e.into_report().to_string())),
    };
    let files = root.song_files(root.folder_of(&path));
    let doc = match duck_echo_tools::song::load_document(&path, &files) {
        Ok(doc) => doc,
        Err(e) => return failed(first_line(&e.into_report().to_string())),
    };
    match measure_document(&doc, None, &Options::default(), "") {
        Ok(m) => {
            let analysis = &m.measured.analysis;
            let length = match (&doc, &m.measured.song) {
                (Document::Song(_), Some(song)) => format!(
                    "{} bar{}{}",
                    song.bars,
                    if song.bars == 1 { "" } else { "s" },
                    if song.loop_ { ", loops" } else { "" }
                ),
                _ => format!("{:.2} s", analysis.duration_seconds),
            };
            let levels = analysis
                .energy_over_time
                .rms_dbfs
                .iter()
                .map(|db| db.map_or(0.0, |db| ((db + 60.0) / 60.0).clamp(0.0, 1.0)))
                .collect();
            Thumb {
                length,
                levels,
                problem: None,
            }
        }
        Err(e) => failed(first_line(&e.into_report().to_string())),
    }
}

/// The first line of a report, without its "error[code]" start.
fn first_line(report: &str) -> String {
    let line = report.lines().next().unwrap_or("it has a problem");
    line.split_once("] ")
        .map_or(line, |(_, rest)| rest)
        .chars()
        .take(120)
        .collect()
}

/// The cards' pictures, made on a background thread and kept by file name
/// and text, so a file is rendered again only when its text changes.
pub struct Thumbs {
    done: BTreeMap<(String, String), Thumb>,
    asked: BTreeSet<(String, String)>,
    todo: Sender<(PathBuf, String, String)>,
    results: Receiver<((String, String), Thumb)>,
}

impl Thumbs {
    /// Start the thread that makes the pictures.
    pub fn new() -> Thumbs {
        let (todo, jobs) = channel::<(PathBuf, String, String)>();
        let (send, results) = channel();
        std::thread::Builder::new()
            .name("duck-echo-thumbnails".into())
            .spawn(move || {
                while let Ok((project, rel, text)) = jobs.recv() {
                    let thumb = make_thumb(&project, &rel);
                    if send.send(((rel, text), thumb)).is_err() {
                        break;
                    }
                }
            })
            .expect("start the thumbnail thread");
        Thumbs {
            done: BTreeMap::new(),
            asked: BTreeSet::new(),
            todo,
            results,
        }
    }

    /// Take in finished pictures.
    pub fn poll(&mut self) {
        while let Ok((key, thumb)) = self.results.try_recv() {
            self.asked.remove(&key);
            self.done.insert(key, thumb);
        }
    }

    /// The picture of `file`, if it is made; asks for it if not.
    pub fn get(&mut self, project: &Path, file: &MyFile) -> Option<&Thumb> {
        let key = (file.rel.clone(), file.text.clone());
        if !self.done.contains_key(&key) && self.asked.insert(key.clone()) {
            let _ = self
                .todo
                .send((project.to_path_buf(), file.rel.clone(), file.text.clone()));
        }
        self.done.get(&key)
    }

    /// True while pictures are being made.
    pub fn busy(&self) -> bool {
        !self.asked.is_empty()
    }

    /// Forget pictures of files that are gone or changed.
    pub fn keep_only(&mut self, files: &[MyFile]) {
        let keys: BTreeSet<(String, String)> = files
            .iter()
            .map(|f| (f.rel.clone(), f.text.clone()))
            .collect();
        self.done.retain(|key, _| keys.contains(key));
    }
}

impl Default for Thumbs {
    fn default() -> Thumbs {
        Thumbs::new()
    }
}

/// Which part of the gallery is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// The 30 built-in sounds.
    Presets,
    /// The built-in instruments and the drum kit.
    Instruments,
    /// The example songs.
    Songs,
}

/// One card of the gallery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryItem {
    /// What `file_copy_from` copies: `preset:coin`, `instrument:piano`,
    /// `example:four_bars`. `None` for the drum kit, which is not one file.
    pub source: Option<String>,
    /// Its name.
    pub name: String,
    /// A sentence about it.
    pub description: String,
    /// The preset's category (`pickup`...), or what it is.
    pub category: String,
    /// Words to search by.
    pub tags: Vec<String>,
}

/// The gallery's categories of presets, in the order of the design, with
/// their names for people.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("pickup", "Pickup"),
    ("impact", "Impact"),
    ("weapon", "Weapon"),
    ("alert", "Alert"),
    ("movement", "Movement"),
    ("ui", "UI"),
    ("ambient", "Ambient"),
];

/// Every item of a shelf, in order.
pub fn gallery(shelf: Shelf) -> Vec<GalleryItem> {
    match shelf {
        Shelf::Presets => {
            let mut items: Vec<GalleryItem> = presets::list(None)
                .unwrap_or_default()
                .into_iter()
                .map(|p| GalleryItem {
                    source: Some(format!("preset:{}", p.name)),
                    name: p.name,
                    description: p.description,
                    category: p.category,
                    tags: p.tags,
                })
                .collect();
            items.sort_by(|a, b| a.name.cmp(&b.name));
            items
        }
        Shelf::Instruments => instruments::list()
            .unwrap_or_default()
            .into_iter()
            .map(|i| GalleryItem {
                source: (i.kind != "kit").then(|| format!("instrument:{}", i.name)),
                name: i.name,
                description: i.description,
                category: i.kind,
                tags: i.tags,
            })
            .collect(),
        Shelf::Songs => examples::EXAMPLES
            .iter()
            .map(|e| GalleryItem {
                source: Some(format!("example:{}", e.name)),
                name: e.name.to_string(),
                description: examples::description(e),
                category: "song".into(),
                tags: Vec::new(),
            })
            .collect(),
    }
}

/// True when `item` matches the search words (each word somewhere in its
/// name, description, category or tags; no words matches everything).
pub fn matches(search: &str, texts: &[&str]) -> bool {
    let all = texts.join(" ").to_lowercase();
    search
        .split_whitespace()
        .all(|word| all.contains(&word.to_lowercase()))
}

/// A song from its text, ready to play: the song and its arrangement.
/// `folder` is where its instrument files are (relative to the project).
pub fn load_song(
    root: &Root,
    folder: &str,
    text: &str,
) -> Result<(duck_echo_core::Song, duck_echo_core::Arrangement), String> {
    let files = root.song_files(folder.to_string());
    duck_echo_core::load_song_str(text, Format::Toml, &Library::new(&files))
        .map(|loaded| (loaded.song, loaded.arrangement))
        .map_err(|report| first_line(&report.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("duck-echo-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_scan_finds_sounds_and_songs_and_skips_the_rest() {
        let dir = folder("scan");
        std::fs::create_dir_all(dir.join("sfx")).unwrap();
        std::fs::create_dir_all(dir.join(".duck-echo/history")).unwrap();
        std::fs::write(
            dir.join("sfx/Coin.toml"),
            presets::get("coin").unwrap().source,
        )
        .unwrap();
        std::fs::write(
            dir.join("tune.toml"),
            examples::get("four_bars").unwrap().source,
        )
        .unwrap();
        std::fs::write(dir.join(".duck-echo/history/000001.json"), "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        let files = scan(&dir);
        let names: Vec<(&str, &str, Kind)> = files
            .iter()
            .map(|f| (f.rel.as_str(), f.name.as_str(), f.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("sfx/Coin.toml", "Coin", Kind::Sound),
                ("tune.toml", "tune", Kind::Song)
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn free_names_count_up() {
        let dir = folder("free");
        assert_eq!(free_name(&dir, "", "coin", "toml"), "coin.toml");
        std::fs::write(dir.join("coin.toml"), "").unwrap();
        assert_eq!(free_name(&dir, "", "coin", "toml"), "coin 2.toml");
        std::fs::write(dir.join("coin 2.toml"), "").unwrap();
        assert_eq!(free_name(&dir, "", "coin", "toml"), "coin 3.toml");
        assert_eq!(free_name(&dir, "sfx", "coin", "json"), "sfx/coin.json");
        assert_eq!(folder_of("sfx/a/coin.toml"), "sfx/a");
        assert_eq!(folder_of("coin.toml"), "");
        assert_eq!(extension_of("x/coin.json"), "json");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_thumbnail_has_a_length_and_twenty_levels() {
        let dir = folder("thumb");
        std::fs::write(dir.join("coin.toml"), presets::get("coin").unwrap().source).unwrap();
        std::fs::write(
            dir.join("tune.toml"),
            examples::get("duck_craft_day").unwrap().source,
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "version = 0\nlayers = 5\n").unwrap();
        let coin = make_thumb(&dir, "coin.toml");
        assert_eq!(coin.length, "0.31 s");
        assert_eq!(coin.levels.len(), 20);
        assert!(coin.levels.iter().any(|l| *l > 0.5), "{:?}", coin.levels);
        assert_eq!(make_thumb(&dir, "tune.toml").length, "8 bars, loops");
        let broken = make_thumb(&dir, "broken.toml");
        assert!(broken.problem.is_some(), "{broken:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_gallery_holds_every_preset_instrument_and_example() {
        assert_eq!(gallery(Shelf::Presets).len(), 30);
        let instruments = gallery(Shelf::Instruments);
        assert_eq!(instruments.len(), 9);
        assert!(
            instruments
                .iter()
                .any(|i| i.name == "drums" && i.source.is_none())
        );
        assert!(
            instruments
                .iter()
                .any(|i| i.source.as_deref() == Some("instrument:piano"))
        );
        let songs = gallery(Shelf::Songs);
        assert_eq!(songs.len(), 3);
        assert!(songs.iter().all(|s| !s.description.is_empty()));
        for (category, _) in CATEGORIES {
            assert!(
                gallery(Shelf::Presets)
                    .iter()
                    .any(|p| p.category == *category),
                "{category}"
            );
        }
        assert!(matches("EXPLO", &["big_explosion", "Large"]));
        assert!(matches("coin pickup", &["coin", "Classic coin pickup."]));
        assert!(!matches("coin laser", &["coin", "Classic coin pickup."]));
        assert!(matches("", &["anything"]));
    }
}
