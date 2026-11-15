//! The window: a bar along the top (Undo, Redo, Settings, Theme, Help), the
//! new-version banner, Home (see [`crate::home`]) and a status line.
//!
//! Everything a sound does comes from the Duck Echo libraries; this file
//! only draws and turns clicks into calls.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use duck_echo_synth::Mastering;
use duck_echo_tools::root::Root;
use eframe::egui::{self, RichText};

use crate::app_menu;
use crate::library::{MyFile, Shelf, Thumbs};
use crate::settings::{self, Settings};
use crate::speaker::Speakers;
use crate::theme;
use crate::update_banner::{self, BannerChoice, UpdateState};

/// The Linux app-menu entry, as this Studio sees it.
#[derive(Debug, Clone, Default)]
pub struct AppMenu {
    /// The AppImage this Studio runs from, if it does.
    pub appimage: Option<PathBuf>,
    /// The person's data folder (`~/.local/share`).
    pub data: Option<PathBuf>,
    /// Ask "Add Duck Echo Studio to your app menu?" now.
    pub ask: bool,
}

impl AppMenu {
    /// No app menu: Windows, a test, or a Studio not run from an AppImage.
    pub fn none() -> AppMenu {
        AppMenu::default()
    }

    /// Look at this system: on Linux, from an AppImage, follow a moved
    /// AppImage, and ask once if the Studio is not in the app menu.
    pub fn detect(settings: &Settings) -> AppMenu {
        let (Some(appimage), Some(data)) = (app_menu::running_appimage(), app_menu::data_folder())
        else {
            return AppMenu::none();
        };
        app_menu::follow_if_moved(&data, &appimage);
        let ask = !settings.asked_about_app_menu && !app_menu::is_added(&data);
        AppMenu {
            appimage: Some(appimage),
            data: Some(data),
            ask,
        }
    }

    /// True when this Studio can add itself to the app menu.
    pub fn can_add(&self) -> bool {
        self.appimage.is_some() && self.data.is_some()
    }

    /// True when the entry is there.
    pub fn is_added(&self) -> bool {
        self.data.as_deref().is_some_and(app_menu::is_added)
    }

    fn add(&self) -> Result<(), String> {
        match (&self.data, &self.appimage) {
            (Some(data), Some(appimage)) => app_menu::add(data, appimage),
            _ => Err("only the AppImage can add itself to the app menu".into()),
        }
    }

    fn remove(&self) -> Result<(), String> {
        match &self.data {
            Some(data) => app_menu::remove(data),
            None => Ok(()),
        }
    }
}

/// The app.
pub struct StudioApp {
    /// The settings, saved whenever they change.
    pub settings: Settings,
    /// Where the person's sounds and songs go.
    pub project: PathBuf,
    /// Why the project folder could not be made, if it could not.
    pub project_problem: Option<String>,
    /// The speakers, opened when something first plays.
    pub speakers: Speakers,
    /// What plays now, if anything: `preset:coin`, `instrument:piano`,
    /// `example:four_bars`, or one of the person's files, `my:coin.toml`.
    pub playing: Option<String>,
    /// The project folder's path rules (`None` if it cannot be read).
    pub root: Option<Root>,
    /// The person's sounds and songs, as last read.
    pub files: Vec<MyFile>,
    /// The cards' pictures.
    pub thumbs: Thumbs,
    /// The part of the gallery shown.
    pub shelf: Shelf,
    /// The category of presets shown.
    pub category: &'static str,
    /// The search box's words.
    pub search: String,
    /// The Rename window, when it is open: the file and the name typed.
    pub rename: Option<(String, String)>,
    /// What Undo and Redo would do now (the steps' titles).
    pub undo_title: Option<String>,
    /// What Redo would do now.
    pub redo_title: Option<String>,
    /// When the project folder was last read. (`None` reads it again at
    /// the next frame.)
    pub last_scan: Option<Instant>,
    /// The new-version check and its banner.
    pub update: UpdateState,
    /// A sentence for the status line (an answer, a problem).
    pub note: Option<String>,
    /// True when the note is a problem (shown in the colour for problems).
    pub note_is_problem: bool,
    /// The About box is open.
    pub about_open: bool,
    /// The Settings window is open.
    pub settings_open: bool,
    /// The Linux app-menu entry.
    pub app_menu: AppMenu,
    /// The level and length each built-in sound's rendered file has,
    /// worked out the first time it plays (see `duck_echo_live::Sound`).
    pub(crate) masterings: BTreeMap<String, Mastering>,
    /// For CI: write a line to this file once a few frames are drawn, then
    /// close (`DUCK_ECHO_STUDIO_SMOKE_TEST`).
    smoke_test: Option<PathBuf>,
    frames: u64,
    /// The duck icon, made into a texture the first time it is drawn.
    icon: Option<egui::TextureHandle>,
}

