//! AGENTS.md and CLAUDE.md, at the top of the repository, tell agents
//! what to run. Everything they name must be there: every `duck-echo`
//! command (and its subcommand, guide page or `do` command), every crate
//! and test that `-p` and `--test` name, every file and folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn top() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A file at the top of the repository, with Windows line endings (a
/// checkout with `core.autocrlf`) read as plain ones.
fn read(name: &str) -> String {
    std::fs::read_to_string(top().join(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .replace("\r\n", "\n")
}

fn both() -> String {
    read("AGENTS.md") + "\n" + &read("CLAUDE.md")
}

fn duck_echo(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_duck-echo"))
        .args(args)
        .output()
        .unwrap()
}

/// A word that can be a command's name: lowercase letters, digits, `-`
/// and `_`, starting with a letter.
fn is_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_lowercase())
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Every `duck-echo <command> [<word>]` in the text. The second word is
/// kept only when the command goes on (no backtick or other mark after the
/// first) and it is a name, not a file, an option or a comment.
fn commands(text: &str) -> Vec<(String, Option<String>)> {
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for after in text.split("duck-echo ").skip(1) {
        let mut words = after.split_whitespace();
        let Some(first) = words.next() else { continue };
        let name = first.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
        if !is_name(name) {
            continue;
        }
        let second = if name == first {
            words.next().filter(|w| is_name(w)).map(str::to_owned)
        } else {
            None
        };
        found.push((name.to_owned(), second));
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_command_the_agent_files_name_is_there() {
    let found = commands(&both());
    for command in [
        "guide", "presets", "do", "validate", "render", "compare", "commands",
    ] {
        assert!(
            found.iter().any(|(c, _)| c == command),
            "AGENTS.md names `duck-echo {command}`"
        );
    }
    let listing = duck_echo(&["commands"]);
    assert!(listing.status.success());
    let listing = String::from_utf8(listing.stdout).unwrap();
    let do_commands: Vec<&str> = listing
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    for (command, second) in &found {
        let output = duck_echo(&[command, "--help"]);
        assert!(
            output.status.success(),
            "AGENTS.md names `duck-echo {command}`, which isn't a command"
        );
        let Some(second) = second else { continue };
        match command.as_str() {
            // `do NAME`: one of the commands `duck-echo commands` lists.
            "do" => assert!(
                do_commands.contains(&second.as_str()),
                "AGENTS.md names `duck-echo do {second}`, which `duck-echo commands` doesn't list"
            ),
            // `guide TOPIC`: the page must print.
            "guide" => assert!(
                duck_echo(&["guide", second]).status.success(),
                "AGENTS.md names `duck-echo guide {second}`, which isn't a page"
            ),
            // Anything else: a subcommand such as `presets list`. (For a
            // command that takes a name or a value, `--help` still works.)
            _ => assert!(
                duck_echo(&[command, second, "--help"]).status.success(),
                "AGENTS.md names `duck-echo {command} {second}`, which isn't a command"
            ),
        }
    }
}

#[test]
fn every_crate_test_and_file_the_agent_files_name_is_there() {
    let text = both();
    let top = top();
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | ','))
        .filter(|word| !word.is_empty())
        .collect();
    let mut crates_named = 0;
    for pair in words.windows(2) {
        match pair[0] {
            "-p" => {
                crates_named += 1;
                assert!(
                    top.join("crates")
                        .join(pair[1])
                        .join("Cargo.toml")
                        .is_file(),
                    "AGENTS.md names the crate {}, which isn't there",
                    pair[1]
                );
            }
            "--test" => {
                let found = std::fs::read_dir(top.join("crates")).unwrap().any(|krate| {
                    krate
                        .unwrap()
                        .path()
                        .join("tests")
                        .join(format!("{}.rs", pair[1]))
                        .is_file()
                });
                assert!(
                    found,
                    "AGENTS.md names the test {}, which isn't there",
                    pair[1]
                );
            }
            _ => {}
        }
    }
    assert!(crates_named >= 2, "AGENTS.md names the crates to build");

    // Folders of this repository that the files point into, and the files
    // at its top. (`sfx/coin.toml` in an example is a file the agent makes,
    // so it is not checked.)
    let folders = [
        "crates/",
        "docs/",
        "schema/",
        "scripts/",
        "examples/",
        "presets/",
        "instruments/",
        "playtest/",
        "studio/",
        ".github/",
    ];
    let mut paths_named = 0;
    for word in &words {
        let word = word
            .trim_start_matches('@')
            .trim_end_matches(['.', ':', ';']);
        let in_a_folder = folders.iter().any(|folder| word.starts_with(folder));
        let at_the_top = !word.contains('/') && (word.ends_with(".md") || word == ".mcp.json");
        if (in_a_folder || at_the_top) && !word.contains(['<', '*', '$']) {
            paths_named += 1;
            assert!(
                top.join(word).exists(),
                "AGENTS.md or CLAUDE.md names {word}, which isn't there"
            );
        }
    }
    assert!(paths_named >= 10, "found only {paths_named} paths");
}

#[test]
fn claude_md_brings_in_agents_md() {
    assert!(
        read("CLAUDE.md").starts_with("@AGENTS.md\n"),
        "CLAUDE.md starts with the line @AGENTS.md, which brings AGENTS.md in"
    );
}
