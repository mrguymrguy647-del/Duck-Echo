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
//!
//! Checking then writing leaves a window in which another local process
//! could swap a directory for a link; the server is a local tool for one
//! user's agent, and these rules stop the agent itself from leaving the
//! root.

use duck_echo_tools::{presets, ToolError, PRESET_PREFIX};
use std::path::{Component, Path, PathBuf};

/// The root directory, canonical.
pub struct Root {
    dir: PathBuf,
}

fn refuse(arg: &str, why: impl std::fmt::Display) -> ToolError {
    ToolError::Io(format!("path `{arg}` is refused: {why}"))
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

    /// Resolve a path to read: `preset:NAME` for a known preset, or a path
    /// inside the root.
    pub fn read(&self, arg: &str) -> Result<PathBuf, ToolError> {
        if let Some(name) = arg.strip_prefix(PRESET_PREFIX) {
            if !is_preset_name(name) {
                return Err(refuse(
                    arg,
                    "`preset:` must be followed by a preset name of lower-case letters, digits and underscores, such as preset:coin",
                ));
            }
            presets::get(name)?;
            return Ok(PathBuf::from(format!("{PRESET_PREFIX}{name}")));
        }
        self.inside(arg)
    }

    /// Resolve a path to write: always a path inside the root.
    pub fn write(&self, arg: &str) -> Result<PathBuf, ToolError> {
        if arg.starts_with(PRESET_PREFIX) {
            return Err(refuse(
                arg,
                "presets are built in and read-only; write to a file inside the root, such as out/coin.wav",
            ));
        }
        self.inside(arg)
    }

    fn inside(&self, arg: &str) -> Result<PathBuf, ToolError> {
        if arg.is_empty() {
            return Err(refuse(arg, "it is empty"));
        }
        let rel = Path::new(arg);
        for c in rel.components() {
            match c {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir => {
                    return Err(refuse(
                        arg,
                        "`..` is not allowed; paths stay inside the server root",
                    ))
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(refuse(
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
                None => return Err(refuse(arg, "no part of it exists")),
            };
        }
        let real = existing
            .canonicalize()
            .map_err(|_| refuse(arg, "it goes through a broken symbolic link"))?;
        if !real.starts_with(&self.dir) {
            return Err(refuse(
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

    /// A path as shown to the agent: relative to the root when inside it.
    pub fn show(&self, path: &Path) -> String {
        path.strip_prefix(&self.dir)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
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
}
