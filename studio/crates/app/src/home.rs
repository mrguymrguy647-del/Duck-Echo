//! Home: the person's sounds and songs as cards, and the gallery to start
//! from. Every change is a named command (`duck_echo_tools::commands`),
//! the same ones an AI assistant uses, so each is one step of the shared
//! Undo history.

use std::time::{Duration, Instant};

use duck_echo_live::Sound;
use duck_echo_tools::commands::{self, Command};
use duck_echo_tools::history::Who;
use duck_echo_tools::project::Project;
use duck_echo_tools::{instruments, presets};
use eframe::egui::{self, Color32, RichText, Sense};
use serde_json::{Value, json};

use crate::app::StudioApp;
use crate::library::{self, CATEGORIES, GalleryItem, Kind, MyFile, Shelf};
use crate::theme;

/// How often Home looks at the project folder again, to show files an AI
/// assistant (or another program) made or changed.
const RESCAN: Duration = Duration::from_secs(1);

/// The width of a card.
const CARD_WIDTH: f32 = 186.0;

impl StudioApp {
    /// Say something in the status line.
    pub fn say(&mut self, text: String) {
        self.note = Some(text);
        self.note_is_problem = false;
    }

    /// Say a problem in the status line (in the colour for problems).
    pub fn warn(&mut self, text: String) {
        self.note = Some(text);
        self.note_is_problem = true;
    }

    /// Read the project folder again: the files, and the Undo and Redo
    /// steps' titles.
    pub fn rescan(&mut self) {
        self.files = library::scan(&self.project);
        self.thumbs.keep_only(&self.files);
        self.last_scan = Some(Instant::now());
        let (undo, redo) = match &self.root {
            Some(root) => match Project::new(root).history_list(1) {
                Ok(list) => (
                    list.steps.first().map(|s| s.title.clone()),
                    list.undone.first().map(|s| s.title.clone()),
                ),
                Err(_) => (None, None),
            },
            None => (None, None),
        };
        self.undo_title = undo;
        self.redo_title = redo;
    }

    /// Run a command as the person, say what happened, and look at the
    /// folder again. Returns the answer, or `None` (and says why) when it
    /// was refused.
    pub fn run_command(&mut self, name: &str, args: Value) -> Option<Value> {
        let Some(root) = &self.root else {
            self.note = self.project_problem.clone();
            self.note_is_problem = true;
            return None;
        };
        let result = commands::parse(name, &args, "").and_then(|command: Command| {
            commands::run(&Project::new(root), command, Who::Person, None, "")
        });
        let answer = match result {
            Ok(answer) => Some(answer),
            Err(e) => {
                let report = e.into_report();
                self.warn(
                    report
                        .errors
                        .first()
                        .map_or("That did not work.".to_string(), |i| sentence(&i.message)),
                );
                None
            }
        };
        self.rescan();
        answer
    }

    /// New sound or New song: a fresh file, named "new sound" (or "new
    /// sound 2"...), to rename later.
    pub fn new_file(&mut self, kind: Kind) {
        let (command, stem) = match kind {
            Kind::Sound => ("file_new_sound", "new sound"),
            Kind::Song => ("file_new_song", "new song"),
        };
        let rel = library::free_name(&self.project, "", stem, "toml");
        if self.run_command(command, json!({ "file": rel })).is_some() {
            self.say(format!(
                "Made {}. Rename it with ... on its card.",
                stem_of(&rel)
            ));
        }
    }

    /// Make a copy: `source` (`preset:coin`) is copied into the project as
    /// a file of its own, named after it.
    pub fn make_copy(&mut self, source: &str, name: &str) {
        let rel = library::free_name(&self.project, "", name, "toml");
        if self
            .run_command("file_copy_from", json!({ "from": source, "to": rel }))
            .is_some()
        {
            self.say(format!(
                "Made a copy of {name}: {} is under My sounds and songs.",
                stem_of(&rel)
            ));
        }
    }

