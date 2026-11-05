# Duck Echo

An audio studio for AI agents and humans: retro sound effects now, music and
synthetic voices later, for games and videos. Sibling of the Duck Forge game
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

## Commands

| Command | What it does |
|---|---|
| `duck-echo validate <recipe> [--json]` | Reports every problem with its field path, allowed range and a suggestion. Exit 1 when invalid. |
| `duck-echo render <recipe> [-o out.wav] [--seed N] [--format pcm16\|pcm24\|float32] [--json]` | Renders to WAV. `--json` adds the analysis of the result. |
| `duck-echo analyze <wav-or-recipe> [--compact]` | JSON measurements headed by a one-line `summary`: duration, peak, RMS, loudness in LUFS with its method, clipping, silence, attack, decay, crest factor, brightness, tonality and a pitch track. A recipe is rendered in memory. See `docs/ANALYSIS.md`. |
| `duck-echo compare <a> <b> [--compact]` | What changed from A to B (loudness, brightness, length, attack, ...), waveform correlation, and both analyses. WAV files or recipes. |
| `duck-echo schema` | The JSON Schema for recipes, with descriptions and defaults. |

Recipes are TOML on disk and JSON in tool calls; both are the same schema.

## Documents

* `docs/RECIPE_SPEC.md`: every field, default and range, plus examples.
* `docs/ANALYSIS.md`: every `analyze` and `compare` field and how it is measured.
* `docs/ARCHITECTURE.md`: crates, data flow, determinism, realtime rules,
  the planned MCP tools and the Duck Ear bridge.
* `docs/ROADMAP.md`: ordered tasks for the next builder.
* `schema/recipe.schema.json`: generated schema for agents.

## Layout

```
crates/duck-echo-core     recipe types, loading, validation, schema
crates/duck-echo-audio    AudioBuffer and WAV I/O
crates/duck-echo-synth    deterministic renderer
crates/duck-echo-analyze  measurements and compare
crates/duck-echo-tools    operations shared by the CLI and the MCP server
crates/duck-echo-cli      the duck-echo binary and end-to-end tests
examples/recipes          coin, jump, laser, explosion, hit, powerup,
                          ricochet (stereo), chime (note names)
```

## Development

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets && cargo fmt --all -- --check
```

After changing the recipe types, regenerate the checked-in schema (a test
fails when it is stale):

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```
