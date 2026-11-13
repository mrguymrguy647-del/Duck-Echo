//! Operations shared by the `duck-echo` CLI and the MCP server: loading
//! audio from a WAV or a recipe, rendering, comparing, variations, presets
//! and recipe edits. Each returns plain data or a [`ToolError`], so both
//! front ends report the same results the same way.

use duck_echo_audio::{read_wav, AudioBuffer, WavError};
use duck_echo_core::{load_path, Code, Format, Issue, Report};
use std::fmt;
use std::path::Path;

pub mod files;
pub mod measure;
pub mod song;

pub use measure::{Extras, LayerAnalysis, Measured, Options, TrackAnalysis};
pub use song::{load_document, load_document_value, Document, FileResolver, SongFacts, SongRef};

/// Why an operation failed. Both shapes become the same validation report
/// for callers (see [`ToolError::into_report`]).
#[derive(Debug)]
pub enum ToolError {
    /// The recipe is invalid; the report lists every problem.
    Invalid(Report),
    /// Something outside the recipe: a missing or unwritable file, a
    /// refused path, an unusable argument. The issue has a specific code;
    /// its path is empty until a front end names the argument with
    /// [`ToolError::at`].
    Failed(Box<Issue>),
}

impl ToolError {
    /// A failure with a code and message and no path yet.
    pub fn failed(code: Code, message: impl Into<String>) -> Self {
        Self::issue(Issue::error(code, "", message))
    }

    /// A failure described by one issue.
    pub fn issue(issue: Issue) -> Self {
        ToolError::Failed(Box::new(issue))
    }

    /// Give every issue without a path the path `path`: the argument the
    /// failing value came from, such as `arguments.path`. Issues that
    /// already point into a recipe keep their paths.
    pub fn at(mut self, path: &str) -> Self {
        for issue in self.issues_mut() {
            if issue.path.is_empty() {
                issue.path = path.to_string();
            }
        }
        self
    }

    /// Every issue, errors and warnings.
    pub fn issues_mut(&mut self) -> Vec<&mut Issue> {
        match self {
            ToolError::Invalid(r) => r.errors.iter_mut().chain(r.warnings.iter_mut()).collect(),
            ToolError::Failed(i) => vec![i.as_mut()],
        }
    }

    /// The validation report shape: `{ok: false, errors, warnings}`.
    pub fn into_report(self) -> Report {
        match self {
            ToolError::Invalid(r) => r,
            ToolError::Failed(i) => Report::from_error(*i),
        }
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::Invalid(r) => write!(f, "{r}"),
            ToolError::Failed(i) => writeln!(f, "{i}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<Report> for ToolError {
    fn from(r: Report) -> Self {
        ToolError::Invalid(r)
    }
}

/// The code for a failed file operation: `not_found` when the file is
/// missing, `io` otherwise.
pub fn io_code(e: &std::io::Error) -> Code {
    if e.kind() == std::io::ErrorKind::NotFound {
        Code::NotFound
    } else {
        Code::Io
    }
}

/// True when `path` names a recipe (`.toml` or `.json`) rather than audio.
pub fn is_recipe_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("toml" | "json")
    )
}

/// Audio from a path: a WAV file is read; a recipe or song file or
/// `preset:NAME` is validated and rendered in memory. A song's instrument
/// files are read from the song file's own folder.
pub fn load_audio(path: &Path) -> Result<AudioBuffer, ToolError> {
    load_audio_with(path, &song::files_next_to(path))
}

/// [`load_audio`], with a song's instrument files read through `files`.
pub fn load_audio_with(path: &Path, files: &FileResolver) -> Result<AudioBuffer, ToolError> {
    if is_recipe_ref(path) {
        let doc = load_document(path, files)?;
        Ok(measure::measure_document(&doc, None, &Options::default(), "")?.buffer)
    } else {
        read_wav(path).map_err(|e| match e {
            WavError::Io(io) => {
                ToolError::failed(io_code(&io), format!("cannot read {}: {io}", path.display()))
            }
            WavError::Format(why) => ToolError::failed(
                Code::Io,
                format!(
                    "{} is not a WAV file Duck Echo can read ({why}); give a .wav file, a .toml/.json recipe or preset:NAME",
                    path.display()
                ),
            ),
        })
    }
}

/// Compare two sounds given as WAV or recipe paths.
pub fn compare_paths(a: &Path, b: &Path) -> Result<duck_echo_analyze::Comparison, ToolError> {
    let (a, b) = (load_audio(a)?, load_audio(b)?);
    Ok(duck_echo_analyze::compare(signal(&a), signal(&b)))
}

/// Borrow a buffer as an analysis signal.
pub fn signal(buf: &AudioBuffer) -> duck_echo_analyze::Signal<'_> {
    duck_echo_analyze::Signal {
        samples: &buf.samples,
        sample_rate: buf.sample_rate,
        channels: buf.channels,
    }
}

/// Analysis of a buffer.
pub fn analyze_buffer(buf: &AudioBuffer) -> duck_echo_analyze::Analysis {
    duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels)
}

