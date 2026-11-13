# Playtest 2: agent notes

I used only the guide (`AGENT_GUIDE.md`) and the bridge. Total: **38 tool calls, 0 failed**,
plus one `bridge.py list`. Calls by deliverable are below. I generated the song's JSON
argument with a small Python script in my scratchpad and passed it to `validate`/`create`
through the bridge. Nothing in the repo was written except through the tools, apart from this
file.

Extra files I left on purpose as before/after evidence: `playtest-2/sfx/coin_base.wav`,
`hit_base.wav`, `explosion_base.wav`, and two PNGs in `playtest-2/pictures/`.

---

## 1. Coin: `playtest-2/sfx/coin.toml`, `playtest-2/sfx/coin.wav`

- **Started from:** `preset:coin` (a square wave on B5 that jumps up 5 semitones after 0.07 s). I copied it with `create`, then used the guide's three ops to level it to -19 LUFS.
- **Change:** `layers[0].pitch.arpeggio.step_time` went from 0.07 to 0.05. I wanted a shorter first note so the jump comes sooner and the pickup feels snappier. The numbers confirm it: the second onset moved from 0.0697 s to 0.0523 s (1319 Hz).
- **Final summary:** `0.31 s, ~-19.0 LUFS, bright, sharp attack, tonal, pitch rising from 988 to 1319 Hz, 2 onsets, no clipping`. Peak -16.4 dBFS, no warnings, trailing silence 0.
- **Calls:** 9 (list_presets, get_preset, create, edit, render, edit, render, compare ×2). 0 failed. I ran compare twice only because I piped the first one through `head` and lost the summary.
- **Confidence:** high. It is the classic two-note coin, at target and clean.

## 2. Hit: `playtest-2/sfx/hit.toml`, `playtest-2/sfx/hit.wav`

- **Started from:** `preset:hit`, copied and levelled to -16 LUFS (impact).
- **Change:** `layers[0].volume` (the band-passed noise) went from 0.6 to 1.5. With `per_layer: true` the preset's noise layer was ~-29.9 LUFS against the square's ~-16.2. That is 14 LU under, so the "strike" texture was nearly lost and the hit was mostly a falling square blip. After the change the layers are ~-23.0 and ~-16.9. The mix's word changed from "tone and noise" to "noisy", and tonality fell from 0.45 to 0.29.
- **Final summary:** `0.12 s, ~-16.0 LUFS, bright, sharp attack, noisy, no clipping, 2 layers`. Peak -6.2 dBFS, no warnings. The picture shows a broadband burst over a falling low tone.
- **Calls:** 7 (get_preset, create, edit, render, edit, render, analyze+picture). 0 failed.
- **Confidence:** medium-high. The numbers fit a hit. Whether the noise/tone balance sounds good needs ears.

## 3. Explosion: `playtest-2/sfx/explosion.toml`, `playtest-2/sfx/explosion.wav`

- **Started from:** `preset:explosion`, copied and levelled to -16 LUFS.
- **Changes:**
  1. `layers[0].envelope.attack` went from 0.01 to 0.002, because the mix summary said "soft attack". This **did nothing measurable**: mix `attack_seconds` stayed at exactly 0.0808, and compare said "same loudness, 0.03 s shorter, waveforms closely match (1.00)". I kept it because it is harmless, but it is not the real change.
  2. `layers[0].filter.cutoff` went from 5000 to 2500 (the real change), for a deeper, less hissy boom. Centroid fell from 2688 to 1670 Hz, the word went from "bright" to "mid", and the mix attack became 0.006 s, "sharp attack".
  3. I tried `layers[1].envelope.attack` at 0.005 → 0.012 to follow the warning's advice. It gained only 0.014 LU and turned "sharp" into "quick", so I reverted it.
- **Final summary:** `1.12 s, -16.1 LUFS, mid, sharp attack, noisy, no clipping, 2 layers; per_layer describes each`. Peak -1.00 dBFS, loudness -16.085 LUFS (bs1770_gated).
- **Warning kept:** `suspicious` at `master.normalize_lufs`: "the target -16 LUFS needs the peak above the -1 dBFS ceiling, so the gain stopped there and the render measures -16.1 LUFS ...". I accepted 0.085 LU under target, as the guide allows ("or accept it quieter").
- **Calls:** 13 (get_preset, create, edit, render, edit, render, compare, edit, render, edit, render, edit, render). 0 failed.
- **Confidence:** medium-high. It is at target within 0.1 LU, with no clipping and a clean decay to -65 dB.

