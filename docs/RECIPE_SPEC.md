# Duck Echo recipe specification, version 0

This document covers both kinds of document: the **recipe** (one sound),
and the **song** (music made of recipes; see "Songs").

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
| `description` | string | none | | One sentence for humans and agents; ignored by the renderer. |
| `tags` | array of strings | `[]` | | Search tags such as `["pickup", "bright"]`; ignored by the renderer. Presets put their category first. |
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
| `normalize_lufs` | number | none | -60..0 | Scale the render so its loudness, as `analyze` measures it, is this many LUFS. Replaces `normalize` and `normalize_peak_db`; the peak never goes above -1 dBFS. Offline only. |
| `trim_silence` | bool | `false` | | Cut the render after its last sample above -60 dBFS. Offline only. Ignored, with a warning, when `duration` is set. |

Order of the offline steps: master effects, gain, normalize (by peak or by
loudness), clip to -1..1, trim.

`normalize_lufs` levels a sound to a loudness target in one render, so a
recipe keeps its level through edits; set it to the category target that
`list_presets` reports (`target_lufs`). It measures the finished render the
way `analyze` does (gated BS.1770 from 400 ms, the ungated short-clip
method below; see `ANALYSIS.md`) and lands within 0.1 LU of the target as
measured from the written 16-bit WAV. Rules:

* The gain never puts the peak above -1 dBFS. A target that would need
  more renders quieter, and `render` (and `analyze` of the recipe) returns
  a `suspicious` warning at `master.normalize_lufs` with the loudness
  reached. A sine peaking at -1 dBFS measures about -4 LUFS, so very loud
  targets only suit full, steady sounds.
* `normalize` and `normalize_peak_db` have no effect while it is set;
  `normalize = true` next to it gives an `unused` warning, so a preset
  edited to use it says which line to remove.
* Silence (or a long sound entirely under -70 LUFS) is left as it is.
* It is deterministic like the rest of the render path: the meter used
  while rendering avoids platform math (see `ARCHITECTURE.md`).

Trimming only removes trailing frames, so a trimmed render is an exact
prefix of the untrimmed one, and `analyze` reports no trailing silence for
it (both use the same -60 dBFS threshold).

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
| `aliasing` | string | `"clean"` | `clean`, `retro` | How `square` and `saw` handle high notes (see below). Other sources sound the same either way (`retro` on them gives a warning). |
| `noise_mode` | string | `"white"` | `white`, `pitched` | What kind of noise `noise` makes (see below). Other sources ignore it (warning). |
| `pan` | number | `0` | -1..1 | Stereo position, left to right. Mono recipes ignore it (warning). |
| `envelope` | table | defaults | | Volume envelope. |
| `pitch` | table | defaults | | Pitch over time. |
| `filter` | table | none | | Optional filter. |
| `lfo` | table | none | | Optional slow wave that moves the volume, the cutoff or the duty. |
| `effects` | array | `[]` | | Per-layer effect chain. |

Source details:

* `sine` is a pure tone.
* `square` is a pulse wave. The high and low levels are chosen so the wave
  has no DC offset and a peak of 1: 50% duty gives +1/-1, 25% gives
  +1/-0.333. Narrow pulses therefore sound thinner and quieter, as on real
  chips.
* `saw` rises from -1 to +1 each cycle. `triangle` is symmetric.
* `noise` is noise from the seed. With `noise_mode = "white"` (default)
  it is plain hiss, a new random value every sample, and it ignores
  `pitch`. With `noise_mode = "pitched"` it picks a new random value
  `pitch` times per second and holds it in between, like the noise channel
  of old game consoles. A low pitch gives a rough, crunchy sound and a
  high pitch gives hiss; it follows pitch sweeps, and in a song it follows
  the note. Good for chip-style drums and hi-hats.
