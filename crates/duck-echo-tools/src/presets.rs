//! The preset library: tagged starting-point recipes compiled into the
//! binary, so the CLI and MCP server find them from any directory.
//!
//! Conventions (checked by tests): the file name is the recipe `name`; the
//! recipe has a `description` and exactly one category tag from
//! [`CATEGORIES`]; it is levelled to its category's loudness target in
//! [`LOUDNESS_TARGETS`] (through `master.normalize_peak_db`, never above
//! -1 dBFS peak) and trims its silent tail.

use crate::ToolError;
use duck_echo_core::{load_str, Format, Loaded};
use serde::{Deserialize, Serialize};

/// Category tags. Every preset has exactly one.
pub const CATEGORIES: &[&str] = &[
    "ui", "pickup", "weapon", "impact", "movement", "alert", "ambient",
];

/// Loudness target per category, in LUFS (as `analyze` measures it:
/// gated for clips of 400 ms or more, `k_weighted_ungated` below). Impacts
/// are the loudest a typical explosion can be with 1 dB of peak headroom
/// (its peak sits about 14 dB above its loudness); the others step down so
/// that feedback sounds never mask gameplay and ambient beds sit under
/// everything. They are starting levels for a game mix, not a mix.
pub const LOUDNESS_TARGETS: &[(&str, f32)] = &[
    ("impact", -16.0),
    ("weapon", -17.0),
    ("alert", -18.0),
    ("pickup", -19.0),
    ("movement", -20.0),
    ("ui", -22.0),
    ("ambient", -24.0),
];

/// The loudness target for a category.
pub fn target_lufs(category: &str) -> Option<f32> {
    LOUDNESS_TARGETS
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, t)| *t)
}

/// A preset's name and TOML source.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// Name, equal to the file stem and the recipe `name`.
    pub name: &'static str,
    /// The recipe as written, comments included.
    pub source: &'static str,
}

macro_rules! presets {
    ($($name:literal),* $(,)?) => {
        /// Every preset, in alphabetical order.
        pub const PRESETS: &[Preset] = &[
            $(Preset {
                name: $name,
                source: include_str!(concat!("../../../presets/", $name, ".toml")),
            }),*
        ];
    };
}

presets!(
    "alarm",
    "big_explosion",
    "blaster",
    "charge_up",
    "coin",
    "engine_hum",
    "explosion",
    "extra_life",
    "footstep",
    "gem",
    "hit",
    "jump",
    "key_pickup",
    "land",
    "laser",
    "low_health",
    "menu_open",
    "metal_clang",
    "missile",
    "powerup",
    "punch",
    "shotgun",
    "thud",
    "ui_cancel",
    "ui_click",
    "ui_confirm",
    "ui_error",
    "ui_hover",
    "warning_beep",
    "wind",
);

/// What a listing shows about a preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetInfo {
    /// Preset name; load it as `preset:<name>`.
    pub name: String,
    /// The category tag.
    pub category: String,
    /// The category's loudness target in LUFS; the preset is levelled to it.
    pub target_lufs: Option<f32>,
    /// One-sentence description.
    pub description: String,
    /// All tags, category first.
    pub tags: Vec<String>,
}

/// Find a preset by name.
pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// A preset by name, or a `not_found` error with every preset name as the
/// allowed options and the closest one as the suggestion. The message names
/// no command, since the CLI and the MCP server list presets differently.
pub fn get(name: &str) -> Result<&'static Preset, ToolError> {
    find(name).ok_or_else(|| {
        let names: Vec<String> = PRESETS.iter().map(|p| p.name.to_string()).collect();
        let mut issue = duck_echo_core::Issue::error(
            duck_echo_core::Code::NotFound,
            "",
            format!("no preset named `{name}`; `allowed` lists every preset"),
        )
        .got(name)
        .one_of(names.clone());
        if let Some(s) = duck_echo_core::load::did_you_mean(name, &names) {
            issue = issue.suggest(s);
        }
        ToolError::issue(issue)
    })
}

/// Parse and validate a preset.
pub fn load(preset: &Preset) -> Result<Loaded, ToolError> {
    Ok(load_str(preset.source, Format::Toml)?)
}

/// Listing info for a preset.
pub fn info(preset: &Preset) -> Result<PresetInfo, ToolError> {
    let recipe = load(preset)?.recipe;
    let category = recipe
        .tags
        .iter()
        .find(|t| CATEGORIES.contains(&t.as_str()))
        .cloned()
        .unwrap_or_default();
    Ok(PresetInfo {
        name: preset.name.to_string(),
        target_lufs: target_lufs(&category),
        category,
        description: recipe.description.unwrap_or_default(),
        tags: recipe.tags,
    })
}

/// Listing of all presets, or of those carrying `tag`.
pub fn list(tag: Option<&str>) -> Result<Vec<PresetInfo>, ToolError> {
    let mut out = Vec::new();
    for p in PRESETS {
        let i = info(p)?;
        if tag.map_or(true, |t| i.tags.iter().any(|x| x == t)) {
            out.push(i);
        }
    }
    Ok(out)
}
