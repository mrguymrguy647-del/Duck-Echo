//! The guide and the specs, built into the program.
//!
//! An agent that only has the MCP server, and not the Duck Echo
//! repository, could not read `docs/AGENT_GUIDE.md` in playtest 2. So the
//! guide and the reference pages are compiled in with `include_str!`, and
//! `get_guide` (MCP) and `duck-echo guide` (CLI) return them. They are the
//! same files as in the repository, so they can never disagree.

use crate::ToolError;
use duck_echo_core::load::did_you_mean;
use duck_echo_core::{Code, Issue};

const AGENT_GUIDE: &str = include_str!("../../../docs/AGENT_GUIDE.md");
const RECIPE_SPEC: &str = include_str!("../../../docs/RECIPE_SPEC.md");
const ANALYSIS: &str = include_str!("../../../docs/ANALYSIS.md");
const INSTRUMENTS: &str = include_str!("../../../instruments/README.md");
const PRESETS: &str = include_str!("../../../presets/README.md");

/// The heading in `RECIPE_SPEC.md` where the song part starts, and the one
/// after it, where the recipe part goes on.
const SONGS_HEADING: &str = "\n## Songs\n";
const AFTER_SONGS_HEADING: &str = "\n## Examples\n";

/// One page: its topic name and what it holds.
pub struct Topic {
    /// The name to ask for.
    pub name: &'static str,
    /// What the page is, in a few words.
    pub about: &'static str,
}

/// Every page, the one to read first first.
pub const TOPICS: &[Topic] = &[
    Topic { name: "guide", about: "The agent guide: the whole workflow for sound effects and music in one page. Read it first." },
    Topic { name: "recipe_spec", about: "Every recipe field, with its default and range." },
    Topic { name: "song_spec", about: "Every song field: tempo, bars, tracks, notes, patterns, instruments and loops." },
    Topic { name: "analysis", about: "Every analysis field and how it is measured." },
    Topic { name: "instruments", about: "The built-in instruments and the drum kit." },
    Topic { name: "presets", about: "The presets, their categories and loudness targets." },
];

/// The two parts of `RECIPE_SPEC.md`: recipes (everything but the songs
/// section) and songs.
fn spec_parts() -> (String, &'static str) {
    let songs = RECIPE_SPEC
        .find(SONGS_HEADING)
        .expect("RECIPE_SPEC.md has a Songs section");
    let after = RECIPE_SPEC[songs..]
        .find(AFTER_SONGS_HEADING)
        .map_or(RECIPE_SPEC.len(), |i| songs + i);
    let recipe = format!("{}{}", &RECIPE_SPEC[..songs + 1], &RECIPE_SPEC[after + 1..]);
    (recipe, &RECIPE_SPEC[songs + 1..after + 1])
}

/// The text of one page. An unknown topic is an error at `key` that lists
/// the topics and suggests the closest.
pub fn page(topic: &str, key: &str) -> Result<String, ToolError> {
    Ok(match topic {
        "guide" => AGENT_GUIDE.to_string(),
        "recipe_spec" => spec_parts().0,
        "song_spec" => spec_parts().1.to_string(),
        "analysis" => ANALYSIS.to_string(),
        "instruments" => INSTRUMENTS.to_string(),
        "presets" => PRESETS.to_string(),
        other => {
            let names: Vec<String> = TOPICS.iter().map(|t| t.name.to_string()).collect();
            let mut issue = Issue::error(
                Code::UnknownVariant,
                key,
                format!("there is no page `{other}`"),
            )
            .got(other)
            .one_of(names.iter().cloned());
            if let Some(s) = did_you_mean(other, &names) {
                issue = issue.suggest(s);
            }
            return Err(ToolError::issue(issue));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_has_its_page_and_the_spec_splits_in_two() {
        for t in TOPICS {
            let text = page(t.name, "topic").unwrap();
            assert!(text.len() > 1000, "{} is {} bytes", t.name, text.len());
            assert!(text.starts_with('#'), "{}", t.name);
        }
        let recipe = page("recipe_spec", "topic").unwrap();
        let song = page("song_spec", "topic").unwrap();
        assert!(song.starts_with("## Songs\n"));
        assert!(!song.contains("\n## Examples\n"));
        assert!(!recipe.contains("\n## Songs\n"));
        assert!(recipe.contains("\n## `layers[]`\n"));
        assert!(recipe.contains("\n## Examples\n"));
        // Nothing is lost or doubled by the split.
        assert_eq!(recipe.len() + song.len(), RECIPE_SPEC.len());
    }

    #[test]
    fn an_unknown_topic_suggests_the_closest() {
        let e = page("song_specs", "arguments.topic")
            .unwrap_err()
            .into_report();
        assert_eq!(e.errors[0].path, "arguments.topic");
        assert_eq!(e.errors[0].suggestion.as_deref(), Some("song_spec"));
    }
}
