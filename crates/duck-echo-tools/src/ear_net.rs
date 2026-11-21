//! Getting files for the Duck Ear: downloading with `curl`, checking each
//! file's SHA-256, and unpacking zip files.
//!
//! Downloads use the `curl` program that comes with Windows 10 and 11 and
//! with Linux, as the Studio's update check does, so Duck Echo needs no
//! download library. A file is written as `<name>.part`, carried on from
//! where it stopped if a `.part` is already there, and renamed only when
//! its SHA-256 matches. One that does not match is deleted and fetched
//! once more; then the install stops and names the file.
//!
//! Only a person starts a download (`duck-echo ear install`, or a button
//! in the Studio): never an AI assistant, and never Duck Echo by itself.

use crate::asset::sha256_of_file;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A switch a person can throw to stop a long job (a "Cancel" button).
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A new switch, not thrown.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Stop the job at its next safe point.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Was the job told to stop?
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Why a file could not be fetched or unpacked, in a form a person can act
/// on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// There is no `curl` program.
    NoCurl(String),
    /// The other side could not be reached: no internet, or the site is down.
    NoInternet {
        /// The address asked for.
        url: String,
        /// What `curl` said.
        why: String,
    },
    /// The site answered that the file is not there.
    NotFound {
        /// The address asked for.
        url: String,
    },
    /// The disk is full, or cannot be written.
    NoRoom {
        /// The file being written.
        file: PathBuf,
        /// What was needed, if known.
        needed: Option<u64>,
        /// What is free, if known.
        free: Option<u64>,
    },
    /// The file arrived, but is not the file that was listed.
    WrongSha256 {
        /// The file's name.
        file: String,
        /// The SHA-256 it should have.
        expected: String,
        /// The SHA-256 it has.
        got: String,
    },
    /// The person stopped it.
    Cancelled,
    /// Anything else, in words.
    Failed(String),
}

impl NetError {
    /// A short code for programs: `no_curl`, `no_internet`, `not_found`,
    /// `no_room`, `wrong_sha256`, `cancelled`, `failed`.
    pub fn code(&self) -> &'static str {
        match self {
            NetError::NoCurl(_) => "no_curl",
            NetError::NoInternet { .. } => "no_internet",
            NetError::NotFound { .. } => "not_found",
            NetError::NoRoom { .. } => "no_room",
            NetError::WrongSha256 { .. } => "wrong_sha256",
            NetError::Cancelled => "cancelled",
            NetError::Failed(_) => "failed",
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::NoCurl(why) => write!(
                f,
                "Duck Echo downloads with the `curl` program, and could not start it ({why}). Windows 10 and 11 have it; on Linux, install the `curl` package."
            ),
            NetError::NoInternet { url, why } => write!(
                f,
                "Could not reach {} ({why}). Check the internet connection and try again: what was already downloaded is kept.",
                host_of(url)
            ),
            NetError::NotFound { url } => {
                write!(f, "The file is not there: {url} (the site answered \"not found\").")
            }
            NetError::NoRoom { file, needed, free } => {
                write!(f, "There is not enough room to write {}", file.display())?;
                if let (Some(needed), Some(free)) = (needed, free) {
                    write!(
                        f,
                        ": it needs {}, and {} is free on that disk",
                        megabytes(*needed),
                        megabytes(*free)
                    )?;
                }
                write!(f, ". Free some space and try again.")
            }
            NetError::WrongSha256 { file, expected, got } => write!(
                f,
                "{file} arrived twice with the wrong SHA-256 (it is {got}, the list says {expected}), so it was deleted and nothing was installed from it. Try again later; if it happens again, an antivirus or the network is changing the file."
            ),
            NetError::Cancelled => write!(f, "Stopped. What was already downloaded is kept."),
            NetError::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// A size in words: "784 MB", "19.9 GB".
pub fn megabytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", (bytes as f64 / 1e6).max(1.0))
    }
}

