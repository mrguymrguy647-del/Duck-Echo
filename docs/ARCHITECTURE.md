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
  duck-echo-audio/         AudioBuffer type, WAV read/write (hound behind a
                           `wav` feature so synth can depend on just the type)
                           and OGG Vorbis (vorbis_rs and lewton behind `ogg`).
  duck-echo-synth/         deterministic renderer: Recipe -> Voice -> samples,
                           and the song player (one Voice per note).
  duck-echo-analyze/       measurements over f32 samples -> JSON report, plus
                           compare (deltas and cross-correlation), onsets and
                           energy over time, how a sound feels (character),
                           a picture (spectrogram and level) as PNG, a board
                           with a small font for labelled pictures (board),
                           and its own small FFT and PNG encoder.
  duck-echo-tools/         operations shared by the CLI and the MCP server:
                           load a WAV or render a recipe, compare, presets,
                           variations, edits, analysis extras (per layer,
                           solo, mute, picture) and file listings; the path
                           rules (`root`), the named commands (`commands`),
                           the shared Undo history (`project`, `history`)
                           and writing files in one step (`save`); a
                           rendered WAV's facts file (`asset`), writing and
                           checking OGG music (`ogg`), `expect`
                           lines, the variant export (`variants`), the
                           sound-set check (`sets`), the mix check from
                           a cue list (`mix`), sending sounds into a
                           Duck Forge game (`forge`) and asking the Duck
                           Ear (`ear`: one kept Ear, the answer cache,
                           `listen`, `ear_status`, `analyze` with
                           `ear: true`).
  duck-echo-live/          the realtime mixer for playing through speakers:
                           a pool of 32 voice slots, two lock-free queues,
                           fades and clipping, and the background song
                           player. No window and no sound card: the Studio
                           connects it to both. See "Realtime playback".
  duck-echo-cli/           the `duck-echo` binary and the end-to-end tests.
  duck-echo-mcp/           the `duck-echo-mcp` binary: the agent tools as an MCP
                           server over stdio.
  duck-echo-ear/           the bridge to the Duck Ear (Phase 4): starts it,
                           speaks its JSON Lines protocol, keeps time,
                           stops it with everything it started. Knows
                           nothing about recipes. `fake-ear`, a test
                           program that breaks the protocol on purpose.
studio/                    Duck Echo Studio, a workspace of its own (Rust
                           1.95 for egui; the libraries above need 1.87):
  crates/app/              DuckEchoStudio: the window, Home (home.rs, with
                           library.rs for the files, thumbnails and
                           gallery without a window), the sound editor
                           (editor.rs for its state, saves and renders,
                           controls.rs for the sliders made from the
                           schema, sound_screen.rs to draw it), the
                           a person and an AI assistant together
                           (together.rs: notes about others' changes,
                           the History panel; connect.rs: Settings >
                           Connect an AI assistant), the
                           song timeline (song_view.rs for its lanes,
                           mute, solo and note check, song_edit.rs for
                           the commands its edits run, song_screen.rs to
                           draw it), Send to game and the Duck Forge
                           folder (send.rs), `--open FILE` (opening.rs), the
                           speakers (tinyaudio + duck-echo-live),
                           settings, the update check, the Linux app-menu
                           entry.
  brand/                   the Duck Echo icon (SVG sources, PNGs, .ico).
  release/                 DuckEchoStudio.iss (the Windows installer),
                           READ ME FIRST.txt, linux/ (AppRun, the menu entry,
                           make-appimage.sh).
docs/                      this file, RECIPE_SPEC.md, ANALYSIS.md, ROADMAP.md
ear/stub/                  the stub Duck Ear (Python, standard library only):
                           the protocol with no models, for tests
ear/duck_ear/              the real Duck Ear (Python: numpy, ONNX Runtime,
                           tokenizers), with its tests in ear/tests/
ear/tools/                 get_describe_model.py: a person downloads a
                           describe model with it, checked by SHA-256;
                           measure_describe_model.py: downloads and
                           measures one, in one command
ear/convert/               converts the score models to ONNX and checks
                           them (PyTorch; run once, by builders)
ear/testset/               sounds with known answers, for `ear check`
examples/recipes/          the example recipes (TOML, plus coin.json)
examples/songs/            the example songs
examples/film/             a sound set and a cue list to check (set-check, mix)
presets/                   tagged starting-point recipes, compiled into the
                           tools crate with include_str!
