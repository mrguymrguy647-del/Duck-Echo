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

/// Audio from a path: a WAV file is read; a recipe is validated and
/// rendered in memory.
pub fn load_audio(path: &Path) -> Result<AudioBuffer, ToolError> {
    if is_recipe_path(path) {
        let loaded = load_path(path)?;
        Ok(duck_echo_synth::render(&loaded.recipe)?)
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
