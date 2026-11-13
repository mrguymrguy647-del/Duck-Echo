//! The directory tools may read and write, and the only way tool arguments
//! become filesystem paths.
//!
//! Rules, for every path argument:
//!
//! * It is relative to the root. Absolute paths and `..` components are
//!   refused outright rather than normalized, so there is nothing clever to
//!   get wrong.
//! * Symbolic links are resolved: the deepest part of the path that exists
//!   is canonicalized and must still lie inside the (canonical) root. A
//!   link inside the root that points out, or a broken link, is refused.
//!   The parts after it do not exist yet and are created as plain
//!   directories when writing.
//! * `preset:NAME` is accepted only by [`Root::read`], only for a known
//!   preset whose name is letters, digits and underscores. [`Root::write`]
//!   refuses it, since presets are built in and cannot be written.
//! * Errors name the argument (`arguments.path`) and show paths as the
//!   agent wrote them, never the root's absolute location.
//!
//! Checking then writing leaves a window in which another local process
//! could swap a directory for a link; the server is a local tool for one
//! user's agent, and these rules stop the agent itself from leaving the
//! root.

use duck_echo_core::{load::did_you_mean, Code, Issue};
use duck_echo_tools::{presets, ToolError, PRESET_PREFIX};
use std::path::{Component, Path, PathBuf};

/// The root directory, canonical.
pub struct Root {
    dir: PathBuf,
}

fn refuse(key: &str, arg: &str, why: impl std::fmt::Display) -> ToolError {
    ToolError::issue(
        Issue::error(
            Code::PathRefused,
            key,
            format!("path `{arg}` is refused: {why}"),
        )
        .got(arg),
    )
}

