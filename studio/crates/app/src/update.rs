//! New versions: is there a newer Duck Echo Studio than this one?
//!
//! The same rules as Duck Forge Studio and Duck Quill:
//!
//! * When the Studio starts (unless Settings says not to), it asks GitHub
//!   for the list of Duck Echo's releases: one plain request for a public
//!   page, with nothing about the person or their computer in it, made by
//!   the `curl` program that comes with Windows and with Linux.
//! * Only releases tagged `studio-vX.Y.Z` count; drafts and pre-releases
//!   never do; a version older than or equal to this one never does. With
//!   no internet, or an answer it cannot read, it says nothing.
//! * Nothing is downloaded until the person presses **Update**. Then it
//!   downloads this system's file (`DuckEchoStudioSetup.exe` on Windows,
//!   `DuckEchoStudio.AppImage` on Linux) and the release's `SHA256SUMS`,
//!   works out the file's own SHA-256 fingerprint, and **refuses to use it
//!   if the two differ** (as Duck Quill does): the download is deleted and
//!   the banner says so.
//! * Windows: the checked installer is started, and the Studio closes so
//!   it can put the new version in. Linux: the checked AppImage replaces
//!   the running one in one step (a rename, so a failed download leaves the
//!   old one working), and the Studio starts again.
//!
//! The fingerprint proves the file arrived whole and is the one listed with
//! that release. It cannot prove who published it; only a paid
//! code-signing certificate does that, as with the siblings.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Where Duck Echo's releases are listed.
pub const RELEASES: &str =
    "https://api.github.com/repos/mrguymrguy647-del/Duck-Echo/releases?per_page=30";

/// The release's page, for "What's new" when a release has no page.
pub const RELEASES_PAGE: &str = "https://github.com/mrguymrguy647-del/Duck-Echo/releases";

/// Where the list is read from: [`RELEASES`], or the address in
/// `DUCK_ECHO_RELEASES` (CI points it at pretend lists, to try the real
/// download program on both systems).
pub fn releases_address() -> String {
    std::env::var("DUCK_ECHO_RELEASES").unwrap_or_else(|_| RELEASES.to_string())
}

/// The installer's name in a release (Windows).
pub const INSTALLER: &str = "DuckEchoStudioSetup.exe";
/// The AppImage's name in a release (Linux).
pub const APPIMAGE: &str = "DuckEchoStudio.AppImage";
/// The list of SHA-256 fingerprints in a release.
pub const SUMS: &str = "SHA256SUMS";
/// The tag of a Studio release: `studio-v0.1.0`.
pub const TAG_PREFIX: &str = "studio-v";

/// The file this system updates from.
pub fn this_systems_file() -> &'static str {
    if cfg!(windows) { INSTALLER } else { APPIMAGE }
}

/// A file in a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Where to download it.
    pub address: String,
    /// Its size in bytes (0 when the list did not say).
    pub size: u64,
}

/// A newer Studio than this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    /// "0.2.0".
    pub version: String,
    /// The release's page: what's new in it.
    pub whats_new: String,
    /// This system's file in the release, if it has one.
    pub file: Option<Asset>,
    /// The release's `SHA256SUMS`, if it has one.
    pub sums: Option<Asset>,
}

/// `[major, minor, patch]` from "0.2.0", "v0.2.0" or "studio-v0.2.0".
pub fn version_numbers(text: &str) -> Option<[u64; 3]> {
    let text = text.strip_prefix("studio-").unwrap_or(text);
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        *number = parts.next()?.trim().parse().ok()?;
    }
    parts.next().is_none().then_some(numbers)
}

/// The newest Studio release in GitHub's list, if it is newer than
/// `running`. `file` is this system's file name. Drafts, pre-releases, other
/// tags and anything that cannot be read do not count.
pub fn newest(releases: &str, running: &str, file: &str) -> Option<NewVersion> {
    let running = version_numbers(running)?;
    let list: Value = serde_json::from_str(releases).ok()?;
    let mut best: Option<([u64; 3], &Value)> = None;
    for release in list.as_array()? {
        let Some(tag) = release["tag_name"].as_str() else {
            continue;
        };
        if !tag.starts_with(TAG_PREFIX)
            || release["draft"].as_bool() == Some(true)
            || release["prerelease"].as_bool() == Some(true)
        {
            continue;
        }
        let Some(numbers) = version_numbers(tag) else {
            continue;
        };
        if best.is_none_or(|(newest, _)| numbers > newest) {
            best = Some((numbers, release));
        }
    }
    let (numbers, release) = best?;
    if numbers <= running {
        return None;
    }
    let asset = |name: &str| {
        release["assets"].as_array().and_then(|assets| {
            assets.iter().find_map(|asset| {
                if asset["name"].as_str() != Some(name) {
                    return None;
                }
                Some(Asset {
                    address: asset["browser_download_url"].as_str()?.to_string(),
                    size: asset["size"].as_u64().unwrap_or(0),
                })
            })
        })
    };
    Some(NewVersion {
        version: format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]),
        whats_new: release["html_url"]
            .as_str()
            .unwrap_or(RELEASES_PAGE)
            .to_string(),
        file: asset(file),
        sums: asset(SUMS),
    })
}