fn host_of(url: &str) -> &str {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest)
}

/// The `curl` program: Windows' own (`System32\curl.exe`), or the one on
/// the PATH.
pub fn curl_program() -> PathBuf {
    if cfg!(windows) {
        let own = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("curl.exe"))
            .filter(|curl| curl.is_file());
        if let Some(own) = own {
            return own;
        }
    }
    PathBuf::from("curl")
}

fn curl() -> Command {
    let mut command = Command::new(curl_program());
    command
        .arg("--fail")
        .arg("--location")
        .arg("--silent")
        .arg("--show-error")
        .args(["--connect-timeout", "30"])
        .args(["--user-agent", "duck-echo"])
        .stdin(Stdio::null());
    command
}

/// What a finished `curl` means.
fn curl_error(code: Option<i32>, stderr: &str, url: &str, file: &Path) -> NetError {
    let why = stderr
        .trim()
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    match code {
        // Could not resolve, could not connect, timed out, TLS would not start.
        Some(5 | 6 | 7 | 28 | 35 | 52 | 56) => NetError::NoInternet {
            url: url.to_string(),
            why,
        },
        Some(22) if why.contains("404") => NetError::NotFound {
            url: url.to_string(),
        },
        Some(23) => NetError::NoRoom {
            file: file.to_path_buf(),
            needed: None,
            free: free_bytes(file.parent().unwrap_or(Path::new("."))),
        },
        _ => NetError::Failed(format!(
            "The download of {url} failed: {}",
            if why.is_empty() {
                "curl did not say why"
            } else {
                &why
            }
        )),
    }
}

/// The text at `url`: a small file, such as a list of SHA-256s or a list
/// of releases.
pub fn fetch_text(url: &str) -> Result<String, NetError> {
    let out = curl()
        .args(["--max-time", "120"])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| NetError::NoCurl(e.to_string()))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(curl_error(out.status.code(), &stderr, url, Path::new("")));
    }
    String::from_utf8(out.stdout).map_err(|_| NetError::Failed(format!("{url} is not text")))
}

/// What a downloaded file must be.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expected {
    /// Its size in bytes, when the list says.
    pub bytes: Option<u64>,
    /// Its SHA-256 (64 hex digits), when the list says.
    pub sha256: Option<String>,
}

impl Expected {
    /// Is the file at `path` the expected one? `Err` holds its SHA-256
    /// when that is what differs.
    fn matches(&self, path: &Path) -> Result<bool, String> {
        let Ok(meta) = std::fs::metadata(path) else {
            return Ok(false);
        };
        if self.bytes.is_some_and(|bytes| bytes != meta.len()) {
            return Ok(false);
        }
        match &self.sha256 {
            None => Ok(self.bytes.is_some()),
            Some(expected) => match sha256_of_file(path) {
                Ok(got) if got.eq_ignore_ascii_case(expected) => Ok(true),
                Ok(got) => Err(got),
                Err(_) => Ok(false),
            },
        }
    }
}

