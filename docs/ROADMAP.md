# Duck Echo roadmap

Tasks for the next builder. Each task fits one session and has acceptance
criteria. Task numbers are stable names, not the order: work in the order
below, which puts real agents on Duck Echo as early as possible.

**Order:** 1 ✓, 2 ✓, 6, 7, 9, then 3, 4, 5, 8, then 10 onward.

| Step | Task | Status |
|---|---|---|
| 1 | 1. Stereo output and pan | done |
| 2 | 2. Note names | done |
| 3 | 6. Richer analysis and `compare` | done |
| 4 | 7. Presets and `variations` | done |
| 5 | 9. MCP server | done |
| 6 | 3. Band-limited oscillators | |
| 7 | 4. Pitched noise and LFOs | |
| 8 | 5. Per-parameter random ranges | |
| 9 | 8. Cross-platform determinism in CI | |
| 10 | 10. Agent workflow guide | |
| 11+ | 11 to 16 as listed | |

Before starting any task: run `cargo test --workspace`, read
`docs/ARCHITECTURE.md` and `docs/RECIPE_SPEC.md`, and keep the rules in
`CLAUDE.md`. After any change to the recipe types, regenerate the schema:

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```

## Phase 0 review fixes

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

## Phase 1: finish sound effects

### 1. Stereo output and per-layer pan [done]

Done. Decision worth knowing: layer effects run on the mono layer signal
*before* the pan, and only master effects run per channel. Running a
nonlinear layer effect (distortion, bitcrush) after the pan would make its
sound depend on the pan position, because the pan gain changes how hard it
is driven; linear ones (echo, reverb) give the same result either way. A
per-layer `spread` for stereo echo and reverb is still open. `analyze` now
reports `per_channel` levels and sums loudness over channels. Example:
`ricochet.toml`. Original task text follows.

* Add `channels` to the recipe (`1` or `2`, default `1`) and `pan` to the
  layer (`-1..1`, default `0`, equal-power law).
* `AudioBuffer` already carries `channels`; `Voice::process` writes
  interleaved frames when stereo. Keep mono renders byte-identical to today.
* Effects run per channel with independent state (echo and reverb can take
  a `spread` later; not now). As built: master effects only, see above.
* Accept: `cargo test` passes; a recipe with `channels = 2` and two layers
  panned left and right renders a WAV where each channel's RMS matches the
  expected layer within 0.1 dB; `analyze` reports `channels: 2` and per
  channel peaks; all mono examples produce the same bytes as before.

### 2. Note names for pitch [done]

Done. Choices made: bad names use a new
`invalid_format` code rather than `unknown_variant` (a note is not an
enum), with suggestions for common slips (`H4`, missing octave, `♯`/`♭`);
note-to-Hz uses a literal semitone-ratio table and exact octave scaling,
so it is platform-independent; lower-case letters are accepted; filter
cutoffs stay Hz-only. Existing examples keep their Hz values because
rewriting them as notes would change their pinned bytes; `chime.toml`
shows note names. For the music phase: `core::note::note_number` gives
MIDI numbers, which the song timeline can reuse. Original task text
follows.

* `pitch.start` and `pitch.end` accept either a number
  (Hz) or a string note name (`"C5"`, `"A#3"`, `"Bb2"`), A4 = 440.
* Implement as a `Frequency` newtype with a custom serde impl that accepts
  both and serializes back to what it was given, so round trips preserve
  the author's choice. Keep the error message precise: an invalid string
  must report `unknown_variant`-style text with an example, not a serde
  "untagged enum" message.
* Accept: tests for `"C5"` = 523.25 Hz, flats and sharps, octave bounds;
  TOML and JSON round trips preserve strings; schema shows `number | string`
  with the pattern; a wrong note like `"H4"` gives a clear message at the
  exact path.

### 6. Richer analysis for agents [done]

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
"very bright". Original task text follows.

* Add to `Analysis`: spectral centroid, dominant frequency over time (a few
  windows), attack time (time to reach 90% of peak), decay time (peak to
  -20 dB), crest factor, and a short `summary` string
  ("0.31 s, bright, sharp attack, no clipping").
* Add `duck-echo compare a.wav b.wav` returning the deltas and a normalized
  cross-correlation.
* Accept: tests on synthetic signals with known values; JSON output stays
  backward compatible (only additions).

### 7. Presets and `variations` [done]

Done. 30 presets in `presets/` across seven categories (the six planned
plus `movement`), compiled into the tools crate so the CLI and MCP server
find them anywhere. Choices: tags live in the recipe as new metadata
fields `description` and `tags` rather than in comments or an index, so
JSON tool calls see them; presets are peak-normalized to -3 dBFS and
trimmed; `preset:NAME` works wherever a recipe path does. Every preset's
render is pinned like the examples. `variations` reports `seeds_matter:
false` with a note when nothing depends on the seed, instead of writing
identical files. Conventions and how to add a preset: `presets/README.md`.
Notes for the agent guide (task 10): preset loudness ranges from about -4
LUFS (alarm, ui_error) to -17 LUFS (explosions, metal clang) at the same
peak, which shows why peak and loudness must both be checked; the
`big_explosion` reads "tonal" because its sub-bass sine carries most of the
power. Original task text follows.

* `presets/` directory with 20 to 30 tagged recipes (ui, pickup, weapon,
  impact, ambient, alert). Each has `name` and a `# tags:` comment turned
  into a sidecar index at build time, or a small `presets/index.toml`.
