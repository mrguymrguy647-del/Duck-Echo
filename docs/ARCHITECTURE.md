# Duck Echo architecture

Duck Echo is an audio studio for AI agents and humans: sound effects first,
then music, then synthetic voices, for games and videos. It is the sibling of
the Duck Forge game engine and the art app. Agents are first-class users, so
everything is designed to be written, read, verified and fixed from text,
without hearing the result.

## Principles

1. **The recipe is the source of truth.** Audio is an output. Any sound can
   be reproduced, diffed, versioned and edited as text.
2. **One schema, two syntaxes.** TOML on disk, JSON in tool calls. Both
   deserialize into the same Rust types through serde, so there is exactly
   one set of fields, defaults and ranges.
3. **Deterministic renders.** Same recipe plus same seed gives byte-identical
   audio. No wall clock, no global state, no platform math on the audio path.
4. **Errors are data.** Every problem has a field path, a stable code, the
   allowed range or options, and a suggestion when one is obvious.
5. **Numbers instead of ears.** Analysis turns audio into measurements an
   agent can assert on. Later, the Duck Ear adds learned listening.
6. **Small core.** Few dependencies, no DSP frameworks, plain Rust.

## Workspace layout

```
Cargo.toml                 workspace: shared versions, lints, profiles
crates/
  duck-echo-core/          recipe and song types, defaults, TOML/JSON loading,
                           validation, diagnostics, JSON Schemas. No audio code.
  duck-echo-audio/         AudioBuffer type and WAV read/write (hound behind a
                           `wav` feature so synth can depend on just the type).
  duck-echo-synth/         deterministic renderer: Recipe -> Voice -> samples,
                           and the song player (one Voice per note).
  duck-echo-analyze/       measurements over f32 samples -> JSON report, plus
                           compare (deltas and cross-correlation), onsets and
                           energy over time, a picture (spectrogram and level)
                           as PNG, and its own small FFT and PNG encoder.
  duck-echo-tools/         operations shared by the CLI and the MCP server:
                           load a WAV or render a recipe, compare, presets,
                           variations, edits, analysis extras (per layer,
                           solo, mute, picture) and file listings.
  duck-echo-cli/           the `duck-echo` binary and the end-to-end tests.
  duck-echo-mcp/           the `duck-echo-mcp` binary: the agent tools as an MCP
                           server over stdio.
  duck-echo-ear/           (planned) bridge to the Python Duck Ear process.
docs/                      this file, RECIPE_SPEC.md, ANALYSIS.md, ROADMAP.md
examples/recipes/          the example recipes (TOML, plus coin.json)
examples/songs/            the example songs
presets/                   tagged starting-point recipes, compiled into the
                           tools crate with include_str!
instruments/               the starter instruments for songs, compiled in the
                           same way (see instruments/README.md)
schema/recipe.schema.json  generated from the Rust types; a test fails when stale
schema/song.schema.json    the same, for songs
```

Dependency direction (arrows point at what is depended on):

```
cli ──> tools ──> synth ──> core
 │        │        └────> audio (buffer type only)
 │        ├────> analyze
 │        ├────> audio (with wav feature)
 │        └────> core
 └────> core, synth, analyze, audio (directly, for render and validate)
mcp ──> tools (and core, audio)
ear (planned) ──> core, audio, analyze
```

`core` never depends on anything audio-related; `synth` and `analyze` never
touch files. That keeps every piece testable in isolation and lets the MCP
server and a future UI reuse the libraries rather than shelling out.

## Data flow

