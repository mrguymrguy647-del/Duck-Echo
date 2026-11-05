//! Operations shared by the `duck-echo` CLI and the MCP server: loading
//! audio from a WAV or a recipe, rendering, comparing, variations, presets
//! and recipe edits. Each returns plain data or a [`ToolError`], so both
//! front ends report the same results the same way.

use duck_echo_audio::{read_wav, AudioBuffer};
use duck_echo_core::{load_path, Format, Report};
use std::fmt;
use std::path::Path;

/// Why an operation failed.
#[derive(Debug)]
pub enum ToolError {
    /// The recipe is invalid; the report lists every problem.
    Invalid(Report),
    /// A file could not be read or written, or arguments were unusable.
    Io(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::Invalid(r) => write!(f, "{r}"),
            ToolError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<Report> for ToolError {
    fn from(r: Report) -> Self {
        ToolError::Invalid(r)
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

/// Audio from a path: a WAV file is read; a recipe file or `preset:NAME`
/// is validated and rendered in memory.
pub fn load_audio(path: &Path) -> Result<AudioBuffer, ToolError> {
    if is_recipe_ref(path) {
        let r = load_recipe(path)?;
        Ok(duck_echo_synth::render(&r.loaded.recipe)?)
    } else {
        read_wav(path).map_err(|e| ToolError::Io(format!("cannot read {}: {e}", path.display())))
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
    /// Validation warnings for the recipe.
    pub warnings: Vec<duck_echo_core::Issue>,
    /// Analysis of the rendered audio.
    pub analysis: duck_echo_analyze::Analysis,
}

/// Render a validated recipe to a WAV file, optionally overriding the
/// seed, and analyze the result.
pub fn render_to_file(
    loaded: &duck_echo_core::Loaded,
    output: &Path,
    seed: Option<u64>,
    format: duck_echo_audio::WavFormat,
) -> Result<Rendered, ToolError> {
    if !has_wav_extension(output) {
        return Err(ToolError::Io(format!(
            "render output {} must end in .wav; refusing to write audio over another kind of file",
            output.display()
        )));
    }
    let mut recipe = loaded.recipe.clone();
    if let Some(s) = seed {
        recipe.seed = s;
    }
    let buffer = duck_echo_synth::render(&recipe)?;
    write_wav_file(output, &buffer, format)?;
    Ok(Rendered {
        ok: true,
        output: output.display().to_string(),
        seed: recipe.seed,
        warnings: loaded.report.warnings.clone(),
        analysis: analyze_buffer(&buffer),
    })
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
        std::fs::create_dir_all(dir)
            .map_err(|e| ToolError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    duck_echo_audio::write_wav(output, buffer, format)
        .map_err(|e| ToolError::Io(format!("cannot write {}: {e}", output.display())))
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
        return Err(ToolError::Io(format!(
            "count must be 1..={MAX_VARIATIONS}, got {count}"
        )));
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
