//! A person and an AI assistant on the same file.
//!
//! The Studio and the MCP server are two programs. They share the
//! project's files and its Undo history (`.duck-echo/history`), and
//! nothing else. So the Studio learns about an assistant's change the way
//! it learns about any change: the open file is different on disk (the
//! editor and the timeline look twice a second). Then [`Watch::changed`]
//! asks the history who made it:
//!
//! * a step by the AI assistant: the note "AI assistant: Set
//!   layers[0].envelope.decay to 0.5 in coin.toml" shows for three
//!   seconds, with Undo right there;
//! * no step at all (a text editor, a sync tool, a copy): the change is
//!   recorded first, as "Changed outside Duck Echo", so it is in the
//!   History and Undo never jumps over it, and the note says so.
//!
//! The person's own steps never make a note.

use std::time::{Duration, Instant};

use duck_echo_tools::history::Who;
use duck_echo_tools::project::{HistoryLine, Project};
use duck_echo_tools::root::Root;
use eframe::egui::{self, RichText};

use crate::theme;

/// How long the note about someone else's change stays.
pub const NOTE_TIME: Duration = Duration::from_secs(3);

/// A change someone else made to the open file.
#[derive(Debug, Clone, PartialEq)]
pub struct Visit {
    /// Who made it.
    pub by: Who,
    /// What the note says.
    pub text: String,
    /// When the Studio saw it.
    pub since: Instant,
}

impl Visit {
    /// True while the note shows.
    pub fn shows(&self, now: Instant) -> bool {
        now.duration_since(self.since) < NOTE_TIME
    }
}

/// What the Studio has seen of the history.
///
/// A step is known by its number, time and title together: after an
/// Undo the next new step takes the undone step's number again.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    seen: std::collections::BTreeSet<(u64, u64, String)>,
}

fn identity(step: &HistoryLine) -> (u64, u64, String) {
    (step.step, step.time, step.title.clone())
}

impl Watch {
    /// Start watching: every step so far counts as seen.
    pub fn new(root: &Root) -> Watch {
        let steps = Project::new(root)
            .history_list(SEEN)
            .map(|l| l.steps)
            .unwrap_or_default();
        Watch {
            seen: steps.iter().map(identity).collect(),
        }
    }

    /// The open file `rel` changed on disk. Record a change no Duck Echo
    /// program made, and return the note about the newest step someone
    /// else made to it since the last look (`None` for the person's own).
    pub fn changed(&mut self, root: &Root, rel: &str, now: Instant) -> Option<Visit> {
        let project = Project::new(root);
        let _ = project.notice_outside(rel);
        let steps = project.history_list(SEEN).ok()?.steps;
        let theirs: Vec<&HistoryLine> = steps
            .iter()
            .filter(|s| !self.seen.contains(&identity(s)))
            .filter(|s| s.by != Who::Person && s.files.iter().any(|f| f == rel))
            .collect();
        self.seen = steps.iter().map(identity).collect();
        let newest = theirs.first()?;
        let more = theirs.len() - 1;
        let what = match newest.by {
            Who::Outside => format!("{rel} was changed by another program (not Duck Echo)"),
            _ => newest.title.clone(),
        };
        let and_more = match more {
            0 => String::new(),
            1 => " (and 1 more change)".into(),
            n => format!(" (and {n} more changes)"),
        };
        Some(Visit {
            by: newest.by,
            text: format!("{}: {what}{and_more}", newest.who),
            since: now,
        })
    }
}

/// How many of the newest steps the Studio remembers having seen.
const SEEN: usize = 50;

/// The note, as a band across the screen. Returns true when its Undo
/// button was pressed.
pub fn note(ui: &mut egui::Ui, visit: &Visit) -> bool {
    let mut undo = false;
    egui::Frame::new()
        .fill(theme::ECHO_CYAN.gamma_multiply(0.25))
        .stroke(egui::Stroke::new(1.0, theme::ECHO_CYAN))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&visit.text).strong());
                if ui
                    .button("Undo")
                    .on_hover_text("Take this change back (Ctrl+Z)")
                    .clicked()
                {
                    undo = true;
                }
            });
        });
    undo
}

