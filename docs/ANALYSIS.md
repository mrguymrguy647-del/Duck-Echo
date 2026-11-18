# Analysis reference

`duck-echo analyze` (and the MCP `analyze` tool) turn a sound into numbers
an agent can check without listening. `duck-echo compare` reports what
changed between two sounds. Both accept a WAV file, a recipe or a song;
recipes and songs are rendered in memory. This page defines every field; the code is in
`crates/duck-echo-analyze`.

Silence means below -60 dBFS (0.001). The *active span* runs from the first
to the last sample above that level.

## `analyze` fields

| Field | Meaning |
|---|---|
| `summary` | One line in words, built only from the numbers below (see Summary words). |
| `sample_rate`, `channels`, `frames`, `duration_seconds` | Format and length. |
| `peak`, `peak_dbfs` | Largest absolute sample over all channels. |
| `rms`, `rms_dbfs` | RMS over all samples, silence included. |
| `loudness_lufs`, `loudness_method` | `bs1770_gated` (ITU-R BS.1770-4) for clips of 400 ms or more; `k_weighted_ungated` over the active span for shorter clips. Compare short clips with short clips. |
| `per_channel` | `peak`, `peak_dbfs`, `rms`, `rms_dbfs` for each channel. |
| `clipping` | `clipped`, `clipped_samples` (at or above 0.99), `longest_run` within a channel. |
| `leading_silence_seconds`, `trailing_silence_seconds`, `is_silent` | Silence at each end. |
| `dc_offset` | Mean sample value; should be near 0. |
| `crest_factor_db` | Peak over RMS of the active span. Sine 3 dB, square 0 dB; percussive sounds are higher. |
| `attack_seconds` | From the first non-silent sample until the short-term peak level first reaches 90% of the sound's peak. The short-term level is the largest sample in the previous 20 ms (1 ms steps), which stays steady for content down to 50 Hz. It measures the time to the *loudest* moment: when a later note or layer is slightly louder than the first (two-note chimes, a layer entering later), this reads long even though the sound starts sharply. Check `pitch_track` levels in that case. |
| `decay_seconds` | From the last sample at 90% of the peak until the moment after which no sample in the next 20 ms reaches 10% of the peak (-20 dB). A held note does not count as decay. `null` if it never falls that far. |
| `spectral_centroid_hz` | Magnitude-weighted mean frequency over the active span. Higher is brighter. Square and saw waves read bright because of their many harmonics, and `retro` ones (raw waves with folded extra tones) brighter still. |
| `tonality` | Share of A-weighted power in spectral peaks at least 10 dB above their neighbourhood (median of ±16 FFT bins). Peaks are found on the raw spectrum; the power is weighted with the A curve (IEC 61672, 0 dB at 1 kHz) so each frequency counts roughly as loud as it sounds, and a sub-bass thump under a noise blast does not make the blast tonal. Measured with 46 ms and 186 ms frames and the higher value kept: short frames follow sweeps, long ones resolve the close harmonics of low tones. Tones near 1, broadband noise near 0. Noise confined to a band narrower than about 700 Hz can still read tonal. |
| `pitch_track` | Up to 8 evenly spaced 46 ms windows inside the active span, each with `time_seconds`, `dominant_hz` (strongest raw frequency, usually the fundamental of chip waveforms), `level_dbfs` and `tonality` (same method as above, with a 186 ms frame around the same centre). `dominant_hz` and `tonality` are `null` in windows below -60 dBFS RMS; `dominant_hz` means little when the window's tonality is low. |
| `energy_over_time` | `slice_seconds` and `rms_dbfs`: the RMS level of all channels in 20 equal slices over the whole file, rounded to 0.1 dB, `null` for digital silence. A coarse level-over-time picture in numbers: a fade, a gap, a second hit or a tail. |
| `onsets` | Where notes or bursts start, each with `time_seconds`, `note_hz` and `level_dbfs` (see Onsets). |
| `ending` | The click check for a sound that stops: `jump`, `largest_step_near_end`, `last_level_dbfs`, `clean` (see Ending). |
| `roughness`, `sharpness`, `boom`, `movement`, `feel` | How the sound feels, measured cheaply until Duck Ear listens properly (see How it feels). |
| `peaks` | Up to 8 strongest peaks of the average spectrum (93 ms Hann frames, half overlap), strongest first: `hz` (placed between bins by a parabola) and `db` against the strongest (0). Only peaks within 40 dB of the strongest. A fast LFO shows as side tones at the note plus and minus its rate. |
| `silence_threshold_dbfs` | The -60 dBFS threshold used above. |

