# Phase 3 design: the link with Duck Forge

Status: **done**. Approved with all seven questions in section 10 as
recommended. Steps 1 to 4 and step 9's Duck Echo part are in Duck Echo;
steps 5 to 8 and step 9's Duck Forge part are on Duck Forge's branch
`claude/phase3-studio-sounds`. What was learned is in `docs/ROADMAP.md`
("Phase 3 is done").

This plan covers two repositories. Every part is marked with the one it
changes:

* **[Duck Echo]**: this repository, `mrguymrguy647-del/Duck-Echo`.
* **[Duck Forge]**: the game engine, `mrguymrguy647-del/Duck-Forge`.
  Everything said about it here was read from its `main` branch
  (commit `9ebb6ca`): `src/sound.rs`, `src/speaker.rs`, `src/engine.rs`,
  `src/files.rs`, `src/simulation.rs`, `src/script.rs`,
  `src/headless.rs`, `docs/decisions.md` and `studio/`.

Decision already made: **sound effects stay WAV. Music is OGG Vorbis,
streamed while it plays (not decoded whole into memory), and loops stay
seamless.**

## 1. Where we start

### Already done in Duck Echo

* Random ranges (`volume = { min = 0.6, max = 0.9 }`) and
  `sound_variants`: `<name>_1.wav` ... `<name>_N.wav` with
  `<name>.variants.json`.
* A facts file beside every WAV, `<name>.wav.json`: length, loudness,
  peak, `hit_seconds`, `loops`, `seed`, `recipe_hash`.
* `mix_check` reads a cue list. **But see "Found while reading Duck
  Forge" below: it does not yet read a real `log.json`.**

### Already done in Duck Forge

* Stereo WAV files play as stereo (left and right kept apart).
* Volume and pan for each play: `ctx.play_with(sound, volume, pan)`,
  `ctx.play_loop(sound, volume)`, `ctx.set_loop_volume(id, volume)`.
* WAV files at other sample rates are converted to 44.1 kHz with a
  windowed-sinc resampler.

### Still missing

* **WAV files are not watched.** Only text files read through
  `src/files.rs` (levels, legends) are watched, with the game's `reload`
  hook, and recordings keep their text so replays match. A WAV is read
  once by `Wav::load` and never looked at again. (Duck Forge Studio
  watches the `assets` folder for pictures and sends the game a `file`
  message, but a replay "draws today's pictures": pictures are not kept
  in recordings.)
* **No compressed format** for long music.
* Duck Forge **does not read** `.variants.json` or `.wav.json`.
* **No way to pick a Duck Echo sound** in Duck Forge Studio, or to open
  one in Duck Echo.

### Found while reading Duck Forge (to fix first, in Duck Echo)

1. **`mix_check` refuses a real `log.json`.** Duck Forge's log has many
   top-level fields (`game`, `seed`, `frames`, `seconds`, `files`,
   `final`, `tune`, `warnings`, `changes` and more), and its built-in
   synthesizer sounds are written with `wave`, `freq`, `ms` ... instead of
   `file`. `mix_check` treats every unknown field as an error, so it
   stops at the first one. Its tests used hand-written logs.
2. **Its sound paths are read from the wrong folder.** Duck Forge writes
   a file as the game named it (`examples/duck_craft/assets/hum.wav`,
   from where the game ran). `mix_check` reads it from the cue list's
   own folder.
3. **Its pan is not Duck Forge's.** Duck Forge pans by turning one side
   down (`pan_gains`: centre is full volume on both sides).
   `mix_check` uses equal power (centre is -3 dB on each side), so a
   centred sound is 3 dB quieter in the check than in the game.
4. A comment in `variants.rs` says Duck Forge "avoids playing the same
   one twice in a row". Duck Forge does not read the list at all yet.

These are small and come first (build step 1).

## 2. The whole picture

```
   Duck Echo Studio / MCP agent                    Duck Forge game folder
  +---------------------------+   game_send      examples/duck_craft/
  | sfx/step.toml (recipe)    | ---------------> |  art/step.toml       (recipe copy)
  | music/day.toml (song)     |  writes only     |  assets/step_1.wav ...
  |                           |  assets/ + art/  |  assets/step.variants.json
  +---------------------------+                  |  assets/step_1.wav.json ...
                                                 |  assets/day.ogg
                                                 |  assets/day.ogg.json
                                                 +----------+-----------
                                                            | file changed
                                                            v
                                     Duck Forge (running game, window or Studio)
                                       sound watcher: notices, checks, decodes
                                       swaps the sound between two updates
                                       recording: notes frame + file hash
                                                            |
                                     Duck Forge Studio      |
                                       Inspector: pick a sound, see its facts,
                                       "Open in Duck Echo" -> DuckEchoStudio --open art/step.toml
```

