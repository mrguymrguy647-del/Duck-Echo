# Duck Echo

An audio studio for AI agents and humans: retro sound effects and music
now, synthetic voices later, for games and videos. Sibling of the Duck Forge game
engine.

Every sound is a text **recipe**. The recipe is the source of truth; audio
is its output. Renders are deterministic: same recipe plus same seed gives
byte-identical WAV files.

## Quick start

```bash
cargo build --release
```

```bash
./target/release/duck-echo validate examples/recipes/coin.toml
```

```bash
./target/release/duck-echo render examples/recipes/coin.toml -o coin.wav
```

```bash
./target/release/duck-echo analyze coin.wav
```

A recipe can be tiny:

```toml
version = 0
[[layers]]
source = "square"
[layers.pitch]
start = "A5"        # or a number in Hz, such as 880
end = "A6"
curve = "exponential"
```

A **song** is music as text: tracks of notes, each played by an
instrument, and an instrument is a recipe. The same commands take both:

```bash
./target/release/duck-echo render examples/songs/four_bars.toml -o four_bars.wav
```

```bash
./target/release/duck-echo analyze examples/songs/four_bars.toml --per-track
```

## Commands

| Command | What it does |
|---|---|
| `duck-echo validate <recipe> [--json]` | Reports every problem with its field path, allowed range and a suggestion. Exit 1 when invalid. |
| `duck-echo render <recipe> [-o out.wav] [--seed N] [--format pcm16\|pcm24\|float32] [--json]` | Renders to WAV. `--json` adds the analysis of the result. Takes the same `--per-layer`, `--solo`, `--mute` and `--picture` options as `analyze`; with `--solo` or `--mute` the WAV holds only the heard layers. |
| `duck-echo analyze <wav-or-recipe> [--compact] [--per-layer] [--solo 0,2] [--mute 1] [--picture out.png]` | JSON measurements headed by a one-line `summary`: duration, peak, RMS, loudness in LUFS with its method, clipping, silence, attack, decay, crest factor, brightness, tonality, a pitch track, energy over time and note onsets. A recipe is rendered in memory. `--per-layer` also analyzes each layer as heard in the mix; `--solo`/`--mute` hear only some layers; `--picture` draws level over time above a spectrogram. See `docs/ANALYSIS.md`. |
| `duck-echo compare <a> <b> [--compact]` | What changed from A to B (loudness, brightness, length, attack, ...), waveform correlation, and both analyses. WAV files or recipes. |
| `duck-echo presets list [--tag T] [--json]` | The 30 built-in presets with category, loudness target and description. |
| `duck-echo presets show <name> [--json]` | A preset's recipe, to copy and edit. |
| `duck-echo variations <recipe> [--count N] [--variation V] [-o dir] [--json]` | Render seeds 1..N and print one analysis summary each, to pick a variant by its numbers. |
| `duck-echo schema [song]` | The JSON Schema for recipes, or for songs, with descriptions and defaults. |

`validate`, `render`, `analyze` and `compare` also take a song (a document
with `tracks`). For a song, `--per-track` analyzes each track alone and
`--solo`/`--mute` take track names; `render --solo bass` writes that
track's stem, which lines up with the full mix in time and level.

Recipes and songs are TOML on disk and JSON in tool calls; both are the same schema.
Anywhere a recipe is expected, `preset:NAME` names a built-in preset, for
example `duck-echo render preset:coin -o coin.wav`.

## Use it from an agent (MCP)

`duck-echo-mcp` serves the Duck Echo tools over the Model Context Protocol:
`get_schema`, `list_presets`, `get_preset`, `list_files`, `validate`,
`create`, `edit`, `render`, `analyze`, `compare` and `variations`. File
paths are relative to `--root` and cannot leave it: absolute paths, `..`
and symbolic links that lead out are refused, and `render` only writes
`.wav` files. `render` and `analyze` can return a picture of the sound as
an MCP image (`picture: true`) and analyze layers on their own
(`per_layer`, `solo`, `mute`).

### Playtest in this repository (project `.mcp.json`)

