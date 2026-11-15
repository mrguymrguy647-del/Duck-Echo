//! The sound editor: a recipe's text on the left, sliders on the right,
//! the picture and the summary line below, and Play, Loop and Stop.
//!
//! **The text and the sliders always agree**, because both change the one
//! file, through the same commands an AI assistant uses:
//!
//! * Typing runs `file_set_text` a moment after the typing stops, if the
//!   text is a working recipe. Until then (and while it has a mistake) the
//!   problems show under their lines, with the suggestion as a button.
//! * A slider runs `sound_set`. The text shows the new value at once.
//! * Anything else (an Undo, an AI assistant, a text editor) changes the
//!   file on disk, and the editor reads it again within half a second.
//!
//! **A drag never floods the disk.** The Documents folder is often synced
//! by OneDrive (or Dropbox, or a Linux sync tool), and every save is an
//! upload. While a slider is dragged you hear each new value at once, but
//! the file is saved at most every [`SAVE_EVERY`] (about 5 times a
//! second), and once more when the mouse is let go, so the file always
//! ends at the value you let go on. All the saves of one drag share a
//! stroke number, so they make one Undo step.
//!
//! **What you hear is the file.** Each change is rendered on a background
//! thread with the same code as `render`; the finished sound takes over
//! from the one playing at the same place, with the mixer's 10 ms
//! cross-fade, so a slider change is smooth. The same render draws the
//! picture and the summary line agents read.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Format, Issue, Recipe, Severity};
use duck_echo_live::ParamUpdate;
use duck_echo_synth::Mastering;
use duck_echo_tools::commands::{self, Command};
use duck_echo_tools::edit::{self, EditOp};
use duck_echo_tools::history::Who;
use duck_echo_tools::project::{HistoryLine, Project};
use duck_echo_tools::root::Root;
use eframe::egui::{self, Color32, RichText};
use serde_json::{Value, json};

use crate::controls::{self, Edit};

/// While a slider is dragged, the file is saved at most this often.
pub const SAVE_EVERY: Duration = Duration::from_millis(200);

/// Typing is saved this long after the last key.
pub const TYPING_PAUSE: Duration = Duration::from_millis(600);

/// How often the editor looks at the file on disk for changes made
/// elsewhere (an Undo, an AI assistant, a text editor).
pub const LOOK_EVERY: Duration = Duration::from_millis(500);

/// A slider being dragged.
#[derive(Debug, Clone)]
struct Drag {
    /// The stroke number all its saves share (one Undo step).
    stroke: u64,
    /// The field it moves.
    path: String,
    /// The value it is at now.
    value: Value,
    /// The value last saved, and when.
    saved: Option<(Value, Instant)>,
}

/// A finished render of the sound, from the background thread.
pub struct Heard {
    /// Which change it is the render of: the editor asks for a render
    /// after each change, and only the newest is shown.
    pub generation: u64,
    /// The sound, exactly as `render` writes it.
    pub buffer: AudioBuffer,
    /// The picture: level over time on top, the spectrogram below.
    pub picture: egui::ColorImage,
    /// The summary line agents read.
    pub summary: String,
    /// The length in seconds.
    pub seconds: f64,
    /// The recipe it is the render of.
    pub recipe: Recipe,
    /// Its level and length as a file, for a live voice that sounds the
    /// same (see `duck_echo_live::Sound::from_recipe_mastered`).
    pub mastering: Mastering,
}

/// The background thread that renders each change.
struct Preview {
    jobs: Sender<(u64, Recipe)>,
    done: Receiver<Result<Heard, String>>,
    asked: u64,
    answered: u64,
}

