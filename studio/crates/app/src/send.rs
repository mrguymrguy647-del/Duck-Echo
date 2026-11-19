//! Send to game: the sound and song screens' `...` menu sends the open
//! file into a Duck Forge game with the `game_send` command (Phase 3,
//! step 4). Settings chooses the Duck Forge folder.
//!
//! The menu lists the games of the Duck Forge folder. The game this file
//! was last sent to comes first, marked "(last time)", and so does the
//! game this window was opened on (`--open`), marked "(this game)". The
//! last game of each file is kept in the settings, by the file's full
//! path.

use std::path::Path;

use eframe::egui;
use serde_json::json;

use crate::StudioApp;

/// What the person chose in the Send to game menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendChoice {
    /// Send to this game.
    Game(String),
    /// Choose the Duck Forge folder first.
    ChooseFolder,
}

impl StudioApp {
    /// The key a file's last game is kept under: its full path.
    fn last_game_key(&self, rel: &str) -> String {
        self.project.join(rel).to_string_lossy().into_owned()
    }

    /// The game `rel` was last sent to, if any.
    pub fn last_game(&self, rel: &str) -> Option<&str> {
        self.settings
            .last_game
            .get(&self.last_game_key(rel))
            .map(String::as_str)
    }

    /// The games to offer for `rel`, in order, each with its mark: the game
    /// this window is on and the file's last game first, then the rest.
    pub fn games_for(&self, rel: &str) -> Vec<(String, &'static str)> {
        let Some(forge) = self.root.as_ref().and_then(|r| r.duck_forge()) else {
            return Vec::new();
        };
        let games = forge.games();
        let mut out: Vec<(String, &'static str)> = Vec::new();
        let first = |name: &str, mark: &'static str, out: &mut Vec<(String, &'static str)>| {
            if games.iter().any(|g| g == name) && !out.iter().any(|(g, _)| g == name) {
                out.push((name.to_string(), mark));
            }
        };
        if let Some(game) = self.game.clone() {
            first(&game, "(this game)", &mut out);
        }
        if let Some(last) = self.last_game(rel).map(str::to_string) {
            first(&last, "(last time)", &mut out);
        }
        for game in games {
            if !out.iter().any(|(g, _)| *g == game) {
                out.push((game, ""));
            }
        }
        out
    }

    /// Draw the Send to game menu for `rel` inside a `...` menu. Returns
    /// what was chosen.
    pub(crate) fn send_menu(&self, ui: &mut egui::Ui, rel: &str) -> Option<SendChoice> {
        let mut chosen = None;
        ui.menu_button("Send to game", |ui| {
            let games = self.games_for(rel);
            if self.root.as_ref().and_then(|r| r.duck_forge()).is_none() {
                ui.label("No Duck Forge folder yet.");
                if ui.button("Choose the Duck Forge folder...").clicked() {
                    chosen = Some(SendChoice::ChooseFolder);
                    ui.close();
                }
                return;
            }
            if games.is_empty() {
                ui.label("The Duck Forge folder has no games yet.");
            }
            for (game, mark) in games {
                let words = if mark.is_empty() {
                    game.clone()
                } else {
                    format!("{game} {mark}")
                };
                if ui
                    .button(words)
                    .on_hover_text(
                        "Into its assets folder with its facts file, and the recipe into its \
                         art folder. Sending again with no change writes nothing.",
                    )
                    .clicked()
                {
                    chosen = Some(SendChoice::Game(game));
                    ui.close();
                }
            }
        });
        chosen
    }

    /// Act on a Send to game choice for `rel`.
    pub fn act_on_send(&mut self, rel: &str, choice: SendChoice) {
        match choice {
            SendChoice::Game(game) => self.send_to_game(rel, &game),
            SendChoice::ChooseFolder => self.choose_duck_forge(),
        }
    }

    /// Send `rel` into `game` (the `game_send` command), say what
    /// happened, and remember the game for this file.
    pub fn send_to_game(&mut self, rel: &str, game: &str) {
        let Some(answer) = self.run_command("game_send", json!({ "file": rel, "game": game }))
        else {
            return;
        };
        let key = self.last_game_key(rel);
        self.settings.last_game.insert(key, game.to_string());
        self.settings.save();
        self.say(answer["summary"].as_str().unwrap_or("Sent.").to_string());
    }

    /// Choose the Duck Forge folder with the system's folder window.
    pub fn choose_duck_forge(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Where is Duck Forge?");
        if let Some(documents) = dirs::document_dir() {
            dialog = dialog.set_directory(documents);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.use_duck_forge_folder(&folder);
        }
    }

    /// Use `folder` as the Duck Forge folder, if it is one: save it in the
    /// settings and send there from now on. Says why not otherwise.
    pub fn use_duck_forge_folder(&mut self, folder: &Path) -> bool {
        match duck_echo_tools::forge::DuckForge::open(folder) {
            Ok(forge) => {
                let games = forge.games().len();
                self.say(format!(
                    "Sounds go to the games of {} ({games} game{}).",
                    forge.dir().display(),
                    if games == 1 { "" } else { "s" }
                ));
                self.settings.duck_forge_folder = Some(folder.to_path_buf());
                self.settings.save();
                if let Some(root) = &mut self.root {
                    root.set_duck_forge(Some(forge));
                }
                true
            }
            Err(why) => {
                self.warn(why);
                false
            }
        }
    }
}
