//! Linux: Duck Echo Studio in the desktop's app menu.
//!
//! An AppImage is one file with no installer, so nothing puts it in the
//! app menu. The first time it starts, the Studio asks "Add Duck Echo
//! Studio to your app menu?", and **Add** writes two files into the
//! person's own folders (nothing system-wide), as the freedesktop.org
//! standards describe:
//!
//! * `applications/duck-echo-studio.desktop` in `$XDG_DATA_HOME` (usually
//!   `~/.local/share`): the menu entry, which starts the AppImage;
//! * `icons/hicolor/256x256/apps/duck-echo-studio.png` there: the icon.
//!
//! If the AppImage is moved, the entry would point at nothing, so at each
//! start the Studio checks and points it at the AppImage's new place.
//! Settings has **Remove from app menu**, which deletes both files.
//!
//! Every function takes the data folder as an argument, so tests use a
//! folder of their own.

use std::path::{Path, PathBuf};

/// The menu entry's file name. It is also the window's app id, so the
/// desktop knows the window belongs to this entry.
pub const APP_ID: &str = "duck-echo-studio";

/// The icon, as the menu shows it.
pub const ICON: &[u8] = include_bytes!("../../../brand/duck-echo-icon-256.png");

/// The AppImage this Studio was started from, if it was: the AppImage's
/// own start-up code sets `APPIMAGE` to its full path.
pub fn running_appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("APPIMAGE")?);
    path.is_file().then_some(path)
}

/// The person's data folder (`$XDG_DATA_HOME`, usually `~/.local/share`).
pub fn data_folder() -> Option<PathBuf> {
    dirs::data_dir()
}

/// The menu entry's file in `data`.
pub fn entry_file(data: &Path) -> PathBuf {
    data.join("applications").join(format!("{APP_ID}.desktop"))
}

/// The icon's file in `data`.
pub fn icon_file(data: &Path) -> PathBuf {
    data.join("icons")
        .join("hicolor")
        .join("256x256")
        .join("apps")
        .join(format!("{APP_ID}.png"))
}

/// `path` as one argument of a desktop entry's `Exec` line: in double
/// quotes, with `"`, `` ` ``, `$` and `\` escaped, and `%` doubled, as
/// the Desktop Entry Specification asks.
fn exec_argument(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The menu entry for an AppImage at `appimage`.
pub fn entry_text(appimage: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Duck Echo Studio\n\
         GenericName=Sound and music maker\n\
         Comment=Make sound effects and music without a terminal\n\
         Exec={}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;\n\
         StartupWMClass={APP_ID}\n\
         X-Duck-Echo-AppImage={}\n",
        exec_argument(appimage),
        appimage.display()
    )
}

/// The AppImage the entry in `data` starts, if there is an entry.
pub fn entry_points_to(data: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(entry_file(data)).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("X-Duck-Echo-AppImage="))
        .map(PathBuf::from)
}

/// True when the menu entry is there.
pub fn is_added(data: &Path) -> bool {
    entry_file(data).is_file()
}

/// Put Duck Echo Studio in the app menu, starting `appimage`.
pub fn add(data: &Path, appimage: &Path) -> Result<(), String> {
    let write = |file: PathBuf, bytes: &[u8]| {
        duck_echo_tools::save::write_atomic(&file, bytes)
            .map_err(|_| format!("could not write {}", file.display()))
    };
    write(icon_file(data), ICON)?;
    write(entry_file(data), entry_text(appimage).as_bytes())?;
    refresh_menu(data);
    Ok(())
}

/// Take Duck Echo Studio out of the app menu.
pub fn remove(data: &Path) -> Result<(), String> {
    for file in [entry_file(data), icon_file(data)] {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("could not remove {}: {error}", file.display())),
        }
    }
    refresh_menu(data);
    Ok(())
}

/// If the entry starts an AppImage that is not `appimage` (it was moved),
/// point it at `appimage`. Returns true when the entry was fixed.
pub fn follow_if_moved(data: &Path, appimage: &Path) -> bool {
    match entry_points_to(data) {
        Some(old) if old != appimage => add(data, appimage).is_ok(),
        _ => false,
    }
}

/// Most desktops notice a new entry by themselves; this asks the others.
/// The program may well not be there, which is fine.
fn refresh_menu(data: &Path) {
    let _ = std::process::Command::new("update-desktop-database")
        .arg(data.join("applications"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| child.wait());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("duck-echo-menu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn add_writes_the_entry_and_the_icon_and_remove_takes_both_away() {
        let data = folder("add");
        let appimage = PathBuf::from("/home/me/Apps/DuckEchoStudio.AppImage");
        assert!(!is_added(&data));
        add(&data, &appimage).unwrap();
        assert!(is_added(&data));
        let text = std::fs::read_to_string(entry_file(&data)).unwrap();
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("Exec=\"/home/me/Apps/DuckEchoStudio.AppImage\"\n"));
        assert!(text.contains("Icon=duck-echo-studio\n"));
        assert_eq!(std::fs::read(icon_file(&data)).unwrap(), ICON);
        assert_eq!(entry_points_to(&data), Some(appimage));
        remove(&data).unwrap();
        assert!(!is_added(&data));
        assert!(!icon_file(&data).exists());
        // Removing twice is fine.
        remove(&data).unwrap();
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn the_written_entry_passes_the_freedesktop_checker_when_it_is_there() {
        // desktop-file-validate (desktop-file-utils) is on CI's Linux
        // machine; elsewhere this checks nothing.
        let data = folder("validate");
        add(&data, Path::new("/home/me/My Apps/DuckEchoStudio.AppImage")).unwrap();
        match std::process::Command::new("desktop-file-validate")
            .arg(entry_file(&data))
            .output()
        {
            Ok(out) => assert!(
                out.status.success() && out.stdout.is_empty(),
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            Err(_) => println!("desktop-file-validate is not installed here"),
        }
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn a_moved_appimage_is_followed() {
        let data = folder("moved");
        let old = PathBuf::from("/home/me/Downloads/DuckEchoStudio.AppImage");
        let new = PathBuf::from("/home/me/Apps/DuckEchoStudio.AppImage");
        assert!(!follow_if_moved(&data, &new), "no entry: nothing to follow");
        add(&data, &old).unwrap();
        assert!(!follow_if_moved(&data, &old));
        assert!(follow_if_moved(&data, &new));
        assert_eq!(entry_points_to(&data), Some(new));
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn odd_characters_in_the_path_are_escaped_for_the_exec_line() {
        let path = Path::new("/home/me/My \"Apps\" $5 100%/Duck\\Echo.AppImage");
        assert_eq!(
            exec_argument(path),
            "\"/home/me/My \\\"Apps\\\" \\$5 100%%/Duck\\\\Echo.AppImage\""
        );
    }
}
