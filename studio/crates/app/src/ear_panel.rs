//! Settings > Duck Ear: install the Ear, remove it, and on a PC with a big
//! graphics card try a bigger describe model.
//!
//! The Duck Ear is an optional program that listens to a sound with
//! models on this PC, so an AI assistant can ask "does this sound like a
//! coin pickup?". Nothing here happens by itself: every download starts
//! with a button and a question that shows the size.
//!
//! * **Install** puts the Ear in with scores only (no describe model has
//!   passed Duck Echo's listening test). **Remove** takes it out again.
//! * **Try a bigger model (Large)** is shown only when a graphics card
//!   with 16 GB or more is found. It downloads llama.cpp and the model,
//!   asks the model about the whole test set, and shows how it did, with
//!   a button to save the two result files. It never makes the model the
//!   Ear's; only if the model reached every bar is there a button to use
//!   it.
//!
//! The work itself is in `duck_echo_tools` (`ear_install`, `ear_try`),
//! shared with `duck-echo ear install` and `duck-echo ear try large`. It
//! runs on its own thread, so the window never waits.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use duck_echo_tools::ear_cards::{self, Card};
use duck_echo_tools::ear_install::{self, Event, InstallError, Installed};
use duck_echo_tools::ear_net::{Cancel, megabytes};
use duck_echo_tools::ear_try;
use eframe::egui::{self, RichText};

use crate::theme;

/// The size a person tries from here.
const LARGE: &str = "large";

/// A question the panel is asking before it does something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// Install (or update) the Ear?
    Install,
    /// Remove the Ear?
    Remove,
    /// Download and try the Large model?
    TryLarge,
}

/// How a job ended.
#[derive(Debug)]
pub enum Outcome {
    /// The Ear was installed, or why not.
    Installed(Result<Installed, InstallError>),
    /// The Ear was removed (bytes freed), or why not.
    Removed(Result<u64, InstallError>),
    /// The Large model was tried (its summary), or why not.
    Tried(Result<String, InstallError>),
}

/// What a job's thread tells the window.
enum Told {
    Step(String),
    Download {
        what: String,
        done: u64,
        of: Option<u64>,
    },
    Question {
        done: usize,
        of: usize,
    },
    Done(Outcome),
}

/// Work in progress: an install, a removal or a try.
pub struct Job {
    /// "Installing the Duck Ear" ...
    pub title: &'static str,
    /// The step it is at, in words.
    pub step: String,
    /// What is arriving, or which question it is at.
    pub detail: String,
    /// How far along that is, 0 to 1, when known.
    pub fraction: Option<f32>,
    cancel: Cancel,
    told: Receiver<Told>,
}

/// One measure of a try, for the results table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Its name.
    pub name: String,
    /// "36 of 47 (77%)".
    pub got: String,
    /// "80%".
    pub bar: String,
    /// Did it reach the bar?
    pub reached: bool,
}

/// The last try of the Large model, as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Results {
    /// The model, `name@revision`.
    pub model: String,
    /// Where it ran, in words.
    pub device: String,
    /// Its measures.
    pub rows: Vec<Row>,
    /// Its times, in one line.
    pub times: String,
    /// Reached every bar a describe model must reach, on the whole set?
    pub passed: bool,
    /// The bars it missed, in words.
    pub missed: Vec<String>,
}

impl Results {
    /// The results kept beside the Large model in `home`, if it was tried.
    pub fn read(home: &Path) -> Option<Results> {
        let report = ear_try::last_results(home, LARGE)?;
        let last = ear_try::last_check(home, LARGE)?;
        let describe = &report["describe"];
        let rows = describe["measures"]
            .as_array()?
            .iter()
            .filter_map(|m| {
                let (right, of) = (m["right"].as_u64()?, m["of"].as_u64()?);
                Some(Row {
                    name: m["name"].as_str()?.to_string(),
                    got: format!("{right} of {of} ({:.0}%)", m["share"].as_f64()? * 100.0),
                    bar: format!("{:.0}%", m["bar"].as_f64()? * 100.0),
                    reached: m["passed"].as_bool()?,
                })
            })
            .collect();
        let times = &describe["times"];
        Some(Results {
            model: describe["model"].as_str()?.to_string(),
            device: describe["device_name"]
                .as_str()
                .or(describe["device"].as_str())
                .unwrap_or("?")
                .to_string(),
            rows,
            times: format!(
                "Loading the model {:.0} s, hearing a sound {:.1} s, a further question {:.1} s.",
                times["load"].as_f64().unwrap_or(0.0),
                times["describe_mean"].as_f64().unwrap_or(0.0),
                times["ask_mean"].as_f64().unwrap_or(0.0)
            ),
            passed: last.passed && last.whole_set,
            missed: last.missed,
        })
    }
}

