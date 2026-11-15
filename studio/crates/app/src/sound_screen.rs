//! The sound editor's screen: the bar along its top (back to Home, Play,
//! Loop, Stop, History, Export WAV), the text, the sliders, the picture and
//! the summary line. What the editor does is in [`crate::editor`]; this
//! file draws it and passes clicks on.

use std::path::{Path, PathBuf};
use std::time::Instant;

use duck_echo_live::Sound;
use eframe::egui::{self, RichText};

use crate::app::StudioApp;
use crate::editor::{self, Arrival, Editor, Heard};
use crate::theme;

/// A live voice of a render: it plays exactly the file's samples, and a
/// slider can change it while it plays.
fn voice_of(heard: &Heard) -> Result<Sound, String> {
    Sound::from_recipe_mastered(&heard.recipe, &heard.mastering).map_err(|r| r.to_string())
}

/// The key of the sound playing from the editor, in [`StudioApp::playing`].
fn key_of(rel: &str) -> String {
    format!("edit:{rel}")
}

impl StudioApp {
    /// Open one of the person's sounds in the editor.
    pub fn open_sound(&mut self, rel: &str) {
        let Some(root) = &self.root else {
            self.warn("Your sounds folder cannot be read.".into());
            return;
        };
        match Editor::open(root, rel) {
            Ok(editor) => {
                self.stop();
                self.editor = Some(editor);
                self.note = None;
            }
            Err(why) => self.warn(why),
        }
    }

    /// Back to Home: save what waits, stop the sound.
    pub fn close_editor(&mut self) {
        if let (Some(editor), Some(root)) = (&mut self.editor, &self.root) {
            let _ = editor.before_undo(root, Instant::now());
        }
        if self
            .playing
            .as_deref()
            .is_some_and(|k| k.starts_with("edit:"))
        {
            self.stop();
        }
        self.editor = None;
        self.rescan();
    }