impl Preview {
    fn new() -> Preview {
        let (jobs, inbox) = mpsc::channel::<(u64, Recipe)>();
        let (outbox, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("sound editor renders".into())
            .spawn(move || {
                while let Ok(mut job) = inbox.recv() {
                    // Only the newest change matters: a drag asks for many.
                    while let Ok(newer) = inbox.try_recv() {
                        job = newer;
                    }
                    let (generation, recipe) = job;
                    if outbox.send(hear(generation, &recipe)).is_err() {
                        break;
                    }
                }
            })
            .expect("a thread for the sound editor's renders");
        Preview {
            jobs,
            done,
            asked: 0,
            answered: 0,
        }
    }

    fn ask(&mut self, recipe: Recipe) {
        self.asked += 1;
        let _ = self.jobs.send((self.asked, recipe));
    }

    /// The newest render that has arrived, if any.
    fn poll(&mut self) -> Option<Result<Heard, String>> {
        let mut newest = None;
        while let Ok(heard) = self.done.try_recv() {
            if let Ok(h) = &heard {
                self.answered = h.generation;
            } else {
                self.answered = self.asked;
            }
            newest = Some(heard);
        }
        newest
    }

    fn busy(&self) -> bool {
        self.answered < self.asked
    }
}

/// Render `recipe` and draw its picture, on the background thread.
fn hear(generation: u64, recipe: &Recipe) -> Result<Heard, String> {
    let (buffer, mastering) =
        duck_echo_synth::render_mastered(recipe).map_err(|r| r.to_string())?;
    let analysis = duck_echo_tools::analyze_buffer(&buffer);
    let onsets: Vec<f64> = analysis.onsets.iter().map(|o| o.time_seconds).collect();
    let (_, image) = duck_echo_analyze::picture::draw_image(
        &buffer.samples,
        buffer.sample_rate,
        buffer.channels,
        &onsets,
        &[],
    );
    let palette: Vec<Color32> = image
        .palette
        .iter()
        .map(|[r, g, b]| Color32::from_rgb(*r, *g, *b))
        .collect();
    let pixels = image
        .pixels
        .iter()
        .map(|i| palette.get(*i as usize).copied().unwrap_or(Color32::BLACK))
        .collect();
    let picture = egui::ColorImage::new([image.width as usize, image.height as usize], pixels);
    Ok(Heard {
        generation,
        seconds: buffer.duration_seconds(),
        buffer,
        picture,
        summary: analysis.summary,
        recipe: recipe.clone(),
        mastering,
    })
}

/// One sound open in the editor.
pub struct Editor {
    /// The file, relative to the project folder.
    pub rel: String,
    format: Format,
    /// The text box.
    pub text: String,
    /// The text the file held when last read or saved.
    saved: String,
    /// When the person last typed, while that typing is not saved yet.
    typed_at: Option<Instant>,
    /// The problems of [`Editor::text`], errors first.
    pub problems: Vec<Issue>,
    /// The recipe as written, from the last text that worked.
    doc: Option<Value>,
    drag: Option<Drag>,
    strokes: u64,
    /// How many times the editor has saved the file (for the tests: a
    /// drag must not save more than about 5 times a second).
    pub saves: u64,
    /// Play repeats the sound.
    pub looping: bool,
    /// The History panel is open.
    pub history_open: bool,
    /// The steps of the history, as last read (newest first).
    pub history: Vec<HistoryLine>,
    preview: Preview,
    /// The newest finished render.
    pub heard: Option<Heard>,
    /// Why the newest render failed, if it did.
    pub render_problem: Option<String>,
    picture: Option<(u64, egui::TextureHandle)>,
    looked_at: Instant,
    schema: Value,
    /// True when the file is no longer there (deleted or renamed
    /// elsewhere).
    pub gone: bool,
    /// Play this once the render arrives: `Some(looping)`.
    pub play_when_ready: Option<bool>,
    /// The recipe of the newest text that works.
    recipe: Option<Recipe>,
    /// What the editor has seen of the shared history.
    watch: crate::together::Watch,
    /// Someone else's change to this file, for the three-second note.
    pub visit: Option<crate::together::Visit>,
    /// The file's text when someone else changed it while the person was
    /// typing: the typing is not saved over it until the person chooses
    /// (see [`Editor::keep_mine`] and [`Editor::use_theirs`]).
    pub conflict: Option<String>,
    /// What the speakers play, kept in step with each edit.
    pub live: Live,
}

impl Editor {
    /// Open the sound `rel` of the project.
    pub fn open(root: &Root, rel: &str) -> Result<Editor, String> {
        let text = read(root, rel)?;
        let mut editor = Editor {
            rel: rel.to_string(),
            format: Format::from_path(Path::new(rel)),
            saved: text.clone(),
            text,
            typed_at: None,
            problems: Vec::new(),
            doc: None,
            drag: None,
            strokes: 0,
            saves: 0,
            looping: false,
            history_open: false,
            history: Vec::new(),
            preview: Preview::new(),
            heard: None,
            render_problem: None,
            picture: None,
            looked_at: Instant::now(),
            schema: duck_echo_core::schema::recipe_schema(),
            gone: false,
            play_when_ready: None,
            recipe: None,
            live: Live::default(),
            watch: crate::together::Watch::new(root),
            visit: None,
            conflict: None,
        };
        editor.check_text(true);
        editor.read_history(root);
        Ok(editor)
    }