* CLI `duck-echo presets list [--tag]` and `duck-echo presets show <name>`.
* CLI `duck-echo variations recipe.toml --count 8 --variation 0.4 -o dir/`
  rendering seeds 1..N and printing one analysis per file.
* Accept: every preset renders clean (extend the examples test to presets);
  list output is JSON with `--json`.

## Agent interface

### 9. MCP server crate (`duck-echo-mcp`) [done]

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
the agent loop; keep it in step with the guide. Original task text follows.

* Implement the tools designed in `ARCHITECTURE.md`: `get_schema`,
  `list_presets`, `validate`, `create`, `edit`, `render`, `analyze`,
  `compare`, `variations`. Use the official Rust MCP SDK over stdio.
  Reuse the library crates; do not shell out to the CLI.
* `edit` takes a list of `{path, value}` operations using the same path
  syntax as error messages (`layers[0].pitch.start`), applies them to the
  JSON tree, revalidates, and returns the full recipe plus the report.
* Accept: an integration test drives the server over stdio through a
  create → edit → render → analyze loop; invalid edits return the `Report`
  shape unchanged; a `README` section shows the Claude Desktop/Code config.

## Phase 1, continued: sound quality and robustness

These were planned before the agent interface and moved after task 9.

### 3. Band-limited oscillators

* PolyBLEP for square and saw (triangle via integrated square or leave
  naive). Add `aliasing = "retro" | "clean"` on the layer, default `clean`,
  so `retro` keeps today's byte-identical output.
* Accept: a 2 kHz saw with `clean` has aliased components at least 40 dB
  below the fundamental in a spectrum test (reuse the FFT in
  `duck-echo-analyze` from task 6); `retro` renders are unchanged.

### 4. Pitched noise and LFOs

* `noise` gains a sample-and-hold mode when `pitch` is set explicitly:
  add `noise_mode = "white" | "pitched"` on the layer (default `white`).
* `[layers.lfo]` with `target` (`volume`, `cutoff`, `duty`), `rate`,
  `depth`, `shape` (`sine`, `triangle`, `square`), `delay`.
* Accept: unit tests on rate and depth; spec and schema updated; new
  example `siren.toml` using an LFO on cutoff.

### 5. Per-parameter random ranges

* Any numeric field may be written as `{ min = x, max = y }`; `prepare`
  resolves it from the seed (its own forked stream, resolved in field order).
* Validation applies the field's range to both ends and requires
  `min <= max`.
* Accept: tests that a ranged field stays within bounds across 100 seeds,
  that `variation` and ranges compose, and that error paths point at
  `layers[0].volume.max`.

### 8. Cross-platform determinism in CI

