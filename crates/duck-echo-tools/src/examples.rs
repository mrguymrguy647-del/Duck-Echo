//! The example songs, built into the program.
//!
//! Duck Echo Studio's gallery offers them to play and to copy, and
//! `file_copy_from` (and the MCP `create` tool's `from`) accepts
//! `example:NAME`, so a copy is one command and one Undo step, for a
//! person or an agent. They are the very files in `examples/songs`, so
//! they can never disagree.

use crate::ToolError;
use duck_echo_core::load::did_you_mean;
use duck_echo_core::{Code, Issue};

/// How an example is named after `from`: `example:duck_craft_day`.
pub const EXAMPLE_PREFIX: &str = "example:";

/// One example song.
#[derive(Debug)]
pub struct Example {
    /// Its name: the file name without `.toml`.
    pub name: &'static str,
    /// Its TOML text, with its comments.
    pub source: &'static str,
}

/// Every example song.
pub const EXAMPLES: &[Example] = &[
    Example {
        name: "duck_craft_day",
        source: include_str!("../../../examples/songs/duck_craft_day.toml"),
    },
    Example {
        name: "duck_craft_night",
        source: include_str!("../../../examples/songs/duck_craft_night.toml"),
    },
    Example {
        name: "four_bars",
        source: include_str!("../../../examples/songs/four_bars.toml"),
    },
];

/// The example called `name`, or an error that lists them all and
/// suggests the closest.
pub fn get(name: &str) -> Result<&'static Example, ToolError> {
    if let Some(example) = EXAMPLES.iter().find(|e| e.name == name) {
        return Ok(example);
    }
    let names: Vec<String> = EXAMPLES.iter().map(|e| e.name.to_string()).collect();
    let mut issue = Issue::error(
        Code::NotFound,
        "",
        format!("there is no example song named `{name}`"),
    )
    .got(name)
    .one_of(names.iter().cloned());
    if let Some(s) = did_you_mean(name, &names) {
        issue = issue.suggest(s);
    }
    Err(ToolError::issue(issue))
}

/// The `description` line of an example, for a gallery card.
pub fn description(example: &Example) -> String {
    example
        .source
        .lines()
        .find_map(|line| line.strip_prefix("description = "))
        .map(|d| d.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_examples_are_the_files_in_examples_songs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/songs");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        on_disk.sort();
        let built_in: Vec<&str> = EXAMPLES.iter().map(|e| e.name).collect();
        assert_eq!(built_in, on_disk, "add new example songs to EXAMPLES");
        for e in EXAMPLES {
            assert!(!description(e).is_empty(), "{}", e.name);
        }
        assert_eq!(
            get("duck_craft_dya").unwrap_err().into_report().errors[0]
                .suggestion
                .as_deref(),
            Some("duck_craft_day")
        );
    }
}
