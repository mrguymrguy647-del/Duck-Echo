//! Analysis with extras: each layer on its own, only some layers heard
//! (`solo`, `mute`), and a picture of the sound.
//!
//! Layers are isolated by rendering the recipe with the other layers
//! silenced, finished with the full mix's gain and length
//! ([`duck_echo_synth::render_layers`]), so a layer analyzed alone reads at
//! the level and times it has in the mix.

use crate::ToolError;
use duck_echo_analyze::{Analysis, PictureInfo};
use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Code, Issue, Recipe};
use serde::Serialize;

/// What to measure besides the whole sound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// Also analyze each heard layer on its own. Recipes only.
    pub per_layer: bool,
    /// Hear only these layers (indices into `layers`); empty hears all.
    /// Recipes only.
    pub solo: Vec<usize>,
    /// Silence these layers. Recipes only.
    pub mute: Vec<usize>,
    /// Draw a picture (level panel and spectrogram) as PNG.
    pub picture: bool,
}

impl Extras {
    /// True when solo or mute changes what is heard.
    pub fn isolates(&self) -> bool {
        !self.solo.is_empty() || !self.mute.is_empty()
    }

    /// True when these extras need a recipe (a WAV file has no layers).
    pub fn needs_recipe(&self) -> bool {
        self.per_layer || self.isolates()
    }

    /// The name of the first extra that needs a recipe, for errors.
    fn recipe_only_argument(&self) -> Option<&'static str> {
        if self.per_layer {
            Some("per_layer")
        } else if !self.solo.is_empty() {
            Some("solo")
        } else if !self.mute.is_empty() {
            Some("mute")
        } else {
            None
        }
    }

    /// The error for asking a WAV file for layers, at `{prefix}per_layer`,
    /// `{prefix}solo` or `{prefix}mute`.
    pub fn wav_error(&self, prefix: &str) -> Option<ToolError> {
        self.recipe_only_argument().map(|arg| {
            ToolError::issue(Issue::error(
                Code::InvalidArgument,
                format!("{prefix}{arg}"),
                format!("`{arg}` needs a recipe (a .toml/.json file, preset:NAME or an inline recipe); a WAV file has no layers"),
            ))
        })
    }

    /// The indices of the layers heard, in order, for a recipe with
    /// `count` layers: `solo` (or every layer when it is empty) minus
    /// `mute`. Indices past the end are `out_of_range` errors at
    /// `{prefix}solo[i]` or `{prefix}mute[i]`; hearing nothing is an
    /// `invalid_argument` error.
    pub fn heard(&self, count: usize, prefix: &str) -> Result<Vec<usize>, ToolError> {
        let mut report = duck_echo_core::Report::new();
        for (arg, list) in [("solo", &self.solo), ("mute", &self.mute)] {
            for (i, &layer) in list.iter().enumerate() {
                if layer >= count {
                    report.push(
                        Issue::error(
                            Code::OutOfRange,
                            format!("{prefix}{arg}[{i}]"),
                            format!(
                                "layer {layer} does not exist; the recipe has {count} layer(s), numbered from 0"
                            ),
                        )
                        .got(layer)
                        .range(0.0, count.saturating_sub(1) as f64),
                    );
                }
            }
        }
        if !report.is_ok() {
            return Err(ToolError::Invalid(report));
        }
        let heard: Vec<usize> = (0..count)
            .filter(|i| self.solo.is_empty() || self.solo.contains(i))
            .filter(|i| !self.mute.contains(i))
            .collect();
        if heard.is_empty() {
            let arg = if self.mute.is_empty() { "solo" } else { "mute" };
            return Err(ToolError::issue(Issue::error(
                Code::InvalidArgument,
                format!("{prefix}{arg}"),
                "solo and mute leave no layer to hear",
            )));
        }
        Ok(heard)
    }
}

/// One layer analyzed on its own.
#[derive(Debug, Clone, Serialize)]
pub struct LayerAnalysis {
    /// Position in the recipe's `layers`.
    pub index: usize,
    /// The layer's `name`, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The layer as heard in the mix: same gain, timing and length.
    pub analysis: Analysis,
}

