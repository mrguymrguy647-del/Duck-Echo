# Playtest 4: review

Phase 3, step 9. A fresh agent, with only the guide (`get_guide`) and the
MCP server, sent sounds into a real Duck Forge game and checked the game's
mix.

## How it was played

* The server: `target/release/duck-echo-mcp --root playtest/
  --duck-forge <Duck Forge>`, through `bridge.py` (one session per call,
  every call logged in `calls.jsonl`). The Duck Forge folder was a clone
  of its branch `claude/phase3-studio-sounds` (commit 1051264).
* `run_game.sh` stood in for the person: it runs Duck Craft's recorded
  playthrough headless and writes `out/duck_craft/log.json` here. (Its
  `audio.wav`, 2.3 MB, is not kept; the script makes it again.)
* The task: a new landing hit as `land`, an 8-bar looping day tune as
  `music_day` (OGG), footstep variants as `step`; send one again
  unchanged; run the game; `mix_check` its log; fix and check again.
* 41 tool calls, about four minutes.

## What happened

All five parts worked on the first try, and the agent's numbers match the
files:

* `sfx/land.toml` (from `preset:land`, made deeper, with a new "slap"
  layer), `music/day.toml` (C major, 84 BPM, 8 bars, 4 tracks, 144 notes,
  loop joins cleanly), `sfx/step.toml` (6 variants from ranges). The game
  got each WAV or OGG, its facts and its recipe copy in `art/`.
* Sending again answered `unchanged: true, written: []`.
* `mix_check` (`out/duck_craft/log.mix.json`): 13.3 s, -24.4 LUFS, peak
  -6.1 dBFS, no overs, nothing buried. The agent saw that the step played
  on the same frame as the landing was 14.3 dB under it (the line is 15),
  raised the steps to -18 LUFS, sent them again, ran the game again: 12.3
  dB. A sensible fix found by reading every cue.
* It caught on its own that its first tune (140 BPM, the guide's example
  tempo) would not line up with the game's night theme (84 BPM), from the
  lengths in `game_list`.

## Found and fixed

1. **Matching a theme the game already has.** The guide said themes need
   the same tempo, beats per bar and bars, but not where to find them.
   `game_list` now gives each song's `song: {tempo, beats_per_bar, bars}`
   from its recipe copy, and the guide says to match it (test in
   `duck_forge_link.rs`).
2. **"ENDS WITH A CLICK" on the tracks of a clean loop.** Each track of a
   looping song is now judged by its join, as the whole song is (test in
   `playtest2_items.rs`).
3. **`render` with the file as `recipe`** gave "`recipe` must be a JSON
   object". The error now says to give a file as `path` (`from` in
   `create`), and the guide's step 4 says so.
4. **`example:NAME` in `analyze`** was refused with words about drive
   letters. It now says `example:` only copies an example (`create`,
   `file_copy_from`).
5. **"per_track describes each"** read as if the tracks were described.
   It is now "pass per_track to describe each" (and the same for
   `per_layer`); the guide says where `layers` are in the result.
6. **`edit`'s description said comments are not kept**; they have been
   kept since Phase 2. Fixed.
7. **A mono sound reads about 3 LU louder in the mix** than alone (it
   plays in both ears). One line in the guide.

## Still weak (in the roadmap)

* `mix_check` gives a loop's first volume (0.0 for a fade-in) and no
  reason when a loop's loudness is null (never heard); a "nearly buried"
  list (within 3 dB of the line) would have saved the agent reading every
  cue.
* `game_send` does not warn when a new theme's shape differs from a theme
  already in the game (now visible in `game_list`, but not checked).
* Three names for "the file": `path`, `file`, `cues`.
* `hit_seconds` in a song's facts means little; `game_list` shows a
  variant set's first variant's length and loudness only.
* Nobody listened. The agent said so itself: a person should play
  `music/day.ogg` and `out/duck_craft/log.mix.wav`. That is Phase 4.