## 3. Send to a game [Duck Echo]

### What a person does

In Duck Echo Studio, on a sound's (or a song's) `...` menu: **Send to
game > duck_craft** (the games are listed), or **Send to game > Choose
a game...**. The Studio remembers the last game for each file, so the
second time it is one click. The note says what happened:

* `Sent step to duck_craft: assets/step.wav (0.31 s, -20.0 LUFS).`
* `step is already in duck_craft and has not changed: nothing written.`

### The command and the MCP tool

One named command, so the Studio button, `duck-echo do`, a new
`duck-echo send` and the MCP tool are the same thing:

```json
{"do": "game_send", "file": "sfx/step.toml", "game": "duck_craft"}
{"do": "game_send", "file": "sfx/step.toml", "game": "duck_craft", "variants": 8}
{"do": "game_send", "file": "music/day.toml", "game": "duck_craft", "name": "theme_day"}
{"do": "game_list"}
```

| Argument | Meaning |
|---|---|
| `file` | A sound or song in the Duck Echo project, or `preset:NAME`. |
| `game` | A game's name, as `game_list` gives it (`duck_craft`). Never a path. |
| `name` | The name in the game. Default: the file's name. |
| `variants` | Send N numbered variants and their list instead of one sound. |
| `format` | `wav` or `ogg`. Default: `wav` for a sound, `ogg` for a song. |

`game_list` answers with the games of the Duck Forge folder and, for
each, the Duck Echo sounds already in it (read from their facts files).

### Which folders it may write to

An agent must not write anywhere it likes. The rule is the same as
Duck Echo's project folder today, with one more allowed place:

* **The Duck Forge folder** is a setting. Duck Echo Studio finds it the
  way Duck Forge Studio does (`Documents\Duck Forge` on Windows, the
  `GamesFolder::find` place), or the person chooses it in Settings. The
  MCP server takes it as `--duck-forge DIR`. With no Duck Forge folder,
  `game_send` says how to set one.
* **A game** is a folder `examples/<game>/` in it with a `main.rs`.
  (That is what Duck Forge Studio calls a game today, in
  `studio/crates/core/src/folder.rs`.)
* `game_send` writes **only** into `examples/<game>/assets/` and
  `examples/<game>/art/`, and only files whose names come from `name`
  made safe (`file_safe`). It goes through the same path checks as
  `Root` (no `..`, no links out).

### What it writes

For `{"file": "sfx/step.toml", "game": "duck_craft"}`:

```
examples/duck_craft/
  assets/step.wav          the sound
  assets/step.wav.json     its facts (as today, plus "recipe" and "file_sha256")
  art/step.toml            the recipe, exactly as it is in Duck Echo
```

For a song sent as OGG: `assets/theme_day.ogg` and
`assets/theme_day.ogg.json`. For `"variants": 8`:
`assets/step_1.wav` ... `step_8.wav`, a facts file each, and
`assets/step.variants.json`.

**Where the recipe goes is an open question** (see 10, question 1). You
asked for it beside the WAV. I recommend `art/<name>.toml` because every
Duck Forge game already keeps its sources there (`art/README.md`,
`art/music.toml`), and Duck Forge Studio's "Recipe" view already looks
for `art/<name>.toml` (`studio/crates/core/src/assets.rs`). The facts
file beside the WAV names it: `"recipe": "art/step.toml"`, so the WAV,
its facts and its recipe are linked either way.

Two new fields in the facts file (still `"version": 1`; a reader that
does not know them ignores them):

* `"recipe"`: where the recipe copy is, relative to the game folder.
* `"file_sha256"`: the SHA-256 of the audio file itself.

### Sending again writes nothing when nothing changed

```
game_send step -> duck_craft
  1. render in memory? not yet. First read assets/step.wav.json (if any).
  2. Same recipe_hash, same format and quality, and the WAV on disk still
     has the file_sha256 the facts say?          -> "unchanged", write nothing.
  3. Otherwise render, and write in this order, each one whole:
       assets/step.wav        (written to step.wav.tmp, then renamed)
       assets/step.wav.json   (the same way)
       art/step.toml          (the same way)
```

The check in step 2 needs no rendering, so it is quick, and a game that
is running never sees a file change that changes nothing. Checking the
file's own hash as well catches a WAV someone changed by hand: it is
sent again. Renaming a whole file into place means a running game never
reads half a file.

