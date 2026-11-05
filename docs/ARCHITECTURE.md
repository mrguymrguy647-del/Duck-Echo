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
  duck-echo-core/          recipe types, defaults, TOML/JSON loading, validation,
                           diagnostics, JSON Schema. No audio code.
  duck-echo-audio/         AudioBuffer type and WAV read/write (hound behind a
                           `wav` feature so synth can depend on just the type).
  duck-echo-synth/         deterministic renderer: Recipe -> Voice -> samples.
  duck-echo-analyze/       measurements over f32 samples -> JSON report, plus
                           compare (deltas and cross-correlation) and its own
                           small FFT.
  duck-echo-tools/         operations shared by the CLI and the MCP server:
                           load a WAV or render a recipe, compare, and (from
                           task 7 on) presets, variations and edits.
  duck-echo-cli/           the `duck-echo` binary and the end-to-end tests.
  duck-echo-mcp/           the `duck-echo-mcp` binary: the agent tools as an MCP
                           server over stdio.
  duck-echo-ear/           (planned) bridge to the Python Duck Ear process.
docs/                      this file, RECIPE_SPEC.md, ANALYSIS.md, ROADMAP.md
examples/recipes/          the example recipes (TOML, plus coin.json)
presets/                   tagged starting-point recipes, compiled into the
                           tools crate with include_str!
schema/recipe.schema.json  generated from the Rust types; a test fails when stale
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
 synth::render   offline wrapper: normalize (optional), clip -> AudioBuffer
        │
        ├──> audio::write_wav  (16/24-bit PCM or 32-bit float)
        │
        ▼
 analyze::analyze  summary line, duration, peak, RMS, loudness (gated
        │          BS.1770 from 400 ms, ungated K-weighted below, method
        │          named), clipping, silence, DC, crest factor, attack,
        │          decay, spectral centroid, tonality, pitch track
        │          (all defined in ANALYSIS.md)
        │
 analyze::compare  two sounds -> both analyses, b-minus-a deltas, waveform
        │          cross-correlation, summary line
        │
        ▼
 JSON report for the agent (or text for a human)
```

Per layer, the signal path inside `Voice::process` is:

```
oscillator (pitch curve: sweep + arpeggio + vibrato)
  × envelope × volume
  → filter (biquad, cutoff sweep updated every 16 samples)
  → layer effects (in order), still mono
  → [stereo: equal-power pan] summed into the mix at `offset`
master: effects (in order, one state per channel) → gain
        → [offline: normalize → clip → trim_silence]
```

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
* Tests assert byte-identical WAVs from repeated renders on one platform.
  Cross-platform identity is a design goal, verified in CI on the roadmap.

The analysis crate uses `f64` and platform libm freely: it measures, it does
not produce audio, and differences at the fifth decimal do not matter there.

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

### Phase 2: music

A song is a second document type that references sound recipes rather than a
change to the recipe itself:

```toml
version = 0
tempo = 120
[[tracks]]
instrument = "recipes/lead.toml"     # or an inline recipe table
[[tracks.notes]]
time = 0.0        # beats
note = "C5"       # note names arrive first as a recipe feature (roadmap)
length = 0.5      # beats
velocity = 0.8
[[tracks.patterns]]                  # reusable blocks, placed on the timeline
```

Recipes become instruments: a note sets the pitch (`pitch.start` times a
ratio) and the note length drives the envelope's `sustain_time`, with
`release` after note-off. This is why the envelope already has
`sustain`, `sustain_time` and `release`: the sound effect phase sets them
by hand, the music phase sets them from the timeline. The renderer gains a
scheduler that creates one `Voice` per note at the right frame, which is the
same `prepare` and `process` API used today.

### Phase 3: voice

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
`.toml`/`.json` file, or `preset:NAME`).

| Tool | Input | Output |
|---|---|---|
| `get_schema` | none | The recipe JSON Schema, same as `duck-echo schema`. |
| `list_presets` | optional `tag` | `{presets: [{name, category, target_lufs, description, tags}]}`. Presets are levelled to per-category loudness targets (see `presets/README.md`). |
| `get_preset` | `name` | `{name, category, description, tags, recipe, toml}`: the recipe as a JSON object and its TOML source with comments. |
| `validate` | recipe | The `Report`. An invalid recipe is a normal answer here, not a tool error. |
| `create` | `path`, `recipe`, optional `overwrite` | `{ok, path, warnings}`. Validates first; writes TOML or JSON by extension; refuses to replace a file unless `overwrite` is true. |
| `edit` | recipe, `ops`, optional `write` | `{ok, recipe, warnings, written, path}`. Each op is `{path, value}` or `{path, remove: true}`, with paths as in error messages; an index equal to the length appends. Edits apply to the document as written (defaults stay implicit), and a file is rewritten only when the result is valid. Comments are not kept. |
| `render` | recipe, `output`, optional `seed`, `format` | Same as `duck-echo render --json`: `{ok, output, seed, warnings, analysis}`. |
| `analyze` | `path` (WAV, recipe or preset) or `recipe` | The `Analysis` (see `ANALYSIS.md`). |
| `compare` | `a`, `b`: each a path or a recipe object | The `Comparison`. |
| `variations` | recipe, optional `count`, `variation`, `output_dir` | `{variation, seeds_matter, note, renders: [{seed, output, summary, analysis}]}`. |
| `export` | (task 13) | Not built: the Duck Forge format is still to be agreed. |

Rules the server keeps:

* **One error shape.** A failed call sets `isError` and returns the
  validation `Report` shape, whether the recipe was invalid, an argument was
  misspelled (with a suggestion, at path `arguments.<name>`), or a file was
  missing. Line numbers are dropped for inline recipes, since they would
  refer to text the agent never wrote.
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

Codes: `syntax`, `unknown_field`, `unknown_variant`, `missing_field`,
`type_mismatch`, `out_of_range`, `invalid_format`, `unsupported_version`,
`invalid`, `other`,
and for warnings `unused`, `suspicious`. Human output is one line per issue
in the same order.

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
recipes.

Synth and analyze bring no external dependencies beyond serde for the
analysis report. No `rand`, no DSP crates, no async runtime.
