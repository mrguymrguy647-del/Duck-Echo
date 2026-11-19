//! Writing a render as OGG Vorbis, and checking it (Phase 3, step 2).
//!
//! The file's ending picks the format: `render -o theme.wav` writes a WAV
//! as always, `render -o theme.ogg` an OGG. Music goes to Duck Forge as
//! OGG (about 14 times smaller, streamed while it plays); sound effects
//! stay WAV.
//!
//! An OGG is lossy, so Duck Echo does not trust it blindly. Before the
//! file is written, it is decoded again with `lewton` (the decoder Duck
//! Forge plays it with) and checked:
//!
//! * it must give back exactly as many frames as were rendered (the
//!   `DUCKECHO_FRAMES` comment makes sure of it);
//! * a loop must still join with no click (the same rule as for a WAV,
//!   [`measure::loop_seam`], on the decoded samples).
//!
//! Its facts file (`theme.ogg.json`) is measured on the decoded samples,
//! because that is what a game plays. The stream's serial number comes
//! from the recipe's hash, so the same recipe makes the same stream.
//!
//! The WAV stays the exact, pinned render. An OGG's bytes come from C
//! code with floating point, so they are not promised to be the same on
//! Windows and Linux (docs/phase3-design.md, question 3).

use std::path::Path;

use duck_echo_audio::ogg::{self as codec, OggSettings};
use duck_echo_audio::{AudioBuffer, WavError, WavFormat};
use duck_echo_core::{Code, Issue};
use serde::Serialize;

use crate::{analyze_buffer, asset, measure, save, write_wav_file, ToolError};

/// Quality used unless told otherwise (`oggenc -q 4`): about 105 kbit/s
/// for Duck Echo's music.
pub const DEFAULT_QUALITY: f32 = codec::DEFAULT_QUALITY;

/// A sound shorter than this, written as OGG, gets a warning: sound
/// effects stay WAV.
pub const SHORT_SECONDS: f64 = 5.0;

/// How a render is written. The file's ending says WAV or OGG; this holds
/// the settings for each. A plain [`WavFormat`] turns into one with no
/// quality, so every older caller still works.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderFormat {
    /// Sample format of a WAV (`pcm16`, `pcm24`, `float32`).
    pub wav: WavFormat,
    /// Quality of an OGG, 0 to 10 (default [`DEFAULT_QUALITY`]).
    pub quality: Option<f32>,
}

impl From<WavFormat> for RenderFormat {
    fn from(wav: WavFormat) -> Self {
        RenderFormat { wav, quality: None }
    }
}

/// What kind of file a render writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// `.wav`
    Wav,
    /// `.ogg`
    Ogg,
}

/// True when `path` ends in `.ogg` (any case).
pub fn is_ogg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ogg"))
}

/// The kind of file `output` names, or an error (with an empty path, for
/// the caller to name its argument) when it is neither.
pub fn output_kind(output: &Path) -> Result<OutputKind, ToolError> {
    if crate::has_wav_extension(output) {
        Ok(OutputKind::Wav)
    } else if is_ogg_path(output) {
        Ok(OutputKind::Ogg)
    } else {
        Err(ToolError::issue(
            Issue::error(
                Code::PathRefused,
                "",
                format!(
                    "render output {} must end in .wav (sound effects) or .ogg (music); refusing to write audio over another kind of file",
                    output.display()
                ),
            )
            .got(output.display().to_string())
            .one_of([".wav", ".ogg"]),
        ))
    }
}

/// Read an OGG file into a buffer, trimmed to its `DUCKECHO_FRAMES`.
pub fn read_ogg_file(path: &Path) -> Result<AudioBuffer, ToolError> {
    codec::read_ogg(path)
        .map(|d| d.buffer)
        .map_err(|e| match e {
            WavError::Io(io) => ToolError::failed(
                crate::io_code(&io),
                format!("cannot read {}: {io}", path.display()),
            ),
            WavError::Format(why) => ToolError::failed(
                Code::Io,
                format!(
                    "{} is not an OGG Vorbis file Duck Echo can read ({why})",
                    path.display()
                ),
            ),
        })
}

/// An OGG, decoded again and checked, as `render` reports it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OggCheck {
    /// The quality it was written at, 0 to 10.
    pub quality: f32,
    /// The OGG file's size in bytes.
    pub bytes: usize,
    /// The size the same sound has as a 16-bit WAV, for comparison.
    pub wav_bytes: usize,
    /// Frames rendered (and, after trimming to `DUCKECHO_FRAMES`,
    /// decoded: the check fails otherwise).
    pub frames: usize,
    /// Frames the decoder gave before trimming. More than `frames` for
    /// short files (see `duck_echo_audio::ogg`).
    pub decoded_frames: usize,
    /// For a loop: how the decoded end joins the decoded start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_seam: Option<measure::LoopSeam>,
}