    /// The file's name without its folder or extension: "coin".
    pub fn name(&self) -> String {
        Path::new(&self.rel)
            .file_stem()
            .map_or(self.rel.clone(), |s| s.to_string_lossy().into_owned())
    }

    /// True while a render is being made.
    pub fn rendering(&self) -> bool {
        self.preview.busy()
    }

    /// True while typing or a drag is waiting to be saved.
    pub fn unsaved(&self) -> bool {
        self.typed_at.is_some()
            || self
                .drag
                .as_ref()
                .is_some_and(|d| d.saved.as_ref().map(|s| &s.0) != Some(&d.value))
    }

    /// Check the text: its problems, and (if it works) its document and a
    /// new render. `render` is false when the text is the one just
    /// rendered.
    fn check_text(&mut self, render: bool) {
        match duck_echo_core::load_str(&self.text, self.format) {
            Ok(loaded) => {
                self.problems = loaded.report.warnings.clone();
                self.doc = edit::parse_document(&self.text, self.format).ok();
                self.recipe = Some(loaded.recipe.clone());
                if render {
                    self.preview.ask(loaded.recipe);
                }
            }
            Err(report) => {
                let mut problems = report.errors;
                problems.extend(report.warnings);
                self.problems = problems;
            }
        }
    }

    /// True when the text has no errors.
    pub fn works(&self) -> bool {
        !self.problems.iter().any(|p| p.severity == Severity::Error)
    }

    /// The person typed: check the text now, save it a moment after the
    /// typing stops.
    pub fn typed(&mut self, now: Instant) {
        self.typed_at = Some(now);
        self.check_text(true);
    }

    /// Run a command on this sound as the person.
    fn run(
        &mut self,
        root: &Root,
        name: &str,
        args: Value,
        stroke: Option<u64>,
    ) -> Result<Value, String> {
        let result = commands::parse(name, &args, "")
            .and_then(|command: Command| {
                commands::run(&Project::new(root), command, Who::Person, stroke, "")
            })
            .map_err(|e| {
                let report = e.into_report();
                report
                    .errors
                    .first()
                    .map_or_else(|| report.to_string(), |i| i.message.clone())
            });
        if result.is_ok() {
            self.saves += 1;
        }
        result
    }

    /// Call every frame (and from the tests, with the time): save typing
    /// after its pause, save a held slider at most every [`SAVE_EVERY`],
    /// and look for changes made elsewhere. Returns a problem to show.
    pub fn tick(&mut self, root: &Root, now: Instant) -> Option<String> {
        let mut problem = None;
        if let Some(at) = self.typed_at
            && now.duration_since(at) >= TYPING_PAUSE
        {
            problem = self.save_typing(root).err();
        }
        if let Some(drag) = &self.drag {
            let due = drag
                .saved
                .as_ref()
                .is_none_or(|(v, at)| *v != drag.value && now.duration_since(*at) >= SAVE_EVERY);
            if due {
                problem = self.save_drag(root, now).err().or(problem);
            }
        }
        if now.duration_since(self.looked_at) >= LOOK_EVERY {
            self.look(root);
            self.looked_at = now;
        }
        problem
    }

    /// Save what was typed, if it is a working recipe.
    fn save_typing(&mut self, root: &Root) -> Result<(), String> {
        if !self.works() {
            // Nothing is saved while the text has a mistake: the file
            // keeps its last working text, and an AI assistant reading it
            // meanwhile reads a working recipe. The problem shows under
            // its line.
            self.typed_at = None;
            return Ok(());
        }
        self.typed_at = None;
        if self.text == self.saved || self.conflict.is_some() {
            return Ok(());
        }
        // Saving the whole text over a file someone else changed meanwhile
        // would quietly undo their change. Ask first.
        if let Ok(disk) = read(root, &self.rel)
            && disk != self.saved
        {
            self.conflict = Some(disk);
            if let Some(visit) = self.watch.changed(root, &self.rel, Instant::now()) {
                self.visit = Some(visit);
            }
            return Err(
                "The file changed while you typed (an AI assistant or another program). \
                 Choose Keep mine or Use theirs."
                    .into(),
            );
        }
        let text = self.text.clone();
        self.run(
            root,
            "file_set_text",
            json!({"file": self.rel, "text": text}),
            None,
        )?;
        self.saved = text;
        self.read_history(root);
        Ok(())
    }

