//! Build script: Cargo runs this before compiling the app.
//!
//! On Windows it puts the Duck Echo icon and the version into
//! DuckEchoStudio.exe, so File Explorer, the taskbar and desktop shortcuts
//! show them. On other systems it does nothing (Linux takes the icon from
//! the AppImage's own files).

fn main() {
    println!("cargo:rerun-if-changed=../../brand/duck-echo.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../../Cargo.toml");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../brand/duck-echo.ico");
    resource.set("ProductName", "Duck Echo Studio");
    resource.set("FileDescription", "Duck Echo Studio");
    if let Err(error) = resource.compile() {
        // On your own PC a missing icon is not worth failing the build. On
        // GitHub's machines (which set CI) it is: the download must have
        // its icon.
        if std::env::var_os("CI").is_some() {
            panic!("could not add the icon to DuckEchoStudio.exe: {error}");
        }
        println!("cargo:warning=could not add the icon to DuckEchoStudio.exe: {error}");
    }
}