/// Download `url` to `to`. A file already there that matches `expected`
/// is kept as it is. `progress` hears how many bytes have arrived (and of
/// how many, when known) a few times a second; `cancel` stops it, keeping
/// the `.part` file to carry on from.
pub fn download(
    url: &str,
    to: &Path,
    expected: &Expected,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &Cancel,
) -> Result<(), NetError> {
    if cancel.is_cancelled() {
        return Err(NetError::Cancelled);
    }
    if matches!(expected.matches(to), Ok(true)) {
        progress(expected.bytes.unwrap_or(0), expected.bytes);
        return Ok(());
    }
    let folder = to.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder).map_err(|e| {
        NetError::Failed(format!("cannot make the folder {}: {e}", folder.display()))
    })?;
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let part = folder.join(name);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if let (Some(needed), Some(free)) = (expected.bytes, free_bytes(folder)) {
        let still = needed.saturating_sub(have);
        // A little spare, so the disk is not filled to the last byte.
        if still.saturating_add(50_000_000) > free {
            return Err(NetError::NoRoom {
                file: to.to_path_buf(),
                needed: Some(still),
                free: Some(free),
            });
        }
    }
    let file_name = to
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    for attempt in 0..2 {
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        // A `.part` that is already whole needs no more of the file.
        if expected.bytes.is_none_or(|bytes| have < bytes) {
            fetch_part(url, &part, expected.bytes, progress, cancel)?;
        }
        match expected.matches(&part) {
            Ok(true) => {
                progress(expected.bytes.unwrap_or(0), expected.bytes);
                let _ = std::fs::remove_file(to);
                return std::fs::rename(&part, to).map_err(|e| {
                    NetError::Failed(format!("cannot put {} in place: {e}", to.display()))
                });
            }
            Ok(false) if expected.sha256.is_none() && expected.bytes.is_none() => {
                // Nothing to check it against: take it as it came.
                let _ = std::fs::remove_file(to);
                return std::fs::rename(&part, to).map_err(|e| {
                    NetError::Failed(format!("cannot put {} in place: {e}", to.display()))
                });
            }
            wrong => {
                let _ = std::fs::remove_file(&part);
                if attempt == 1 {
                    return Err(NetError::WrongSha256 {
                        file: file_name,
                        expected: expected.sha256.clone().unwrap_or_else(|| {
                            format!("a file of {} bytes", expected.bytes.unwrap_or(0))
                        }),
                        got: wrong.err().unwrap_or_else(|| "another size".to_string()),
                    });
                }
            }
        }
    }
    unreachable!("the second attempt returns")
}

/// One run of `curl` into `part`, carrying on from what is there.
fn fetch_part(
    url: &str,
    part: &Path,
    bytes: Option<u64>,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &Cancel,
) -> Result<(), NetError> {
    for carry_on in [true, false] {
        let mut command = curl();
        if carry_on {
            command.args(["--continue-at", "-"]);
        } else {
            let _ = std::fs::remove_file(part);
        }
        let mut child = command
            .args(["--retry", "2"])
            .arg("--output")
            .arg(part)
            .arg(url)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| NetError::NoCurl(e.to_string()))?;
        let status = loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(NetError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    progress(std::fs::metadata(part).map(|m| m.len()).unwrap_or(0), bytes);
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(NetError::Failed(format!("curl was lost: {e}"))),
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_string(&mut stderr);
        }
        if status.success() {
            return Ok(());
        }
        // The site cannot carry on from the middle (curl's 33), or says the
        // part is already past the end (HTTP 416): fetch it whole, once.
        let from_the_start = status.code() == Some(33) || stderr.contains("416");
        if !(carry_on && from_the_start) {
            return Err(curl_error(status.code(), &stderr, url, part));
        }
    }
    unreachable!("the second run returns")
}

/// How many bytes are free on the disk that holds `folder`, when the
/// system can say.
pub fn free_bytes(folder: &Path) -> Option<u64> {
    let mut existing = folder;
    while !existing.exists() {
        existing = existing.parent()?;
    }
    if cfg!(windows) {
        let script = format!(
            "(Get-Item -LiteralPath '{}').PSDrive.Free",
            existing.display().to_string().replace('\'', "''")
        );
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        let out = Command::new("df")
            .arg("-Pk")
            .arg(existing)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().nth(1)?;
        let free_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
        Some(free_kb * 1024)
    }
}

// ---------------------------------------------------------------------
// Zip files.
// ---------------------------------------------------------------------