`recipe_hash` already covers everything that changes the sound (with
the seed and the random ranges). For variants, the list's
`recipe_hash` covers all of them.

### Undo

Sending writes into the game, not into the Duck Echo project, so it is
not a step of Duck Echo's Undo history (like `render`). Duck Forge
Studio's own History already notices files changed outside it ("Changed
outside the Studio") and can take them back.

## 4. Live reload [Duck Forge]

### Today

```
Wav::load("assets/step.wav")  -> Wav { name, path, samples: Arc<[f32]> }
ctx.play_wav(&step)           -> copies the Arc into this frame's sounds
window: speaker mixes it      headless: mixed into audio.wav, listed in log.json
```

The samples are read once. Nothing looks at the file again.

### The plan

**One table of sound files.** A new part of the engine, beside
`files.rs` (call it `sound_files.rs`), keeps every sound file the game
has loaded: its name (as `files.rs` names files, forward slashes), its
samples, and its SHA-256. A `Wav` keeps its name and looks its samples
up in this table when it is played, instead of holding them for ever.
`Wav`'s public functions stay the same (`load`, `name`, `seconds`,
`is_stereo`), so no game has to change.

**Noticing a change.** Two ways in, one way through:

* In a window, the engine already calls `pick_up_edits` twice a second
  to look at text files. It also looks at the sound files: first only
  the size and the modified time (cheap); when those change, it reads
  the file, checks it decodes, and works out its hash. A file that does
  not decode yet (still being written) is looked at again next time.
* Inside Duck Forge Studio, the Studio already watches the game's
  `assets` folder and sends a `file` message for a changed picture. It
  sends the same message for a `.wav` or `.ogg`.

Both end in the same engine function: "this sound file changed".

**Swapping at a safe moment.** The swap happens **between two updates**,
in the same place text files are reloaded today, so it lands on a frame
boundary that a replay can repeat:

* A sound played from that frame on uses the new samples.
* A one-shot sound already playing finishes with the old samples (they
  are shared, so they stay alive until it ends).
* A looping sound switches at its **next wrap** (when it reaches its end
  and starts again), so music never jumps mid-bar. A streamed OGG
  reopens the file at its next wrap.
* `Wav::seconds()` gives the new length from that frame on.
* The game's `reload` hook is **not** needed: a sound changing does not
  change the game's state. (A game may still ask for its own news later;
  not in this phase.)
* The terminal says `duckforge: assets/step.wav changed; plays from frame 913`.

**Recordings and replays stay exactly the same.** Text files are kept
whole in a recording. Sounds are too big for that, so a recording keeps
each sound's **hash**, and the sound itself goes into a small store
named by its hash:

```json
"sounds": {"examples/duck_craft/assets/step.wav": "sha256:9f2c..."},
"changes": [
  {"frame": 913, "sound_file": "examples/duck_craft/assets/step.wav", "sha256": "sha256:4a71..."}
]
```

```
out/sound-store/9f2c....wav    the step as it was on frame 1
out/sound-store/4a71....wav    the step from frame 913 on
```

