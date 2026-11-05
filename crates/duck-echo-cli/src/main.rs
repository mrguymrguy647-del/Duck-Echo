//! `duck-echo`: validate, render, analyze and compare sound recipes from the
//! shell, browse presets and render variations.
//!
//! Anywhere a recipe is expected, `preset:NAME` names a built-in preset.
//!
//! Exit codes: 0 success, 1 the recipe is invalid, 2 I/O or usage error.
//! Commands that print JSON (`analyze`, `compare`, and the others with
//! `--json`) print exactly one JSON document on stdout, failures included:
//! a failure is a validation report (`{"ok": false, "errors": [...]}`).
//! Human-readable diagnostics go to stderr.

use clap::{Parser, Subcommand, ValueEnum};
use duck_echo_audio::WavFormat;
use duck_echo_core::{Code, Issue, Report};
use duck_echo_tools::ToolError;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "duck-echo",
    version,
    about = "Audio studio for agents and humans"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a recipe and report every problem with its field path.
    Validate {
        /// Recipe file (.toml or .json) or preset:NAME.
        recipe: PathBuf,
        /// Print a machine-readable JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Render a recipe to a WAV file.
    Render {
        /// Recipe file (.toml or .json) or preset:NAME.
        recipe: PathBuf,
        /// Output WAV path. Defaults to the recipe or preset name with `.wav`.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Override the recipe seed.
        #[arg(long)]
        seed: Option<u64>,
        /// WAV sample format.
        #[arg(long, value_enum, default_value_t = Bits::Pcm16)]
        format: Bits,
        /// Print a JSON summary with the analysis of the render.
        #[arg(long)]
        json: bool,
    },
    /// Measure a sound: summary, duration, peak, RMS, loudness, clipping,
    /// silence, attack, decay, brightness, tonality and pitch track.
    Analyze {
        /// WAV file, or a recipe (.toml/.json or preset:NAME) to render in
        /// memory.
        input: PathBuf,
        /// Compact single-line JSON instead of pretty-printed.
        #[arg(long)]
        compact: bool,
    },
    /// Compare two sounds: what changed from A to B, and how similar the
    /// waveforms are.
    Compare {
        /// First sound: WAV file, recipe or preset:NAME.
        a: PathBuf,
        /// Second sound: WAV file, recipe or preset:NAME.
        b: PathBuf,
        /// Compact single-line JSON instead of pretty-printed.
        #[arg(long)]
        compact: bool,
    },
    /// Browse the built-in presets.
    Presets {
        #[command(subcommand)]
        action: PresetsAction,
    },
    /// Render seeds 1..N of a recipe and analyze each, to pick a variant by
    /// its numbers.
    Variations {
        /// Recipe file (.toml or .json) or preset:NAME.
        recipe: PathBuf,
        /// How many seeds to render (1..=64).
        #[arg(long, default_value_t = 8)]
        count: u32,
        /// Override the recipe's `variation` amount (0..=1).
        #[arg(long)]
        variation: Option<f32>,
        /// Directory for `<name>_seed<N>.wav` files. Without it nothing is
        /// written and only the analyses are printed.
        #[arg(short, long)]
        output_dir: Option<PathBuf>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the JSON Schema for recipes.
    Schema,
}

#[derive(Subcommand)]
enum PresetsAction {
    /// List presets with their category, tags and description.
    List {
        /// Only presets with this tag, such as `ui` or `explosion`.
        #[arg(long)]
        tag: Option<String>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a preset's recipe (TOML with comments, or JSON).
    Show {
        /// Preset name.
        name: String,
        /// Print the recipe as JSON instead of TOML.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Bits {
    /// 16-bit PCM.
    Pcm16,
    /// 24-bit PCM.
    Pcm24,
    /// 32-bit float.
    Float32,
}

impl From<Bits> for WavFormat {
    fn from(b: Bits) -> Self {
        match b {
            Bits::Pcm16 => WavFormat::Pcm16,
            Bits::Pcm24 => WavFormat::Pcm24,
            Bits::Float32 => WavFormat::Float32,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { recipe, json } => validate(&recipe, json),
        Command::Render {
            recipe,
            output,
            seed,
            format,
            json,
        } => render(&recipe, output, seed, format.into(), json),
        Command::Analyze { input, compact } => match duck_echo_tools::load_audio(&input) {
            Ok(buffer) => print_json(&duck_echo_tools::analyze_buffer(&buffer), compact),
            Err(e) => tool_error(e, true),
        },
        Command::Compare { a, b, compact } => match duck_echo_tools::compare_paths(&a, &b) {
            Ok(c) => print_json(&c, compact),
            Err(e) => tool_error(e, true),
        },
        Command::Presets { action } => presets(action),
        Command::Variations {
            recipe,
            count,
            variation,
            output_dir,
            json,
        } => variations(&recipe, count, variation, output_dir.as_deref(), json),
        Command::Schema => {
            println!("{}", duck_echo_core::schema::recipe_schema_json());
            ExitCode::SUCCESS
        }
    }
}

fn validate(reference: &Path, json: bool) -> ExitCode {
    match duck_echo_tools::load_recipe(reference) {
        Ok(r) => {
            if json {
                println!("{}", r.loaded.report.to_json());
            } else {
                eprint!("{}", r.loaded.report);
            }
            ExitCode::SUCCESS
        }
        Err(e) => tool_error(e, json),
    }
}

fn render(
    reference: &Path,
    output: Option<PathBuf>,
    seed: Option<u64>,
    format: WavFormat,
    json: bool,
) -> ExitCode {
    let r = match duck_echo_tools::load_recipe(reference) {
        Ok(r) => r,
        Err(e) => return tool_error(e, json),
    };
    let output = output.unwrap_or_else(|| {
        if duck_echo_tools::is_recipe_path(reference) {
            reference.with_extension("wav")
        } else {
            PathBuf::from(format!("{}.wav", r.stem))
        }
    });
    let done = match duck_echo_tools::render_to_file(&r.loaded, &output, seed, format) {
        Ok(d) => d,
        Err(e) => return tool_error(e, json),
    };
    if json {
        return print_json(&done, false);
    }
    for w in &done.warnings {
        eprintln!("{w}");
    }
    eprintln!("wrote {}: {}", done.output, done.analysis.summary);
    ExitCode::SUCCESS
}

fn presets(action: PresetsAction) -> ExitCode {
    match action {
        PresetsAction::List { tag, json } => match duck_echo_tools::presets::list(tag.as_deref()) {
            Ok(list) if json => print_json(&list, false),
            Ok(list) => {
                for p in list {
                    let target = p
                        .target_lufs
                        .map(|t| format!("{t:.0} LUFS"))
                        .unwrap_or_default();
                    println!(
                        "{:<14} {:<9} {:<9} {}",
                        p.name, p.category, target, p.description
                    );
                }
                ExitCode::SUCCESS
            }
            Err(e) => tool_error(e, json),
        },
        PresetsAction::Show { name, json } => match duck_echo_tools::presets::get(&name) {
            Ok(p) if json => match duck_echo_tools::presets::load(p) {
                Ok(l) => {
                    println!("{}", duck_echo_core::to_json(&l.recipe));
                    ExitCode::SUCCESS
                }
                Err(e) => tool_error(e, true),
            },
            Ok(p) => {
                print!("{}", p.source);
                ExitCode::SUCCESS
            }
            Err(e) => tool_error(e, json),
        },
    }
}

fn variations(
    reference: &Path,
    count: u32,
    variation: Option<f32>,
    out_dir: Option<&Path>,
    json: bool,
) -> ExitCode {
    let result = duck_echo_tools::load_recipe(reference)
        .and_then(|r| duck_echo_tools::variations(&r, count, variation, out_dir));
    match result {
        Ok(v) if json => print_json(&v, false),
        Ok(v) => {
            if let Some(note) = &v.note {
                eprintln!("note: {note}");
            }
            for r in &v.renders {
                let place = r.output.as_deref().unwrap_or("(not written)");
                println!("seed {:>2}  {place}  {}", r.seed, r.summary);
            }
            ExitCode::SUCCESS
        }
        Err(e) => tool_error(e, json),
    }
}

fn print_json<T: serde::Serialize>(value: &T, compact: bool) -> ExitCode {
    let text = if compact {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    }
    .expect("json");
    println!("{text}");
    ExitCode::SUCCESS
}

/// Report a failure: an invalid recipe exits 1, anything else 2. In JSON
/// mode the failure is printed on stdout as a report.
fn tool_error(e: ToolError, json: bool) -> ExitCode {
    let (report, code) = match e {
        ToolError::Invalid(report) => (report, 1),
        ToolError::Io(msg) => (Report::from_error(Issue::error(Code::Other, "", msg)), 2),
    };
    if json {
        println!("{}", report.to_json());
    } else {
        eprint!("{report}");
    }
    ExitCode::from(code)
}