/// The fingerprint `SHA256SUMS` gives for `name`, in lower-case hex. The
/// list has one line per file: the fingerprint, spaces (and a `*` for a
/// binary file), and the name.
pub fn listed_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, rest) = line.trim().split_once(char::is_whitespace)?;
        let file = rest.trim_start().trim_start_matches('*');
        let ok = file == name && sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit());
        ok.then(|| sum.to_ascii_lowercase())
    })
}

/// The SHA-256 fingerprint of a file, in lower-case hex.
pub fn sha256_of(file: &Path) -> std::io::Result<String> {
    let mut reader = std::fs::File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// How the Studio fetches a page or a file. Tests give it a pretend
/// internet instead.
pub trait Internet: Send + 'static {
    /// The text at `address`.
    fn get(&self, address: &str) -> Result<String, String>;
    /// Save what is at `address` in the file `to`.
    fn save(&self, address: &str, to: &Path) -> Result<(), String>;
}

/// The real internet, through `curl`: Windows' own `curl.exe` (in
/// System32, not another one that happens to be first on the PATH), or
/// the system's `curl` on Linux. It sends only what any download sends.
pub struct Curl;

impl Curl {
    fn command() -> std::process::Command {
        let windows = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("curl.exe"))
            .filter(|curl| cfg!(windows) && curl.is_file());
        #[allow(unused_mut)]
        let mut command = std::process::Command::new(windows.unwrap_or_else(|| "curl".into()));
        #[cfg(windows)]
        {
            // No black window flashing up behind the Studio.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        command.args(["--silent", "--show-error", "--fail", "--location"]);
        command.stdin(std::process::Stdio::null());
        command
    }

    fn said(output: &std::process::Output) -> String {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.trim();
        if said.is_empty() {
            "curl did not say why".to_string()
        } else {
            said.to_string()
        }
    }
}

impl Internet for Curl {
    fn get(&self, address: &str) -> Result<String, String> {
        let output = Curl::command()
            .args(["--connect-timeout", "5", "--max-time", "15"])
            .args(["--header", "Accept: application/vnd.github+json"])
            .arg(address)
            .output()
            .map_err(|error| format!("could not start curl: {error}"))?;
        if !output.status.success() {
            return Err(Curl::said(&output));
        }
        String::from_utf8(output.stdout).map_err(|_| "the answer was not text".to_string())
    }

    fn save(&self, address: &str, to: &Path) -> Result<(), String> {
        let output = Curl::command()
            .args(["--connect-timeout", "10", "--max-time", "900", "--output"])
            .arg(to)
            .arg(address)
            .output()
            .map_err(|error| format!("could not start curl: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Curl::said(&output))
        }
    }
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// A newer Studio is out.
    Newer(NewVersion),
    /// This is the newest.
    UpToDate,
    /// The list could not be fetched or read, and why.
    NoAnswer(String),
}

/// Ask once, and wait for the answer.
pub fn check_now(internet: &impl Internet, running: &str) -> Found {
    match internet.get(&releases_address()) {
        Err(why) => Found::NoAnswer(why),
        Ok(list) => {
            if serde_json::from_str::<Value>(&list).map_or(true, |v| !v.is_array()) {
                return Found::NoAnswer("the answer was not a list of releases".into());
            }
            match newest(&list, running, this_systems_file()) {
                Some(new) => Found::Newer(new),
                None => Found::UpToDate,
            }
        }
    }
}

/// A check for a new version, running in the background so the Studio
/// starts at once.
pub struct UpdateCheck {
    answer: Receiver<Found>,
}

impl UpdateCheck {
    /// Start asking. `running` is this Studio's version.
    pub fn start(internet: impl Internet, running: &str) -> UpdateCheck {
        let (send, answer) = channel();
        let running = running.to_string();
        std::thread::spawn(move || {
            let _ = send.send(check_now(&internet, &running));
        });
        UpdateCheck { answer }
    }

