//! A project: the folder of sounds and songs that a person (in Duck Echo
//! Studio) and an agent (through the MCP server) work on together, with
//! its shared Undo history.
//!
//! The one rule this file keeps: **no change is ever lost.**
//!
//! 1. A change reads the files it needs, works out their new text, and
//!    checks it. That can be slow (checking a song reads its instruments),
//!    so it happens without holding anything.
//! 2. Then it takes the project's lock (`.duck-echo/lock`, a file created
//!    with "fail if it exists"), reads the files again, and writes only if
//!    each one still holds exactly the text it read. The lock makes "is it
//!    still the same?" and "write" happen together, so two programs can
//!    never both see the old text and both write.
//! 3. If a file changed in between (the other program saved first), the
//!    change is worked out again on the new text, once, this time while
//!    holding the lock, so no other Duck Echo program can save in between.
//!    If the file changed even then (only a program that is not Duck Echo
//!    can do that), the change is refused with a plain message, and
//!    nothing is written. The other program's change is always kept.
//! 4. Files are written with [`crate::save::write_atomic`], so nobody ever
//!    reads half of one, and each change becomes one step in the history.
//!
//! A file whose text is not what the history last wrote was changed
//! outside Duck Echo. That change becomes a step of its own ("Changed
//! outside Duck Echo") before anything else happens to the file, so Undo
//! never jumps over it.

use crate::history::{now_seconds, FileChange, History, Step, Who};
use crate::root::{Root, OWN_FOLDER};
use crate::ToolError;
use duck_echo_core::{Code, Issue};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A lock left by a program that stopped (a crash, a power cut) is
/// removed once it is this old. A real change holds it for milliseconds.
const STALE_LOCK: Duration = Duration::from_secs(10);
/// How long a change waits for another program's lock before giving up.
const LOCK_WAIT: Duration = Duration::from_secs(20);

/// A project folder, through the path rules of its [`Root`].
pub struct Project<'r> {
    root: &'r Root,
}

/// The project's lock, held while it lives. Dropping it removes the file.
struct Lock {
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// What one change wants to write: each file's new text (`None` deletes
/// the file), and a title for Undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// What the change does, in a few words, for Undo.
    pub title: String,
    /// Each file to write (relative to the project) and its new text;
    /// `None` deletes the file.
    pub writes: Vec<(String, Option<String>)>,
}

/// What a change did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// Always true; a failure is an error instead.
    pub ok: bool,
    /// What was done, in a few words.
    pub done: String,
    /// The history step that holds the change. `None` when nothing
    /// changed (the file already held that text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u64>,
    /// The files it changed, relative to the project folder.
    pub files: Vec<String>,
}

/// One line of the history, for people and agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryLine {
    /// The step's number.
    pub step: u64,
    /// What it did.
    pub title: String,
    /// Who made it.
    pub by: Who,
    /// "You", "AI assistant" or "Outside Duck Echo".
    pub who: &'static str,
    /// Seconds since 1970 (UTC).
    pub time: u64,
    /// The files it changed.
    pub files: Vec<String>,
}

/// The history as a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryList {
    /// Steps that can be undone, newest first: `edit_undo` undoes the
    /// first.
    pub steps: Vec<HistoryLine>,
    /// Steps that were undone, the next for `edit_redo` first.
    pub undone: Vec<HistoryLine>,
}

impl From<&Step> for HistoryLine {
    fn from(step: &Step) -> Self {
        Self {
            step: step.number,
            title: step.title.clone(),
            by: step.by,
            who: step.by.label(),
            time: step.time,
            files: step.files.iter().map(|c| c.path.clone()).collect(),
        }
    }
}

fn read_text(path: &Path) -> Result<Option<String>, ToolError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ToolError::failed(
            Code::Io,
            format!("cannot read {}: {e}", path.display()),
        )),
    }
}

impl<'r> Project<'r> {
    /// The project in `root`'s folder.
    pub fn new(root: &'r Root) -> Self {
        Self { root }
    }

