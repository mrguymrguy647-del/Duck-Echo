# Duck Echo recipe specification, version 0

A recipe is a text document that fully describes one sound. The same fields
are accepted as TOML (files) and JSON (tool calls). The authoritative
definition is the Rust types in `crates/duck-echo-core/src/recipe.rs`; the
JSON Schema in `schema/recipe.schema.json` is generated from them and
carries every description, default and range below.

Conventions in this document:

* Times are seconds. Frequencies are Hz. Levels are linear unless they say
  dB. Ranges are inclusive.
* Every optional field has a default. A valid recipe can be as short as:

```toml
version = 0
[[layers]]
```

which renders a 0.205 s square-wave blip at 440 Hz.

## Top level

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `version` | integer | required | `0` | Schema version. Other values are rejected. |
| `name` | string | none | | For humans and agents; ignored by the renderer. |
| `seed` | integer | `0` | 0..2^63 | Drives the noise source and `variation`. Same recipe and seed give identical audio. |
| `sample_rate` | integer | `44100` | 8000..192000 | Output rate in Hz. |
| `channels` | integer | `1` | 1..2 | `1` mono, `2` stereo (see Stereo). |
| `variation` | number | `0` | 0..1 | How far the seed may jitter the sound (see Variation). |
| `duration` | number | computed | 0.001..60 | Force the output length. Omit to compute it. |
| `master` | table | defaults | | Master output stage. |
| `layers` | array of tables | `[]` | at least one | The sounds to mix. |
| `effects` | array of tables | `[]` | | Master effect chain, applied in order. |

### `master`

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `gain` | number | `1.0` | 0..4 | Linear gain after the master effects. |
| `normalize` | bool | `false` | | Scale the whole render so its peak equals `normalize_peak_db`. Offline only; realtime playback ignores it. |
| `normalize_peak_db` | number | `-1` | -40..0 | Target peak in dBFS. |
| `trim_silence` | bool | `false` | | Cut the render after its last sample above -60 dBFS. Offline only. Ignored, with a warning, when `duration` is set. |

Order of the offline steps: master effects, gain, normalize, clip to
-1..1, trim. Trimming only removes trailing frames, so a trimmed render is
an exact prefix of the untrimmed one, and `analyze` reports no trailing
silence for it (both use the same -60 dBFS threshold).

Output is hard-clipped to -1..1 after normalization. Analysis reports
clipping, so leave `normalize` off while designing and turn it on when a
stack of layers must not clip.

## `layers[]`

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `name` | string | none | | Label. |
| `source` | string | `"square"` | `sine`, `square`, `saw`, `triangle`, `noise` | Waveform. |
| `volume` | number | `0.8` | 0..2 | Linear level, applied after the envelope and before the filter. |
| `offset` | number | `0` | 0..30 | Start time of the layer. |
| `duty` | number | `0.5` | 0.01..0.99 | Pulse width for `square`. Other sources ignore it (warning). |
| `pan` | number | `0` | -1..1 | Stereo position, left to right. Mono recipes ignore it (warning). |
| `envelope` | table | defaults | | Volume envelope. |
| `pitch` | table | defaults | | Pitch over time. |
| `filter` | table | none | | Optional filter. |
| `effects` | array | `[]` | | Per-layer effect chain. |

Source details:

* `sine` is a pure tone.
* `square` is a pulse wave. The high and low levels are chosen so the wave
  has no DC offset and a peak of 1: 50% duty gives +1/-1, 25% gives
  +1/-0.333. Narrow pulses therefore sound thinner and quieter, as on real
  chips.
* `saw` rises from -1 to +1 each cycle. `triangle` is symmetric.
* `noise` is white noise from the seed. It ignores `pitch` (pitched noise is
  on the roadmap).
* Oscillators are naive, not band-limited. Aliasing is audible on high
  pitches, which suits retro effects; anti-aliasing is on the roadmap.

### `layers[].envelope`

Shape: rise 0→1 over `attack`, stay at 1 for `hold`, fall 1→`sustain` over
`decay`, stay for `sustain_time`, fall `sustain`→0 over `release`. All
segments are linear. The layer's length is the sum of the five times.

| Field | Default | Range |
|---|---|---|
| `attack` | `0.005` | 0..30 |
| `hold` | `0` | 0..30 |
| `decay` | `0.2` | 0..30 |
| `sustain` | `0` (level) | 0..1 |
| `sustain_time` | `0` | 0..30 |
| `release` | `0` | 0..30 |

A zero-length envelope is an error. `sustain > 0` with no `sustain_time`
and no `release` gives a warning because the sound stops abruptly.

