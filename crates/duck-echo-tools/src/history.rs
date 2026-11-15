//! The Undo history of a project, kept on disk so that every program
//! working on the project shares it.
//!
//! A person in Duck Echo Studio and an agent through the MCP server are
//! two separate programs. Undo cannot live in one program's memory, so
//! each change becomes one small JSON file:
//!
//! ```text
//! <project>/.duck-echo/history/000041.json   a step that can be undone
//! <project>/.duck-echo/history/undone/        steps that were undone,
//!                                             waiting for Redo
//! ```
//!
//! A step names the files it changed, with their whole text before and
//! after. Undo writes the "before" text back; Redo writes the "after".
//!
//! The history is only read and written while the project's lock is held
//! (see [`crate::project`]), so two programs never change it at once.
//! Every step file is written with [`crate::save::write_atomic`].

use crate::save::write_atomic;
use crate::ToolError;
use duck_echo_core::Code;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Most steps kept. Older ones are deleted when a new one is added.
pub const MOST_STEPS: usize = 500;

/// Who made a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    /// A person: in the Studio, or typing a command.
    Person,
    /// An AI agent, through the MCP server.
    Agent,
    /// Nobody Duck Echo knows: the file changed while no Duck Echo
    /// program was changing it (a text editor, a copy, a git pull).
    Outside,
}

impl Who {
    /// "You", "AI assistant" or "Outside Duck Echo", for people.
    pub fn label(self) -> &'static str {
        match self {
            Who::Person => "You",
            Who::Agent => "AI assistant",
            Who::Outside => "Outside Duck Echo",
        }
    }
}

/// One file's text before and after a step. `None` means the file did
/// not exist (it was made, or deleted, by the step).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// The file, relative to the project folder, with `/` between parts.
    pub path: String,
    /// The text before the step (`None`: the file did not exist).
    pub before: Option<String>,
    /// The text after the step (`None`: the step deleted the file).
    pub after: Option<String>,
}

/// One step for Undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// The step's number, from its file name. Not stored inside the file.
    #[serde(skip)]
    pub number: u64,
    /// What it did, in a few words: "Set layers[0].volume to 0.6".
    pub title: String,
    /// Who made it.
    pub by: Who,
    /// When, in seconds since 1970 (UTC).
    pub time: u64,
    /// Edits made in one press of the mouse share a stroke number, and
    /// join one step (a slider drag is one step, not fifty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<u64>,
    /// The files it changed.
    pub files: Vec<FileChange>,
}

/// The history folder of a project.
pub struct History {
    dir: PathBuf,
}

/// Seconds since 1970, for a step's time. Not used for anything that is
/// rendered, so it cannot change a sound.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn io_error(what: &str, path: &Path, e: std::io::Error) -> ToolError {
    ToolError::failed(Code::Io, format!("cannot {what} {}: {e}", path.display()))
}

impl History {
    /// The history in `dir` (`<project>/.duck-echo/history`).
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn undone_dir(&self) -> PathBuf {
        self.dir.join("undone")
    }

