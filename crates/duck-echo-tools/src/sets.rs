//! Sound sets: render many sounds at once and check them side by side
//! (`set_check`).
//!
//! A set is a folder of sounds, or a set file (`film.set.toml`) that names
//! sounds, folders and presets and the loudness each tag should have:
//!
//! ```toml
//! name = "pick a door"
//! sounds = ["sfx", "music/reel.toml", "preset:thud"]
//! targets = { ambient = -30, hit = -16 }
//! tolerance = 2
//! ```
//!
//! The check renders every sound (a WAV and its facts file each), then
//! writes three things a person can read at a glance:
//!
//! * `<name>.table.csv`: one row per sound, its length, loudness, peak,
//!   clipping, how it ends (or how its loop joins) and its summary;
//! * `<name>.sheet.png`: a contact sheet, every sound's picture with its
//!   name and numbers, problems in red;
//! * `<name>.check.json`: all of it, plus the loudness report per tag:
//!   each sound against its tag's target, with the change that fixes it.
//!
//! Made for Duck Forge while making the sound of "Pick a Door: Blocks":
//! forty sounds made one by one sat at forty levels.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use duck_echo_analyze::board::{text_width, Board, Ink, CHAR_H, CHAR_W};
use duck_echo_core::{Code, Issue, Report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::measure::{measure_document, Options};
use crate::root::Root;
use crate::{asset, file_safe, presets, Document, ToolError, PRESET_PREFIX};

/// Most sounds one check renders.
pub const MAX_SOUNDS: usize = 256;
/// How far from its target a sound may be when the set does not say, LU.
pub const DEFAULT_TOLERANCE: f32 = 2.0;

/// A set file: the sounds, and the loudness each tag should have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundSet {
    /// A name for the set, used for its report files. Default: the set
    /// file's name (`film` for `film.set.toml`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The sounds, in order: files (`sfx/door.toml`), folders (`sfx`:
    /// every sound and song in it and below) or `preset:NAME`. Paths are
    /// relative to the project folder.
    pub sounds: Vec<String>,
    /// Loudness target per tag, LUFS, -60..=0: `{ ambient = -30, hit =
    /// -16 }`. A sound is held to the target of its first tag that has
    /// one here, or else of its first category tag (impact -16, weapon
    /// -17, alert -18, pickup -19, movement -20, ui -22, ambient -24).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<String, f32>,
    /// How far from its target a sound may be, LU, 0..=20. Default 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f32>,
}

/// The JSON Schema of a set file (`duck-echo schema set`).
pub fn set_schema() -> serde_json::Value {
    let mut schema = schemars::schema_for!(SoundSet);
    let obj = schema.ensure_object();
    obj.insert(
        "$id".into(),
        "https://github.com/mrguymrguy647-del/Duck-Echo/schema/set.schema.json".into(),
    );
    obj.insert("title".into(), "Duck Echo sound set".into());
    serde_json::to_value(schema).expect("schema serializes")
}

/// Read a set file's text, with an exact path for every problem.
pub fn parse_set(text: &str, json: bool) -> Result<SoundSet, ToolError> {
    let tree = if json {
        duck_echo_core::tree::parse_json(text)
    } else {
        duck_echo_core::tree::parse_toml(text)
    }
    .map_err(|issue| ToolError::Invalid(Report::from_error(*issue)))?;
    let checked = duck_echo_core::structure::check_against(&tree, &set_schema());
    let mut report = Report::new();
    for issue in checked.issues {
        report.push(issue);
    }
    let set = checked
        .pruned
        .and_then(|v| serde_json::from_value::<SoundSet>(v).ok());
    let Some(set) = set.filter(|_| report.is_ok()) else {
        return Err(ToolError::Invalid(report));
    };
    if set.sounds.is_empty() {
        report.push(
            Issue::error(Code::Invalid, "sounds", "a set needs at least one sound")
                .suggest("sounds = [\"sfx\"]"),
        );
    }
    for (tag, lufs) in &set.targets {
        if !(-60.0..=0.0).contains(lufs) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    format!("targets.{tag}"),
                    format!("a loudness target is -60..=0 LUFS, got {lufs}"),
                )
                .got(*lufs)
                .range(-60.0, 0.0),
            );
        }
    }
    if let Some(t) = set.tolerance {
        if !(0.0..=20.0).contains(&t) {
            report.push(
                Issue::error(
                    Code::OutOfRange,
                    "tolerance",
                    format!("tolerance is 0..=20 LU, got {t}"),
                )
                .got(t)
                .range(0.0, 20.0)
                .suggest("2"),
            );
        }
    }
    if report.is_ok() {
        Ok(set)
    } else {
        Err(ToolError::Invalid(report))
    }
}