    /// Play the open sound, once or over and over.
    pub fn play_open_sound(&mut self, looping: bool) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.looping = looping;
        match &editor.heard {
            Some(heard) if !editor.rendering() => {
                let key = key_of(&editor.rel);
                let result = voice_of(heard).and_then(|sound| self.speakers.play(sound, looping));
                match result {
                    Ok(()) => {
                        editor.live.started(heard);
                        self.playing = Some(key);
                        self.note = None;
                    }
                    Err(why) => self.warn(why),
                }
            }
            _ => editor.play_when_ready = Some(looping),
        }
    }

    /// Send the live changes of the newest edit to the playing voice (a
    /// volume, a cutoff or the master gain slides there at once).
    fn send_live_changes(&mut self) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        if self.playing.as_deref() != Some(key_of(&editor.rel).as_str()) {
            editor.live.stopped();
            return;
        }
        let Some(recipe) = editor.recipe().cloned() else {
            return;
        };
        for update in editor.live.changed(&recipe) {
            if let Err(why) = self.speakers.update(update) {
                self.warn(why);
                break;
            }
        }
    }

    /// Hand a new render to the speakers: it takes over from the sound
    /// playing (a slider was moved), or starts if Play was waiting for it.
    fn hear_changes(&mut self) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let key = key_of(&editor.rel);
        if editor.poll().is_none() {
            return;
        }
        let Some(heard) = editor.heard.as_ref().filter(|h| editor.is_newest(h)) else {
            return;
        };
        let playing_here = self.playing.as_deref() == Some(key.as_str());
        let result = if let Some(looping) = editor.play_when_ready.take() {
            let started = voice_of(heard).and_then(|sound| self.speakers.play(sound, looping));
            if started.is_ok() {
                editor.live.started(heard);
                self.playing = Some(key);
            }
            started
        } else if playing_here {
            match editor.live.rendered(heard) {
                Arrival::Replace => voice_of(heard).and_then(|sound| self.speakers.replace(sound)),
                Arrival::Level(level) => self.speakers.set_level(level),
                Arrival::Nothing => Ok(()),
            }
        } else {
            Ok(())
        };
        if let Err(why) = result {
            self.warn(why);
        }
    }

    /// Export the sound `rel` as a WAV file. With `ask`, the system's
    /// Save As window chooses where; without, it goes to `exports` in the
    /// project folder.
    pub fn export(&mut self, rel: &str, ask: bool) {
        let Some(root) = &self.root else {
            return;
        };
        let to: PathBuf = if ask {
            let name = Path::new(rel)
                .file_stem()
                .map_or("sound".into(), |s| s.to_string_lossy().into_owned());
            let mut dialog = rfd::FileDialog::new()
                .set_title("Export WAV")
                .set_file_name(format!("{name}.wav"))
                .add_filter("WAV sound", &["wav"]);
            if let Some(music) = editor::music_folder() {
                dialog = dialog.set_directory(music);
            }
            match dialog.save_file() {
                Some(path) => path,
                None => {
                    self.say(
                        "Nothing was exported. (If no Save As window opened, use Export WAV to \
                         my Duck Echo folder.)"
                            .into(),
                    );
                    return;
                }
            }
        } else {
            editor::export_place(root, rel)
        };
        match editor::export_wav(root, rel, &to) {
            Ok(path) => self.say(format!("Exported {}.", path.display())),
            Err(why) => self.warn(why),
        }
    }

    /// The editor, in the middle of the window.
    pub(crate) fn editor_screen(&mut self, ui: &mut egui::Ui) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let now = Instant::now();
        if let Some(root) = &self.root
            && let Some(problem) = editor.tick(root, now)
        {
            self.warn(problem);
        }
        self.editor = Some(editor);
        self.hear_changes();
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut back = false;
        let mut play = None;
        let mut stop = false;
        let mut export = None;
        let playing_here = self.playing.as_deref() == Some(key_of(&editor.rel).as_str());

        egui::Panel::top("sound bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("< Home").clicked() {
                    back = true;
                }
                ui.label(RichText::new(&editor.rel).strong().size(16.0));
                ui.add_space(10.0);
                if ui.add(theme::primary_button("Play")).clicked() {
                    play = Some(false);
                }
                if ui
                    .selectable_label(playing_here && editor.looping, "Loop")
                    .on_hover_text("Play it over and over: move a slider to hear it change.")
                    .clicked()
                {
                    play = Some(true);
                }
                if ui.button("Stop").clicked() {
                    stop = true;
                }
                if editor.rendering() || editor.unsaved() {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("...", |ui| {
                        if ui.button("Export WAV...").clicked() {
                            export = Some(true);
                            ui.close();
                        }
                        if ui.button("Export WAV to my Duck Echo folder").clicked() {
                            export = Some(false);
                            ui.close();
                        }
                    });
                    ui.toggle_value(&mut editor.history_open, "History");
                });
            });
            if editor.gone {
                ui.label(
                    RichText::new(format!(
                        "{} is gone: it was deleted or renamed. Press < Home.",
                        editor.rel
                    ))
                    .color(theme::LOOK),
                );
            }
        });

        egui::Panel::bottom("sound picture")
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal_top(|ui| {
                    match editor.picture(ui.ctx()) {
                        Some(texture) => {
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(egui::vec2(360.0, 177.0)),
                            );
                        }
                        None => {
                            ui.allocate_exact_size(egui::vec2(360.0, 177.0), egui::Sense::hover());
                        }
                    }
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("WHAT IT SOUNDS LIKE")
                                .small()
                                .strong()
                                .color(theme::NIGHT_INDIGO_TEXT),
                        );
                        if let Some(problem) = &editor.render_problem {
                            ui.label(RichText::new(problem).color(theme::LOOK));
                        } else if let Some(heard) = &editor.heard {
                            ui.label(&heard.summary);
                        } else {
                            ui.label(RichText::new("Listening...").weak());
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(
                                "Top: the level over time. Below: the pitches over time, low notes \
                                 at the bottom, loud parts bright.",
                            )
                            .small()
                            .weak(),
                        );
                    });
                });
            });

        if editor.history_open {
            egui::Panel::right("history")
                .default_size(250.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("HISTORY")
                            .small()
                            .strong()
                            .color(theme::NIGHT_INDIGO_TEXT),
                    );
                    ui.label(
                        RichText::new("Newest first. Undo takes back the top one.")
                            .small()
                            .weak(),
                    );
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if editor.history.is_empty() {
                            ui.label(RichText::new("No changes yet.").weak());
                        }
                        let now = duck_echo_tools::history::now_seconds();
                        for step in &editor.history {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(step.who).strong());
                                ui.label(RichText::new(ago(now, step.time)).small().weak());
                            });
                            ui.label(&step.title);
                            ui.add_space(4.0);
                        }
                    });
                });
        }

        let mut edits = Vec::new();
        egui::Panel::right("sliders")
            .default_size(400.0)
            .min_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| edits = editor::sliders(ui, &editor));
            });

        let mut suggestion = None;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label(
                RichText::new("TEXT")
                    .small()
                    .strong()
                    .color(theme::NIGHT_INDIGO_TEXT),
            );
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    let (picked, changed) =
                        editor::text_box(ui, &mut editor.text, &editor.problems.clone());
                    if changed {
                        editor.typed(now);
                    }
                    suggestion = picked;
                    // Problems without a line (rare) are listed below.
                    for p in editor.problems.iter().filter(|p| p.line.is_none()) {
                        let at = if p.path.is_empty() {
                            String::new()
                        } else {
                            format!("{}: ", p.path)
                        };
                        ui.label(
                            RichText::new(format!("{at}{}", p.message))
                                .small()
                                .color(theme::LOOK),
                        );
                    }
                });
        });

        if let Some(i) = suggestion {
            editor.use_suggestion(i, now);
        }
        let mut problems = Vec::new();
        if let Some(root) = &self.root {
            for edit in edits {
                if let Err(why) = editor.apply(root, edit, now) {
                    problems.push(why);
                }
            }
        }
        if let Some(why) = problems.pop() {
            self.warn(why);
        }
        if editor.rendering() || editor.unsaved() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let rel = editor.rel.clone();
        self.editor = Some(editor);
        // A volume, a cutoff or the gain is heard sliding at once.
        self.send_live_changes();
        if let Some(looping) = play {
            self.play_open_sound(looping);
        }
        if stop {
            self.stop();
        }
        if let Some(ask) = export {
            self.export(&rel, ask);
        }
        if back {
            self.close_editor();
        }
    }
}

/// "just now", "3 min ago", "2 h ago", "4 days ago".
fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_read_plainly() {
        assert_eq!(super::ago(100, 90), "just now");
        assert_eq!(super::ago(1000, 100), "15 min ago");
        assert_eq!(super::ago(10_000, 0), "2 h ago");
        assert_eq!(super::ago(200_000, 0), "2 days ago");
    }
}
