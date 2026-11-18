//! `duck-echo`: validate, render, analyze and compare sound recipes from the
//! shell, browse presets and render variations.
//!
//! Anywhere a recipe is expected, `preset:NAME` names a built-in preset
//! and `instrument:NAME` a built-in instrument.
//! `validate`, `render`, `analyze` and `compare` also take a song (a
//! document with `tracks`); they tell the two apart by looking inside.
//!
//! `commands`, `do` and `run` reach the same named commands as Duck Echo
//! Studio's buttons and the MCP server's command tools (see
//! `duck_echo_tools::commands`). Each change they make is a step of the
//! project's shared Undo history.
//!
//! Exit codes: 0 success, 1 the recipe is invalid, 2 I/O or usage error.
//! Commands that print JSON (`analyze`, `compare`, and the others with
//! `--json`) print exactly one JSON document on stdout, failures included:
//! a failure is a validation report (`{"ok": false, "errors": [...]}`).
//! Every failure has a specific code and a path: a recipe field, or the
//! argument at fault (`input`, `--output`, `--solo[1]`). Human-readable
//! diagnostics go to stderr.

use clap::{Parser, Subcommand, ValueEnum};
use duck_echo_audio::WavFormat;
use duck_echo_tools::commands;
use duck_echo_tools::history::Who;
use duck_echo_tools::project::Project;
use duck_echo_tools::root::Root;
use duck_echo_tools::song::files_next_to;
use duck_echo_tools::{measure, Document, Options, ToolError};
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
    /// Check a recipe or a song and report every problem with its field
    /// path.
    Validate {
        /// Recipe file (.toml or .json) or preset:NAME.
        recipe: PathBuf,
        /// Print a machine-readable JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Render a recipe or a song to a WAV file.
    Render {
        /// Recipe file (.toml or .json) or preset:NAME.
        recipe: PathBuf,
        /// Output file: `.wav` (sound effects), or `.ogg` (music: OGG
        /// Vorbis, decoded again and checked before it is kept). Defaults to
        /// the recipe or preset name with `.wav`.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Override the recipe seed.
        #[arg(long)]
        seed: Option<u64>,
        /// WAV sample format.
        #[arg(long, value_enum, default_value_t = Bits::Pcm16)]
        format: Bits,
        /// OGG quality, 0 (smallest) to 10 (best), as `oggenc -q`. Default
        /// 4. Only for an `.ogg` output.
        #[arg(long)]
        quality: Option<f32>,
        /// Print a JSON summary with the analysis of the render.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        extras: ExtrasArgs,
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
        #[command(flatten)]
        extras: ExtrasArgs,
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
    /// Browse the built-in instruments a song can use.
    Instruments {
        #[command(subcommand)]
        action: InstrumentsAction,
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
    /// Print the agent guide, or a reference page: recipe_spec, song_spec,
    /// analysis, instruments or presets.
    Guide {
        /// Which page. Default `guide`.
        #[arg(default_value = "guide")]
        topic: String,
    },
    /// List the named commands that change a project (the same ones the
    /// Studio's buttons and the MCP server use).
    Commands {
        /// Print JSON: each command's name, description and the JSON
        /// Schema of its arguments.
        #[arg(long)]
        json: bool,
    },
    /// Run one command, such as
    /// `duck-echo do sound_set '{"file": "coin.toml", "path": "layers[0].volume", "value": 0.6}'`.
    /// Prints the result as JSON. The change is one step of the project's
    /// Undo history.
    Do {
        /// The command's name (`duck-echo commands` lists them).
        name: String,
        /// Its arguments as a JSON object. Default `{}`.
        #[arg(default_value = "{}")]
        arguments: String,
        #[command(flatten)]
        project: ProjectArgs,
    },
    /// Run a script: a JSON file holding a list of commands, each an object
    /// whose `"do"` is the command's name. They run in order and the first
    /// failure stops the script. Each command is its own Undo step.
    Run {
        /// The script, a .json file inside the project folder.
        script: String,
        #[command(flatten)]
        project: ProjectArgs,
    },
    /// Mix a cue list and check it: overs with the sounds behind them and
    /// volumes that fix them, loudness each second and per section, buried
    /// cues, and a picture with a marker for each cue. The cue list is
    /// Duck Forge's log.json, or a JSON list with times in seconds. The
    /// same as `duck-echo do mix_check`.
    Mix {
        /// The cue list, a .json file inside the project folder.
        cues: String,
        /// The mix's length in seconds. Default: the end of the last sound.
        #[arg(long)]
        length: Option<f64>,
        /// Sections as JSON, seconds: '[{"name": "intro", "start": 0, "end": 12}]'.
        /// Default: every 10 s.
        #[arg(long)]
        sections: Option<String>,
        /// The folder for the mix, the report and the picture. Default:
        /// the cue list's folder.
        #[arg(short, long)]
        out: Option<String>,
        /// The folder the cues' files are named from. Default: for Duck
        /// Forge's log.json, the Duck Forge folder it is in.
        #[arg(long)]
        base: Option<String>,
        /// Print the whole report as JSON instead of a few lines.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        project: ProjectArgs,
    },
    /// Render a set of sounds (a folder or a set file) and write a table,
    /// a contact sheet and a loudness report per tag. The same as
    /// `duck-echo do set_check`.
    SetCheck {
        /// The set: a set file (film.set.toml) or a folder.
        set: String,
        /// The folder for the WAVs and the reports. Default checks/<name>.
        #[arg(short, long)]
        out: Option<String>,
        /// Print the whole report as JSON instead of a few lines.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        project: ProjectArgs,
    },
    /// Print the JSON Schema for recipes, songs (`schema song`) or sound
    /// sets (`schema set`).
    Schema {
        /// Which schema: `recipe` (default), `song` or `set`.
        #[arg(value_enum, default_value_t = SchemaKind::Recipe)]
        kind: SchemaKind,
    },
}

/// The project a command works in, shared by `do` and `run`.
#[derive(clap::Args)]
struct ProjectArgs {
    /// The project folder. Every file a command names is inside it.
    /// Default: the current folder.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Who the history says made the change.
    #[arg(long, value_enum, default_value_t = By::Person)]
    by: By,
}

/// Who made a change, for the Undo history.
#[derive(Clone, Copy, ValueEnum)]
enum By {
    /// A person typing commands.
    Person,
    /// An AI agent running commands.
    Agent,
}

impl From<By> for Who {
    fn from(b: By) -> Self {
        match b {
            By::Person => Who::Person,
            By::Agent => Who::Agent,
        }
    }
}

/// Extra measurements shared by `analyze` and `render`.
#[derive(clap::Args)]
struct ExtrasArgs {
    /// Also analyze each heard layer on its own, as heard in the mix
    /// (`layers` in the JSON). Recipes only.
    #[arg(long)]
    per_layer: bool,
    /// Also analyze each heard track on its own, as heard in the mix
    /// (`tracks` in the JSON). Songs only.
    #[arg(long)]
    per_track: bool,
    /// Hear only these (comma-separated): layer numbers from 0 for a
    /// recipe, track names for a song. With `render`, the WAV holds only
    /// these.
    #[arg(long, value_delimiter = ',')]
    solo: Vec<String>,
    /// Silence these (comma-separated): layer numbers for a recipe, track
    /// names for a song.
    #[arg(long, value_delimiter = ',')]
    mute: Vec<String>,
    /// List every written note of a song with what was heard for it
    /// (`note_check.tracks[].notes` in the JSON), not only the problems.
    /// Songs only.
    #[arg(long)]
    notes: bool,
    /// Draw a picture of the sound (level over time and a spectrogram) to
    /// this PNG file. A song's picture has a line at each bar.
    #[arg(long, value_name = "FILE.png")]
    picture: Option<PathBuf>,
}

impl ExtrasArgs {
    /// The options as the tools take them. `check_notes` says whether a
    /// song's written notes are listened for: `analyze` always does,
    /// `render` only when asked for per-track detail, because the check
    /// renders every track once more.
    fn options(&self, check_notes: bool) -> Options {
        Options {
            per_layer: self.per_layer,
            per_track: self.per_track,
            solo: self.solo.clone(),
            mute: self.mute.clone(),
            picture: self.picture.is_some(),
            check_notes,
            list_notes: self.notes,
        }
    }
}

/// Which schema to print.
#[derive(Clone, Copy, ValueEnum)]
enum SchemaKind {
    /// The schema of a sound recipe.
    Recipe,
    /// The schema of a song.
    Song,
    /// The schema of a sound set file (`set_check`).
    Set,
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

#[derive(Subcommand)]
enum InstrumentsAction {
    /// List the built-in instruments and kits with their range and
    /// description.
    List {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print an instrument's recipe (TOML with comments, or JSON).
    Show {
        /// Instrument name.
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
            quality,
            json,
            extras,
        } => {
            let format = duck_echo_tools::ogg::RenderFormat {
                wav: format.into(),
                quality,
            };
            render(&recipe, output, seed, format, json, &extras)
        }
        Command::Analyze {
            input,
            compact,
            extras,
        } => analyze(&input, compact, &extras),
        Command::Compare { a, b, compact } => {
            let a = duck_echo_tools::load_audio(&a).map_err(|e| e.at("a"));
            let b = duck_echo_tools::load_audio(&b).map_err(|e| e.at("b"));
            match a.and_then(|a| b.map(|b| duck_echo_tools::compare_buffers(&a, &b))) {
                Ok(c) => print_json(&c, compact),
                Err(e) => tool_error(e, true),
            }
        }
        Command::Presets { action } => presets(action),
        Command::Instruments { action } => instruments(action),
        Command::Variations {
            recipe,
            count,
            variation,
            output_dir,
            json,
        } => variations(&recipe, count, variation, output_dir.as_deref(), json),
        Command::Guide { topic } => match duck_echo_tools::guide::page(&topic, "topic") {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => tool_error(e, false),
        },
        Command::Commands { json } => list_commands(json),
        Command::Do {
            name,
            arguments,
            project,
        } => do_command(&name, &arguments, &project),
        Command::Run { script, project } => run_script(&script, &project),
        Command::Mix {
            cues,
            length,
            sections,
            out,
            base,
            json,
            project,
        } => {
            let mut args = serde_json::json!({ "cues": cues });
            if let Some(b) = base {
                args["base"] = b.into();
            }
            if let Some(l) = length {
                args["length_seconds"] = l.into();
            }
            if let Some(o) = out {
                args["out"] = o.into();
            }
            if let Some(text) = sections {
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(v) => args["sections"] = v,
                    Err(e) => {
                        return tool_error(
                            ToolError::issue(duck_echo_core::Issue::error(
                                duck_echo_core::Code::Syntax,
                                "--sections",
                                format!("the sections are not JSON: {e}"),
                            )),
                            json,
                        )
                    }
                }
            }
            check_command(
                "mix_check",
                &args,
                &project,
                json,
                &["mix_wav", "picture_png", "report_json"],
            )
        }
        Command::SetCheck {
            set,
            out,
            json,
            project,
        } => {
            let mut args = serde_json::json!({ "set": set });
            if let Some(o) = out {
                args["out"] = o.into();
            }
            check_command(
                "set_check",
                &args,
                &project,
                json,
                &["table_csv", "sheet_png", "check_json"],
            )
        }
        Command::Schema { kind } => {
            println!(
                "{}",
                match kind {
                    SchemaKind::Recipe => duck_echo_core::schema::recipe_schema_json(),
                    SchemaKind::Song => duck_echo_core::schema::song_schema_json(),
                    SchemaKind::Set =>
                        serde_json::to_string_pretty(&duck_echo_tools::sets::set_schema())
                            .expect("schema serializes"),
                }
            );
            ExitCode::SUCCESS
        }
    }
}

fn list_commands(json: bool) -> ExitCode {
    if json {
        let list: Vec<serde_json::Value> = commands::TABLE
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.name,
                    "description": commands::description(e),
                    "changes_files": e.changes_files,
                    "arguments": (e.schema)(),
                })
            })
            .collect();
        return print_json(&list, false);
    }
    for e in commands::TABLE {
        println!("{:<20} {}", e.name, e.description);
    }
    ExitCode::SUCCESS
}