    /// Save the held slider's value now.
    fn save_drag(&mut self, root: &Root, now: Instant) -> Result<(), String> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        let args = json!({"file": self.rel, "path": drag.path, "value": drag.value});
        let result = self.run(root, "sound_set", args, Some(drag.stroke));
        if let Some(d) = &mut self.drag {
            d.saved = Some((drag.value.clone(), now));
        }
        result?;
        self.adopt_file(root);
        Ok(())
    }

    /// A slider moved to `value` and is still held. You hear it at once;
    /// the file is saved now if the last save was [`SAVE_EVERY`] ago (or
    /// this is the drag's first value), else later.
    pub fn slide(
        &mut self,
        root: &Root,
        path: &str,
        value: Value,
        now: Instant,
    ) -> Result<(), String> {
        if self.drag.as_ref().is_some_and(|d| d.path != path) {
            self.release(root, now)?;
        }
        if self.typed_at.is_some() {
            self.save_typing(root)?;
        }
        if self.drag.is_none() {
            self.strokes += 1;
            self.drag = Some(Drag {
                stroke: new_stroke(self.strokes),
                path: path.to_string(),
                value: value.clone(),
                saved: None,
            });
        }
        if let Some(d) = &mut self.drag {
            d.value = value.clone();
        }
        // The text shows the new value at once, and the sound follows.
        self.show_value(path, value)?;
        let due = self
            .drag
            .as_ref()
            .and_then(|d| d.saved.as_ref())
            .is_none_or(|(_, at)| now.duration_since(*at) >= SAVE_EVERY);
        if due {
            self.save_drag(root, now)?;
        }
        Ok(())
    }

    /// The slider was let go: save its last value (if it is not saved
    /// yet), so the file ends at the value let go on.
    pub fn release(&mut self, root: &Root, now: Instant) -> Result<(), String> {
        let Some(drag) = self.drag.clone() else {
            return Ok(());
        };
        let result = if drag.saved.as_ref().map(|s| &s.0) != Some(&drag.value) {
            self.save_drag(root, now)
        } else {
            Ok(())
        };
        self.drag = None;
        self.read_history(root);
        result
    }

    /// Put `value` at `path` in the text the person sees (keeping its
    /// comments), and render the result.
    fn show_value(&mut self, path: &str, value: Value) -> Result<(), String> {
        let mut doc = self
            .doc
            .clone()
            .or_else(|| edit::parse_document(&self.saved, self.format).ok())
            .ok_or("The text has a mistake. Fix it first, then move the slider.")?;
        edit::apply_edits(
            &mut doc,
            &[EditOp {
                path: path.to_string(),
                value: Some(value),
                remove: false,
            }],
        )
        .map_err(|e| e.into_report().to_string())?;
        let base = if self.works() {
            self.text.clone()
        } else {
            self.saved.clone()
        };
        self.text = edit::document_text_from(Path::new(&self.rel), &base, &doc)
            .map_err(|e| e.into_report().to_string())?;
        self.check_text(true);
        Ok(())
    }

    /// Do what the person did in the panel.
    pub fn apply(&mut self, root: &Root, edit: Edit, now: Instant) -> Result<(), String> {
        let file = self.rel.clone();
        let command = match edit {
            Edit::Set {
                path,
                value,
                held: true,
            } => return self.slide(root, &path, value, now),
            Edit::Set {
                path,
                value,
                held: false,
            } => {
                self.slide(root, &path, value, now)?;
                return self.release(root, now);
            }
            Edit::Release => return self.release(root, now),
            Edit::Remove { path } => ("sound_remove", json!({"file": file, "path": path})),
            Edit::AddLayer => ("sound_add_layer", json!({"file": file})),
            Edit::RemoveLayer(index) => {
                ("sound_remove_layer", json!({"file": file, "index": index}))
            }
            Edit::AddEffect { layer, kind } => {
                let mut args = json!({"file": file, "effect": {"type": kind}});
                if let Some(l) = layer {
                    args["layer"] = json!(l);
                }
                ("sound_add_effect", args)
            }
            Edit::RemoveEffect { layer, index } => {
                let mut args = json!({"file": file, "index": index});
                if let Some(l) = layer {
                    args["layer"] = json!(l);
                }
                ("sound_remove_effect", args)
            }
        };
        self.release(root, now)?;
        self.save_typing(root)?;
        self.run(root, command.0, command.1, None)?;
        self.adopt_file(root);
        self.read_history(root);
        Ok(())
    }

