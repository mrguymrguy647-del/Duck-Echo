# Duck Echo roadmap

Duck Echo makes sound for games and videos from text recipes, for AI agents
and for people. This file is the plan: what is done, what comes next, and
in what order. Each task should fit in one working session and says how to
know it is finished ("Accept").

Task numbers are names, not the order. Old numbers are kept so that
commits and notes that mention them still make sense.

Before starting any task: run `cargo test --workspace`, read `CLAUDE.md`,
`docs/ARCHITECTURE.md` and `docs/RECIPE_SPEC.md`, and keep their rules.
After any change to the recipe types, regenerate the schema:

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```

## The plan at a glance

| Phase | What | Status |
|---|---|---|
| 0 | Sound effects foundation: recipes, renderer, analysis, presets, MCP server, clean oscillators, Linux and Windows CI | done |
| 1 | **Music**: songs, starter instruments, seamless loops | next |
| 2 | Duck Echo Studio: an app for people, with no terminal | |
| 3 | Link with Duck Forge, the game engine | |
| 4 | Duck Ear: listening models | |
| 5 | Voices made from math only | |
| Later | Sample layers, video editor link | |

Where the unfinished Phase 0 tasks went:

* **Task 4 (pitched noise and LFOs) moved to Phase 1**, before the
  instruments: drums need pitched noise (hi-hats, snares) and pads need
  slow wobble (LFOs).
* **Task 10 (agent guide) moved to the end of Phase 1**, so it is written
  once, for sound effects and music together. Until then, the MCP
  server's instructions, the tool descriptions and `docs/ANALYSIS.md` guide
  agents.
* **Task 5 (random ranges) moved to Phase 3**: it matters most when a game
  plays the same sound many times (footsteps, hits) and wants each one a
  little different.

## Phase 0: sound effects foundation (done)

Everything an agent needs to make, check and fix a sound effect without
hearing it: the recipe format (TOML and JSON, one schema, errors with
exact paths and suggestions), a deterministic renderer, analysis with a
summary line and a picture, 30 presets levelled by loudness, an MCP server
with eleven tools, clean (band-limited) oscillators, and CI on Linux and
Windows. Done tasks: 1, 2, 3, 6, 7, 8, 9, 17, plus review fixes R1 to R10
and playtest fixes P1 to P7. Their notes are in "Phase 0 record" at the
end of this file.

## Phase 1: music (in progress)

Why music now: the Duck Craft game had to fake its music in Python because
Duck Echo had none. Phase 1 makes Duck Echo the place a game gets its
music from, with the same promises as sound effects: text in, the same
file out every time, and numbers an agent can check.

Goals for the whole phase:

* Songs are text documents, like recipes, and render deterministically.
* **44.1 kHz stereo** is the default for songs (recipes stay mono unless
  they say `channels = 2`).
* Loops join with no seam, so a game can play them forever.
* A small set of starter instruments, so an agent can write a tune
  without designing every sound first.

Order: 16, then 4, then 19, then 20, then 21, then 10.

### 16. Song document and scheduler [done]

Built as designed in `docs/phase1-design.md`; the reference is the
"Songs" part of `docs/RECIPE_SPEC.md`.

- [x] `Song` in `core::song`: `tempo`, `beats_per_bar`, `bars`, `tracks`
  (each with a required `name`, an `instrument`, `volume`, `pan`, `notes`
  and `play`), `patterns`, the song's own `instruments` and `kits`, and
  the same `master` and `effects` as a recipe. Bars count from 1, beats
  from 0. A chord is a list of notes.
- [x] `core::arrange` checks a song and flattens it into events per track.
  The renderer and the analyzer both read that list.
- [x] A note is a `Voice` (`synth::prepare_note`): pitch moved by
  `note / home`, the envelope let go when the note ends, the instrument's
  effects run once per track, the instrument levelled once.
- [x] Seeds by track name and note start sample.
- [x] `validate`, `render`, `analyze`, `compare`, `create` and `edit` take
  songs in the CLI and the MCP server, through one loader
  (`tools::load_document`). `--per-track`, `--solo NAME`, `--mute NAME`.
  `schema/song.schema.json`, `duck-echo schema song`.
- [x] Stems: a track heard alone uses the full song's master gain, so the
  stems add up to the mix (tested to 0.00001).
- [x] A song's instrument files go through `Root::read` in the MCP server
  (tested: no `..`, no absolute paths, no links out).
- [x] `examples/songs/four_bars.toml` is pinned, and CI renders it on
  Linux and Windows.

Lessons for the next builder:

* **A drum kit is its own table.** The design first put kits inside
  `instruments`. They are now `[kits.NAME]`, a map from piece name to an
  instrument reference (a string). Two tables that both hold tables cannot
  be told apart by the schema walker, and one string per piece is shorter
  to write anyway.
* **The structure walker now handles maps.** `additionalProperties` with a
  schema (what `BTreeMap<String, T>` produces) checks every value against
  it. Use `BTreeMap`, never `HashMap`, in document types, so the order is
  always the same.
* **The loader is generic.** `load.rs` runs the same four steps for
  recipes and songs. A third document type needs a `Document` impl and a
  schema, nothing else.
* **Onset pitch is rough when notes touch.** `onsets[].note_hz` reads the
  100 ms after an onset against the 100 ms before. When a note starts
  while the last one, a semitone away, is still fading, the reading can be
  half a semitone off; low notes are also rough. The demo's long notes
  were shortened a little for this. Task 21 reads the held part of each
  written note instead, which is the right tool for "is this the note I
  wrote".
* **Speed.** The four-bar demo (29 notes, 9 seconds) renders in 0.75 s in
  a release build. Song tests run in a few seconds in test builds
  (`opt-level = 1`), so no profile change was needed yet.
* **Stems are exact only for linear song effects.** Echo and reverb on
  the song are fine; distortion or bitcrush on the whole song means the
  stems no longer add up (the same limit as `per_layer`).

### 4. Pitched noise and LFOs [done]

- [x] `noise_mode = "white" | "pitched"` on a layer (default `white`).
  Pitched noise holds each random value for one cycle of the layer's
  pitch, and follows the note in a song.
- [x] `[layers.lfo]`: `target` (`volume`, `cutoff`, `duty`), `rate`,
  `depth`, `shape` (`sine`, `triangle`, `square`), `delay`. One per layer.
- [x] Unit tests on rate and depth; tests that hear the tremolo, the
  moving filter and the moving pulse; validation with paths and
  suggestions; spec and both schemas updated.
- [x] `examples/recipes/siren.toml`, pinned. Every older fingerprint is
  unchanged.

Lessons for the next builder:

* **Defaults must fit every choice.** The first depth range for `duty`
  was 0..0.45, but the default depth is 0.5, so `lfo = { target = "duty" }`
  alone was an error. The range is now 0..0.5 (the duty is clamped
  anyway). When a range depends on another field, check the default
  against each case.
* **New optional things must not touch the old path.** White noise draws
  nothing extra when the oscillator is made, and the layer loop only does
  LFO work when there is an LFO. That is why no old fingerprint moved.
* **The LFO has no state.** Its wave is worked out from the time, so it
  does not matter how a render is cut into blocks.
* **Held noise is never dark.** Even at 200 Hz it keeps a lot of hiss,
  because each step is a sharp edge. For a deep drum, put a low-pass
  filter after it or use a sine.
* An LFO restarts with each note. A slow filter sweep across a whole
  phrase is not possible yet (listed under Deferred).

### 19. Starter instruments [done]

- [x] `instruments/`: `piano`, `pad`, `bass`, `chip_lead`, and the drum
  pieces `kick`, `snare`, `hat`, `clap`, which make up the kit `drums`.
  All are plain recipes, compiled in (`tools::instruments`). See
  `instruments/README.md`.
- [x] The piano needs no new sound source: six sine layers and a noise
  hammer. It sounds like an electric piano, not a grand.
- [x] A track names one plainly (`instrument = "piano"`).
  `duck-echo instruments list|show`, MCP `list_instruments` and
  `get_instrument`. `instrument:NAME` works wherever a recipe path does,
  and in the MCP server it is checked like `preset:NAME`: a known name
  only, read-only, never a path (tested).
- [x] Every pitched instrument plays a C major scale in each octave of its
  range and reads back within 10 cents; every kit piece is found at its
  time; each is at its loudness target; all eight are pinned, and CI
  renders them on Linux and Windows.
- [x] A new precise pitch reader, `analyze::notes::spectrum_peak`, reads a
  steady tone to a fraction of a cent. Task 21 builds on it.

Lessons for the next builder:

* **Detuned layers can cancel.** The first pad had three equal saws, 5
  cents apart. Twice per beat cycle they cancelled, the note dropped away
  for a moment, and its pitch read 12 cents off. The fix is in the recipe:
  the centre layer is louder than the two side layers together, so they
  can never cancel. Keep that rule for any detuned instrument.
* **A pad cannot be tested like a piano.** It swells in slowly and rings
  for a second after each note, so in a fast scale the note before is
  louder than the new one. The scale test plays the pad in three-beat
  notes and reads the second half of each note.
* **White noise is too bright for a snare.** High-passed white noise has
  its centre near 11 kHz, brighter than the hi-hat. The snare uses a wide
  band-pass at 3.2 kHz instead.
* **Levels.** With each instrument at its target and every track at
  `volume = 1`, a five-track test mix came out within 3 LU from track to
  track. That is the reason for the targets in the table.
* **Filters do not follow the note** (Deferred). Each instrument therefore
  has a useful range, listed in the README. Outside it the instrument
  still plays but sounds dull or thin.
* **Still weak:** the piano's fade is a straight line (the envelope has no
  curved decay), so long notes sound a little flat and electric; all
  notes of an instrument have the same tone whatever their `velocity`.

### 20. Loops with no seam

* A song can say `loop = true` (and a length in bars). The render is then
  exactly that length, and anything still ringing at the end (reverb,
  echo, long releases) is wrapped round and added to the start, so the
  file can repeat forever with no click and no gap.
* Accept: the last and first samples join smoothly (no step larger than
  the music's own); `analyze` of the loop played twice finds no onset at
  the join; the length is exact to the sample.

### 21. Checking music without ears

* `analyze` on a song reports per track: the notes heard (time, pitch,
  level) next to the notes written, and any that are missing or wrong;
  plus tempo and bar lines in the picture.
* Accept: a test song with one deliberately wrong note is reported with
  that note's track, beat and both pitches.

### 10. Agent workflow guide (moved here from Phase 0, last in Phase 1)

* `docs/AGENT_GUIDE.md`: how to design a sound or a tune without hearing
  it. Start from a preset or instrument, change one thing, render, read the
  analysis, compare.
* Level targets: the per-category loudness targets in `presets/README.md`
  (impact -16, weapon -17, alert -18, pickup -19, movement -20, ui -22,
  ambient -24 LUFS), and why peak alone is not enough (at equal peak the
  presets once spanned 13 dB of loudness). Teach `master.normalize_lufs`
  set to the target as the way to level a sound.
* Teach `loudness_method` (compare short clips with short clips),
  `picture: true` for texture, `per_layer` when the summary says a recipe
  has several layers, `onsets` (with `note_hz`) for melodies, and
  `aliasing = "retro"` only when a raw chip sound is wanted.
* A music chapter: writing a song, choosing instruments, checking it with
  task 21.
* Try it with a real agent through the repository's `.mcp.json` (root
  `playtest/`), and commit the playtest folder with its review.
* Accept: another agent given only the guide and the MCP server produces a
  usable coin, hit and explosion, and a looping eight-bar tune, in one
  session.

## Phase 2: Duck Echo Studio

An app for people who never open a terminal, built on the same libraries
and commands the agents use.

Order: 25 first (the command layer every button will use, so Undo and the
MCP tools come for free instead of being added to each button later), then
22, 23, 11, 12, 24. This phase brings the first big dependencies (a window
and widget toolkit, audio output to the speakers, an installer builder);
justify each one in `ARCHITECTURE.md`, as `CLAUDE.md` asks.

### 22. Launcher and installer

* A `setup.exe` for Windows (and a matching package for other systems
  later) that installs Duck Echo Studio with a desktop icon. Starting it
  opens the app; no terminal window, no Rust needed.
* Accept: installs and starts on a clean Windows machine; uninstalls
  cleanly.

### 23. Home screen

* Sound cards for the user's recipes and songs (name, length, a small
  picture, a play button) and a gallery of presets and instruments to
  start from.
* Accept: a new user can find, play and copy a preset in under a minute.

### 11. Sliders that change the sound while it plays

* A recipe editor with sliders made from the JSON Schema, next to the text
  with validation messages inline, plus waveform and spectrogram views and
  Play through the computer's speakers.
* Playback uses `Voice` on the audio thread, following the realtime rules
  in `ARCHITECTURE.md` (no allocation and no locks there); the audio thread
  clips its own output.
* Accept: moving a slider is heard within one render; the text and slider
  views always agree because both edit the same `Recipe`; no allocation in
  the audio callback (checked by a debug counter).

### 12. Smooth live changes

* `ParamUpdate` messages let a slider drag change the running `Voice`
  (volume, cutoff, gain) without rebuilding it; structure changes still
  rebuild.
* Accept: no clicks on a volume drag in a recorded loopback test.

### 24. Song timeline

* Tracks as lanes, notes as blocks, patterns as reusable clips, a play
  head, and loop markers; edits change the song document.
* Accept: a song made in the timeline renders the same bytes as the same
  song written as text.

### 25. Every button is a command, with Undo

* Each action in the Studio is a named command (for example
  `set_layer_volume`, `add_note`) with arguments, recorded on an undo
  stack, and the same commands are MCP tools, so a person and an agent can
  work on the same project and undo each other's steps.
* Accept: every button's action can be undone and redone; each command is
  listed by the MCP server with the same name and arguments.

## Phase 3: link with Duck Forge

### 13. Save into a game, with live reload

* **Ask the Duck Forge owner for the details before writing code**: the
  sounds folder layout, any manifest or asset IDs, and how the engine
  notices changed files. Do not guess.
* Duck Echo saves a sound (or a song) straight into a game's sounds
  folder, with its recipe beside it so it can be re-rendered, and the
  running game reloads it live.
* Write down what the engine needs from Duck Echo, then build it:
  * **per-sound volume** (a level the engine applies, matched to the
    loudness targets);
  * **real stereo** (stereo files played as stereo, not folded to mono);
  * **better resampling** when the engine's mix rate is not the file's;
  * **OGG** (or another compressed format) for music, since songs are
    long.
* Accept: a Duck Forge project hears a sound change within a second of
  saving it in Duck Echo; the export is idempotent; the recipe path or hash
  is recorded with the asset.

### 5. Random ranges (moved here from Phase 0)

* Any number in a recipe may be written as `{ min = x, max = y }`;
  `prepare` picks the value from the seed (its own stream, in field
  order). With Duck Forge, a game can ask for a fresh variant each time a
  sound plays.
* Validation applies the field's range to both ends and requires
  `min <= max`.
* Accept: a ranged field stays within bounds across 100 seeds; `variation`
  and ranges work together; errors point at paths like
  `layers[0].volume.max`.

## Phase 4: Duck Ear

### 14. Duck Ear bridge (`duck-echo-ear`)

* The JSON Lines protocol from `ARCHITECTURE.md`, with a stub Python
  `duck-ear` that answers `hello` and `describe` with fixed text, so the
  Rust side and the "Ear not installed" path can be tested without models.
  Then real listening models behind the same protocol.
* `analyze --ear` and the MCP `analyze` tool add an `ear` section when the
  Ear is available, and `ear: {available: false}` otherwise.
* Accept: tests for handshake, timeout, missing program and a broken line;
  the app never waits on the Ear longer than a set timeout.

## Phase 5: voices from math only

* Formant synthesis (see "Voice (roadmap Phase 5)" in `ARCHITECTURE.md`):
  gibberish and robot voices first, invented natural voices later.
* **Never copy a real person's voice**: no cloning, no recordings of
  people as a source, no "sounds like" a named person.
* Accept for the first step: a recipe makes a short gibberish phrase with
  vowels an analysis can tell apart; every render deterministic.

## Later

* **15. Sample layers:** `source = "sample"` with `[layers.sample]` (`path`,
  `hash`, `start`, `loop`), playback rate from the pitch curve; a missing
  or changed file is an error at `layers[n].sample.path`.
* **Video editor link:** render sounds and music to a video's timeline
  (cue points from the editor, files back at the right frames).
* A `delete` tool for the MCP server (same root rules as `create`, only
  recipes, WAVs and pictures).
* Housekeeping: add a `LICENSE` once chosen (MIT + Apache-2.0 dual was
  discussed); publish crates only after the schema has been stable for a
  phase; move the CI's `actions/checkout`, `upload-artifact` and
  `download-artifact` to their Node 24 versions (GitHub warns that the v4
  ones run on deprecated Node 20).

## Phase 0 record

What was built in Phase 0, with the choices made and the lessons for
later work. The original task texts are in the git history.

### 1. Stereo output and per-layer pan

Done. Decision worth knowing: layer effects run on the mono layer signal
*before* the pan, and only master effects run per channel. Running a
nonlinear layer effect (distortion, bitcrush) after the pan would make its
sound depend on the pan position, because the pan gain changes how hard it
is driven; linear ones (echo, reverb) give the same result either way. A
per-layer `spread` for stereo echo and reverb is still open. `analyze` now
reports `per_channel` levels and sums loudness over channels. Example:
`ricochet.toml`.

### 2. Note names for pitch

Done. Choices made: bad names use a new
`invalid_format` code rather than `unknown_variant` (a note is not an
enum), with suggestions for common slips (`H4`, missing octave, `♯`/`♭`);
note-to-Hz uses a literal semitone-ratio table and exact octave scaling,
so it is platform-independent; lower-case letters are accepted; filter
cutoffs stay Hz-only. Existing examples keep their Hz values because
rewriting them as notes would change their pinned bytes; `chime.toml`
shows note names. For the music phase: `core::note::note_number` gives
MIDI numbers, which the song timeline can reuse.

### 3. Band-limited oscillators

Done: `layers[].aliasing`, `clean` by default. Choices and numbers:

* **A 4-point B-spline BLEP, not the usual 2-point PolyBLEP.** Measured on
  a 2 kHz saw at 44.1 kHz, the worst folded tone (at 20.1 kHz, from the
  24 kHz harmonic) is -21.6 dB raw, -31.0 dB with 2-point PolyBLEP and
  -40.4 dB with the cubic B-spline correction, so only the wider one meets
  the 40 dB goal. A 2 kHz square measures -44.9 dB. The price is a softer
  top end (-3 dB at 10 kHz, -7 dB at 15 kHz). Getting both flat highs and
  40 dB would need oversampling or a long filter with delay; not worth it
  for a light, realtime-ready oscillator.
* **No memory and no delay.** The correction is worked out from the phase
  and pitch step alone (`dsp::blep_near`), so `Voice::process` stays
  allocation-free and nothing is delayed against the envelope.
* **Triangle stays raw:** it has no jumps, so it aliases far less.
* **Examples stay retro, presets go clean.** Every square and saw layer in
  `examples/recipes/` says `aliasing = "retro"`, so the examples still
  match their first renders and pin the retro path. The 14 presets with a
  square or saw layer changed sound and were re-pinned; 8 of them also
  needed a new `normalize_peak_db` to stay within 0.1 LU of their target
  (coin moved most, 0.48 LU, because clean high notes lose the folded
  tones' energy). The other 16 presets did not change.
* Tests: `clean_oscillators.rs` (spectrum, saw and square, and that the
  same measurement catches retro), plus unit tests in `dsp.rs`.

### 6. Richer analysis for agents

Done. `analyze` gained `summary`, `crest_factor_db`, `attack_seconds`,
`decay_seconds`, `spectral_centroid_hz`, `tonality` and `pitch_track`;
`compare` gives b-minus-a deltas, a cross-correlation with lag, and a
summary line. Every field and threshold is defined in `docs/ANALYSIS.md`.
Choices: `tonality` (share of power in prominent spectral peaks) was added
because a dominant frequency means nothing for noise and the summary needs
to say which it is; decay starts at the last moment at 90% so held notes
are not counted; `compare` only gives a loudness delta when both sides use
the same loudness method. The FFT lives in `duck_echo_analyze::fft` for
task 3 to reuse. Both commands accept recipes as well as WAV files. A new
`duck-echo-tools` crate holds operations shared with the MCP server.
Known limits, worth knowing when writing the agent guide: `attack_seconds`
measures time to the loudest moment, so a sound whose second layer is
loudest reads as a slow attack (powerup); naive square and saw waves read
"very bright".

### 7. Presets and `variations`

Done. 30 presets in `presets/` across seven categories (the six planned
plus `movement`), compiled into the tools crate so the CLI and MCP server
find them anywhere. Choices: tags live in the recipe as new metadata
fields `description` and `tags` rather than in comments or an index, so
JSON tool calls see them; presets are levelled to per-category loudness
targets (R10) and trimmed; `preset:NAME` works wherever a recipe path does. Every preset's
render is pinned like the examples. `variations` reports `seeds_matter:
false` with a note when nothing depends on the seed, instead of writing
identical files. Conventions and how to add a preset: `presets/README.md`.
Note: presets were first peak-normalized to -3 dBFS, which left their
loudness from -4 to -17.5 LUFS; R10 re-levelled them by loudness per
category.

### 8. Cross-platform determinism in CI

Done. `.github/workflows/ci.yml` runs on every push: on Linux and Windows
it runs clippy (warnings are errors) and every test (and `cargo fmt` on
Linux), then renders every example and preset with a release build
(`scripts/render-fingerprints.sh`) and uploads the SHA-256 of each WAV; a
last job compares the two lists. The guarantees are written down in
`ARCHITECTURE.md`, "What is guaranteed to match".

First run (run 1, commit 610bc5b): all tests passed on both, and the
Linux and Windows lists (40 files each: 10 example files, 30 presets) were
identical, so no fix in `synth::math` was needed. Windows also passed the
golden tests, which compare unoptimized renders with fingerprints made by
a release build on Linux.

Choices: macOS was left out (the plan said Linux and Windows); add it as
a third matrix entry when needed. `.gitattributes` keeps LF line endings on
Windows checkouts, and the MCP server shows paths with `/` on every
platform, so the same tests pass on both. Lesson: keep the render path to
`+ - * /`, `sqrt`, `floor` and bit tricks (see `synth::math`); one call to
the platform's `sin` or `powf` there would break this check.

### 9. MCP server crate (`duck-echo-mcp`)

Done: ten tools over stdio, documented in `ARCHITECTURE.md`, with setup for
Claude Code and Claude Desktop in the README. Choices that differ from the
plan below:

* **No SDK.** The stdio protocol layer is written directly on `serde_json`
  instead of the official Rust MCP SDK, which is async and would add tokio
  and a large dependency tree. Only `protocol.rs` would change to adopt the
  SDK later, for example to add resources or HTTP transport.
* **`get_preset` added**, and `list_presets` returns metadata only, so a
  listing stays small.
* **`--root`** confines all file access to one directory, since clients
  like Claude Desktop give the server no other sandbox.
* **`export` waits for task 13.**

Shared operations (presets, variations, edits, render-to-file) live in
`duck-echo-tools` so the CLI and server behave the same. For task 10: the
`instructions` string sent at `initialize` is the shortest description of
the agent loop; keep it in step with the guide.

### 17. Loudness normalization in the recipe

Done with the playtest 1 fixes, as `master.normalize_lufs` (a number, so
there is no `normalize = "loudness"` mode). Choices that differ from the
plan below:

* **The peak ceiling is fixed at -1 dBFS** (`core::LOUDNESS_CEILING_DBFS`)
  instead of using `normalize_peak_db`. Presets set `normalize_peak_db` to
  whatever peak hits their target (-9.4 for coin), so using it as the
  ceiling capped an agent who added `normalize_lufs = -16` to a preset at
  the preset's old level: the first MCP test of the feature failed that
  way. `normalize` and `normalize_peak_db` are now unused while
  `normalize_lufs` is set, with an `unused` warning on `normalize = true`.
  A target the ceiling does not allow renders quieter with a `suspicious`
  render warning.
* **It measures the finished render** (scaled, clipped, trimmed) and
  corrects over up to four passes, because trimming and the short-clip
  active span move with the gain; the result is within 0.1 LU of the target
  as `analyze` reads the 16-bit WAV.
* **Presets keep their peak levels** (`normalize_peak_db`), so their pinned
  renders are unchanged; the test
  `normalize_lufs_hits_every_category_target_in_one_render` adds
  `normalize_lufs` to every preset as written and checks it lands on its
  category target. Moving presets to `normalize_lufs` would change every
  preset fingerprint; do it only on purpose.
* The deterministic meter is `synth::loudness`; the test
  `render_loudness_meter_matches_the_analyzer` holds it within 0.001 LU of
  the analyzer on every preset and example, at 44.1 and 48 kHz.

### Phase 0 review fixes (R1 to R10)

Found in a review of Phase 0 and fixed before task 1.

- [x] **R1. Report every problem in one validate call.** Validation used to
  stop at the first structural (serde) error, so a recipe with four typos
  and two bad values took five calls. A schema-driven walker
  (`core::structure`) now reports all unknown fields, enum values, missing
  fields and type mismatches with paths, lines and suggestions, and range
  checks run in the same call on a pruned copy. Range errors now carry
  lines too. Test: `one_call_reports_all_six_mistakes_in_toml_and_json`.
  Lesson for later tasks: new recipe types need no walker changes unless
  schemars starts emitting a new keyword, which a unit test catches.
- [x] **R2. Loudness for short sounds.** `analyze` returned
  `loudness_lufs: null` under 400 ms, which covers most sound effects.
  Clips under 400 ms now get an ungated K-weighted loudness over their
  active span (first to last sample above -60 dBFS), and the output names
  the method in `loudness_method` (`bs1770_gated` or
  `k_weighted_ungated`). For a steady tone the two methods agree within
  0.2 dB. Loudness is computed per channel and summed, ready for stereo.
  Note for task 10: the Phase 0 coin reads about -5 LUFS, far louder than
  typical UI targets, so the guide's targets will drive preset levels.
- [x] **R3. Trailing silence.** Computed lengths left 25 to 28% silence
  at the end of explosion, laser and powerup because effect tails are
  worst-case estimates. Added opt-in `master.trim_silence` (cuts after the
  last sample above -60 dBFS, offline, after clipping). The default path
  is unchanged and pinned by `default_render_path_is_byte_identical_to_phase0`,
  which fingerprints every example's WAV against Phase 0. The three
  examples now use trimming. Use that golden test to prove later changes
  (stereo, note names) leave existing renders alone.
- [x] **R4. Coin comment.** `coin.toml` said "a fifth" for steps
  `[0, 5]`, which is a fourth.
- [x] **R5. Check inside misspelled entries.** An unknown key or effect
  `type` with a confident suggestion (`[[layer]]` for `layers`, `"delay"`
  for `echo`) now has its contents validated as the suggested name in the
  same call, reported at the written path; range checks run on it too.
  Confidence comes from `load::suggest`; weak guesses only suggest.
  Test: `misspelled_entries_are_checked_as_the_suggested_name`.
- [x] **R6. Quoted numbers.** `"440"` or `"440hz"` for a pitch (and a
  quoted number for any numeric field) now suggests the bare number.
- [x] **R7. Top-level keys under a table.** Writing `seed = 4` after
  `[master]` in TOML makes it `master.seed`; the error now says the field
  is top-level and must go above the first table header. Found while
  writing the presets, where the template made exactly this mistake.
- [x] **R8. MCP write paths could leave the root (security).** The
  `preset:` prefix skipped the root guard, so `render` with output
  `preset:/../../x.wav` wrote outside the root, and `render` overwrote an
  unrelated `important.txt`; `create` and `variations` had the same hole,
  and `variations` also used the recipe `name` in file names. Fixed in
  `duck-echo-mcp/src/root.rs`: separate read and write resolvers;
  `preset:` only for reads and only with a known name; no `..` or absolute
  paths; symbolic links resolved and kept inside the root; render writes
  only `.wav`; names made file-safe. Tests:
  `write_paths_cannot_leave_the_root`, `read_paths_cannot_leave_the_root`.
  Lesson: every new tool argument that names a file must go through
  `Root::read` or `Root::write`, never `PathBuf::from`.

- [x] **R9. Summary words on noisy sounds.** Tonality used raw power, so
  a low thump dominated: the shotgun read "tonal" and big_explosion read
  "pitch rising" from a window 45 dB below its loudest. Tonality is now
  A-weighted, measured at 46 ms and 186 ms (the longer frame keeps low
  harmonic tones such as engine_hum tonal), the "tonal" bar moved from 0.7
  to 0.6 because weighting lowers fast sweeps, and pitch words ignore
  windows more than 20 dB below the loudest. Shotgun, both explosions and
  missile now read noisy; hit and land (noise bursts over short tones)
  read "tone and noise"; every pitched preset keeps "tonal". Test:
  `summary_words_match_what_the_presets_are`. Analysis only; no render
  changed.
- [x] **R10. Presets levelled by loudness.** At -3 dBFS peak the presets
  ranged from -4 LUFS (ui_error, alarm) to -17.5 LUFS (explosions), so
  feedback sounds were 13 dB louder than explosions. Each preset's
  `normalize_peak_db` is now set so it hits its category's loudness
  target within 0.25 LU (table in `presets/README.md`, constants in
  `presets::LOUDNESS_TARGETS`), with at least 1 dB of peak headroom. All
  preset fingerprints were re-pinned on purpose; example renders are
  unchanged. Tail-heavy presets needed a second pass because
  `trim_silence` uses an absolute -60 dBFS threshold, so a quieter render
  is trimmed sooner. Follow-up: task 17.

### Playtest 1 fixes (P1 to P7)

An agent made eight platformer sounds through the MCP tools only and
reviewed the experience (`playtest/REVIEW.md` on branch `playtest-1`,
rated 8/10). Its findings, fixed together:

- [x] **P1. A picture of the sound.** `render` and `analyze` take
  `picture: true` (CLI `--picture out.png`) and return a 480×254 PNG as an
  MCP image block: level over time (peak, RMS, onset ticks) above a
  log-frequency spectrogram, with a `picture` object describing the axes.
  Drawn in `analyze::picture`, encoded by the in-house `analyze::png` (no
  new dependency). Every analysis also gained `energy_over_time`, RMS in
  20 slices. The reviewer could not tell whether coin's brightness was
  aliasing; the picture shows the alias lines.
- [x] **P2. Per-layer analysis and onsets.** `per_layer`, `solo` and `mute`
  on `render` and `analyze` (CLI `--per-layer`, `--solo 0,2`, `--mute 1`).
  Layers are isolated after `prepare` (`Voice::keep_layers`) and finished
  with the mix's gain and length (`synth::render_layers`), so a layer reads
  as it sounds in the mix. Every analysis gained `onsets`: the start of
  the sound, then spectral-flux peaks, each with the pitch that starts
  there (`note_hz`, the strongest *new* frequency, so a melody over a bass
  reads as the melody). The summary says "N onsets". On the playtest's
  level_complete jingle, one `analyze` call with `per_layer` now gives six
  onsets in the mix (0.1 s steps, the chord at 0.55 s) and reads the lead
  as C5 E5 G5 C6 E6 and the chord as C6, E6 and G6, which the reviewer
  could not verify. Tests pin the note counts of the presets.
- [x] **P3. `master.normalize_lufs`.** Task 17, below.
- [x] **P4. `list_files`.** Lists the server root or a folder in it with
  kinds, sizes and WAV lengths; hidden files skipped, links not followed.
  Logic in `tools::files`. No delete tool yet (the review asked for one
  too; see Open below).
- [x] **P5. Error polish.** `Code::Other` is gone: new codes
  `duplicate_field`, `not_found`, `already_exists`, `path_refused`, `io`,
  `invalid_argument`, and argument errors reuse `type_mismatch`,
  `out_of_range`, `unknown_variant`. `ToolError::Io(String)` became
  `ToolError::Failed(Issue)`; front ends name the argument with
  `ToolError::at`, so no MCP error has an empty path. Shared messages name
  no CLI command (a missing preset lists every name in `allowed`), a
  missing file suggests the closest name and `list_files`, and the server
  replaces its absolute root in every message. Tool descriptions and the
  server instructions explain `isError` (validate answers a bad recipe
  with `isError: false`, `ok: false`) and the two loudness methods. Test:
  `every_error_has_a_code_a_path_and_no_absolute_path` calls each tool
  wrongly 29 ways.
- [x] **P6. Playtest results are committed.** `.gitignore` no longer
  ignores `playtest/`, and WAVs under it are not ignored either. Give each
  playtest its own folder (README, "Playtest in this repository").

Lessons for later tasks:

* Onsets were tuned on every preset, example and playtest sound; the
  thresholds are in `analyze::timeline` with the numbers that chose them.
  If a new source (pitched noise, LFOs in task 4) adds false onsets, check
  the share of new spectrum and the frame tonality first, and add the
  preset to `onsets_count_the_notes_of_each_preset`.
* Isolating layers by removing them from the `Voice` keeps seed streams
  and jitter only because both are drawn per layer index in `prepare`.
  Keep it that way, or solo renders will stop matching the mix.
* A new failure gets a new `Code`, never a catch-all; and every tool
  argument that names a file goes through `Root::read`, `Root::write` or
  `Root::folder` with its argument name.

- [x] **P7. Two small agent helps** (after the playtest). The summary of
  a recipe with several layers ends with "N layers; per_layer describes
  each", because the rest of the summary describes only the whole mix;
  with `solo`/`mute` it says "K of N layers heard". `list_presets` with a
  tag no preset has returns a `not_found` error suggesting the closest
  real tag, instead of an empty list. Tests: `agent_hints.rs` and
  `tag_typos_and_layer_counts_help_the_agent`.

Open, from the same review:

* A note-list layer for melodies (proposed as task 18; now part of task
  16, the song document, in Phase 1).
* A `delete` tool (it needs the same root rules as `create`, and a way to
  refuse deleting anything but recipes, WAVs and pictures).
* `attack_seconds` still measures the time to the loudest moment, so a
  layered sound whose last part is loudest reads "slow attack"; the first
  onset and `per_layer` now show the real start. Task 10's guide should say
  so.
* "Is this brightness real or aliasing?" is answered by the picture, and
  since task 3 the default `clean` oscillators keep folded tones more than
  40 dB down.
