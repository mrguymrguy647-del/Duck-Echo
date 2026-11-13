//! The built-in starter instruments: recipes compiled into the program,
//! like the presets, so a song can name one (`instrument = "piano"`)
//! without anything else on disk.
//!
//! Each instrument is a normal recipe in the `instruments/` folder. The
//! drum kit `drums` is a list of four of them.

use crate::ToolError;
use duck_echo_core::note::{note_name, note_number};
use duck_echo_core::song::home_hz;
use duck_echo_core::{load_str, Code, Format, Issue, Loaded};
use serde::Serialize;

/// Prefix that names a built-in instrument where a recipe path is
/// expected, as in `duck-echo render instrument:piano`.
pub const INSTRUMENT_PREFIX: &str = "instrument:";

/// A built-in instrument: its name, its recipe, and the notes it is good
/// for.
#[derive(Debug, Clone, Copy)]
pub struct BuiltIn {
    /// Name, equal to the file stem and the recipe `name`.
    pub name: &'static str,
    /// The recipe as written, comments included.
    pub source: &'static str,
    /// The lowest and highest notes it sounds right at. `None` for drum
    /// pieces, which are not played at a pitch.
    pub range: Option<(&'static str, &'static str)>,
}

macro_rules! instruments {
    ($(($name:literal, $range:expr)),* $(,)?) => {
        /// Every built-in instrument, in alphabetical order.
        pub const INSTRUMENTS: &[BuiltIn] = &[
            $(BuiltIn {
                name: $name,
                source: include_str!(concat!("../../../instruments/", $name, ".toml")),
                range: $range,
            }),*
        ];
    };
}

instruments!(
    ("bass", Some(("E1", "C4"))),
    ("chip_lead", Some(("C3", "C7"))),
    ("clap", None),
    ("hat", None),
    ("kick", None),
    ("pad", Some(("C3", "C6"))),
    ("piano", Some(("C2", "C7"))),
    ("snare", None),
);

/// A built-in drum kit: a name and its pieces (piece name, instrument).
#[derive(Debug, Clone, Copy)]
pub struct BuiltInKit {
    /// The kit's name.
    pub name: &'static str,
    /// One-sentence description.
    pub description: &'static str,
    /// Piece name and the built-in instrument that plays it.
    pub pieces: &'static [(&'static str, &'static str)],
}

/// Every built-in kit.
pub const KITS: &[BuiltInKit] = &[BuiltInKit {
    name: "drums",
    description: "A drum kit: write `note = \"kick\"`, \"snare\", \"hat\" or \"clap\".",
    pieces: &[
        ("clap", "clap"),
        ("hat", "hat"),
        ("kick", "kick"),
        ("snare", "snare"),
    ],
}];

/// What a listing shows about an instrument or a kit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstrumentInfo {
    /// The name a song's track writes as `instrument`.
    pub name: String,
    /// `instrument` or `kit`.
    pub kind: String,
    /// One-sentence description.
    pub description: String,
    /// Tags.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The note the recipe is written at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The lowest and highest notes it sounds right at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<[String; 2]>,
    /// The loudness the instrument is levelled to when played alone at its
    /// home note, LUFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_lufs: Option<f32>,
    /// For a kit: the names its notes can use.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<String>,
}

/// Find a built-in instrument by name.
pub fn find(name: &str) -> Option<&'static BuiltIn> {
    INSTRUMENTS.iter().find(|i| i.name == name)
}

/// Find a built-in kit by name.
pub fn find_kit(name: &str) -> Option<&'static BuiltInKit> {
    KITS.iter().find(|k| k.name == name)
}

/// Every name a track can write: instruments, then kits.
pub fn names() -> Vec<String> {
    INSTRUMENTS
        .iter()
        .map(|i| i.name.to_string())
        .chain(KITS.iter().map(|k| k.name.to_string()))
        .collect()
}

/// A `not_found` error for `name`, with every name as the allowed options
/// and the closest one as the suggestion.
pub fn unknown(name: &str) -> Issue {
    let names = names();
    let mut issue = Issue::error(
        Code::NotFound,
        "",
        format!("no built-in instrument named `{name}`; `allowed` lists every one"),
    )
    .got(name)
    .one_of(names.clone());
    if let Some(s) = duck_echo_core::load::did_you_mean(name, &names) {
        issue = issue.suggest(s);
    }
    issue
}

/// A built-in instrument by name. A kit is not one recipe, so asking for
/// it here is an error that names its pieces.
pub fn get(name: &str) -> Result<&'static BuiltIn, ToolError> {
    if let Some(found) = find(name) {
        return Ok(found);
    }
    if let Some(kit) = find_kit(name) {
        let pieces: Vec<String> = kit.pieces.iter().map(|(_, i)| i.to_string()).collect();
        return Err(ToolError::issue(
            Issue::error(
                Code::Invalid,
                "",
                format!(
                    "`{name}` is a kit, not one sound; its pieces are instruments of their own: {}",
                    pieces.join(", ")
                ),
            )
            .got(name)
            .one_of(pieces),
        ));
    }
    Err(ToolError::issue(unknown(name)))
}

/// Parse and validate a built-in instrument.
pub fn load(instrument: &BuiltIn) -> Result<Loaded, ToolError> {
    Ok(load_str(instrument.source, Format::Toml)?)
}

/// Listing info for an instrument.
pub fn info(instrument: &BuiltIn) -> Result<InstrumentInfo, ToolError> {
    let recipe = load(instrument)?.recipe;
    // The home note as a name: the note whose frequency is the recipe's
    // first pitched layer. Drum pieces have no range and show no home.
    let home = instrument.range.and_then(|_| {
        let hz = home_hz(&recipe)? as f64;
        note_name((69.0 + 12.0 * (hz / 440.0).log2()).round() as i32)
    });
    Ok(InstrumentInfo {
        name: instrument.name.to_string(),
        kind: "instrument".into(),
        description: recipe.description.unwrap_or_default(),
        tags: recipe.tags,
        home,
        range: instrument
            .range
            .map(|(lo, hi)| [lo.to_string(), hi.to_string()]),
        target_lufs: recipe.master.normalize_lufs,
        pieces: Vec::new(),
    })
}

/// Every instrument and kit, for a listing.
pub fn list() -> Result<Vec<InstrumentInfo>, ToolError> {
    let mut out: Vec<InstrumentInfo> = INSTRUMENTS.iter().map(info).collect::<Result<_, _>>()?;
    out.extend(KITS.iter().map(|kit| InstrumentInfo {
        name: kit.name.to_string(),
        kind: "kit".into(),
        description: kit.description.to_string(),
        tags: vec!["instrument".into(), "drum".into(), "kit".into()],
        home: None,
        range: None,
        target_lufs: None,
        pieces: kit.pieces.iter().map(|(p, _)| p.to_string()).collect(),
    }));
    Ok(out)
}

/// True when `note` (a note name) is inside the instrument's useful range.
/// Always true for an instrument without a range.
pub fn in_range(instrument: &BuiltIn, note: &str) -> bool {
    let (Some((lo, hi)), Ok(n)) = (instrument.range, note_number(note)) else {
        return true;
    };
    match (note_number(lo), note_number(hi)) {
        (Ok(lo), Ok(hi)) => (lo..=hi).contains(&n),
        _ => true,
    }
}