    /// Read the file again now (after an Undo or Redo, say).
    pub fn reload(&mut self, root: &Root) {
        self.look(root);
        self.read_history(root);
    }

    /// Look at the file on disk: if it changed elsewhere and nothing here
    /// is waiting to be saved, show the new text.
    fn look(&mut self, root: &Root) {
        match read(root, &self.rel) {
            Ok(text) => {
                self.gone = false;
                if text != self.saved
                    && self.typed_at.is_none()
                    && self.drag.is_none()
                    && self.conflict.is_none()
                {
                    self.saved = text.clone();
                    self.text = text;
                    self.check_text(true);
                    // Who changed it? An AI assistant or another program
                    // gets the three-second note.
                    if let Some(visit) = self.watch.changed(root, &self.rel, Instant::now()) {
                        self.visit = Some(visit);
                    }
                    self.read_history(root);
                }
            }
            Err(_) => self.gone = true,
        }
    }

    /// After a conflict: save the person's text over the file. The other
    /// change stays in the History, so Undo can still bring it back.
    pub fn keep_mine(&mut self, root: &Root) -> Result<(), String> {
        let Some(theirs) = self.conflict.take() else {
            return Ok(());
        };
        self.saved = theirs;
        self.typed_at = Some(Instant::now());
        self.save_typing(root)
    }

    /// After a conflict: drop the person's typing and take the file as it
    /// is now.
    pub fn use_theirs(&mut self, root: &Root) {
        if let Some(theirs) = self.conflict.take() {
            self.saved = theirs.clone();
            self.text = theirs;
            self.typed_at = None;
            self.check_text(true);
            self.read_history(root);
        }
    }

    /// The file was just saved by a command: take its text as it is now.
    fn adopt_file(&mut self, root: &Root) {
        if let Ok(text) = read(root, &self.rel) {
            let render = text != self.text;
            self.saved = text.clone();
            self.text = text;
            self.check_text(render);
        }
    }

    fn read_history(&mut self, root: &Root) {
        if let Ok(list) = Project::new(root).history_list(50) {
            self.history = list.steps;
        }
    }

    /// Before an Undo or Redo: save what waits. If the typed text has a
    /// mistake, Undo takes the typing back instead: returns true then.
    pub fn before_undo(&mut self, root: &Root, now: Instant) -> bool {
        if self.conflict.is_some() {
            // Keep mine or Use theirs first; Undo would only confuse it.
            return true;
        }
        let _ = self.release(root, now);
        if self.typed_at.is_some() || self.text != self.saved {
            if self.works() {
                let _ = self.save_typing(root);
            } else {
                self.text = self.saved.clone();
                self.typed_at = None;
                self.check_text(true);
                return true;
            }
        }
        false
    }

    /// Use the suggestion of problem `index` ("Use `volume`"): put it in
    /// the text in place of what was written.
    pub fn use_suggestion(&mut self, index: usize, now: Instant) {
        let Some(issue) = self.problems.get(index).cloned() else {
            return;
        };
        if let Some(text) = with_suggestion(&self.text, &issue) {
            self.text = text;
            self.typed(now);
        }
    }

    /// The newest render, when one arrives. Returns it once.
    pub fn poll(&mut self) -> Option<&Heard> {
        match self.preview.poll()? {
            Ok(heard) => {
                self.render_problem = None;
                self.heard = Some(heard);
                self.heard.as_ref()
            }
            Err(why) => {
                self.render_problem = Some(why);
                None
            }
        }
    }

    /// Wait (up to `timeout`) for the newest render. For the tests and
    /// for Export WAV of a change that is still rendering.
    pub fn wait_for_render(&mut self, timeout: Duration) -> bool {
        let start = Instant::now();
        while self.preview.busy() && start.elapsed() < timeout {
            self.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        self.poll();
        !self.preview.busy()
    }

    /// The picture of the newest render, as a texture.
    pub fn picture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let heard = self.heard.as_ref()?;
        if self
            .picture
            .as_ref()
            .is_none_or(|(g, _)| *g != heard.generation)
        {
            let texture = ctx.load_texture(
                "sound picture",
                heard.picture.clone(),
                egui::TextureOptions::LINEAR,
            );
            self.picture = Some((heard.generation, texture));
        }
        self.picture.as_ref().map(|(_, t)| t.clone())
    }