/// True for names a preset can have.
fn is_preset_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Root {
    /// A root at `dir`, which must exist.
    pub fn new(dir: &Path) -> Result<Self, String> {
        let dir = dir
            .canonicalize()
            .map_err(|e| format!("cannot use root {}: {e}", dir.display()))?;
        if !dir.is_dir() {
            return Err(format!("root {} is not a directory", dir.display()));
        }
        Ok(Self { dir })
    }

    /// The root directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Resolve a path to read, given as argument `key` (such as
    /// `arguments.path`): `preset:NAME` for a known preset, or an existing
    /// file inside the root.
    pub fn read(&self, key: &str, arg: &str) -> Result<PathBuf, ToolError> {
        if let Some(name) = arg.strip_prefix(PRESET_PREFIX) {
            if !is_preset_name(name) {
                return Err(refuse(
                    key,
                    arg,
                    "`preset:` must be followed by a preset name of lower-case letters, digits and underscores, such as preset:coin",
                ));
            }
            presets::get(name).map_err(|e| e.at(key))?;
            return Ok(PathBuf::from(format!("{PRESET_PREFIX}{name}")));
        }
        let path = self.inside(key, arg)?;
        if std::fs::symlink_metadata(&path).is_err() {
            return Err(self.missing(key, arg, &path));
        }
        Ok(path)
    }

    /// Resolve a path to write, given as argument `key`: always a path
    /// inside the root.
    pub fn write(&self, key: &str, arg: &str) -> Result<PathBuf, ToolError> {
        if arg.starts_with(PRESET_PREFIX) {
            return Err(refuse(
                key,
                arg,
                "presets are built in and read-only; write to a file inside the root, such as out/coin.wav",
            ));
        }
        self.inside(key, arg)
    }

    /// Resolve a folder to list: the root itself when `arg` is empty or
    /// `.`, otherwise an existing path inside the root.
    pub fn folder(&self, key: &str, arg: Option<&str>) -> Result<PathBuf, ToolError> {
        match arg {
            None | Some("" | ".") => Ok(self.dir.clone()),
            Some(a) if a.starts_with(PRESET_PREFIX) => Err(refuse(
                key,
                a,
                "presets are built in, not files; list_presets lists them",
            )),
            Some(a) => {
                let path = self.inside(key, a)?;
                if std::fs::symlink_metadata(&path).is_err() {
                    return Err(self.missing(key, a, &path));
                }
                Ok(path)
            }
        }
    }

    /// A `not_found` error for `arg`, suggesting the closest name in its
    /// folder when there is one.
    fn missing(&self, key: &str, arg: &str, path: &Path) -> ToolError {
        let mut issue = Issue::error(
            Code::NotFound,
            key,
            format!("`{arg}` does not exist under the server root; list_files shows what does"),
        )
        .got(arg);
        let parent = path.parent().unwrap_or(&self.dir);
        if let (Some(name), Ok(read)) = (path.file_name(), std::fs::read_dir(parent)) {
            let siblings: Vec<String> = read
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect();
            if let Some(s) = did_you_mean(&name.to_string_lossy(), &siblings) {
                issue = issue.suggest(self.show(&parent.join(s)));
            }
        }
        ToolError::issue(issue)
    }

    fn inside(&self, key: &str, arg: &str) -> Result<PathBuf, ToolError> {
        if arg.is_empty() {
            return Err(refuse(key, arg, "it is empty"));
        }
        let rel = Path::new(arg);
        for c in rel.components() {
            match c {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => {
                    return Err(refuse(
                        key,
                        arg,
                        "`..` is not allowed; paths stay inside the server root",
                    ))
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(refuse(
                        key,
                        arg,
                        "absolute paths are not allowed; give a path relative to the server root",
                    ))
                }
            }
        }
        let joined = self.dir.join(rel);
        // The deepest part that exists in any form, a broken link included.
        let mut existing = joined.as_path();
        while std::fs::symlink_metadata(existing).is_err() {
            existing = match existing.parent() {
                Some(p) => p,
                None => return Err(refuse(key, arg, "no part of it exists")),
            };
        }
        let real = existing
            .canonicalize()
            .map_err(|_| refuse(key, arg, "it goes through a broken symbolic link"))?;
        if !real.starts_with(&self.dir) {
            return Err(refuse(
                key,
                arg,
                "it leads outside the server root through a symbolic link",
            ));
        }
        let rest = joined
            .strip_prefix(existing)
            .expect("existing is an ancestor of joined");
        // `join("")` would add a trailing separator, so only join a real rest.
        Ok(if rest.as_os_str().is_empty() {
            real
        } else {
            real.join(rest)
        })
    }

    /// A path as shown to the agent: relative to the root when inside it,
    /// with `/` between the parts on every platform (Windows too), so an
    /// agent sees the same paths wherever the server runs.
    pub fn show(&self, path: &Path) -> String {
        if path == self.dir {
            return ".".into();
        }
        match path.strip_prefix(&self.dir) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => path.display().to_string(),
        }
    }

    /// `text` with the root's absolute location replaced by paths relative
    /// to it, so messages never reveal where the server runs.
    ///
    /// On Windows the remaining `\` separators also become `/`, to match
    /// [`Root::show`].
    pub fn scrub(&self, text: &str) -> String {
        let dir = self.dir.display().to_string();
        let sep = std::path::MAIN_SEPARATOR;
        let out = text.replace(&format!("{dir}{sep}"), "").replace(&dir, ".");
        if sep == '\\' {
            out.replace('\\', "/")
        } else {
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_names() {
        assert!(is_preset_name("coin"));
        assert!(is_preset_name("big_explosion"));
        for bad in ["", "/../x", "../coin", "coin.wav", "Coin", "a/b", "a b"] {
            assert!(!is_preset_name(bad), "{bad}");
        }
    }

    #[test]
    fn scrub_hides_the_root() {
        let root = Root::new(&std::env::temp_dir()).unwrap();
        let dir = root.dir().display().to_string();
        let sep = std::path::MAIN_SEPARATOR;
        let msg = format!("cannot read {dir}{sep}sfx/a.wav in {dir}");
        assert_eq!(root.scrub(&msg), "cannot read sfx/a.wav in .");
    }
}
