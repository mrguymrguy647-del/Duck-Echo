//! Writing files so that no program ever reads half of one.
//!
//! A person in Duck Echo Studio and an agent through the MCP server can
//! work on the same folder at the same moment. If one program wrote a file
//! byte by byte while the other read it, the reader could see half a
//! recipe. So every file Duck Echo writes goes through [`write_atomic`]:
//! the bytes go to a temporary file in the same folder first, and that
//! file is then renamed onto the real name in one step. A reader sees
//! either the old file or the new one, never a mix.

use crate::ToolError;
use duck_echo_core::Code;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Makes each temporary name in this program different. (Only used for
/// file names, never for sound, so renders stay deterministic.)
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// How many times a rename is tried when Windows says the file is busy
/// (another program has it open in a way that blocks renaming), and how
/// long to wait between tries.
const RENAME_TRIES: u32 = 100;
const RENAME_WAIT: std::time::Duration = std::time::Duration::from_millis(10);

/// A temporary name next to `path`: hidden (it starts with a dot, so file
/// listings skip it) and unique to this program and this write.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

fn io_error(what: &str, path: &Path, e: std::io::Error) -> ToolError {
    ToolError::failed(Code::Io, format!("cannot {what} {}: {e}", path.display()))
}

/// Write `bytes` to `path` in one step, making its folder if needed.
///
/// The bytes go to a temporary file beside `path`, which is flushed to the
/// disk and then renamed onto `path`. If anything fails, the temporary
/// file is removed and `path` is left as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| io_error("create", dir, e))?;
    }
    let temp = temp_path(path);
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(io_error("write", path, e));
    }
    rename_into_place(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Rename `from` onto `to`, replacing `to`. On Windows a rename fails
/// while another program has `to` open without allowing it (a text editor,
/// a virus scanner), so a "permission denied" is tried again for about a
/// second before giving up.
pub fn rename_into_place(from: &Path, to: &Path) -> Result<(), ToolError> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < RENAME_TRIES => {
                tries += 1;
                std::thread::sleep(RENAME_WAIT);
            }
            Err(e) => return Err(io_error("replace", to, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("duck-echo-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_replaces_and_leaves_no_temporary_file() {
        let dir = folder("basic");
        let path = dir.join("deep/sound.toml");
        write_atomic(&path, b"version = 0\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version = 0\n");
        write_atomic(&path, b"version = 0\nname = \"x\"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "version = 0\nname = \"x\"\n"
        );
        let names: Vec<String> = std::fs::read_dir(dir.join("deep"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["sound.toml"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_reader_never_sees_half_a_file() {
        let dir = folder("readers");
        let path = dir.join("big.txt");
        let a = vec![b'a'; 200_000];
        let b = vec![b'b'; 300_000];
        write_atomic(&path, &a).unwrap();
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..2000 {
                let text = std::fs::read(&reader_path).unwrap();
                let whole_a = text.len() == 200_000 && text.iter().all(|c| *c == b'a');
                let whole_b = text.len() == 300_000 && text.iter().all(|c| *c == b'b');
                assert!(whole_a || whole_b, "read {} mixed bytes", text.len());
            }
        });
        for i in 0..200 {
            write_atomic(&path, if i % 2 == 0 { &b } else { &a }).unwrap();
        }
        reader.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
