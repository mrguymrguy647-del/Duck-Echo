# Duck Echo agent guide

How to make sound effects and music with Duck Echo **without hearing
them**. Read this once, top to bottom. It is short on purpose.

You work through the MCP tools (`duck-echo-mcp`) or the `duck-echo`
command line. Both do the same things. Paths are relative to the server
root. `preset:NAME` names a built-in sound and `instrument:NAME` a
built-in instrument, anywhere a file path is accepted.

## The loop

Never write a sound from nothing, and never change many things at once.

1. **Start from something that works.** `list_presets` (sounds) or
   `list_instruments` (music), then `get_preset` or `get_instrument`.
2. **Save a copy.** `create` with a `path` such as `sfx/coin.toml` and
   `from: "preset:coin"` (or `instrument:piano`, or a file). The copy is
   named after its file, its description says what it was copied from,
   and its comments are kept. Edit its `tags` if they no longer fit.
3. **Change one thing.** `edit` with one op, for example
   `{"path": "layers[0].pitch.start", "value": "C6"}`.
4. **Render.** `render` with an `output` such as `sfx/coin.wav`. The
   result already holds the analysis.
5. **Read the numbers.** Start with `summary`. Then look at the one field
   your change should have moved.
6. **Compare.** `compare` with `a` (before) and `b` (after) says what
   changed in plain words, such as "b is 3.0 LU louder, 20% brighter".
   Its summary covers loudness, brightness, length, attack, clipping and
   how alike the waveforms are; `delta` has the numbers (peak too). It does not cover notes or pitch: for those,
   `analyze` both and read `onsets` and `pitch_track`. To keep a "before"
   to compare with, `render` it to a second file (`sfx/coin_before.wav`)
   first.
7. Keep the change, or take it back with `edit_undo`. Go back to step 3.

Every failed call returns `{ok: false, errors: [...]}`. Each error has
a `code`, a `path` (the field at fault, such as `layers[0].volume`, or
the argument, such as `arguments.output`), and often `allowed` and a
`suggestion`. `validate` reports every problem in one call. Fix them all
at once, then call again.

## Sound effects

### Pick the preset by category

Every preset has one category, and each category has a loudness target.
Sounds made to their target sit together in a game.

| Category | Target | Presets |
|---|---|---|
| impact | -16 LUFS | big_explosion, explosion, hit, metal_clang, punch, thud |
| weapon | -17 LUFS | blaster, charge_up, laser, missile, shotgun |
| alert | -18 LUFS | alarm, low_health, warning_beep |
| pickup | -19 LUFS | coin, extra_life, gem, key_pickup, powerup |
| movement | -20 LUFS | footstep, jump, land |
| ui | -22 LUFS | menu_open, ui_cancel, ui_click, ui_confirm, ui_error, ui_hover |
| ambient | -24 LUFS | engine_hum, wind |

### Keep the level while you edit

A preset is levelled by its peak (`master.normalize_peak_db`). Change
its sound and its loudness drifts. So the first edit to a copy should
always be to level it by loudness instead. These are the `ops` of one
`edit` call:

```json
[
  {"path": "master.normalize", "remove": true},
  {"path": "master.normalize_peak_db", "remove": true},
  {"path": "master.normalize_lufs", "value": -19}
]
```

Use the target of the category. From then on, every render comes out at
that loudness, whatever you change. If the target cannot be reached
without the peak going over -1 dBFS, the render says so in `warnings`.
Then make the sound less spiky (a longer attack, less resonance) or
accept it quieter. Within 0.5 LU of the target is fine; a game player
cannot hear that.

Loudness alone matters, not peak. Two sounds with the same peak can be
13 dB apart in loudness.

### Read loudness the right way

`loudness_lufs` comes with `loudness_method`:

* `bs1770_gated` for sounds of 0.4 s or longer: the broadcast standard.
* `k_weighted_ungated` for shorter sounds, shown with `~` in the summary
  (`~-19.0 LUFS`).

Compare short sounds with short sounds. When the two methods differ,
`compare` gives no loudness change and reports the RMS change instead.

### Which field to read

| You want to know | Read |
|---|---|
| Is it the right length? | `duration_seconds`, `trailing_silence_seconds` |
| Does it start hard or soft? | `attack_seconds`, summary word ("sharp attack"); see below |
| Is it bright or dark? | `spectral_centroid_hz`, summary word ("bright") |
| Is it a tone or noise? | `tonality` (near 1 is a tone, near 0 is noise) |
| Where does the pitch go? | `pitch_track` (up to 8 points) |
| Where are the notes? | `onsets`: time, `note_hz` and level of each start |
| How does it fade? | `energy_over_time` (20 slices, dBFS) |
| Is it broken? | `clipping.clipped`, `dc_offset`, `is_silent` |
| Does it end with a click? | `ending.clean`, summary words "ends cleanly" or "ENDS WITH A CLICK" |

