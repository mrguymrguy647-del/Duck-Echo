# Playtest 4: agent notes

I only used `get_guide` and the tool answers (through `bridge.py`). I did not
read any source, docs or other playtests.

## What I made

### 1. Landing thud: `land`
- Recipe: `playtest-4/sfx/land.toml`.
- Started with `create` from `preset:land`, then the guide's first edit
  (removed `normalize` and `normalize_peak_db`, set `normalize_lufs = -20`,
  the movement target).
- Made it mine: the sine drop is now deeper and longer (110 Hz down to 45 Hz,
  decay 0.16 s, volume 0.9), the low noise layer is louder and longer
  (volume 1.5, decay 0.12 s), and I added a third layer: a short band-passed
  noise "slap" at 1800 Hz for a webbed foot. I also added
  `expect = { boom = "high", loudness_lufs = [-20.5, -19.5] }`.
- Render: `0.16 s, ~-20.0 LUFS, mid, sharp attack, noisy, booming, no
  clipping, ends cleanly, 3 layers, expect: all 2 met`. `hit_seconds` 0.021.
- Sent with `game_send {"file": "playtest-4/sfx/land.toml", "game":
  "duck_craft", "name": "land"}`. The game got `assets/land.wav`,
  `assets/land.wav.json` and `art/land.toml`.

### 2. Day music: `music_day`
- Recipe: `playtest-4/music/day.toml`, written by hand as TOML, using the
  guide's "song in one screen" as the shape.
- C major, 8 bars, `loop = true`, `normalize_lufs = -18`. Four tracks: pad
  chords (C, Am, F, G, C, Am, F-G, C), bass from one pattern moved with
  `transpose`, a piano tune (`volume = 1.4` so it leads), and drums
  (`volume = 1.3`).