/// The text format for a recipe path.
pub fn format_of(path: &Path) -> Format {
    Format::from_path(path)
}

pub mod presets;

/// Prefix that names a preset wherever a recipe path is accepted.
pub const PRESET_PREFIX: &str = "preset:";

/// A recipe loaded from a reference, with where it came from.
#[derive(Debug, Clone)]
pub struct RecipeRef {
    /// The validated recipe and its warnings.
    pub loaded: duck_echo_core::Loaded,
    /// A short label for output names: the preset name or the file stem.
    pub stem: String,
}

/// Load a recipe from a file path or a `preset:NAME` reference.
pub fn load_recipe(reference: &Path) -> Result<RecipeRef, ToolError> {
    let text = reference.to_string_lossy();
    if let Some(name) = text.strip_prefix(PRESET_PREFIX) {
        let preset = presets::get(name)?;
        return Ok(RecipeRef {
            loaded: presets::load(preset)?,
            stem: preset.name.to_string(),
        });
    }
    let loaded = load_path(reference)?;
    let stem = reference
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sound".into());
    Ok(RecipeRef { loaded, stem })
}

/// True when `reference` names a recipe (a `.toml`/`.json` path or a
/// preset) rather than audio.
pub fn is_recipe_ref(reference: &Path) -> bool {
    reference.to_string_lossy().starts_with(PRESET_PREFIX) || is_recipe_path(reference)
}

/// Result of rendering a recipe to a file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Rendered {
    /// Always true; failures are errors instead.
    pub ok: bool,
    /// Where the WAV was written.
    pub output: String,
    /// The seed used.
    pub seed: u64,
    /// Validation warnings for the recipe, and render warnings (a
    /// `normalize_lufs` target the peak ceiling did not allow).
    pub warnings: Vec<duck_echo_core::Issue>,
    /// Analysis of the rendered audio, plus the extras asked for.
    pub analysis: duck_echo_analyze::Analysis,
    /// With `solo` or `mute`: the layers heard in the written file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_layers: Option<Vec<usize>>,
    /// With `per_layer`: each heard layer analyzed on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<LayerAnalysis>>,
    /// For a song: tempo, bars and exact length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<SongFacts>,
    /// For a song with `solo` or `mute`: the tracks heard in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_tracks: Option<Vec<String>>,
    /// For a song with `per_track`: each heard track analyzed on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<TrackAnalysis>>,
    /// With `picture`: what the picture shows. The PNG is in `png`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<duck_echo_analyze::PictureInfo>,
    /// The picture as PNG bytes, not serialized.
    #[serde(skip)]
    pub png: Option<Vec<u8>>,
}

/// Render a validated recipe to a WAV file, optionally overriding the
/// seed, and analyze the result. `extras` adds per-layer analyses, a
/// picture, or renders only some layers (`solo`, `mute`).
pub fn render_to_file(
    loaded: &duck_echo_core::Loaded,
    output: &Path,
    seed: Option<u64>,
    format: duck_echo_audio::WavFormat,
    extras: &Extras,
) -> Result<Rendered, ToolError> {
    if !has_wav_extension(output) {
        return Err(ToolError::failed(
            Code::PathRefused,
            format!(
                "render output {} must end in .wav; refusing to write audio over another kind of file",
                output.display()
            ),
        ));
    }
    let mut recipe = loaded.recipe.clone();
    if let Some(s) = seed {
        recipe.seed = s;
    }
    let m = measure::measure_recipe(&recipe, &loaded.report.warnings, extras)?;
    write_wav_file(output, &m.buffer, format)?;
    Ok(rendered(output, recipe.seed, m.measured))
}

fn rendered(output: &Path, seed: u64, measured: Measured) -> Rendered {
    Rendered {
        ok: true,
        output: output.display().to_string(),
        seed,
        warnings: measured.warnings,
        analysis: measured.analysis,
        heard_layers: measured.heard_layers,
        layers: measured.layers,
        song: measured.song,
        heard_tracks: measured.heard_tracks,
        tracks: measured.tracks,
        picture: measured.picture,
        png: measured.png,
    }
}

/// Render a recipe or a song to a WAV file and analyze the result. Like
/// [`render_to_file`], for either kind of document. `prefix` is put before
/// argument names in errors (`--` for the CLI, `arguments.` for MCP).
pub fn render_document_to_file(
    doc: &Document,
    output: &Path,
    seed: Option<u64>,
    format: duck_echo_audio::WavFormat,
    options: &Options,
    prefix: &str,
) -> Result<Rendered, ToolError> {
    if !has_wav_extension(output) {
        return Err(ToolError::failed(
            Code::PathRefused,
            format!(
                "render output {} must end in .wav; refusing to write audio over another kind of file",
                output.display()
            ),
        ));
    }
    let m = measure::measure_document(doc, seed, options, prefix)?;
    write_wav_file(output, &m.buffer, format)?;
    let used = seed.unwrap_or(match doc {
        Document::Recipe(r) => r.loaded.recipe.seed,
        Document::Song(s) => s.loaded.song.seed,
    });
    Ok(rendered(output, used, m.measured))
}

