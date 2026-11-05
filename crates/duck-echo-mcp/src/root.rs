//! The directory tools may read and write.

use duck_echo_tools::ToolError;
use std::path::{Component, Path, PathBuf};

/// The root directory, canonical.
pub struct Root {
    dir: PathBuf,
}

impl Root {
    /// A root at `dir`, which must exist.
    pub fn new(dir: &Path) -> Result<Self, String> {
        let dir = dir
            .canonicalize()
            .map_err(|e| format!("cannot use root {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    /// The root directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Resolve a path argument. `preset:NAME` passes through; anything
    /// else is taken relative to the root and must not leave it.
    pub fn resolve(&self, arg: &str) -> Result<PathBuf, ToolError> {
        if arg.starts_with(duck_echo_tools::PRESET_PREFIX) {
            return Ok(PathBuf::from(arg));
        }
        let joined = self.dir.join(arg);
        let mut clean = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => {
                    clean.pop();
                }
                Component::CurDir => {}
                other => clean.push(other),
            }
        }
        if clean.starts_with(&self.dir) {
            Ok(clean)
        } else {
            Err(ToolError::Io(format!(
                "path `{arg}` is outside the server root {}; use a path inside it",
                self.dir.display()
            )))
        }
    }

    /// A path as shown to the agent: relative to the root when inside it.
    pub fn show(&self, path: &Path) -> String {
        path.strip_prefix(&self.dir)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}