A replay asks for each sound by its hash. If the store has it, the
replay uses exactly those bytes, so `audio.wav` comes out the same,
byte for byte. If it does not (a recording made on another computer),
the replay uses the file on disk when its hash matches. Otherwise it
plays today's file and warns: `assets/step.wav differs from the
recording; the game plays out the same, audio.wav will not`. The game's
state never depends on a sound's samples, only on when it was played,
so a playthrough's `expect` still passes. (The one exception, a game
that reads `seconds()` or a facts value to decide something, is listed
in the risks.) `out/` is already left out of Git and of Duck Forge
Studio's History, so the store does not grow the repository. It holds
one copy of each version, at most a few MB for sound effects and about
0.3 MB for each OGG version.

## 5. OGG music

### Duck Echo writes OGG [Duck Echo]

* `render -o day.ogg` (and `game_send` with `"format": "ogg"`) writes
  OGG Vorbis. The WAV path is unchanged.
* Encoder: **`vorbis_rs` 0.5** (BSD-3-Clause). It builds Xiph's
  `libvorbis` with the aoTuV tuning from C (about 37,700 lines of C,
  through `aotuv_lancer_vorbis_sys` and `ogg_next_sys`; a C compiler is
  needed, which Rust on Windows already has with the Build Tools). There
  is no mature encoder in pure Rust.
* Quality: **4** (about 105 kbit/s for the Duck Craft themes), a
  setting in the facts file and a `--quality` option.
* The Ogg "serial number" (normally random) is made from `recipe_hash`,
  so the same recipe makes the same stream.
* The facts file `day.ogg.json` says `"frames": 1008000` and
  `"loops": true`, and the file carries the same in a Vorbis comment
  (`DUCKECHO_FRAMES=1008000`), so a player can trim to the exact length
  even without the facts.
* **Duck Echo checks its own OGG** before it keeps it: it decodes it
  with `lewton` (the decoder Duck Forge will use, see below), checks the
  frame count, and for a loop runs the same seam check as for a WAV
  (`loop_seam`). A loop that clicks after encoding is an error, not a
  file. `analyze` and `mix_check` also read `.ogg` this way.

### Sizes, measured

The two Duck Craft themes, rendered from `examples/songs/`, 22.86 s each,
stereo, 44.1 kHz. OGG encoded with Xiph's libvorbis (through `ffmpeg`,
the same library `vorbis_rs` builds; aoTuV may differ a little):

| Theme | WAV (16-bit) | OGG q3 | OGG q4 | OGG q5 |
|---|---|---|---|---|
| duck_craft_day | 4,032,044 B (3.85 MB) | 255,593 B | **298,987 B** | 354,295 B |
| duck_craft_night | 4,032,044 B (3.85 MB) | 242,451 B | **278,871 B** | 333,254 B |

At quality 4 the files are about **14 times smaller**.

### Duck Forge streams OGG [Duck Forge]

* Decoder: **`lewton` 0.10** (MIT or Apache-2.0), pure Rust, about
  6,400 lines, with `ogg` 0.8 (BSD-3-Clause, about 2,700 lines),
  `tinyvec` and `byteorder`. Nothing to build from C, nothing loaded at
  run time. Measured here: it gives back exactly **1,008,000 frames** for
  both themes, the same as the WAV, so the loop length comes through
  exact.
* A new `Music::load("assets/day.ogg")` reads only the headers. It plays
  where a `Wav` loop does: `ctx.play_loop(&music, 0.6)`,
  `set_loop_volume`, `stop_loop`. A scene's `sound` object may name an
  `.ogg`.

**In a window**, one decoder thread per playing stream keeps a buffer
about half a second ahead; the speaker's sound thread only copies from
that buffer and never reads a file.

```
   file  ->  [decoder thread: lewton]  ->  ring buffer (0.5 s)  ->  speaker thread mixes
                    |                         ^
                    | at the last frame:      | the next samples are the first
                    | start again at frame 0  | samples again: no gap
```

**Seamless loop.** The wrap happens in the decoder thread, not in the
sound thread: after the last frame (trimmed to exactly `frames`), it goes
back to the first audio packet and keeps decoding. The ring buffer sees
one unbroken line of samples, so there is no gap and no wait. Measured
on the themes: the jump at the join grows from 0.0031 to 0.0115 (day) and
from 0.0006 to 0.0087 (night) after encoding. The largest jump between
any two neighbouring samples inside the music is 0.10 and 0.05, so both
joins are still inaudible by Duck Echo's own rule. If a song ever fails
that rule, Duck Echo can encode the loop with a little of its start
after its end (so the encoder sees the music go on) and record where to
cut; this is kept in reserve, not built.

**Headless runs** (`audio.wav`, tests, replays) decode the stream in the
mixer, in step with the frames, with no thread, so `audio.wav` is exact
and the same on every run.

### Memory

| | Today (`Wav`, whole file as f32) | Streamed OGG |
|---|---|---|
| One theme | 1,008,000 frames x 2 x 4 B = **8.06 MB** | ring buffer 0.5 s = 176 KB, plus the decoder's own (well under 0.2 MB): **about 0.35 MB** (to measure in step 6) |
| Day and night together | 16.1 MB | about 0.7 MB |

## 6. Variants and facts in Duck Forge [Duck Forge]

### Playing a random variant

```rust
let steps = SoundSet::load("assets/step.variants.json")?; // loads step_1..8.wav
ctx.play_with(&steps, 0.8, 0.0);                          // one of them, chosen at random
```

* `SoundSet::load` reads the list Duck Echo wrote and loads each `Wav`
  (each one watched and reloaded like any WAV). A wrong list is an
  error that names the line and the field, like Duck Forge's other
  errors.
* **Seeded, so replays match.** The choice uses a second random
  generator, `ctx.sound_rng`, made from the run's seed (the seed with a
  fixed number mixed in), not `ctx.rng`. Recordings already keep the
  seed, so a replay picks the same variants. Using a separate generator
  means adding variants to a game does not change the game's own random
  numbers (a serve angle, a spawn place), so its playthroughs still pass.
* It does not play the same variant twice in a row (when the set has two
  or more). That rule only uses `sound_rng`, so it replays the same too.
* `log.json` names the variant: `{"frame": 40, "sound": "step", "file":
  ".../step_3.wav", "variant": 3}`.

### Using the facts

`Wav::load("assets/thud.wav")` also reads `assets/thud.wav.json` when it
is there. Then:

* `thud.hit_seconds()`: when the hit lands in the sound (`None` with no
  facts file).
* `ctx.play_landing(&thud, frames_ahead)`: the game knows the hit happens
  in `frames_ahead` frames (a box will land in 6 frames); the engine
  starts the sound early, so its loudest moment lands on that frame. If
  it is too late to start early, it starts now, part of the way into the
  sound, so the hit still lands on the frame.
* `ctx.play_on_hit(&thud)`: the hit happens now (nothing warned it); the
  sound starts at its hit and skips what comes before.
* Both work in frames and samples only, so headless runs and replays are
  exact.
* `thud.loudness_lufs()` is there for a game that wants to level its
  sounds itself. (Duck Echo's sounds are already at their category's
  target, so most games need nothing.)

```
frame:       100        101        102        103   (box lands on 103)
sound:   [lead-in ...... hit]
         ^ starts here: 103 minus hit_seconds