impl StudioApp {
    /// The app with these settings and app-menu state. Makes the project
    /// folder if it is not there.
    pub fn new(settings: Settings, app_menu: AppMenu) -> StudioApp {
        let project = settings.project_folder();
        let mut project_problem = settings::make_project_folder(&project).err();
        let root = match Root::new(&project) {
            Ok(root) => Some(root),
            Err(why) => {
                project_problem.get_or_insert(format!(
                    "Your sounds folder, {}, cannot be read: {why}",
                    project.display()
                ));
                None
            }
        };
        let mut app = StudioApp {
            settings,
            project,
            project_problem,
            speakers: Speakers::default(),
            playing: None,
            root,
            files: Vec::new(),
            thumbs: Thumbs::new(),
            shelf: Shelf::Presets,
            category: "pickup",
            search: String::new(),
            rename: None,
            undo_title: None,
            redo_title: None,
            last_scan: None,
            update: UpdateState::Off,
            note: None,
            note_is_problem: false,
            about_open: false,
            settings_open: false,
            app_menu,
            masterings: BTreeMap::new(),
            smoke_test: std::env::var_os("DUCK_ECHO_STUDIO_SMOKE_TEST").map(PathBuf::from),
            frames: 0,
            icon: None,
        };
        app.rescan();
        app
    }

    /// Start the new-version check, if the settings allow it.
    pub fn checking_for_updates(mut self) -> StudioApp {
        if self.settings.check_for_updates {
            self.update = UpdateState::check(false);
        }
        self
    }

    /// Stop what plays.
    pub fn stop(&mut self) {
        self.speakers.stop();
        self.playing = None;
    }

    /// Draw one frame.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.speakers.collect();
        if self.playing.is_some() && !self.speakers.is_playing() && self.frames > 2 {
            // It played to its end. (The mixer starts a sound at its next
            // block, so the first frames may not show it yet.)
            self.playing = None;
        }
        let (said, close) = self.update.poll();
        if let Some(said) = said {
            self.say(said);
        }
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        self.top_bar(ui);
        if self.update.shows() {
            let chosen = egui::Panel::top("new version")
                .show(ui, |ui| update_banner::banner(ui, &self.update))
                .inner;
            match chosen {
                Some(BannerChoice::WhatsNew(page)) => ctx.open_url(egui::OpenUrl::new_tab(page)),
                Some(BannerChoice::Update) => self.update.update(&ctx),
                Some(BannerChoice::NotNow) => self.update = UpdateState::Dismissed,
                None => {}
            }
        }
        self.status_line(ui);
        self.shortcuts(&ctx);
        egui::CentralPanel::default().show(ui, |ui| self.home(ui));
        self.dialogs(&ctx);