fn le16(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn le32(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// One file inside a zip file.
struct Entry {
    name: String,
    method: usize,
    packed: usize,
    unpacked: usize,
    header: usize,
    unix_mode: u32,
}

/// The list of files in a zip file's bytes.
fn entries(zip: &[u8]) -> Option<Vec<Entry>> {
    // The list's place is written at the very end of the file.
    let end = (0..=zip.len().checked_sub(22)?)
        .rev()
        .take(66_000)
        .find(|&at| zip[at..].starts_with(b"PK\x05\x06"))?;
    let count = le16(zip, end + 10)?;
    let mut at = le32(zip, end + 16)?;
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        if !zip.get(at..)?.starts_with(b"PK\x01\x02") {
            return None;
        }
        let name_len = le16(zip, at + 28)?;
        let extra_len = le16(zip, at + 30)?;
        let comment_len = le16(zip, at + 32)?;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len)?).to_string();
        list.push(Entry {
            name,
            method: le16(zip, at + 10)?,
            packed: le32(zip, at + 20)?,
            unpacked: le32(zip, at + 24)?,
            header: le32(zip, at + 42)?,
            // The top half of the "external attributes" is the Unix mode,
            // when the zip was made on Unix.
            unix_mode: (le32(zip, at + 38)? >> 16) as u32,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Some(list)
}

/// Unpack the zip file `zip` into the folder `to`: every file, or with
/// `only`, the one file with that name (written straight into `to` under
/// its own last name). Returns the files written. Programs keep (or, for
/// `only`, get) their "may be run" mark on Linux.
pub fn unzip(zip: &Path, to: &Path, only: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let name = zip.display();
    let bytes = std::fs::read(zip).map_err(|e| format!("cannot read {name}: {e}"))?;
    let list = entries(&bytes).ok_or_else(|| format!("{name} is not a zip file"))?;
    std::fs::create_dir_all(to).map_err(|e| format!("cannot make {}: {e}", to.display()))?;
    let mut written = Vec::new();
    for entry in &list {
        if entry.name.ends_with('/') || only.is_some_and(|only| only != entry.name) {
            continue;
        }
        // Never write outside the folder, whatever the zip file says.
        let inside = Path::new(&entry.name);
        let safe = inside
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)));
        if !safe || entry.name.contains('\\') {
            return Err(format!(
                "{name} holds a file with an unsafe name: {}",
                entry.name
            ));
        }
        if entry.packed == 0xFFFF_FFFF || entry.unpacked == 0xFFFF_FFFF {
            return Err(format!(
                "{name} holds a file too large to unpack: {}",
                entry.name
            ));
        }
        let broken = || format!("{name} is broken at {}", entry.name);
        let header = entry.header;
        if !bytes
            .get(header..)
            .is_some_and(|b| b.starts_with(b"PK\x03\x04"))
        {
            return Err(broken());
        }
        let start = header
            + 30
            + le16(&bytes, header + 26).ok_or_else(broken)?
            + le16(&bytes, header + 28).ok_or_else(broken)?;
        let packed = bytes.get(start..start + entry.packed).ok_or_else(broken)?;
        let data = match entry.method {
            0 => packed.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(packed, entry.unpacked)
                .map_err(|_| broken())?,
            other => {
                return Err(format!(
                    "{name} packs {} in a way Duck Echo cannot unpack (method {other})",
                    entry.name
                ))
            }
        };
        if data.len() != entry.unpacked {
            return Err(broken());
        }
        let target = match only {
            Some(_) => to.join(inside.file_name().unwrap_or_default()),
            None => to.join(inside),
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot make {}: {e}", parent.display()))?;
        }
        std::fs::write(&target, data)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if only.is_some() || entry.unix_mode & 0o111 != 0 {
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
            }
        }
        #[cfg(not(unix))]
        let _ = entry.unix_mode;
        written.push(target);
    }
    if written.is_empty() {
        return Err(match only {
            Some(only) => format!("{name} does not hold {only}"),
            None => format!("{name} is empty"),
        });
    }
    Ok(written)
}