    /// `None` while it is still asking; then the answer, once.
    pub fn poll(&mut self) -> Option<Found> {
        match self.answer.try_recv() {
            Ok(found) => Some(found),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Found::NoAnswer("the check stopped".into())),
        }
    }
}

/// Downloading and checking a new version, after Update was pressed.
pub struct Download {
    done: Receiver<Result<PathBuf, String>>,
}

impl Download {
    /// Start downloading `new`'s file into `folder`, and check it.
    pub fn start(internet: impl Internet, new: &NewVersion, folder: &Path) -> Download {
        let (send, done) = channel();
        let new = new.clone();
        let folder = folder.to_path_buf();
        std::thread::spawn(move || {
            let _ = send.send(download(&internet, &new, &folder));
        });
        Download { done }
    }

    /// `None` while it is downloading; then the checked file, or why it
    /// could not be downloaded or was refused.
    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the download stopped".into())),
        }
    }
}

/// Download `new`'s file into `folder` and check its fingerprint. A file
/// that does not match is deleted, never returned.
pub fn download(
    internet: &impl Internet,
    new: &NewVersion,
    folder: &Path,
) -> Result<PathBuf, String> {
    let name = this_systems_file();
    let Some(file) = &new.file else {
        return Err(format!("version {} has no {name} to download", new.version));
    };
    let Some(sums) = &new.sums else {
        return Err(format!(
            "version {} has no list of fingerprints ({SUMS}), so its download cannot be checked",
            new.version
        ));
    };
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    // A hidden name while it downloads: a half-downloaded file never looks
    // like a finished one.
    let target = folder.join(format!(".{name}.{}.download", new.version));
    let _ = std::fs::remove_file(&target);
    let refuse = |why: String| {
        let _ = std::fs::remove_file(&target);
        Err(why)
    };
    internet.save(&file.address, &target)?;
    let got = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if got == 0 || (file.size > 0 && got != file.size) {
        return refuse(format!(
            "the download was {got} bytes instead of {}; try again",
            file.size
        ));
    }
    let listed = match internet.get(&sums.address) {
        Ok(text) => listed_sum(&text, name),
        Err(why) => {
            return refuse(format!(
                "the list of fingerprints could not be downloaded ({why})"
            ));
        }
    };
    let Some(listed) = listed else {
        return refuse(format!("the list of fingerprints does not mention {name}"));
    };
    let actual = match sha256_of(&target) {
        Ok(sum) => sum,
        Err(error) => return refuse(format!("the download could not be read: {error}")),
    };
    if actual != listed {
        return refuse(format!(
            "the download's fingerprint does not match the one listed with the release, so it \
             was deleted and not used (got {actual}, the list says {listed})"
        ));
    }
    Ok(target)
}

/// Where a Windows installer is downloaded: the system's temporary folder.
pub fn installer_folder() -> PathBuf {
    std::env::temp_dir().join("Duck Echo Studio update")
}

/// Start a downloaded installer (Windows). The Studio closes after this,
/// so the installer can put the new version in its place.
pub fn start_installer(file: &Path) -> Result<(), String> {
    // An installer must end in .exe for Windows to run it.
    let exe = file.with_file_name(INSTALLER.replace(".exe", "-new.exe"));
    let _ = std::fs::remove_file(&exe);
    std::fs::rename(file, &exe).map_err(|error| error.to_string())?;
    std::process::Command::new(&exe)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start {}: {error}", exe.display()))
}

/// Put a downloaded and checked AppImage in place of the running one
/// (Linux), in one step: a rename in the same folder, so either the old or
/// the new one is there, never half of one.
pub fn replace_appimage(new: &Path, running: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("could not make the new version runnable: {error}"))?;
    }
    std::fs::rename(new, running).map_err(|error| {
        format!(
            "could not put the new version in place of {}: {error}",
            running.display()
        )
    })
}

