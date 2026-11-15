//! Settings > Connect an AI assistant: the few lines an AI assistant app
//! (such as Claude Desktop) needs to start Duck Echo's MCP server on your
//! sounds folder, ready to copy.
//!
//! The MCP server is a program of its own, `duck-echo-mcp`. The Windows
//! installer puts it next to `DuckEchoStudio.exe`; the AppImage holds it
//! and starts it when its first word is `mcp`. Either way the assistant
//! is told to start it with `--root` set to the same folder the Studio
//! uses, so the two share the files and the Undo history.

use std::path::{Path, PathBuf};

/// How an AI assistant starts Duck Echo's MCP server.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    /// The program to start.
    pub command: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// False when the program is not where it should be (a Studio built
    /// from source without the server next to it).
    pub found: bool,
}

/// The server for the sounds folder `project`: the AppImage this Studio
/// runs from (with `mcp`), or `duck-echo-mcp` next to the Studio.
pub fn server(project: &Path, appimage: Option<&Path>) -> Server {
    let root = project.display().to_string();
    if let Some(appimage) = appimage {
        return Server {
            command: appimage.to_path_buf(),
            args: vec!["mcp".into(), "--root".into(), root],
            found: appimage.is_file(),
        };
    }
    let name = if cfg!(windows) {
        "duck-echo-mcp.exe"
    } else {
        "duck-echo-mcp"
    };
    let command = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name));
    Server {
        found: command.is_file(),
        command,
        args: vec!["--root".into(), root],
    }
}

/// The text to paste into the assistant's settings: Claude Desktop's
/// `claude_desktop_config.json` (other MCP apps take the same lines).
pub fn config_text(server: &Server) -> String {
    let config = serde_json::json!({
        "mcpServers": {
            "duck-echo": {
                "command": server.command.display().to_string(),
                "args": server.args,
            }
        }
    });
    serde_json::to_string_pretty(&config).expect("plain JSON")
}

/// Where to paste it, in plain steps for this system.
pub fn steps() -> &'static [&'static str] {
    if cfg!(windows) {
        &[
            "1. Click Copy.",
            "2. In Claude Desktop, open Settings, then Developer, then Edit Config. \
             The file claude_desktop_config.json opens.",
            "3. If the file is empty or holds only {}, replace everything with what you \
             copied. If it already has \"mcpServers\", add just the \"duck-echo\" part \
             inside it.",
            "4. Save the file, then quit Claude Desktop fully (from the tray icon too) and \
             open it again.",
            "5. Ask Claude something like: \"In Duck Echo, make my coin a little lower.\"",
        ]
    } else {
        &[
            "1. Click Copy.",
            "2. Paste it into your AI assistant app's MCP settings. For Claude Desktop's \
             config file it goes in as it is; other apps ask for the same command and \
             arguments.",
            "3. Restart the assistant app.",
            "4. Ask it something like: \"In Duck Echo, make my coin a little lower.\"",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_starts_the_server_on_the_sounds_folder() {
        let project = Path::new("/home/me/Documents/Duck Echo");
        let next_to = server(project, None);
        let text = config_text(&next_to);
        let back: serde_json::Value = serde_json::from_str(&text).unwrap();
        let entry = &back["mcpServers"]["duck-echo"];
        assert!(entry["command"].as_str().unwrap().contains("duck-echo-mcp"));
        assert_eq!(
            entry["args"],
            serde_json::json!(["--root", project.display().to_string()])
        );

        let appimage = Path::new("/home/me/DuckEchoStudio.AppImage");
        let from_appimage = server(project, Some(appimage));
        let back: serde_json::Value = serde_json::from_str(&config_text(&from_appimage)).unwrap();
        let entry = &back["mcpServers"]["duck-echo"];
        assert_eq!(entry["command"], "/home/me/DuckEchoStudio.AppImage");
        assert_eq!(entry["args"][0], "mcp");
        assert!(!steps().is_empty());
    }
}