    /// Duplicate one of the person's files.
    pub fn duplicate(&mut self, rel: &str) {
        let stem = format!("{} copy", stem_of(rel));
        let to = library::free_name(
            &self.project,
            library::folder_of(rel),
            &stem,
            library::extension_of(rel),
        );
        if self
            .run_command("file_duplicate", json!({ "file": rel, "to": to }))
            .is_some()
        {
            self.say(format!("Made {}.", stem_of(&to)));
        }
    }

    /// Delete one of the person's files. It goes to the recycle bin, and
    /// Undo brings it back.
    pub fn delete(&mut self, rel: &str) {
        if self.playing.as_deref() == Some(&format!("my:{rel}")) {
            self.stop();
        }
        if self
            .run_command("file_delete", json!({ "file": rel }))
            .is_some()
        {
            self.say(format!(
                "Deleted {}. Undo (Ctrl+Z) brings it back.",
                stem_of(rel)
            ));
        }
    }

    /// Give one of the person's files a new name (its `name` field
    /// follows). Returns false, saying why, when the name cannot be used.
    pub fn rename_to(&mut self, rel: &str, new_name: &str) -> bool {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            self.warn("A name needs at least one letter.".into());
            return false;
        }
        if new_name.contains(['/', '\\', ':']) || new_name.starts_with('.') {
            self.warn(
                "A name cannot hold / \\ or :, or start with a dot. Letters, numbers, spaces, - \
                 and _ are fine."
                    .into(),
            );
            return false;
        }
        let folder = library::folder_of(rel);
        let file = format!("{new_name}.{}", library::extension_of(rel));
        let to = if folder.is_empty() {
            file
        } else {
            format!("{folder}/{file}")
        };
        if to == rel {
            return true;
        }
        if self
            .run_command("file_rename", json!({ "file": rel, "to": to }))
            .is_some()
        {
            self.say(format!("Renamed {} to {new_name}.", stem_of(rel)));
            true
        } else {
            false
        }
    }

    /// Undo the newest step, whoever made it.
    pub fn undo(&mut self) {
        let title = self.undo_title.clone();
        if self.run_command("edit_undo", json!({})).is_some() {
            self.say(match title {
                Some(t) => format!("Undid: {t}."),
                None => "Undone.".into(),
            });
        }
    }

    /// Redo the step undone last.
    pub fn redo(&mut self) {
        let title = self.redo_title.clone();
        if self.run_command("edit_redo", json!({})).is_some() {
            self.say(match title {
                Some(t) => format!("Redid: {t}."),
                None => "Redone.".into(),
            });
        }
    }

    /// Play a gallery item (`preset:coin`, `instrument:piano`,
    /// `example:four_bars`) or one of the person's files (`my:coin.toml`).
    pub fn play(&mut self, key: &str) {
        let result = self.start_playing(key);
        match result {
            Ok(()) => {
                self.playing = Some(key.to_string());
                self.note = None;
            }
            Err(why) => {
                self.playing = None;
                self.warn(why);
            }
        }
    }

    fn start_playing(&mut self, key: &str) -> Result<(), String> {
        let report = |e: duck_echo_tools::ToolError| sentence(&e.into_report().to_string());
        if let Some(name) = key.strip_prefix("preset:") {
            let recipe = presets::get(name)
                .and_then(presets::load)
                .map_err(report)?
                .recipe;
            return self.play_recipe(key, &recipe);
        }
        if let Some(name) = key.strip_prefix("instrument:") {
            let recipe = instruments::get(name)
                .and_then(instruments::load)
                .map_err(report)?
                .recipe;
            return self.play_recipe(key, &recipe);
        }
        let root = self
            .root
            .as_ref()
            .ok_or("The project folder cannot be read.")?;
        if let Some(name) = key.strip_prefix("example:") {
            let example = duck_echo_tools::examples::get(name).map_err(report)?;
            let (song, arrangement) = library::load_song(root, "", example.source)?;
            return self.speakers.play_song(song, arrangement);
        }
        let rel = key.strip_prefix("my:").ok_or("Nothing to play.")?;
        let file = self
            .files
            .iter()
            .find(|f| f.rel == rel)
            .cloned()
            .ok_or("That file is gone.")?;
        match file.kind {
            Kind::Song => {
                let (song, arrangement) =
                    library::load_song(root, library::folder_of(rel), &file.text)?;
                self.speakers.play_song(song, arrangement)
            }
            Kind::Sound => {
                let format = duck_echo_core::Format::from_path(std::path::Path::new(rel));
                let recipe = duck_echo_core::load_str(&file.text, format)
                    .map_err(|r| sentence(&r.to_string()))?
                    .recipe;
                let sound = Sound::from_recipe(&recipe).map_err(|r| sentence(&r.to_string()))?;
                self.speakers.play(sound, false)
            }
        }
    }

    fn play_recipe(&mut self, key: &str, recipe: &duck_echo_core::Recipe) -> Result<(), String> {
        let mastering = match self.masterings.get(key) {
            Some(m) => *m,
            None => {
                let (_, m) = duck_echo_synth::render_mastered(recipe)
                    .map_err(|report| sentence(&report.to_string()))?;
                self.masterings.insert(key.to_string(), m);
                m
            }
        };
        let sound = Sound::from_recipe_mastered(recipe, &mastering)
            .map_err(|r| sentence(&r.to_string()))?;
        self.speakers.play(sound, false)
    }

    /// Ctrl+Z and Ctrl+Y (or Ctrl+Shift+Z), unless a text box is being
    /// typed in (it has its own Undo).
    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let undo = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
        let redo = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
        let redo_too = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Z,
        );
        // The longer shortcut first, so Ctrl+Shift+Z is not taken as Ctrl+Z.
        if ctx.input_mut(|i| i.consume_shortcut(&redo_too) || i.consume_shortcut(&redo)) {
            self.redo();
        } else if ctx.input_mut(|i| i.consume_shortcut(&undo)) {
            self.undo();
        }
    }

    /// Undo and Redo, for the top bar.
    pub(crate) fn undo_buttons(&mut self, ui: &mut egui::Ui) {
        let undo = ui
            .add_enabled(self.undo_title.is_some(), egui::Button::new("Undo"))
            .on_hover_text(match &self.undo_title {
                Some(t) => format!("Undo: {t} (Ctrl+Z)"),
                None => "Nothing to undo".into(),
            })
            .on_disabled_hover_text("Nothing to undo");
        let redo = ui
            .add_enabled(self.redo_title.is_some(), egui::Button::new("Redo"))
            .on_hover_text(match &self.redo_title {
                Some(t) => format!("Redo: {t} (Ctrl+Y)"),
                None => "Nothing to redo".into(),
            })
            .on_disabled_hover_text("Nothing to redo");
        if undo.clicked() {
            self.undo();
        }
        if redo.clicked() {
            self.redo();
        }
    }

    /// Home, in the middle of the window.
    pub(crate) fn home(&mut self, ui: &mut egui::Ui) {
        if self.last_scan.is_none_or(|t| t.elapsed() >= RESCAN) {
            self.rescan();
        }
        self.thumbs.poll();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("New sound")).clicked() {
                        self.new_file(Kind::Sound);
                    }
                    if ui.add(theme::primary_button("New song")).clicked() {
                        self.new_file(Kind::Song);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("name, sound or word")
                                .desired_width(200.0),
                        );
                        let label = ui.label("Search:");
                        edit.labelled_by(label.id);
                    });
                });
                ui.add_space(8.0);
                self.my_files(ui);
                ui.add_space(14.0);
                self.gallery(ui);
            });
        if self.rename.is_some() {
            self.rename_dialog(ui.ctx());
        }
    }

    fn my_files(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("My sounds and songs")
                .strong()
                .size(17.0)
                .color(theme::NIGHT_INDIGO_TEXT),
        );
        let shown: Vec<MyFile> = self
            .files
            .iter()
            .filter(|f| library::matches(&self.search, &[&f.name, &f.rel]))
            .cloned()
            .collect();
        if self.files.is_empty() {
            ui.label(
                RichText::new(
                    "Nothing here yet. Press Make a copy on a sound below, or New sound. Your \
                     sounds are saved in your Duck Echo folder.",
                )
                .weak(),
            );
            return;
        }
        if shown.is_empty() {
            ui.label(RichText::new("None of your sounds or songs match the search.").weak());
            return;
        }
        for row in shown.chunks(cards_per_row(ui)) {
            ui.horizontal_top(|ui| {
                for file in row {
                    self.my_card(ui, file);
                }
            });
        }
    }

    fn my_card(&mut self, ui: &mut egui::Ui, file: &MyFile) {
        let key = format!("my:{}", file.rel);
        let playing = self.playing.as_deref() == Some(key.as_str());
        let project = self.project.clone();
        let thumb = self.thumbs.get(&project, file).cloned();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.push_id(&file.rel, |ui| {
                ui.vertical(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, 38.0), Sense::hover());
                    draw_levels(ui, rect, thumb.as_ref().map(|t| t.levels.as_slice()));
                    ui.label(RichText::new(&file.name).strong());
                    match &thumb {
                        Some(t) if t.problem.is_some() => {
                            ui.label(
                                RichText::new(t.problem.clone().unwrap_or_default())
                                    .small()
                                    .color(theme::LOOK),
                            );
                        }
                        Some(t) => {
                            let what = match file.kind {
                                Kind::Sound => "sound",
                                Kind::Song => "song",
                            };
                            ui.label(RichText::new(format!("{what}, {}", t.length)).weak());
                        }
                        None => {
                            ui.label(RichText::new("measuring...").weak());
                        }
                    }
                    ui.horizontal(|ui| {
                        if playing {
                            if ui.button("Stop").clicked() {
                                self.stop();
                            }
                        } else if ui.button("Play").clicked() {
                            self.play(&key);
                        }
                        ui.menu_button("...", |ui| {
                            if ui.button("Rename...").clicked() {
                                self.rename = Some((file.rel.clone(), file.name.clone()));
                                ui.close();
                            }
                            if ui.button("Duplicate").clicked() {
                                self.duplicate(&file.rel);
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                self.delete(&file.rel);
                                ui.close();
                            }
                        });
                    });
                });
            });
        });
    }

    fn gallery(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Start from")
                .strong()
                .size(17.0)
                .color(theme::NIGHT_INDIGO_TEXT),
        );
        ui.horizontal(|ui| {
            for (shelf, title) in [
                (Shelf::Presets, "Presets"),
                (Shelf::Instruments, "Instruments"),
                (Shelf::Songs, "Example songs"),
            ] {
                if ui.selectable_label(self.shelf == shelf, title).clicked() {
                    self.shelf = shelf;
                }
            }
        });
        let searching = !self.search.trim().is_empty();
        if self.shelf == Shelf::Presets && !searching {
            ui.horizontal_wrapped(|ui| {
                for (category, title) in CATEGORIES {
                    if ui
                        .selectable_label(self.category == *category, *title)
                        .clicked()
                    {
                        self.category = category;
                    }
                }
            });
        }
        ui.add_space(4.0);
        let items: Vec<GalleryItem> = library::gallery(self.shelf)
            .into_iter()
            .filter(|item| {
                if searching {
                    let tags = item.tags.join(" ");
                    library::matches(
                        &self.search,
                        &[&item.name, &item.description, &item.category, &tags],
                    )
                } else {
                    self.shelf != Shelf::Presets || item.category == self.category
                }
            })
            .collect();
        if items.is_empty() {
            ui.label(RichText::new("Nothing here matches the search.").weak());
            return;
        }
        for row in items.chunks(cards_per_row(ui)) {
            ui.horizontal_top(|ui| {
                for item in row {
                    self.gallery_card(ui, item);
                }
            });
        }
    }

    fn gallery_card(&mut self, ui: &mut egui::Ui, item: &GalleryItem) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.push_id(&item.name, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&item.name).strong());
                    ui.add(
                        egui::Label::new(RichText::new(&item.description).small().weak()).wrap(),
                    );
                    let Some(source) = &item.source else {
                        ui.label(
                            RichText::new("Play its pieces: kick, snare, hat and clap.")
                                .small()
                                .weak(),
                        );
                        return;
                    };
                    let playing = self.playing.as_deref() == Some(source.as_str());
                    ui.horizontal(|ui| {
                        if playing {
                            if ui.button("Stop").clicked() {
                                self.stop();
                            }
                        } else if ui.button("Play").clicked() {
                            self.play(source);
                        }
                        if ui.button("Make a copy").clicked() {
                            self.make_copy(source, &item.name);
                        }
                    });
                });
            });
        });
    }

    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some((rel, mut name)) = self.rename.take() else {
            return;
        };
        let mut answer: Option<bool> = None;
        egui::Modal::new(egui::Id::unique("rename")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(format!("Rename {}", stem_of(&rel)));
            ui.add_space(6.0);
            let label = ui.label("New name:");
            let edit = ui
                .add(egui::TextEdit::singleline(&mut name).desired_width(320.0))
                .labelled_by(label.id);
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(true);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Rename")).clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                if !self.rename_to(&rel, &name) {
                    // Keep the window open, with the note saying why.
                    self.rename = Some((rel, name));
                }
            }
            Some(false) => {}
            None => self.rename = Some((rel, name)),
        }
    }
}