/// Write a picture's PNG bytes to `path`, which must end in `.png`.
pub fn write_png(path: &Path, png: &[u8]) -> Result<(), ToolError> {
    let is_png = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(ToolError::failed(
            Code::PathRefused,
            format!("picture file {} must end in .png", path.display()),
        ));
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| {
            ToolError::failed(Code::Io, format!("cannot create {}: {e}", dir.display()))
        })?;
    }
    std::fs::write(path, png)
        .map_err(|e| ToolError::failed(Code::Io, format!("cannot write {}: {e}", path.display())))
}

/// True when `path` ends in `.wav` (any case).
pub fn has_wav_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
}

/// A name safe to use as part of a file name: letters, digits, `-` and `_`
/// kept, anything else (path separators, dots) replaced by `_`. Recipe
/// names come from callers, so they never reach the filesystem unchanged.
pub fn file_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if cleaned.trim_matches('_').is_empty() {
        "sound".into()
    } else {
        cleaned
    }
}

fn write_wav_file(
    output: &Path,
    buffer: &AudioBuffer,
    format: duck_echo_audio::WavFormat,
) -> Result<(), ToolError> {
    if let Some(dir) = output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| {
            ToolError::failed(Code::Io, format!("cannot create {}: {e}", dir.display()))
        })?;
    }
    duck_echo_audio::write_wav(output, buffer, format)
        .map_err(|e| ToolError::failed(Code::Io, format!("cannot write {}: {e}", output.display())))
}

/// Most renders one `variations` call may make.
pub const MAX_VARIATIONS: u32 = 64;

/// One render of a variation set.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VariationRender {
    /// Seed used.
    pub seed: u64,
    /// WAV path, when an output directory was given.
    pub output: Option<String>,
    /// The analysis summary line, repeated here for quick scanning.
    pub summary: String,
    /// Full analysis.
    pub analysis: duck_echo_analyze::Analysis,
}

/// Result of [`variations`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct Variations {
    /// The `variation` amount used.
    pub variation: f32,
    /// False when nothing in the recipe depends on the seed, so every
    /// render is the same.
    pub seeds_matter: bool,
    /// Explanation when `seeds_matter` is false.
    pub note: Option<String>,
    /// One entry per seed, seeds 1..=count.
    pub renders: Vec<VariationRender>,
}

/// Render seeds `1..=count` of a recipe, optionally with a different
/// `variation`, and analyze each. With `out_dir`, each is written as
/// `<stem>_seed<N>.wav`, with the stem made file-safe by [`file_safe`].
pub fn variations(
    recipe: &RecipeRef,
    count: u32,
    variation: Option<f32>,
    out_dir: Option<&Path>,
) -> Result<Variations, ToolError> {
    if !(1..=MAX_VARIATIONS).contains(&count) {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                "count",
                format!("count must be 1..={MAX_VARIATIONS}, got {count}"),
            )
            .got(count)
            .range(1.0, MAX_VARIATIONS as f64),
        ));
    }
    let mut base = recipe.loaded.recipe.clone();
    if let Some(v) = variation {
        base.variation = v;
    }
    let report = duck_echo_core::validate(&base);
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    let uses_noise = base
        .layers
        .iter()
        .any(|l| duck_echo_synth::source_uses_seed(l.source));
    let seeds_matter = base.variation > 0.0 || uses_noise;
    let note = (!seeds_matter).then(|| {
        "variation is 0 and no layer uses noise, so every seed renders the same audio; set a variation such as 0.3".to_string()
    });
    let mut renders = Vec::with_capacity(count as usize);
    for seed in 1..=count as u64 {
        let mut r = base.clone();
        r.seed = seed;
        let buffer = duck_echo_synth::render(&r)?;
        let output = match out_dir {
            Some(dir) => {
                let path = dir.join(format!("{}_seed{seed}.wav", file_safe(&recipe.stem)));
                write_wav_file(&path, &buffer, duck_echo_audio::WavFormat::Pcm16)?;
                Some(path.display().to_string())
            }
            None => None,
        };
        let analysis = analyze_buffer(&buffer);
        renders.push(VariationRender {
            seed,
            output,
            summary: analysis.summary.clone(),
            analysis,
        });
    }
    Ok(Variations {
        variation: base.variation,
        seeds_matter,
        note,
        renders,
    })
}

pub mod edit;

pub use duck_echo_analyze::{Analysis, Comparison};

/// Render a validated recipe in memory.
pub fn render_buffer(recipe: &duck_echo_core::Recipe) -> Result<AudioBuffer, ToolError> {
    Ok(duck_echo_synth::render(recipe)?)
}

/// Compare two buffers.
pub fn compare_buffers(a: &AudioBuffer, b: &AudioBuffer) -> Comparison {
    duck_echo_analyze::compare(signal(a), signal(b))
}
