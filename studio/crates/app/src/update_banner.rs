//! The "a new version is out" banner, at the top of the window.
//!
//! It shows only when the check found a newer Studio (see [`crate::update`]).
//! "What's new" opens the release's page. "Update" downloads this system's
//! file and checks its fingerprint; then on Windows it starts the installer
//! and the Studio closes, and on Linux the AppImage is replaced and starts
//! again. Nothing is downloaded until Update is pressed. "Not now" hides it
//! until the Studio starts again.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use crate::app_menu;
use crate::theme;
use crate::update::{self, Curl, Download, Found, NewVersion, UpdateCheck};

/// Where the new-version check is.
#[derive(Default)]
pub enum UpdateState {
    /// Not checking (turned off in Settings, or a test).
    #[default]
    Off,
    /// Asking GitHub. `asked` is true when the person asked (Help menu),
    /// so the answer is said even when there is nothing new.
    Checking {
        /// The check in the background.
        check: UpdateCheck,
        /// The person asked for it.
        asked: bool,
    },
    /// Up to date, or no internet: nothing to show.
    Nothing,
    /// A newer version is out.
    Available(NewVersion),
    /// Downloading and checking it.
    Downloading(NewVersion, Download),
    /// The new version is being put in: the Studio closes.
    Installing,
    /// The download (after Update was pressed) did not work, and why.
    Failed(NewVersion, String),
    /// "Not now".
    Dismissed,
}

/// What a person did with the banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerChoice {
    /// Open the release's page.
    WhatsNew(String),
    /// Download and install it.
    Update,
    /// Hide the banner.
    NotNow,
}

impl UpdateState {
    /// Start checking, with the real internet.
    pub fn check(asked: bool) -> UpdateState {
        UpdateState::Checking {
            check: UpdateCheck::start(Curl, env!("CARGO_PKG_VERSION")),
            asked,
        }
    }

    /// Take in the check's or the download's answer, if it came. Returns a
    /// sentence to show when there is one, and whether the Studio must now
    /// close (the new version is going in).
    pub fn poll(&mut self) -> (Option<String>, bool) {
        let running = env!("CARGO_PKG_VERSION");
        match self {
            UpdateState::Checking { check, asked } => {
                let asked = *asked;
                let Some(found) = check.poll() else {
                    return (None, false);
                };
                let (next, said) = match found {
                    Found::Newer(new) => (UpdateState::Available(new), None),
                    Found::UpToDate => (
                        UpdateState::Nothing,
                        asked.then(|| format!("You have the newest version, {running}.")),
                    ),
                    Found::NoAnswer(why) => (
                        UpdateState::Nothing,
                        asked.then(|| format!("Could not ask GitHub for new versions ({why}).")),
                    ),
                };
                *self = next;
                (said, false)
            }
            UpdateState::Downloading(new, download) => {
                let Some(done) = download.poll() else {
                    return (None, false);
                };
                let new = new.clone();
                match done.and_then(put_in_place) {
                    Ok(()) => {
                        *self = UpdateState::Installing;
                        (None, true)
                    }
                    Err(why) => {
                        *self = UpdateState::Failed(new, why);
                        (None, false)
                    }
                }
            }
            _ => (None, false),
        }
    }

    /// Update was pressed: download and check the new version, or, if
    /// this copy cannot update itself (no file for this system, or a Linux
    /// Studio that is not running from an AppImage), open the release's
    /// page.
    pub fn update(&mut self, ctx: &egui::Context) {
        let new = match self {
            UpdateState::Available(new) | UpdateState::Failed(new, _) => new.clone(),
            _ => return,
        };
        let folder = if cfg!(windows) {
            Some(update::installer_folder())
        } else {
            app_menu::running_appimage().and_then(|a| a.parent().map(PathBuf::from))
        };
        match folder {
            Some(folder) if new.file.is_some() => {
                let download = Download::start(Curl, &new, &folder);
                *self = UpdateState::Downloading(new, download);
            }
            _ => ctx.open_url(egui::OpenUrl::new_tab(&new.whats_new)),
        }
    }

    /// Whether the banner shows.
    pub fn shows(&self) -> bool {
        matches!(
            self,
            UpdateState::Available(_)
                | UpdateState::Downloading(..)
                | UpdateState::Installing
                | UpdateState::Failed(..)
        )
    }
}

/// Put a checked download in place: start the installer (Windows), or
/// replace the running AppImage and start it again (Linux).
fn put_in_place(file: PathBuf) -> Result<(), String> {
    if cfg!(windows) {
        return update::start_installer(&file);
    }
    let running = app_menu::running_appimage()
        .ok_or("this Studio is not running from an AppImage, so it cannot replace itself")?;
    update::replace_appimage(&file, &running)?;
    update::restart(&running)
}

/// The banner, if there is something to show.
pub fn banner(ui: &mut egui::Ui, state: &UpdateState) -> Option<BannerChoice> {
    let mut choice = None;
    let new = match state {
        UpdateState::Available(new)
        | UpdateState::Downloading(new, _)
        | UpdateState::Failed(new, _) => new,
        UpdateState::Installing => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Putting the new version in. The Studio closes and opens again...");
            });
            return None;
        }
        _ => return None,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("Duck Echo Studio {} is out.", new.version))
                .strong()
                .color(theme::DUCK_YELLOW),
        );
        ui.label(RichText::new(format!("You have {}.", env!("CARGO_PKG_VERSION"))).weak());
        if ui.link("What's new").clicked() {
            choice = Some(BannerChoice::WhatsNew(new.whats_new.clone()));
        }
        match state {
            UpdateState::Downloading(..) => {
                ui.spinner();
                ui.label("Downloading and checking the new version...");
            }
            _ => {
                let label = if matches!(state, UpdateState::Failed(..)) {
                    "Try again"
                } else {
                    "Update"
                };
                if ui
                    .add(theme::primary_button(label))
                    .on_hover_text(
                        "Downloads the new version, checks its fingerprint, and puts it in. \
                         Your sounds, songs and settings stay as they are.",
                    )
                    .clicked()
                {
                    choice = Some(BannerChoice::Update);
                }
                if ui.button("Not now").clicked() {
                    choice = Some(BannerChoice::NotNow);
                }
            }
        }
    });
    if let UpdateState::Failed(_, why) = state {
        ui.label(
            RichText::new(format!(
                "The update did not work: {why}. Try again, or download it from the What's new \
                 page."
            ))
            .small()
            .color(theme::LOOK),
        );
    }
    choice
}
