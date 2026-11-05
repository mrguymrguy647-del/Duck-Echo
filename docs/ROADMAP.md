# Duck Echo roadmap

Ordered tasks for the next builder. Each task fits one session and has
acceptance criteria. Do them in order unless a dependency note says
otherwise; the first two are deliberately early because the music phase
needs them.

Before starting any task: run `cargo test --workspace`, read
`docs/ARCHITECTURE.md` and `docs/RECIPE_SPEC.md`, and keep the rules in
`CLAUDE.md`. After any change to the recipe types, regenerate the schema:

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```

## Phase 1: finish sound effects

### 1. Stereo output and per-layer pan

* Add `channels` to the recipe (`1` or `2`, default `1`) and `pan` to the
  layer (`-1..1`, default `0`, equal-power law).
* `AudioBuffer` already carries `channels`; `Voice::process` writes
  interleaved frames when stereo. Keep mono renders byte-identical to today.
* Effects run per channel with independent state (echo and reverb can take
  a `spread` later; not now).
* Accept: `cargo test` passes; a recipe with `channels = 2` and two layers
  panned left and right renders a WAV where each channel's RMS matches the
  expected layer within 0.1 dB; `analyze` reports `channels: 2` and per
  channel peaks; all mono examples produce the same bytes as before.

### 2. Note names for pitch

* `pitch.start`, `pitch.end` and arpeggio-free uses accept either a number
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

### 3. Band-limited oscillators

* PolyBLEP for square and saw (triangle via integrated square or leave
  naive). Add `aliasing = "retro" | "clean"` on the layer, default `clean`,
  so `retro` keeps today's byte-identical output.
* Accept: a 2 kHz saw with `clean` has aliased components at least 40 dB
  below the fundamental in a spectrum test (add a small FFT helper to
  `duck-echo-analyze` for tests, it will be reused in task 6); `retro`
  renders are unchanged.

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

### 6. Richer analysis for agents

* Add to `Analysis`: spectral centroid, dominant frequency over time (a few
  windows), attack time (time to reach 90% of peak), decay time (peak to
  -20 dB), crest factor, and a short `summary` string
  ("0.31 s, bright, sharp attack, no clipping").
* Add `duck-echo compare a.wav b.wav` returning the deltas and a normalized
  cross-correlation.
* Accept: tests on synthetic signals with known values; JSON output stays
  backward compatible (only additions).

### 7. Presets and `variations`

* `presets/` directory with 20 to 30 tagged recipes (ui, pickup, weapon,
  impact, ambient, alert). Each has `name` and a `# tags:` comment turned
  into a sidecar index at build time, or a small `presets/index.toml`.
* CLI `duck-echo presets list [--tag]` and `duck-echo presets show <name>`.
* CLI `duck-echo variations recipe.toml --count 8 --variation 0.4 -o dir/`
  rendering seeds 1..N and printing one analysis per file.
* Accept: every preset renders clean (extend the examples test to presets);
  list output is JSON with `--json`.

### 8. Cross-platform determinism in CI

* GitHub Actions matrix: ubuntu, macos, windows. Each job renders the
  examples and uploads SHA-256 of every WAV; a final job asserts all three
  sets match.
* Accept: workflow green on all three; a doc note listing the exact
  guarantees. If a mismatch appears, the fix belongs in `synth::math`, not
  in the test.

## Agent interface

### 9. MCP server crate (`duck-echo-mcp`)

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

### 10. Agent workflow guide

* `docs/AGENT_GUIDE.md`: how to design a sound without hearing it. Start
  from a preset, change one thing, render, read the analysis, compare.
  Include target numbers (peak -1 to -6 dBFS, LUFS ranges for UI sounds vs
  impacts, lengths by category) and a checklist for "done".
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