Spectral fields use the mean of the channels; level fields use all
channels.

### Ending

When a player reaches the end of a file it stops, and the wave jumps from
the last sample to silence. A sound that is still loud there clicks.
`jump` is the size of the last sample (largest of the channels, linear).
`largest_step_near_end` is the biggest step between two neighbouring
samples in the last 5 ms, and `last_level_dbfs` is the RMS level of those
5 ms. `clean` is true when `jump` is at most -50 dBFS (0.0032), or no
bigger than `largest_step_near_end`: a jump that size is the sound's own
kind of step, so it cannot stand out. The renderer trims a fading tail
once it falls below -60 dBFS, so every rendered sound that fades out ends
cleanly; a `duration` that cuts a held note does not.

Two limits: a cut at a zero crossing of a tone is clean (there is no jump
to hear), and a cut of a square wave or loud noise is clean too, because
their own steps are as big as the cut. For a loop, read `loop_seam`
instead: a loop never stops, so its last sample leads into its first.

### Onsets

The first onset is always the start of the sound (its first sample above
-60 dBFS). Later onsets are peaks of *spectral flux*: how much new energy
appears from one 23 ms frame (Hann, hops of 5.8 ms) to the frame 11.6 ms
earlier, on log-compressed magnitudes from 30 Hz to 8 kHz. A bin only
counts as new if no bin within about half a semitone had that much before,
so vibrato and sweeps do not read as notes, while an arpeggio step, a
chord entering, an echo repeat or a noise burst does. A peak is kept when
it is the largest within 17 ms, stands 10% of the sound's largest flux over
the mean of its neighbourhood, makes up at least 15% of its frame's
spectrum in a tonal frame (tonality 0.6 or more) or 35% in any other (the
flicker of sustained noise stays near 20%), and does not leave the sound
more than 10 dB quieter than just before (so a note cut off abruptly is
not a new one). Onsets closer than 25 ms are one. Times are accurate to
about 6 ms.

`note_hz` is the pitch that starts at the onset: the strongest frequency in
the stretch up to the next onset (at most 100 ms) that is more than 3 dB
louder than in the same length just before it, so a melody over a held
bass reads as the melody, and a chord reads as its loudest new note. It is
measured at 46 ms and at the whole stretch, keeping the more tonal reading
(short follows sweeps, long resolves low notes), and is `null` when the
stretch is noisy (tonality 0.3 or less). `level_dbfs` is the RMS of the
stretch.

Checked on the presets: arpeggios and beeps give one onset per note (coin
2, alarm 7, extra_life 7, gem 6); explosions, wind, engine hum, sweeps and
vibrato give one. Limits: two notes starting within 25 ms are one onset; a
note repeated at the same pitch without a gap or a level rise is not a new
onset; `note_hz` names one note of a chord (use `per_layer` to check the
rest).

## Layers, solo and mute

For a recipe (not a WAV), `analyze` and `render` can hear layers on their
own: `per_layer` adds `layers: [{index, name, analysis}]`, one full
analysis per heard layer, and `solo`/`mute` choose the heard layers (the
result lists them in `heard_layers`). A layer is rendered with the others
silenced but otherwise exactly as in the mix: same seed stream, variation,
timing, master effects, normalization gain and length. So its loudness is
its contribution to the mix, its onsets line up with the mix's, and with no
master effects the parts add up to the whole (a nonlinear master effect,
distortion or bitcrush, makes them differ). Use it when the summary of a
layered sound describes only its loudest part.

For a recipe, `analyze` also returns `warnings` when there are any: the
recipe's validation warnings and render warnings (a `normalize_lufs` target
the -1 dBFS peak ceiling did not allow).

## Songs

For a song (a document with `tracks`), `analyze` and `render --json`
measure the finished mix with every field above, and add:

