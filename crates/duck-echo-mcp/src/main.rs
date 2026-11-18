//! `duck-echo-mcp`: the Duck Echo agent tools as a Model Context Protocol
//! server over stdio.
//!
//! Usage: `duck-echo-mcp [--root DIR]`. Every file path a tool receives is
//! resolved against DIR (default: the current directory) and must stay
//! inside it. Messages are newline-delimited JSON-RPC 2.0 on stdin and
//! stdout; logs go to stderr.

mod protocol;
mod tools;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut root = PathBuf::from(".");
    let mut duck_forge: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--duck-forge" => match args.next() {
                Some(dir) => duck_forge = Some(PathBuf::from(dir)),
                None => {
                    eprintln!("duck-echo-mcp: --duck-forge needs the Duck Forge folder");
                    return ExitCode::from(2);
                }
            },
            "--root" => match args.next() {
                Some(dir) => root = PathBuf::from(dir),
                None => {
                    eprintln!("duck-echo-mcp: --root needs a directory");
                    return ExitCode::from(2);
                }
            },
            "--version" | "-V" => {
                println!("duck-echo-mcp {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!(
                    "duck-echo-mcp [--root DIR] [--duck-forge DIR]\n\nDuck Echo tools as an MCP server over stdio. File paths given to tools\nare resolved against DIR (default: the current directory) and must stay\ninside it. With --duck-forge, game_send and game_list work with the games\nof that Duck Forge folder, writing only into their assets and art folders."
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("duck-echo-mcp: unknown argument `{other}` (try --help)");
                return ExitCode::from(2);
            }
        }
    }
    let mut root = match duck_echo_tools::root::Root::new(&root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("duck-echo-mcp: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(dir) = duck_forge {
        match duck_echo_tools::forge::DuckForge::open(&dir) {
            Ok(forge) => {
                eprintln!("duck-echo-mcp: games of {}", forge.dir().display());
                root = root.with_duck_forge(forge);
            }
            Err(e) => {
                eprintln!("duck-echo-mcp: --duck-forge: {e}");
                return ExitCode::from(2);
            }
        }
    }
    eprintln!("duck-echo-mcp: serving tools for {}", root.dir().display());
    let mut server = protocol::Server::new(root);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line) {
            if writeln!(stdout, "{reply}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}
