//! Listing the files under a folder, for agents that cannot run `ls` (the
//! MCP server's `list_files`).
//!
//! Entries are sorted by path. Hidden entries (names starting with `.`)
//! are left out. Symbolic links are listed as `link` and never followed,
//! so a listing cannot wander out of the folder it was asked about.

use crate::{io_code, ToolError};
use serde::Serialize;
use std::path::Path;

/// Most entries one listing returns; the rest are cut and `truncated` is
/// set.
pub const MAX_ENTRIES: usize = 500;

/// One file or folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    /// Path relative to the listing's base, with `/` separators.
    pub path: String,
    /// `folder`, `recipe` (.toml or .json), `audio` (.wav), `image`
    /// (.png), `link` (a symbolic link, not followed) or `other`.
    pub kind: &'static str,
    /// File size in bytes (files only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Length in seconds, for WAV files whose header reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// A folder listing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listing {
    /// The folder listed, relative to the base (`.` for the base itself).
    pub folder: String,
    /// Entries in path order.
    pub entries: Vec<Entry>,
    /// True when more than [`MAX_ENTRIES`] entries were found and the rest
    /// were left out; list a subfolder to see them.
    pub truncated: bool,
}

fn relative(path: &Path, base: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

fn entry(path: &Path, base: &Path) -> Result<Entry, ToolError> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| {
        ToolError::failed(io_code(&e), format!("cannot read {}: {e}", path.display()))
    })?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let kind = if meta.file_type().is_symlink() {
        "link"
    } else if meta.is_dir() {
        "folder"
    } else {
        match ext.as_deref() {
            Some("toml" | "json") => "recipe",
            Some("wav") => "audio",
            Some("png") => "image",
            _ => "other",
        }
    };
    let seconds = (kind == "audio")
        .then(|| duck_echo_audio::wav_info(path).ok())
        .flatten()
        .filter(|(rate, _, _)| *rate > 0)
        .map(|(rate, _, frames)| frames as f64 / rate as f64);
    Ok(Entry {
        path: relative(path, base),
        kind,
        bytes: meta.is_file().then_some(meta.len()),
        seconds,
    })
}

/// List `dir` (and its subfolders when `recursive`), with paths relative to
/// `base`. A file instead of a folder lists just that file.
pub fn list(dir: &Path, base: &Path, recursive: bool) -> Result<Listing, ToolError> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| {
        ToolError::failed(io_code(&e), format!("cannot list {}: {e}", dir.display()))
    })?;
    let mut entries = Vec::new();
    let mut truncated = false;
    if meta.is_dir() {
        walk(dir, base, recursive, &mut entries, &mut truncated)?;
    } else {
        entries.push(entry(dir, base)?);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Listing {
        folder: relative(dir, base),
        entries,
        truncated,
    })
}

fn walk(
    dir: &Path,
    base: &Path,
    recursive: bool,
    out: &mut Vec<Entry>,
    truncated: &mut bool,
) -> Result<(), ToolError> {
    let read = std::fs::read_dir(dir).map_err(|e| {
        ToolError::failed(io_code(&e), format!("cannot list {}: {e}", dir.display()))
    })?;
    let mut paths: Vec<_> = read
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if out.len() >= MAX_ENTRIES {
            *truncated = true;
            return Ok(());
        }
        let e = entry(&path, base)?;
        let descend = recursive && e.kind == "folder";
        out.push(e);
        if descend {
            walk(&path, base, recursive, out, truncated)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_sorted_with_kinds_and_skips_hidden() {
        let base = std::env::temp_dir().join("duck_echo_files_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sfx/deep")).unwrap();
        std::fs::write(base.join("sfx/coin.toml"), "version = 0\n").unwrap();
        std::fs::write(base.join("sfx/deep/notes.txt"), "x").unwrap();
        std::fs::write(base.join(".gitkeep"), "").unwrap();
        let buf = duck_echo_audio::AudioBuffer {
            sample_rate: 1000,
            channels: 1,
            samples: vec![0.0; 500],
        };
        duck_echo_audio::write_wav(
            &base.join("sfx/coin.wav"),
            &buf,
            duck_echo_audio::WavFormat::Pcm16,
        )
        .unwrap();

        let all = list(&base, &base, true).unwrap();
        let paths: Vec<(&str, &str)> = all
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind))
            .collect();
        assert_eq!(
            paths,
            [
                ("sfx", "folder"),
                ("sfx/coin.toml", "recipe"),
                ("sfx/coin.wav", "audio"),
                ("sfx/deep", "folder"),
                ("sfx/deep/notes.txt", "other"),
            ]
        );
        assert_eq!(all.folder, ".");
        assert_eq!(all.entries[2].seconds, Some(0.5));
        assert_eq!(all.entries[1].bytes, Some(12));
        assert!(!all.truncated);

        let flat = list(&base.join("sfx"), &base, false).unwrap();
        assert_eq!(flat.folder, "sfx");
        assert_eq!(flat.entries.len(), 3);
        let one = list(&base.join("sfx/coin.toml"), &base, true).unwrap();
        assert_eq!(one.entries[0].path, "sfx/coin.toml");
        match list(&base.join("nope"), &base, true) {
            Err(ToolError::Failed(i)) => assert_eq!(i.code, duck_echo_core::Code::NotFound),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