* GitHub Actions matrix: ubuntu, macos, windows. Each job renders the
  examples and uploads SHA-256 of every WAV; a final job asserts all three
  sets match.
* Accept: workflow green on all three; a doc note listing the exact
  guarantees. If a mismatch appears, the fix belongs in `synth::math`, not
  in the test.

## Agent guide

### 10. Agent workflow guide

* `docs/AGENT_GUIDE.md`: how to design a sound without hearing it. Start
  from a preset, change one thing, render, read the analysis, compare.
  Include target numbers (peak -1 to -6 dBFS, LUFS ranges for UI sounds vs
  impacts, lengths by category) and a checklist for "done".
* Use `loudness_method` in the guide: compare `k_weighted_ungated` values
  (clips under 400 ms) with each other, not with gated values.
* Accept: another agent given only the guide and the MCP server produces a
  usable coin, hit and explosion in one session.

## Human interface

### 11. Simple human UI

* A small desktop app (egui is the suggested fit: pure Rust, no web stack)
  with: open/save recipe, text editor pane with validation messages inline,
  parameter sliders generated from the JSON Schema, a Render button, a
  waveform and simple spectrogram view, Play through the OS audio device
  (cpal), and seed +/- buttons.
* Playback uses `Voice` on the audio thread per the realtime rules in
  `ARCHITECTURE.md`. Clip on the audio thread.
* Accept: edit a slider, hear the change within one render; no allocation
  on the audio callback (verify with a debug assertion counter around
  `process`); the text and slider views always agree because both edit the
  same `Recipe`.

### 12. Realtime preview parameter smoothing

* Only after 11. Add `ParamUpdate` messages so slider drags update the
  running `Voice` without rebuilding it for simple fields (volume, cutoff,
  gain). Changing structure still rebuilds.
* Accept: no clicks on a volume drag in a recorded loopback test.

## Integration

### 13. Duck Forge export

* **Ask the Duck Forge owner for the audio format before writing code.**
  Needed: file format (WAV or compressed), sample rate and bit depth,
  mono/stereo, whether a manifest or asset ID file is expected, and where
  files go in a Duck Forge project. Do not guess.
* Then implement `duck-echo export recipe.toml --target duck-forge
  --project <dir>` and the `export` MCP tool: render, convert, write the
  manifest, keep the recipe next to the asset so it can be re-rendered.
* Accept: a Duck Forge project loads the exported sound; the export is
  idempotent; the recipe path or hash is recorded in the manifest.

### 14. Duck Ear bridge crate (`duck-echo-ear`)

* Implement the JSON Lines protocol from `ARCHITECTURE.md` with a stub
  Python `duck-ear` that answers `hello` and `describe` with fixed text, so
  the Rust side and the absence path can be tested without models.
* `analyze --ear` and the MCP `analyze` tool add an `ear` section when
  available and `ear: {available: false}` otherwise.
* Accept: tests for handshake, timeout, missing binary, malformed line;
  the app never blocks on the Ear for more than a configured timeout.

### 15. Sample layers

* `source = "sample"` with `[layers.sample] path`, `hash`, `start`, `loop`.
  Playback rate from the pitch curve relative to a `root` Hz. Decoding via
  `hound` (WAV only at first).
* Accept: determinism test including the hash check; a missing or changed
  file is an `invalid` error at `layers[n].sample.path`.

## Phase 2 bootstrap

### 16. Song document and scheduler

* New `Song` type in core (`version`, `tempo`, `tracks[].instrument`,
  `tracks[].notes[]`, patterns) and a `render_song` in synth that schedules
  one `Voice` per note. Note length drives `sustain_time` and `release`
  follows note-off.
* Accept: a four-bar two-track demo renders deterministically; the
  recipe schema is unchanged; `duck-echo render song.toml` works by
  detecting the document type.

## Housekeeping (any time)

* Add a `LICENSE` once chosen (MIT + Apache-2.0 dual was discussed).
* Publish crates only after the schema has been stable for a phase.
* Keep `cargo clippy --workspace --all-targets` and `cargo fmt --check`
  clean; add them to CI with task 8.