instruments/               the starter instruments for songs, compiled in the
                           same way (see instruments/README.md)
schema/recipe.schema.json  generated from the Rust types; a test fails when stale
schema/song.schema.json    the same, for songs
schema/set.schema.json     the same, for sound set files
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
ear ──> nothing of Duck Echo's (serde_json; windows-sys on Windows)
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
        │        mismatch at once; prune them. A `{ min, max }` table where
        │        a number goes is a random range: recorded on
        │        `Recipe::ranges`, decoded as its `min`
        ▼
 typed decode    clean documents straight from the source with serde;
        │        documents with structural errors from the pruned tree
        ▼
 core::validate  range and rule checks, all collected, warnings separate
        │        (a recipe with random ranges twice: all at min, all at max);
        │        issues under an already-reported path are dropped
        │        -> Issue { code, path, message, got, allowed, suggestion, line }
        ▼
 synth::prepare  random ranges picked from the seed (core::random), then
        │        variation jitter from seed, lengths, allocations -> Voice
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
  the others. `variation` draws its jitter from a separate stream. A random
  range's pick (`core::random`) is SplitMix64 seeded with the render's (or
  the note's) seed mixed with an FNV-1a hash of the field's path, so every
  field has a stream of its own; a recipe without ranges skips this step
  entirely, so its bytes are as before.
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
* **OGG music is compared, not promised.** An `.ogg` render comes from
  Xiph's libvorbis, C code with floating point, so its bytes may differ
  between systems. CI renders the two Duck Craft themes as OGG on both
  and says whether they match (`OGGSUMS`), without failing. What is
  promised: the WAV of the same recipe is exact, and the OGG decodes to
  the same number of frames and passes the same loop check everywhere;
  `recipe_hash` (not the bytes) says whether a sound changed.
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
It is started as `duck-echo-mcp --root DIR [--duck-forge DIR]`. Every path
a tool takes stays inside the root; with `--duck-forge` (a folder whose
`Cargo.toml` names `duckforge`), `game_send` may also write into that
folder's games, by name only and only into each game's `assets` and `art`
folders (`tools::forge`). The command line takes the same folder from
`DUCK_ECHO_DUCK_FORGE` (or `duck-echo send --duck-forge`), and the Studio
from its settings (`Documents/Duck Forge` when that is one).

A project's Undo history and recycle bin are kept in `.duck-echo` inside
it, unless its `Root` is given another folder (`Root::with_own_folder`):
the Studio does this for a Duck Forge game opened with `--open`, keeping
it in its own settings folder (`projects/<game>-<8 hex digits of the
game folder's path's SHA-256>`), so nothing of Duck Echo's lands in the
game. An agent working on that game through the MCP server (root = the
game) would keep its own `.duck-echo` there instead: the two do not share
one history in that case.
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
| `render` | recipe, `output`, optional `seed`, `format`, extras | Same as `duck-echo render --json`: `{ok, output, seed, warnings, analysis, asset, asset_file}`, plus `heard_layers`, `layers` and `picture` when asked. `warnings` adds a render warning when `normalize_lufs` hit the peak ceiling. A song adds `song` (`tempo`, `beats_per_bar`, `bars`, `loop`, `song_frames`, `seconds_per_bar`, `tracks`, `notes`), `heard_tracks` and `tracks` when asked, for a loop `loop_seam` (`seam_step`, `largest_step`, `clean`), and with `per_track` or `notes` the `note_check`. Every render also writes `<output>.json` (`asset::AssetInfo`: length, loudness, peak, leading silence, `hit_seconds`, `loops`, `recipe_hash` as the SHA-256 of the document's JSON data), and the Studio's Export does the same. |
| `analyze` | `path` (WAV, recipe or preset) or `recipe`, extras, `ear` | The `Analysis` (see `ANALYSIS.md`), plus `warnings` (recipes only, when there are any), `heard_layers`, `layers` and `picture` when asked. For a song also `song`, `loop_seam` and always `note_check` (the notes written next to the notes heard; `notes: true` lists every note). With `expect`, its lines checked. With `ear: true`, the Duck Ear's `ear` section (see "Duck Ear bridge"). |
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
and `edit_undo`, `edit_redo`, `edit_history`. `sound_variants` writes
audio and lists, not recipes or songs, and `game_send` writes into a
Duck Forge game, not the project, so neither is an Undo step
(`changes_files: false`). A command checks its result
(a recipe, or a song with its instruments) before it writes; an invalid
result changes nothing and returns the usual report. Commands that change
data (and the MCP `edit` tool) change only the values that changed,
inside the file's text (`edit::document_text_from`, with `toml_edit`), so
`#` comments, blank lines and inline tables stay as written. The new text
is read back and must hold exactly the new data; if it does not (or the
old text does not parse), the file is written whole from its data, as
before. JSON files are always written whole. `file_set_text` and
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

`file_copy_from` copies from `preset:NAME`, `instrument:NAME`, an example
song (`example:NAME`, the files of `examples/songs` compiled into the tools
crate by `examples.rs`), or a file in the project, so the Studio's **Make a
copy** is one command and one Undo step for every kind of starting point,
and agents can copy an example song the same way.

**The sound editor runs commands too.** A slider runs `sound_set`, typing
runs `file_set_text` (0.6 s after the last key, and only when the text
works), and the "+" and "x" buttons run `sound_add_layer`,
`sound_remove`, `sound_add_effect` and the rest. While a slider is held,
the editor saves at most every 200 ms and once more on release, all with
one stroke number (one Undo step), because the Documents folder is often
synced by OneDrive and every save is an upload. The sliders are made from
`recipe_schema()` (`controls.rs`), so they have the ranges validation
enforces. Each change is rendered on a background thread with the same
code as `render` (the newest change wins), and the same render draws the
analyze picture (`picture::draw_image`, the
pixels of the PNG agents get) and the summary line. Export WAV writes the
bytes `render` writes, through `Root::write` in the chosen folder.

**Live changes** (`editor::Live`). Play and Loop play a live voice
(`Sound::from_recipe_mastered`, the file's samples exactly). After each
edit, `ParamUpdate::between(playing, new)` (in `duck-echo-synth`) says
whether the change is only to a layer's volume, a filter's cutoff or the
master gain. If so, the updates go to the playing voice at once and slide
over 10 ms; when the edit's render arrives, only the level slides to the
file's new level (a normalized sound keeps its loudness). Any other
change waits for its render, and the new voice takes over at the same
place with a 10 ms cross-fade (`Player::replace`). A render that is not
the newest change's is shown but never played.

**The song timeline draws the arrangement** (`song_view.rs`): the list
of notes `arrange` gives the renderer, with patterns placed, chords split
and bars turned into beats, so what you see is what you hear. Play and
Loop use the background song player; Mute and Solo play the stem of the
tracks heard at their level in the full mix (`SongPlayback::set_heard`,
the same stem as `render --solo`). The note check runs on a background
thread with `measure_song` and outlines the notes it finds a problem
with. Its edits (add, move, stretch and delete notes; place, move and
take off clips; a track's instrument and volume) are built as the shared
song commands in `song_edit.rs` and run as the person, one Undo step
each; a drag runs its command once, on release. A test makes a song with
these commands and checks it renders the same WAV bytes as the same song
written by hand. The screen counts beats from 1; files and commands
count from 0.

**A person and an agent together** (`together.rs`). The Studio and the
MCP server share only the project folder: its files and its history. The
editor and the timeline look at the open file twice a second; when it
changed, `Project::notice_outside` first records a change no Duck Echo
program made ("Changed outside Duck Echo", by `outside`), then the
newest steps someone else made to that file give the three-second note,
with Undo. Steps are known by number, time and title, since an Undo
frees its number for the next step. Typed text is saved whole, so before
saving it the editor checks the file still holds what the typing started
from; if not, the person chooses Keep mine or Use theirs. Settings >
Connect an AI assistant (`connect.rs`) writes the lines for Claude
Desktop: `duck-echo-mcp` next to the Studio (or the AppImage with `mcp`)
and `--root` set to the sounds folder. The test
`studio/crates/app/tests/together.rs` runs the real MCP server and the
Studio code on one sound (playtest 3, `docs/playtest-3.md`).

**Home runs commands, nothing else.** Every button on Home that changes a
file (New sound, New song, Make a copy, Rename, Duplicate, Delete, Undo,
Redo) runs one of these commands as `Who::Person`, so the Studio holds no
file logic an agent cannot reach. Home reads the project folder again
every second, so a file an agent makes appears by itself.

The tests: every command undone and redone gives back every file byte for
byte (`crates/duck-echo-tools/tests/commands.rs`), and the real CLI and
the real MCP server add 40 layers each to one file at the same moment
while a reader checks the file always parses; every change is there, in
each program's order, as one step each, and undoing all 80 gives back the
first text exactly (`crates/duck-echo-mcp/tests/shared_undo.rs`). The
MCP `create` and `edit` tools save through the same path, so they are
steps too.

## Duck Ear bridge

The Duck Ear is a separate program that listens with machine-learning
models (Phase 4; the plan is `docs/phase4-design.md`). It is optional:
every feature of Duck Echo works without it, a render never waits for it,
and nothing in core, synth or analyze knows it exists. `duck-echo-ear` is
the bridge; `duck-echo-tools` decides what to ask (step 2).

* **Transport**: the bridge starts the Ear as a child process and speaks
  JSON Lines over its stdin and stdout: one request per line, one answer
  per line, matched by `id`. stderr is the Ear's log; the bridge keeps its
  last 64 KB and can write it to `ear.log` (started again past 1 MB). A
  `.py` Ear is run by Python (`DUCK_ECHO_EAR_PYTHON`, else `python3`, or
  `python` on Windows).
* **Three threads** (writer, reader, log reader), so the Ear can never
  stop Duck Echo by filling a pipe, and every wait has a deadline. The
  reader keeps at most 1 MB of a line: a longer line is a broken answer.
* **Messages**: `{"id":1,"method":"score","params":{...}}` in;
  `{"id":1,"result":{...}}`, `{"id":1,"error":{"code","message"}}` or
  `{"id":1,"progress":{"message","done"}}` out. Progress never gives a
  request more time. Sounds go by path, never as bytes. Inside protocol
  version 1 an answer's unknown fields are skipped (a newer Ear may add
  some); recipes and tool calls stay strict.
* **Handshake**: `hello` with `{"protocol": 1, "describe_idle_seconds":
  120}`; the answer (`protocol::Hello`) names the Ear's version, its
  capabilities (`score`, `describe`, `ask`), its device and its models
  with their revisions, and `stub: true` for the stub.
* **Timeouts**: one table (`times::Times`): 20 s for `hello`, 60 s and
  180 s for `load` (score, describe), 15 s for `score`, 60 s for `describe`
  and `ask` on a graphics card and 240 s on the CPU, 3 s for `bye`.
  `DUCK_ECHO_EAR_TIMEOUT_SCALE` multiplies them all, from 0.25 to 10: no
  value makes a wait endless. When time runs out the Ear is stopped and
  the failure says it starts again on the next question.
* **Failures**: the Ear cannot be asked at all (`Unavailable`:
  `not_installed`, `will_not_start`, `no_hello`, `broken_hello`,
  `protocol`), or one question failed (`Failure`: `timeout`,
  `broken_answer`, `ear_stopped` with its exit code and last log lines,
  the Ear's own error code, `not_running`). Every failure but the Ear's
  own error stops the Ear.
* **Stopping everything it started**: the Ear will start `llama-server`
  (step 4). On Linux the Ear heads its own process group and the bridge
  stops the whole group (`kill -KILL -- -<group>`); on Windows it is put
  in a job object that kills everything in it when stopped or when Duck
  Echo ends. And every Ear exits as soon as its stdin closes. A test
  starts a pretend Ear with a child of its own and checks the child stops
  after `stop`, after a drop and after a timeout.
* **Determinism**: the Ear's answers are descriptions, never audio, and
  are never written into a sound or its facts, so renders stay
  byte-for-byte the same.

### What agents get (`duck-echo-tools/src/ear.rs`)

* **Where the Ear is**: `--ear-program PROGRAM` (the MCP server, `listen`
  and `ear status`; the old spelling `--ear PROGRAM` still works, with a
  note), else `DUCK_ECHO_EAR`, else the installed Ear
  in the Ear's folder (`env/bin/duck-ear`, or `env\Scripts\duck-ear.exe`).
  The Ear's folder is `DUCK_ECHO_EAR_HOME`, else
  `%LOCALAPPDATA%\DuckEcho\ear` on Windows and
  `$XDG_DATA_HOME/duck-echo/ear` (usually `~/.local/share/duck-echo/ear`)
  elsewhere. It holds `ear.log` and the answer cache.
* **One Ear, kept** (`EarKeeper`): the MCP server's `Root` holds one
  (`Root::with_ear`); a CLI command makes its own and stops it when done.
  It starts the Ear on the first question and a background thread stops
  it after 10 minutes with none. Duck Echo sends `describe_idle_seconds:
  120` in `hello`: the Ear lets go of its describe model (`llama-server`,
  and the graphics card's memory) after 2 minutes with no describe
  question, and Duck Echo sends `load` before every `describe` or `ask`,
  so starting it again has `load`'s own time limit.
* **The answer cache**: every answer is a file in `answers/` (named by the
  SHA-256 of its key, written whole then renamed). The key is the sound's
  SHA-256, the method and its parameters without the sound's path, the
  Ear's version and every model's revision. Duck Echo greets the Ear
  before every question (starting it takes well under a second; models
  load only when needed), so the revisions are always the Ear's own: a
  new model, word list or calibration misses the cache. Each section says
  `cached: true` when every answer came from it. Errors are never cached.
* **The sound heard**: the render (or the WAV or OGG file) is written as a
  16-bit WAV, at most its first 30 s, into the keeper's own temporary
  folder, and deleted as soon as it was heard; the folder goes with the
  keeper.
* **Verdicts**: the score model's `match` says yes from its `yes_from`
  and no below its `no_below`, which the Ear reports in `hello` with the
  model (0.662 and 0.338 for LAION-CLAP, set from the test set:
  `docs/ear-check.md`; 0.6 and 0.3 when an Ear gives none, as the stub);
  with the describe model's answer, both must agree for a yes or a no,
  otherwise `unsure`.
* **`duck-echo ear check`** (`ear_check.rs`): renders every sound of a
  test set, asks the Ear to score every text, and measures the parts in
  `docs/phase4-design.md` section 8 against their bars. It suggests yes
  and no lines, and writes `ear-check.json` and `ear-check.md`. When the
  Ear has a describe model (`--part all`, the default, or `--part
  describe`), it also asks what the model hears in every sound and a yes
  or no for every true and wrong text, and measures: yes on true texts,
  no on wrong texts, silence and noise, the verdict of both models with
  its mistakes, the same words when asked again of an Ear started
  afresh, and the times. `--fresh` asks the Ear everything again,
  whatever the answer cache holds, so the times are that run's own.
  `ear/tools/measure_describe_model.py` wraps it for a person: it
  downloads a size's model (after asking) and runs the check.
* **`closest`** (the Ear's own words that fit a sound best) is left out
  of `listen`'s answer and summary unless asked for with `closest: true`
  (`--closest`), and `analyze` never gives it: for short sounds it is
  often wrong (`docs/ear-check.md`).

### The real Ear (`ear/duck_ear/`)

A Python package, run as `duck-ear` (`duck_ear.server:main`). The score
model answers `score`. The describe model, when one is installed with
`llama-server`, answers `describe` and `ask`; without one they answer
`not_available`, and `hello` says why in `not_available`.

* **No PyTorch.** The models run in ONNX Runtime. The audio is prepared
  in numpy (`audio.py`: the WAV reader, a sinc resampler and the mel
  spectrogram, the same sums as the PyTorch code the models were trained
  with), and texts are cut into tokens by `tokenizers`. Three libraries,
  each needed: numpy for the sums, ONNX Runtime to run the models on any
  PC (and graphics cards later), tokenizers for the exact tokens each
  model was trained on.
* **The score model** (`score.py`): a model folder holds `audio.onnx`,
  `text.onnx`, the mel filters, the tokenizer, `reference.npy` (the
  embeddings of Duck Echo's 43 presets and examples) and `model.json`
  (source, revision, licence, settings, calibration, lines, every file's
  SHA-256). `match` comes from the text's z-score against the reference
  sounds. A sound quieter than -60 dBFS is not given to the model.
* **Converting** (`ear/convert/convert_clap.py`, PyTorch): downloads a
  model at a pinned revision, exports its audio part (from the mel
  spectrogram on) and text part to ONNX, and fails unless the Ear's path
  gives the original's embeddings (closeness 0.999 or more). Its own
  `pyproject.toml`, so the Ear never needs PyTorch.
* **Nothing leaves the PC** (`guard.py`): an audit hook refuses every
  connection and name lookup that is not to this PC.
* **The describe model** (`describe.py`): the Ear starts `llama-server`
  (llama.cpp) on `127.0.0.1` with `--offline`, a free port and its own
  log file (`llama-server.log`; never the Ear's pipes), and talks to its
  `/v1/chat/completions`. No new Python library: the standard library's
  `http.client`. The sound goes as data in the request (one channel, 16
  kHz, 16-bit, padded with silence to at least 1 second).
  * **Evidence first, in two turns.** First "What do you hear?", with no
    hint; `describe` returns those words. A question (`ask`) is a second
    turn after the model's own words. Asked in one turn, these models
    hear whatever the question names. `llama-server` keeps what it worked
    out for a sound (`cache_prompt`), so a second question about the same
    sound does not listen to it all again.
  * **The same words each time**: temperature 0, `top_k` 1, seed 42, and
    a yes or no answer forced into the shape `{"answer": "yes" | "no" |
    "partly", "reason": "..."}` (a JSON schema, which `llama-server` turns
    into a grammar).
  * **The graphics card is let go**: a thread stops `llama-server` after
    `describe_idle_seconds` (120) with no describe question; `describe`
    and `ask` then answer `model_not_loaded` until the next `load`, which
    Duck Echo sends before each of them. The Ear stops it too when its
    stdin closes.
  * **Device**: `llama-server --list-devices` names the graphics cards
    and their memory. The biggest real card is used, by name (`--device
    Vulkan0`), so a model is never spread over a processor's built-in
    graphics (which share the PC's memory and do not count as a card);
    `hello` reports it (`device`, `device_name`, `device_memory_mb`). If
    `llama-server` will not start on the card, it is started on the CPU
    (`--device none`) and `hello` says why. `DUCK_EAR_DEVICE=cpu` asks
    for the CPU, `DUCK_EAR_DEVICE=Vulkan1` for one device.
  * **Why llama.cpp**: one program with ready builds for Windows and
    Linux (Vulkan for any maker's card, or CPU only), MIT licence, and
    it runs the GGUF files these models are published as. It is started
    as a program, never linked: Duck Echo's Rust workspace gains no
    library.
  * **Models that think** before an answer in a forced shape (Gemma 4)
    have `plain_answers` in their settings: they are asked plainly and
    the Ear reads "Yes, ..." itself.
  * **Silence** (peak below -60 dBFS) is answered without the model,
    unless a request says `guard: false` (`ear check` does, to see what
    the model itself says to silence).
* **Three sizes** (`catalog.py`): Small has no describe model (scores
  only: no describe model passed the listening test, so this is what
  the Ear ships as), Full is Qwen2.5-Omni-7B, Large is
  Qwen3-Omni-30B-A3B, offered only when a graphics card with 16 GB or
  more is found. Each model's `listening_test` line says how it did. Each model is
  two GGUF files at a pinned Hugging Face revision with their sizes and
  SHA-256. `ear/tools/get_describe_model.py` (run by a person) downloads
  one, checks it, writes its `model.json`, and names it in
  `models/describe.txt`, which is the model the Ear uses
  (`DUCK_EAR_DESCRIBE_MODEL` names another folder). The installer of step
  5 takes this over.
* **`analyze` with `ear: true`** adds an `ear` section after the numbers
  (which do not change), puts the verdict on `expect.sounds_like`, and
  ends the summary with the Ear's words, marked as advice (or "stub, not
  listening"). `listen` and `ear_status` are commands
  (`commands::TABLE`), so the MCP server and `duck-echo do` have them too.

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
**"Only for this source" rules live in one table too**,
`duck-echo-core/src/applies.rs`: `duty` only for `square`, `aliasing`
for `square` and `saw`, `noise_mode` for `noise`, `pitch` for every wave
and pitched noise. Validation's `unused` warnings read it, the schema
carries each rule on its field as `x-applies-when`, and the Studio's
panel hides a setting whose rule does not hold. A test checks the table
against `Layer::has_pitch`, which the renderer uses.

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
| vorbis_rs 0.5 | audio (`ogg` feature), tools | Writes OGG Vorbis, for music sent to a game (Phase 3: about 14 times smaller than a WAV, streamed by Duck Forge). There is no mature Vorbis encoder in Rust, so it builds Xiph's `libvorbis`, with the aoTuV tuning, from C (about 37,700 lines, through `aotuv_lancer_vorbis_sys` and `ogg_next_sys`): building Duck Echo from source now needs a C compiler. BSD-3-Clause. Its default feature (a random stream serial, through `getrandom`) is off: Duck Echo makes the serial from the recipe's hash. It needs Rust 1.87, so the libraries' `rust-version` went from 1.80 to 1.87 (Duck Forge needs 1.87 as well). |
| lewton 0.10 | audio (`ogg` feature), tools | Reads OGG Vorbis: `analyze`, `compare` and `mix_check` take `.ogg`, and every OGG Duck Echo writes is decoded again and checked before it is kept. Pure Rust (about 6,400 lines, with `ogg` 0.8, `tinyvec` and `byteorder`), MIT or Apache-2.0, and the decoder Duck Forge streams music with, so Duck Echo checks its files with the code that will play them. |
| clap | cli | Argument parsing. |
| rtrb | live | A lock-free ring buffer with one writer and one reader, made for audio threads: the audio thread takes and sends messages without ever waiting on a lock or allocating. No dependencies of its own. Writing one ourselves needs `unsafe` code, which the workspace forbids. |
| eframe (egui) 0.37 | studio | The window and every widget: the same library and version as Duck Quill and Duck Forge Studio, so the three apps look alike. Its default wgpu renderer draws on Direct3D 12 (or its software fallback, WARP) on Windows and on Vulkan or OpenGL on Linux. |
| egui_kittest 0.37 | studio (tests only) | Draws every screen with no window and finds things by their words, as a person would; with wgpu it also saves pictures of the screens. The same as Duck Forge Studio. |
| tinyaudio 2 | studio | Plays the mixer's blocks through the speakers: DirectSound on Windows, ALSA on Linux, linked in the normal way (ALSA reaches PipeWire and PulseAudio too), with no `unsafe` code of ours. The lightest crate that does it, and Duck Forge's choice. Building on Linux needs `libasound2-dev`. |
| dirs 6 | studio | Where Documents and the settings folder really are: Windows' own answer (OneDrive often moves Documents), the XDG folders on Linux. The design named `windows-sys` for this, as Duck Forge uses; `windows-sys` would need `unsafe` code of ours to call Windows, and would leave Linux to us, so `dirs` (which wraps the same Windows call) does both. |
| sha2 0.10 | studio | The SHA-256 fingerprint of a downloaded update, checked against the release's `SHA256SUMS` before it is used, as Duck Quill does.  The tools crate does not use it: the `recipe_hash` in a render's facts file comes from a 60-line SHA-256 in `tools::asset` (checked against the standard test vectors), so the command line and the MCP server gain no crate for one hash per render. |
| rfd 0.18 | studio | The system's own Save As window for Export WAV: Windows' standard one, and on Linux the desktop's own through the XDG portal (spoken to over D-Bus in Rust, no system library; the AppImage still links only ALSA). Without a portal, "Export WAV to my Duck Echo folder" needs no window. |
| toml_edit 0.25 | tools | Changes values inside a TOML file's text, so a person's comments and layout survive every command and the `edit` tool. It shares its parser and writer crates (`toml_parser`, `toml_writer`, `toml_datetime`) with `toml`, so it adds one crate. |
| winresource 0.1 | studio (build only) | Puts the icon and the version into `DuckEchoStudio.exe`, as both siblings do. |
| windows-sys 0.61 | ear (Windows only) | Four Windows calls that make a job object, so stopping the Ear also stops every program it started (`llama-server`, holding the graphics card's memory), even after a crash. It was already in `Cargo.lock` through other crates. These calls need `unsafe` code, the only `unsafe` code in Duck Echo: `duck-echo-ear` denies `unsafe` (the workspace forbids it), and its one module `windows_job.rs` allows it, with a comment on why each call is safe. On Linux the same is done with the system's `kill` program and a process group, with no `unsafe` code. |
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

The MCP server and the tools crate add no external dependencies beyond
`toml_edit`: the protocol layer is plain `serde_json` (see above), and
`toml` and `toml_edit` write edited recipes. Pictures add none either: `analyze::png` is a small PNG encoder
(palette image, zlib with fixed-Huffman deflate and LZ77, CRC-32 and
Adler-32, about 240 lines plus a test-only inflater that proves round
trips), and the server's base64 is twenty lines. An image crate would pull
in a compression crate and more for one 480×254 palette image.

Synth and analyze bring no external dependencies beyond serde for the
analysis report. No `rand`, no DSP crates, no async runtime.
