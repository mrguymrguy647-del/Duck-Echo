# Duck Echo, for AI agents

Duck Echo makes sound effects and music for games and videos from text.
A **recipe** (a TOML or JSON file) describes one sound; a **song** is
tracks of notes, each played by an instrument, which is a recipe too.
`duck-echo` checks them, renders them to WAV and measures the result in
words and numbers, so you can work without hearing. The same recipe and
seed always give the same bytes. Duck Echo Studio is the app for people.

## Making sounds with it

Start with the guide: one page on making sounds and music without ears.

```bash
duck-echo guide                    # read this first
duck-echo guide recipe_spec        # every field; also song_spec, analysis, instruments, presets
duck-echo presets list             # 30 sounds to start from (instruments list for music)
duck-echo do file_copy_from '{"from": "preset:coin", "to": "sfx/coin.toml"}' --by agent
duck-echo do sound_set '{"file": "sfx/coin.toml", "path": "layers[0].pitch.start", "value": "C6"}' --by agent
duck-echo validate sfx/coin.toml --json        # every problem, with its field and a suggestion
duck-echo render sfx/coin.toml -o sfx/coin.wav --json   # the WAV, and its analysis
duck-echo compare preset:coin sfx/coin.toml    # what changed, in plain words
duck-echo do edit_undo             # take the last step back
```

From the source, `cargo run -p duck-echo-cli -- <command>` runs the same
program. `duck-echo commands` lists everything `duck-echo do` can do.

In an app that speaks MCP, `duck-echo-mcp --root <folder>` gives the same
as tools (`get_guide`, `list_presets`, `create`, `edit`, `render`,
`analyze`, `compare`, and one for each of `duck-echo commands`), with
every file inside the folder. Build it with
`cargo build --release -p duck-echo-mcp`, then add it, for example
`claude mcp add duck-echo -- /path/to/target/release/duck-echo-mcp --root .`
or, in Codex's `~/.codex/config.toml`, `[mcp_servers.duck-echo]` with
`command` and `args = ["--root", "."]`.

The rules:

- **Never write a sound from nothing; change one thing at a time.** Start
  from a preset, an instrument or an example song, render, read the
  numbers, compare, then keep the change or undo it.
- **Change files with `duck-echo do` or the MCP tools**, with
  `--by agent`. They keep a person's comments and layout, and each change
  is one step of the Undo history a person in the Studio shares.
- `preset:NAME` and `instrument:NAME` work anywhere a file is expected.
- With `--json`, stdout holds one JSON document, failures too. Each error
  has a `code`, a `path` and often `allowed` and a `suggestion`.
- Exit codes: 0 worked, 1 the recipe or song has mistakes (or could not be
  read), 2 the command was wrong or something else failed.

## Working on the code

A Rust workspace (edition 2021; its locked libraries need Rust 1.85+):

| Crate | What it is |
|---|---|
| `crates/duck-echo-core` | Recipe and song types, loading, checking, the JSON Schema |
| `crates/duck-echo-audio` | The sound buffer and WAV files |
| `crates/duck-echo-synth` | The renderer: the same bytes on every machine |
| `crates/duck-echo-analyze` | Measurements, pictures and `compare` |
| `crates/duck-echo-tools` | All the work the CLI, the MCP server and the Studio share |
| `crates/duck-echo-live` | Playing through the speakers, live |
| `crates/duck-echo-cli` | The `duck-echo` program, and end-to-end tests |
| `crates/duck-echo-mcp` | The `duck-echo-mcp` program: the MCP server over stdio |

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p duck-echo-cli && scripts/render-fingerprints.sh   # hash every render
```

**Before a commit:** the tests, `cargo fmt --all` and clippy with
`-D warnings`, all clean. CI runs them on Linux and Windows, and checks
that both render every example and preset to the same bytes.

**With no internet**: once `cargo fetch` has run,
`cargo build --offline --release -p duck-echo-cli -p duck-echo-mcp`
builds both programs with nothing downloaded. No system libraries needed.

**Duck Echo Studio** is its own workspace in `studio/` (Rust 1.95). On
Linux it needs `sudo apt-get install libasound2-dev pkg-config` first.
Its tests start the real MCP server, so build that here first
(`cargo build -p duck-echo-mcp`), then
`cd studio && cargo test --workspace -- --test-threads 1`. Start it with
`cd studio && cargo run --release`.

Read `docs/ARCHITECTURE.md`, `docs/RECIPE_SPEC.md` and `docs/ROADMAP.md`
before changing anything. The rules:

- The recipe is the source of truth. Never add behaviour that is not in
  the recipe or in a documented CLI or tool option.
- Renders are deterministic: same recipe and seed, identical bytes. No
  wall clock, no global state, no hash-map iteration, no platform libm on
  the audio path (use `duck_echo_synth::math`).
- TOML files and JSON tool calls read into the same Rust types: one
  schema. Unknown fields are errors, never ignored.
- Every error carries the exact field path and, where it can, the allowed
  range or options and a suggestion. Agents read errors as JSON.
- Nothing is silently deferred: a field in the schema works, and one that
  does not yet is left out and listed under Deferred in `docs/RECIPE_SPEC.md`.
- `Voice::process` stays free of allocation and locks.
- Keep dependencies few; justify each new one in `docs/ARCHITECTURE.md`.
- Never make voice cloning or anything that imitates a real person.
- Add behaviour in `crates/duck-echo-tools`, which the CLI and the MCP
  server share, not in one front end only.
- Every example and preset render is pinned by a fingerprint
  (`crates/duck-echo-cli/tests/vertical_slice.rs`,
  `crates/duck-echo-tools/tests/presets.rs`). Change one only when the
  sound is meant to change, and say so in the commit.
- After changing the recipe types, run
  `cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json` and
  update `docs/RECIPE_SPEC.md`; for the song types, the same with
  `schema song > schema/song.schema.json`. A test fails while either is stale.
- `docs/AGENT_GUIDE.md` and other docs are built in as `duck-echo guide` pages.
- New features get an example recipe or a test that renders it.
- Keep `docs/ROADMAP.md` current: tick off what you did, and add what the
  next builder needs to know.