/// The target a sound is held to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    /// The tag it comes from.
    pub tag: String,
    /// The loudness wanted, LUFS.
    pub lufs: f32,
    /// `set` (the set's `targets`) or `category` (the preset categories).
    pub from: &'static str,
}

/// One sound of the set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Its name: the file name without the extension, or the preset's.
    pub name: String,
    /// Where it came from: the file (`sfx/door.toml`) or `preset:NAME`.
    pub file: String,
    /// `sound` or `song`.
    pub kind: &'static str,
    /// Its tags.
    pub tags: Vec<String>,
    /// The WAV rendered, `null` when it could not be.
    pub wav: Option<String>,
    /// Length in seconds.
    pub duration_seconds: Option<f64>,
    /// Loudness, LUFS.
    pub loudness_lufs: Option<f32>,
    /// Largest sample, dBFS.
    pub peak_dbfs: Option<f32>,
    /// True when samples hit full scale.
    pub clipping: bool,
    /// How it ends: `clean` or `click`; for a loop, `loop clean` or `loop
    /// click` (how its end joins its start).
    pub ending: String,
    /// The target it is held to, `null` when none of its tags has one.
    pub target: Option<Target>,
    /// Its loudness minus its target, LU (+ is too loud).
    pub off_by_lu: Option<f32>,
    /// True within the tolerance, `null` with no target.
    pub on_target: Option<bool>,
    /// The analysis summary line, or why it could not be rendered.
    pub summary: String,
    /// Why it could not be loaded or rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Row {
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.error.is_some() {
            out.push("did not render".into());
        }
        if self.clipping {
            out.push("clips".into());
        }
        if self.ending.ends_with("click") {
            out.push(self.ending.clone());
        }
        if let (Some(false), Some(off)) = (self.on_target, self.off_by_lu) {
            out.push(format!("{off:+.1} LU off target"));
        }
        out
    }
}

/// One sound in the loudness report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagSound {
    /// The sound's name.
    pub name: String,
    /// Its loudness, LUFS.
    pub loudness_lufs: Option<f32>,
    /// Its loudness minus the target, LU.
    pub off_by_lu: Option<f32>,
    /// True within the tolerance.
    pub ok: bool,
    /// What to change when it is not, in words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// The loudness report for one tag.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagReport {
    /// The tag.
    pub tag: String,
    /// Its target, LUFS.
    pub target_lufs: f32,
    /// `set` or `category`.
    pub from: &'static str,
    /// The tolerance, LU.
    pub tolerance: f32,
    /// How many of its sounds are on target.
    pub met: usize,
    /// Its sounds.
    pub sounds: Vec<TagSound>,
}

/// What `set_check` answers (and writes as `<name>.check.json`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetChecked {
    /// Always true; failures are errors instead.
    pub ok: bool,
    /// Always `duck-echo-set-check`.
    pub kind: &'static str,
    /// The set's name.
    pub name: String,
    /// The folder holding the WAVs and the reports.
    pub out: String,
    /// The table, CSV.
    pub table_csv: String,
    /// The contact sheet, PNG.
    pub sheet_png: String,
    /// This report, JSON.
    pub check_json: String,
    /// One line: how many sounds, and every problem by name.
    pub summary: String,
    /// Every sound, in order.
    pub sounds: Vec<Row>,
    /// The loudness report, one entry per tag that holds a sound.
    pub loudness: Vec<TagReport>,
    /// Sounds no tag gives a target.
    pub no_target: Vec<String>,
}

/// One sound to check: how the set names it and where it is.
struct Item {
    file: String,
    path: PathBuf,
}

