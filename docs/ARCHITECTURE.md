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
  duck-echo-analyze/       measurements over f32 samples -> JSON report.
  duck-echo-cli/           the `duck-echo` binary and the end-to-end tests.
  duck-echo-mcp/           (planned) MCP server exposing the agent tools.
  duck-echo-ear/           (planned) bridge to the Python Duck Ear process.
docs/                      this file, RECIPE_SPEC.md, ROADMAP.md
examples/recipes/          the example recipes (TOML, plus coin.json)
schema/recipe.schema.json  generated from the Rust types; a test fails when stale
```

Dependency direction (arrows point at what is depended on):

```
cli ──> synth ──> core
 │        └────> audio (buffer type only)
 ├────> analyze
 ├────> audio (with wav feature)
 └────> core
mcp (planned) ──> same set as cli
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
 core::load      parse (serde_path_to_error keeps the field path)
        │        -> Issue { code, path, message, got, allowed, suggestion, line }
        ▼
 core::validate  range and structure checks, all collected, warnings separate
        │
        ▼
 synth::prepare  variation jitter from seed, lengths, allocations -> Voice
        │
        ▼
 Voice::process  block by block, allocation-free, mono f32
        │
        ▼
 synth::render   offline wrapper: normalize (optional), clip -> AudioBuffer
        │
        ├──> audio::write_wav  (16/24-bit PCM or 32-bit float)
        │
        ▼
 analyze::analyze  duration, peak, RMS, LUFS, clipping, silence, DC
        │
        ▼
 JSON report for the agent (or text for a human)
```

Per layer, the signal path inside `Voice::process` is:

```
oscillator (pitch curve: sweep + arpeggio + vibrato)
  × envelope × volume
  → filter (biquad, cutoff sweep updated every 16 samples)
  → layer effects (in order)
  → summed into the mix at `offset`
master: effects (in order) → gain → [offline: normalize] → clip
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

## Agent tools for the MCP server (design)

All tools take and return JSON. Recipes are passed inline as JSON objects or
by path. Every tool that touches a recipe runs validation and returns the
same `Report` shape the CLI prints with `--json`.

| Tool | Input | Output | Notes |
|---|---|---|---|
| `get_schema` | none | JSON Schema | Same document as `duck-echo schema`. |
| `list_presets` | optional tag/query | list of `{name, tags, description, recipe}` | Starts from `examples/recipes`; a preset library is a roadmap task. |
| `validate` | `recipe` | `Report` | No side effects. |
| `create` | `recipe` (JSON), `path` | `{path, report}` | Writes TOML. Refuses to overwrite unless `overwrite: true`. |
| `edit` | `path`, `patch` (JSON merge patch or `{path, value}` list) | `{recipe, report}` | Applies, validates, writes only when valid. Returns the updated recipe so the agent sees the result. |
| `render` | `recipe` or `path`, `output`, optional `seed`, `format` | `{output, duration, analysis, warnings}` | Same as the CLI `render --json`. |
| `analyze` | `wav` path | `Analysis` | Also the "listen" tool until the Ear arrives; later gains `ear: true` to add Ear descriptions. |
| `compare` | two wav paths or two recipes | `{a, b, delta: {peak_db, lufs, duration, ...}, correlation}` | Lets an agent check that an edit moved the sound the intended way. |
| `variations` | `recipe`, `count`, `variation` | list of `{seed, analysis}` | Renders N seeds and reports each; agent picks by numbers. |
| `export` | `recipe` or `wav`, `target: "duck_forge"`, options | `{files}` | Format to be decided with the Duck Forge owner (roadmap). |

Design rules: tools are idempotent where possible, never block on audio
playback, and always return the recipe they ended with so the agent does not
have to re-read files. Errors use the `Report` shape, not free text.

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
`type_mismatch`, `out_of_range`, `unsupported_version`, `invalid`, `other`,
and for warnings `unused`, `suspicious`. Human output is one line per issue
in the same order.

## Dependencies and why

| Crate | Used by | Why |
|---|---|---|
| serde, serde_json, toml | core | One schema, two syntaxes. |
| schemars | core | JSON Schema from the same types, with docs and defaults. |
| serde_path_to_error | core | Exact field path for deserialization errors. |
| strsim | core | "Did you mean" suggestions. |
| hound | audio | WAV read and write, zero dependencies of its own. |
| clap | cli | Argument parsing. |

Synth and analyze bring no external dependencies beyond serde for the
analysis report. No `rand`, no DSP crates, no async runtime.