| Field | Meaning |
|---|---|
| `song` | `tempo`, `beats_per_bar`, `bars`, `loop`, `song_frames` (the exact length of the bars in samples), `seconds_per_bar`, `tracks` and `notes` (after patterns are placed and chords split). Enough for a game engine to switch music on a bar line. |
| `tracks` | With `per_track`: `[{index, name, instrument, notes, analysis}]`, one full analysis per heard track. |
| `heard_tracks` | With `solo` or `mute` (track names): the names of the tracks heard. |
| `loop_seam` | For a song with `loop = true`: `seam_step`, `largest_step`, `clean` (see below). |
| `note_check` | The notes written next to the notes heard (see below). Always in `analyze`; in `render` with `per_track` or `notes`. |

The summary of a song uses words for a mix, not a sound effect's
(brightness, attack, pitch and onsets are left out; the fields are all
still there): its shape, then its length, loudness, clipping, how it ends
("loop joins cleanly", or "ends cleanly" for a song that stops), the note
check, and, when the tracks were heard alone (always in `analyze`), the
loudest and quietest track:

`8 bars looping at 140 BPM, 4 tracks; 13.71 s, -18.0 LUFS, no clipping,
loop joins cleanly, 155 of 155 notes heard; loudest track lead (-22.1
LUFS), quietest drums (-27.4 LUFS)`

When the tracks were not heard alone (`render` without `per_track` or
`notes`), it ends "per_track describes each" instead.

**Stems.** A track heard alone (`solo`, `mute`, `per_track`) is rendered
with the other tracks silenced and finished with the full song's master
gain and length, the same rule as `per_layer`. So a stem reads at the
level and times it has in the mix, and the stems of all tracks add up to
the mix (to within float rounding), unless the mix clips or the song's
own `effects` include distortion or bitcrush.

### Loop seam

A click is a sudden jump between two samples. `seam_step` is the jump
from the last sample of the file back to the first, the place where a
loop repeats (largest of the channels, linear). `largest_step` is the
biggest jump between two neighbouring samples anywhere inside the file.
`clean` is true when the first is no bigger than the second: the join
jumps no more than the music itself does, so it cannot click.

### Notes written and notes heard

The question this answers: "I wrote these notes. Can each one really be
heard, at the pitch I wrote?" A note can go missing in ways the song text
does not show: it can be outside what its instrument can play, buried,
too short to sound, or played by an instrument whose loudest part is not
the note itself.

Each track is rendered alone (a stem), and every written note is listened
for at its place:

* A **pitched note** is read from its held part: from just after its
  start (a quarter of its length, at most 80 ms, to skip the attack) to
  its end, at most 0.8 s. The spectrum of that stretch is searched for a
  peak within half a semitone of the written pitch. If one is there, the
  note is heard, whatever else is sounding, and `cents_off` says how far
  it is from the written pitch. For a steady tone this reads to a
  fraction of a cent, far finer than `onsets[].note_hz`.
* If there is no such peak, the strongest pitch in the stretch decides:
  when it belongs to another note of the same track that could be
  sounding there (its overtones count too), the note is `missing`;
  otherwise something else is playing, which is `wrong_pitch`.
* A **kit piece** has no pitch to check. It is heard when the 50 ms after
  it was written are not silent.

`note_check` holds `notes_written`, `notes_heard`, `notes_uncertain`
(when not 0) and `tracks`, one entry per heard track with the same
counts, plus:

* `problems`: every note that was not simply heard. Each has `status`,
  `path` (where it is written, such as `tracks[1].notes[3]` or
  `patterns.walk.notes[2]`), `bar` (from 1) and `beat` (from 0) where it
  landed, `written` and `written_hz`, and, when something was heard,
  `heard` (the nearest note name), `heard_hz`, `cents_off` and
  `level_dbfs`.
* `extra_onsets`: clear starts of sound where no note was written, with
  `time_seconds`, `bar`, `beat` (rounded to the nearest eighth of a beat,
  the place you would write a note there) and `note_hz`. An echo's
  repeats show up here, and so does a note that is in the audio but not
  in the song. They are information, not problems. A start inside a
  note that is still held on the same track is **not** extra when it
  brings no new pitch (no clear pitch, a reading above 5 kHz, the held
  note's own pitch or one of its overtones, or a pitch that was already
  sounding just before, such as a resonant filter sweeping past) and the
  level does not jump by 6 dB or more. Long low notes wobble as their filter closes and their
  layers beat, and the onset finder sees those wobbles as starts; this
  rule keeps them out. Drum tracks hold no notes, so their extras are
  found by time alone. The onset finder cannot see a pure sine entering
  under another pure sine, so such a note is not reported.