* `aliasing` chooses how `square` and `saw` treat high notes. A raw square
  or saw jumps from one level to another in a single sample, and that jump
  has energy above half the sample rate, which folds back down as harsh
  extra tones ("aliasing") that are not part of the note.
  * `clean` (default) smooths every jump over 2 samples on each side with
    a cubic B-spline curve (a "BLEP" correction). On a 2 kHz saw at
    44.1 kHz the folded tones are more than 40 dB under the note (they are
    about 22 dB under it with `retro`). The cost is a gentler top end:
    about -3 dB at 10 kHz and -7 dB at 15 kHz. It adds no delay and needs
    no memory, so it is safe for realtime playback later.
  * `retro` keeps the raw wave, bright and gritty like old sound chips. It
    is exactly how Duck Echo rendered before this option, so a recipe
    marked `retro` renders byte for byte as it did then.
  * `sine` and `triangle` stay as they are in both modes (a triangle has
    no jumps, so it aliases very little), and so does `noise`.

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

### `layers[].lfo`

An LFO ("low-frequency oscillator") is a slow wave that moves one setting
while the sound plays. One per layer. It starts when the layer starts, so
in a song it starts again with each note.

| Field | Type | Default | Range | Meaning |
|---|---|---|---|---|
| `target` | string | `"volume"` | `volume`, `cutoff`, `duty` | What the wave moves. `cutoff` needs a `filter` on the layer and `duty` needs `source = "square"` (errors otherwise). |
| `rate` | number | `5` | 0.01..50 | Cycles per second. |
| `depth` | number | `0.5` | by target | How far it moves; see below. |
| `shape` | string | `"sine"` | `sine`, `triangle`, `square` | The wave. `square` jumps between the two ends. |
| `delay` | number | `0` | 0..30 | Seconds before the wave starts. |

What `depth` means:

* `volume` (0..1): the level swings between full and `1 - depth` of full.
  It starts at full, so nothing jumps when the wave begins. `depth = 1`
  goes all the way to silence. This is a tremolo.
* `cutoff` (0..4): octaves up and down from the filter's cutoff (after its
  own sweep). `depth = 1` moves between half and double the cutoff. It
  starts at the cutoff and goes up first. Updated every 16 samples, like
  the filter.
* `duty` (0..0.5): added to and taken from `duty`; the result is kept
  inside 0.01..0.99. It starts at `duty` and goes up first. This is
  pulse-width movement, which makes a square wave sound thicker.

```toml
[[layers]]
source = "saw"
filter = { cutoff = 800, resonance = 2.0 }
lfo = { target = "cutoff", rate = 1.0, depth = 2.0 }   # a slow "wah"
```

Pitch has its own wave already: `pitch.vibrato`.

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
a syntax error stops early, because nothing after it can be trusted. A
misspelled table or effect type that is clearly a typo (`[[layer]]`,
`type = "delay"`) has its contents checked as the intended name, so typos
inside it show up in the same call. Numbers written in quotes (`"440"`,
`"440hz"`) are rejected with a suggestion to drop the quotes. Errors
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

## Songs

A **song** is a second kind of document: music written as text. A document
with `tracks` at the top is a song; a document with `layers` is a recipe.
The same commands and tools take both. The Rust types are in
`crates/duck-echo-core/src/song.rs`, and the generated schema is
`schema/song.schema.json` (`duck-echo schema song`).

Every note plays an **instrument**, and an instrument is a recipe. Nothing
about recipes changes.

```toml
version = 0
tempo = 110            # beats per minute
bars = 4               # 4 beats in a bar by default

[patterns.walk]        # a named group of notes
notes = [
  { beat = 0, note = "C2" },
  { beat = 1, note = "E2" },
  { beat = 2, note = "G2" },
  { beat = 3, note = "E2" },
]

[instruments.bass]     # a full recipe, written inside the song
version = 0
[[instruments.bass.layers]]
source = "saw"
envelope = { attack = 0.005, decay = 0.25, sustain = 0.5, release = 0.08 }
pitch = { start = "C2" }

[[tracks]]
name = "bass"
instrument = "bass"
play = [
  { pattern = "walk", bars = [1, 2, 4] },
  { pattern = "walk", bars = [3], transpose = 5 },
]
notes = [{ bar = 4, beat = 3.5, note = "G2", length = 0.5 }]
```

`examples/songs/four_bars.toml` is a complete two-track song.

### Song top level