/// Unpack `archive` into `to`: a `.zip` here, a `.tar.gz` with the
/// system's `tar` (Linux has it; llama.cpp's Linux builds come that way).
pub fn unpack(archive: &Path, to: &Path) -> Result<(), String> {
    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") || name.ends_with(".whl") {
        return unzip(archive, to, None).map(|_| ());
    }
    std::fs::create_dir_all(to).map_err(|e| format!("cannot make {}: {e}", to.display()))?;
    let out = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(to)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot start tar to unpack {}: {e}", archive.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tar could not unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::sha256_hex;
    use std::io::{Read, Write};
    use std::sync::Mutex;

    /// A folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("duck-echo-net-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tiny web server on this PC for one test: it serves `files` by
    /// name (carrying on from the middle when asked to) and remembers each
    /// request's first line and `Range`.
    struct Site {
        address: String,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Site {
        fn serving(files: Vec<(&'static str, Vec<u8>)>) -> Site {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            let asked = Arc::new(Mutex::new(Vec::new()));
            let log = Arc::clone(&asked);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut request = Vec::new();
                    let mut byte = [0u8; 1];
                    while !request.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte) {
                            Ok(1) => request.push(byte[0]),
                            _ => break,
                        }
                    }
                    let request = String::from_utf8_lossy(&request).to_string();
                    let first = request.lines().next().unwrap_or_default().to_string();
                    let range = request.lines().find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_string)
                    });
                    log.lock().unwrap().push(match &range {
                        Some(range) => format!("{first} [from {range}]"),
                        None => first.clone(),
                    });
                    let name = first
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .trim_start_matches('/');
                    let answer = match files.iter().find(|(file, _)| *file == name) {
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                        Some((_, data)) => {
                            let from: usize = range
                                .and_then(|r| r.trim_end_matches('-').parse().ok())
                                .unwrap_or(0);
                            if from >= data.len() && from > 0 {
                                b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                            } else {
                                let body = &data[from..];
                                let head = if from > 0 {
                                    format!(
                                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\n",
                                        data.len() - 1,
                                        data.len()
                                    )
                                } else {
                                    "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\n".to_string()
                                };
                                let mut answer =
                                    format!("{head}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                                        .into_bytes();
                                answer.extend_from_slice(body);
                                answer
                            }
                        }
                    };
                    let _ = stream.write_all(&answer);
                }
            });
            Site { address, asked }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    fn some_bytes(count: usize) -> Vec<u8> {
        (0..count).map(|i| (i * 7 % 253) as u8).collect()
    }

    fn the(data: &[u8]) -> Expected {
        Expected {
            bytes: Some(data.len() as u64),
            sha256: Some(sha256_hex(data)),
        }
    }

    #[test]
    fn a_file_is_downloaded_checked_and_put_in_place() {
        let data = some_bytes(300_000);
        let site = Site::serving(vec![("model.bin", data.clone())]);
        let dir = folder("whole");
        let to = dir.join("models").join("model.bin");
        let mut seen = Vec::new();
        download(
            &format!("{}/model.bin", site.address),
            &to,
            &the(&data),
            &mut |done, of| seen.push((done, of)),
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), data);
        assert!(!dir.join("models").join("model.bin.part").exists());
        assert_eq!(seen.last(), Some(&(300_000, Some(300_000))));
        // Asked again, the file that is there and right is kept: no request.
        download(
            &format!("{}/model.bin", site.address),
            &to,
            &the(&data),
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(site.asked().len(), 1, "{:?}", site.asked());
    }

    #[test]
    fn a_stopped_download_carries_on_from_where_it_stopped() {
        let data = some_bytes(200_000);
        let site = Site::serving(vec![("big.bin", data.clone())]);
        let dir = folder("resume");
        let to = dir.join("big.bin");
        std::fs::write(dir.join("big.bin.part"), &data[..120_000]).unwrap();
        download(
            &format!("{}/big.bin", site.address),
            &to,
            &the(&data),
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), data);
        assert_eq!(site.asked(), ["GET /big.bin HTTP/1.1 [from 120000-]"]);
    }

    #[test]
    fn a_part_that_is_already_whole_is_only_checked() {
        let data = some_bytes(50_000);
        let site = Site::serving(vec![("whole.bin", data.clone())]);
        let dir = folder("whole-part");
        std::fs::write(dir.join("whole.bin.part"), &data).unwrap();
        download(
            &format!("{}/whole.bin", site.address),
            &dir.join("whole.bin"),
            &the(&data),
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap();
        assert!(site.asked().is_empty());
        assert_eq!(std::fs::read(dir.join("whole.bin")).unwrap(), data);
    }

    #[test]
    fn a_file_with_the_wrong_sha256_is_fetched_once_more_then_refused() {
        let data = some_bytes(40_000);
        let site = Site::serving(vec![("bad.bin", data.clone())]);
        let dir = folder("wrong");
        let to = dir.join("bad.bin");
        let wanted = Expected {
            bytes: Some(40_000),
            sha256: Some("0".repeat(64)),
        };
        let error = download(
            &format!("{}/bad.bin", site.address),
            &to,
            &wanted,
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "wrong_sha256");
        assert!(
            error
                .to_string()
                .contains("bad.bin arrived twice with the wrong SHA-256"),
            "{error}"
        );
        assert_eq!(
            site.asked().len(),
            2,
            "fetched once more: {:?}",
            site.asked()
        );
        assert!(
            !to.exists() && !dir.join("bad.bin.part").exists(),
            "nothing is kept"
        );
    }

    #[test]
    fn a_file_that_is_not_there_and_a_site_that_is_not_there_say_so() {
        let site = Site::serving(vec![]);
        let dir = folder("missing");
        let nothing = Expected::default();
        let error = download(
            &format!("{}/nothing.bin", site.address),
            &dir.join("nothing.bin"),
            &nothing,
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "not_found", "{error:?}");
        assert_eq!(
            fetch_text(&format!("{}/SHA256SUMS", site.address))
                .unwrap_err()
                .code(),
            "not_found"
        );
        // A port nobody listens on: the "no internet" words.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let error = fetch_text(&format!("http://127.0.0.1:{port}/SHA256SUMS")).unwrap_err();
        assert_eq!(error.code(), "no_internet", "{error:?}");
        assert!(
            error.to_string().contains("Check the internet connection"),
            "{error}"
        );
    }

    #[test]
    fn text_is_fetched() {
        let site = Site::serving(vec![("SHA256SUMS", b"abc  model.json\n".to_vec())]);
        assert_eq!(
            fetch_text(&format!("{}/SHA256SUMS", site.address)).unwrap(),
            "abc  model.json\n"
        );
    }

    #[test]
    fn a_file_too_big_for_the_disk_is_refused_before_it_is_fetched() {
        let site = Site::serving(vec![("huge.bin", vec![1, 2, 3])]);
        let dir = folder("room");
        let huge = Expected {
            bytes: Some(u64::MAX / 4),
            sha256: None,
        };
        if free_bytes(&dir).is_none() {
            eprintln!("skipped: this system does not say how much room is free");
            return;
        }
        let error = download(
            &format!("{}/huge.bin", site.address),
            &dir.join("huge.bin"),
            &huge,
            &mut |_, _| {},
            &Cancel::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "no_room");
        assert!(error.to_string().contains("not enough room"), "{error}");
        assert!(site.asked().is_empty(), "nothing was fetched");
    }

    #[test]
    fn a_cancelled_download_stops_and_says_so() {
        let data = some_bytes(10_000);
        let site = Site::serving(vec![("file.bin", data.clone())]);
        let dir = folder("cancel");
        let cancel = Cancel::new();
        cancel.cancel();
        let error = download(
            &format!("{}/file.bin", site.address),
            &dir.join("file.bin"),
            &the(&data),
            &mut |_, _| {},
            &cancel,
        )
        .unwrap_err();
        assert_eq!(error, NetError::Cancelled);
        assert!(!dir.join("file.bin").exists());
    }

    /// A zip file with `files` in it: `packed` ones deflated, the others
    /// stored as they are.
    fn zip_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut list = Vec::new();
        for (name, data, packed) in files {
            let body = if *packed {
                miniz_oxide::deflate::compress_to_vec(data, 6)
            } else {
                data.to_vec()
            };
            let at = out.len() as u32;
            let method: u16 = if *packed { 8 } else { 0 };
            let mut common = Vec::new();
            common.extend_from_slice(&20u16.to_le_bytes()); // version needed
            common.extend_from_slice(&0u16.to_le_bytes()); // flags
            common.extend_from_slice(&method.to_le_bytes());
            common.extend_from_slice(&[0; 8]); // time, date, crc (not checked here)
            common.extend_from_slice(&(body.len() as u32).to_le_bytes());
            common.extend_from_slice(&(data.len() as u32).to_le_bytes());
            common.extend_from_slice(&(name.len() as u16).to_le_bytes());
            common.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&common);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            list.extend_from_slice(b"PK\x01\x02");
            list.extend_from_slice(&20u16.to_le_bytes()); // made by
            list.extend_from_slice(&common);
            list.extend_from_slice(&[0; 6]); // comment, disk, internal attributes
            list.extend_from_slice(&(0o100755u32 << 16).to_le_bytes());
            list.extend_from_slice(&at.to_le_bytes());
            list.extend_from_slice(name.as_bytes());
        }
        let list_at = out.len() as u32;
        out.extend_from_slice(&list);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(list.len() as u32).to_le_bytes());
        out.extend_from_slice(&list_at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn a_zip_file_is_unpacked_whole_or_one_file_of_it() {
        let dir = folder("zip");
        let program = some_bytes(70_000);
        let zip = dir.join("uv.zip");
        std::fs::write(
            &zip,
            zip_of(&[
                ("uv-1.data/scripts/uv", &program, true),
                ("uv-1.dist-info/METADATA", b"Name: uv\n", false),
            ]),
        )
        .unwrap();
        let all = dir.join("all");
        unpack(&zip, &all).unwrap();
        assert_eq!(
            std::fs::read(all.join("uv-1.data/scripts/uv")).unwrap(),
            program
        );
        assert_eq!(
            std::fs::read(all.join("uv-1.dist-info/METADATA")).unwrap(),
            b"Name: uv\n"
        );
        let one = dir.join("one");
        let written = unzip(&zip, &one, Some("uv-1.data/scripts/uv")).unwrap();
        assert_eq!(written, [one.join("uv")]);
        assert_eq!(std::fs::read(one.join("uv")).unwrap(), program);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(one.join("uv"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "the program may be run");
        }
        assert!(unzip(&zip, &one, Some("not-there"))
            .unwrap_err()
            .contains("does not hold"));
    }

    #[test]
    fn a_zip_file_cannot_write_outside_its_folder() {
        let dir = folder("zip-unsafe");
        let zip = dir.join("bad.zip");
        std::fs::write(&zip, zip_of(&[("../outside.txt", b"no", false)])).unwrap();
        let error = unzip(&zip, &dir.join("in"), None).unwrap_err();
        assert!(error.contains("unsafe name"), "{error}");
        assert!(!dir.join("outside.txt").exists());
        std::fs::write(&zip, b"not a zip file at all, just words").unwrap();
        assert!(unzip(&zip, &dir.join("in"), None)
            .unwrap_err()
            .contains("is not a zip file"));
    }

    #[test]
    fn sizes_in_words() {
        assert_eq!(megabytes(784_000_000), "784 MB");
        assert_eq!(megabytes(19_882_074_080), "19.9 GB");
        assert_eq!(megabytes(10), "1 MB");
    }
}