/// The sounds a set names, in order, folders opened (sounds and songs in
/// path order, the `out` folder left out). `key` is where an entry is
/// named, for errors: `sounds` (a set file) or `arguments.set`.
fn items(root: &Root, entries: &[(String, String)], out: &Path) -> Result<Vec<Item>, ToolError> {
    let mut items = Vec::new();
    for (key, entry) in entries {
        let path = root.read(key, entry)?;
        if path.is_dir() {
            let listing = crate::files::list(&path, root.dir(), true)?;
            for e in listing.entries.iter().filter(|e| e.kind == "recipe") {
                let p = root.dir().join(&e.path);
                if p.starts_with(out) {
                    continue;
                }
                items.push(Item {
                    file: e.path.clone(),
                    path: p,
                });
            }
        } else {
            items.push(Item {
                file: entry.clone(),
                path,
            });
        }
    }
    if items.len() > MAX_SOUNDS {
        return Err(ToolError::failed(
            Code::Invalid,
            format!(
                "the set holds {} sounds; one check renders at most {MAX_SOUNDS}: split it into smaller sets",
                items.len()
            ),
        ));
    }
    Ok(items)
}

/// Check a set: `set` is a set file or a folder, relative to the project
/// folder; `out` is where the WAVs and reports go (default
/// `checks/<name>`). Arguments in errors are `<prefix>set` and
/// `<prefix>out`.
pub fn check(
    root: &Root,
    set: &str,
    out: Option<&str>,
    prefix: &str,
) -> Result<SetChecked, ToolError> {
    let key = format!("{prefix}set");
    let path = root.read(&key, set)?;
    if set.starts_with(PRESET_PREFIX) {
        return Err(ToolError::issue(
            Issue::error(
                Code::InvalidArgument,
                &key,
                "a set is a set file (such as film.set.toml) or a folder of sounds",
            )
            .got(set),
        ));
    }
    let (def, entries) = if path.is_dir() {
        let name = path
            .file_name()
            .map_or("sounds".into(), |n| n.to_string_lossy().into_owned());
        let def = SoundSet {
            name: Some(name),
            sounds: vec![set.to_string()],
            targets: BTreeMap::new(),
            tolerance: None,
        };
        (def, vec![(key.clone(), set.to_string())])
    } else {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ToolError::failed(Code::Io, format!("cannot read {set}: {e}")))?;
        let json = path.extension().is_some_and(|e| e == "json");
        let mut def = parse_set(&text, json)?;
        if def.name.is_none() {
            let file = path
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let stem = file
                .trim_end_matches(".toml")
                .trim_end_matches(".json")
                .trim_end_matches(".set");
            def.name = Some(stem.to_string());
        }
        let entries = def
            .sounds
            .iter()
            .enumerate()
            .map(|(i, s)| (format!("sounds[{i}]"), s.clone()))
            .collect();
        (def, entries)
    };
    let name = file_safe(def.name.as_deref().unwrap_or("sounds"));
    let out_dir = match out {
        Some(o) => root.write(&format!("{prefix}out"), o)?,
        None => root.dir().join("checks").join(&name),
    };
    let items = items(root, &entries, &out_dir)?;
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        ToolError::failed(
            Code::Io,
            format!("cannot make {}: {e}", root.show(&out_dir)),
        )
    })?;
    let tolerance = def.tolerance.unwrap_or(DEFAULT_TOLERANCE);

    let mut rows = Vec::with_capacity(items.len());
    let mut pictures = Vec::with_capacity(items.len());
    let mut used_names: Vec<String> = Vec::new();
    for item in &items {
        let (row, picture) = check_one(root, item, &def, tolerance, &out_dir, &mut used_names);
        rows.push(row);
        pictures.push(picture);
    }

    // The loudness report, tag by tag in the order they first appear.
    let mut loudness: Vec<TagReport> = Vec::new();
    let mut no_target = Vec::new();
    for row in rows.iter().filter(|r| r.error.is_none()) {
        let Some(t) = &row.target else {
            no_target.push(row.name.clone());
            continue;
        };
        let at = match loudness.iter().position(|r| r.tag == t.tag) {
            Some(i) => i,
            None => {
                loudness.push(TagReport {
                    tag: t.tag.clone(),
                    target_lufs: t.lufs,
                    from: t.from,
                    tolerance,
                    met: 0,
                    sounds: Vec::new(),
                });
                loudness.len() - 1
            }
        };
        let ok = row.on_target == Some(true);
        let fix = (!ok).then(|| fix_words(row, t.lufs));
        let report = &mut loudness[at];
        report.met += usize::from(ok);
        report.sounds.push(TagSound {
            name: row.name.clone(),
            loudness_lufs: row.loudness_lufs,
            off_by_lu: row.off_by_lu,
            ok,
            fix,
        });
    }

    let summary = summary_line(&rows);
    let table = out_dir.join(format!("{name}.table.csv"));
    let sheet = out_dir.join(format!("{name}.sheet.png"));
    let report_file = out_dir.join(format!("{name}.check.json"));
    crate::save::write_atomic(&table, table_csv(&rows).as_bytes())?;
    crate::save::write_atomic(&sheet, &contact_sheet(&name, &rows, &pictures))?;
    let checked = SetChecked {
        ok: true,
        kind: "duck-echo-set-check",
        name,
        out: root.show(&out_dir),
        table_csv: root.show(&table),
        sheet_png: root.show(&sheet),
        check_json: root.show(&report_file),
        summary,
        sounds: rows,
        loudness,
        no_target,
    };
    let text = serde_json::to_string_pretty(&checked).expect("plain JSON") + "\n";
    crate::save::write_atomic(&report_file, text.as_bytes())?;
    Ok(checked)
}