/// The History panel: every step, newest first, with who made it. The
/// steps of an AI assistant and of other programs stand out.
pub fn history(ui: &mut egui::Ui, steps: &[HistoryLine]) {
    ui.label(
        RichText::new("HISTORY")
            .small()
            .strong()
            .color(theme::NIGHT_INDIGO_TEXT),
    );
    ui.label(
        RichText::new("Newest first. Undo takes back the top one, whoever made it.")
            .small()
            .weak(),
    );
    egui::ScrollArea::vertical().show(ui, |ui| {
        if steps.is_empty() {
            ui.label(RichText::new("No changes yet.").weak());
        }
        let now = duck_echo_tools::history::now_seconds();
        for step in steps {
            ui.horizontal_wrapped(|ui| {
                let who = RichText::new(step.who).strong();
                ui.label(match step.by {
                    Who::Person => who,
                    Who::Agent => who.color(theme::ECHO_CYAN),
                    Who::Outside => who.color(theme::LOOK),
                });
                ui.label(RichText::new(ago(now, step.time)).small().weak());
            });
            ui.label(&step.title);
            ui.add_space(4.0);
        }
    });
}

/// "just now", "3 min ago", "2 h ago", "4 days ago".
pub fn ago(now: u64, then: u64) -> String {
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
    use super::*;
    use duck_echo_tools::commands;
    use serde_json::json;

    fn run(root: &Root, by: Who, name: &str, args: serde_json::Value) {
        let command = commands::parse(name, &args, "").unwrap();
        commands::run(&Project::new(root), command, by, None, "").unwrap();
    }

    #[test]
    fn only_someone_elses_change_makes_a_note() {
        let dir = std::env::temp_dir().join(format!("duck-echo-together-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = Root::new(&dir).unwrap();
        run(
            &root,
            Who::Person,
            "file_copy_from",
            json!({"from": "preset:coin", "to": "coin.toml"}),
        );
        let mut watch = Watch::new(&root);
        let now = Instant::now();
        // The person's own change: no note.
        run(
            &root,
            Who::Person,
            "sound_set",
            json!({"file": "coin.toml", "path": "layers[0].volume", "value": 0.5}),
        );
        assert_eq!(watch.changed(&root, "coin.toml", now), None);
        // The assistant's change: a note with its title.
        run(
            &root,
            Who::Agent,
            "sound_set",
            json!({"file": "coin.toml", "path": "layers[0].duty", "value": 0.3}),
        );
        let visit = watch.changed(&root, "coin.toml", now).unwrap();
        assert_eq!(visit.by, Who::Agent);
        assert!(
            visit.text.starts_with("AI assistant: Set layers[0].duty"),
            "{}",
            visit.text
        );
        assert!(visit.shows(now) && !visit.shows(now + NOTE_TIME));
        // Seen once.
        assert_eq!(watch.changed(&root, "coin.toml", now), None);
        // Two assistant changes in a row: the newest, and a count.
        for v in [0.4, 0.6] {
            run(
                &root,
                Who::Agent,
                "sound_set",
                json!({"file": "coin.toml", "path": "layers[0].duty", "value": v}),
            );
        }
        let visit = watch.changed(&root, "coin.toml", now).unwrap();
        assert!(
            visit.text.ends_with("(and 1 more change)"),
            "{}",
            visit.text
        );
        // The person undoes the newest; the next step takes its number
        // again, and is still noticed.
        run(&root, Who::Person, "edit_undo", json!({}));
        let _ = watch.changed(&root, "coin.toml", now);
        run(
            &root,
            Who::Agent,
            "sound_set",
            json!({"file": "coin.toml", "path": "layers[0].duty", "value": 0.7}),
        );
        assert!(watch.changed(&root, "coin.toml", now).is_some());
        // A text editor: recorded as a step of its own, and noted.
        let text = std::fs::read_to_string(dir.join("coin.toml")).unwrap();
        std::fs::write(dir.join("coin.toml"), text + "# by hand\n").unwrap();
        let visit = watch.changed(&root, "coin.toml", now).unwrap();
        assert_eq!(visit.by, Who::Outside);
        assert!(visit.text.contains("another program"), "{}", visit.text);
        std::fs::remove_dir_all(&dir).ok();
    }
}
