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
                           solo, mute, picture) and file listings; the path
                           rules (`root`), the named commands (`commands`),
                           the shared Undo history (`project`, `history`)
                           and writing files in one step (`save`).
  duck-echo-live/          the realtime mixer for playing through speakers:
                           a pool of 32 voice slots, two lock-free queues,
                           fades and clipping, and the background song
                           player. No window and no sound card: the Studio
                           connects it to both. See "Realtime playback".
  duck-echo-cli/           the `duck-echo` binary and the end-to-end tests.
  duck-echo-mcp/           the `duck-echo-mcp` binary: the agent tools as an MCP
                           server over stdio.
  duck-echo-ear/           (planned) bridge to the Python Duck Ear process.
studio/                    Duck Echo Studio, a workspace of its own (Rust
                           1.95 for egui; the libraries above stay at 1.80):
  crates/app/              DuckEchoStudio: the window, the speakers
                           (tinyaudio + duck-echo-live), settings, the
                           update check, the Linux app-menu entry.
  brand/                   the Duck Echo icon (SVG sources, PNGs, .ico).
  release/                 DuckEchoStudio.iss (the Windows installer),
                           READ ME FIRST.txt, linux/ (AppRun, the menu entry,
                           make-appimage.sh).
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
live ──> synth, core, audio (and rtrb; tools only in its tests)
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

## Realtime playback

The renderer was shaped for a realtime audio thread from the start:

* `prepare(recipe) -> Voice` does all validation, planning and allocation.
* `Voice::process(&mut [f32])` allocates nothing, takes no locks, does no I/O
  and never reads the recipe again. Effects own fixed-size delay lines made
  in `prepare`. The only per-block state is a few counters and the filter
  coefficients.
* The offline `render` is just `process` in a loop plus the offline-only
  `normalize` step, which needs the whole signal.

Phase 2 step 3 built the playback on top of it, in two places.

**Live changes, in `duck-echo-synth/src/live.rs`.** A `ParamUpdate` is a
change a slider can make without a new voice: a layer's volume, a filter's
cutoff (and `cutoff_end`), or the master gain. A change does not jump,
which would click: the value slides to its target over 10 ms, a little
each sample. `Voice::process` and everything under it were not edited for
this. `Voice::process_live` is a wrapper: while nothing slides it calls
`process`; while something slides it moves each sliding value one step
and calls `process` for one frame, again and again. That is the same
sound, because `process` gives the same samples however its blocks are
cut (tested). So a render, which never sends a `ParamUpdate`, runs the
same code on the same numbers as before. `Voice::rewind` plays a voice
again from its first frame without building a new one (delay lines are
cleared in place); a rewound voice makes exactly a new voice's samples.

**The mixer, in `duck-echo-live`.**

```
 control thread (the window)                audio thread (the sound card)
 ---------------------------                -----------------------------
 Player                                      Mixer
   play / replace / update / stop  ------>     process(block), every ~23 ms:
   (sounds are made here, where                  take messages (never waits)
    allocating is fine)                          mix the sounds in 32 slots
                                                 slide changes, fade, clip
   collect()  <-------------------------       send finished sounds back
   (finished sounds are dropped here)
```

* **Two one-way queues**, both `rtrb` ring buffers (one writer, one
  reader, lock-free). The mixer's state for the window (play heads,
  clipped samples) is a few atomics.
* **Nothing is allocated or freed on the audio thread.** Every buffer is
  made in `live()`. A finished or replaced sound is sent back to the
  control thread and dropped there. If the ring back is full, sounds wait
  in a parking area made in advance; if that is nearly full, the mixer
  stops taking messages (the player is told `Busy`) until the control
  thread has collected. It never blocks and never drops a sound.
* **What you hear is the file.** `Sound::from_recipe` plays the voice at
  the gain and length the offline master steps give the rendered file
  (`master.normalize`, `normalize_lufs`, `trim_silence`), and the mixer
  clips its output to -1..=1 as a render does. A test plays every preset
  and example through the mixer and compares it with `render`, sample for
  sample, once and repeated.
* **No clicks.** `Player::update` slides (above). `Player::replace` (an
  edit that needs a new voice) puts the new voice at the old one's place
  and cross-fades over 10 ms; `stop`, and `play` over a playing sound,
  fade the old one out. A request that arrives during a fade waits for it
  (at most 10 ms and one block), so a fade is never cut short.
* **Songs** play from a render made on a worker thread
  (`SongPlayback`): the same `render_song` that writes the file. After an
  edit the song is rendered again and the new render takes over at the
  play head with the same cross-fade. (`SongPlayer` makes a voice per
  note, which allocates; live song voices from a pool are a Phase 3 job.)