/// The project folder of `do` and `run`.
fn open_root(args: &ProjectArgs) -> Result<Root, ToolError> {
    Root::new(&args.root).map_err(|e| {
        ToolError::issue(duck_echo_core::Issue::error(
            duck_echo_core::Code::NotFound,
            "--root",
            e,
        ))
    })
}

fn do_command(name: &str, arguments: &str, args: &ProjectArgs) -> ExitCode {
    let result = open_root(args).and_then(|root| {
        let value: serde_json::Value = serde_json::from_str(arguments).map_err(|e| {
            ToolError::issue(duck_echo_core::Issue::error(
                duck_echo_core::Code::Syntax,
                "arguments",
                format!("the arguments are not JSON: {e}"),
            ))
        })?;
        let command = commands::parse(name, &value, "arguments.")?;
        commands::run(
            &Project::new(&root),
            command,
            args.by.into(),
            None,
            "arguments.",
        )
    });
    match result {
        Ok(v) => print_json(&v, false),
        Err(e) => tool_error(e, true),
    }
}

/// Run a check command and print its summary and the files it wrote (or,
/// with `json`, its whole answer). Arguments are named as the command's
/// (`arguments.cues`), the same as `duck-echo do`.
fn check_command(
    name: &str,
    arguments: &serde_json::Value,
    args: &ProjectArgs,
    json: bool,
    files: &[&str],
) -> ExitCode {
    let result = open_root(args).and_then(|root| {
        let command = commands::parse(name, arguments, "arguments.")?;
        commands::run(
            &Project::new(&root),
            command,
            args.by.into(),
            None,
            "arguments.",
        )
    });
    match result {
        Ok(v) if json => print_json(&v, false),
        Ok(v) => {
            println!("{}", v["summary"].as_str().unwrap_or_default());
            for key in files {
                if let Some(f) = v[*key].as_str() {
                    println!("wrote {f}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => tool_error(e, json),
    }
}

fn run_script(script: &str, args: &ProjectArgs) -> ExitCode {
    let root = match open_root(args) {
        Ok(r) => r,
        Err(e) => return tool_error(e, true),
    };
    let list = root.read("script", script).and_then(|path| {
        let text = std::fs::read_to_string(&path).map_err(|e| {
            ToolError::failed(duck_echo_core::Code::Io, format!("cannot read {script}: {e}"))
                .at("script")
        })?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Array(list)) => Ok(list),
            Ok(_) => Err(ToolError::issue(duck_echo_core::Issue::error(
                duck_echo_core::Code::TypeMismatch,
                "script",
                "a script is a JSON list of commands: [{\"do\": \"file_new_sound\", \"file\": \"a.toml\"}]",
            ))),
            Err(e) => Err(ToolError::issue(duck_echo_core::Issue::error(
                duck_echo_core::Code::Syntax,
                "script",
                format!("the script is not JSON: {e}"),
            ))),
        }
    });
    let list = match list {
        Ok(l) => l,
        Err(e) => return tool_error(e, true),
    };
    let project = Project::new(&root);
    let mut results = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let prefix = format!("script[{i}].");
        let name = item.get("do").and_then(|v| v.as_str()).unwrap_or_default();
        let outcome = if name.is_empty() {
            Err(ToolError::issue(
                duck_echo_core::Issue::error(
                    duck_echo_core::Code::MissingField,
                    format!("{prefix}do"),
                    "each command needs `do`, the command's name",
                )
                .one_of(commands::names()),
            ))
        } else {
            commands::parse(name, item, &prefix)
                .and_then(|c| commands::run(&project, c, args.by.into(), None, &prefix))
        };
        match outcome {
            Ok(v) => results.push(v),
            Err(e) => {
                eprintln!(
                    "duck-echo run: command {i} ({name}) failed; the ones before it were done"
                );
                return tool_error(e, true);
            }
        }
    }
    print_json(
        &serde_json::json!({ "ok": true, "results": results }),
        false,
    )
}

fn validate(reference: &Path, json: bool) -> ExitCode {
    match duck_echo_tools::load_document(reference, &files_next_to(reference)) {
        Ok(doc) => {
            if json {
                println!("{}", doc.report().to_json());
            } else {
                eprint!("{}", doc.report());
            }
            ExitCode::SUCCESS
        }
        Err(e) => tool_error(e.at("recipe"), json),
    }
}

fn analyze(input: &Path, compact: bool, args: &ExtrasArgs) -> ExitCode {
    let options = args.options(true);
    let measured = if duck_echo_tools::is_recipe_ref(input) {
        duck_echo_tools::load_document(input, &files_next_to(input))
            .map_err(|e| e.at("input"))
            .and_then(|doc| Ok(measure::measure_document(&doc, None, &options, "--")?.measured))
    } else {
        options.for_wav("--").and_then(|extras| {
            duck_echo_tools::load_audio(input)
                .map_err(|e| e.at("input"))
                .and_then(|b| measure::measure_audio(&b, &extras))
        })
    };
    match measured {
        Ok(m) => {
            if let (Some(path), Some(png)) = (&args.picture, &m.png) {
                if let Err(e) = duck_echo_tools::write_png(path, png) {
                    return tool_error(e.at("--picture"), true);
                }
                eprintln!("wrote {}", path.display());
            }
            print_json(&m, compact)
        }
        Err(e) => tool_error(e, true),
    }
}

fn render(
    reference: &Path,
    output: Option<PathBuf>,
    seed: Option<u64>,
    format: duck_echo_tools::ogg::RenderFormat,
    json: bool,
    args: &ExtrasArgs,
) -> ExitCode {
    let doc = match duck_echo_tools::load_document(reference, &files_next_to(reference)) {
        Ok(doc) => doc,
        Err(e) => return tool_error(e.at("recipe"), json),
    };
    let output = output.unwrap_or_else(|| {
        if duck_echo_tools::is_recipe_path(reference) {
            reference.with_extension("wav")
        } else {
            PathBuf::from(format!("{}.wav", doc.stem()))
        }
    });
    let done = match duck_echo_tools::render_document_to_file(
        &doc,
        &output,
        seed,
        format,
        &args.options(args.per_track),
        "--",
    ) {
        Ok(d) => d,
        Err(e) => return tool_error(e.at("--output"), json),
    };
    if let (Some(path), Some(png)) = (&args.picture, &done.png) {
        if let Err(e) = duck_echo_tools::write_png(path, png) {
            return tool_error(e.at("--picture"), json);
        }
        eprintln!("wrote {}", path.display());
    }
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
        PresetsAction::List { tag, json } => {
            match duck_echo_tools::presets::list(tag.as_deref()).map_err(|e| e.at("--tag")) {
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
            }
        }
        PresetsAction::Show { name, json } => {
            match duck_echo_tools::presets::get(&name).map_err(|e| e.at("name")) {
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
            }
        }
    }
}