```

## 7. Duck Forge Studio [Duck Forge] and Duck Echo Studio [Duck Echo]

### Picking a sound in the Inspector [Duck Forge]

A Sound object's **Sound** box today lists the game's `.wav` files. It
will list:

```
Sound  [ step (8 variants)        v ]  [Play] [Open in Duck Echo]
         step_1.wav ... are hidden: the set stands for them
         theme_day.ogg (music)
         thud.wav
0.31 s  -20.0 LUFS  hit at 0.006 s  from art/step.toml
```

* `.ogg` files (with "(music)"), and each `.variants.json` as one entry
  ("step (8 variants)"), whose numbered WAVs are not listed one by one.
* Under the box, the facts (length, loudness, hit) when there is a facts
  file, and which recipe made it.
* A script setting of kind Sound gets the same list.

### "Open in Duck Echo" [Duck Forge] [Duck Echo]

* **[Duck Forge]** Duck Forge Studio finds Duck Echo Studio the way it
  finds Duck Quill (`find_duck_quill`): the program chosen in Settings,
  or `DuckEchoStudio.exe` (Linux: `duck-echo-studio`) next to it, in a
  "Duck Echo Studio" folder beside it, or the installed one. Not found:
  "Where is Duck Echo Studio?" with Choose... and a link to its download
  page. The button is shown for a sound whose facts name a recipe, and
  in Files for any `art/*.toml` that is a Duck Echo recipe. It runs
  `DuckEchoStudio --open <game>/art/step.toml`.
* **[Duck Echo]** Duck Echo Studio learns `--open FILE`. It opens the
  file's game folder as the project for that window (the title says
  "Duck Echo Studio - duck_craft (a Duck Forge game)"), opens the sound
  in the editor, and makes that game the default for Send to game.
  Duck Echo's Undo history for that project is kept in Duck Echo's own
  settings folder, not in the game (so it does not show up in Duck Forge
  Studio's Files and History; see question 4).
* A person can then change the sound and press **Send to game**; the
  running game picks it up (section 4).

## 8. Build steps

Each step is one session, ends with tests passing on both systems, and is
committed on its own branch. Steps marked **(stronger model)** touch
threads, the engine's core or recordings, where a mistake is costly and
hard to see in a test.

### Step 1 [Duck Echo]: mix_check reads Duck Forge's real log [done]

Done as planned, plus two things the real log showed: frame `f` starts
at sample `(f - 1) x 44100 / updates_per_second` (the check was a frame
late), and a loop's volume changes in steps on its frame, not in
straight lines. Checked against Duck Forge's own `audio.wav` for a Duck
Craft run: same length, lined up to the sample, largest difference
0.0036 (the log writes volumes to two decimals). Tests:
`crates/duck-echo-tools/tests/duck_forge_link.rs`, with a real log in
`tests/data/duck_craft_log.json`.


* Read only `updates_per_second`, `sounds` and `loops` from a Duck Forge
  log; its other fields are the game's report and are left alone (a
  plain cue list still rejects unknown fields).
* Built-in synthesizer sounds (`"wave"` instead of `"file"`) are listed
  as "not in the mix: Duck Forge's own synthesizer sound", not an error.
* Sound paths are read from the folder the game ran in (an argument
  `base`, default: the Duck Forge folder when the log is under it).
* Duck Forge's pan for Duck Forge logs; equal power stays for plain
  lists (both documented).
* Fix the variants comment. Tests use a `log.json` written by a real
  Duck Forge run (kept as a test file), with WAVs, synthesizer sounds
  and a loop.
* What you test on Windows: run Duck Craft headless in Duck Forge, then
  `duck-echo mix out\duck_craft\log.json --base <Duck Forge folder>`;
  the summary names its sounds; the picture opens.
* What you test on Linux: the same; and `cargo test --workspace`.

### Step 2 [Duck Echo]: write and check OGG [done]

Done as planned. The Duck Craft themes come out at 299,954 bytes (day)
and 285,796 bytes (night), within 3% of the sizes measured with ffmpeg's
plain libvorbis (aoTuV's tuning spends a little more), decode to
exactly 1,008,000 frames each, and still join cleanly. One thing found:
**`lewton` gives a few hundred frames too many at the end of a short
file** (about a second long), because it trims by the position of the
page before the last one, and in a short file all the music is on one
page; the start always lines up. Long files (the themes) come back exact.
The `DUCKECHO_FRAMES` comment fixes it, and Duck Forge must trim to it too
(step 6). Also noticed: `analyze` of a looping WAV or OGG says "ENDS WITH
A CLICK", because a file alone does not say it loops; its facts file does
(`"loops": true`), and `analyze` could read it (a small step for later).


* `vorbis_rs` and `lewton` as dependencies (justified in
  ARCHITECTURE.md), the Rust version question settled (question 2).
* `render -o x.ogg`, `--quality`, the serial from `recipe_hash`, the
  `DUCKECHO_FRAMES` comment, `<name>.ogg.json`.
* Decode-and-check: frame count, loop seam. `analyze`, `compare` and
  `mix_check` read `.ogg`.
* Tests: both Duck Craft themes round-trip with the exact frame count and
  a clean seam; the sizes are checked against the table above (within
  10%); a short sound effect as OGG is allowed but warned against.
* Windows: render both themes to OGG; play them in a media player; the
  loop in a player that loops (VLC) has no click. Linux: the same, and
  the CI compares the OGG bytes made on both systems (question 3).

### Step 3 [Duck Echo]: game_send and game_list [done]

Done as planned. Two points made firm while building it:

* **"Unchanged" never compares OGG bytes with this computer's.** CI showed
  an OGG's bytes differ between computers. So the check is the game
  folder against itself: the facts' `recipe_hash`, format and quality
  against the recipe, and the sound file against the facts' own
  `file_sha256`. A game folder committed on Windows and sent again from
  Linux with the same recipe says "unchanged" and keeps the Windows
  files, line endings and all (tested, with a valid OGG of different
  bytes and `\r\n` text files). Every facts file now carries
  `file_sha256`, worked out from the file on disk as the facts are
  written, so it always describes the file beside it.
* **A comment is not a change to the sound.** If only the recipe's text
  changed, only `art/<name>.toml` is written.

The Studio keeps the Duck Forge folder in its settings
(`duck_forge_folder`, else `Documents/Duck Forge` when that is one); the
Settings box to choose it and the Send to game menu are step 4.


* The Duck Forge folder setting (Studio Settings, `--duck-forge` for the
  MCP server, `DUCK_ECHO_DUCK_FORGE` for the CLI), the game rule, the
  write rule (only `assets/` and `art/`).
* `game_send` for a sound, a song (OGG) and variants; `game_list`;
  `duck-echo send`. Facts gain `recipe` and `file_sha256`.
* "Unchanged: nothing written" from the facts alone; atomic writes.
* Tests: a made-up Duck Forge folder; second send writes nothing (file
  times unchanged); a hand-changed WAV is sent again; `../` and links
  are refused; an agent cannot name a path as `game`.
* Windows: send a sound into Duck Craft from the CLI; the files appear;
  send again: "unchanged". Linux: the same, plus the MCP tool from an
  agent.

### Step 4 [Duck Echo]: Send to game in the Studio, and --open [done]

Done as planned. Notes for later steps:

* The last game of each file is kept in the Studio's settings, by the
  file's full path (`last_game`); the game a window was opened on comes
  first ("(this game)"), then the last one ("(last time)").
* With `--open` on a game, Home lists only `art/` (a game's other `.json`
  files are scenes, sheets and tune files, not sounds), and New sound and
  Make a copy write into `art/`.
* The history for such a game is in Duck Echo's settings folder, so an
  agent working on the same game through the MCP server (with the game as
  its root) keeps its own `.duck-echo` history there: the person and the
  agent do not share Undo in that one case. Step 8 (Open in Duck Echo
  from Duck Forge Studio) could pass the history folder to the MCP server
  too if that matters in use.


* Settings: "Duck Forge folder" (found, or Choose...).
* The `...` menu on the sound and song screens: Send to game > (games),
  remembering each file's last game.
* `DuckEchoStudio --open FILE`, the game as the project, Undo history in
  Duck Echo's settings folder.
* Tests: the screens send through the command; `--open` opens the right
  file with the game as the default target.
* Windows: from Duck Forge's Explorer folder, run `DuckEchoStudio --open
  ...\art\step.toml`; change it; Send; the WAV changes. Linux: the same
  from the AppImage.

### Step 5 [Duck Forge]: watch WAV files, swap them, keep replays exact (stronger model) [done]

* The sound-file table, `Wav` looking up its samples, the watcher in the
  window, the Studio's `file` message for `.wav`, the swap between
  updates, loops switching at their wrap.
* Recordings: `"sounds"` hashes, `"sound_file"` changes, the
  `out/sound-store`; replays from the store; the warning when a sound is
  missing.
* Tests: a headless run where a WAV changes on frame 40 (given the way
  `files::give` gives text); the replay's `audio.wav` is the same bytes;
  a loop changes only at its wrap; a half-written file is not taken.
* Windows: run Duck Craft in a window, send a changed sound from Duck
  Echo, hear it change within a second; record, replay, compare
  `audio.wav`. Linux: the same.

### Step 6 [Duck Forge]: stream OGG music (stronger model) [done]

* `lewton`, `Music::load`, `play_loop(&music)`, the decoder thread and
  ring buffer, the exact wrap, headless decoding in the mixer, reload at
  the wrap, `.ogg` in scenes.
* **Trim to `DUCKECHO_FRAMES`** (or the facts file's `frames`): `lewton`
  gives a few hundred frames too many at the end of a short OGG (see
  step 2), and a loop must wrap at the exact frame.
* **The two paths give the same samples.** The window's decoder thread
  (with its ring buffer) and the headless in-mixer decoding must produce
  identical output, sample for sample. Both are built on one piece of
  code that decodes and wraps, fed by different callers, and a test runs
  the Duck Craft day theme through both for three loops (3 x 1,008,000
  frames, with a volume change and the wraps in the middle) and checks
  the two outputs are equal, every sample.
* Tests: that test; the Duck Craft day theme loops three times in
  `audio.wav` with every sample equal to the decoded file repeated;
  volume changes and stops land on their frames; memory for one stream
  stays under 1 MB; the sound thread never waits for the decoder in a
  10-minute headless soak.
* Windows: Duck Craft with OGG themes: no clicks at the loop, no gaps
  during heavy frames; Task Manager shows the memory drop. Linux: the
  same with ALSA; `DUCKFORGE_SOUND_BLOCK` small and large.

### Step 7 [Duck Forge]: variants and facts [done]

* `SoundSet::load`, `ctx.sound_rng`, no twice in a row, log lines;
  `hit_seconds`, `play_landing`, `play_on_hit`, `loudness_lufs`;
  `.variants.json` in scenes.
* Tests: a replay picks the same variants; the game's own `ctx.rng`
  numbers are unchanged by playing variants; a landing hit's loudest
  sample is on the right frame in `audio.wav`.
* Windows and Linux: footsteps in Duck Craft sound different each step;
  a recorded walk replays with the same steps (`log.json` matches).

### Step 8 [Duck Forge]: the Inspector, and Open in Duck Echo [done]

* The Sound box lists `.ogg` and sound sets, shows the facts; Open in
  Duck Echo; finding Duck Echo Studio; Files panel.
* Tests: the Studio's screen tests find "step (8 variants)" and the
  facts line; the command line it would run is right.
* Windows: click Open in Duck Echo; Duck Echo Studio opens the recipe;
  Send; the Inspector's facts change. Linux: the same with the AppImage.

### Step 9 [both]: Duck Craft, end to end, and the docs [done]

* Duck Craft's themes become OGG sent from Duck Echo; its footsteps a
  sound set; its playthroughs re-recorded once, on purpose.
* AGENT_GUIDE (send to a game), Duck Forge's AI guide and decisions,
  both ROADMAPs.
* Windows and Linux: the full list above, start to finish, by a person.

## 9. Risks

| Risk | What could happen | Recommendation |
|---|---|---|
| OGG bytes differ between Windows and Linux | `vorbis_rs` is C with floating point; compilers may round differently, so the same recipe could give different OGG bytes on each system. Duck Echo's rule is "same recipe, same bytes". | The WAV stays the pinned, exact render. For OGG, pin the **decoded** samples within a small tolerance, and make `recipe_hash` (not the bytes) what "unchanged" means. Check in step 2 whether the bytes do match; if they do, pin them too. |
| A short OGG decodes too long | `lewton` gives a few hundred frames too many at the end of a file of about a second (found in step 2), so a naive loop would play silence before it wraps. | Duck Echo writes `DUCKECHO_FRAMES` and its facts file says `frames`; Duck Forge trims to it (step 6). Duck Echo's own check already does. |
| The decoder is not bit-exact across systems | `lewton` uses `sin`/`cos` from the system for its tables, so decoded samples may differ in the last bit between Windows and Linux; `audio.wav` from a replay on the other system could differ. | Test in Duck Forge's CI on both systems in step 6 (hash of the 16-bit `audio.wav` of a run with music). If they differ, compute the tables with Duck Forge's own fixed functions (a small patch) rather than accept it. |
| A game decides something from a sound | A game that reads `seconds()` or `hit_seconds()` to plan its logic would play differently if the sound changes. | The sound store makes replays use the recorded bytes, so this only matters for a replay missing its store; the warning says so. Document: plan game logic from frames, not sound lengths. |
| A half-written file | A game reads a WAV while Duck Echo is still writing it. | Duck Echo renames whole files into place; Duck Forge only swaps a file that decodes completely and keeps the old one otherwise. |
| The sound thread starves | A slow disk or a busy computer and the decoder falls behind; the music stutters. | Half a second of buffer, the decoder thread at normal priority, and an underrun counted in the existing "late refills" line. A test feeds a slow reader. |
| Loop clicks after encoding | Lossy coding changes the samples at the join. | Duck Echo decodes its own OGG and refuses a loop whose seam fails; the "start after the end" trick is ready if needed. |
| Two programs write one game | Duck Forge Studio saves a scene while Duck Echo sends a sound. | They write different files (scenes and levels against `assets/*.wav|ogg|json` and `art/*.toml`). Both write by renaming whole files. |
| Recordings change format | Old playthroughs must still replay. | `"sounds"` and `"sound_file"` are new and optional; an old recording simply has none. |

## 10. Open questions (my recommendation first)

1. **Recipe beside the WAV, or in `art/`?** Recommend `art/<name>.toml`,
   named by the facts file (`"recipe": "art/step.toml"`): that is where
   every Duck Forge game keeps its sources and where Duck Forge Studio's
   Recipe view already looks. If you want it beside the WAV
   (`assets/step.toml`), it works the same; only the Recipe view needs a
   change.
2. **Rust version for Duck Echo.** `vorbis_rs` needs Rust 1.87; Duck
   Echo's libraries are held at 1.80. Recommend raising the main
   workspace to **1.87** (Duck Forge already needs 1.87, and the Studio
   1.95). The other way is a separate `duck-echo-ogg` crate at 1.87,
   which adds a crate for no gain.
3. **What "deterministic" means for OGG.** Recommend: the WAV is the
   exact render; an OGG is a delivery copy pinned by its decoded samples
   (and by its bytes if step 2 shows they match on both systems).
4. **Where Duck Echo keeps Undo for a game opened with `--open`.**
   Recommend Duck Echo's own settings folder
   (`%APPDATA%\Duck Echo Studio\projects\<game>`), so nothing hidden
   appears in the game folder. The other way, `<game>/.duck-echo/`,
   is simpler but shows up in Duck Forge Studio's History.
5. **Sound store in `out/`.** Recommend `out/sound-store/`, because
   `out/` is already ignored by Git and History. The cost: a recording
   copied to another computer replays with a warning unless the store is
   copied too. (Keeping sounds inside recordings would make them several
   MB each.)
6. **Variant choice: `ctx.sound_rng` or `ctx.rng`?** Recommend the
   separate `sound_rng`, so adding sounds never changes a game's own
   random numbers or its playthroughs.
7. **OGG quality.** Recommend 4 (about 105 kbit/s, 14 times smaller than
   WAV for the themes), with `--quality` for anyone who wants more or
   less.