```
 recipe.toml / JSON tool call
        │
        ▼
 core::tree      syntax: TOML or JSON -> positioned tree (stops on a syntax error)
        │
        ▼
 core::structure walk the tree against the generated JSON Schema: every
        │        unknown field, bad enum value, missing field and type
        │        mismatch at once; prune them
        ▼
 typed decode    clean documents straight from the source with serde;
        │        documents with structural errors from the pruned tree
        ▼
 core::validate  range and rule checks, all collected, warnings separate;
        │        issues under an already-reported path are dropped
        │        -> Issue { code, path, message, got, allowed, suggestion, line }
        ▼
 synth::prepare  variation jitter from seed, lengths, allocations -> Voice
        │
        ▼
 Voice::process  block by block, allocation-free, f32 (interleaved if stereo)
        │
        ▼
 synth::render   offline wrapper: normalize by peak or loudness (optional),
        │        clip, trim -> AudioBuffer (render_mastered also returns the
        │        gain and length, so render_layers can finish one layer
        │        exactly like the mix)
        │
        ├──> audio::write_wav  (16/24-bit PCM or 32-bit float)
        │
        ▼
 analyze::analyze  summary line, duration, peak, RMS, loudness (gated
        │          BS.1770 from 400 ms, ungated K-weighted below, method
        │          named), clipping, silence, DC, crest factor, attack,
        │          decay, spectral centroid, tonality, pitch track,
        │          energy over time, onsets (all defined in ANALYSIS.md);
        │          picture::draw for a PNG of level and spectrogram
        │
 analyze::compare  two sounds -> both analyses, b-minus-a deltas, waveform
        │          cross-correlation, summary line
        │
        ▼
 JSON report for the agent (or text for a human)
```

Per layer, the signal path inside `Voice::process` is:

```
oscillator (pitch curve: sweep + arpeggio + vibrato;
            square and saw add a BLEP correction at each jump unless
            the layer says aliasing = "retro")
  × envelope × volume
  → filter (biquad, cutoff sweep updated every 16 samples)
  → layer effects (in order), still mono
  → [stereo: equal-power pan] summed into the mix at `offset`
master: effects (in order, one state per channel) → gain
        → [offline: normalize or normalize_lufs → clip → trim_silence]
```

`normalize_lufs` measures loudness on the audio path, so the meter there
(`synth::loudness`) is written for determinism: `f64` with `+ - * /` and
`sqrt` only, the K-filter's `tan` as a Taylor series, the constants that
need `pow` written out, and gates compared as mean squares instead of LUFS.
It measures the finished render (scaled, clipped and trimmed, as written),
correcting the gain over up to four passes, because trimming and the
short-clip active span depend on the gain. The analyzer's `f64`/libm meter
is the reference; a test checks the two agree within 0.001 LU on every
preset and example.

Solo, mute and per-layer analysis render the recipe with the other layers
removed from the `Voice` after it is prepared (`Voice::keep_layers`), so
every layer keeps its seed stream, `variation` jitter and timing, and the
length stays the mix's. `render_layers` then applies the full mix's
normalization gain and cuts or pads to the mix's trimmed length, so a layer
reads at the level and times it has in the mix.

Lengths are computed, never guessed: each layer lasts `offset + envelope
length + layer effect tails`, the recipe lasts the longest layer plus the
master effect tails, unless `duration` overrides it. Effect tails are derived
with multiplication only so every platform agrees on the frame count.

## Determinism

* All randomness comes from the recipe `seed` through SplitMix64. Each layer
  gets its own forked stream, so adding a layer does not change the noise of
  the others. `variation` draws its jitter from a separate stream.
* The audio path never calls platform libm. `synth::math` provides
  polynomial `sin`, `exp2`, `log2` and `pow` built from `+ - * /`, `floor`,
  `sqrt` and bit casts, all of which are IEEE-754 exact. Rust does not
  contract `a * b + c` into a fused multiply-add without explicit opt-in, so
  results match across x86 and ARM.
* No threads in the renderer, no iteration over hash maps, no time.
* The golden tests pin a fingerprint of every example's and preset's WAV,
  made by a release build; the tests run unoptimized, so they also check
  that the optimization level does not change a single byte.

The analysis crate uses `f64` and platform libm freely: it measures, it does
not produce audio, and differences at the fifth decimal do not matter there.

### What is guaranteed to match

CI (`.github/workflows/ci.yml`) runs every test on Linux and on Windows,
then renders every example and every preset on both with
`scripts/render-fingerprints.sh` and checks that the SHA-256 of every WAV
is the same on both. In plain words:

* **Same recipe, same file.** With the same Duck Echo version, a recipe
  (with its seed) renders to the same WAV bytes on Linux and Windows
  (x86-64), in a release or a debug build. This is checked for every
  example and preset on each push, and it holds for any recipe by design:
  the render path uses only operations that IEEE-754 defines exactly.