/// Start the AppImage at `appimage` again (after an update).
pub fn restart(appimage: &Path) -> Result<(), String> {
    std::process::Command::new(appimage)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start {}: {error}", appimage.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// A release, as GitHub lists it.
    fn release(tag: &str, draft: bool, prerelease: bool, files: bool) -> String {
        let assets = if files {
            format!(
                r#"[{{"name": "DuckEchoStudioSetup.exe", "browser_download_url": "https://example.test/{tag}/setup", "size": 5}},
                   {{"name": "DuckEchoStudio.AppImage", "browser_download_url": "https://example.test/{tag}/appimage", "size": 5}},
                   {{"name": "SHA256SUMS", "browser_download_url": "https://example.test/{tag}/sums", "size": 100}}]"#
            )
        } else {
            "[]".to_string()
        };
        format!(
            r#"{{"tag_name": "{tag}", "draft": {draft}, "prerelease": {prerelease},
                "html_url": "https://example.test/releases/tag/{tag}", "assets": {assets}}}"#
        )
    }

    fn list(releases: &[String]) -> String {
        format!("[{}]", releases.join(","))
    }

    #[test]
    fn version_numbers_are_read_from_tags() {
        assert_eq!(version_numbers("studio-v0.2.10"), Some([0, 2, 10]));
        assert_eq!(version_numbers("v1.0.0"), Some([1, 0, 0]));
        assert_eq!(version_numbers("0.1"), None);
        assert_eq!(version_numbers("0.1.0.4"), None);
        assert_eq!(version_numbers("studio-vX"), None);
    }

    #[test]
    fn only_a_newer_published_studio_release_counts() {
        let releases = list(&[
            release("studio-v0.3.0", true, false, true), // a draft
            release("studio-v0.4.0", false, true, true), // a pre-release
            release("v9.0.0", false, false, true),       // not a Studio tag
            release("studio-v0.2.0", false, false, true),
            release("studio-v0.1.5", false, false, true),
        ]);
        let new = newest(&releases, "0.1.0", APPIMAGE).unwrap();
        assert_eq!(new.version, "0.2.0");
        assert_eq!(
            new.whats_new,
            "https://example.test/releases/tag/studio-v0.2.0"
        );
        assert_eq!(
            new.file.unwrap().address,
            "https://example.test/studio-v0.2.0/appimage"
        );
        assert_eq!(
            new.sums.unwrap().address,
            "https://example.test/studio-v0.2.0/sums"
        );
        let new = newest(&releases, "0.1.0", INSTALLER).unwrap();
        assert_eq!(
            new.file.unwrap().address,
            "https://example.test/studio-v0.2.0/setup"
        );
        // Never the same or an older version.
        assert_eq!(newest(&releases, "0.2.0", APPIMAGE), None);
        assert_eq!(newest(&releases, "0.9.0", APPIMAGE), None);
        // Nothing readable: nothing to say.
        assert_eq!(newest("not json", "0.1.0", APPIMAGE), None);
        assert_eq!(newest("[]", "0.1.0", APPIMAGE), None);
        // A release without files is still news, with nothing to download.
        let bare = list(&[release("studio-v1.0.0", false, false, false)]);
        let new = newest(&bare, "0.1.0", APPIMAGE).unwrap();
        assert_eq!((new.file, new.sums), (None, None));
    }

    #[test]
    fn the_listed_fingerprint_is_found_by_name() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums =
            format!("{a}  DuckEchoStudioSetup.exe\n{b} *DuckEchoStudio.AppImage\nnot a line\n");
        assert_eq!(listed_sum(&sums, INSTALLER), Some(a));
        assert_eq!(listed_sum(&sums, APPIMAGE), Some("b".repeat(64)));
        assert_eq!(listed_sum(&sums, "other"), None);
        assert_eq!(listed_sum("abc  DuckEchoStudio.AppImage", APPIMAGE), None);
    }

    #[test]
    fn sha256_matches_the_standard_example() {
        let dir = std::env::temp_dir().join(format!("duck-echo-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(
            sha256_of(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A pretend internet: pages by address. `get` of a missing page fails,
    /// as with no internet.
    #[derive(Clone, Default)]
    struct Pretend {
        pages: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    }

    impl Pretend {
        fn with(self, address: &str, bytes: &[u8]) -> Self {
            self.pages
                .lock()
                .unwrap()
                .insert(address.into(), bytes.to_vec());
            self
        }
    }

    impl Internet for Pretend {
        fn get(&self, address: &str) -> Result<String, String> {
            let pages = self.pages.lock().unwrap();
            let page = pages.get(address).ok_or("no internet")?;
            Ok(String::from_utf8_lossy(page).into_owned())
        }

        fn save(&self, address: &str, to: &Path) -> Result<(), String> {
            let bytes = self
                .pages
                .lock()
                .unwrap()
                .get(address)
                .cloned()
                .ok_or("no internet")?;
            std::fs::write(to, bytes).map_err(|e| e.to_string())
        }
    }

    fn new_version(size: u64) -> NewVersion {
        NewVersion {
            version: "0.2.0".into(),
            whats_new: RELEASES_PAGE.into(),
            file: Some(Asset {
                address: "https://example.test/file".into(),
                size,
            }),
            sums: Some(Asset {
                address: "https://example.test/sums".into(),
                size: 0,
            }),
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("duck-echo-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_download_with_the_right_fingerprint_is_kept() {
        let dir = folder("good");
        // "abc" and its SHA-256.
        let sums = format!(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  {}\n",
            this_systems_file()
        );
        let internet = Pretend::default()
            .with("https://example.test/file", b"abc")
            .with("https://example.test/sums", sums.as_bytes());
        let file = download(&internet, &new_version(3), &dir).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"abc");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_download_with_another_fingerprint_is_deleted_and_refused() {
        let dir = folder("bad");
        let sums = format!("{}  {}\n", "0".repeat(64), this_systems_file());
        let internet = Pretend::default()
            .with("https://example.test/file", b"abc")
            .with("https://example.test/sums", sums.as_bytes());
        let why = download(&internet, &new_version(3), &dir).unwrap_err();
        assert!(why.contains("does not match"), "{why}");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "the bad download is gone"
        );

        // The wrong size, a list that does not name the file, no list at
        // all, and no file: all refused, nothing left behind.
        let why = download(&internet, &new_version(99), &dir).unwrap_err();
        assert!(why.contains("3 bytes instead of 99"), "{why}");
        let other = Pretend::default()
            .with("https://example.test/file", b"abc")
            .with(
                "https://example.test/sums",
                format!("{}  other.exe\n", "0".repeat(64)).as_bytes(),
            );
        let why = download(&other, &new_version(3), &dir).unwrap_err();
        assert!(why.contains("does not mention"), "{why}");
        let no_list = Pretend::default().with("https://example.test/file", b"abc");
        let why = download(&no_list, &new_version(3), &dir).unwrap_err();
        assert!(why.contains("could not be downloaded"), "{why}");
        let unlisted = NewVersion {
            sums: None,
            ..new_version(3)
        };
        assert!(
            download(&internet, &unlisted, &dir)
                .unwrap_err()
                .contains("cannot be checked")
        );
        let empty = NewVersion {
            file: None,
            ..new_version(3)
        };
        assert!(
            download(&internet, &empty, &dir)
                .unwrap_err()
                .contains("has no")
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_check_says_newer_up_to_date_or_no_answer() {
        // The check reads releases_address(); without DUCK_ECHO_RELEASES
        // that is the real address, which the pretend internet serves.
        if std::env::var_os("DUCK_ECHO_RELEASES").is_some() {
            return;
        }
        let newer = Pretend::default().with(
            RELEASES,
            list(&[release("studio-v0.9.0", false, false, true)]).as_bytes(),
        );
        match check_now(&newer, "0.1.0") {
            Found::Newer(new) => assert_eq!(new.version, "0.9.0"),
            other => panic!("{other:?}"),
        }
        assert_eq!(check_now(&newer, "0.9.0"), Found::UpToDate);
        assert!(matches!(
            check_now(&Pretend::default(), "0.1.0"),
            Found::NoAnswer(_)
        ));
        let garbled = Pretend::default().with(RELEASES, b"{\"message\": \"rate limited\"}");
        assert!(matches!(check_now(&garbled, "0.1.0"), Found::NoAnswer(_)));
        // In the background, too.
        let mut check = UpdateCheck::start(newer, "0.1.0");
        let found = loop {
            if let Some(found) = check.poll() {
                break found;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(matches!(found, Found::Newer(_)));
    }

    #[cfg(unix)]
    #[test]
    fn a_new_appimage_replaces_the_old_one_in_one_step() {
        use std::os::unix::fs::PermissionsExt;
        let dir = folder("replace");
        std::fs::create_dir_all(&dir).unwrap();
        let running = dir.join("DuckEchoStudio.AppImage");
        std::fs::write(&running, "old").unwrap();
        let new = dir.join(".DuckEchoStudio.AppImage.0.2.0.download");
        std::fs::write(&new, "new").unwrap();
        replace_appimage(&new, &running).unwrap();
        assert_eq!(std::fs::read_to_string(&running).unwrap(), "new");
        assert!(!new.exists());
        let mode = std::fs::metadata(&running).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        std::fs::remove_dir_all(&dir).ok();
    }
}