        self.frames += 1;
        self.smoke_test(&ctx);
        if self.playing.is_some()
            || self.thumbs.busy()
            || matches!(
                self.update,
                UpdateState::Checking { .. } | UpdateState::Downloading(..)
            )
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// The duck icon as a texture, made the first time it is needed.
    fn icon_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        self.icon
            .get_or_insert_with(|| {
                let icon = eframe::icon_data::from_png_bytes(include_bytes!(
                    "../../../brand/duck-echo-icon-64.png"
                ))
                .expect("brand/duck-echo-icon-64.png should be a valid PNG file");
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [icon.width as usize, icon.height as usize],
                    &icon.rgba,
                );
                ctx.load_texture("duck echo icon", image, egui::TextureOptions::LINEAR)
            })
            .clone()
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let icon = self.icon_texture(ui.ctx());
        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::vec2(28.0, 28.0)));
                ui.heading(RichText::new("Duck Echo Studio").strong());
                ui.add_space(12.0);
                self.undo_buttons(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.help_menu(ui);
                    if let Some(chosen) = theme::theme_menu(ui) {
                        self.settings.theme = Some(theme::name(chosen).to_string());
                        self.settings.save();
                    }
                    if ui.button("Settings").clicked() {
                        self.settings_open = true;
                    }
                });
            });
        });
    }

    fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui.button("About Duck Echo Studio").clicked() {
                self.about_open = true;
                ui.close();
            }
            if ui.button("Check for new versions").clicked() {
                self.update = UpdateState::check(true);
                self.say("Asking GitHub for new versions...".into());
                ui.close();
            }
            ui.hyperlink_to(
                "Duck Echo on GitHub",
                "https://github.com/mrguymrguy647-del/Duck-Echo",
            );
        });
    }

    fn status_line(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if let Some(key) = self.playing.clone() {
                    let name = key.split_once(':').map_or(key.as_str(), |(_, n)| n);
                    if self.speakers.song_getting_ready() {
                        ui.spinner();
                        ui.label(
                            RichText::new(format!("Getting {name} ready..."))
                                .color(theme::ECHO_CYAN),
                        );
                    } else {
                        ui.label(RichText::new(format!("Playing {name}")).color(theme::ECHO_CYAN));
                    }
                    if ui.button("Stop").clicked() {
                        self.stop();
                    }
                } else if let Some(note) = &self.note {
                    let text = RichText::new(note);
                    ui.label(if self.note_is_problem {
                        text.color(theme::LOOK)
                    } else {
                        text.strong()
                    });
                } else if let Some(problem) = &self.project_problem {
                    ui.label(RichText::new(problem).color(theme::LOOK));
                } else {
                    ui.label(
                        RichText::new(format!("Your sounds and songs: {}", self.project.display()))
                            .weak(),
                    );
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.about_open {
            let icon = self.icon_texture(ctx);
            let mut close = false;
            egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::vec2(48.0, 48.0)));
                    ui.heading("Duck Echo Studio");
                });
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label(
                    "Make sound effects and music from short text recipes, without a terminal. \
                     The same sounds an AI assistant makes with Duck Echo's tools.",
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Your sounds and songs: {}", self.project.display()))
                        .weak(),
                );
                ui.hyperlink_to(
                    "Duck Echo on GitHub",
                    "https://github.com/mrguymrguy647-del/Duck-Echo",
                );
                ui.add_space(8.0);
                if ui.add(theme::primary_button("Close")).clicked() {
                    close = true;
                }
            });
            if close {
                self.about_open = false;
            }
        }
        if self.settings_open {
            self.settings_window(ctx);
        }
        if self.app_menu.ask {
            self.app_menu_question(ctx);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut close = false;
        egui::Modal::new(egui::Id::unique("settings")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Settings");
            ui.add_space(6.0);

            ui.label(RichText::new("Your sounds and songs").strong());
            ui.label(self.project.display().to_string());
            if ui.button("Open the folder").clicked() {
                settings::open_folder(&self.project);
            }
            ui.add_space(8.0);

            ui.label(RichText::new("New versions").strong());
            let mut check = self.settings.check_for_updates;
            if ui
                .checkbox(&mut check, "Tell me when a new version is out")
                .on_hover_text(
                    "When the Studio starts, it looks at Duck Echo's releases page on GitHub. \
                     Nothing about you is sent, and nothing is downloaded unless you press \
                     Update.",
                )
                .changed()
            {
                self.settings.check_for_updates = check;
                self.settings.save();
                self.update = if check {
                    UpdateState::check(false)
                } else {
                    UpdateState::Off
                };
            }

            if cfg!(target_os = "linux") {
                ui.add_space(8.0);
                ui.label(RichText::new("App menu").strong());
                if !self.app_menu.can_add() {
                    ui.label(
                        RichText::new(
                            "Only the AppImage of Duck Echo Studio can add itself to the app menu.",
                        )
                        .weak(),
                    );
                } else if self.app_menu.is_added() {
                    ui.label("Duck Echo Studio is in your app menu.");
                    if ui.button("Remove from app menu").clicked() {
                        match self.app_menu.remove() {
                            Ok(()) => {
                                self.say("Duck Echo Studio was removed from your app menu.".into())
                            }
                            Err(why) => {
                                self.warn(format!("Could not remove it from the app menu: {why}."))
                            }
                        }
                    }
                } else {
                    ui.label("Duck Echo Studio is not in your app menu.");
                    if ui.button("Add to app menu").clicked() {
                        match self.app_menu.add() {
                            Ok(()) => self.say("Duck Echo Studio is in your app menu now.".into()),
                            Err(why) => {
                                self.warn(format!("Could not add it to the app menu: {why}."))
                            }
                        }
                    }
                }
            }
            ui.add_space(10.0);
            if ui.add(theme::primary_button("Close")).clicked() {
                close = true;
            }
        });
        if close {
            self.settings_open = false;
        }
    }

    fn app_menu_question(&mut self, ctx: &egui::Context) {
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("app menu")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Add Duck Echo Studio to your app menu?");
            ui.add_space(6.0);
            ui.label(
                "Then you can start it from your desktop's app menu, like any other app. This \
                 adds a menu entry and the duck icon in your own folders, nothing for the whole \
                 computer. You can remove it again in Settings.",
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Add")).clicked() {
                    answer = Some(true);
                }
                if ui.button("Not now").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(add) = answer {
            self.app_menu.ask = false;
            self.settings.asked_about_app_menu = true;
            self.settings.save();
            if add {
                match self.app_menu.add() {
                    Ok(()) => self.say("Duck Echo Studio is in your app menu now.".into()),
                    Err(why) => self.warn(format!("Could not add it to the app menu: {why}.")),
                }
            }
        }
    }

    /// For CI (`DUCK_ECHO_STUDIO_SMOKE_TEST=<file>`): once the window has
    /// drawn a few frames, say so in the file and close. Proves the
    /// installed program really opens its window and draws.
    fn smoke_test(&mut self, ctx: &egui::Context) {
        let Some(file) = &self.smoke_test else {
            return;
        };
        ctx.request_repaint();
        if self.frames == 30 {
            let appimage = self
                .app_menu
                .appimage
                .as_ref()
                .map_or("none".to_string(), |a| a.display().to_string());
            let line = format!(
                "Duck Echo Studio {} drew {} frames; project folder {}; AppImage {appimage}; \
                 asks about the app menu: {}\n",
                env!("CARGO_PKG_VERSION"),
                self.frames,
                self.project.display(),
                self.app_menu.ask
            );
            let _ = std::fs::write(file, line);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// The settings to keep when the window closes.
    pub fn on_close(&mut self) {
        self.stop();
        self.settings.save();
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        StudioApp::ui(self, ui);
    }

    fn on_exit(&mut self) {
        self.on_close();
    }
}
