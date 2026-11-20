//! `duck-echo ear install`, `ear update` and `ear remove`: a person
//! installs the Duck Ear from the command line. Each shows what it will
//! do and asks first (`--yes` skips the question).

use duck_echo_tools::ear::default_home;
use duck_echo_tools::ear_install::{self, Event};
use duck_echo_tools::ear_net::{megabytes, Cancel};
use std::io::Write;
use std::process::ExitCode;

/// Ask a yes or no question on the terminal. Anything but y or yes is no.
pub fn asked(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Prints what is being done: each step on its own line, and a download's
/// progress every tenth of the way (or every 100 MB when its size is not
/// known).
pub fn printer() -> impl FnMut(Event) {
    let mut last = (String::new(), u64::MAX);
    move |event| match event {
        Event::Step(words) => println!("{words}"),
        Event::Download { what, done, of } => {
            let mark = match of {
                Some(of) if of > 0 => done * 10 / of,
                _ => done / 100_000_000,
            };
            if last != (what.to_string(), mark) {
                last = (what.to_string(), mark);
                match of {
                    Some(of) => println!("  {what}: {} of {}", megabytes(done), megabytes(of)),
                    None if done > 0 => println!("  {what}: {}", megabytes(done)),
                    None => {}
                }
            }
        }
        Event::Question { done, of } => {
            // Every twentieth of the way.
            let mark = (done * 20 / of.max(1)) as u64;
            if last != ("questions".to_string(), mark) {
                last = ("questions".to_string(), mark);
                println!("  question {} of {of}", done + 1);
            }
        }
    }
}

fn failed(error: &ear_install::InstallError, json: bool) -> ExitCode {
    if json {
        println!(
            "{}",
            serde_json::json!({"ok": false, "code": error.code, "message": error.message})
        );
    } else {
        eprintln!("error[{}]: {}", error.code, error.message);
    }
    ExitCode::from(2)
}

/// `duck-echo ear install` (and `ear update`, which is the same work: what
/// is already there and right is kept).
pub fn install(yes: bool, json: bool, update: bool) -> ExitCode {
    let home = default_home();
    let plan = match ear_install::plan(&home) {
        Ok(plan) => plan,
        Err(error) => return failed(&error, json),
    };
    if update && ear_install::installed(&home).is_none() {
        eprintln!(
            "The Duck Ear is not installed in {}: `duck-echo ear install` installs it.",
            home.display()
        );
        return ExitCode::from(2);
    }
    if !json {
        println!("{}", plan.summary);
        for (part, bytes) in &plan.parts {
            println!("  {part}: about {}", megabytes(*bytes));
        }
    }
    if !yes
        && !asked(if update {
            "Update the Duck Ear?"
        } else {
            "Install the Duck Ear?"
        })
    {
        println!("Nothing was installed.");
        return ExitCode::from(2);
    }
    let mut print = printer();
    let mut quiet = |_: Event| {};
    let events: &mut dyn FnMut(Event) = if json { &mut quiet } else { &mut print };
    match ear_install::install(&home, events, &Cancel::new()) {
        Ok(installed) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"ok": true, "home": home, "installed": installed})
                );
            } else {
                println!(
                    "The Duck Ear {} with {} is in {} ({} on disk). Try it: duck-echo listen preset:coin --like \"a coin pickup\"",
                    installed.ear_version,
                    installed.score_model,
                    home.display(),
                    megabytes(ear_install::disk_use(&home))
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => failed(&error, json),
    }
}

/// `duck-echo ear remove`.
pub fn remove(yes: bool, json: bool) -> ExitCode {
    let home = default_home();
    let size = ear_install::disk_use(&home);
    if size == 0 {
        println!("There is no Duck Ear in {}.", home.display());
        return ExitCode::SUCCESS;
    }
    let question = format!(
        "Remove the Duck Ear from {} ({}: its programs, its models and its kept answers)?",
        home.display(),
        megabytes(size)
    );
    if !yes && !asked(&question) {
        println!("Nothing was removed.");
        return ExitCode::from(2);
    }
    match ear_install::remove(&home) {
        Ok(freed) => {
            if json {
                println!("{}", serde_json::json!({"ok": true, "freed_bytes": freed}));
            } else {
                println!("Removed the Duck Ear: {} is free again.", megabytes(freed));
            }
            ExitCode::SUCCESS
        }
        Err(error) => failed(&error, json),
    }
}

/// `duck-echo ear try <model>`: download a describe model (after asking),
/// measure it on the test set, show the results and save them.
pub fn try_model(which: &str, quick: bool, yes: bool, out: Option<&std::path::Path>) -> ExitCode {
    use duck_echo_tools::ear_try;
    let home = default_home();
    let cards = duck_echo_tools::ear_cards::cards();
    let plan = match ear_try::try_plan(&home, which, quick, &cards) {
        Ok(plan) => plan,
        Err(error) => return failed(&error, false),
    };
    println!("{}", plan.summary);
    for (part, bytes) in &plan.parts {
        println!("  {part}: {}", megabytes(*bytes));
    }
    if !yes && !asked(&format!("Try {}?", plan.model)) {
        println!("Nothing was downloaded.");
        return ExitCode::from(2);
    }
    let mut print = printer();
    let tried = match ear_try::try_model(&home, which, quick, &cards, &mut print, &Cancel::new()) {
        Ok(tried) => tried,
        Err(error) => return failed(&error, false),
    };
    println!();
    if let Some(describe) = &tried.report.describe {
        println!(
            "{} on {}:",
            describe.model,
            describe.device_name.as_deref().unwrap_or(&describe.device)
        );
        println!();
        print!("{}", describe.table);
        println!();
    }
    println!("{}", tried.summary);
    let folder = match out {
        Some(out) => out.to_path_buf(),
        None => std::path::PathBuf::from(format!(
            "ear-check-{}{}",
            tried.model,
            if quick { "-quick" } else { "" }
        )),
    };
    if let Err(error) = ear_try::save_results(&home, which, &folder) {
        return failed(&error, false);
    }
    println!(
        "The results to send are {} and {}.",
        folder.join("ear-check.json").display(),
        folder.join("ear-check.md").display()
    );
    if tried.passed {
        println!("To describe with it: duck-echo ear use {which}");
    }
    ExitCode::SUCCESS
}

/// `duck-echo ear use <model>` (or `none`): which describe model the Ear
/// uses. A model is accepted only after it passed on this PC.
pub fn use_model(which: &str) -> ExitCode {
    let home = default_home();
    let which = (which != "none").then_some(which);
    match duck_echo_tools::ear_try::use_model(&home, which) {
        Ok(words) => {
            println!("{words}");
            ExitCode::SUCCESS
        }
        Err(error) => failed(&error, false),
    }
}
