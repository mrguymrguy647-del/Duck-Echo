//! `duck-echo`: validate, render and analyze sound recipes from the shell.
//!
//! Exit codes: 0 success, 1 the recipe or file is invalid, 2 I/O or usage
//! error. With `--json` every command prints exactly one JSON document on
//! stdout so agents can parse it; diagnostics go to stderr.

use clap::{Parser, Subcommand, ValueEnum};
use duck_echo_audio::{write_wav, WavFormat};
use duck_echo_core::{load_path, Report};
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
        /// Recipe file (.toml or .json).
        recipe: PathBuf,
        /// Print a machine-readable JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Render a recipe to a WAV file.
    Render {
        /// Recipe file (.toml or .json).
        recipe: PathBuf,
        /// Output WAV path. Defaults to the recipe name with `.wav`.
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
        /// WAV file, or a recipe (.toml/.json) to render in memory.
        input: PathBuf,
        /// Compact single-line JSON instead of pretty-printed.
        #[arg(long)]
        compact: bool,
    },
    /// Compare two sounds: what changed from A to B, and how similar the
    /// waveforms are.
    Compare {
        /// First sound: WAV file or recipe.
        a: PathBuf,
        /// Second sound: WAV file or recipe.
        b: PathBuf,
        /// Compact single-line JSON instead of pretty-printed.
        #[arg(long)]
        compact: bool,
    },
    /// Print the JSON Schema for recipes.
    Schema,
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
        Command::Analyze { input, compact } => analyze(&input, compact),
        Command::Compare { a, b, compact } => match duck_echo_tools::compare_paths(&a, &b) {
            Ok(c) => print_json(&c, compact),
            Err(e) => tool_error(e),
        },
        Command::Schema => {
            println!("{}", duck_echo_core::schema::recipe_schema_json());
            ExitCode::SUCCESS
        }
    }
}

fn print_report(report: &Report, json: bool) {
    if json {
        println!("{}", report.to_json());
    } else {
        eprint!("{report}");
    }
}

fn validate(path: &Path, json: bool) -> ExitCode {
    match load_path(path) {
        Ok(loaded) => {
            print_report(&loaded.report, json);
            ExitCode::SUCCESS
        }
        Err(report) => {
            print_report(&report, json);
            ExitCode::from(1)
        }
    }
}

fn render(
    path: &Path,
    output: Option<PathBuf>,
    seed: Option<u64>,
    format: WavFormat,
    json: bool,
) -> ExitCode {
    let loaded = match load_path(path) {
        Ok(l) => l,
        Err(report) => {
            print_report(&report, json);
            return ExitCode::from(1);
        }
    };
    let mut recipe = loaded.recipe;
    if let Some(s) = seed {
        recipe.seed = s;
    }
    let output = output.unwrap_or_else(|| path.with_extension("wav"));

    let buffer = match duck_echo_synth::render(&recipe) {
        Ok(b) => b,
        Err(report) => {
            print_report(&report, json);
            return ExitCode::from(1);
        }
    };
    if let Err(e) = write_wav(&output, &buffer, format) {
        eprintln!("error: cannot write {}: {e}", output.display());
        return ExitCode::from(2);
    }
    let analysis = duck_echo_analyze::analyze(&buffer.samples, buffer.sample_rate, buffer.channels);

    if json {
        let doc = serde_json::json!({
            "ok": true,
            "output": output,
            "seed": recipe.seed,
            "warnings": loaded.report.warnings,
            "analysis": analysis,
        });
        println!("{}", serde_json::to_string_pretty(&doc).expect("json"));
    } else {
        for w in &loaded.report.warnings {
            eprintln!("{w}");
        }
        let peak = analysis
            .peak_dbfs
            .map(|p| format!("{p:.1} dBFS"))
            .unwrap_or_else(|| "silent".into());
        let clip = if analysis.clipping.clipped {
            format!(", CLIPPING ({} samples)", analysis.clipping.clipped_samples)
        } else {
            String::new()
        };
        let loud = match (analysis.loudness_lufs, analysis.loudness_method) {
            (Some(l), Some(duck_echo_analyze::LoudnessMethod::KWeightedUngated)) => {
                format!(", ~{l:.1} LUFS (short clip, ungated)")
            }
            (Some(l), _) => format!(", {l:.1} LUFS"),
            _ => String::new(),
        };
        eprintln!(
            "wrote {} ({:.3} s, peak {peak}{loud}{clip})",
            output.display(),
            analysis.duration_seconds
        );
    }
    ExitCode::SUCCESS
}

fn analyze(path: &Path, compact: bool) -> ExitCode {
    match duck_echo_tools::load_audio(path) {
        Ok(buffer) => print_json(&duck_echo_tools::analyze_buffer(&buffer), compact),
        Err(e) => tool_error(e),
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

/// Print a tool error: an invalid recipe as its report (exit 1), anything
/// else as one line (exit 2).
fn tool_error(e: ToolError) -> ExitCode {
    match e {
        ToolError::Invalid(report) => {
            eprint!("{report}");
            ExitCode::from(1)
        }
        ToolError::Io(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}