/// What [`write_render`] wrote.
pub(crate) struct Written {
    /// For an OGG: the decoded samples, which the facts describe.
    decoded: Option<AudioBuffer>,
    /// For an OGG: the analysis of the decoded samples.
    pub(crate) analysis: Option<duck_echo_analyze::Analysis>,
    /// For an OGG: the check.
    pub(crate) check: Option<OggCheck>,
    /// A warning for the render's list (a short sound as OGG).
    pub(crate) warning: Option<Issue>,
}

impl Written {
    /// The samples the facts describe: the decoded OGG, or the render.
    pub(crate) fn buffer<'a>(&'a self, rendered: &'a AudioBuffer) -> &'a AudioBuffer {
        self.decoded.as_ref().unwrap_or(rendered)
    }

    /// Say in the facts what was written.
    pub(crate) fn mark(&self, info: &mut asset::AssetInfo) {
        if let Some(check) = &self.check {
            info.format = "ogg";
            info.quality = Some(check.quality);
        }
    }
}

/// Write a rendered `buffer` to `output` as `kind`. `data` is the
/// document's data (for the OGG's serial number); `loops` says it is a
/// seamless loop. Arguments in errors are `<prefix>quality` and
/// `<prefix>format`; problems with the output itself have an empty path.
pub(crate) fn write_render(
    output: &Path,
    kind: OutputKind,
    buffer: &AudioBuffer,
    format: RenderFormat,
    data: &serde_json::Value,
    loops: bool,
    prefix: &str,
) -> Result<Written, ToolError> {
    if kind == OutputKind::Wav {
        if let Some(q) = format.quality {
            return Err(ToolError::issue(
                Issue::error(
                    Code::InvalidArgument,
                    format!("{prefix}quality"),
                    "quality is for an .ogg output; a WAV is written exactly, with no quality to choose",
                )
                .got(q),
            ));
        }
        write_wav_file(output, buffer, format.wav)?;
        return Ok(Written {
            decoded: None,
            analysis: None,
            check: None,
            warning: None,
        });
    }

    if format.wav != WavFormat::Pcm16 {
        return Err(ToolError::issue(Issue::error(
            Code::InvalidArgument,
            format!("{prefix}format"),
            "pcm16, pcm24 and float32 are WAV formats; an .ogg output takes a quality instead",
        )));
    }
    let quality = format.quality.unwrap_or(DEFAULT_QUALITY);
    if !codec::QUALITY_RANGE.contains(&quality) {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                format!("{prefix}quality"),
                format!("quality is 0 (smallest) to 10 (best), got {quality}"),
            )
            .got(quality)
            .range(0.0, 10.0)
            .suggest(DEFAULT_QUALITY.to_string()),
        ));
    }
    let hash = asset::recipe_hash(data);
    let settings = OggSettings {
        quality,
        serial: codec::serial_from_hash(&hash),
        tags: vec![("DUCKECHO_RECIPE_HASH".into(), hash)],
    };
    let failed = |e: WavError| {
        ToolError::failed(
            Code::Io,
            format!("cannot write {} as OGG: {e}", output.display()),
        )
    };
    let bytes = codec::encode_ogg(buffer, &settings).map_err(failed)?;
    let decoded = codec::decode_ogg(&bytes).map_err(failed)?;

    // Checked before anything is written: a bad OGG is an error, not a file.
    let frames = buffer.frames();
    let back = &decoded.buffer;
    if back.frames() != frames
        || back.channels != buffer.channels
        || back.sample_rate != buffer.sample_rate
    {
        return Err(ToolError::failed(
            Code::Invalid,
            format!(
                "the OGG decodes to {} frames, {} channels at {} Hz, but {frames} frames, {} channels at {} Hz were rendered; nothing was written",
                back.frames(),
                back.channels,
                back.sample_rate,
                buffer.channels,
                buffer.sample_rate
            ),
        ));
    }
    let loop_seam = loops.then(|| measure::loop_seam(back));
    if let Some(seam) = loop_seam.as_ref().filter(|s| !s.clean) {
        return Err(ToolError::issue(
            Issue::error(
                Code::Invalid,
                "",
                format!(
                    "after encoding, the loop clicks where its end joins its start (a jump of {:.4} there, larger than any jump inside it, {:.4}); nothing was written. Try a higher quality, or send it as a WAV",
                    seam.seam_step, seam.largest_step
                ),
            )
            .suggest((quality + 2.0).min(10.0).to_string()),
        ));
    }
    let warning = (!loops && buffer.duration_seconds() < SHORT_SECONDS).then(|| {
        Issue::warning(
            Code::Suspicious,
            "",
            format!(
                "a {:.2} s sound written as OGG: sound effects stay WAV (a game holds them whole, ready at once); OGG is for music, which a game streams",
                buffer.duration_seconds()
            ),
        )
    });
    save::write_atomic(output, &bytes)?;
    let analysis = analyze_buffer(back);
    Ok(Written {
        check: Some(OggCheck {
            quality,
            bytes: bytes.len(),
            wav_bytes: 44 + frames * usize::from(buffer.channels) * 2,
            frames,
            decoded_frames: decoded.decoded_frames,
            loop_seam,
        }),
        decoded: Some(decoded.buffer),
        analysis: Some(analysis),
        warning,
    })
}