/// Render and measure one sound. A sound that does not load or render
/// is a row with its `error`, so one broken file does not hide the rest.
fn check_one(
    root: &Root,
    item: &Item,
    def: &SoundSet,
    tolerance: f32,
    out_dir: &Path,
    used: &mut Vec<String>,
) -> (Row, Option<duck_echo_analyze::png::Indexed>) {
    let name = item
        .file
        .strip_prefix(PRESET_PREFIX)
        .map(str::to_string)
        .unwrap_or_else(|| {
            Path::new(&item.file)
                .file_stem()
                .map_or("sound".into(), |s| s.to_string_lossy().into_owned())
        });
    let mut row = Row {
        name: name.clone(),
        file: item.file.clone(),
        kind: "sound",
        tags: Vec::new(),
        wav: None,
        duration_seconds: None,
        loudness_lufs: None,
        peak_dbfs: None,
        clipping: false,
        ending: String::new(),
        target: None,
        off_by_lu: None,
        on_target: None,
        summary: String::new(),
        error: None,
    };
    let fail = |mut row: Row, e: ToolError| {
        let report = e.into_report();
        let why = report
            .errors
            .first()
            .map(|i| {
                if i.path.is_empty() {
                    i.message.clone()
                } else {
                    format!("{}: {}", i.path, i.message)
                }
            })
            .unwrap_or_else(|| "it could not be rendered".into());
        row.summary = why.clone();
        row.error = Some(why);
        (row, None)
    };
    let files = root.song_files(root.folder_of(&item.path));
    let doc = match crate::load_document(&item.path, &files) {
        Ok(d) => d,
        Err(e) => return fail(row, e),
    };
    let (tags, loops) = match &doc {
        Document::Recipe(r) => (r.loaded.recipe.tags.clone(), false),
        Document::Song(s) => {
            row.kind = "song";
            (s.loaded.song.tags.clone(), s.loaded.song.loop_)
        }
    };
    row.tags = tags;
    let m = match measure_document(&doc, None, &Options::default(), "") {
        Ok(m) => m,
        Err(e) => return fail(row, e),
    };
    // A file name of its own in the out folder.
    let mut stem = file_safe(&name);
    let mut n = 2;
    while used.contains(&stem) {
        stem = format!("{}_{n}", file_safe(&name));
        n += 1;
    }
    used.push(stem.clone());
    let wav = out_dir.join(format!("{stem}.wav"));
    let a = &m.measured.analysis;
    let (data, seed) = match &doc {
        Document::Recipe(r) => (asset::recipe_data(&r.loaded.recipe), r.loaded.recipe.seed),
        Document::Song(s) => (asset::song_data(&s.loaded.song), s.loaded.song.seed),
    };
    let written = crate::write_wav_file(&wav, &m.buffer, duck_echo_audio::WavFormat::Pcm16)
        .and_then(|()| {
            let mut info = asset::info(&m.buffer, a, &wav, &data, seed, loops);
            asset::write_sidecar(&wav, &mut info)
        });
    if let Err(e) = written {
        return fail(row, e);
    }
    row.wav = Some(root.show(&wav));
    row.duration_seconds = Some(a.duration_seconds);
    row.loudness_lufs = a.loudness_lufs;
    row.peak_dbfs = a.peak_dbfs;
    row.clipping = a.clipping.clipped;
    row.ending = match &m.measured.loop_seam {
        Some(seam) if seam.clean => "loop clean".into(),
        Some(_) => "loop click".into(),
        None if a.ending.clean => "clean".into(),
        None => "click".into(),
    };
    row.summary = a.summary.clone();
    row.target = target_for(&row.tags, def);
    if let (Some(t), Some(l)) = (&row.target, row.loudness_lufs) {
        // (+ 0.0 turns a -0.0 into 0.0.)
        let off = ((l - t.lufs) * 10.0).round() / 10.0 + 0.0;
        row.off_by_lu = Some(off);
        row.on_target = Some(off.abs() <= tolerance);
    }
    let onsets: Vec<f64> = a.onsets.iter().map(|o| o.time_seconds).collect();
    let picture = duck_echo_analyze::picture::draw_image(
        &m.buffer.samples,
        m.buffer.sample_rate,
        m.buffer.channels,
        &onsets,
        &[],
    )
    .1;
    (row, Some(picture))
}