    /// The recipe of the newest text that works.
    pub fn recipe(&self) -> Option<&Recipe> {
        self.recipe.as_ref()
    }

    /// True when `heard` is the render of the newest change (a drag asks
    /// for many; one that is already old is only shown, never played).
    pub fn is_newest(&self, heard: &Heard) -> bool {
        heard.generation == self.preview.asked
    }

    /// The schema the sliders are made from.
    pub fn schema(&self) -> &Value {
        &self.schema
    }

    /// The recipe as written (for the sliders).
    pub fn doc(&self) -> Option<&Value> {
        self.doc.as_ref()
    }
}

/// What the speakers play from the editor, kept in step with each edit.
///
/// A change to a layer's volume, a filter's cutoff or the master gain is
/// sent to the playing voice at once as a [`ParamUpdate`], which slides to
/// the new value over 10 ms: you hear the slider move, with no click.
/// When its render arrives, only the level is slid to the file's new
/// level (a normalized sound keeps its loudness). Any other change waits
/// for its render, and the new voice takes over at the same place with a
/// 10 ms cross-fade.
#[derive(Debug, Default)]
pub struct Live {
    /// The recipe the playing voice makes now (with the live changes
    /// sent to it), or `None` when nothing plays.
    playing: Option<Recipe>,
    /// A change was made that only a new voice can play.
    rebuild: bool,
    /// The length of the playing voice, in frames.
    frames: usize,
}

/// What to do when the newest render arrives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// Nothing plays.
    Nothing,
    /// A new voice of the render takes over, with a cross-fade.
    Replace,
    /// The voice already sounds like the render: slide its level to this.
    Level(f32),
}

impl Live {
    /// A voice of `heard` started playing.
    pub fn started(&mut self, heard: &Heard) {
        self.playing = Some(heard.recipe.clone());
        self.frames = heard.mastering.frames;
        self.rebuild = false;
    }

    /// Nothing plays any more.
    pub fn stopped(&mut self) {
        self.playing = None;
        self.rebuild = false;
    }

    /// True while a voice from the editor plays.
    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// The text now makes `recipe`. Returns the live changes to send to
    /// the playing voice now: none when nothing changed, or when only a
    /// new voice can play the change (it then takes over when its render
    /// arrives).
    pub fn changed(&mut self, recipe: &Recipe) -> Vec<ParamUpdate> {
        let Some(playing) = &self.playing else {
            return Vec::new();
        };
        if self.rebuild || playing == recipe {
            return Vec::new();
        }
        match ParamUpdate::between(playing, recipe) {
            Some(updates) => {
                self.playing = Some(recipe.clone());
                updates
            }
            None => {
                self.rebuild = true;
                Vec::new()
            }
        }
    }

    /// The newest render arrived: what to do with it.
    pub fn rendered(&mut self, heard: &Heard) -> Arrival {
        let Some(playing) = &self.playing else {
            return Arrival::Nothing;
        };
        if self.rebuild || *playing != heard.recipe || heard.mastering.frames != self.frames {
            self.started(heard);
            Arrival::Replace
        } else {
            Arrival::Level(heard.mastering.gain)
        }
    }
}

/// A stroke number nobody has used: the time this drag started, in
/// microseconds, plus a count. (It only names the drag's Undo step; it
/// never reaches a sound.)
fn new_stroke(count: u64) -> u64 {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    micros.wrapping_mul(16).wrapping_add(count % 16)
}

/// Read a file of the project through its path rules.
fn read(root: &Root, rel: &str) -> Result<String, String> {
    let path = root
        .read("file", rel)
        .map_err(|e| e.into_report().to_string())?;
    std::fs::read_to_string(&path).map_err(|e| format!("{rel} cannot be read: {e}"))
}