| Field | Type | Default | Range / notes |
|---|---|---|---|
| `version` | integer | required | must be `0` |
| `name`, `description`, `tags` | | none | metadata, not used by the renderer |
| `tempo` | number | required | beats per minute, 20..=400 |
| `beats_per_bar` | integer | `4` | 1..=16 |
| `bars` | integer | computed | length in bars, 1 or more; when omitted the song ends at the bar line after its last note |
| `seed` | integer | `0` | see "Song seeds" |
| `sample_rate` | integer | `44100` | 8000..=192000 Hz |
| `channels` | integer | `2` | `1` or `2`. Songs are stereo by default (recipes are mono) |
| `master` | table | no change | the same `master` table as a recipe, for the whole song |
| `effects` | array | `[]` | song-wide effects, the same list as a recipe's; run once on the mix |
| `instruments` | table of recipes | none | the song's own instruments, by name |
| `kits` | table | none | the song's own drum kits, by name |
| `patterns` | table | none | named groups of notes |
| `tracks` | array | required | at least one, at most 64 |

Limits: a song is at most 600 s long and has at most 20000 notes (after
patterns are placed and chords are split).

### `tracks[]`

| Field | Type | Default | Range / notes |
|---|---|---|---|
| `name` | string | required | unique in the song. Used in reports, by `solo` and `mute`, and for the track's seed |
| `instrument` | string | required | see "Instruments" |
| `volume` | number | `1` | 0..=2 |
| `pan` | number | `0` | -1 (left) ..= 1 (right). A centred track passes through unchanged; panning fades the far side out and raises the near side by up to 3 dB (equal power) |
| `notes` | array | `[]` | notes written on the track |
| `play` | array | `[]` | patterns placed at bars; added to `notes` |

### Notes

`tracks[].notes[]`:

| Field | Type | Default | Range / notes |
|---|---|---|---|
| `beat` | number | required | when the note starts, in beats after the bar line. `0` is on the line, `0.5` half a beat later |
| `bar` | integer | `1` | the bar `beat` is counted from. **Bars count from 1.** With no `bar`, `beat` counts from the start of the song |
| `note` | name, number or list | required | a note name (`"C5"`), a frequency in Hz (1..=20000), a list of these for a chord, or on a kit track the name of a piece (`"kick"`) |
| `length` | number | `1` | how long the note is held, in beats, more than 0. Not used by kit pieces |
| `velocity` | number | `1` | 0..=1; multiplies the note's volume (0.5 is 6 dB quieter) |

`{ beat = 9, note = "A5" }` and `{ bar = 3, beat = 1, note = "A5" }` are
the same note in a 4-beat bar. A note may ring past the end of the song
but must start before it.

`patterns.NAME.notes[]` has `beat`, `note`, `length` and `velocity` (no
`bar`); beats count from the bar the pattern is placed at, and beat
`beats_per_bar` is the first beat of the next bar.

`tracks[].play[]`:

| Field | Type | Default | Range / notes |
|---|---|---|---|
| `pattern` | string | required | a name in `patterns` |
| `bars` | array of integers | required | the bars it starts at, counted from 1 |
| `transpose` | integer | `0` | semitones up or down, -48..=48. Not used by kit pieces |

### Instruments

A track's `instrument` is one of:

* A name in the song's `instruments` (a recipe) or `kits`.
* `"preset:NAME"`: a built-in preset.
* A recipe file ending in `.toml` or `.json`, relative to the folder the
  song file is in. In the MCP server the path is checked like every other
  path: no absolute paths, no `..`, nothing outside the server root.

A **kit** maps piece names to instruments, and its notes name a piece:

```toml
[kits.drums]
kick = "preset:thud"     # a preset, a recipe file, or a name in `instruments`
hat = "tick"

[[tracks]]
name = "drums"
instrument = "drums"
notes = [{ beat = 0, note = "kick" }, { beat = 0.5, note = "hat", velocity = 0.5 }]
```

### How a note plays an instrument

One voice is made per note from the instrument's recipe:

* **Pitch.** The instrument's home note is the `pitch.start` of its first
  layer that is not noise. Every layer's pitch is multiplied by
  `note / home`, so layers stay in step with each other. Sweeps, vibrato
  and arpeggios move with the note. A layer that would start above 20 kHz
  is left out of that note. Filters do not move with the note.