* **A sound only changes on purpose.** Every example and preset is pinned;
  a commit that changes one says so.
* **Not promised to match:** analysis numbers (`analyze`, `compare`) may
  differ in the last decimals between platforms, because they use the
  platform's math library; pictures are drawn from those numbers. macOS
  and ARM machines are not in CI yet; nothing in the design should differ
  there, but it is not checked. A different Duck Echo version may change a
  sound, and its commit will say which.

## Realtime constraints (for later phases)

Even though this phase renders offline, the renderer is already shaped for a
realtime audio thread:

* `prepare(recipe) -> Voice` does all validation, planning and allocation.
* `Voice::process(&mut [f32])` allocates nothing, takes no locks, does no I/O
  and never reads the recipe again. Effects own fixed-size delay lines made
  in `prepare`. The only per-block state is a few counters and the filter
  coefficients.
* The offline `render` is just `process` in a loop plus the offline-only
  `normalize` step, which needs the whole signal.

Rules for whoever adds playback: build a `Voice` on the control thread, move
it to the audio thread through a lock-free channel or a triple buffer, and
never share it. Parameter changes create a new `Voice` (or a small
`ParamUpdate` message with plain numbers). The audio thread must clip its own
output since `process` does not. Keep `RENDER_BLOCK` as the upper bound of
scratch memory.

## How music and voice will extend the recipe model

The schema is versioned and every optional field has a default, so new
features are added as new optional fields or new enum variants. Existing
recipes keep rendering identically. The `version` number only increments when
the meaning of an existing field changes.

### Music (roadmap Phase 1, songs are built)

A song is a second document type that plays sound recipes as instruments.
The recipe itself did not change. `docs/RECIPE_SPEC.md` ("Songs") is the
reference; `docs/phase1-design.md` explains the choices. In short:

* **Types** (`core::song`): `Song`, `Track`, `Note`, `Pattern`,
  `Placement`. `core::arrange` checks a song and flattens it into plain
  `Event`s per track (patterns placed, chords split, bars turned into
  beats). The renderer and the analyzer both read that list, so they agree
  on what was written. Instruments the song does not define itself come
  through the `InstrumentSource` trait, which the front ends supply,
  because only they know where files may be read.
* **One document loader.** `core::load` is generic over the two document
  types; `document_kind` tells them apart by their top-level fields
  (`tracks` means song). `tools::load_document` is the one entry point the
  CLI and the MCP server use, so every command works on both kinds.
* **A note is a `Voice`.** `synth::prepare_note` builds the same `Voice`
  as a sound effect, with three differences: every layer's pitch is
  multiplied by `note / home`, the envelope is told when the note is let
  go (`EnvShape::gated`), and the recipe's master effects are left out.
  `Voice::process` is unchanged and still allocation-free.
* **The song player** (`synth::song::SongPlayer`) works block by block. Per
  track it sums the note voices, runs the instrument's effects once, and
  applies the instrument's level, then the track's volume and pan. It
  makes a new `Voice` when a note starts, so the player itself is not
  allocation-free yet; live playback (Phase 2) needs a pool of voices made
  ahead of time. The shape (a sorted note list worked through in blocks)
  stays.
* **Seeds.** Track stream = song seed forked by a fixed hash (FNV-1a) of
  the track **name**; note stream = track stream forked by the note's
  **start sample**, then by its place among notes starting together, then
  by the instrument's own seed. Nothing depends on position in the file.