The repository ships a project-scoped `.mcp.json` that starts the server
for anyone who opens Claude Code here. It runs
`cargo run --release -q -p duck-echo-mcp -- --root <repo>/playtest`, so
the server is built when needed, and every file the tools write lands in
`playtest/`. Playtest results are committed: give each playtest its own
folder (for example `playtest/2-shooter/`, with the recipes, the WAVs and a
`REVIEW.md`), so playtests do not overwrite each other. The repository's
`*.wav` ignore rule does not apply under `playtest/`, so renders can be
pushed with their recipes. The first playtest's results and review are on
the `playtest-1` branch.

1. Build once before the first session:

   ```bash
   cargo build --release -p duck-echo-mcp
   ```

   A cold release build takes about a minute, longer than Claude Code's
   default 30-second server start timeout. Without this step the first
   start can time out. After the build, starting takes well under a
   second. To let the first start build instead, raise the timeout, for
   example to two minutes:

   ```bash
   MCP_TIMEOUT=120000 claude
   ```

2. Start Claude Code from the repository root and approve the `duck-echo`
   server when asked. Project servers need approval once. Check them with
   `/mcp` in a session, or with:

   ```bash
   claude mcp list
   ```

Only the MCP protocol is written to stdout. Cargo's build output and the
server's log line go to stderr, and the server stops with a message on
stderr if `playtest/` cannot be found (for example when Claude Code is
started from a subdirectory).

### Your own projects

```bash
cargo build --release
```

Claude Code, from the folder where sounds should live:

```bash
claude mcp add duck-echo -- /path/to/Duck-Echo/target/release/duck-echo-mcp --root .
```

Claude Desktop, in `claude_desktop_config.json` (absolute paths):

```json
{
  "mcpServers": {
    "duck-echo": {
      "command": "/path/to/Duck-Echo/target/release/duck-echo-mcp",
      "args": ["--root", "/path/to/my-game/sounds"]
    }
  }
}
```

A typical agent loop: `list_presets`, `get_preset`, `create` a file from it,
`edit` fields by path (`master.normalize_lufs` to the preset's
`target_lufs` keeps its level through edits), `render`, then `analyze` or
`compare` to check the result by numbers, and `list_files` to see what is
there. A failed call sets `isError` and returns a report whose errors each
have a code and a path (a recipe field or the argument at fault). Tool
details: `docs/ARCHITECTURE.md`.

## Documents

* `docs/RECIPE_SPEC.md`: every field, default and range, plus examples,
  for recipes and for songs.
* `docs/phase1-design.md`: why songs work the way they do.
* `docs/ANALYSIS.md`: every `analyze` and `compare` field and how it is measured.
* `docs/ARCHITECTURE.md`: crates, data flow, determinism, realtime rules,
  the planned MCP tools and the Duck Ear bridge.
* `docs/ROADMAP.md`: ordered tasks for the next builder.
* `schema/recipe.schema.json`, `schema/song.schema.json`: generated
  schemas for agents.

## Layout

```
crates/duck-echo-core     recipe types, loading, validation, schema
crates/duck-echo-audio    AudioBuffer and WAV I/O
crates/duck-echo-synth    deterministic renderer
crates/duck-echo-analyze  measurements and compare
crates/duck-echo-tools    operations shared by the CLI and the MCP server
crates/duck-echo-cli      the duck-echo binary and end-to-end tests
crates/duck-echo-mcp      the duck-echo-mcp binary: MCP server over stdio
presets                   30 tagged starting points (see presets/README.md)
playtest                  root for the project MCP server (.mcp.json); one
                          committed folder per playtest
examples/songs            four_bars (two tracks, instruments written inline)
examples/recipes          coin, jump, laser, explosion, hit, powerup,
                          ricochet (stereo), chime (note names), fanfare
                          (layers, normalize_lufs)
```

## Development

CI runs the tests on Linux and Windows and checks that both render every
example and preset to identical bytes (see "What is guaranteed to match"
in `docs/ARCHITECTURE.md`). To fingerprint the renders on your machine:

```bash
cargo build --release -p duck-echo-cli && scripts/render-fingerprints.sh
```

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets && cargo fmt --all -- --check
```

After changing the recipe or song types, regenerate the checked-in schemas
(a test fails when one is stale):

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```

```bash
cargo run -p duck-echo-cli -- schema song > schema/song.schema.json
```