## 4. Tune: `playtest-2/music/tune.toml`, `playtest-2/music/tune.wav`

- **Started from:** the guide's "song in one screen" layout, written from scratch in that shape. It is in C major at 140 BPM, 8 bars, `loop = true`, `normalize_lufs = -18`. The chords are C G Am F | C G F G (bar 8 on G leads back to C at the loop). The tracks are:
  - `lead` on `chip_lead` (35 notes, C5–E6);
  - `chords` on `pad` (whole-bar triads);
  - `bass` on `bass` (an octave-bounce pattern `root` transposed per bar);
  - `drums` (kick/snare on the beats, hats on the off-beats).
- **Change after the first render:** `tracks[0].volume` went from 1 to 1.4. With every track at 1, as the guide advises, `per_track` gave pad -22.6, lead -24.2, bass -24.2 and drums -26.5 LUFS. The melody sat under the pad. After the change the tracks measure lead -22.1, chords -23.5, bass -25.1 and drums -27.4.
- **Final summary:** `8 bars looping at 140 BPM, 4 tracks; 13.71 s, -18.0 LUFS, bright, quick attack, tonal, pitch rising from 110 to 784 Hz, 34 onsets, no clipping, loop joins cleanly, 155 of 155 notes heard`.
- **Note check:** 155 of 155 heard, no problems on any track.
- **Loop:** `loop_seam = {clean: true, seam_step: 0.0068, largest_step: 0.165}`. `song_frames` is 604800 (8 × 1.714 s exactly). Peak -6.7 dBFS. `validate` reports no errors and no warnings.
- **Calls:** 9 (list_instruments, validate, create, render+per_track, analyze solo bass+picture, edit, render+per_track, validate, list_files). 0 failed.
- **Confidence:** medium. Every check in the guide's "before you call a tune done" list passes. Whether it is cheerful or catchy needs a person to listen, as the guide itself says.

---

## Where the guide or tools confused, slowed or misled me

1. **The guide is wrong about the coin example interval.** It says "A coin, one fifth higher" with `start = "E6"`, but the preset starts on B5, and B5 to E6 is 5 semitones: a **fourth**. The preset's own TOML comment says "jumps up a fourth". A careful reader will distrust either the guide or their own music theory.

2. **The guide says loop_seam is "in the analysis", but it is not.** The guide says "`loop_seam.clean` in the analysis confirms it". In the `render` result, `loop_seam` is a top-level key next to `analysis`, `note_check`, `song` and `tracks`, not inside `analysis`. My first script looked for it in `analysis` and printed `None`. The same goes for `note_check`, which is top-level in the render result.

3. **Attack metric and words on a multi-layer sound were misleading.**
   - On the preset explosion, the mix said "soft attack" with `attack_seconds: 0.0808`.
   - But each layer alone (`per_layer`) said "sharp attack" (0.004 s and 0.005 s), and the first `energy_over_time` slice was the loudest (-7.6 dBFS).
   - I followed the guide's table ("Does it start hard or soft? → `attack_seconds`") and shortened the noise attack. The metric did not move at all (0.0808 → 0.0808).
   - Then an unrelated change (filter cutoff) flipped it to 0.006 s, "sharp attack".
   - It looks like `attack_seconds` is time to the overall peak. On a sound with a hold plateau, the peak can land anywhere in the plateau. If so, the guide should say it, because "soft attack" on a sound that is full level from the first 5 ms sends you down the wrong path.

4. **`compare` summaries:**
   - For coin, the summary was "same loudness, waveforms partly similar (-0.86)". Calling a correlation of **-0.86** "partly similar" is confusing: a strong negative number reads like "inverted", not "partly similar".
   - Neither compare summary mentioned the change I actually made: the onset moving from 0.070 to 0.052 s for coin, and the attack edit for explosion. I had to dig through the `a`/`b` onsets myself.
   - The guide promises compare "says what changed in plain words". It only says loudness, length, brightness and correlation.