/// Settings > Duck Ear.
pub struct EarPanel {
    /// The Ear's folder.
    pub home: PathBuf,
    /// This PC's graphics cards, the biggest first.
    pub cards: Vec<Card>,
    /// The installed Ear, if there is one.
    pub installed: Option<Installed>,
    /// The installed Ear's own code is not this Studio's: Update brings it
    /// up to date.
    pub needs_update: bool,
    /// How much the Ear takes on disk.
    pub disk: u64,
    /// The describe model the Ear uses, if a person chose one.
    pub in_use: Option<String>,
    /// The last try of the Large model.
    pub large: Option<Results>,
    /// The question being asked, if any.
    pub asking: Option<Asking>,
    /// The work in progress, if any.
    pub job: Option<Job>,
    /// What the last job said: a problem (true) or good news, in words.
    pub note: Option<(bool, String)>,
    /// The PC was looked at (cards, what is installed).
    looked: bool,
}

impl Default for EarPanel {
    fn default() -> EarPanel {
        EarPanel {
            home: duck_echo_tools::ear::default_home(),
            cards: Vec::new(),
            installed: None,
            needs_update: false,
            disk: 0,
            in_use: None,
            large: None,
            asking: None,
            job: None,
            note: None,
            looked: false,
        }
    }
}

impl EarPanel {
    /// The panel for the Ear in `home` on a PC with these graphics cards
    /// (tests name both; the Studio looks when Settings is first opened).
    pub fn at(home: impl Into<PathBuf>, cards: Vec<Card>) -> EarPanel {
        let mut panel = EarPanel {
            home: home.into(),
            cards,
            looked: true,
            ..EarPanel::default()
        };
        panel.refresh();
        panel
    }

    /// Read again what is in the Ear's folder.
    pub fn refresh(&mut self) {
        self.installed = ear_install::installed(&self.home);
        self.needs_update = ear_install::needs_update(&self.home);
        self.disk = ear_install::disk_use(&self.home);
        self.in_use = ear_try::model_in_use(&self.home);
        self.large = Results::read(&self.home);
    }

    /// Is the Large model offered here? Only with the Ear installed and a
    /// graphics card of 16 GB or more.
    pub fn large_offered(&self) -> bool {
        self.installed.is_some()
            && ear_try::model_named(LARGE).is_ok_and(|m| ear_try::offered(&m, &self.cards).is_ok())
    }