/// `text` with the suggestion of `issue` put in place of what was written
/// on its line: `volme` becomes `volume`, `"sqare"` becomes `"square"`.
pub fn with_suggestion(text: &str, issue: &Issue) -> Option<String> {
    let suggestion = issue.suggestion.as_ref()?;
    let line = issue.line? - 1;
    let wrong = match &issue.got {
        Some(Value::String(s)) => s.clone(),
        _ => issue.path.rsplit(['.', ']']).next()?.to_string(),
    };
    if wrong.is_empty() || wrong == *suggestion {
        return None;
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    let old = *lines.get(line)?;
    let at = find_word(old, &wrong)?;
    let fixed = format!("{}{}{}", &old[..at], suggestion, &old[at + wrong.len()..]);
    lines[line] = &fixed;
    Some(lines.join("\n"))
}

/// Where `word` stands on its own in `line` (not inside a longer word).
fn find_word(line: &str, word: &str) -> Option<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    line.match_indices(word).map(|(i, _)| i).find(|&i| {
        let before = line[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let after = line[i + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word(c));
        before && after
    })
}

/// Where Export WAV saves when the system's Save As window cannot open:
/// `exports` in the project folder.
pub const EXPORTS: &str = "exports";

/// Render the sound `rel` and write it as a WAV file at `to`: exactly the
/// bytes `duck-echo render` writes for the same file. The file is written
/// through the path rules of its folder, in one step (a temporary file,
/// then a rename).
pub fn export_wav(root: &Root, rel: &str, to: &Path) -> Result<PathBuf, String> {
    let text = read(root, rel)?;
    let format = Format::from_path(Path::new(rel));
    let recipe = duck_echo_core::load_str(&text, format)
        .map_err(|r| format!("{rel} has a mistake, so it cannot be exported: {r}"))?
        .recipe;
    let buffer =
        duck_echo_tools::render_buffer(&recipe).map_err(|e| e.into_report().to_string())?;
    let bytes = duck_echo_audio::encode_wav(&buffer, duck_echo_audio::WavFormat::default())
        .map_err(|e| e.to_string())?;
    let folder = to
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = to
        .file_name()
        .ok_or("Choose a file name to save the sound as.")?
        .to_string_lossy()
        .into_owned();
    let name = if name.to_lowercase().ends_with(".wav") {
        name
    } else {
        format!("{name}.wav")
    };
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("{} cannot be made: {e}", folder.display()))?;
    let target_root = Root::new(folder)?;
    let path = target_root
        .write("file", &name)
        .map_err(|e| e.into_report().to_string())?;
    duck_echo_tools::save::write_atomic(&path, &bytes).map_err(|e| e.into_report().to_string())?;
    Ok(path)
}

/// The place Export WAV saves to without asking: `exports/<name>.wav` in
/// the project folder (a new name if that one is taken).
pub fn export_place(root: &Root, rel: &str) -> PathBuf {
    let stem = Path::new(rel)
        .file_stem()
        .map_or("sound".into(), |s| s.to_string_lossy().into_owned());
    let folder = root.dir().join(EXPORTS);
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}.wav")
        } else {
            format!("{stem} {n}.wav")
        };
        let path = folder.join(name);
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

/// Where the Save As window opens: the Music folder, or else Documents.
pub fn music_folder() -> Option<PathBuf> {
    dirs::audio_dir().or_else(dirs::document_dir)
}