### `layers[].pitch`

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `start` | number or note | `440` | 1..20000 Hz | Frequency at the start. |
| `end` | number or note | none | 1..20000 Hz | Frequency at the end of the sweep. Omit for a constant pitch. |
| `curve` | string | `"linear"` | `linear`, `exponential`, `ease_in`, `ease_out` | Sweep shape. |
| `sweep_time` | number | layer length | 0.001..30 | How long the sweep takes; the pitch then stays at `end`. |
| `vibrato` | table | none | | Periodic wobble. |
| `arpeggio` | table | none | | Stepped offsets. |

Note names: `start` and `end` take either a number in Hz or a string in
scientific pitch notation: a letter `A`..`G` (either case), an optional
`#` or `b`, and an octave from `-1` to `9`. `"A4"` is 440 Hz, `"C4"` is
middle C, `"C5"` is 523.25 Hz; enharmonics such as `"A#3"` and `"Bb3"` are
the same pitch. The recipe keeps what was written, so a note stays a note
when the recipe is saved again as TOML or JSON. A bad name is an
`invalid_format` error with an example and, when obvious, a suggestion
(`"H4"` suggests `"B4"`, `"C#"` suggests `"C#4"`). Conversion is exact
across platforms. Filter cutoffs take Hz only.

Curves: `linear`, `ease_in` and `ease_out` interpolate in Hz. `exponential`
interpolates in octaves (constant semitones per second), which is what a
musical sweep sounds like; prefer it for anything wider than a few
semitones.

`vibrato`: `rate` Hz (default 6, 0..100), `depth` semitones peak (default
0.5, 0..48), `delay` seconds before it starts (default 0, 0..30).

`arpeggio`: `steps` list of semitone offsets (required, 1 or more, each
-96..96), `step_time` seconds per step (default 0.05, 0.001..30), `repeat`
(default `true`; `false` holds the last step).

Final pitch per sample is `sweep(t) × 2^((arpeggio + vibrato) / 12)`,
clamped below 45% of the sample rate.

### `layers[].filter`

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `kind` | string | `"lowpass"` | `lowpass`, `highpass`, `bandpass` | Filter type (2-pole). |
| `cutoff` | number | `2000` | 10..20000 | Cutoff at the start. |
| `cutoff_end` | number | none | 10..20000 | Cutoff at the end of the sweep. |
| `curve` | string | `"linear"` | as pitch | Sweep shape. |
| `sweep_time` | number | layer length | 0.001..30 | Sweep length. |
| `resonance` | number | `0.707` | 0.1..30 | Q. `0.707` is flat; higher values peak at the cutoff. |

Cutoff is clamped to 45% of the sample rate. Coefficients update every 16
samples, which is inaudible for sweeps but means the filter is not sample
accurate.

### Stereo

With `channels = 2` the output is interleaved stereo.

* Each layer is still a mono source. Its oscillator, envelope, filter and
  its own `effects` run in mono, then `pan` places the result with an
  equal-power law: gains `sin((1 - pan)·π/4)` left and
  `sin((1 + pan)·π/4)` right. Centre gives both channels -3 dB
  (bit-identical), hard left or right gives full level on one side and
  silence on the other. Running layer effects before the pan means a
  distortion or bitcrush behaves the same whatever the pan.
* Master `effects` run on each channel with their own state, so an echo
  on the left never carries the right channel's sound.
* `gain`, `normalize` (one peak over both channels), clipping and
  `trim_silence` (a frame is silent only when both channels are) apply to
  both channels together.
* A mono recipe renders exactly as before; `channels` and `pan` change
  nothing unless `channels = 2`.

## `effects[]` (master and per layer)

Each entry has a `type` and that type's fields. Every effect has `mix` in
0..1: `0` is fully dry, `1` fully wet, in between a linear blend. Effects
run in the order written.

### `type = "echo"`

| Field | Default | Range |
|---|---|---|
| `delay` | `0.25` | 0.001..5 |
| `feedback` | `0.4` | 0..0.99 |
| `mix` | `0.35` | 0..1 |

### `type = "reverb"`

| Field | Default | Range | Meaning |
|---|---|---|---|
| `size` | `0.5` | 0..1 | Room size (scales the delay lines). |
| `decay` | `1.0` | 0.05..10 | Seconds for the tail to fall 60 dB. |
| `damping` | `0.5` | 0..1 | High-frequency loss in the tail. |
| `mix` | `0.3` | 0..1 | |

Implementation: eight damped comb filters into four allpasses (Freeverb
family), with the combs normalized so long decays are not louder than short
ones. A fully wet reverb sits roughly 6 dB under its dry input. The tail is
cut at `decay` seconds.

### `type = "distortion"`

| Field | Default | Range | Meaning |
|---|---|---|---|
| `drive` | `2` | 1..100 | Input gain before shaping. |
| `kind` | `"soft"` | `soft`, `hard`, `fold` | Smooth saturation, hard clip, wave fold. |
| `mix` | `1` | 0..1 | |