/// An analysis with the extras asked for. Serializes as the analysis
/// fields with the extras next to them, so a plain analysis keeps its shape.
#[derive(Debug, Clone, Serialize)]
pub struct Measured {
    /// The analysis of the sound (or of the heard layers).
    #[serde(flatten)]
    pub analysis: Analysis,
    /// Recipe validation and render warnings; omitted when there are none
    /// or the sound is a WAV file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Issue>,
    /// With `solo` or `mute`: the layers heard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_layers: Option<Vec<usize>>,
    /// With `per_layer`: each heard layer analyzed on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<LayerAnalysis>>,
    /// With `picture`: what the picture shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<PictureInfo>,
    /// The picture as PNG bytes; not serialized.
    #[serde(skip)]
    pub png: Option<Vec<u8>>,
}

/// A measured recipe and the audio that was measured.
pub struct RecipeMeasurement {
    /// The audio analyzed: the whole render, or only the heard layers.
    pub buffer: AudioBuffer,
    /// The analysis and extras.
    pub measured: Measured,
}

/// Measure a WAV buffer. Extras that need layers are an error.
pub fn measure_audio(buffer: &AudioBuffer, extras: &Extras) -> Result<Measured, ToolError> {
    if let Some(e) = extras.wav_error("") {
        return Err(e);
    }
    let analysis = crate::analyze_buffer(buffer);
    let (picture, png) = picture_of(buffer, &analysis, extras);
    Ok(Measured {
        analysis,
        warnings: Vec::new(),
        heard_layers: None,
        layers: None,
        picture,
        png,
    })
}

/// Render and measure a validated recipe. `warnings` are its validation
/// warnings, passed through; a `normalize_lufs` target the peak ceiling
/// did not allow adds one.
pub fn measure_recipe(
    recipe: &Recipe,
    warnings: &[Issue],
    extras: &Extras,
) -> Result<RecipeMeasurement, ToolError> {
    let heard = extras.heard(recipe.layers.len(), "")?;
    let (full, mastering) = duck_echo_synth::render_mastered(recipe)?;
    let mut warnings = warnings.to_vec();
    if mastering.ceiling_limited {
        warnings.push(ceiling_warning(recipe, &full));
    }
    let buffer = if extras.isolates() {
        duck_echo_synth::render_layers(recipe, &heard, &mastering)?
    } else {
        full
    };
    let analysis = crate::analyze_buffer(&buffer);
    let layers = if extras.per_layer {
        let mut out = Vec::with_capacity(heard.len());
        for &i in &heard {
            let alone = duck_echo_synth::render_layers(recipe, &[i], &mastering)?;
            out.push(LayerAnalysis {
                index: i,
                name: recipe.layers[i].name.clone(),
                analysis: crate::analyze_buffer(&alone),
            });
        }
        Some(out)
    } else {
        None
    };
    let (picture, png) = picture_of(&buffer, &analysis, extras);
    Ok(RecipeMeasurement {
        buffer,
        measured: Measured {
            analysis,
            warnings,
            heard_layers: extras.isolates().then_some(heard),
            layers,
            picture,
            png,
        },
    })
}

fn picture_of(
    buffer: &AudioBuffer,
    analysis: &Analysis,
    extras: &Extras,
) -> (Option<PictureInfo>, Option<Vec<u8>>) {
    if !extras.picture {
        return (None, None);
    }
    let onsets: Vec<f64> = analysis.onsets.iter().map(|o| o.time_seconds).collect();
    let pic = duck_echo_analyze::picture::draw(
        &buffer.samples,
        buffer.sample_rate,
        buffer.channels,
        &onsets,
    );
    (Some(pic.info), Some(pic.png))
}

/// The warning for a `normalize_lufs` target that needed more gain than
/// the peak ceiling allowed.
fn ceiling_warning(recipe: &Recipe, full: &AudioBuffer) -> Issue {
    let ch = full.channels.max(1) as usize;
    let planar: Vec<Vec<f32>> = (0..ch)
        .map(|c| full.samples.iter().skip(c).step_by(ch).copied().collect())
        .collect();
    let slices: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
    let reached = duck_echo_analyze::loudness(&slices, full.sample_rate)
        .map(|(l, _)| format!("{l:.1} LUFS"))
        .unwrap_or_else(|| "too quiet to measure".into());
    let target = recipe.master.normalize_lufs.unwrap_or_default();
    Issue::warning(
        Code::Suspicious,
        "master.normalize_lufs",
        format!(
            "the target {target} LUFS needs the peak above the {} dBFS ceiling, so the gain stopped there and the render measures {reached}; lower the target, or make the sound less peaky (a longer attack, less resonance, a fuller wave) so it is louder at the same peak",
            duck_echo_core::LOUDNESS_CEILING_DBFS
        ),
    )
}