    /// The path rules for this project's files.
    pub fn root(&self) -> &'r Root {
        self.root
    }

    fn own_dir(&self) -> PathBuf {
        self.root.dir().join(OWN_FOLDER)
    }

    /// The project's Undo history.
    pub fn history(&self) -> History {
        History::new(self.own_dir().join("history"))
    }

    /// A file's full path from its name in the history. The name was
    /// checked by [`Root::write`] when the step was made, and is joined
    /// one plain part at a time.
    fn full(&self, rel: &str) -> PathBuf {
        let mut path = self.root.dir().to_path_buf();
        for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
            path.push(part);
        }
        path
    }

    /// The name a file is kept under in the history: relative to the
    /// project folder, with `/` between parts on every system.
    pub fn name_of(&self, path: &Path) -> String {
        self.root.show(path)
    }

    /// Take the project's lock, waiting while another program holds it.
    fn lock(&self) -> Result<Lock, ToolError> {
        let dir = self.own_dir();
        std::fs::create_dir_all(&dir).map_err(|e| {
            ToolError::failed(Code::Io, format!("cannot create {}: {e}", dir.display()))
        })?;
        let path = dir.join("lock");
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Lock { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let old = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if old {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > LOCK_WAIT {
                        return Err(ToolError::failed(
                            Code::Io,
                            "another Duck Echo program has been saving to this project for 20 seconds; try again",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                // On Windows, a lock being removed at this moment can say
                // "access denied" for an instant: try again.
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    if started.elapsed() > LOCK_WAIT {
                        return Err(ToolError::failed(
                            Code::Io,
                            format!("cannot take the project lock {}: {e}", path.display()),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => {
                    return Err(ToolError::failed(
                        Code::Io,
                        format!("cannot take the project lock {}: {e}", path.display()),
                    ))
                }
            }
        }
    }

    /// Note any change made outside Duck Echo to `rel`, whose text is now
    /// `now`, as a step of its own. Call with the lock held.
    fn note_outside(
        &self,
        history: &History,
        rel: &str,
        now: &Option<String>,
    ) -> Result<(), ToolError> {
        if let Some(known) = history.last_known(rel)? {
            if &known != now {
                history.record(Step {
                    number: 0,
                    title: format!("Changed outside Duck Echo: {rel}"),
                    by: Who::Outside,
                    time: now_seconds(),
                    stroke: None,
                    files: vec![FileChange {
                        path: rel.to_string(),
                        before: known,
                        after: now.clone(),
                    }],
                })?;
            }
        }
        Ok(())
    }

    /// Make one change, keeping the rule "no change is ever lost".
    ///
    /// `reads` are the files (relative to the project folder) the change
    /// depends on. `plan` gets their text (`None` for a file that does not
    /// exist) and says what to write; every file it writes must be in
    /// `reads`. `plan` may be called twice, if another program changed one
    /// of the files in between.
    pub fn apply(
        &self,
        reads: &[String],
        by: Who,
        stroke: Option<u64>,
        mut plan: impl FnMut(&[Option<String>]) -> Result<Plan, ToolError>,
    ) -> Result<Done, ToolError> {
        let read_all = || -> Result<Vec<Option<String>>, ToolError> {
            reads.iter().map(|rel| read_text(&self.full(rel))).collect()
        };
        let check_writes = |planned: &Plan| {
            for (rel, _) in &planned.writes {
                assert!(
                    reads.contains(rel),
                    "a plan wrote {rel}, which it did not read"
                );
            }
        };
        // First try: work the change out without holding the lock.
        let before = read_all()?;
        let mut planned = plan(&before)?;
        check_writes(&planned);
        let _lock = self.lock()?;
        let mut now = read_all()?;
        if now != before {
            // Another program saved first. Work the change out again on its
            // text, this time holding the lock, so no other Duck Echo
            // program can save in between.
            planned = plan(&now)?;
            check_writes(&planned);
            let again = read_all()?;
            if again != now {
                // Only a program that is not Duck Echo (a text editor, a
                // sync tool) can have saved now. Keep its change.
                let names = reads.join(", ");
                return Err(ToolError::issue(Issue::error(
                    Code::Invalid,
                    "",
                    format!(
                        "{names} kept changing while this change was being saved (another program is saving to it too). Nothing was changed; try again."
                    ),
                )));
            }
            now = again;
        }
        let history = self.history();
        for (rel, text) in reads.iter().zip(&now) {
            self.note_outside(&history, rel, text)?;
        }
        let mut files = Vec::new();
        for (rel, text) in &planned.writes {
            let path = self.full(rel);
            let old = &now[reads.iter().position(|r| r == rel).expect("checked")];
            if old == text {
                continue;
            }
            match text {
                Some(text) => crate::save::write_atomic(&path, text.as_bytes())?,
                None => std::fs::remove_file(&path).map_err(|e| {
                    ToolError::failed(Code::Io, format!("cannot delete {rel}: {e}"))
                })?,
            }
            files.push(FileChange {
                path: rel.clone(),
                before: old.clone(),
                after: text.clone(),
            });
        }
        let names = files.iter().map(|c| c.path.clone()).collect();
        let step = history.record(Step {
            number: 0,
            title: planned.title.clone(),
            by,
            time: now_seconds(),
            stroke,
            files,
        })?;
        Ok(Done {
            ok: true,
            done: planned.title,
            step,
            files: names,
        })
    }

    /// Undo the newest step: put each of its files back as it was before.
    ///
    /// If a file no longer holds the text the step wrote, it was changed
    /// outside Duck Echo since. That change is noted as a step of its own,
    /// and this Undo stops with a message saying so: nothing is
    /// overwritten. The next Undo takes the outside change back.
    pub fn undo(&self, by: Who) -> Result<Done, ToolError> {
        let _lock = self.lock()?;
        let history = self.history();
        let Some(step) = history.newest()? else {
            return Err(ToolError::failed(
                Code::Invalid,
                "there is nothing to undo in this project",
            ));
        };
        self.swap(&history, &step, Direction::Undo, by)?;
        history.mark_undone(&step)?;
        Ok(Done {
            ok: true,
            done: format!("Undid: {}", step.title),
            step: Some(step.number),
            files: step.files.iter().map(|c| c.path.clone()).collect(),
        })
    }

    /// Redo the step undone last.
    pub fn redo(&self, by: Who) -> Result<Done, ToolError> {
        let _lock = self.lock()?;
        let history = self.history();
        let Some(step) = history.undone()?.into_iter().next() else {
            return Err(ToolError::failed(
                Code::Invalid,
                "there is nothing to redo: nothing was undone since the last change",
            ));
        };
        self.swap(&history, &step, Direction::Redo, by)?;
        history.mark_redone(&step)?;
        Ok(Done {
            ok: true,
            done: format!("Redid: {}", step.title),
            step: Some(step.number),
            files: step.files.iter().map(|c| c.path.clone()).collect(),
        })
    }

    /// Write a step's files back (Undo) or forward (Redo), only if each
    /// file holds exactly what it should. Called with the lock held.
    fn swap(
        &self,
        history: &History,
        step: &Step,
        way: Direction,
        _by: Who,
    ) -> Result<(), ToolError> {
        let expected = |c: &FileChange| match way {
            Direction::Undo => c.after.clone(),
            Direction::Redo => c.before.clone(),
        };
        let wanted = |c: &FileChange| match way {
            Direction::Undo => c.before.clone(),
            Direction::Redo => c.after.clone(),
        };
        let mut changed = Vec::new();
        for change in &step.files {
            let now = read_text(&self.full(&change.path))?;
            if now != expected(change) {
                changed.push((change.path.clone(), now));
            }
        }
        if !changed.is_empty() {
            let names: Vec<&str> = changed.iter().map(|(p, _)| p.as_str()).collect();
            if way == Direction::Undo {
                for (rel, now) in &changed {
                    history.record(Step {
                        number: 0,
                        title: format!("Changed outside Duck Echo: {rel}"),
                        by: Who::Outside,
                        time: now_seconds(),
                        stroke: None,
                        files: vec![FileChange {
                            path: rel.clone(),
                            before: expected(
                                step.files.iter().find(|c| &c.path == rel).expect("listed"),
                            ),
                            after: now.clone(),
                        }],
                    })?;
                }
                return Err(ToolError::failed(
                    Code::Invalid,
                    format!(
                        "{} changed outside Duck Echo after the step \"{}\". Nothing was undone. That change is now the newest step: undo again to take it back.",
                        names.join(", "),
                        step.title
                    ),
                ));
            }
            return Err(ToolError::failed(
                Code::Invalid,
                format!(
                    "{} changed since the step \"{}\" was undone, so it cannot be redone without losing that change. Nothing was changed.",
                    names.join(", "),
                    step.title
                ),
            ));
        }
        for change in &step.files {
            let path = self.full(&change.path);
            match wanted(change) {
                Some(text) => crate::save::write_atomic(&path, text.as_bytes())?,
                None => {
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(|e| {
                            ToolError::failed(
                                Code::Io,
                                format!("cannot delete {}: {e}", change.path),
                            )
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The history, newest first, at most `limit` lines of each list.
    pub fn history_list(&self, limit: usize) -> Result<HistoryList, ToolError> {
        let history = self.history();
        Ok(HistoryList {
            steps: history
                .steps()?
                .iter()
                .take(limit)
                .map(HistoryLine::from)
                .collect(),
            undone: history
                .undone()?
                .iter()
                .take(limit)
                .map(HistoryLine::from)
                .collect(),
        })
    }

    /// Keep a copy of a file that is being deleted in the project's
    /// recycle bin, so it can be restored even after the step that deleted
    /// it is no longer in the history.
    pub fn recycle(&self, rel: &str, text: &str) -> Result<(), ToolError> {
        let dir = self.own_dir().join("recycle");
        let entry = Recycled {
            path: rel.to_string(),
            text: text.to_string(),
            time: now_seconds(),
        };
        let n = std::fs::read_dir(&dir).into_iter().flatten().count();
        let file = dir.join(format!("{:06}-{}.json", n + 1, crate::file_safe(rel)));
        crate::save::write_atomic(
            &file,
            serde_json::to_string_pretty(&entry)
                .expect("json")
                .as_bytes(),
        )
    }

    /// The newest copy of `rel` in the recycle bin.
    pub fn recycled(&self, rel: &str) -> Option<String> {
        let dir = self.own_dir().join("recycle");
        let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        names.sort();
        names.iter().rev().find_map(|p| {
            let entry: Recycled = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
            (entry.path == rel).then_some(entry.text)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Undo,
    Redo,
}

/// A file in the recycle bin.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Recycled {
    path: String,
    text: String,
    time: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("duck-echo-project-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set(text: &str) -> impl FnMut(&[Option<String>]) -> Result<Plan, ToolError> + '_ {
        move |_| {
            Ok(Plan {
                title: format!("Write {text}"),
                writes: vec![("a.txt".into(), Some(text.to_string()))],
            })
        }
    }

    #[test]
    fn a_change_is_recorded_undone_and_redone_exactly() {
        let dir = project_dir("basic");
        let root = Root::new(&dir).unwrap();
        let p = Project::new(&root);
        let reads = ["a.txt".to_string()];
        p.apply(&reads, Who::Person, None, set("one")).unwrap();
        let done = p.apply(&reads, Who::Agent, None, set("two")).unwrap();
        assert_eq!(done.step, Some(2));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "two");
        p.undo(Who::Person).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one");
        p.undo(Who::Person).unwrap();
        assert!(!dir.join("a.txt").exists());
        assert!(p.undo(Who::Person).is_err());
        p.redo(Who::Person).unwrap();
        p.redo(Who::Person).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "two");
        assert!(p.redo(Who::Person).is_err());
        let list = p.history_list(10).unwrap();
        assert_eq!(list.steps.len(), 2);
        assert_eq!(list.steps[0].who, "AI assistant");
        // A new change after an undo drops what was waiting for Redo.
        p.undo(Who::Person).unwrap();
        p.apply(&reads, Who::Person, None, set("three")).unwrap();
        assert!(p.redo(Who::Person).is_err());
        assert!(!dir.join(".duck-echo/lock").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_change_made_in_between_is_kept_then_retried_then_refused() {
        let dir = project_dir("retry");
        let root = Root::new(&dir).unwrap();
        let p = Project::new(&root);
        let reads = ["a.txt".to_string()];
        p.apply(&reads, Who::Person, None, set("start")).unwrap();

        // Another program saves once while this change is being worked
        // out: the change is made again on top of its text.
        let path = dir.join("a.txt");
        let mut calls = 0;
        let done = p
            .apply(&reads, Who::Person, None, |texts| {
                calls += 1;
                if calls == 1 {
                    std::fs::write(&path, "other program").unwrap();
                }
                Ok(Plan {
                    title: "Append".into(),
                    writes: vec![(
                        "a.txt".into(),
                        Some(format!("{} + mine", texts[0].as_deref().unwrap())),
                    )],
                })
            })
            .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "other program + mine"
        );
        // The outside change became its own step, before this one.
        let list = p.history_list(10).unwrap();
        assert_eq!(list.steps[1].by, Who::Outside);
        assert_eq!(done.step, Some(list.steps[0].step));

        // It keeps changing: refused, and the other program's text stays.
        let mut n = 0;
        let e = p
            .apply(&reads, Who::Person, None, |_| {
                n += 1;
                std::fs::write(&path, format!("busy {n}")).unwrap();
                Ok(Plan {
                    title: "Mine".into(),
                    writes: vec![("a.txt".into(), Some("mine".into()))],
                })
            })
            .unwrap_err()
            .into_report();
        assert!(e.errors[0].message.contains("kept changing"), "{e}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "busy 2");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn undo_never_overwrites_an_outside_change() {
        let dir = project_dir("outside");
        let root = Root::new(&dir).unwrap();
        let p = Project::new(&root);
        let reads = ["a.txt".to_string()];
        p.apply(&reads, Who::Person, None, set("one")).unwrap();
        p.apply(&reads, Who::Person, None, set("two")).unwrap();
        std::fs::write(dir.join("a.txt"), "edited by hand").unwrap();
        let e = p.undo(Who::Person).unwrap_err().into_report();
        assert!(
            e.errors[0].message.contains("changed outside Duck Echo"),
            "{e}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "edited by hand"
        );
        // The next undo takes the outside change back, then the one after.
        p.undo(Who::Person).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "two");
        p.undo(Who::Person).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stroke_is_one_step_and_the_history_is_trimmed() {
        let dir = project_dir("stroke");
        let root = Root::new(&dir).unwrap();
        let p = Project::new(&root);
        let reads = ["a.txt".to_string()];
        p.apply(&reads, Who::Person, None, set("start")).unwrap();
        for i in 0..20 {
            let text = format!("drag {i}");
            p.apply(&reads, Who::Person, Some(7), set(&text)).unwrap();
        }
        assert_eq!(p.history_list(100).unwrap().steps.len(), 2);
        p.undo(Who::Person).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "start");

        for i in 0..(crate::history::MOST_STEPS + 5) {
            let text = format!("n{i}");
            p.apply(&reads, Who::Agent, None, set(&text)).unwrap();
        }
        assert_eq!(
            p.history_list(10_000).unwrap().steps.len(),
            crate::history::MOST_STEPS
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stale_lock_is_removed() {
        let dir = project_dir("lock");
        let root = Root::new(&dir).unwrap();
        let p = Project::new(&root);
        std::fs::create_dir_all(dir.join(".duck-echo")).unwrap();
        let lock = dir.join(".duck-echo/lock");
        std::fs::write(&lock, "").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        p.apply(&["a.txt".to_string()], Who::Person, None, set("x"))
            .unwrap();
        assert!(!lock.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
