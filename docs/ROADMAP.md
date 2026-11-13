# Duck Echo roadmap

Tasks for the next builder. Each task fits one session and has acceptance
criteria. Task numbers are stable names, not the order: work in the order
below, which puts real agents on Duck Echo as early as possible.

**Order:** 1 ✓, 2 ✓, 6 ✓, 7 ✓, 9 ✓, playtest 1 fixes ✓ (with 17 ✓),
then 3, 4, 5, 8, then 10, then 11 onward. 18 (note sequences) is proposed
from playtest 1; place it before 16, which it prepares.

| Step | Task | Status |
|---|---|---|
| 1 | 1. Stereo output and pan | done |
| 2 | 2. Note names | done |
| 3 | 6. Richer analysis and `compare` | done |
| 4 | 7. Presets and `variations` | done |
| 5 | 9. MCP server | done |
| 5b | Playtest 1 fixes (P1 to P6) and 17. Loudness normalization | done |
| 6 | 3. Band-limited oscillators | done |
| 7 | 4. Pitched noise and LFOs | |
| 8 | 5. Per-parameter random ranges | |
| 9 | 8. Cross-platform determinism in CI | |
| 10 | 10. Agent workflow guide | |
| 11+ | 11 to 15 as listed, then 18 (proposed), then 16 | |

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

## Playtest 1 fixes

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

Open, from the same review:

* A note-list layer for melodies (task 18, proposed below).
* A `delete` tool (it needs the same root rules as `create`, and a way to
  refuse deleting anything but recipes, WAVs and pictures).
* `attack_seconds` still measures the time to the loudest moment, so a
  layered sound whose last part is loudest reads "slow attack"; the first
  onset and `per_layer` now show the real start. Task 10's guide should say
  so.
* "Is this brightness real or aliasing?" is answered by the picture today
  and by task 3's band-limited oscillators later.

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
JSON tool calls see them; presets are levelled to per-category loudness
targets (R10) and trimmed; `preset:NAME` works wherever a recipe path does. Every preset's
render is pinned like the examples. `variations` reports `seeds_matter:
false` with a note when nothing depends on the seed, instead of writing
identical files. Conventions and how to add a preset: `presets/README.md`.
Note: presets were first peak-normalized to -3 dBFS, which left their
loudness from -4 to -17.5 LUFS; R10 re-levelled them by loudness per
category. Original task text follows.

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

### 3. Band-limited oscillators [done]

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

Original task text follows.

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

### 17. Loudness normalization in the recipe [done]

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

Original task text follows.

* Presets are levelled by a fixed `normalize_peak_db` that a script chose,
  so an edited preset drifts off its target. Add
  `master.normalize = "loudness"` with `master.loudness_lufs` and a peak
  ceiling, so a recipe keeps its level through edits.
* The render path must stay deterministic across platforms, so the K
  filters and gating need `synth::math` versions (no libm); the analysis
  crate's f64 version stays the reference and a test compares them.
* Accept: every preset expressed this way renders within 0.1 LU of target
  and below its ceiling; example renders unchanged; schema and spec
  updated.

## Agent guide

### 10. Agent workflow guide

* `docs/AGENT_GUIDE.md`: how to design a sound without hearing it. Start
  from a preset, change one thing, render, read the analysis, compare.
  Include target numbers (peak -1 to -6 dBFS, LUFS ranges for UI sounds vs
  impacts, lengths by category) and a checklist for "done".
* Use `loudness_method` in the guide: compare `k_weighted_ungated` values
  (clips under 400 ms) with each other, not with gated values.
* Use the per-category loudness targets in `presets/README.md` (impact -16,
  weapon -17, alert -18, pickup -19, movement -20, ui -22, ambient -24
  LUFS) as the guide's level targets, and explain why peak alone is not
  enough: at equal peak the presets once spanned 13 dB of loudness.
* Teach `master.normalize_lufs` set to the category target as the way to
  level a sound, `picture: true` to check texture, `per_layer` when a
  layered sound's summary describes only its loudest part, and `onsets`
  (with `note_hz`) to check melodies note by note.
* The repository's `.mcp.json` serves the tools with `playtest/` as the
  root (README, "Playtest in this repository"); use it to try the guide
  with a real agent, and commit the playtest's folder with its review.
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

### 18. Note sequences in a layer (proposed)

* From playtest 1: an arpeggio has one step time and one envelope for all
  notes, so a real melody needs one layer per note. Add an optional
  `[layers.notes]` list (`note`, `start`, `length`, optional `volume`) that
  retriggers the layer's envelope and sets its pitch per note, as a small
  step toward task 16's song timeline.
* Accept: a five-note melody in one layer reads as five onsets with the
  right `note_hz`; existing renders unchanged; schema and spec updated.

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