/// The target of a sound with `tags`: its first tag the set gives one,
/// or else its first category tag.
fn target_for(tags: &[String], def: &SoundSet) -> Option<Target> {
    if let Some((tag, lufs)) = tags
        .iter()
        .find_map(|t| def.targets.get(t).map(|l| (t, *l)))
    {
        return Some(Target {
            tag: tag.clone(),
            lufs,
            from: "set",
        });
    }
    tags.iter().find_map(|t| {
        presets::target_lufs(t).map(|lufs| Target {
            tag: t.clone(),
            lufs,
            from: "category",
        })
    })
}

/// What to change so a sound meets its target, in words.
fn fix_words(row: &Row, target: f32) -> String {
    let Some(off) = row.off_by_lu else {
        return "it is silent: give it some sound".into();
    };
    let way = if off > 0.0 { "down" } else { "up" };
    let set = format!("master.normalize_lufs = {target}");
    if row.file.starts_with(PRESET_PREFIX) {
        format!(
            "copy the preset into the project and set {set} (or turn it {way} {:.1} dB)",
            off.abs()
        )
    } else {
        format!(
            "set {set} in {} (or turn it {way} {:.1} dB)",
            row.file,
            off.abs()
        )
    }
}

/// "12 sounds; clipping: a, b; ends with a click: c; off target: d (+4.1 LU)".
fn summary_line(rows: &[Row]) -> String {
    let mut parts = vec![format!(
        "{} sound{}",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    )];
    let names =
        |f: &dyn Fn(&Row) -> Option<String>| -> Vec<String> { rows.iter().filter_map(f).collect() };
    let groups: [(&str, Vec<String>); 4] = [
        (
            "did not render",
            names(&|r| r.error.as_ref().map(|_| r.name.clone())),
        ),
        ("clipping", names(&|r| r.clipping.then(|| r.name.clone()))),
        (
            "a click at the end or the loop",
            names(&|r| r.ending.ends_with("click").then(|| r.name.clone())),
        ),
        (
            "off target",
            names(&|r| match (r.on_target, r.off_by_lu) {
                (Some(false), Some(off)) => Some(format!("{} ({off:+.1} LU)", r.name)),
                _ => None,
            }),
        ),
    ];
    let mut clean = true;
    for (what, list) in groups {
        if !list.is_empty() {
            clean = false;
            parts.push(format!("{what}: {}", list.join(", ")));
        }
    }
    if clean {
        parts.push("no problems".into());
    }
    parts.join("; ")
}