    /// The step numbers in `dir`, oldest first.
    fn numbers_in(dir: &Path) -> Vec<u64> {
        let mut out: Vec<u64> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json")?.parse().ok()
            })
            .collect();
        out.sort_unstable();
        out
    }

    fn file_of(dir: &Path, number: u64) -> PathBuf {
        dir.join(format!("{number:06}.json"))
    }

    fn read_step(dir: &Path, number: u64) -> Result<Step, ToolError> {
        let path = Self::file_of(dir, number);
        let text = std::fs::read_to_string(&path).map_err(|e| io_error("read", &path, e))?;
        let mut step: Step = serde_json::from_str(&text).map_err(|e| {
            ToolError::failed(
                Code::Invalid,
                format!("the Undo history file {} is damaged: {e}", path.display()),
            )
        })?;
        step.number = number;
        Ok(step)
    }

    fn write_step(dir: &Path, step: &Step) -> Result<(), ToolError> {
        let text = serde_json::to_string_pretty(step).expect("a step serializes");
        write_atomic(&Self::file_of(dir, step.number), text.as_bytes())
    }

    /// Every step that can be undone, newest first.
    pub fn steps(&self) -> Result<Vec<Step>, ToolError> {
        Self::numbers_in(&self.dir)
            .into_iter()
            .rev()
            .map(|n| Self::read_step(&self.dir, n))
            .collect()
    }

    /// Steps that were undone and can be redone, the next to redo first.
    /// Undo takes the newest step first, so the one undone last has the
    /// lowest number: Redo goes up from there.
    pub fn undone(&self) -> Result<Vec<Step>, ToolError> {
        let dir = self.undone_dir();
        Self::numbers_in(&dir)
            .into_iter()
            .map(|n| Self::read_step(&dir, n))
            .collect()
    }

    /// The newest step, if there is one.
    pub fn newest(&self) -> Result<Option<Step>, ToolError> {
        match Self::numbers_in(&self.dir).last() {
            Some(n) => Self::read_step(&self.dir, *n).map(Some),
            None => Ok(None),
        }
    }

    /// The text of `path` after the newest step that changed it: what
    /// Duck Echo last wrote there. `None` when no step mentions the file.
    pub fn last_known(&self, path: &str) -> Result<Option<Option<String>>, ToolError> {
        for n in Self::numbers_in(&self.dir).into_iter().rev() {
            let step = Self::read_step(&self.dir, n)?;
            if let Some(change) = step.files.iter().find(|c| c.path == path) {
                return Ok(Some(change.after.clone()));
            }
        }
        Ok(None)
    }

    /// Add a step. A step whose stroke matches the newest step's (same
    /// person or agent) joins it instead: each file keeps its first
    /// "before" and takes the new "after". A step that changes nothing is
    /// not kept. Anything waiting for Redo is dropped, as in every editor.
    /// Returns the number of the step the change ended up in, if any.
    pub fn record(&self, mut step: Step) -> Result<Option<u64>, ToolError> {
        step.files.retain(|c| c.before != c.after);
        if step.files.is_empty() {
            return Ok(None);
        }
        let _ = std::fs::remove_dir_all(self.undone_dir());
        if let (Some(stroke), Some(mut last)) = (step.stroke, self.newest()?) {
            if last.stroke == Some(stroke) && last.by == step.by {
                for change in step.files {
                    match last.files.iter_mut().find(|c| c.path == change.path) {
                        Some(earlier) => earlier.after = change.after,
                        None => last.files.push(change),
                    }
                }
                last.files.retain(|c| c.before != c.after);
                last.title = step.title;
                last.time = step.time;
                Self::write_step(&self.dir, &last)?;
                return Ok(Some(last.number));
            }
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| io_error("create", &self.dir, e))?;
        let numbers = Self::numbers_in(&self.dir);
        step.number = numbers.last().map_or(1, |n| n + 1);
        Self::write_step(&self.dir, &step)?;
        // Keep the newest MOST_STEPS.
        let total = numbers.len() + 1;
        if total > MOST_STEPS {
            for old in &numbers[..total - MOST_STEPS] {
                let _ = std::fs::remove_file(Self::file_of(&self.dir, *old));
            }
        }
        Ok(Some(step.number))
    }

    /// Move the newest step to the "undone" folder. The caller has already
    /// put the files back.
    pub fn mark_undone(&self, step: &Step) -> Result<(), ToolError> {
        let dir = self.undone_dir();
        std::fs::create_dir_all(&dir).map_err(|e| io_error("create", &dir, e))?;
        crate::save::rename_into_place(
            &Self::file_of(&self.dir, step.number),
            &Self::file_of(&dir, step.number),
        )
    }

    /// Move a redone step back into the history.
    pub fn mark_redone(&self, step: &Step) -> Result<(), ToolError> {
        crate::save::rename_into_place(
            &Self::file_of(&self.undone_dir(), step.number),
            &Self::file_of(&self.dir, step.number),
        )
    }
}