5. **The guide does not say when you are close enough to a loudness target.** The explosion lands at -16.085 with a warning, and the guide says "accept it quieter". But is 0.1 LU acceptable, or 1 LU? I also followed the warning's own advice ("a longer attack") on the thump layer. It gained only 0.014 LU and cost the sharp attack. The advice is generic, and it fits poorly when the peak comes from two layers summing plus distortion.

6. **`extra_onsets` I could not explain:**
   - The bass track (no effects) reported extra onsets in bar 3 with `note_hz: 17982.707` and `14027.6455`. Pitches like that are absurd for a bass.
   - The guide only says "An echo's repeats appear there. They are not errors", but the bass has no echo.
   - The picture of the soloed bass showed nothing obvious.
   - The lead track also had an extra onset at `bar 8, beat 3.99` (13.711 s, the last 3 ms of the file). I guess it is the loop fold or the release, but the guide does not mention it.

7. **Song summaries describe the whole mix with sound-effect words.**
   - The full mix says "pitch rising from 110 to 784 Hz, 34 onsets". For a 4-track song, "pitch rising" and an onset count of the mix mean nothing. Worse, the onset count (25 in the first render, 34 in the second, for the same 155 notes) changed only because I raised the lead's volume.
   - The lead's per-track summary says "pitch rising from 523 to 785 Hz", which is just the first and last pitch-track points.
   - The guide should say to ignore those parts for songs.

8. **"Start every track at 1" did not give a usable balance for a lead tune.** At 1, the pad was 1.6 LU louder than the chip_lead melody, and the guide's "10 LU or more under the others" check passed anyway. I would add: "the melody should usually be the loudest pitched track; raise its `volume` if `per_track` shows otherwise."

9. **Copying a preset is clumsy.**
   - Step 2 says "`create` ... and the `recipe` object from step 1". You have to paste the whole recipe JSON back into `create`.
   - You cannot `create` from `preset:NAME`. `edit` on `preset:NAME` cannot write to a new path.
   - The copied `name`/`description`/`tags` come along unchanged, so my explosion file still says "Medium explosion.", and the preset's comments are lost.
   - A `from: "preset:coin"` on `create` (or an output path on `edit`) would save a step and a big paste.

10. **The levelling ops block lacks context.** "Keep the level while you edit" shows a bare JSON array but does not say it is the `ops` argument of `edit`. It is easy to infer, but it was the one place I had to guess at the call shape.

11. **The guide points to files an MCP agent cannot read.** It sends you to `docs/RECIPE_SPEC.md`, `docs/ANALYSIS.md`, `instruments/README.md`, `presets/README.md` and `examples/songs/duck_craft_day.toml`. No MCP tool shows a file's text: `list_files` lists, and `validate`/`analyze` only check or measure. An MCP-only agent would need `get_schema` instead; the guide should say so. It also mixes in CLI syntax (`--solo drums`) that an MCP user cannot use.

12. **Abrupt endings in one-shot sounds cannot be checked.**
    - For a loop there is `loop_seam`, but for a sound effect with `trim_silence = true` there is no number for whether the end is cut abruptly.
    - The hit's last energy slice was -37.4 dBFS, and its picture shows the level dropping sharply at the very end.
    - The guide says `picture` shows "a click the numbers hide", but I could not tell from a 480-px picture whether that drop is a click.

13. **Small gaps in the guide:**
    - It never states the default `beats_per_bar` (it is 4; `song.beats_per_bar` in the result confirmed it).
    - It does not say that songs render in stereo. I inferred this from the WAV size: 2,419,244 bytes = 604800 frames × 2 channels × 2 bytes.
    - The `edit` result echoes the entire document, which for a song is long. That is not a problem, but I had to truncate it every time.

14. **Untested:** I made no call that failed, so I did not exercise the error report format (`code`/`path`/`allowed`/`suggestion`). I also hit no `missing`/`quiet`/`wrong_pitch`/`uncertain` status in `note_check`.

## What worked well

- The levelling recipe (remove `normalize`/`normalize_peak_db`, set `normalize_lufs`) worked exactly as described. Every render landed on target except where the peak ceiling stopped it, and the warning said so clearly.
- `per_layer` immediately showed the hit preset's buried noise layer. This was the most useful single tip in the guide.
- The song worked on the first validate and first render: 155 of 155 notes, clean loop. The "song in one screen" example plus the rules list was enough to write an 8-bar song without `get_schema`.