/// How many cards fit side by side. (egui's wrapping rows do not wrap a
/// framed card, so the rows are made here.)
fn cards_per_row(ui: &egui::Ui) -> usize {
    let spacing = ui.spacing().item_spacing.x;
    // A card is its width, the frame's margin and stroke on both sides, and
    // the space between cards.
    let card = CARD_WIDTH + 2.0 * 8.0 + spacing;
    ((ui.available_width() + spacing) / card).floor().max(1.0) as usize
}

/// The file name of `rel`, without its folder or extension.
fn stem_of(rel: &str) -> String {
    let file = rel.rsplit_once('/').map_or(rel, |(_, f)| f);
    file.rsplit_once('.').map_or(file, |(s, _)| s).to_string()
}

/// A report's message as a sentence for the status line: its first line,
/// without "error[code]".
fn sentence(text: &str) -> String {
    let line = text.lines().next().unwrap_or("That did not work.");
    let line = line.split_once("] ").map_or(line, |(_, rest)| rest);
    let mut out: String = line.chars().take(200).collect();
    if let Some(first) = out.get(..1) {
        out = first.to_uppercase() + &out[1..];
    }
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// The card's picture: the level over time as bars, or a faint empty
/// strip while it is being measured.
fn draw_levels(ui: &egui::Ui, rect: egui::Rect, levels: Option<&[f32]>) {
    let painter = ui.painter_at(rect);
    let back = ui.visuals().extreme_bg_color;
    painter.rect_filled(rect, 3.0, back);
    let Some(levels) = levels.filter(|l| !l.is_empty()) else {
        return;
    };
    let gap = 1.0;
    let width = (rect.width() - gap * (levels.len() as f32 - 1.0)) / levels.len() as f32;
    let color: Color32 = theme::ECHO_CYAN;
    for (i, level) in levels.iter().enumerate() {
        let height = (rect.height() - 4.0) * level.clamp(0.0, 1.0);
        let x = rect.left() + i as f32 * (width + gap);
        let bar = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - 2.0 - height),
            egui::pos2(x + width, rect.bottom() - 2.0),
        );
        painter.rect_filled(bar, 1.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sentences() {
        assert_eq!(stem_of("sfx/my coin.toml"), "my coin");
        assert_eq!(stem_of("coin.json"), "coin");
        assert_eq!(
            sentence(
                "error[already_exists] to: `coin.toml` already exists; choose another name\nmore"
            ),
            "To: `coin.toml` already exists; choose another name."
        );
    }
}