* **The mixer does not resample.** A sound must have the sound card's
  sample rate; otherwise `Player` refuses it with a message. The Studio
  opens the sound card at the sound's rate.

How the rules are proven (`crates/duck-echo-live/tests`):

* `realtime.rs` replaces the test program's allocator with
  `assert_no_alloc` (always on, in debug and release). Every block is
  mixed inside `assert_no_alloc`, which counts each allocation or free on
  that thread; each test ends with the count at zero. One test runs 60
  seconds of busy playback, one stalls the control thread until every
  queue is full, one uses two real threads, and one first shows the
  detector catching an allocation and the freeing of a voice.
* `clicks.rs` measures the biggest step between two neighbouring samples
  of the output and compares it with the sound's own. Each scene runs
  twice: smoothed (no step bigger than the sound's own) and with the
  smoothing turned off (the same scene clicks).
* `audio_path.rs` is a tripwire for locks: it reads the source of every
  file the audio thread runs and fails if one names a lock, a channel, a
  sleep, a file or a print. Nothing can count locks at run time the way
  allocations are counted, so the lock rule rests on this, on the stall
  test, and on review.
* `matches_render.rs` and `song.rs` compare what plays with what renders.

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
* **Note check.** `tools::notecheck` listens for every written note on
  its track's stem, using `analyze::notes` (a spectrum peak search that
  reads a steady tone to a fraction of a cent). `analyze` always runs it
  for a song; `render` does with `per_track` or `notes`, because it costs
  one more render per track.
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
| `get_guide` | optional `topic` (`guide`, `recipe_spec`, `song_spec`, `analysis`, `instruments`, `presets`) | `{topic, text}`: the page as Markdown, compiled in from the repository's own files (`duck-echo-tools/src/guide.rs`), so an agent without the repository can read the guide. `recipe_spec` and `song_spec` are the two parts of `RECIPE_SPEC.md`. Same as `duck-echo guide TOPIC`. |
| `get_schema` | optional `kind` (`recipe` or `song`) | The recipe JSON Schema, same as `duck-echo schema`, or the song schema (`duck-echo schema song`). |
| `list_presets` | optional `tag` | `{presets: [{name, category, target_lufs, description, tags}]}`. Presets are levelled to per-category loudness targets (see `presets/README.md`). A tag no preset has is a `not_found` error at `arguments.tag` with the closest real tag as `suggestion` and every tag in `allowed`, so a typo is never an empty list. |
| `get_preset` | `name` | `{name, category, description, tags, recipe, toml}`: the recipe as a JSON object and its TOML source with comments. |
| `list_instruments` | none | `{instruments: [{name, kind, description, tags, home, range, target_lufs, pieces}]}`: the built-in instruments and the kit `drums`. |
| `get_instrument` | `name` | The listing entry plus `recipe` (a JSON object) and `toml` (its source with comments). A kit is an error that names its pieces. |
| `list_files` | optional `path` (a folder), `recursive` (default true) | `{folder, entries: [{path, kind, bytes, seconds}], truncated}`. Kinds: `folder`, `recipe`, `audio`, `image`, `link`, `other`; `seconds` for WAV files. Hidden files are skipped, links are not followed, at most 500 entries. |
| `validate` | recipe | The `Report`. An invalid recipe is a normal answer here, not a tool error. |
| `create` | `path`, and `recipe` or `from`, optional `overwrite` | `{ok, path, kind, warnings}` (`kind` is `recipe` or `song`; with `from`, also `from`). Validates first; writes TOML or JSON by extension; refuses to replace a file unless `overwrite` is true. `from` (`preset:NAME`, `instrument:NAME` or a file) runs the `file_copy_from` command: the copy is named after its file, its description says what it was copied from, and a TOML copy keeps its comments. One Undo step either way. |
| `edit` | recipe, `ops`, optional `write` | `{ok, recipe, warnings, written, path}`. Each op is `{path, value}` or `{path, remove: true}`, with paths as in error messages; an index equal to the length appends. Edits apply to the document as written (defaults stay implicit), and a file is rewritten only when the result is valid. Comments are not kept. |
| `render` | recipe, `output`, optional `seed`, `format`, extras | Same as `duck-echo render --json`: `{ok, output, seed, warnings, analysis}`, plus `heard_layers`, `layers` and `picture` when asked. `warnings` adds a render warning when `normalize_lufs` hit the peak ceiling. A song adds `song` (`tempo`, `beats_per_bar`, `bars`, `loop`, `song_frames`, `seconds_per_bar`, `tracks`, `notes`), `heard_tracks` and `tracks` when asked, for a loop `loop_seam` (`seam_step`, `largest_step`, `clean`), and with `per_track` or `notes` the `note_check`. |
| `analyze` | `path` (WAV, recipe or preset) or `recipe`, extras | The `Analysis` (see `ANALYSIS.md`), plus `warnings` (recipes only, when there are any), `heard_layers`, `layers` and `picture` when asked. For a song also `song`, `loop_seam` and always `note_check` (the notes written next to the notes heard; `notes: true` lists every note). |
| `compare` | `a`, `b`: each a path or a recipe object | The `Comparison`. |
| `variations` | recipe (not a song), optional `count`, `variation`, `output_dir` | `{variation, seeds_matter, note, renders: [{seed, output, summary, analysis}]}`. |
| the commands | see below | One tool per command (`sound_set`, `song_add_note`, `edit_undo`...), with the command's name and arguments. |
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
  so is any `:` in a path, on every platform, because on Windows a drive
  letter in the middle of a path (`sub/C:\x`) turns the rest into an
  absolute path when paths are joined. The resolved path is built one
  plain name at a time, never by joining a multi-part tail, and is
  checked once more to lie inside the root. Symbolic links are resolved
  and refused if they lead out of the root or are broken. `preset:NAME`
  and `instrument:NAME` are accepted only where a recipe is read, and
  only for a known name; they can never be a write target. A song's
  instrument files go through the same checks. `render`
  writes only `.wav` files, so it cannot overwrite other files; recipe
  names are made file-safe before they become file names. Results show
  paths relative to the root. All of this lives in
  `crates/duck-echo-tools/src/root.rs` (shared by the server and the
  CLI's `do` and `run`), and stdio tests try each escape. The project's own
  `.duck-echo` folder (the Undo history) is refused as a path, so a tool
  can never write into the history behind its back.
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

## Commands and the shared Undo history

Phase 2 (Duck Echo Studio) needs a person and an agent to work on one
project at the same moment, and to undo each other's changes. So every
action that changes a project is a named **command**
(`crates/duck-echo-tools/src/commands.rs`): a `Command` enum, tagged by
`"do"`, whose arguments deserialize with `deny_unknown_fields` and whose
JSON Schemas come from `schemars`. One table (`commands::TABLE`) lists
each command's name, description and argument schema. Every front end
uses it:

* the MCP server makes one tool per entry, same name, same arguments;
* the CLI has `duck-echo commands`, `duck-echo do NAME '{json}'` and
  `duck-echo run script.json` (a JSON list of commands);
* the Studio's buttons will build the same `Command` values.

Names use `_`, not `.`, because Claude's tool names must match
`^[a-zA-Z0-9_-]{1,128}$`. File commands (`file_new_sound`,
`file_copy_from`, `file_rename`, `file_delete`, `file_restore`,
`file_set_text`...), sound commands (`sound_set`, `sound_add_layer`...),
song commands (`song_add_note`, `song_place_pattern`, `song_set_loop`...)
and `edit_undo`, `edit_redo`, `edit_history`. A command checks its result
(a recipe, or a song with its instruments) before it writes; an invalid
result changes nothing and returns the usual report. Commands that change
data rewrite the file from its data, so `#` comments go, as with `edit`
(the design keeps `toml_edit` for step 6); `file_set_text` and
`file_copy_from` keep the text as written.

**History on disk** (`history.rs`). Undo cannot live in one program's
memory when two programs share it, so each change is one JSON file,
`<project>/.duck-echo/history/NNNNNN.json`, holding each changed file's
whole text before and after, a title, who made it (`person`, `agent` or
`outside`) and when. Undone steps move to `history/undone/` until Redo or
the next change. Edits with the same `stroke` number from the same maker
join one step (a slider drag). The newest 500 steps are kept. A deleted
file is also copied to `.duck-echo/recycle/`. The step's time is only
metadata: nothing rendered reads it.

**No change is ever lost** (`project.rs`). A change works out its new
text without holding anything, then takes the project lock
(`.duck-echo/lock`, created with "fail if it exists"; a lock older than
10 seconds was left by a crash and is removed), reads the files again and
writes only if each still holds exactly the text it read. If another
program saved first, the change is worked out again on the new text, this
time holding the lock, so no Duck Echo program can save in between. If
the file changed even then (only a program that is not Duck Echo can do
that), the change is refused, "kept changing ... Nothing was changed; try
again", and the other change is kept. A file whose text is not what the
history last wrote was changed outside Duck Echo; that change becomes its
own step ("Changed outside Duck Echo") before anything else, and an Undo
that would overwrite it refuses instead and leaves that step on top.

**One-step writes** (`save.rs`). Every file Duck Echo writes (recipes,
songs, history steps, WAVs, PNGs) goes to a hidden temporary file in the
same folder, is flushed to disk and renamed onto its name, so no program
ever reads half a file. On Windows a rename onto a file another program
holds open can fail with "access denied" for a moment; it is tried again
for about a second.

The tests: every command undone and redone gives back every file byte for
byte (`crates/duck-echo-tools/tests/commands.rs`), and the real CLI and
the real MCP server add 40 layers each to one file at the same moment
while a reader checks the file always parses; every change is there, in
each program's order, as one step each, and undoing all 80 gives back the
first text exactly (`crates/duck-echo-mcp/tests/shared_undo.rs`). The
MCP `create` and `edit` tools save through the same path, so they are
steps too.

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
a line and column. The schema walk is driven by the JSON Schema generated
from the Rust types, so it cannot drift from them; a unit test
fails if schemars ever emits a schema keyword the walker does not handle.

**Ranges live in one table**, `duck-echo-core/src/ranges.rs`: the smallest
and largest value of every number in a recipe and a song. Semantic
validation reads its limits from the table, and the published schema
(`duck-echo schema`, `get_schema`, `schema/*.json`) is the schema of the
types plus each range written in as `minimum` and `maximum` (or
`exclusiveMinimum`), so agents and the Studio's sliders see the very
limits that validation enforces. The walker reads the schema *without*
the ranges, so a range error is still reported once, by validation, in
the same words as before. A test puts every field of the table just
outside its range and reads the table's limits back from the error.
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
| schemars | core, tools | JSON Schema from the same types, with docs and defaults; in tools, the argument schemas of the commands, so the MCP tools and the Rust types cannot disagree. |
| serde_path_to_error | core | Field path for the rare serde error the schema walk does not pre-empt. |
| strsim | core | "Did you mean" suggestions. |
| hound | audio | WAV read and write, zero dependencies of its own. |
| clap | cli | Argument parsing. |
| rtrb | live | A lock-free ring buffer with one writer and one reader, made for audio threads: the audio thread takes and sends messages without ever waiting on a lock or allocating. No dependencies of its own. Writing one ourselves needs `unsafe` code, which the workspace forbids. |
| eframe (egui) 0.37 | studio | The window and every widget: the same library and version as Duck Quill and Duck Forge Studio, so the three apps look alike. Its default wgpu renderer draws on Direct3D 12 (or its software fallback, WARP) on Windows and on Vulkan or OpenGL on Linux. |
| egui_kittest 0.37 | studio (tests only) | Draws every screen with no window and finds things by their words, as a person would; with wgpu it also saves pictures of the screens. The same as Duck Forge Studio. |
| tinyaudio 2 | studio | Plays the mixer's blocks through the speakers: DirectSound on Windows, ALSA on Linux, linked in the normal way (ALSA reaches PipeWire and PulseAudio too), with no `unsafe` code of ours. The lightest crate that does it, and Duck Forge's choice. Building on Linux needs `libasound2-dev`. |
| dirs 6 | studio | Where Documents and the settings folder really are: Windows' own answer (OneDrive often moves Documents), the XDG folders on Linux. The design named `windows-sys` for this, as Duck Forge uses; `windows-sys` would need `unsafe` code of ours to call Windows, and would leave Linux to us, so `dirs` (which wraps the same Windows call) does both. |
| sha2 0.10 | studio | The SHA-256 fingerprint of a downloaded update, checked against the release's `SHA256SUMS` before it is used, as Duck Quill does. |
| winresource 0.1 | studio (build only) | Puts the icon and the version into `DuckEchoStudio.exe`, as both siblings do. |
| assert_no_alloc | live (tests only) | Replaces the test program's allocator and counts every allocation and free inside a marked section, so a test fails if the audio block touches the allocator. Its default features switch the check off in release builds; here they are off and the counting ones are on, so it always runs. |

The Studio's crates live only in `studio/`, a workspace of its own (its own
lock file and `target`), so the command-line tool and the MCP server gain
none of them. Two programs are used to package it, not linked: **Inno
Setup 6** builds `DuckEchoStudioSetup.exe` from
`studio/release/DuckEchoStudio.iss` (as both siblings do), and
**appimagetool** (downloaded by CI) builds `DuckEchoStudio.AppImage` with
its static runtime, so a person's Linux does not need the old FUSE 2
library. The Studio checks for new versions with the system's own `curl`
(Windows 10 and later have it; so does nearly every Linux), as Duck Forge
does: no web library.

The MCP server and the tools crate add no external dependencies: the
protocol layer is plain `serde_json` (see above), and `toml` writes edited
recipes. Pictures add none either: `analyze::png` is a small PNG encoder
(palette image, zlib with fixed-Huffman deflate and LZ77, CRC-32 and
Adler-32, about 240 lines plus a test-only inflater that proves round
trips), and the server's base64 is twenty lines. An image crate would pull
in a compression crate and more for one 480×254 palette image.

Synth and analyze bring no external dependencies beyond serde for the
analysis report. No `rand`, no DSP crates, no async runtime.