* **Built-in instruments** (`tools::instruments`) are recipes in
  `instruments/`, compiled in like the presets. `tools::song::Library` is
  the `InstrumentSource` both front ends use: it finds built-in
  instruments and kits by name, `preset:NAME`, and recipe files (through
  the front end's file resolver). `instrument:NAME` names one wherever a
  recipe path is accepted; in the MCP server `Root::read` checks it like
  `preset:NAME` (a known name, read-only, never a path).
* **Loops.** For `loop = true`, `synth::song` runs the player to the end
  and folds everything after the song's last bar back onto the start
  (`wrap_loop`), then masters. This is an offline step; the player itself
  does not loop.
* **Stems.** `synth::song::render_song_tracks` renders some tracks with
  the full mix's master gain and length (the `render_layers` rule), so
  stems add up to the mix.

### Voice (roadmap Phase 5)

Formant synthesis adds a `formant` source and an optional
`[layers.formant]` table:

```toml
[[layers]]
source = "formant"
[layers.formant]
glottal = "pulse"                 # excitation: pulse, noise, mixed
vowels = ["a", "i", "u"]           # or explicit f1/f2/f3 targets in Hz
vowel_time = 0.12
bandwidths = [80, 90, 120]
```

Internally this is the existing pitch curve driving a glottal pulse train
through three or four sweeping bandpass filters, so it reuses `Sweep`,
`Biquad` and the envelope. Gibberish and robot voices are sequences of vowel
targets with pitch curves; invented natural voices later add consonant
bursts (short noise layers, which already exist). No recorded voices are
ever used, so there is nothing to clone.

### Sample layers (later)

A `sample` source with `[layers.sample] path = "..."` plays a short
recording through the same envelope, pitch (as playback rate), filter and
effects. The hash of the file goes into the recipe so determinism holds.

## Agent tools: the MCP server

`duck-echo-mcp` serves these tools over stdio. All take and return JSON
objects; the work is done by `duck-echo-tools`, the same code the CLI uses.
A recipe argument is either `recipe` (an inline JSON object) or `path` (a
`.toml`/`.json` file, or `preset:NAME`). Wherever the table says "recipe",
a song works too: the tools tell the two apart by looking inside.

| Tool | Input | Output |
|---|---|---|
| `get_schema` | optional `kind` (`recipe` or `song`) | The recipe JSON Schema, same as `duck-echo schema`, or the song schema (`duck-echo schema song`). |
| `list_presets` | optional `tag` | `{presets: [{name, category, target_lufs, description, tags}]}`. Presets are levelled to per-category loudness targets (see `presets/README.md`). A tag no preset has is a `not_found` error at `arguments.tag` with the closest real tag as `suggestion` and every tag in `allowed`, so a typo is never an empty list. |
| `get_preset` | `name` | `{name, category, description, tags, recipe, toml}`: the recipe as a JSON object and its TOML source with comments. |
| `list_instruments` | none | `{instruments: [{name, kind, description, tags, home, range, target_lufs, pieces}]}`: the built-in instruments and the kit `drums`. |
| `get_instrument` | `name` | The listing entry plus `recipe` (a JSON object) and `toml` (its source with comments). A kit is an error that names its pieces. |
| `list_files` | optional `path` (a folder), `recursive` (default true) | `{folder, entries: [{path, kind, bytes, seconds}], truncated}`. Kinds: `folder`, `recipe`, `audio`, `image`, `link`, `other`; `seconds` for WAV files. Hidden files are skipped, links are not followed, at most 500 entries. |
| `validate` | recipe | The `Report`. An invalid recipe is a normal answer here, not a tool error. |
| `create` | `path`, `recipe`, optional `overwrite` | `{ok, path, kind, warnings}` (`kind` is `recipe` or `song`). Validates first; writes TOML or JSON by extension; refuses to replace a file unless `overwrite` is true. |
| `edit` | recipe, `ops`, optional `write` | `{ok, recipe, warnings, written, path}`. Each op is `{path, value}` or `{path, remove: true}`, with paths as in error messages; an index equal to the length appends. Edits apply to the document as written (defaults stay implicit), and a file is rewritten only when the result is valid. Comments are not kept. |
| `render` | recipe, `output`, optional `seed`, `format`, extras | Same as `duck-echo render --json`: `{ok, output, seed, warnings, analysis}`, plus `heard_layers`, `layers` and `picture` when asked. `warnings` adds a render warning when `normalize_lufs` hit the peak ceiling. A song adds `song` (`tempo`, `beats_per_bar`, `bars`, `loop`, `song_frames`, `seconds_per_bar`, `tracks`, `notes`), `heard_tracks` and `tracks` when asked, and for a loop `loop_seam` (`seam_step`, `largest_step`, `clean`). |
| `analyze` | `path` (WAV, recipe or preset) or `recipe`, extras | The `Analysis` (see `ANALYSIS.md`), plus `warnings` (recipes only, when there are any), `heard_layers`, `layers` and `picture` when asked. |
| `compare` | `a`, `b`: each a path or a recipe object | The `Comparison`. |
| `variations` | recipe (not a song), optional `count`, `variation`, `output_dir` | `{variation, seeds_matter, note, renders: [{seed, output, summary, analysis}]}`. |
| `export` | (task 13) | Not built: the Duck Forge format is still to be agreed. |

Extras on `render` and `analyze`, shared with the CLI's `--per-layer`,
`--solo`, `--mute` and `--picture`: `picture: true` adds a PNG as an MCP
image block (`{"type": "image", "mimeType": "image/png", "data": base64}`)
after the text, and a `picture` object in the result describing its axes;
`per_layer: true` adds `layers: [{index, name, analysis}]`, each heard layer
alone as heard in the mix; `solo` and `mute` (layer indices) choose the
heard layers, reported as `heard_layers`. For a song, `per_track: true`
adds `tracks: [{index, name, instrument, notes, analysis}]`, and `solo` and
`mute` take track names, reported as `heard_tracks`; a track heard alone is
a stem, finished with the full song's master gain. Layer extras need a
recipe and track extras a song; the wrong kind, or a WAV file, is an
`invalid_argument` error. The tool descriptions name the two
loudness methods.

Rules the server keeps:

* **One error shape.** A failed call sets `isError` and returns the
  validation `Report` shape, whether the recipe was invalid, an argument was
  misspelled (with a suggestion, at path `arguments.<name>`), or a file was
  missing. Line numbers are dropped for inline recipes, since they would
  refer to text the agent never wrote. `validate` is the one tool whose
  answer is a report, so an invalid recipe there is `isError: false` with
  `ok: false`; it is `isError: true` only when the call itself fails.
* **Every error has a code and a path.** Codes are specific (see
  Diagnostics); there is no catch-all. The path is a recipe field
  (`layers[0].volume`) or the argument at fault (`arguments.output`,
  `arguments.solo[1]`); a whole-document problem in a file, such as a
  syntax error, is at `arguments.path` with its line. `duck-echo-tools`
  returns file and argument failures with an empty path and each front end
  names its argument (`ToolError::at`); the server fills any path still
  empty with `arguments`, and a test calls every tool wrongly to check.
* **Messages fit the front end.** Shared errors name no CLI command (a
  missing preset lists every name in `allowed` instead), a missing file
  suggests the closest name in its folder and points at `list_files`, and
  every message has the root's absolute location replaced by paths
  relative to it.
* **Results as data.** Every result is a JSON object, sent both as
  `structuredContent` and as text.
* **Paths stay inside the root.** File paths are relative to `--root`
  (default: the working directory). Absolute paths and `..` are refused;
  symbolic links are resolved and refused if they lead out of the root or
  are broken. `preset:NAME` is accepted only where a recipe is read, and
  only for a known preset name; it can never be a write target. `render`
  writes only `.wav` files, so it cannot overwrite other files; recipe
  names are made file-safe before they become file names. Results show
  paths relative to the root. All of this lives in
  `crates/duck-echo-mcp/src/root.rs`, and stdio tests try each escape.
* **Never silently.** Edits that cannot apply say which op and why; an
  invalid result changes no file.

**Protocol.** MCP over stdio is newline-delimited JSON-RPC 2.0, and Duck
Echo needs only `initialize`, `ping`, `tools/list` and `tools/call`. The
server implements these directly on `serde_json`
(`crates/duck-echo-mcp/src/protocol.rs`, about 150 lines) instead of using
the official Rust SDK, which is async and would bring tokio and a large
dependency tree into a synchronous program. It negotiates protocol versions
2024-11-05 through 2025-11-25. If Duck Echo later needs resources, prompts,
progress notifications or HTTP transport, replacing that one file with the
SDK is the way; the tools module does not depend on it.

## Duck Ear bridge (design)

The Duck Ear is a Python process running audio-understanding models. It is
optional: every feature of Duck Echo works without it, and tools that can use
it say so in their output rather than failing.

* **Transport**: the Rust side spawns `duck-ear` (a Python entry point) as a
  child process and speaks JSON Lines over stdin/stdout. One request per
  line, one response per line, matched by `id`. stderr is logging.
* **Handshake**: the first message is `{"id":0,"method":"hello"}`; the reply
  lists `capabilities` (for example `describe`, `tag`, `similarity`,
  `transcribe`) and model versions. Unknown capabilities are ignored.
* **Requests**: `{"id":1,"method":"describe","params":{"wav":"/abs/path.wav"}}`
  with files passed by path, never inline bytes, so large audio never crosses
  the pipe. Responses are `{"id":1,"result":{...}}` or
  `{"id":1,"error":{"code":"...","message":"..."}}`.
* **Absence**: if the binary is missing, fails the handshake or times out,
  the bridge reports `ear: {"available": false, "reason": "..."}` and the
  app carries on. Nothing in core, synth or analyze knows the Ear exists.
* **Determinism**: Ear outputs are descriptions, never audio, so they do not
  affect renders.

## Diagnostics format

One `validate` call reports every problem: structural ones from the schema
walk and range ones from semantic validation, in document order, each with
a line and column. The schema walk is driven by the same generated JSON
Schema agents read, so it cannot drift from the Rust types; a unit test
fails if schemars ever emits a schema keyword the walker does not handle.
Two rules keep the report free of echoes: a value replaced by a placeholder
suppresses later issues at or under its path, and a misspelled field with a
suggestion suppresses complaints about the suggested field's default.

When a misspelled key or enum value has a *confident* suggestion (a known
alias, a case or separator slip, or at most two typos that are few
relative to the length), its contents are checked as the suggested name in
the same call: `[[layer]]` is checked as `layers`, an effect with
`type = "delay"` as an echo. Problems inside are reported at the name
actually written (`layer[0].pich.strat`). If the intended key is not also
present, the contents are decoded under the suggested name so range checks
run on them too, and their paths are mapped back to the written name.
Weaker guesses are still offered as a suggestion but their contents are
not checked, to avoid misleading nested errors. A number written in quotes
(`"440"`, `"440hz"`) gets a suggestion to write it without quotes.

```json
{
  "ok": false,
  "errors": [
    {
      "severity": "error",
      "code": "unknown_field",
      "path": "layers[0].pich",
      "message": "unknown field `pich`",
      "got": "pich",
      "allowed": { "options": ["name", "source", "volume", "..."] },
      "suggestion": "pitch",
      "line": 3,
      "column": 1
    }
  ],
  "warnings": []
}
```

Codes for recipes: `syntax`, `unknown_field`, `unknown_variant`,
`missing_field`, `type_mismatch`, `out_of_range`, `invalid_format`,
`unsupported_version`, `invalid`, `duplicate_field`. For files and tool
arguments: `not_found` (a file, folder or preset), `already_exists`,
`path_refused` (outside the root, absolute, `..`, a preset as a write
target, the wrong extension), `io` (exists but cannot be read, decoded or
written) and `invalid_argument` (a value or combination that cannot be
used); wrong types and ranges of arguments reuse `type_mismatch`,
`out_of_range` and `unknown_variant`. Warnings: `unused`, `suspicious`.
There is no catch-all code: a new kind of failure gets a new code. Human
output is one line per issue in the same order.

## Dependencies and why

| Crate | Used by | Why |
|---|---|---|
| serde, serde_json, toml | core | One schema, two syntaxes. |
| schemars | core | JSON Schema from the same types, with docs and defaults. |
| serde_path_to_error | core | Field path for the rare serde error the schema walk does not pre-empt. |
| strsim | core | "Did you mean" suggestions. |
| hound | audio | WAV read and write, zero dependencies of its own. |
| clap | cli | Argument parsing. |

The MCP server and the tools crate add no external dependencies: the
protocol layer is plain `serde_json` (see above), and `toml` writes edited
recipes. Pictures add none either: `analyze::png` is a small PNG encoder
(palette image, zlib with fixed-Huffman deflate and LZ77, CRC-32 and
Adler-32, about 240 lines plus a test-only inflater that proves round
trips), and the server's base64 is twenty lines. An image crate would pull
in a compression crate and more for one 480×254 palette image.

Synth and analyze bring no external dependencies beyond serde for the
analysis report. No `rand`, no DSP crates, no async runtime.