* `notes`: every note in the same shape, with `notes: true` (MCP) or
  `--notes` (CLI).

`status` is one of:

| Status | Meaning |
|---|---|
| `heard` | The note is there, within half a semitone of its pitch. |
| `missing` | Nothing is there, or only other notes of the track. |
| `quiet` | The note is there, but more than 24 dB under the usual level of its track (the middle one of its heard notes). |
| `wrong_pitch` | Something is there at another pitch; `written` and `heard` name both. |
| `uncertain` | The stretch is too short to read (under about 8 cycles of the note) and the note was not found. Neither heard nor counted as wrong. |

Limits: two notes of the same pitch that overlap cannot be told apart, so
the second is heard even if only the first sounds. A note is checked on
its own track only; whether it is buried under another track is a matter
for the mix (compare the tracks' loudness with `per_track`).

## Picture

`picture: true` (MCP) or `--picture out.png` (CLI) draws the sound as a
480×254 PNG for agents that read images, and adds a `picture` object
describing it (`width`, `height`, `seconds`, `level_dbfs`,
`spectrogram_hz`, `spectrogram_dbfs`, `layout`). Both panels share a time
axis from 0 to the end of the file. The picture of a song also has a thin
grey line at the start of each bar (dotted over the spectrogram, so it
hides nothing), and `picture.bar_lines` lists their times in seconds:

* **Level** (top): per pixel column, the peak (light blue) and RMS (dark
  blue) of all channels, -60 to 0 dBFS, gridlines at -20 and -40. Red ticks
  mark the onsets.
* **Spectrogram** (bottom): the mean of the channels, 23 ms Hann frames
  (the loudest frame per column when a column spans several), log
  frequency from 40 Hz to 20 kHz (or half the sample rate), colour from
  black at -90 dB to pale yellow at 0 dB (a full-scale sine). Harmonics are
  evenly spaced lines that rise and fall together; aliases of `retro`
  square and saw layers are lines that do not fit that pattern (with the
  `clean` default they are 40 dB or more lower); noise is a smear.

The picture shows texture the numbers miss (grit, a noisy tail, aliasing),
but its colours are not measurements: check numbers with the fields
above.

## How it feels

Four numbers for the questions a film's sounds raise, each with fixed
thresholds for `low`, `medium` and `high` in `feel`. The summary names
each that is `high`: rough, piercing, booming, moving. They are plain
measurements, not a model of hearing; Duck Ear (Phase 4) will do better.
All four use the mean of the channels over the active span.

| Field | What it measures | medium from | high from |
|---|---|---|---|
| `roughness` | Fast loudness wobble at one steady rate between 20 and 150 a second: growls, screams, rasps, low buzzes. The Hilbert envelope (the loudness without the ripple at twice the note, so a low hum is not rough), one value a millisecond, minus its 31 ms average; then only the strongest wobble rate in 20 to 150 Hz, ±3 Hz, as a share of the mean level. Noise flickers at every rate, so steady noise reads low. A wobble of pitch alone (FM) changes the loudness little and reads low here; read `peaks` for it. | 0.12 | 0.3 |
| `sharpness` | Share of the power between 2 and 5 kHz, where the ear is most sensitive: harsh, piercing. | 0.15 | 0.35 |
| `boom` | Share of the power below 80 Hz. | 0.1 | 0.35 |
| `movement` | How much the sound changes over time: the spread of the brightness (spectral centroid, in octaves) plus the spread of the level (dB, over 20), across 50 ms frames within 30 dB of the loudest. Sounds shorter than 100 ms read 0. | 0.3 | 0.75 |

On the presets: `engine_hum` (a 55 Hz saw) is medium rough, the wind low;
`big_explosion`, `thud` and `low_health` boom; `gem` and `extra_life` are
piercing; `laser`, `missile` and the explosions move.

## Summary words

| Word | Rule |
|---|---|
| very dark, dark, mid, bright, very bright | Centroid below 300, 800, 2500, 6000 Hz, or above. |
| sharp, quick, soft, slow attack | Attack below 10, 50, 200 ms, or above. |
| tonal, tone and noise, noisy | Tonality at least 0.6, between, or at most 0.3. |
| pitch rising / falling from X to Y Hz | Uses pitch-track windows with tonality above 0.3 and a level within 20 dB of the loudest window, so a faint tail does not decide the direction. First and last such windows differ by more than one semitone; otherwise "around X Hz". No pitch is given for noisy sounds. |
| `~` before LUFS | Short-clip loudness (`k_weighted_ungated`). |
| rough, piercing, booming, moving | `feel` is `high` for roughness, sharpness, boom or movement (see How it feels). |
| N onsets | Two or more `onsets`. |
| CLIPPING (N samples) | Any clipped sample. |
| ends cleanly, ENDS WITH A CLICK | `ending.clean` (see Ending). Not in a song's summary when it loops; "loop joins cleanly" or "LOOP CLICKS AT THE JOIN" is there instead. |
| silent tail | Trailing silence above 20% of the length. |
| N layers; per_layer describes each | Only for a recipe with two or more layers (a WAV file has no layers). The rest of the summary describes the whole mix, so this says how many layers made it and that `per_layer` gives each one's own analysis. With `per_layer` on, only "N layers"; with `solo` or `mute`, "K of N layers heard". |

The thresholds live in `duck_echo_analyze::features::words`.

## Rendered facts (`<name>.wav.json`)

Every WAV that `render`, `sound_variants`, `set_check` or the Studio's
Export writes has a facts file beside it, and `render` returns the same
object as `asset`: `duration_seconds`, `frames`, `sample_rate`,
`channels`, `loudness_lufs` and `loudness_method`, `peak_dbfs`,
`leading_silence_seconds` (below -60 dBFS), `hit_seconds` (the middle of
the loudest 5 ms, rounded to the millisecond: start the sound this much
before the frame its hit should land on), `loops`, `seed` and
`recipe_hash` (`sha256:` of the recipe's data as JSON with defaults
filled in, its random ranges included: comments and layout do not change
it).

## Sound sets (`set_check`)

`set_check` renders a folder of sounds, or a set file (see `RECIPE_SPEC.md`,
Sound sets), and writes into its `out` folder every WAV with its facts
file and three reports:

| File | What it holds |
|---|---|
| `<name>.table.csv` | One row per sound: `name, file, kind, tags, length_s, lufs, peak_dbfs, clipping, ending, target_lufs, off_lu, on_target, summary`. `ending` is `clean` or `click` (see Ending), or for a loop `loop clean` or `loop click` (see Loop seam). |
| `<name>.sheet.png` | The contact sheet: three pictures to a row (each the same picture as `analyze --picture`), each under its name and a line of numbers (length, LUFS, peak, target, problems). A sound with a problem (clipping, a click, off target, did not render) has its numbers in red and a red frame. |
| `<name>.check.json` | All of it, as the command's answer: `sounds` (the rows), `loudness` (per tag: `target_lufs`, where it came from, `tolerance`, how many `met` it, and each sound's `off_by_lu` with a `fix` in words) and `no_target` (sounds none of whose tags has a target). |

