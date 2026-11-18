//! `DuckEchoStudio --open FILE`: open one recipe or song, as Duck Forge
//! Studio's "Open in Duck Echo" does (docs/phase3-design.md, section 7).
//!
//! When the file is in a Duck Forge game (`.../Duck Forge/examples/
//! duck_craft/art/step.toml`), the Studio works on that game:
//!
//! * the game's folder is the project for this window, and Home shows the
//!   recipes in its `art` folder;
//! * Send to game sends to this game first;
//! * Duck Echo's Undo history for it is kept in Duck Echo's own settings
//!   folder (`projects/duck_craft-1a2b3c4d`), not in the game, so nothing
//!   of Duck Echo's shows up in Duck Forge Studio's Files or History.
//!
//! Any other file opens with its own folder as the project, as a normal
//! Duck Echo project (history in its `.duck-echo` folder).
//!
//! The person's chosen project folder is not changed: the next plain
//! start opens it as before.

use std::path::{Path, PathBuf};

use duck_echo_tools::forge::DuckForge;

/// What `--open FILE` opens.
#[derive(Debug, Clone, PartialEq)]
pub struct Opening {
    /// The project folder for this window.
    pub project: PathBuf,
    /// The file, from the project folder, with `/` (`art/step.toml`).
    pub file: String,
    /// The Duck Forge game it is in, if it is in one.
    pub game: Option<String>,
    /// That game's Duck Forge folder.
    pub duck_forge: Option<DuckForge>,
    /// Where its Undo history goes, when not in the project folder.
    pub own_folder: Option<PathBuf>,
}

impl Opening {
    /// The window's title.
    pub fn title(&self) -> String {
        match &self.game {
            Some(game) => format!("Duck Echo Studio - {game} (a Duck Forge game)"),
            None => {
                let folder = self
                    .project
                    .file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                format!("Duck Echo Studio - {folder}")
            }
        }
    }
}

/// Work out what to open for `file`. `settings_folder` is Duck Echo's own
/// settings folder (the Undo history of a game goes in it). The error is
/// a sentence for the person.
pub fn opening(file: &Path, settings_folder: Option<&Path>) -> Result<Opening, String> {
    let is_document = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("toml") || e.eq_ignore_ascii_case("json"));
    if !is_document {
        return Err(format!(
            "{} is not a Duck Echo sound or song: open a .toml or .json file",
            file.display()
        ));
    }
    let file = file
        .canonicalize()
        .map_err(|e| format!("{} cannot be opened: {e}", file.display()))?;
    if !file.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    // A Duck Forge game: examples/<game>/ with a main.rs, in a Duck Forge
    // folder. The nearest one above the file.
    let game_dir = file.ancestors().skip(1).find(|dir| {
        dir.join("main.rs").is_file()
            && dir
                .parent()
                .is_some_and(|examples| examples.file_name().is_some_and(|n| n == "examples"))
            && dir
                .parent()
                .and_then(Path::parent)
                .is_some_and(|forge| DuckForge::open(forge).is_ok())
    });
    let rel = |project: &Path| -> String {
        file.strip_prefix(project)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/")
    };
    match game_dir {
        Some(game_dir) => {
            let game = game_dir
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let forge = game_dir
                .parent()
                .and_then(Path::parent)
                .and_then(|f| DuckForge::open(f).ok());
            // One history folder per game folder: its name and a short
            // fingerprint of where it is, so two copies of a game do not
            // share one.
            let fingerprint: String =
                duck_echo_tools::asset::sha256_hex(game_dir.to_string_lossy().as_bytes())
                    .chars()
                    .take(8)
                    .collect();
            let own_folder =
                settings_folder.map(|s| s.join("projects").join(format!("{game}-{fingerprint}")));
            Ok(Opening {
                file: rel(game_dir),
                project: game_dir.to_path_buf(),
                game: Some(game),
                duck_forge: forge,
                own_folder,
            })
        }
        None => {
            let project = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            Ok(Opening {
                file: rel(&project),
                project,
                game: None,
                duck_forge: None,
                own_folder: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_in_a_duck_forge_game_opens_the_game() {
        let dir = std::env::temp_dir().join(format!("studio-opening-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let game = dir.join("Duck Forge/examples/duck_craft");
        std::fs::create_dir_all(game.join("art")).unwrap();
        std::fs::write(
            dir.join("Duck Forge/Cargo.toml"),
            "[package]\nname = \"duckforge\"\n",
        )
        .unwrap();
        std::fs::write(game.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(game.join("art/step.toml"), "version = 0\n").unwrap();
        let settings = dir.join("settings");

        let o = opening(&game.join("art/step.toml"), Some(&settings)).unwrap();
        assert_eq!(o.project, game.canonicalize().unwrap());
        assert_eq!(o.file, "art/step.toml");
        assert_eq!(o.game.as_deref(), Some("duck_craft"));
        assert!(o.duck_forge.is_some());
        let own = o.own_folder.clone().unwrap();
        assert!(own.starts_with(settings.join("projects")));
        assert!(
            own.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("duck_craft-")
        );
        assert_eq!(
            o.title(),
            "Duck Echo Studio - duck_craft (a Duck Forge game)"
        );

        // Elsewhere: the file's own folder, as a plain project.
        std::fs::write(dir.join("loose.toml"), "version = 0\n").unwrap();
        let o = opening(&dir.join("loose.toml"), Some(&settings)).unwrap();
        assert_eq!(o.file, "loose.toml");
        assert!(o.game.is_none() && o.own_folder.is_none());

        // Not a recipe, or not there.
        assert!(opening(&game.join("main.rs"), None).is_err());
        assert!(opening(&dir.join("missing.toml"), None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