* **Length.** While the note is held the envelope plays as written and
  stays at `sustain` for as long as the note lasts; `sustain_time` is not
  used. When the note ends the envelope falls to silence over `release`
  (at least 5 ms, so nothing clicks) from whatever level it had reached. A
  sound that fades by itself before the note ends just ends. A layer whose
  `offset` is later than the note's end is not played.
* **Velocity** multiplies the volume.
* **Kit pieces** play their recipe exactly as written: no pitch change and
  no `length`.
* Notes never cut each other off. Each starts on its exact sample.

What a song uses from an instrument recipe:

| Part | In a song |
|---|---|
| `layers` (with their own `effects`) | played for every note |
| top-level `effects` | run **once per track**, on the sum of its notes: one reverb is one room, echoes carry over the next notes, and a fast melody does not stack reverbs. A kit runs each piece's effects once for that piece |
| `seed` | mixed into every note's seed |
| `variation` | applies per note, so each note differs a little |
| `master.gain`, `normalize`, `normalize_peak_db`, `normalize_lufs` | used once, as one fixed level: the gain the recipe's master applies when the recipe is rendered alone. An instrument is as loud in a song as on its own, and `velocity` still works |
| `sample_rate`, `channels`, `duration`, `master.trim_silence` | not used; the song decides |

Order of the signal: note voices, summed, instrument effects, instrument
level, track `volume` and `pan`, all tracks summed, song `effects`, song
`master`.

### Song seeds

The same song and seed give the same bytes. Each track has its own random
stream, picked by the track's **name**; each note has its own, picked by
the **sample it starts on** (and, for notes that start together, its place
in pitch order). So:

* adding, removing or reordering tracks never changes another track;
* adding, removing or changing a note never changes another note;
* a track rendered alone is exactly that track in the mix;
* renaming a track changes that track's random parts only.

### Song length, stems and checking

The file is the song's bars plus whatever still rings after the last bar.
The exact length of the bars in samples is `song.song_frames` in the
result of `render --json` and `analyze`, with `tempo`, `bars`,
`beats_per_bar` and `seconds_per_bar`.

`--solo NAME`, `--mute NAME` and `--per-track` (`solo`, `mute`,
`per_track` in the MCP tools) hear tracks on their own. A track heard alone
is a **stem**: it is finished with the full song's master gain and length,
the same rule as `per_layer`. The stems of all tracks add up to the mix (to
within float rounding), unless the mix clips or the song's own `effects`
include distortion or bitcrush.

Validation reports every problem at once, with paths such as
`tracks[1].notes[3].note`, `tracks[0].play[1].pattern` and
`instruments.lead.layers[0].volume`.

## Examples

All in `examples/recipes/`, all render without clipping. Their square
and saw layers say `aliasing = "retro"`, so each example still renders
exactly as it was first pinned (the presets use the `clean` default):

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
| `fanfare.toml` | four layers (arpeggio lead, two-note chord, bass), `normalize_lufs`, master reverb; reads note by note in `onsets` and layer by layer with `--per-layer` |
| `siren.toml` | an LFO on the filter `cutoff` in time with a wide vibrato, and an LFO on `duty` (uses the `clean` default) |

## Deferred (not in v0, never silently ignored)

These are not accepted by the schema yet; using them is an `unknown_field`
error until implemented. See `ROADMAP.md` for the order.

* More than one LFO per layer, LFO rates in beats, and an LFO that runs
  on across the notes of a song (it restarts with each note).
* Per-parameter random ranges (`volume = { min = 0.6, max = 0.9 }`).
* Sample layers.
* Formant voice sources (Phase 5).
* In songs: seamless loops (`loop`, roadmap task 20), built-in instruments
  (task 19), tempo changes inside a song, swing, volume or filter changes
  over time (automation), sample tracks, filters that follow the note, and
  velocity changing the tone.

## Versioning

`version` is `0`. New optional fields and enum variants can be added without
changing it. It increments only when the meaning of an existing field
changes, and a converter for old recipes ships with that change.