A sound is held to the target of its first tag that the set's `targets`
names, or else of its first category tag (impact -16, weapon -17, alert
-18, pickup -19, movement -20, ui -22, ambient -24 LUFS), within
`tolerance` (default 2 LU). A sound that does not load or render is a row
with its `error`, so one broken file does not hide the others. The
`summary` names every problem: "8 sounds; did not render: broken;
clipping: loud; off target: loud (+16.7 LU), wind (+6.0 LU)".

## Mix check (`mix_check`, `duck-echo mix`)

`mix_check` mixes a cue list and checks the mix. The cue list is JSON in
one of two forms:

* **Duck Forge's `log.json`, as a headless run writes it.** Only
  `updates_per_second`, `sounds` and `loops` are read; the log's other
  fields (`game`, `seed`, `frames`, `final`, `tune` ...) are the game's
  own report and are left alone. A sound's line is `{"frame", "sound",
  "file", "volume", "pan"}`, a loop's `{"loop", "file", "start", "stop",
  "volume": [[frame, level], ...]}`. Times are placed as Duck Forge
  places them: frame `f` starts at sample `(f - 1) x 44100 /
  updates_per_second` (frame 1 is the start). A loop's volume holds from
  its frame until the next point, as the game changes it; a loop with no
  `stop` plays to the end. The mix is as long as the game ran
  (`frames`), or longer if a sound rings on. A sound from Duck Forge's
  own synthesizer (a line with `"wave"` and no `"file"`) cannot be mixed
  here: it is listed in `not_mixed` and named in the summary. Files are
  named from the folder the game ran in: the nearest folder above the
  log whose `Cargo.toml` names the package `duckforge`, or `base`. Pan
  is Duck Forge's: one side turned down, a centred sound at full volume
  on both sides.
* **A plain list**, times in seconds: `[{"time", "file", "name",
  "volume", "pan", "loop", "stop"}]`; only `time` and `file` are needed.
  Unknown fields are errors with their path (`[0].vol`). Files are named
  from the list's own folder (or `base`). Pan is equal-power, as Duck
  Echo's own stereo (a centred sound is -3 dB on each side, as loud as
  in mono).

A `file` is a WAV, a recipe or a song (rendered first) or `preset:NAME`.
Volumes are linear, 0..=16; pan is -1..=1. Checked against Duck Forge's
own `audio.wav` for a Duck Craft run, the mix has the same length to the
sample and lines up to the sample; the largest difference, 0.0036, comes
from the log's volumes being written to two decimals.

The mix is made at 44.1 kHz in stereo: mono files go to both sides, and
other rates are read with straight lines between samples. It is written as `<name>.mix.wav` (32-bit
float, so a mix over full volume is kept as it is) beside
`<name>.mix.json` (the answer) and `<name>.mix.png` (the picture).

| Field | Meaning |
|---|---|
| `summary` | Length, loudness, peak, and every problem by name and time: "6.0 s, -16.8 LUFS, peak +0.2 dBFS; 1 moment over full volume: at 2.54 s (latch + door + wind); buried: step 2 at 1.50 s, 17 dB under". |
| `overs` | Every moment a sample goes over full volume (moments closer than 50 ms are one). Each has `start_seconds`, `end_seconds`, `peak_dbfs`, the `sounds` playing in it (loudest first, with their volume at that moment and their own peak there), and two fixes that bring the peak to -1 dBFS: `all_down` (every one of them down by `all_down_db`) and `loudest_alone` (only the loudest, found by halving until no sample of the moment is over; `null` when that alone cannot do it). |
| `not_mixed` | Sounds in a Duck Forge log that could not be mixed (its own synthesizer sounds), with their name, start and why. |
| `cues` | Every cue in time order: `start_seconds`, `end_seconds`, `volume`, `loudness_lufs` (its own loudness as it plays, K-weighted over its sounding part) and `rest_lufs` (everything else over the same part). A one-shot more than 15 dB under the rest is `buried`, with the sounds it is `under` and `lift_to`, the volume that puts it 10 dB under (`null` above 16: lower the others). Loops are beds and are never called buried. |
| `short_term_lufs` | Short-term loudness at the end of each second: the 3 s before it (EBU R 128), K-weighted and ungated; `null` for silence. |
| `sections` | The loudness (gated BS.1770) and peak of each section: the `sections` given (seconds), or every 10 s. |
| `loudness_lufs`, `peak_dbfs` | The whole mix. |

The rest of the mix under a cue is the K-weighted mix minus the cue's own
K-weighted sound, which is exact because K-weighting is a linear filter.

The picture: at the top, the mix's peak level per column (light blue,
red above full volume, -60 to +6 dBFS, a red line at full volume) and its
short-term loudness, a step per second (dark blue), with red bars over
each over; below it every cue on a lane of its own, a tick and its name
at its start, a light bar for a loop's length, buried cues in red; along
the bottom the seconds, and each section's name with its loudness.

## `compare` fields

| Field | Meaning |
|---|---|
| `summary` | What changed from `a` to `b` in words, for example "b is 6.0 LU quieter, 34% brighter, 0.12 s longer, waveforms closely match (0.98)". |
| `identical` | Same format and exactly the same samples. |
| `delta` | `b` minus `a` for duration, peak, RMS, loudness (`loudness_lu`, only when both use the same method), centroid (Hz and `b/a` ratio), tonality, attack, decay, crest factor and trailing silence. |
| `correlation` | Normalized cross-correlation of the channel means over all time shifts: `peak` (-1..1, largest magnitude; negative means the shapes match upside down, which the summary says as "upside down"), `lag_seconds` (positive when `b` is later), `at_zero_lag`. `null` when sample rates differ, either sound is silent, or the two together exceed about 95 s at 44.1 kHz. |
| `a`, `b` | The full analysis of each. |
