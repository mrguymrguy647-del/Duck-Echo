//! The Studio's settings, and where its folders are.
//!
//! Settings are a small JSON file in the person's own settings folder, never
//! in the project folder: `%APPDATA%\Duck Echo Studio\settings.json` on
//! Windows, `$XDG_CONFIG_HOME/duck-echo-studio/settings.json` (usually
//! `~/.config/...`) on Linux.
//!
//! The project folder, where the person's sounds and songs go, is `Duck
//! Echo` in their Documents: Windows' real Documents folder (even when
//! OneDrive moved it), or the XDG Documents folder on Linux (usually
//! `~/Documents`). Installing, updating and uninstalling never touch it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The settings, as saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The file these settings are saved in. `None` (as in
    /// [`Settings::default`]) saves nothing, so a test never touches a
    /// person's settings.
    #[serde(skip)]
    pub file: Option<PathBuf>,
    /// "dark", "light" or "system".
    pub theme: Option<String>,
    /// Look for a newer Studio when it starts. On unless turned off.
    pub check_for_updates: bool,
    /// Another project folder than `Documents/Duck Echo`, if chosen.
    pub project_folder: Option<PathBuf>,
    /// Linux: the Studio has asked "Add Duck Echo Studio to your app
    /// menu?" once, so it does not ask again.
    pub asked_about_app_menu: bool,
    /// The Duck Forge folder sounds are sent to, if chosen (otherwise
    /// `Duck Forge` in Documents, when it is one).
    pub duck_forge_folder: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            file: None,
            theme: None,
            check_for_updates: true,
            project_folder: None,
            asked_about_app_menu: false,
            duck_forge_folder: None,
        }
    }
}

/// The folder the Studio keeps its settings in, on this system.
pub fn settings_folder() -> Option<PathBuf> {
    // dirs::config_dir is %APPDATA% on Windows and $XDG_CONFIG_HOME (or
    // ~/.config) on Linux.
    let config = dirs::config_dir()?;
    Some(if cfg!(windows) {
        config.join("Duck Echo Studio")
    } else {
        config.join("duck-echo-studio")
    })
}

/// Where the settings are saved. `DUCK_ECHO_STUDIO_SETTINGS` names another
/// file (tests and CI use it, so they never touch a person's settings).
pub fn settings_file() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("DUCK_ECHO_STUDIO_SETTINGS") {
        return Some(PathBuf::from(file));
    }
    Some(settings_folder()?.join("settings.json"))
}

impl Settings {
    /// The saved settings, or fresh ones if there are none. A broken
    /// settings file never stops the Studio: it starts with fresh ones.
    pub fn load() -> Settings {
        let file = settings_file();
        let mut settings: Settings = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        settings.file = file;
        settings
    }

    /// Save them in their file, if they have one, in one step (a temporary
    /// file renamed into place). Failing is fine: the Studio works without.
    pub fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = duck_echo_tools::save::write_atomic(file, text.as_bytes());
        }
    }

    /// The project folder: the one chosen, or `DUCK_ECHO_PROJECT` (for
    /// tests and CI), or `Duck Echo` in Documents.
    pub fn project_folder(&self) -> PathBuf {
        if let Some(folder) = std::env::var_os("DUCK_ECHO_PROJECT") {
            return PathBuf::from(folder);
        }
        if let Some(folder) = &self.project_folder {
            return folder.clone();
        }
        default_project_folder()
    }
}

impl Settings {
    /// The Duck Forge folder sounds are sent to: `DUCK_ECHO_DUCK_FORGE`
    /// (as for the command line, and for tests), the one chosen, or `Duck
    /// Forge` in Documents (where Duck Forge Studio keeps a person's
    /// games). `None` when none of them is a Duck Forge folder.
    pub fn duck_forge(&self) -> Option<duck_echo_tools::forge::DuckForge> {
        use duck_echo_tools::forge::{DuckForge, ENV_VARIABLE};
        if let Some(folder) = std::env::var_os(ENV_VARIABLE) {
            return DuckForge::open(Path::new(&folder)).ok();
        }
        let folder = self.duck_forge_folder.clone().or_else(|| {
            dirs::document_dir()
                .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
                .map(|documents| documents.join("Duck Forge"))
        })?;
        DuckForge::open(&folder).ok()
    }
}

/// `Duck Echo` in the person's Documents folder. On a Linux system whose
/// XDG folders are not set up, that is `~/Documents`.
pub fn default_project_folder() -> PathBuf {
    dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("Documents"))
        .join("Duck Echo")
}

/// Make the project folder if it is not there yet. Returns a sentence for
/// the person if it cannot be made.
pub fn make_project_folder(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| {
        format!(
            "The folder for your sounds, {}, could not be made: {error}",
            folder.display()
        )
    })
}

/// Open `folder` in the system's file manager.
pub fn open_folder(folder: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(folder).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_duck_forge_folder_is_the_chosen_one_when_it_is_duck_forges() {
        if std::env::var_os(duck_echo_tools::forge::ENV_VARIABLE).is_some() {
            return; // the variable wins, as on the command line
        }
        let dir = std::env::temp_dir().join(format!("studio-forge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("examples")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"duckforge\"\n").unwrap();
        let chosen = Settings {
            duck_forge_folder: Some(dir.clone()),
            ..Settings::default()
        };
        let forge = chosen.duck_forge().expect("a Duck Forge folder");
        assert_eq!(forge.dir(), dir.canonicalize().unwrap());
        // A folder that is not Duck Forge's is not used.
        std::fs::remove_file(dir.join("Cargo.toml")).unwrap();
        assert!(chosen.duck_forge().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_survive_being_saved_and_read_and_old_files_still_load() {
        let folder =
            std::env::temp_dir().join(format!("duck-echo-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let file = folder.join("deep").join("settings.json");
        let settings = Settings {
            file: Some(file.clone()),
            theme: Some("light".into()),
            check_for_updates: false,
            project_folder: Some(PathBuf::from("/music")),
            asked_about_app_menu: true,
            duck_forge_folder: Some(PathBuf::from("/games/Duck Forge")),
        };
        settings.save();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("\"file\""),
            "where it is saved is not saved: {text}"
        );
        let read: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(
            read,
            Settings {
                file: None,
                ..settings
            }
        );
        // A file from an older Studio, with only some fields.
        let old: Settings = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        assert!(old.check_for_updates, "on unless turned off");
        assert!(!old.asked_about_app_menu);
        // Default settings have no file and save nothing.
        Settings::default().save();
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn the_project_folder_is_duck_echo_in_documents_unless_chosen() {
        let settings = Settings::default();
        if std::env::var_os("DUCK_ECHO_PROJECT").is_none() {
            assert!(settings.project_folder().ends_with("Duck Echo"));
        }
        let chosen = Settings {
            project_folder: Some(PathBuf::from("/my/sounds")),
            ..Settings::default()
        };
        if std::env::var_os("DUCK_ECHO_PROJECT").is_none() {
            assert_eq!(chosen.project_folder(), PathBuf::from("/my/sounds"));
        }
        let name = if cfg!(windows) {
            "Duck Echo Studio"
        } else {
            "duck-echo-studio"
        };
        if let Some(folder) = settings_folder() {
            assert!(folder.ends_with(name));
        }
    }
}