/// The table as CSV: a header row, then one row per sound.
fn table_csv(rows: &[Row]) -> String {
    fn cell(s: &str) -> String {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let num = |v: Option<f64>, digits: usize| v.map_or(String::new(), |v| format!("{v:.digits$}"));
    let mut out = String::from(
        "name,file,kind,tags,length_s,lufs,peak_dbfs,clipping,ending,target_lufs,off_lu,on_target,summary\n",
    );
    for r in rows {
        let fields = [
            cell(&r.name),
            cell(&r.file),
            r.kind.to_string(),
            cell(&r.tags.join(" ")),
            num(r.duration_seconds, 3),
            num(r.loudness_lufs.map(f64::from), 1),
            num(r.peak_dbfs.map(f64::from), 1),
            if r.error.is_some() {
                String::new()
            } else {
                r.clipping.to_string()
            },
            r.ending.clone(),
            num(r.target.as_ref().map(|t| f64::from(t.lufs)), 1),
            num(r.off_by_lu.map(f64::from), 1),
            r.on_target.map_or(String::new(), |b| b.to_string()),
            cell(&r.summary),
        ];
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

/// Columns of the contact sheet.
const SHEET_COLUMNS: usize = 3;
/// Space around and between the tiles.
const SHEET_GAP: usize = 14;
/// Height of a tile's label: the name (size 2) and a line of numbers.
const LABEL_H: usize = 36;
/// Height of the sheet's title.
const TITLE_H: usize = 40;

/// The contact sheet: every sound's picture with its name and numbers,
/// three to a row, problems in red with a red frame.
fn contact_sheet(
    name: &str,
    rows: &[Row],
    pictures: &[Option<duck_echo_analyze::png::Indexed>],
) -> Vec<u8> {
    let tile_w = duck_echo_analyze::picture::WIDTH as usize;
    let tile_h = duck_echo_analyze::picture::HEIGHT as usize;
    let columns = rows.len().clamp(1, SHEET_COLUMNS);
    let lines = rows.len().div_ceil(SHEET_COLUMNS).max(1);
    let width = SHEET_GAP + columns * (tile_w + SHEET_GAP);
    let height = TITLE_H + lines * (LABEL_H + tile_h + SHEET_GAP) + SHEET_GAP;
    let mut b = Board::new(width, height);
    let max_title = (width - 2 * SHEET_GAP) / (CHAR_W * 2);
    b.text(
        SHEET_GAP,
        12,
        &format!("{name}: {}", summary_line(rows)),
        2,
        Ink::Text,
        max_title,
    );
    for (i, (row, picture)) in rows.iter().zip(pictures).enumerate() {
        let x = SHEET_GAP + (i % SHEET_COLUMNS) * (tile_w + SHEET_GAP);
        let y = TITLE_H + (i / SHEET_COLUMNS) * (LABEL_H + tile_h + SHEET_GAP);
        let problems = row.problems();
        let ink = if problems.is_empty() {
            Ink::Text
        } else {
            Ink::Problem
        };
        b.text(x, y + 2, &row.name, 2, Ink::Text, tile_w / (CHAR_W * 2));
        let facts = match (&row.error, row.duration_seconds) {
            (Some(_), _) | (None, None) => "did not render".to_string(),
            (None, Some(seconds)) => {
                let mut words = vec![format!("{seconds:.2} s")];
                match row.loudness_lufs {
                    Some(l) => words.push(format!("{l:.1} LUFS")),
                    None => words.push("silent".into()),
                }
                if let Some(p) = row.peak_dbfs {
                    words.push(format!("peak {p:.1}"));
                }
                if let Some(t) = &row.target {
                    words.push(format!("target {} {}", t.tag, t.lufs));
                }
                words.extend(problems.iter().cloned());
                words.join("  ")
            }
        };
        b.text(x, y + 2 + CHAR_H * 2 + 6, &facts, 1, ink, tile_w / CHAR_W);
        let top = y + LABEL_H;
        match picture {
            Some(p) => b.place(p, x, top),
            None => {
                b.fill(x, top, tile_w, tile_h, Ink::Panel);
                let words = "could not render";
                b.text(
                    x + (tile_w.saturating_sub(text_width(words, 2))) / 2,
                    top + tile_h / 2 - CHAR_H,
                    words,
                    2,
                    Ink::Problem,
                    40,
                );
            }
        }
        if !problems.is_empty() {
            b.frame(
                x.saturating_sub(2),
                top.saturating_sub(2),
                tile_w + 4,
                tile_h + 4,
                Ink::Problem,
            );
            b.frame(
                x.saturating_sub(1),
                top.saturating_sub(1),
                tile_w + 2,
                tile_h + 2,
                Ink::Problem,
            );
        }
    }
    b.into_png()
}
