# Analysis reference

`duck-echo analyze` (and the MCP `analyze` tool) turn a sound into numbers
an agent can check without listening. `duck-echo compare` reports what
changed between two sounds. Both accept a WAV file or a recipe, which is
rendered in memory. This page defines every field; the code is in
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
| `silence_threshold_dbfs` | The -60 dBFS threshold used above. |

Spectral fields use the mean of the channels; level fields use all
channels.

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

## Picture

`picture: true` (MCP) or `--picture out.png` (CLI) draws the sound as a
480×254 PNG for agents that read images, and adds a `picture` object
describing it (`width`, `height`, `seconds`, `level_dbfs`,
`spectrogram_hz`, `spectrogram_dbfs`, `layout`). Both panels share a time
axis from 0 to the end of the file:

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

## Summary words

| Word | Rule |
|---|---|
| very dark, dark, mid, bright, very bright | Centroid below 300, 800, 2500, 6000 Hz, or above. |
| sharp, quick, soft, slow attack | Attack below 10, 50, 200 ms, or above. |
| tonal, tone and noise, noisy | Tonality at least 0.6, between, or at most 0.3. |
| pitch rising / falling from X to Y Hz | Uses pitch-track windows with tonality above 0.3 and a level within 20 dB of the loudest window, so a faint tail does not decide the direction. First and last such windows differ by more than one semitone; otherwise "around X Hz". No pitch is given for noisy sounds. |
| `~` before LUFS | Short-clip loudness (`k_weighted_ungated`). |
| N onsets | Two or more `onsets`. |
| CLIPPING (N samples) | Any clipped sample. |
| silent tail | Trailing silence above 20% of the length. |

The thresholds live in `duck_echo_analyze::features::words`.

## `compare` fields

| Field | Meaning |
|---|---|
| `summary` | What changed from `a` to `b` in words, for example "b is 6.0 LU quieter, 34% brighter, 0.12 s longer, waveforms closely match (0.98)". |
| `identical` | Same format and exactly the same samples. |
| `delta` | `b` minus `a` for duration, peak, RMS, loudness (`loudness_lu`, only when both use the same method), centroid (Hz and `b/a` ratio), tonality, attack, decay, crest factor and trailing silence. |
| `correlation` | Normalized cross-correlation of the channel means over all time shifts: `peak` (-1..1, largest magnitude), `lag_seconds` (positive when `b` is later), `at_zero_lag`. `null` when sample rates differ, either sound is silent, or the two together exceed about 95 s at 44.1 kHz. |
| `a`, `b` | The full analysis of each. |