fn instruments(action: InstrumentsAction) -> ExitCode {
    use duck_echo_tools::instruments;
    match action {
        InstrumentsAction::List { json } => match instruments::list() {
            Ok(list) if json => print_json(&list, false),
            Ok(list) => {
                for i in list {
                    let range = match (&i.range, i.pieces.is_empty()) {
                        (Some([lo, hi]), _) => format!("{lo}..{hi}"),
                        (None, false) => i.pieces.join(","),
                        (None, true) => "drum".into(),
                    };
                    println!("{:<10} {:<20} {}", i.name, range, i.description);
                }
                ExitCode::SUCCESS
            }
            Err(e) => tool_error(e, json),
        },
        InstrumentsAction::Show { name, json } => {
            match instruments::get(&name).map_err(|e| e.at("name")) {
                Ok(i) if json => match instruments::load(i) {
                    Ok(l) => {
                        println!("{}", duck_echo_core::to_json(&l.recipe));
                        ExitCode::SUCCESS
                    }
                    Err(e) => tool_error(e, true),
                },
                Ok(i) => {
                    print!("{}", i.source);
                    ExitCode::SUCCESS
                }
                Err(e) => tool_error(e, json),
            }
        }
    }
}

fn variations(
    reference: &Path,
    count: u32,
    variation: Option<f32>,
    out_dir: Option<&Path>,
    json: bool,
) -> ExitCode {
    let result = duck_echo_tools::load_document(reference, &files_next_to(reference))
        .map_err(|e| e.at("recipe"))
        .and_then(|doc| match doc {
            Document::Recipe(r) => duck_echo_tools::variations(&r, count, variation, out_dir),
            Document::Song(_) => Err(ToolError::failed(
                duck_echo_core::Code::InvalidArgument,
                "variations works on a recipe (one sound); to hear a song with another seed, use `render --seed N`",
            )
            .at("recipe")),
        })
        .map_err(|e| e.at("--output-dir"));
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
    let code = match e {
        ToolError::Invalid(_) => 1,
        ToolError::Failed(_) => 2,
    };
    let report = e.into_report();
    if json {
        println!("{}", report.to_json());
    } else {
        eprint!("{report}");
    }
    ExitCode::from(code)
}
