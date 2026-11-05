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
| `spectral_centroid_hz` | Magnitude-weighted mean frequency over the active span. Higher is brighter. Naive square and saw waves read very bright because of their many harmonics. |
| `tonality` | Share of A-weighted power in spectral peaks at least 10 dB above their neighbourhood (median of ±16 FFT bins). Peaks are found on the raw spectrum; the power is weighted with the A curve (IEC 61672, 0 dB at 1 kHz) so each frequency counts roughly as loud as it sounds, and a sub-bass thump under a noise blast does not make the blast tonal. Measured with 46 ms and 186 ms frames and the higher value kept: short frames follow sweeps, long ones resolve the close harmonics of low tones. Tones near 1, broadband noise near 0. Noise confined to a band narrower than about 700 Hz can still read tonal. |
| `pitch_track` | Up to 8 evenly spaced 46 ms windows inside the active span, each with `time_seconds`, `dominant_hz` (strongest raw frequency, usually the fundamental of chip waveforms), `level_dbfs` and `tonality` (same method as above, with a 186 ms frame around the same centre). `dominant_hz` and `tonality` are `null` in windows below -60 dBFS RMS; `dominant_hz` means little when the window's tonality is low. |
| `silence_threshold_dbfs` | The -60 dBFS threshold used above. |

Spectral fields use the mean of the channels; level fields use all
channels.

## Summary words

| Word | Rule |
|---|---|
| very dark, dark, mid, bright, very bright | Centroid below 300, 800, 2500, 6000 Hz, or above. |
| sharp, quick, soft, slow attack | Attack below 10, 50, 200 ms, or above. |
| tonal, tone and noise, noisy | Tonality at least 0.6, between, or at most 0.3. |
| pitch rising / falling from X to Y Hz | Uses pitch-track windows with tonality above 0.3 and a level within 20 dB of the loudest window, so a faint tail does not decide the direction. First and last such windows differ by more than one semitone; otherwise "around X Hz". No pitch is given for noisy sounds. |
| `~` before LUFS | Short-clip loudness (`k_weighted_ungated`). |
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