- Tempo 84, so it is exactly as long as the game's night theme (1,008,000
  samples, 22.86 s). I first wrote 140 BPM (the guide's example tempo), then
  saw the game's night theme is 22.86 s long and changed it (see confusions).
- Analyze: `8 bars looping at 84 BPM, 4 tracks; 22.86 s, -18.0 LUFS, no
  clipping, loop joins cleanly, 144 of 144 notes heard; loudest track tune
  (-20.1 LUFS), quietest drums (-28.9 LUFS)` (before I raised the drums).
- Rendered `playtest-4/music/day.ogg` to check: `loop_seam.clean` true.
- Sent with `game_send {"file": "playtest-4/music/day.toml", "game":
  "duck_craft", "name": "music_day"}`. The game got `assets/music_day.ogg`,
  `assets/music_day.ogg.json`, `art/music_day.toml`.

### 3. Footsteps: `step`
- Recipe: `playtest-4/sfx/step.toml`, written by hand (not from a preset,
  but shaped like the footstep preset and the guide's step example).
- Three layers: a band-passed noise scuff (cutoff 700 to 1300 Hz, decay 0.04
  to 0.07 s), a low noise tap (volume 0.5 to 0.8, cutoff 250 to 400 Hz) and a
  tiny falling sine (start 140 to 180 Hz). `variation = 0.15`.
- `sound_variants` with count 6: `seeds_matter: true`, lengths 0.04 to
  0.07 s, all at the target loudness.
- Sent with `game_send {..., "name": "step", "variants": 6}`. The game got
  `assets/step_1.wav` ... `step_6.wav`, a `.wav.json` for each,
  `assets/step.variants.json` and `art/step.toml`.
- After the first mix check I changed `normalize_lufs` from -20 to -18 and
  sent again (see below).

### 4. Sending again unchanged
`game_send` of `land` a second time answered:
`"summary": "land is already in duck_craft and has not changed: nothing
written.", "unchanged": true, "written": []`. The same for the 6 step
variants: `"step (6 variants) is already in duck_craft and has not changed:
nothing written."` Clear, and it matched the guide.

## What the mix check said

Round 1 (`mix_check {"cues": "playtest-4/out/duck_craft/log.json"}`):
- Summary: `13.3 s, -24.4 LUFS, peak -6.1 dBFS; never over full volume; no
  cue buried; not in the mix: 17 of Duck Forge's own synthesizer sounds
  (click, dig, break, place, pick, quack)`.
- `overs` empty. Nothing `buried`.
- `land` at volume 1.0: -17.0 LUFS in the mix. It is the loudest moment (the
  spike at 11.5 s in `log.mix.png`, peak -6.1 dBFS).
- `music_day`: -25.1 LUFS in the mix (the game fades it in).
- `step` at volume 0.3: -27.4 LUFS each. The step at 11.467 s plays on the
  same frame as `land`; its `rest_lufs` was -13.1, so it was 14.3 dB under,
  only 0.7 dB away from the 15 dB "buried" line. The other steps were a
  little under the music.

What I did: raised the step recipe to `normalize_lufs = -18` (2 LU over the
movement target), sent the variants again, ran the game again.

Round 2: same summary line, `overs` empty, nothing buried. Steps now -25.4
LUFS in the mix (about level with the music at -25.1), and the step under the
landing is now 12.3 dB under instead of 14.3. Land and music unchanged. I
stopped there.

## Where the guide or tools confused me

1. **`render` with a file.** I passed the file as `recipe` and got
   `"code":"type_mismatch","message":"`recipe` must be a JSON object","path":"arguments.recipe"`.
   The guide's step 4 says "`render` with an `output` such as
   `sfx/coin.wav`" but never says the file goes in `path`. The error has no
   suggestion like "for a file, use `path`". `edit` and `analyze` take
   `path`, while `game_send`, `sound_variants` take `file`, and `mix_check`
   takes `cues`: three names for "which file".
2. **`example:` works in `create` but nowhere else.** `analyze` with
   `"path": "example:duck_craft_night"` failed:
   `"path `example:duck_craft_night` is refused: `:` is not allowed in a path
   (drive letters and absolute paths are not allowed ...)"`. The guide says
   `preset:NAME` and `instrument:NAME` work "anywhere a file path is
   accepted", so `example:` not working is consistent, but the error talks
   about drive letters instead of saying "examples can only be copied with
   create". I had to copy the example into my folder, read it, and delete it.
3. **Matching the day theme to the night theme.** The guide says two themes
   a game fades between need "the same `tempo`, `beats_per_bar` and `bars`",
   but it does not say what the game's night theme uses. Its example song is
   140 BPM; Duck Craft's night is 84 BPM. I only noticed because `game_list`
   showed `music_night` as 22.857 s and my 140 BPM tune was 13.71 s. `game_list`
   shows a duration but not tempo or bars. Nothing warned me at `game_send`
   that my day theme would not line up with the night theme already there.
4. **"ENDS WITH A CLICK" on a looping song.** With `per_track: true` the
   chords and bass tracks said `ENDS WITH A CLICK`, while the whole song said
   `loop joins cleanly`. The guide says to read `loop_seam` for a loop, so I
   think it is fine, but the per-track words look like an error.
5. **`render` of a song said "per_track describes each"** in its summary
   although I did not pass `per_track`, and the result had no `tracks`.
6. **`hit_seconds` on music.** The music's facts file has `hit_seconds:
   4.301`. For a loop that number means nothing to me.
7. **Where `per_layer` goes.** In `render` the per-layer results are at the
   top level as `layers` (each `{index, analysis}`), not inside `analysis`.
   The guide does not say where to look; I first thought it was empty.
8. **`edit` and comments.** The `edit` tool description says "comments are
   not kept". The guide says "a person's comments and layout stay". In my
   files the comments were kept, so the tool description seems out of date.
9. **Which category is a landing thud?** The task calls it a "hit sound"
   and a "thud". The guide's table puts `thud` in impact (-16 LUFS) and
   `land` in movement (-20 LUFS). The game's existing `thud` is at -16. I
   guessed movement because it is a landing.
10. **mix_check `volume` for loops.** The music_day cue shows `"volume":
    0.0` but `"loudness_lufs": -25.1`. The log shows the volume fades up from
    0, so the report gives only the first value. `music_night` and
    `heartbeat` show `loudness_lufs: null` with no word on why (I guess their
    volume stays 0). A "max volume" or "not heard" note would help.
11. **Mono sounds are ~3 LU louder in the mix.** My land is -20.0 LUFS
    alone but -17.0 in the mix at volume 1.0. I guess a mono sound is placed
    in both stereo channels. The guide does not say this, so the numbers
    looked wrong at first.
12. **Nearly buried.** The step on the same frame as the landing was 14.3 dB
    under, just inside the 15 dB line, and the report says "no cue buried"
    with no hint it was close. I only saw it by reading every cue.
13. **game_list for variants** shows `duration_seconds` and `loudness_lufs`
    of what seems to be the first variant only, with `file` set to the
    `.variants.json`.
14. **History and plain files.** Files I wrote by hand (`day.toml`,
    `step.toml`) did not show up in `edit_history` when created, but my
    later `sed` change to a comment in `day.toml` did, as "Changed outside
    Duck Echo". Not a problem, just surprising.
15. **Local variants go stale.** `sound_variants` wrote `sfx/step_1.wav`...
    at -20 LUFS; `game_send` with `variants` renders its own. After my
    change to -18 the local WAVs were old and the game's were new. Fine, but
    easy to compare the wrong files.

## What I wished existed

- An error hint on `render`/`analyze` when `recipe` gets a string: "did you
  mean `path`?". One name for "the file" across tools.
- `game_list` showing a song's tempo, bars and `song_frames`, and a
  `game_send` warning when a song will not line up with a theme already in
  the game (e.g. same name prefix `music_`).
- `analyze`/`render` accepting `example:NAME` like `preset:NAME`.
- In `mix_check`: a "close to buried" list (say within 3 dB of the line),
  the max (not first) volume of a fading loop, and a short reason when a
  loop's loudness is null.
- A one-line note in the guide that a mono sound reads about 3 LU louder in
  the stereo mix.
- Per-track summaries in a looping song that do not say "ENDS WITH A CLICK".
- A way to hear things. Duck Echo cannot say whether the tune is good, and
  neither can I: a person should listen to `music/day.ogg` and the mix
  `out/duck_craft/log.mix.wav`.