A sound that is still loud when the file ends clicks when it stops. Let
it fade: a longer `release`, or a `duration` long enough for the
envelope to finish. (For a loop, read `loop_seam` instead: a loop never
stops.)

`attack_seconds` is the time from the first sound to the **loudest**
moment. If a sound holds near its peak, or a later layer is a little
louder, it can read "soft attack" although it starts hard, and
shortening an envelope's `attack` may not move it. Then read
`per_layer` (each layer's own attack) and the first `energy_over_time`
slices instead.

### When the summary is not enough

* **Several layers.** When the summary ends with "N layers; per_layer
  describes each", the summary describes the mix, which is often just its
  loudest layer. Add `per_layer: true` to get each layer alone, at its
  level in the mix. Use `solo: [1]` or `mute: [0]` to hear only some.
* **Texture.** `picture: true` returns an image: level over time on top,
  a spectrogram below. It shows grit, a noisy tail or a click the
  numbers hide. Check what you see against the numbers.

### Square and saw waves: leave `aliasing` alone

The default `aliasing = "clean"` keeps high notes free of false tones.
Set `aliasing = "retro"` only when you want the raw, gritty sound of an
old sound chip on purpose.

### A small example

A coin, a fourth higher than the preset (it starts on E6, not B5), at
its target:

```toml
version = 0
name = "coin_high"

[master]
normalize_lufs = -19
trim_silence = true

[[layers]]
source = "square"
envelope = { attack = 0.002, hold = 0.06, decay = 0.25 }
pitch = { start = "E6", arpeggio = { steps = [0, 5], step_time = 0.07, repeat = false } }
```

Pitches are note names (`"C5"`, `"F#3"`, `"Bb2"`) or numbers in Hz.
Top-level keys (`seed`, `channels`, `variation`) go above the first
`[table]`: in TOML every key after a table header belongs to that table.

## Music

A **song** is a document with `tempo` and `tracks` instead of `layers`.
Every note plays an **instrument**, and an instrument is a recipe. The
same tools take songs: `validate`, `create`, `edit`, `render`,
`analyze`, `compare`. `get_schema` with `kind: "song"` documents every
field.

### A song in one screen

```toml
version = 0
name = "meadow"
tempo = 96
bars = 4
loop = true

[master]
normalize_lufs = -18

[patterns.beat]          # one bar of drums, used in every bar
notes = [
  { beat = 0, note = "kick" },
  { beat = 1, note = "snare" },
  { beat = 2, note = "kick" },
  { beat = 3, note = "snare" },
  { beat = 0.5, note = "hat", velocity = 0.5 },
  { beat = 2.5, note = "hat", velocity = 0.5 },
]

[patterns.root]          # one bar of bass, written for C
notes = [{ beat = 0, note = "C2", length = 3 }]

[[tracks]]
name = "chords"
instrument = "pad"
notes = [
  { bar = 1, beat = 0, note = ["C4", "E4", "G4"], length = 4 },
  { bar = 2, beat = 0, note = ["A3", "C4", "E4"], length = 4 },
  { bar = 3, beat = 0, note = ["A3", "C4", "F4"], length = 4 },
  { bar = 4, beat = 0, note = ["B3", "D4", "G4"], length = 4 },
]

[[tracks]]
name = "bass"
instrument = "bass"
play = [
  { pattern = "root", bars = [1] },
  { pattern = "root", bars = [2], transpose = -3 },   # A
  { pattern = "root", bars = [3], transpose = 5 },    # F
  { pattern = "root", bars = [4], transpose = 7 },    # G
]

[[tracks]]
name = "tune"
instrument = "piano"
notes = [
  { bar = 1, beat = 0, note = "E5" },
  { bar = 1, beat = 1, note = "G5" },
  { bar = 1, beat = 2, note = "C6", length = 2 },
  { bar = 3, beat = 0, note = "A5", length = 2 },
  { bar = 4, beat = 0, note = "G5", length = 4 },
]

[[tracks]]
name = "drums"
instrument = "drums"
play = [{ pattern = "beat", bars = [1, 2, 3, 4] }]
```

The rules that trip agents up:

* **Bars count from 1, beats from 0.** `{ bar = 2, beat = 0 }` is the
  first beat of the second bar. With no `bar`, `beat` counts from the
  start of the song.
* `length` is in beats (default 1). `velocity` is 0 to 1 and only
  changes loudness.
* A **chord** is a list: `note = ["C4", "E4", "G4"]`.
* A **pattern** is a named group of notes. Place it with `play`, at one
  or more `bars`, and move it with `transpose` (semitones).
* Every track needs a unique `name`. `solo` and `mute` use it.
* `beats_per_bar` is 4 unless you set it. Songs render in stereo at
  44.1 kHz unless you set `channels` or `sample_rate`.

### Choosing instruments

| Instrument | Use it for | Useful range |
|---|---|---|
| `piano` | tunes, soft chords | C2 to C7 |
| `pad` | long chords under everything; give it 2 beats or more | C3 to C6 |
| `bass` | the low line | E1 to C4 |
| `chip_lead` | a bright retro tune | C3 to C7 |
| `drums` | a kit: notes are `kick`, `snare`, `hat`, `clap` | (no pitch) |

Outside its range an instrument still plays, but sounds dull or thin.
`get_instrument` shows each recipe. To change one, copy it into the
song as `[instruments.NAME]` and edit the copy. A track can also play
`preset:NAME` or a recipe file next to the song.

Balance with track `volume` (0 to 2, default 1). The instruments are
already levelled against each other, so start every track at 1, then
check `per_track`: the **melody should be the loudest pitched track**.
A pad playing three-note chords is often louder than a one-note tune;
raise the tune's `volume` (1.3 to 1.5) until it leads. Level the whole
song with `master.normalize_lufs` (music in a game: -18).

### Loops and stems

* `loop = true` makes the file exactly as long as its bars, with
  whatever still rings at the end folded onto the start. It repeats
  forever with no click. `loop_seam.clean` confirms it (at the top level
  of the result, next to `summary` in `analyze` and next to `analysis`
  in `render`), and the summary says "loop joins cleanly".
* `render` with `solo: ["drums"]` writes one track alone, at its level
  in the mix (on the command line: `--solo drums`). These **stems** line up with the full song and add
  up to it, so a game can switch parts on and off.
* **Two themes a game fades between** (day and night, calm and danger)
  need the same `tempo`, `beats_per_bar` and `bars`. Then they are the
  same length to the sample and stay in time. Keys that share notes
  (C major and A minor) blend best. In the Duck Echo repository,
  `examples/songs/duck_craft_day.toml` and `duck_craft_night.toml` are
  such a pair.

### Checking a song without ears

`analyze` on a song listens for every written note on its own track and
reports `note_check`. The summary ends with "N of M notes heard". Each
problem in `note_check.tracks[].problems` has the note's `path`, `bar`,
`beat`, `written` and, when something was heard, `heard` and `cents_off`:

| `status` | Meaning | Usual fix |
|---|---|---|
| `missing` | nothing there | velocity 0? note past the end? instrument out of range? |
| `quiet` | 24 dB under the rest of the track | raise `velocity`, or the instrument is too low there |
| `wrong_pitch` | another pitch is heard | the instrument's loudest part is not the note; or a typo |
| `uncertain` | too short to read | usually fine; make it longer if it matters |

`extra_onsets` lists clear starts of sound where no note was written,
with their bar and beat (to the nearest eighth of a beat). An echo's
repeats appear there, and so does a sound that is in the audio but not
in your notes. Wobbles inside a held note are not listed. Read them as
information: they are not errors in your song. `note_check` sits at the
top level of the result, like `loop_seam`.

A song's summary uses words for a mix, such as `8 bars looping at 140
BPM, 4 tracks; 13.71 s, -18.0 LUFS, no clipping, loop joins cleanly, 155
of 155 notes heard; loudest track lead (-22.1 LUFS), quietest drums
(-27.4 LUFS)`. A track 10 LU or more under the loudest is lost in the
mix: raise its `volume`. `per_track: true` gives every track's analysis.

### Before you call a tune done

* `validate` has no errors and no warnings.
* The summary has "no clipping", "loop joins cleanly" (for a loop) or
  "ends cleanly" (for a song that stops), and "M of M notes heard".
* The quietest track is not far under the loudest.
* `spectral_centroid_hz` fits the mood: lower is darker (night).

Duck Echo cannot tell you whether a tune is *good*. Say so, and ask a
person to listen.

## Undo, and working beside a person

Every change you make to a file (`create`, `edit`, and the commands such
as `sound_set`, `sound_add_layer` or `song_add_note`) is one step of the
project's Undo history, shared with the person in Duck Echo Studio.
`edit_undo` takes back the newest step, whoever made it, so check
`edit_history` first: it lists each step with who made it (`You`, `AI
assistant` or `Outside Duck Echo`). If a call says a file "kept changing",
the person was saving it at that moment; nothing was changed, so read the
file again and retry.

## Where to read more

`get_guide` returns this guide, and with `topic` the reference pages:

* `recipe_spec`: every recipe field, default and range.
* `song_spec`: every song field.
* `analysis`: every analysis field and how it is measured.
* `instruments` and `presets`: what each one is.

Also `get_schema` (every recipe field; `kind: "song"` for songs),
`get_preset`, `get_instrument`, and each tool's own description. From the
command line, `duck-echo guide TOPIC` prints the same pages.