    fn start(
        &mut self,
        title: &'static str,
        work: impl FnOnce(&mut dyn FnMut(Event), &Cancel) -> Outcome + Send + 'static,
    ) {
        let (tell, told) = channel();
        let cancel = Cancel::new();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let mut events = |event: Event| {
                let _ = tell.send(match event {
                    Event::Step(words) => Told::Step(words.to_string()),
                    Event::Download { what, done, of } => Told::Download {
                        what: what.to_string(),
                        done,
                        of,
                    },
                    Event::Question { done, of } => Told::Question { done, of },
                });
            };
            let outcome = work(&mut events, &stop);
            let _ = tell.send(Told::Done(outcome));
        });
        self.asking = None;
        self.note = None;
        self.job = Some(Job {
            title,
            step: "Starting".to_string(),
            detail: String::new(),
            fraction: None,
            cancel,
            told,
        });
    }

    /// Install the Ear (or bring it up to date), after the person said yes.
    pub fn install(&mut self) {
        let home = self.home.clone();
        self.start("Installing the Duck Ear", move |events, cancel| {
            Outcome::Installed(ear_install::install(&home, events, cancel))
        });
    }

    /// Remove the Ear, after the person said yes.
    pub fn remove(&mut self) {
        let home = self.home.clone();
        self.start("Removing the Duck Ear", move |_, _| {
            Outcome::Removed(ear_install::remove(&home))
        });
    }

    /// Download and try the Large model, after the person said yes.
    pub fn try_large(&mut self) {
        let home = self.home.clone();
        let cards = self.cards.clone();
        self.start("Trying the Large model", move |events, cancel| {
            Outcome::Tried(
                ear_try::try_model(&home, LARGE, false, &cards, events, cancel)
                    .map(|tried| tried.summary),
            )
        });
    }

    /// Hear what the job's thread said since the last look. Returns true
    /// while a job runs.
    pub fn poll(&mut self) -> bool {
        let Some(job) = &mut self.job else {
            return false;
        };
        let mut done = None;
        while let Ok(told) = job.told.try_recv() {
            match told {
                Told::Step(words) => {
                    job.step = words;
                    job.detail.clear();
                    job.fraction = None;
                }
                Told::Download { what, done, of } => {
                    job.fraction = of.filter(|of| *of > 0).map(|of| done as f32 / of as f32);
                    job.detail = match of {
                        Some(of) => format!("{what}: {} of {}", megabytes(done), megabytes(of)),
                        None => format!("{what}: {}", megabytes(done)),
                    };
                }
                Told::Question { done, of } => {
                    job.fraction = Some(done as f32 / of.max(1) as f32);
                    job.detail = format!("Question {} of {of}", done + 1);
                }
                Told::Done(outcome) => done = Some(outcome),
            }
        }
        let Some(outcome) = done else {
            return true;
        };
        self.job = None;
        self.refresh();
        self.note = Some(match outcome {
            Outcome::Installed(Ok(installed)) => (
                false,
                format!(
                    "The Duck Ear {} is installed. An AI assistant can now ask it to listen.",
                    installed.ear_version
                ),
            ),
            Outcome::Removed(Ok(freed)) => (
                false,
                format!(
                    "The Duck Ear was removed: {} is free again.",
                    megabytes(freed)
                ),
            ),
            Outcome::Tried(Ok(summary)) => (false, summary),
            Outcome::Installed(Err(error))
            | Outcome::Removed(Err(error))
            | Outcome::Tried(Err(error)) => (error.code != "cancelled", error.message),
        });
        false
    }

    /// Save the Large model's two result files into a folder
    /// `ear-check-large` inside `folder`.
    pub fn save_results_into(&mut self, folder: &Path) {
        let to = folder.join("ear-check-large");
        self.note = Some(match ear_try::save_results(&self.home, LARGE, &to) {
            Ok(_) => (
                false,
                format!(
                    "Saved ear-check.json and ear-check.md in {}. Those two files are the ones to send.",
                    to.display()
                ),
            ),
            Err(error) => (true, error.message),
        });
    }

    /// Make Large the Ear's describe model (`true`), or go back to scores
    /// only.
    pub fn use_large(&mut self, yes: bool) {
        self.note = Some(match ear_try::use_model(&self.home, yes.then_some(LARGE)) {
            Ok(words) => (
                false,
                words.replace("`duck-echo ear use none`", "\"Use scores only\""),
            ),
            Err(error) => (true, error.message),
        });
        self.refresh();
    }

    /// Draw the section.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.looked {
            self.looked = true;
            self.cards = ear_cards::cards();
            self.refresh();
        }
        if self.poll() {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        ui.label(RichText::new("Duck Ear").strong());
        ui.label(
            RichText::new(
                "An optional program that listens to a sound and says how well words fit it, so \
                 an AI assistant can check its work. It listens on this PC only: nothing is sent \
                 anywhere.",
            )
            .weak(),
        );
        if let Some(job) = &self.job {
            ui.add_space(4.0);
            ui.label(format!("{}: {}", job.title, job.step));
            let bar = match job.fraction {
                Some(fraction) => egui::ProgressBar::new(fraction).show_percentage(),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.add(bar.desired_width(360.0));
            if !job.detail.is_empty() {
                ui.label(RichText::new(&job.detail).weak());
            }
            if ui.button("Stop").clicked() {
                job.cancel.cancel();
            }
            return;
        }
        if let Some((problem, words)) = &self.note {
            ui.add_space(4.0);
            let words = RichText::new(words);
            ui.label(if *problem {
                words.color(ui.visuals().warn_fg_color)
            } else {
                words
            });
        }
        ui.add_space(4.0);
        match self.asking {
            Some(Asking::Install) => self.ask_install(ui),
            Some(Asking::Remove) => self.ask_remove(ui),
            Some(Asking::TryLarge) => self.ask_try(ui),
            None => {
                self.status(ui);
                if self.installed.is_some() {
                    ui.add_space(6.0);
                    self.bigger_model(ui);
                }
            }
        }
    }

    fn ask_install(&mut self, ui: &mut egui::Ui) {
        match ear_install::plan(&self.home) {
            Ok(plan) => {
                ui.label(plan.summary);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("Install")).clicked() {
                        self.install();
                    }
                    if ui.button("Not now").clicked() {
                        self.asking = None;
                    }
                });
            }
            Err(error) => {
                self.note = Some((true, error.message));
                self.asking = None;
            }
        }
    }

    fn ask_remove(&mut self, ui: &mut egui::Ui) {
        ui.label(format!(
            "Remove the Duck Ear from {}? That frees {}: its programs, its models and its kept \
             answers. Your sounds are not touched.",
            self.home.display(),
            megabytes(self.disk)
        ));
        ui.horizontal(|ui| {
            if ui.button("Remove").clicked() {
                self.remove();
            }
            if ui.add(theme::primary_button("Keep it")).clicked() {
                self.asking = None;
            }
        });
    }

    fn ask_try(&mut self, ui: &mut egui::Ui) {
        match ear_try::try_plan(&self.home, LARGE, false, &self.cards) {
            Ok(plan) => {
                ui.label(plan.summary);
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("Download and try")).clicked() {
                        self.try_large();
                    }
                    if ui.button("Not now").clicked() {
                        self.asking = None;
                    }
                });
            }
            Err(error) => {
                self.note = Some((true, error.message));
                self.asking = None;
            }
        }
    }

    /// What is installed, with Install, Update and Remove.
    fn status(&mut self, ui: &mut egui::Ui) {
        let Some(installed) = self.installed.clone() else {
            ui.label("The Duck Ear is not installed.");
            let size = ear_install::plan(&self.home)
                .map(|plan| format!(" (about {})", megabytes(plan.download_bytes)))
                .unwrap_or_default();
            if ui
                .add(theme::primary_button(&format!(
                    "Install the Duck Ear{size}"
                )))
                .clicked()
            {
                self.asking = Some(Asking::Install);
            }
            return;
        };
        ui.label(format!(
            "Installed: Duck Ear {}, {} on disk.",
            installed.ear_version,
            megabytes(self.disk)
        ));
        ui.label(RichText::new(self.home.display().to_string()).weak());
        match &self.in_use {
            Some(model) => {
                ui.label(format!("It scores, and describes with {model}."));
            }
            None => {
                ui.label(
                    RichText::new(
                        "It gives scores only: no describe model has passed Duck Echo's \
                         listening test.",
                    )
                    .weak(),
                );
            }
        }
        if self.needs_update {
            ui.label("This Studio comes with a newer Ear.");
        }
        ui.horizontal(|ui| {
            if self.needs_update {
                if ui
                    .add(theme::primary_button("Update the Duck Ear"))
                    .clicked()
                {
                    self.asking = Some(Asking::Install);
                }
            } else if ui.button("Install again").clicked() {
                self.asking = Some(Asking::Install);
            }
            if ui.button("Remove the Duck Ear").clicked() {
                self.asking = Some(Asking::Remove);
            }
            if ui.button("Open the Ear's folder").clicked() {
                crate::settings::open_folder(&self.home);
            }
            if self.in_use.is_some() && ui.button("Use scores only").clicked() {
                self.use_large(false);
            }
        });
    }

    /// "Try a bigger model", its results, Save and Use.
    fn bigger_model(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("A bigger model").strong());
        let large = ear_try::model_named(LARGE).ok();
        if !self.large_offered() {
            let why = large
                .as_ref()
                .and_then(|m| ear_try::offered(m, &self.cards).err())
                .unwrap_or_default();
            ui.label(
                RichText::new(format!(
                    "A bigger describe model can be tried on a PC with a graphics card of 16 GB \
                     or more ({why})."
                ))
                .weak(),
            );
            return;
        }
        let size = large.map(|m| megabytes(m.bytes())).unwrap_or_default();
        if let Some(card) = self.cards.first() {
            ui.label(format!("Your graphics card: {}.", card.words()));
        }
        ui.label(
            RichText::new(
                "It downloads llama.cpp and the Qwen3-Omni-30B-A3B model, asks the model about \
                 Duck Echo's 41 test sounds, and shows how it did. The Ear is not changed.",
            )
            .weak(),
        );
        let label = if self.large.is_some() {
            format!("Try the Large model again ({size})")
        } else {
            format!("Try a bigger model (Large, {size})")
        };
        if ui.button(label).clicked() {
            self.asking = Some(Asking::TryLarge);
        }
        let Some(results) = self.large.clone() else {
            return;
        };
        ui.add_space(6.0);
        ui.label(format!("{} on {}", results.model, results.device));
        egui::Grid::new("ear-large-results")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for head in ["Measure", "Right", "Bar", "Reached"] {
                    ui.label(RichText::new(head).strong());
                }
                ui.end_row();
                for row in &results.rows {
                    ui.label(&row.name);
                    ui.label(&row.got);
                    ui.label(&row.bar);
                    ui.label(if row.reached { "yes" } else { "no" });
                    ui.end_row();
                }
            });
        ui.label(RichText::new(&results.times).weak());
        if results.passed {
            ui.label("It reached every bar a describe model must reach.");
        } else {
            ui.label(format!(
                "It did not reach every bar, so the Ear keeps scores only: {}.",
                results.missed.join("; ")
            ));
        }
        ui.horizontal(|ui| {
            if ui.button("Save the results...").clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Where to save ear-check.json and ear-check.md?")
                    .pick_folder()
            {
                self.save_results_into(&folder);
            }
            let using = self.in_use.is_some();
            if results.passed
                && !using
                && ui
                    .add(theme::primary_button("Use Large for describing"))
                    .clicked()
            {
                self.use_large(true);
            }
        });
    }
}