/// The text box with line numbers, and each problem under its line.
///
/// egui's text box has no line numbers or messages between lines. So a
/// line with a problem is laid out taller: egui centres a line's text in
/// its height, which leaves room above and below it. The room below holds
/// the message and its suggestion button, and a light band marks the
/// whole of it as one block. Returns the index of a problem whose
/// suggestion button was clicked, and whether the text changed.
pub fn text_box(ui: &mut egui::Ui, text: &mut String, problems: &[Issue]) -> (Option<usize>, bool) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    // Room for two rows of small text and a button.
    let message_room = (row_height * 2.4).round();
    // The lines (0-based) with a problem, and the problems on each.
    let mut on_line: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (i, p) in problems.iter().enumerate() {
        if let Some(line) = p.line {
            on_line.entry(line.saturating_sub(1)).or_default().push(i);
        }
    }
    let text_color = ui.visuals().text_color();
    let problem_color = crate::theme::LOOK;
    let marked: Vec<usize> = on_line.keys().copied().collect();
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = egui::text::LayoutJob::default();
        for (n, line) in buffer.as_str().split_inclusive('\n').enumerate() {
            let has_problem = marked.contains(&n);
            let mut format = egui::TextFormat::simple(font.clone(), text_color);
            if has_problem {
                // Centred in this height: `message_room` above and below.
                format.line_height = Some(row_height + 2.0 * message_room);
            }
            // (egui puts an underline in the wrong place on a tall line,
            // so the line is underlined by hand below.)
            job.append(line, 0.0, format);
        }
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut picked = None;
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let gutter = ui
            .allocate_exact_size(egui::vec2(28.0, 0.0), egui::Sense::hover())
            .0;
        let output = egui::TextEdit::multiline(text)
            .code_editor()
            .desired_width(f32::INFINITY)
            .desired_rows(18)
            .lock_focus(true)
            .layouter(&mut layouter)
            .show(ui);
        changed = output.response.changed();
        let galley = output.galley.clone();
        let origin = output.galley_pos;
        let painter = ui.painter().clone();
        // Line numbers, beside the first row of each line.
        let mut line = 0;
        let mut starts_line = true;
        for row in &galley.rows {
            let rect = row.rect().translate(origin.to_vec2());
            // Where the text of this row starts (lower in a tall row).
            let text_top = rect.top() + ((rect.height() - row_height) / 2.0).max(0.0);
            if starts_line {
                line += 1;
                painter.text(
                    egui::pos2(gutter.right() - 4.0, text_top),
                    egui::Align2::RIGHT_TOP,
                    line.to_string(),
                    font.clone(),
                    ui.visuals().weak_text_color(),
                );
                if let Some(list) = on_line.get(&(line - 1)) {
                    // A light band behind the line and its message.
                    let band = egui::Rect::from_x_y_ranges(
                        origin.x - 2.0..=output.response.rect.right() - 4.0,
                        text_top - 3.0..=text_top + row_height + message_room,
                    );
                    painter.rect_filled(band, 3.0, problem_color.gamma_multiply(0.12));
                    let under = text_top + row_height;
                    let width = row.row.size.x.max(8.0);
                    painter.line_segment(
                        [
                            egui::pos2(rect.left(), under),
                            egui::pos2(rect.left() + width, under),
                        ],
                        egui::Stroke::new(1.0, problem_color),
                    );
                    // The message, in the room under the line.
                    let top = text_top + row_height + 3.0;
                    let left = origin.x + 4.0;
                    let width = output.response.rect.right() - left - 8.0;
                    let area = egui::Rect::from_min_size(
                        egui::pos2(left, top),
                        egui::vec2(width.max(80.0), message_room - 4.0),
                    );
                    for &i in list.iter().take(1) {
                        let p = &problems[i];
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(
                            egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                        ));
                        child.label(
                            RichText::new(sentence(&p.message))
                                .small()
                                .color(problem_color),
                        );
                        if let Some(s) = &p.suggestion
                            && child.small_button(format!("Use `{s}`")).clicked()
                        {
                            picked = Some(i);
                        }
                    }
                }
            }
            starts_line = row.ends_with_newline;
        }
    });
    (picked, changed)
}

/// A message as a sentence: a capital letter first and a full stop last.
fn sentence(text: &str) -> String {
    let mut s = text.trim().to_string();
    if let Some(first) = s.get(..1) {
        s = first.to_uppercase() + &s[1..];
    }
    if !s.ends_with(['.', '!', '?']) {
        s.push('.');
    }
    s
}

/// The panel of sliders. Returns what the person did.
pub fn sliders(ui: &mut egui::Ui, editor: &Editor) -> Vec<Edit> {
    match editor.doc() {
        Some(doc) => controls::panel(ui, editor.schema(), doc),
        None => {
            ui.label(
                RichText::new("Fix the mistake in the text, and the sliders come back.").weak(),
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duck_echo_core::Code;

    #[test]
    fn a_suggestion_replaces_the_wrong_word_on_its_line() {
        let text = "version = 0\n[[layers]]\nsource = \"square\"\nvolme = 0.8\n";
        let issue = Issue::error(Code::UnknownField, "layers[0].volme", "unknown field")
            .suggest("volume")
            .at(4, 1);
        assert_eq!(
            with_suggestion(text, &issue).unwrap(),
            text.replace("volme", "volume")
        );
        let issue = Issue::error(Code::UnknownVariant, "layers[0].source", "unknown")
            .got("sqare")
            .suggest("square")
            .at(3, 10);
        let text = text.replace("\"square\"", "\"sqare\"");
        assert!(
            with_suggestion(&text, &issue)
                .unwrap()
                .contains("source = \"square\"")
        );
        // Not inside a longer word.
        assert_eq!(find_word("my_volme = volme", "volme"), Some(11));
    }

    #[test]
    fn a_message_reads_as_a_sentence() {
        assert_eq!(sentence("unknown field `volme`"), "Unknown field `volme`.");
        assert_eq!(sentence("Done."), "Done.");
    }
}
