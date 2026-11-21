//! What the Ear's folder keeps besides the Ear itself: the person's
//! choice of device, the kept answers, and a note that an AI assistant
//! wanted to listen while the Ear was missing.
//!
//! Each is a small file in the Ear's folder, so the command line, the MCP
//! server and the Studio all read the same one:
//!
//! * `device.txt`: `cpu` when the person chose "CPU only". With no file,
//!   the Ear picks the device itself (a graphics card for a describe
//!   model, when there is one). The score model always runs on the CPU.
//! * `answers/`: every answer the Ear gave, so the same question about the
//!   same sound is answered at once (`ear.rs`). Clearing it only makes
//!   the next questions slower.
//! * `wanted.json`: written when an assistant asked to listen and the
//!   installed Ear was not there. The Studio shows a note once, offering
//!   to install it. Nothing is ever installed because of it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the Ear's describe model may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// The Ear picks: a graphics card when there is one, else the CPU.
    Automatic,
    /// Never a graphics card.
    CpuOnly,
}

impl Device {
    /// The choice as a word: `automatic` or `cpu_only`.
    pub fn word(self) -> &'static str {
        match self {
            Device::Automatic => "automatic",
            Device::CpuOnly => "cpu_only",
        }
    }
}

fn device_file(home: &Path) -> PathBuf {
    home.join("device.txt")
}

/// The person's choice of device for the Ear in `home`.
pub fn device(home: &Path) -> Device {
    match std::fs::read_to_string(device_file(home)) {
        Ok(text) if text.trim() == "cpu" => Device::CpuOnly,
        _ => Device::Automatic,
    }
}

/// Keep the person's choice of device. The Ear reads it when it next
/// starts.
pub fn choose_device(home: &Path, choice: Device) -> std::io::Result<()> {
    match choice {
        Device::Automatic => match std::fs::remove_file(device_file(home)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Device::CpuOnly => {
            std::fs::create_dir_all(home)?;
            std::fs::write(device_file(home), "cpu\n")
        }
    }
}

/// The answers the Ear in `home` keeps: how many, and how many bytes.
pub fn answers(home: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut bytes = 0;
    let Ok(dirs) = std::fs::read_dir(home.join("answers")) else {
        return (0, 0);
    };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            if file.path().extension().is_some_and(|e| e == "json") {
                count += 1;
                bytes += file.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (count, bytes)
}

/// Forget every kept answer. Returns how many there were. The next
/// question about each sound asks the Ear again.
pub fn clear_answers(home: &Path) -> std::io::Result<usize> {
    let (count, _) = answers(home);
    match std::fs::remove_dir_all(home.join("answers")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(count),
    }
}

/// An AI assistant asked to listen while the Ear was not installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wanted {
    /// The tool it used the last time: `listen`, `analyze` ...
    pub tool: String,
    /// How many times it asked.
    pub times: u64,
    /// When it first asked, in seconds since 1970.
    pub first: u64,
    /// When it last asked, in seconds since 1970.
    pub last: u64,
}

fn wanted_file(home: &Path) -> PathBuf {
    home.join("wanted.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Note that an assistant asked to listen with `tool` and the Ear in
/// `home` was not installed. Best effort: a note that cannot be written
/// changes nothing for the assistant.
pub fn note_wanted(home: &Path, tool: &str) {
    let now = now();
    let note = match wanted(home) {
        Some(old) => Wanted {
            tool: tool.to_string(),
            times: old.times + 1,
            first: old.first,
            last: now,
        },
        None => Wanted {
            tool: tool.to_string(),
            times: 1,
            first: now,
            last: now,
        },
    };
    let _ = std::fs::create_dir_all(home);
    let text = serde_json::to_string_pretty(&note).expect("serializes") + "\n";
    let part = wanted_file(home).with_extension(format!("part{}", std::process::id()));
    if std::fs::write(&part, text).is_ok() {
        let _ = std::fs::rename(&part, wanted_file(home));
    }
}

/// The note that an assistant wanted to listen, if there is one.
pub fn wanted(home: &Path) -> Option<Wanted> {
    let text = std::fs::read_to_string(wanted_file(home)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forget the note (once the Ear is installed).
pub fn forget_wanted(home: &Path) {
    let _ = std::fs::remove_file(wanted_file(home));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("duck-echo-ear-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn the_device_choice_is_kept_in_the_ears_folder() {
        let home = home("device");
        assert_eq!(device(&home), Device::Automatic);
        choose_device(&home, Device::CpuOnly).unwrap();
        assert_eq!(device(&home), Device::CpuOnly);
        assert_eq!(
            std::fs::read_to_string(home.join("device.txt")).unwrap(),
            "cpu\n"
        );
        choose_device(&home, Device::Automatic).unwrap();
        assert_eq!(device(&home), Device::Automatic);
        assert!(!home.join("device.txt").exists());
        // Choosing Automatic twice is fine.
        choose_device(&home, Device::Automatic).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn clearing_answers_counts_them_and_leaves_the_rest() {
        let home = home("answers");
        assert_eq!(clear_answers(&home).unwrap(), 0);
        for (dir, name) in [
            ("ab", "ab12.json"),
            ("ab", "ab34.json"),
            ("cd", "cd56.json"),
        ] {
            std::fs::create_dir_all(home.join("answers").join(dir)).unwrap();
            std::fs::write(home.join("answers").join(dir).join(name), "{}").unwrap();
        }
        std::fs::write(home.join("ear.json"), "{}").unwrap();
        assert_eq!(answers(&home), (3, 6));
        assert_eq!(clear_answers(&home).unwrap(), 3);
        assert_eq!(answers(&home), (0, 0));
        assert!(home.join("ear.json").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn each_wish_to_listen_is_counted() {
        let home = home("wanted");
        assert!(wanted(&home).is_none());
        note_wanted(&home, "listen");
        note_wanted(&home, "analyze");
        let note = wanted(&home).unwrap();
        assert_eq!(note.times, 2);
        assert_eq!(note.tool, "analyze");
        assert!(note.first <= note.last && note.first > 0);
        forget_wanted(&home);
        assert!(wanted(&home).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }
}
