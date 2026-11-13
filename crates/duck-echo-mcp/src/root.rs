//! The directory tools may read and write, and the only way tool arguments
//! become filesystem paths.
//!
//! Rules, for every path argument:
//!
//! * It is relative to the root. Absolute paths and `..` components are
//!   refused outright rather than normalized, so there is nothing clever to
//!   get wrong.
//! * No part of a path may contain `:`. On Windows `C:` in the middle of
//!   a path names a drive (and `name:stream` a hidden data stream), and
//!   joining such a path onto a folder *replaces* the folder. The rule is
//!   the same on every platform, so a path means the same thing wherever
//!   the server runs. The resolved path is also built one plain name at a
//!   time, never by joining a multi-part tail, so nothing that is parsed
//!   again later can turn into an absolute path.
//! * Symbolic links are resolved: the deepest part of the path that exists
//!   is canonicalized and must still lie inside the (canonical) root. A
//!   link inside the root that points out, or a broken link, is refused.
//!   The parts after it do not exist yet and are created as plain
//!   directories when writing.
//! * `preset:NAME` is accepted only by [`Root::read`], only for a known
//!   preset whose name is letters, digits and underscores. [`Root::write`]
//!   refuses it, since presets are built in and cannot be written.
//! * `instrument:NAME` (a built-in instrument) follows exactly the same
//!   rules: read-only, a known name only, never treated as a path.
//! * Errors name the argument (`arguments.path`) and show paths as the
//!   agent wrote them, never the root's absolute location.
//!
//! Checking then writing leaves a window in which another local process
//! could swap a directory for a link; the server is a local tool for one
//! user's agent, and these rules stop the agent itself from leaving the
//! root.

use duck_echo_core::{load::did_you_mean, Code, Issue};
use duck_echo_tools::instruments::{self, INSTRUMENT_PREFIX};
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
        if let Some(name) = arg.strip_prefix(INSTRUMENT_PREFIX) {
            if !is_preset_name(name) {
                return Err(refuse(
                    key,
                    arg,
                    "`instrument:` must be followed by an instrument name of lower-case letters, digits and underscores, such as instrument:piano",
                ));
            }
            instruments::get(name).map_err(|e| e.at(key))?;
            return Ok(PathBuf::from(format!("{INSTRUMENT_PREFIX}{name}")));
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
        if arg.starts_with(INSTRUMENT_PREFIX) {
            return Err(refuse(
                key,
                arg,
                "instruments are built in and read-only; write to a file inside the root, such as out/piano.wav",
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
            Some(a) if a.starts_with(INSTRUMENT_PREFIX) => Err(refuse(
                key,
                a,
                "instruments are built in, not files; list_instruments lists them",
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

    /// Check that `arg` is a plain relative path: not empty, no absolute
    /// path, no `..`, and no `:` in any part. This looks only at the text;
    /// [`Root::read`] and [`Root::write`] also resolve links.
    pub fn check_relative(&self, key: &str, arg: &str) -> Result<(), ToolError> {
        if arg.is_empty() {
            return Err(refuse(key, arg, "it is empty"));
        }
        // Checked on the text, before any parsing: on Windows `X:` is a
        // drive, wherever it appears once paths are joined.
        if arg.contains(':') {
            return Err(refuse(
                key,
                arg,
                "`:` is not allowed in a path (drive letters and absolute paths are not allowed; give a path relative to the server root)",
            ));
        }
        for c in Path::new(arg).components() {
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
        Ok(())
    }

    fn inside(&self, key: &str, arg: &str) -> Result<PathBuf, ToolError> {
        self.check_relative(key, arg)?;
        // Build the path one plain name at a time. `join` with a whole
        // path would replace everything before it if that path turned out
        // to be absolute; pushing single checked names cannot.
        let mut joined = self.dir.clone();
        for c in Path::new(arg).components() {
            if let Component::Normal(name) = c {
                joined.push(name);
            }
        }
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
        // The parts after it do not exist yet. Add them back one name at
        // a time, for the same reason as above; anything that is not a
        // plain name is refused.
        let mut out = real;
        for c in rest.components() {
            match c {
                Component::Normal(name) => out.push(name),
                _ => {
                    return Err(refuse(
                        key,
                        arg,
                        "absolute paths are not allowed; give a path relative to the server root",
                    ))
                }
            }
        }
        // Belt and braces: whatever happened above, the result must be
        // inside the root.
        if !out.starts_with(&self.dir) {
            return Err(refuse(key, arg, "it leads outside the server root"));
        }
        Ok(out)
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
    fn drive_letters_and_colons_are_refused_everywhere() {
        let root = Root::new(&std::env::temp_dir()).unwrap();
        for bad in [
            "C:\\Windows\\x.wav",
            "sub/C:\\Windows\\x.wav",
            "sub/D:/x.toml",
            "a:b.toml",
            "file.txt:stream",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "",
        ] {
            assert!(root.read("k", bad).is_err(), "read {bad}");
            assert!(root.write("k", bad).is_err(), "write {bad}");
            assert!(root.check_relative("k", bad).is_err(), "check {bad}");
        }
        // Whatever is accepted resolves inside the root.
        for good in ["a.toml", "sub/a.wav", "./sub/deep/a.wav"] {
            let path = root.write("k", good).unwrap();
            assert!(path.starts_with(root.dir()), "{good} -> {}", path.display());
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
