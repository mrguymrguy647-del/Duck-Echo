//! The preset library: tagged starting-point recipes compiled into the
//! binary, so the CLI and MCP server find them from any directory.
//!
//! Conventions (checked by tests): the file name is the recipe `name`; the
//! recipe has a `description` and exactly one category tag from
//! [`CATEGORIES`]; it is peak-normalized to -3 dBFS and trims its silent
//! tail, so every preset starts at a predictable level and cannot clip.

use crate::ToolError;
use duck_echo_core::{load_str, Format, Loaded};
use serde::{Deserialize, Serialize};

/// Category tags. Every preset has exactly one.
pub const CATEGORIES: &[&str] = &[
    "ui", "pickup", "weapon", "impact", "movement", "alert", "ambient",
];

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
    /// One-sentence description.
    pub description: String,
    /// All tags, category first.
    pub tags: Vec<String>,
}

/// Find a preset by name.
pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// A preset by name, or an error listing the closest names.
pub fn get(name: &str) -> Result<&'static Preset, ToolError> {
    find(name).ok_or_else(|| {
        let names: Vec<String> = PRESETS.iter().map(|p| p.name.to_string()).collect();
        let hint = duck_echo_core::load::did_you_mean(name, &names)
            .map(|s| format!(" Did you mean `{s}`?"))
            .unwrap_or_default();
        ToolError::Io(format!(
            "no preset named `{name}`.{hint} List them with `duck-echo presets list`."
        ))
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
