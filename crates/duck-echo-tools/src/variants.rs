//! Export a sound's variants for a game or film to pick from at random:
//! `step_1.wav` ... `step_N.wav`, each with its facts file
//! (`step_1.wav.json`, see [`crate::asset`]), and one list,
//! `step.variants.json`, naming them all.
//!
//! Variant `i` is the recipe rendered with seed `i`, so a variant is the
//! same file every time it is exported. What changes between seeds is what
//! the recipe lets the seed change: its random ranges
//! (`volume = { min = 0.6, max = 0.9 }`), its `variation`, and its noise.
//!
//! Made for Duck Forge while making the sound of "Pick a Door: Blocks":
//! ten footsteps that were one file played ten times sounded like a
//! machine.

use std::path::Path;

use serde::Serialize;

use crate::{asset, file_safe, RecipeRef, ToolError, MAX_VARIATIONS, SAME_FOR_EVERY_SEED};
use duck_echo_core::{Code, Issue};

/// One exported variant, as the list names it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Variant {
    /// The WAV's name, in the list's folder: `step_3.wav`.
    pub file: String,
    /// The seed it was rendered with (its number).
    pub seed: u64,
    /// Length in seconds.
    pub duration_seconds: f64,
    /// Loudness in LUFS (`null` when silent).
    pub loudness_lufs: Option<f32>,
    /// Largest sample, dBFS (`null` when silent).
    pub peak_dbfs: Option<f32>,
    /// When its main hit lands, seconds from the start (`null` when
    /// silent).
    pub hit_seconds: Option<f64>,
    /// The analysis summary line.
    pub summary: String,
}

/// The list of variants: what is written to `<name>.variants.json`, and
/// the command's answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VariantList {
    /// Always `duck-echo-variants`.
    pub kind: &'static str,
    /// The version of this layout: 1.
    pub version: u32,
    /// The recipe the variants came from, as it was named.
    pub recipe: String,
    /// `sha256:` and the SHA-256 of the recipe's data (as in each
    /// variant's facts file, but without its seed).
    pub recipe_hash: String,
    /// How to use the list: `random`, any one of `files` each time the
    /// sound plays (Duck Forge also avoids playing the same one twice in a
    /// row).
    pub pick: &'static str,
    /// False when nothing in the recipe depends on the seed, so every
    /// variant is the same audio; `note` says what to change.
    pub seeds_matter: bool,
    /// Why every variant is the same, when it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Quietest and loudest variant, LUFS, so a mix can see the spread.
    pub loudness_range_lufs: Option<[f32; 2]>,
    /// The variants, in seed order.
    pub files: Vec<Variant>,
}

/// What the export wrote, beside the list itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Exported {
    /// Always true; failures are errors instead.
    pub ok: bool,
    /// The list file: `sfx/step.variants.json`.
    pub list_file: String,
    /// The folder the files are in.
    pub folder: String,
    /// The list, as written.
    pub list: VariantList,
}

/// What to export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request<'a> {
    /// How many variants, 1..=64.
    pub count: u32,
    /// The files' name before `_1.wav`; default the recipe's stem.
    pub name: Option<&'a str>,
    /// Replaces the recipe's own `variation`.
    pub variation: Option<f32>,
}

/// Render seeds `1..=count` of `recipe` into `folder` as
/// `<name>_<seed>.wav` with their facts files, and write
/// `<name>.variants.json`. `shown` is how paths are written in the
/// answer; `recipe_name` is how the list names the recipe. Arguments in
/// errors are `<prefix>count` and `<prefix>variation`.
pub fn export(
    recipe: &RecipeRef,
    recipe_name: &str,
    request: &Request,
    folder: &Path,
    prefix: &str,
    shown: &dyn Fn(&Path) -> String,
) -> Result<Exported, ToolError> {
    let Request {
        count,
        name,
        variation,
    } = *request;
    if !(1..=MAX_VARIATIONS).contains(&count) {
        return Err(ToolError::issue(
            Issue::error(
                Code::OutOfRange,
                format!("{prefix}count"),
                format!("count must be 1..={MAX_VARIATIONS}, got {count}"),
            )
            .got(count)
            .range(1.0, f64::from(MAX_VARIATIONS)),
        ));
    }
    let mut base = recipe.loaded.recipe.clone();
    if let Some(v) = variation {
        if !(0.0..=1.0).contains(&v) {
            return Err(ToolError::issue(
                Issue::error(
                    Code::OutOfRange,
                    format!("{prefix}variation"),
                    format!("variation must be 0..=1, got {v}"),
                )
                .got(v)
                .range(0.0, 1.0)
                .suggest("0.3"),
            ));
        }
        base.variation = v;
    }
    let report = duck_echo_core::validate(&base);
    if !report.is_ok() {
        return Err(ToolError::Invalid(report));
    }
    let name = file_safe(name.unwrap_or(&recipe.stem));
    std::fs::create_dir_all(folder).map_err(|e| {
        ToolError::failed(
            Code::Io,
            format!("cannot make the folder {}: {e}", shown(folder)),
        )
    })?;
    let uses_noise = base
        .layers
        .iter()
        .any(|l| duck_echo_synth::source_uses_seed(l.source));
    let seeds_matter = base.variation > 0.0 || uses_noise || !base.ranges.is_empty();

    let mut files = Vec::with_capacity(count as usize);
    for seed in 1..=u64::from(count) {
        let mut r = base.clone();
        r.seed = seed;
        let buffer = duck_echo_synth::render(&r)?;
        let analysis = crate::analyze_buffer(&buffer);
        let file = format!("{name}_{seed}.wav");
        let path = folder.join(&file);
        crate::write_wav_file(&path, &buffer, duck_echo_audio::WavFormat::Pcm16)?;
        let info = asset::info(
            &buffer,
            &analysis,
            &path,
            &asset::recipe_data(&r),
            seed,
            false,
        );
        asset::write_sidecar(&path, &info)?;
        files.push(Variant {
            file,
            seed,
            duration_seconds: info.duration_seconds,
            loudness_lufs: info.loudness_lufs,
            peak_dbfs: info.peak_dbfs,
            hit_seconds: info.hit_seconds,
            summary: analysis.summary,
        });
    }
    let loud: Vec<f32> = files.iter().filter_map(|f| f.loudness_lufs).collect();
    let loudness_range_lufs = (!loud.is_empty()).then(|| {
        [
            loud.iter().copied().fold(f32::INFINITY, f32::min),
            loud.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        ]
    });
    let mut data = asset::recipe_data(&base);
    if let Some(obj) = data.as_object_mut() {
        obj.remove("seed");
    }
    let list = VariantList {
        kind: "duck-echo-variants",
        version: 1,
        recipe: recipe_name.to_string(),
        recipe_hash: format!("sha256:{}", asset::sha256_hex(data.to_string().as_bytes())),
        pick: "random",
        seeds_matter,
        note: (!seeds_matter).then(|| SAME_FOR_EVERY_SEED.to_string()),
        loudness_range_lufs,
        files,
    };
    let list_path = folder.join(format!("{name}.variants.json"));
    let text = serde_json::to_string_pretty(&list).expect("plain JSON") + "\n";
    crate::save::write_atomic(&list_path, text.as_bytes())?;
    Ok(Exported {
        ok: true,
        list_file: shown(&list_path),
        folder: shown(folder),
        list,
    })
}