All three shapes cap the wet signal at ±1.

### `type = "bitcrush"`

| Field | Default | Range | Meaning |
|---|---|---|---|
| `bits` | `8` | 1..16 | Quantization depth. |
| `rate` | none | 100..192000 | Sample-and-hold rate in Hz. Omit to keep the full rate. |
| `mix` | `1` | 0..1 | |

## Length

The recipe length is computed unless `duration` is set:

```
layer_end  = offset + envelope length + tails of the layer's effects
recipe_end = max(layer_end) + tails of the master effects
```

Tails: echo lasts `delay × repeats` where repeats is how many times
`feedback` must multiply before falling under -60 dB; reverb lasts `decay`;
distortion and bitcrush add nothing. Each tail is capped at 10 s. The limit
for the whole recipe is 60 s, enforced by validation.

These tails are worst cases: they assume a full-level input right at the
end of the layer, so a sound that has already faded leaves silence at the
end (25 to 28% of the Phase 0 explosion, laser and powerup). The computed
length is kept as is so existing renders stay byte-identical; set
`master.trim_silence = true` to drop that silence.

## Variation

`variation` makes a single recipe a family of sounds. For each layer the
seed draws two numbers in -1..1 and applies:

* pitch `start` and `end` multiplied by `2^(variation × 3 × r₁ / 12)`
  (up to ±3 semitones at `variation = 1`);
* all envelope times multiplied by `1 + variation × 0.3 × r₂`
  (up to ±30%).

With `variation = 0` the seed only affects noise, so a tone recipe gives the
same audio for every seed. The draws happen for every layer regardless, so a
layer's jitter does not depend on earlier layers' settings.

## Validation

Validation collects every problem in one call: unknown fields, unknown
enum values, missing fields, wrong types and out-of-range values all come
back together, in document order, each with its path, line and column. Only
a syntax error stops early, because nothing after it can be trusted. Errors
stop rendering; warnings do not. The JSON form is documented in
`ARCHITECTURE.md`. Typical messages:

```
error[unknown_field] line 4:1 layers[0].pitch.strt: unknown field `strt` (got "strt") [allowed: start, end, curve, sweep_time, vibrato, arpeggio] Did you mean `start`?
error[out_of_range] line 3:1 layers[0].volume: value 5 is outside the allowed range 0..=2 (got 5) [allowed 0..=2]
warning[unused] layers[0].duty: duty only affects `square`; this layer is `sine`
```

Unknown fields are always errors: a misspelled field would otherwise be
silently ignored, which is the worst outcome for an agent.

## JSON equivalence

`examples/recipes/coin.toml` and `examples/recipes/coin.json` are the same
recipe; the test suite checks they deserialize to equal values and render to
identical bytes. Arrays of tables in TOML (`[[layers]]`) are plain arrays in
JSON (`"layers": [...]`), and `type = "echo"` becomes `"type": "echo"`.

```json
{
  "version": 0,
  "name": "coin",
  "layers": [
    {
      "source": "square",
      "volume": 0.7,
      "envelope": { "attack": 0.002, "hold": 0.06, "decay": 0.25 },
      "pitch": {
        "start": 987.77,
        "arpeggio": { "steps": [0, 5], "step_time": 0.07, "repeat": false }
      }
    }
  ]
}
```

## Examples

All in `examples/recipes/`, all render without clipping:

| File | What it shows |
|---|---|
| `coin.toml` | square, arpeggio with `repeat = false` |
| `jump.toml` | exponential pitch sweep, resonant lowpass sweep, `duty` |
| `laser.toml` | saw, fast downward sweep, vibrato, master echo, `trim_silence` |
| `explosion.toml` | noise with a lowpass sweep, sine thump, distortion, reverb, `seed`, `trim_silence` |
| `hit.toml` | bandpass noise burst plus a square drop, bitcrush with `rate` |
| `powerup.toml` | two layers with `offset`, arpeggio over a sweep, `normalize`, `trim_silence` |
| `ricochet.toml` | `channels = 2`, two layers panned left and right, master reverb |
| `chime.toml` | note-name pitches, sustain and release, vibrato with delay |

## Deferred (not in v0, never silently ignored)

These are not accepted by the schema yet; using them is an `unknown_field`
error until implemented. See `ROADMAP.md` for the order.

* Pitched noise (sample-and-hold noise at the layer pitch, NES style).
* Band-limited oscillators (anti-aliasing).
* LFOs on volume, filter cutoff and duty.
* Per-parameter random ranges (`volume = { min = 0.6, max = 0.9 }`).
* Sample layers.
* Music timeline documents and formant voice sources (phases 2 and 3).

## Versioning

`version` is `0`. New optional fields and enum variants can be added without
changing it. It increments only when the meaning of an existing field
changes, and a converter for old recipes ships with that change.
