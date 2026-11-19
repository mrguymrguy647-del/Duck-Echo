# Duck Echo roadmap

Duck Echo makes sound for games and videos from text recipes, for AI agents
and for people. This file is the plan: what is done, what comes next, and
in what order. Each task should fit in one working session and says how to
know it is finished ("Accept").

Task numbers are names, not the order. Old numbers are kept so that
commits and notes that mention them still make sense.

Before starting any task: run `cargo test --workspace`, read `AGENTS.md`,
`docs/ARCHITECTURE.md` and `docs/RECIPE_SPEC.md`, and keep their rules.
After any change to the recipe types, regenerate the schema:

```bash
cargo run -p duck-echo-cli -- schema > schema/recipe.schema.json
```

## The plan at a glance

| Phase | What | Status |
|---|---|---|
| 0 | Sound effects foundation: recipes, renderer, analysis, presets, MCP server, clean oscillators, Linux and Windows CI | done |
| 1 | **Music**: songs, starter instruments, seamless loops, note checks, two Duck Craft loops, the agent guide | done |
| 2 | Duck Echo Studio: an app for people, with no terminal | done |
| 3 | Link with Duck Forge, the game engine | done (Duck Forge's part on its branch `claude/phase3-studio-sounds`) |
| 4 | Duck Ear: listening models | |
| 5 | Voices made from math only | |
| Later | Sample layers, video editor link | |

Where the unfinished Phase 0 tasks went:

* **Task 4 (pitched noise and LFOs) moved to Phase 1**, before the
  instruments: drums need pitched noise (hi-hats, snares) and pads need
  slow wobble (LFOs).
* **Task 10 (agent guide) moved to the end of Phase 1**, so it is written
  once, for sound effects and music together. Until then, the MCP
  server's instructions, the tool descriptions and `docs/ANALYSIS.md` guide
  agents.
* **Task 5 (random ranges) moved to Phase 3**: it matters most when a game
  plays the same sound many times (footsteps, hits) and wants each one a
  little different. It was then done at the start of Phase 3, with the
  film fixes.

## Phase 0: sound effects foundation (done)

Everything an agent needs to make, check and fix a sound effect without
hearing it: the recipe format (TOML and JSON, one schema, errors with
exact paths and suggestions), a deterministic renderer, analysis with a
summary line and a picture, 30 presets levelled by loudness, an MCP server
with eleven tools, clean (band-limited) oscillators, and CI on Linux and
Windows. Done tasks: 1, 2, 3, 6, 7, 8, 9, 17, plus review fixes R1 to R10
and playtest fixes P1 to P7. Their notes are in "Phase 0 record" at the
end of this file.

## Phase 1: music (done)

Why music now: the Duck Craft game had to fake its music in Python because
Duck Echo had none. Phase 1 makes Duck Echo the place a game gets its
music from, with the same promises as sound effects: text in, the same
file out every time, and numbers an agent can check.

Goals for the whole phase:

* Songs are text documents, like recipes, and render deterministically.
* **44.1 kHz stereo** is the default for songs (recipes stay mono unless
  they say `channels = 2`).
* Loops join with no seam, so a game can play them forever.
* A small set of starter instruments, so an agent can write a tune
  without designing every sound first.

Order: 16, then 4, then 19, then 20, then 21, then 10. All are done.

### 16. Song document and scheduler [done]

Built as designed in `docs/phase1-design.md`; the reference is the
"Songs" part of `docs/RECIPE_SPEC.md`.

- [x] `Song` in `core::song`: `tempo`, `beats_per_bar`, `bars`, `tracks`
  (each with a required `name`, an `instrument`, `volume`, `pan`, `notes`
  and `play`), `patterns`, the song's own `instruments` and `kits`, and
  the same `master` and `effects` as a recipe. Bars count from 1, beats
  from 0. A chord is a list of notes.
- [x] `core::arrange` checks a song and flattens it into events per track.
  The renderer and the analyzer both read that list.
- [x] A note is a `Voice` (`synth::prepare_note`): pitch moved by
  `note / home`, the envelope let go when the note ends, the instrument's
  effects run once per track, the instrument levelled once.
- [x] Seeds by track name and note start sample.
- [x] `validate`, `render`, `analyze`, `compare`, `create` and `edit` take
  songs in the CLI and the MCP server, through one loader
  (`tools::load_document`). `--per-track`, `--solo NAME`, `--mute NAME`.
  `schema/song.schema.json`, `duck-echo schema song`.
- [x] Stems: a track heard alone uses the full song's master gain, so the
  stems add up to the mix (tested to 0.00001).
- [x] A song's instrument files go through `Root::read` in the MCP server
  (tested: no `..`, no absolute paths, no links out).
- [x] `examples/songs/four_bars.toml` is pinned, and CI renders it on
  Linux and Windows.

Lessons for the next builder:

* **A drum kit is its own table.** The design first put kits inside
  `instruments`. They are now `[kits.NAME]`, a map from piece name to an
  instrument reference (a string). Two tables that both hold tables cannot
  be told apart by the schema walker, and one string per piece is shorter
  to write anyway.
* **The structure walker now handles maps.** `additionalProperties` with a
  schema (what `BTreeMap<String, T>` produces) checks every value against
  it. Use `BTreeMap`, never `HashMap`, in document types, so the order is
  always the same.
* **The loader is generic.** `load.rs` runs the same four steps for
  recipes and songs. A third document type needs a `Document` impl and a
  schema, nothing else.
* **Onset pitch is rough when notes touch.** `onsets[].note_hz` reads the
  100 ms after an onset against the 100 ms before. When a note starts
  while the last one, a semitone away, is still fading, the reading can be
  half a semitone off; low notes are also rough. The demo's long notes
  were shortened a little for this. Task 21 reads the held part of each
  written note instead, which is the right tool for "is this the note I
  wrote".
* **Speed.** The four-bar demo (29 notes, 9 seconds) renders in 0.75 s in
  a release build. Song tests run in a few seconds in test builds
  (`opt-level = 1`), so no profile change was needed yet.
* **Stems are exact only for linear song effects.** Echo and reverb on
  the song are fine; distortion or bitcrush on the whole song means the
  stems no longer add up (the same limit as `per_layer`).

### 4. Pitched noise and LFOs [done]

- [x] `noise_mode = "white" | "pitched"` on a layer (default `white`).
  Pitched noise holds each random value for one cycle of the layer's
  pitch, and follows the note in a song.
- [x] `[layers.lfo]`: `target` (`volume`, `cutoff`, `duty`), `rate`,
  `depth`, `shape` (`sine`, `triangle`, `square`), `delay`. One per layer.
- [x] Unit tests on rate and depth; tests that hear the tremolo, the
  moving filter and the moving pulse; validation with paths and
  suggestions; spec and both schemas updated.
- [x] `examples/recipes/siren.toml`, pinned. Every older fingerprint is
  unchanged.

Lessons for the next builder:

* **Defaults must fit every choice.** The first depth range for `duty`
  was 0..0.45, but the default depth is 0.5, so `lfo = { target = "duty" }`
  alone was an error. The range is now 0..0.5 (the duty is clamped
  anyway). When a range depends on another field, check the default
  against each case.
* **New optional things must not touch the old path.** White noise draws
  nothing extra when the oscillator is made, and the layer loop only does
  LFO work when there is an LFO. That is why no old fingerprint moved.
* **The LFO has no state.** Its wave is worked out from the time, so it
  does not matter how a render is cut into blocks.
* **Held noise is never dark.** Even at 200 Hz it keeps a lot of hiss,
  because each step is a sharp edge. For a deep drum, put a low-pass
  filter after it or use a sine.
* An LFO restarts with each note. A slow filter sweep across a whole
  phrase is not possible yet (listed under Deferred).

### 19. Starter instruments [done]

- [x] `instruments/`: `piano`, `pad`, `bass`, `chip_lead`, and the drum
  pieces `kick`, `snare`, `hat`, `clap`, which make up the kit `drums`.
  All are plain recipes, compiled in (`tools::instruments`). See
  `instruments/README.md`.
- [x] The piano needs no new sound source: six sine layers and a noise
  hammer. It sounds like an electric piano, not a grand.
- [x] A track names one plainly (`instrument = "piano"`).
  `duck-echo instruments list|show`, MCP `list_instruments` and
  `get_instrument`. `instrument:NAME` works wherever a recipe path does,
  and in the MCP server it is checked like `preset:NAME`: a known name
  only, read-only, never a path (tested).
- [x] Every pitched instrument plays a C major scale in each octave of its
  range and reads back within 10 cents; every kit piece is found at its
  time; each is at its loudness target; all eight are pinned, and CI
  renders them on Linux and Windows.
- [x] A new precise pitch reader, `analyze::notes::spectrum_peak`, reads a
  steady tone to a fraction of a cent. Task 21 builds on it.

Lessons for the next builder:

* **Detuned layers can cancel.** The first pad had three equal saws, 5
  cents apart. Twice per beat cycle they cancelled, the note dropped away
  for a moment, and its pitch read 12 cents off. The fix is in the recipe:
  the centre layer is louder than the two side layers together, so they
  can never cancel. Keep that rule for any detuned instrument.
* **A pad cannot be tested like a piano.** It swells in slowly and rings
  for a second after each note, so in a fast scale the note before is
  louder than the new one. The scale test plays the pad in three-beat
  notes and reads the second half of each note.
* **White noise is too bright for a snare.** High-passed white noise has
  its centre near 11 kHz, brighter than the hi-hat. The snare uses a wide
  band-pass at 3.2 kHz instead.
* **Levels.** With each instrument at its target and every track at
  `volume = 1`, a five-track test mix came out within 3 LU from track to
  track. That is the reason for the targets in the table.
* **Filters do not follow the note** (Deferred). Each instrument therefore
  has a useful range, listed in the README. Outside it the instrument
  still plays but sounds dull or thin.
* **Still weak:** the piano's fade is a straight line (the envelope has no
  curved decay), so long notes sound a little flat and electric; all
  notes of an instrument have the same tone whatever their `velocity`.

### 20. Loops with no seam [done]

- [x] `loop = true` on a song. The render is exactly the song's bars, to
  the sample, and everything that still rings after the last bar is
  folded onto the start (`synth::song`, `wrap_loop`).
- [x] `analyze` and `render --json` report the join as `loop_seam`
  (`seam_step`, `largest_step`, `clean`), and the summary says "loop joins
  cleanly".
- [x] Tests, from simple to strict: the length is exact at several tempos;
  the join has no jump (and a hard cut of the same song does); played
  twice, no onset is found at the join; and the loop equals the second
  pass of the same song written out twice and rendered without looping.
- [x] A tail longer than the loop wraps round more than once. Stems of a
  loop are loops and add up to it. `trim_silence` with `loop` is an error.

Lessons for the next builder:

* **The strict test is the one to trust.** "No jump at the join" only
  proves the join is smooth. "Equals the second pass of the song played
  twice" proves the folded tail is the true tail. It passes to within the
  reverb's own -60 dB tail.
* **That test needs sounds without noise.** A note's random stream comes
  from the sample it starts on, so the same note in bar 1 and in bar 3
  has different noise. The test instrument is a sine.
* **The fold is offline.** The player does not loop by itself. Live
  looping in the Studio (Phase 2) should start the next turn's notes
  while the last turn's voices and effects keep running, which gives the
  same result without a fold.
* **The first sample of a loop file is an onset** for the analyzer,
  because the file starts mid-ring. That is the start of the file, not a
  click at the join.
* A loop file starts with the ring of a turn that never happened. A
  separate "first time" file is listed under Deferred.

### 21. Checking music without ears [done]

- [x] `analyze` on a song reports `note_check`: per track, the notes
  written, the notes heard, and every note that is `missing`, `quiet`, at
  a `wrong_pitch` or `uncertain`, with its path, bar, beat, both pitches,
  the cents between them and its level. `notes: true` / `--notes` lists
  every note. The summary line says "N of M notes heard".
- [x] Each note is read from the held part of the note on its own track
  (`analyze::notes`), to a fraction of a cent for a steady tone. In the
  demo every note, the low bass included, reads within 10 cents.
- [x] Starts of sound with no written note (an echo's repeats) are listed
  as `extra_onsets`.
- [x] The picture of a song has a line at each bar, and `bar_lines` gives
  their times. A recipe's picture is unchanged.
- [x] Test: a song with one wrong note is reported with its track, bar,
  beat and both pitches (F5 heard where G5 is written, 200 cents flat).
  Also tested: a silent note, a quiet note, a note too short to read, an
  instrument that sounds a fifth higher than written, chords, drum hits,
  and a note held across a loop's join.

Lessons for the next builder:

* **Ask "is my note there?", not "what is the loudest thing?".** The
  check looks for a peak near the written pitch first. Only when there is
  none does it look at the strongest pitch. This is why a chord note
  under louder notes, or a pad note under the tail of the last chord, is
  still heard.
* **Read where the sound is.** A piano note fades, so the middle of a
  long note can be silent. The check reads from just after the start,
  for at most 0.8 s.
* **It costs a render per track.** `analyze` always pays that; `render`
  only with `per_track` or `notes`. If this gets slow on long songs, the
  stems could be made in one pass by keeping each track's bus.
* **What it cannot do:** tell apart two overlapping notes of the same
  pitch; say whether a note is buried under another track (it listens to
  the track alone); read very short low notes (they are `uncertain`).
* The checked audio is the stem at its level in the mix, so "audible"
  means above -60 dBFS in the finished song.

### Duck Craft demo: day and night loops [done]

- [x] `examples/songs/duck_craft_day.toml` (calm, C major) and
  `examples/songs/duck_craft_night.toml` (dark, A minor): eight bars each
  at 84 BPM, looping, 44.1 kHz stereo, on the built-in instruments.
- [x] Both are exactly 1,008,000 samples, join cleanly, sit at -18 LUFS,
  do not clip, and every written note is heard (113 and 92 notes). Both
  are pinned, and CI renders them on Linux and Windows.
- [x] Tested as a pair: the same length to the sample, night darker than
  day, a half-way fade between them does not clip, and each keeps its
  level when mixed down to mono (Duck Forge plays mono for now).

Lessons for the next builder:

* **The tools were enough to write music blind.** Both tracks were
  written as text and checked only with `analyze`: per-track loudness to
  balance them, the note check to see nothing was lost, the summary word
  ("mid" for day, "dark" for night) for the mood. Nobody has listened to
  them yet. A human should, before they go into the game.
* **Balance came from one pass.** The first drafts had the drums 10 LU
  under the tune and the night's twinkles 20 LU under. `per_track`
  showed it at once. A hint in the summary when one track is far under
  the rest would save that step.
* **Speed.** An eight-bar, five-track loop renders in about 2 s and
  analyzes (with the note check) in about 10 s in a release build. Most
  of that is the pad: three saw layers with a filter and an LFO each,
  held for four beats, three notes per chord.
* **Same tempo and bars means same length.** That is the whole trick for
  fading between two themes. Keep it for every pair of themes a game
  fades between.
* **Phase 1 is done except task 10** (the agent guide), which is next.

### Security fix found by CI: drive letters inside a path [done]

The Windows CI job failed on the new song test, and it was right to: the
MCP root guard (from Phase 0) could be left on Windows.

- [x] The hole: `sub/C:\outside\x.wav` has no `..` and does not start
  with a drive, so it passed the checks. But when the part after the
  existing folder `sub` was joined back on, Windows read `C:\outside\x.wav`
  as an absolute path, and `Path::join` with an absolute path *replaces*
  the folder. A write or read could land outside the root. Linux was
  never affected.
- [x] The fix, in `crates/duck-echo-mcp/src/root.rs`: `:` is refused in
  every path on every platform; the resolved path is built one plain
  name at a time instead of joining a multi-part tail; and the result is
  checked once more to be inside the root. A song's instrument file is
  checked by itself before it is joined to the song's folder.
- [x] Tests: a unit test and a stdio test try drive letters at the start
  and in the middle of read paths, write paths and song instrument
  files, and check that nothing outside the root is read or changed.

Lessons for the next builder:

* **Never `join` text that came from outside.** `Path::join` (and
  `push`) replace the whole path when the argument is absolute, and what
  counts as absolute differs by platform. Push single names you have
  checked.
* **Run the security tests on Windows.** This bug could not show on
  Linux. The CI job on Windows is what caught it; keep it.
* **A red CI is a finding, not noise.** The first five pushes of this
  phase had a red Windows job, and the cause was only read at the end.
  Look at CI after every push.

### No false "extra" notes inside held notes [done]

The note check (task 21) reported many extra onsets that were not
there: 9 on the day bass and 16 on the night bass, all inside held notes
at about 55 Hz, plus one in the day pad. An agent would have believed
them and "fixed" notes that were fine.

- [x] An onset inside a note still held on the same track is explained
  by that note when it brings no new pitch (no clear pitch, a reading
  above 5 kHz, or the held note's pitch or one of its overtones) and the
  level does not jump by 6 dB or more (`tools/src/notecheck.rs`,
  `inside_held_note`). In a loop, notes held across the end count at the
  start too.
- [x] `extra_onsets[].beat` is rounded to the nearest eighth of a beat,
  so it reads like a note an agent could write (2.5, not 2.51).
- [x] Tests: both Duck Craft themes and the four-bar demo report zero
  extra onsets on every track; an unwritten E4 played over a held A3 is
  reported as exactly one extra at bar 2, beat 1; the same pitch struck
  again (a jump in level) is still extra; an unwritten hi-hat is found
  by time; the earlier wrong-note test still passes. Analysis only: no
  render or fingerprint changed.

Lessons for the next builder:

* **The onset finder is tuned for sound effects.** It reports any sudden
  change, and a held low note changes all the time. Explain onsets with
  what you know was written before calling them extra.
* **Overtones count as the same note.** The night bass's false onset at
  439 Hz was the 8th overtone of its 55 Hz A1.
* **The onset finder has a blind spot:** a pure sine entering under
  another pure sine makes no detectable start. Real instruments (with an
  attack, a filter or noise) are seen. Test with the built-in
  instruments, not bare sines.

### 10. Agent workflow guide [done]

* `docs/AGENT_GUIDE.md`: how to design a sound or a tune without hearing
  it. Start from a preset or instrument, change one thing, render, read the
  analysis, compare.
* Level targets: the per-category loudness targets in `presets/README.md`
  (impact -16, weapon -17, alert -18, pickup -19, movement -20, ui -22,
  ambient -24 LUFS), and why peak alone is not enough (at equal peak the
  presets once spanned 13 dB of loudness). Teach `master.normalize_lufs`
  set to the target as the way to level a sound.
* Teach `loudness_method` (compare short clips with short clips),
  `picture: true` for texture, `per_layer` when the summary says a recipe
  has several layers, `onsets` (with `note_hz`) for melodies, and
  `aliasing = "retro"` only when a raw chip sound is wanted.
* A music chapter: writing a song, choosing instruments, checking it with
  task 21.
* Try it with a real agent through the repository's `.mcp.json` (root
  `playtest/`), and commit the playtest folder with its review.
* Accept: another agent given only the guide and the MCP server produces a
  usable coin, hit and explosion, and a looping eight-bar tune, in one
  session.

Progress:

- [x] `docs/AGENT_GUIDE.md` written: the loop (start from a preset or
  instrument, change one thing, render, read, compare), loudness targets
  and `master.normalize_lufs` (with the edit that levels a preset copy),
  `loudness_method`, which field answers which question, `per_layer`,
  `picture`, `onsets`, `aliasing`, and a music chapter (a song on one
  screen, instruments and their ranges, loops, stems, matching themes
  for cross-fading, reading the note check, and a "done" checklist).
- [x] Linked from the README and from the MCP server's instructions.
- [x] `tools/tests/agent_guide.rs` keeps it honest: every TOML example
  validates with no warnings, the song example loops cleanly with every
  note heard, the levelling edit holds the target, and the preset and
  instrument tables match the built-ins.
- [x] Playtest 2 (`playtest/playtest-2/`, with `REVIEW.md`): a fresh
  agent given only the guide and the MCP server made a usable coin, hit,
  explosion and looping eight-bar tune in one session: 38 tool calls,
  none failed, every check passed (155 of 155 notes heard, a clean loop).
- [x] Its confusions fixed: two mistakes in the guide (the coin's interval,
  where `loop_seam` sits), the `ops` of `edit`, what `compare` covers,
  what `attack_seconds` measures, a loudness tolerance, making the melody
  lead, defaults for songs, stale preset metadata, MCP-first wording.
  `compare` now says "upside down" for a negative correlation. The note
  check ignores a pitch that was already sounding before an onset (a
  resonant filter sweeping past), tested on the agent's own tune.

Bigger confusions, for Phase 2 (the first four done in Phase 2 step 2):

* [x] Copying a preset means pasting the whole recipe back into `create`.
  Now `create` takes `from` (`preset:NAME`, `instrument:NAME` or a file),
  which runs the `file_copy_from` command: the copy is named after its
  file, its description says "Copy of the coin preset.", and its comments
  are kept.
* [x] A song's summary uses sound-effect words ("pitch rising from 110 to
  784 Hz", an onset count that changes with a track's volume). Songs now
  have their own summary: shape, length, loudness, clipping, the loop
  join or the ending, notes heard, and the loudest and quietest track.
* [x] A number for a click at the end of a one-shot sound, as `loop_seam`
  does for loops. Now `ending` (`jump`, `largest_step_near_end`,
  `last_level_dbfs`, `clean`), and "ends cleanly" or "ENDS WITH A CLICK"
  in the summary. Analysis only: no render changes. Every preset and
  example ends cleanly.
* [x] The guide and the specs cannot be read through the MCP server. Now
  `get_guide` (and `duck-echo guide`) returns the guide and the pages
  `recipe_spec`, `song_spec`, `analysis`, `instruments` and `presets`,
  compiled in from the repository's own files. The server's instructions
  say to call it first.
* A playtest that starts from broken files, to try the error reports and
  the `missing`, `quiet`, `wrong_pitch` and `uncertain` statuses, which
  this one never met.

Lessons for the next builder:

* **Playtest with the binary you are about to ship.** The playtest's
  server was built a little before the branch's last edits, and it showed
  two false extra notes that were already fixed. Rebuild in release right
  before a playtest.
* **The guide test pays off.** `tools/tests/agent_guide.rs` runs every
  example in the guide. The two factual mistakes the agent found were in
  prose, which no test reads; keep prose claims few and checkable.
* **An agent follows the guide literally.** "Change one thing" gave
  sound effects that are presets with one edit. That is safe, and fine
  for a first version; a later guide can show how to build a bigger
  change one step at a time.
* **The project MCP server times out on a cold start** (the 30 s limit
  is shorter than a release build). Build it first, as the README says;
  in a cloud session, a setup script that builds it would fix this.
* One note-check test reads `playtest/playtest-2/music/tune.toml`. If that
  file is ever changed, keep the bar 3 bass line, or move the song into
  the test.

## Phase 2: Duck Echo Studio

An app for people who never open a terminal, built on the same libraries
and commands the agents use.

Order: 25 first (the command layer every button will use, so Undo and the
MCP tools come for free instead of being added to each button later), then
22, 23, 11, 12, 24. This phase brings the first big dependencies (a window
and widget toolkit, audio output to the speakers, an installer builder);
justify each one in `ARCHITECTURE.md`, as `AGENTS.md` asks.

### 22. Launcher and installer [done: Phase 2 step 4]

* A `setup.exe` for Windows (and a matching package for other systems
  later) that installs Duck Echo Studio with a desktop icon. Starting it
  opens the app; no terminal window, no Rust needed.
* Accept: installs and starts on a clean Windows machine; uninstalls
  cleanly.

Done in Phase 2 step 4:

* [x] `studio/`, a workspace of its own: the window (egui 0.37) with the
  Duck Echo icon and brand colours, the Theme menu (Dark, Light, Same as
  Windows or the system), Help > About with the version, Settings (the
  project folder, the new-version tick box, and on Linux the app menu),
  and a first list of the 30 built-in sounds with Play.
* [x] Sound out loud: the step 3 mixer through `tinyaudio`, DirectSound on
  Windows and ALSA linked normally on Linux (no `unsafe` code of ours).
  The speakers open on the first Play, so a PC with none still opens the
  Studio and says "No speakers were found" in plain words.
* [x] The project folder `Duck Echo` in the real Documents (`dirs`), the
  settings in `%APPDATA%\Duck Echo Studio` or
  `~/.config/duck-echo-studio`.
* [x] `DuckEchoStudioSetup.exe` (Inno Setup): per user, no administrator
  rights, Start menu, desktop icon and "add to PATH" tick boxes (both off
  at first, as in Duck Quill), the three ready-made programs.
* [x] `DuckEchoStudio.AppImage`: one file, three programs (`AppRun`
  picks the Studio, `mcp` or `cli` by the first word), the menu entry and
  icon, and the first-start "Add Duck Echo Studio to your app menu?"
  (Settings removes it; a moved AppImage is followed).
* [x] The update banner on both systems: GitHub's release list through
  `curl`; drafts, pre-releases and older versions ignored; **Update**
  checks the download against the release's `SHA256SUMS` and refuses a
  mismatch; Windows starts the installer, Linux swaps the AppImage in one
  rename and starts again.
* [x] The `Studio` workflow: on Windows it tests (with screenshots),
  builds the installer, installs it silently with both tick boxes, starts
  the installed Studio (which draws its window and closes itself),
  installs a newer copy over it, and uninstalls it, checking that
  `Documents\Duck Echo` and the settings stay and the Path is as before.
  On Linux (Ubuntu 22.04, so the AppImage runs on older systems too) it
  tests (with screenshots), builds the AppImage, runs its three programs,
  plays a sound into ALSA's null device, and starts its window on a
  virtual screen. On both, the update check against pretend release lists.
  The **Run workflow** button publishes the setup, the AppImage and
  `SHA256SUMS`.
* [ ] Home (step 5): cards, the gallery, Make a copy.

Lessons for the next builder:

* **ALSA's null device does not keep time.** It took about 70 million
  frames a second, so a sound started and ended between two looks at
  "is it playing?". `Status::starts` counts sounds that started, and the
  wait uses it.
* **egui's own fonts have no ▶.** egui stops with "No glyph" on a missing
  letter, which the screen tests caught. Keep button words plain, or add
  a font.
* **The test harness resets the theme** when it builds an eframe app: set
  the theme after building, or a "light" picture comes out dark.
* **The window needs `libxkbcommon-x11`** at run time on Linux. Every
  desktop has it; a bare container or CI machine does not.
* **Hover text is not a button's name** for the screen tests: thirty
  buttons are all "Play". Find a row by its sound's name instead.
* **`dirs` instead of `windows-sys`** for Documents: the same Windows
  call, without `unsafe` code of ours, and the XDG folders on Linux.
* **The AppImage is built on Ubuntu 22.04**, the oldest GitHub offers: an
  AppImage only runs on systems with the same or a newer C library.
* **GitHub only shows the Run workflow button for a workflow on the main
  branch.** Until this is merged, the installer and the AppImage are
  downloads on each run's page.

### 23. Home screen [done: Phase 2 step 5]

* Sound cards for the user's recipes and songs (name, length, a small
  picture, a play button) and a gallery of presets and instruments to
  start from.
* Accept: a new user can find, play and copy a preset in under a minute.

Done in Phase 2 step 5:

* [x] "My sounds and songs": a card per recipe and song in the project
  folder, with a picture of its level over time (the same render and
  `energy_over_time` agents read, made on a background thread and kept
  until the file changes), its length or bars, Play, and `...` with
  Rename, Duplicate and Delete. The folder is read again every second, so
  an agent's new file appears by itself.
* [x] The gallery: Presets (by category: Pickup, Impact, Weapon, Alert,
  Movement, UI, Ambient), Instruments (and the drum kit) and Example songs,
  each with Play and **Make a copy**. Songs play from a background render.
* [x] New sound, New song, Search, and Undo and Redo (buttons with the
  step's title, Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z). Every change is a named
  command run as the person: one step of the shared history each.
* [x] `example:NAME` for `file_copy_from` and the MCP `create`'s `from`:
  the example songs, compiled into the tools crate.
* [x] Screen tests walk the checklist: Impact, Make a copy of coin with
  its picture and length, Rename to "my coin" (the `name` field follows),
  Delete, Ctrl+Z brings it back, Redo deletes it again, and three steps in
  the history; New sound, New song, Duplicate, Search; instruments and
  example songs copied. `--play` takes songs, and CI plays two into ALSA's
  null device.
* [x] Clicking a card opens it, and Export WAV in `...`: done in step 6,
  with the sound editor.

Lessons for the next builder:

* **egui's wrapping row does not wrap a framed card.** The cards ran off
  the right edge of the window, where they cannot be clicked; a screen
  test caught it because the test clicked a card that was off screen.
  Home makes its own rows (`cards_per_row`).
* **Thirty buttons are all called "Play".** The screen tests find a
  card's button by its position: the nearest one below the card's name
  (`button_on`).
* **A looping song never ends.** `--play` stops a loop after it has gone
  round once.
* **Good news is not a warning.** The status line shows problems in
  orange and everything else plainly (`say` and `warn`).

### 11. Sliders that change the sound while it plays [done: Phase 2 steps 3 and 6]

* A recipe editor with sliders made from the JSON Schema, next to the text
  with validation messages inline, plus waveform and spectrogram views and
  Play through the computer's speakers.
* Playback uses `Voice` on the audio thread, following the realtime rules
  in `ARCHITECTURE.md` (no allocation and no locks there); the audio thread
  clips its own output.
* Accept: moving a slider is heard within one render; the text and slider
  views always agree because both edit the same `Recipe`; no allocation in
  the audio callback (checked by a debug counter).

Done in Phase 2 step 3 (no window yet; see `ARCHITECTURE.md`, "Realtime
playback"):

* [x] `duck-echo-live`: the mixer. A pool of 32 slots, two `rtrb`
  queues, clipping to -1..=1, play heads as atomics. Finished sounds go
  back to the control thread to be dropped.
* [x] No allocation and no free in the audio block, checked by
  `assert_no_alloc` in `crates/duck-echo-live/tests/realtime.rs`: 60
  seconds of busy playback, a stalled control thread with every queue
  full, and two real threads. A first test shows the detector catching
  an allocation and the freeing of a voice.
* [x] What plays is the rendered file, sample for sample, for every
  preset and example, once and repeated (`tests/matches_render.rs`).
* [x] Ranges in one table (`duck-echo-core/src/ranges.rs`) that
  validation and the schema both read; the schema has `minimum` and
  `maximum` for every number, ready for sliders.
* [x] The background song player (`SongPlayback`): plays the song's
  render, and swaps in a new render at the play head after an edit.
* [x] The window: sliders, text, pictures, Play (step 6, below).

Done in Phase 2 step 6 (the sound editor):

* [x] Click a sound's card (or `...` > Open) to open it: the text on the
  left with line numbers, the sliders on the right, the picture and the
  summary line below, and Play, Loop, Stop, History and `...` along the
  top.
* [x] **Sliders made from the schema** (`studio/crates/app/src/controls.rs`):
  each number with a range is a slider with that range (times and Hz move
  by ratio), fixed choices are drop-downs, a pitch is a slider in Hz with
  a note picker, yes-or-no fields are tick boxes, a table that may be left
  out (`filter`, `lfo`, `vibrato`, `arpeggio`) has "+" and "x" buttons,
  and layers and effects have add and remove buttons. Nothing in it names
  a recipe field, so a new field appears by itself.
* [x] **The text and the sliders always agree**: both change the one file
  through commands (`sound_set`, `file_set_text`, `sound_add_layer`...).
  Typing is saved 0.6 s after the last key, and only when the text works;
  a slider shows its value in the text at once. The editor reads the file
  again twice a second, so an Undo, an agent or a text editor shows up.
* [x] **Problems under their line**, in a light band, with the suggestion
  as a button ("Use `volume`").
* [x] **A drag never floods the disk**: while a slider is held the file is
  saved at most every 200 ms and once more on release, all as one stroke
  (one Undo step). A test drags through 200 values in one second: at most
  6 saves, one Undo step, the file ends at the last value, and only that
  number changed.
* [x] **What you hear is the file**: each change is rendered on a
  background thread (the newest change wins), and the new sound takes
  over from the one playing at the same place with the mixer's 10 ms
  cross-fade (`Player::replace`). The same render draws the analyze
  picture and the summary line.
* [x] **Ctrl+Z in the editor** undoes a step of the shared history even
  while the text box is in use, so the text box's own Undo never
  disagrees with it. Typing with a mistake (never saved) is taken back
  instead.
* [x] **History panel**: every step, newest first, with "You" or "AI
  assistant" and how long ago.
* [x] **Export WAV** (`...` in the editor and on a card): the system's
  Save As window (`rfd`), opening in Music; or "Export WAV to my Duck Echo
  folder", which needs no window (`exports/` in the project). The bytes
  are exactly what `duck-echo render` writes, through the path rules of
  the chosen folder and a one-step write.
* [x] **Comments survive every command** (`toml_edit`, in
  `duck-echo-tools/src/edit.rs`): a command changes only the values that
  changed, inside the text. The result is read back and must be exactly
  the new data, else the file is written whole as before. This also
  fixed `to_toml_document` writing an empty list (`effects = []`) after a
  `[table]`, where it belonged to that table.
* [x] `ParamUpdate` for the volume, cutoff and gain sliders: done in step
  7.

Lessons for the next builder (step 6):

* **egui may "change" a slider by drawing it.** A slider rounds its value
  to the decimals it shows and reports a change: opening the coin turned
  `start = "B5"` into `987.8`. The panel only counts a change while the
  person drags, clicks or has focus on the slider, and a screen test
  opens every preset and instrument and checks no file changed.
* **egui draws an underline in the wrong place on a tall line**, and a
  newline's line height is ignored. The problem band and its underline are
  painted by hand.
* **A label takes the click.** egui's labels are selectable, so a click on
  a card's name selected the name instead of opening the card. Card words
  are not selectable.
* **Run the keys before the text box.** Ctrl+Z is taken from the input
  before the text box draws, or the text box's own Undo would run too.
* **The libraries' Rust 1.80 promise is not checked.** `toml` 1.2 and
  `toml_parser` in the lock file already need Rust 1.85, and so does
  `toml_edit` 0.25.18. CI builds with the newest Rust only. Either raise
  `rust-version` to 1.85 or add a CI job on 1.80 and pin older versions.
  *Settled in Phase 3 step 2: `rust-version` is now 1.87 (the OGG encoder
  needs it), which covers these too. CI still builds with the newest Rust
  only.*

### 12. Smooth live changes [done: Phase 2 steps 3 and 7]

* `ParamUpdate` messages let a slider drag change the running `Voice`
  (volume, cutoff, gain) without rebuilding it; structure changes still
  rebuild.
* Accept: no clicks on a volume drag in a recorded loopback test.

Done in Phase 2 step 3:

* [x] `ParamUpdate` (a layer's volume, a filter's cutoff, the master
  gain) slides to its target over 10 ms, in `duck-echo-synth/src/live.rs`.
  `Voice::process` was not edited: `process_live` wraps it.
* [x] Structure changes build a new voice, which takes over at the same
  place with a 10 ms cross-fade (`Player::replace`); Stop, and Play over
  a playing sound, fade out.
* [x] The click test (`crates/duck-echo-live/tests/clicks.rs`): the
  volume dragged from 0.2 to 1.0 and back in 30 steps on a repeating 55
  Hz tone. Smoothed, the biggest step between two samples is 0.00784,
  the tone's own; with smoothing off it is 0.0538, seven times bigger.
  A stop or a swap with no fade steps 0.78 to 0.98, a hundred times
  bigger; faded, 0.00784. Every scene is run both ways, so the test is
  known to fail without smoothing.
* [x] Every render and fingerprint is unchanged: the pinned fingerprint
  tests pass, and all 104 renders of presets, instruments, examples and
  the playtest files hash the same as before the step.
* [x] The sliders in the window send these (step 7, below).

Done in Phase 2 step 7:

* [x] Play and Loop in the editor play a live voice, not finished
  samples, so a slider can change it while it plays.
* [x] `ParamUpdate::between(before, after)`: the live changes between two
  recipes, or `None` when anything else changed. The editor sends a
  volume, cutoff or gain change at once, and slides the level to the
  file's new level when the render arrives. Other changes (a wave, an
  envelope, a filter added) wait for their render and cross-fade.
* [x] The click test through the editor (`studio/.../tests/smooth.rs`):
  the volume slider dragged from 0.2 to 1.0 and back in 30 steps on a
  repeating 55 Hz tone, through `Editor::slide` and `Live::changed` into
  a mixer with no sound card. The biggest step is the tone's own; with
  the 10 ms slide off, the same drag clicks.
* [x] Settings that do nothing for a layer's source are hidden (a square
  has no noise settings, noise has no duty), from `x-applies-when` in
  the schema (`duck-echo-core/src/applies.rs`).
* [x] Every slider, number box, drop-down and tick box is named by its
  field for screen readers (egui_kittest checks it).

Lessons for the next builder:

* **Do not put live changes inside `process`.** The first idea was a
  "smoothed value" read in the sample loop. Moving values between
  one-frame calls of the untouched `process` keeps every render safe by
  construction, and the cost is too small to measure.
* **`assert_no_alloc` is off in release by default.** Its default
  feature `disable_release` makes it do nothing without debug
  assertions. The crate turns default features off and the two `warn`
  features on, so it always counts, and a test can assert the count.
  In abort mode a violation kills the test program with no message.
* **Prove the detector, then break the code on purpose.** While
  building, the mixer was changed to drop a finished sound itself, and
  to allocate in the block; both times the tests failed. Without that a
  green test says little.
* **Freeing is the easy mistake.** Every path that takes a sound out of
  a slot (ended, faded, replaced, refused, never heard) must hand it
  back. The test "every sound sent comes back exactly once" counts them.
* **A lock cannot be counted.** There is no `assert_no_lock`. The rule
  rests on the ring buffer's own guarantee, a source-reading tripwire
  test, the stall test, and review. Say so, do not pretend.
* **Pick the test tone for the measure.** A step measure only sees a
  click if the sound's own steps are small: a 55 Hz sine. A square or a
  saw has steps as big as any click. And stop it near the top of a wave
  (8 blocks in), not at a zero crossing (9 blocks in), where a cut makes
  no step at all.
* **Presets are normalized, so the voice alone is not the file.** The
  mixer needs the mastering gain and the trimmed length (`Sound`), and
  step 7 must refresh the level after a live change (see the design,
  6.4).
* **The structure walker must not read the ranges.** With `minimum` and
  `maximum` in its schema it reported whole-number fields itself, with a
  slightly different `got`, and pruned the value. It now reads the
  schema without ranges; the published schema adds them.

Lessons for the next builder (step 7):

* **A normalized sound fights a volume slider.** Turning a layer up
  makes `normalize` turn the whole sound down, so the speakers would be
  louder than the file. The live volume change is followed, when the
  render arrives, by a slide of the level to the file's new gain.
* **Only the newest render plays.** A drag asks for many renders; an
  older one arriving late would undo the live changes for a moment.
* **Name every control, or the screen tests fail.** egui_kittest checks
  that every widget has a name. A slider's own number box cannot be
  named, so the panel draws its own number box beside the slider.

### 24. Song timeline [done: Phase 2 steps 8 and 9]

* Tracks as lanes, notes as blocks, patterns as reusable clips, a play
  head, and loop markers; edits change the song document.
* Accept: a song made in the timeline renders the same bytes as the same
  song written as text.

Done in Phase 2 step 8 (to look at and play):

* [x] Click a song's card to open it in the timeline
  (`studio/crates/app/src/song_view.rs` for what it knows,
  `song_screen.rs` to draw it). It is drawn from the song's
  arrangement, the very list of notes the renderer plays, so what you
  see is what you hear.
* [x] A lane per track (name, volume, instrument, **Mute** and
  **Solo**); notes as blocks at their beat and length, higher for a
  higher pitch, with their names; a drum kit's pieces on rows of their
  own; each placed pattern as a clip with its name and transpose.
* [x] The ruler of bars, the loop marker ("loop: bars 1 to 8") for a
  song with `loop = true`, and the play head while it plays.
* [x] Play (once, or looping as the song says) and Loop (always
  repeats), through the background song player. Mute and Solo play the
  stem of the tracks heard at their level in the full mix, as `render
  --solo` writes it (`SongPlayback::set_heard`, `set_looping`), and a
  change takes over at the play head.
* [x] The note check, the same one `analyze` runs, on a background
  thread: the summary line under the lanes, and every note that is
  missing, quiet or at the wrong pitch outlined in red with the reason
  in its tooltip.
* [x] Hover over a note: its name, bar and beat, how long it is held and
  how hard it is played.
* [x] The file is read again twice a second, so an agent's change shows
  (and is heard, if it plays).
* [x] Editing in the timeline: step 9, below.

Done in Phase 2 step 9 (editing the song):

* [x] Double-click an empty spot of a lane: a note there, a beat long,
  on the row's pitch (or kit piece), and you hear it.
* [x] Drag a note to move it in time and pitch (you hear each new
  pitch as it moves; a chord moves together); drag its right edge to
  make it longer; click it and press Delete, or right-click > Delete
  note.
* [x] Drag a pattern clip to another bar; right-click it > Take this
  clip off; right-click an empty spot > Place a pattern at bar N.
* [x] Each track's instrument (a drop-down of the built-in instruments
  and the song's own) and its volume (drag or type).
* [x] Every change is one named command run as the person
  (`song_add_note`, `song_move_note`, `song_resize_note`,
  `song_remove_note`, `song_place_pattern`, `song_move_pattern`,
  `song_remove_pattern`, `song_set`), built in
  `studio/crates/app/src/song_edit.rs`, so each is one Undo step shared
  with agents, and a playing song is heard with the change at the play
  head.
* [x] "A song made in the timeline renders the same bytes as the same
  song written as text": `studio/crates/app/tests/song_edit.rs` makes a
  song with the timeline's commands and compares its WAV bytes with a
  hand-written one.
* [x] From the approval of step 8: beats are shown counted from 1, like
  bars ("bar 1, beat 1"), in labels, tooltips and messages; the file
  keeps counting from 0. A lane is as tall as its notes need: a row per
  semitone (with two spare rows above and below), and on a kit a row
  per piece of the kit, named. The Height slider makes the rows taller
  or shorter, and Snap sets the grid (1, 1/2 or 1/4 beat).
* [x] The add-note command's title no longer names a beat ("Add note E5
  to keys in bar 3"), since people and the file count beats
  differently and both read titles.
* [ ] Notes inside a pattern are changed in the pattern's text for now;
  the timeline moves and places whole clips.

Lessons for the next builder (step 8):

* **Draw from the arrangement, not the file.** Patterns, chords,
  transposes and bars are already worked out there; a note's `path`
  says where it was written, so a clip's notes are known by their
  `patterns.` path.
* **Painted shapes are invisible to tests and screen readers.** Each
  note block is also an invisible widget named "keys: E5, bar 1, beat
  0", so a test can hover it and a screen reader can read it.
* **A tooltip waits half a second.** A test that hovers must let that
  much time pass before it looks.

Lessons for the next builder (step 9):

* **A drag starts a few pixels after the press.** Measure it from
  `pointer.press_origin()`, not from where egui says the drag started, or
  every move comes out a row short.
* **egui counts three quick clicks as a triple-click**, even when the
  first was elsewhere moments before; a test that double-clicks lets
  half a second pass first. egui_kittest moves its clock a quarter of a
  second a frame unless told otherwise (`with_step_dt`).
* **Hearing a note is a help, not the change.** With no speakers the
  note is still added and said; only Play says why nothing is heard.

### 25. Every button is a command, with Undo [done: Phase 2 steps 1 to 10]

* Each action in the Studio is a named command (for example
  `set_layer_volume`, `add_note`) with arguments, recorded on an undo
  stack, and the same commands are MCP tools, so a person and an agent can
  work on the same project and undo each other's steps.
* Accept: every button's action can be undone and redone; each command is
  listed by the MCP server with the same name and arguments.

Done in Phase 2 step 1 (see `docs/phase2-design.md`, section 3):

* [x] 30 commands in `duck-echo-tools/src/commands.rs`, one table, in
  four groups: `file_*` (new sound, new song, copy from a preset,
  instrument or file, rename, duplicate, delete, restore, set the whole
  text), `sound_*` (set or remove a field by path, add, remove and move
  layers, add and remove effects), `song_*` (set or remove a field, add
  and remove tracks, add, move, resize and remove notes, place, move and
  remove patterns, set the loop) and `edit_undo`, `edit_redo`,
  `edit_history`. Names use `_` because Claude's tool names cannot hold
  a dot.
* [x] The MCP server lists one tool per command, same name and arguments
  (schemas from `schemars`); the CLI has `duck-echo commands`, `do` and
  `run`. `create` and `edit` are Undo steps too.
* [x] The Undo history is on disk in `<project>/.duck-echo/history`, so
  the Studio, the CLI and the MCP server share it. Steps say who made
  them; slider strokes join one step; 500 are kept; deleted files go to
  `.duck-echo/recycle`. Changes made by other programs become their own
  step, and Undo never overwrites them.
* [x] No change is lost: compare-before-write under a project lock, one
  retry, then a plain refusal; every file is written to a temporary file
  and renamed into place.
* [x] `Root` (the path rules) moved from the MCP server into
  `duck-echo-tools`, and refuses the `.duck-echo` folder.
* [x] Tests: every command undone and redone gives back every file
  exactly; the real CLI and the real MCP server add 40 layers each to one
  file at the same moment and none is lost; retry-then-refuse; a stale
  lock; Undo after an outside change.
* [x] The Studio's buttons build these commands (steps 5 to 9): Home,
  the sound editor and the song timeline run them as the person.

Done in Phase 2 step 10 (a person and an agent together):

* [x] The Studio notices an AI assistant's change to the open sound or
  song (it looks twice a second), and a note says so for three seconds,
  with Undo right there: "AI assistant: Set layers[0].envelope.decay to
  0.5 in coin.toml" (`studio/crates/app/src/together.rs`).
* [x] A change made by a program that is not Duck Echo (a text editor, a
  sync tool) is recorded as its own step, "Changed outside Duck Echo",
  when the Studio sees it (`Project::notice_outside`), noted the same
  way, and undoable.
* [x] History lists every step with "You", "AI assistant" or "Outside
  Duck Echo", in the sound editor and now in the song timeline too.
* [x] Settings > Connect an AI assistant: the lines for Claude Desktop's
  config (the MCP server next to the Studio on Windows, the AppImage with
  `mcp` on Linux, `--root` set to the sounds folder), a Copy button and
  the steps (`studio/crates/app/src/connect.rs`).
* [x] Playtest 3 (`docs/playtest-3.md`): the real MCP server and the real
  Studio code on one sound and one song; scripted in
  `studio/crates/app/tests/together.rs`, so CI plays it on Windows and
  Linux.
* [x] Found in the playtest and fixed: typing saved over an assistant's
  change made meanwhile (now **Keep mine** or **Use theirs**); a new step
  after an Undo was not noticed (steps are now known by number, time and
  title).

Lessons for the next builder (step 10):

* **Step numbers come back after an Undo.** The next new step takes the
  undone step's number. Anything that remembers "the steps I have seen"
  must remember more than the number.
* **Whole-text saves need a look first.** `file_set_text` writes the
  text it is given; a command that changes data merges by itself, but
  typed text does not. Check the file still holds what the typing
  started from.
* **A new window settles over its first frames.** egui sizes and centres
  a window over a few frames; a test (or a very quick person) clicking
  at once can miss. The tests let it settle for half a second.
* **Test with the real other program.** The playtest found both bugs
  only because the MCP server ran as its own process on the same folder.

### Phase 2 is done

The Studio: install on Windows (setup) or Linux (AppImage), Home with
your sounds and the gallery, the sound editor (text, sliders from the
schema, live sound, Export WAV), the song timeline (look, play, edit),
one Undo history shared with AI assistants, and Connect an AI
assistant. Every change is a named command that agents have too.

Lessons for Phase 3, from the whole of Phase 2:

* **Build the shared layer first.** The command layer (step 1) made
  every later screen small: each button is a command, Undo and agent
  access came free.
* **Every screen gets a screen test that walks its checklist**, with the
  mouse where it can, and pictures in CI. Most bugs of Phase 2 were found
  there, not by hand.
* **egui is quick to build with, but check what it does by itself**:
  rounding slider values, selectable labels, triple clicks, settling
  windows. Each cost an hour once.
* **What you hear must be the file.** Every preview, slider and song
  edit plays the render the file makes; tests compare samples. Keep that
  rule when the game engine plays these files.

Lessons for the next builder:

* **Retry the second time while holding the lock.** The first version
  retried without the lock, and the concurrency test showed two Duck Echo
  programs refusing each other's changes under load. Working the change
  out again under the lock means only a program that is not Duck Echo can
  make a change fail.
* **A test cannot run cargo.** Cargo holds its build lock while tests
  run, so a nested `cargo build` waits forever. The concurrency test
  needs the `duck-echo` binary from another package; it checks the binary
  is newer than every source file and otherwise says to run
  `cargo test --workspace`. An old binary once made this test fail with
  code that was already fixed.
* **Close a child's stdin to end it.** A closure that writes to a child
  only borrows its stdin; dropping the closure does not close the pipe,
  and `wait()` then hangs.
* **Commands rewrite a file from its data, so `#` comments go** (as the
  `edit` tool always did). `file_set_text` and `file_copy_from` keep the
  text. Step 6 added `toml_edit`, so comments now stay on every edit.
* **Errors keep the field path but drop line numbers** when the text was
  made by a command, since nobody has seen those lines.

Phase 2 step 2 (the four playtest-2 items, listed under task 10 above)
is done. Lessons for the next builder:

* **The renderer stops a fading tail at -60 dBFS**, so "near silence" for
  the click check had to be above that: -50 dBFS. At -60 dBFS half the
  presets read as clicks that nobody can hear.
* **A click test needs a cut at the top of a wave.** A 220 Hz tone cut
  at exactly 0.1 s ends on a zero crossing and is clean; the test cuts a
  quarter wave later. A square wave or loud noise never reads as a
  click, because its own steps are as big as the cut: that is the rule
  the design chose, and `ANALYSIS.md` says so.
* **A loop's `ending` is not its click check.** Its last sample leads
  into its first, so `loop_seam` decides, and a loop's summary never says
  "ENDS WITH A CLICK". The `ending` field stays, measured truthfully.
* **One spec file, two pages.** `get_guide` splits `RECIPE_SPEC.md` at
  its "Songs" heading, and a test checks the two parts add up to the
  whole file, so nothing is lost or doubled when the spec grows.
* **Changing summary words means changing the guide.** The guide said
  to check "the summary's brightness word" for a song; songs no longer
  have one, so it now says `spectral_centroid_hz`.

## Phase 3: link with Duck Forge

### Film fixes: what "Pick a Door: Blocks" taught us [done]

Making the sound for "Pick a Door: Blocks", a 58.6 s horror film made with
Duck Forge, showed seven gaps. Each is a named command or a recipe field,
so the CLI, the MCP server and the Studio all have it, and each has tests
in `crates/duck-echo-tools/tests/film_fixes.rs`.

1. **A layer longer than 30 s was cut at 30 s without a word.** Layers
   now run to the 60 s limit; one that would run past it is an error at
   `layers[n].envelope` with the length and a shorter `hold`.
2. **Mix check** (`mix_check`, `duck-echo mix cues.json`): Duck Forge's
   `log.json` as it is, or a plain list in seconds; every moment over full
   volume with the sounds behind it and volumes that fix it; short-term
   loudness each second and per section; buried cues (15 dB under the
   rest) with the volume that lifts them; a picture with a marker and a
   name for each cue; the mix itself as a WAV. Studio: Home > Check >
   Check a mix from a cue list.
3. **Facts beside every WAV** (`<name>.wav.json`, and `asset` in
   `render --json`): length, loudness, peak, leading silence,
   `hit_seconds`, loops, seed, `recipe_hash`. The Studio's Export writes
   it too.
4. **Sound sets** (`set_check`, `duck-echo set-check`): a folder or a set
   file (`schema/set.schema.json`); a table (CSV), a contact sheet (PNG,
   problems in red) and a loudness report per tag with the fix in words.
   Studio: Home > Check > Check all my sounds.
5. **Audio-rate LFO**: volume and the new `pitch` target up to 2000 Hz
   (cutoff and duty stay at 50). Rates up to 50 use the old math, so every
   pinned render is byte-identical.
6. **How it feels**: `roughness`, `sharpness`, `boom`, `movement` and
   `feel` in `analyze`, the words in its summary, and `expect` in a recipe
   (`expect = { roughness = "high", loudness_lufs = [-20, -16] }`) with a
   pass or fail line for each.
7. **Random ranges** (task 5 below) and `sound_variants`: N numbered
   variants with their facts and a `.variants.json` list for Duck Forge to
   pick from at random. Studio: the sound's ... > Export 8 variants.

What the next builder needs to know:

* Ranges ride on the structure walker: a `{ min, max }` table where the
  schema takes a decimal number is recorded on `Recipe::ranges` and
  decoded as its `min`; `structure::check_against` (set files, cue lists)
  keeps them off. Validation checks two corners, all-min and all-max, so a
  rule that joins two ranged fields can still fail at a mixed corner (a
  long ranged attack with a short ranged decay). No such rule bites today;
  check all four corners if one is added.
* The song schema shares its `Master` and `Effect` types with the
  instruments inside it, so it shows the range form on a song's own
  `master` too, where the loader refuses it. Splitting those types would
  make the schema exact.
* The Duck Forge log is read exactly as described when the film was made;
  any other field is an error with its path. When Duck Forge writes more
  (a total frame count, a bus), add it to `mix::ForgeLog` rather than
  loosening the check.
* The mix check holds the whole mix in memory (f32 sides plus f64
  K-weighted copies: about 11 MB a second of mix). It is capped at 600 s;
  a feature film would need it to work in pieces.
* The rest of the mix under a cue is the K-weighted mix minus the cue's
  own K-weighted sound, exact because K-weighting is linear
  (`analyze::k_weighted`). Keep it that way rather than mixing twice.
* The board's font (`analyze::board`) has capital letters only; small
  letters are drawn as capitals. The sound pictures keep their own font,
  so they did not change.
* The Studio shows a ranged field in words ("random, 0.3 to 0.9") instead
  of a slider; ranges are edited in the text. A slider for each end is
  the obvious next step.
* `list_files` now tells `asset_facts`, `variant_list`, `sound_set` and
  `report` files apart from recipes, which keeps them off the Studio's
  Home screen. Any new file Duck Echo writes beside sounds needs a kind.

### 13. Save into a game, with live reload [done: Phase 3 steps 1 to 9]

This section was out of date (it was written before Duck Forge changed,
and before the film fixes). Corrected after reading Duck Forge's `main`
(`src/sound.rs`, `src/engine.rs`, `src/files.rs`, `docs/decisions.md`,
`studio/`):

* **Already done in Duck Echo:** random ranges (task 5 below) and
  `sound_variants` (`<name>_1..N.wav` plus `<name>.variants.json`); a
  facts file beside every WAV (`<name>.wav.json`: length, loudness,
  `hit_seconds`, `loops`, `seed`, `recipe_hash`); `mix_check` reads a cue
  list in Duck Forge's `log.json` form (but see "Found" below).
* **Already done in Duck Forge** (so they are no longer Duck Echo's to
  ask for): stereo WAVs play as stereo (the old "known gap, folded to
  mono" is fixed); volume and pan for each play (`play_with`,
  `play_loop`, `set_loop_volume`); windowed-sinc resampling for WAVs at
  other rates.
* **Still missing:**
  * WAV files are not watched. Only the text files read through
    `files.rs` are, with the game's `reload` hook, and recordings keep
    their text so replays match.
  * No compressed format for long music. Decided: sound effects stay
    WAV; music is OGG Vorbis, streamed while it plays, loops seamless.
  * Duck Forge does not read `.variants.json` or `.wav.json`.
  * No way to pick a Duck Echo sound in Duck Forge Studio, or to open
    one in Duck Echo.
* **Found while reading Duck Forge** (Duck Echo bugs, fixed in step 1
  of the design, along with two more the real log showed: frames start at
  1, and a loop's volume changes in steps): `mix_check` refuses a real `log.json`, because the log has
  many more top-level fields than the cue list and its synthesizer
  sounds have no `file`; it reads sound files from the log's folder,
  where Duck Forge names them from the folder the game ran in; and it
  pans with equal power where Duck Forge turns one side down. A comment
  in `variants.rs` says Duck Forge avoids repeating a variant; it does
  not read the list yet.
* The plan, in nine steps across both repositories, with the measured
  OGG sizes and the open questions: `docs/phase3-design.md`.
* Done: step 1 (`mix_check` reads a real `log.json`), step 2 (Duck Echo
  writes and checks OGG music), step 3 (`game_send` and `game_list`,
  `duck-echo send`), step 4 (Send to game in the Studio, the Duck Forge
  folder in Settings, `DuckEchoStudio --open`). In Duck Forge: step 5
  (WAVs watched and swapped, replays exact through `out/sound-store`),
  step 6 (OGG music streamed with `lewton`, the same samples in a window
  and headless, on Linux and Windows), step 7 (`SoundSet`, `play_landing`,
  facts and their warnings), step 8 (the Inspector lists Duck Echo's
  sounds and opens their recipes). Step 9: Duck Craft uses sent sounds
  end to end, the guide's "Send to a game" chapter, and playtest 4.
* Accept: a Duck Forge game hears a sound change within a second of
  sending it from Duck Echo; sending again with no change writes nothing;
  the recipe and its hash are recorded with the asset; replays stay the
  same; music is OGG and loops with no click.

### 5. Random ranges (moved here from Phase 0) [done: film fixes]

Done as written, with one change: each field picks from its own stream
(the seed mixed with the field's path) rather than one stream in field
order, so adding a range never changes another field's value. Songs take
ranges inside their own instruments, picked note by note.

* Any number in a recipe may be written as `{ min = x, max = y }`;
  `prepare` picks the value from the seed (its own stream, in field
  order). With Duck Forge, a game can ask for a fresh variant each time a
  sound plays.
* Validation applies the field's range to both ends and requires
  `min <= max`.
* Accept: a ranged field stays within bounds across 100 seeds; `variation`
  and ranges work together; errors point at paths like
  `layers[0].volume.max`.

### Phase 3 is done

Duck Echo sends a sound (WAV), a song (OGG) or a set of variants into a
Duck Forge game by name (`game_send`, the Studio's Send to game, `duck-echo
send`), and sending with no change writes nothing. Duck Forge hears the
change within a second, streams OGG music with an exact loop, plays
variants from its seed and lands hits on their frame, and replays sound
the same. `mix_check` checks a game's real headless log. The guide's
"Send to a game" chapter teaches it, and playtest 4
(`playtest/playtest-4/`) did all of it with only the guide: three sounds
sent, the game run, its mix clean, one near-buried step found and fixed.

Lessons from the whole of Phase 3:

* **Design across two repositories, and mark each part with its
  repository.** `docs/phase3-design.md` said which steps were Duck Echo's
  and which Duck Forge's, and the facts file was the one contract between
  them. Nothing had to be redone when the second repository was built.
* **"Unchanged" must be decided from the game's own files**, never from
  this computer's encoder: the OGG bytes differ between Windows and Linux.
  The facts file keeps the recipe's hash and the sound file's SHA-256.
* **A decoder's maths can differ between computers.** `lewton` built its
  tables with the system's `sin` and `cos`, so Windows decoded different
  samples. Duck Forge vendors it with its own fixed maths. Any new
  dependency on the audio path needs a Linux-and-Windows hash test.
* **Decoders give extra frames at the end**, so a loop is trimmed to the
  length written in the file (`DUCKECHO_FRAMES`), or it has a gap.
* **One piece of code fed by two callers** (the window's decoder thread
  and the headless mixer) is how the window and the test give the same
  samples.
* **Paths that cross from one program to the other are plain names
  only.** A game is a name, never a path; a log's sound files are read
  only from plain paths inside the Duck Forge folder.
* **Writing the guide's chapter found a real gap** (`mix_check` could not
  read a game's sounds when Duck Forge sits outside the project). Write
  the chapter, then make its test run it.
* **`cargo fmt` formats vendored code** unless it has a `rustfmt.toml`
  with `disable_all_formatting`.

From playtest 4 (fixed, see `playtest/playtest-4/REVIEW.md`): `game_list`
gives a song's `tempo`, `beats_per_bar` and `bars`; a looping song's tracks
are judged by their join; clearer errors for a file given as `recipe` and
for `example:` outside `create`; "pass per_track to describe each". Still
to do, for whoever next touches `mix_check` and `game_send`:

* `mix_check`: a "nearly buried" list (within 3 dB of the line); a loop's
  highest volume, not its first; a reason when a loop is never heard.
* `game_send`: warn when a new theme's shape differs from a theme the
  game already has.
* One argument name for "the file" across tools (`path`, `file` and
  `cues` today), with the old names still accepted.

## Phase 4: Duck Ear

What Phase 4 should start with (from playtest 4: the agent made a whole
tune, sent it and checked the mix by numbers alone, and said that nobody
could tell whether it sounded good): write `docs/phase4-design.md` first,
as for Phase 3, then build task 14 in this order. The protocol and the
stub `duck-ear`, with the missing-program, timeout and broken-line tests,
come before any model. Then the `ear` section in `analyze`, and only then
real listening models.

### 14. Duck Ear bridge (`duck-echo-ear`) [approved: docs/phase4-design.md]

* The JSON Lines protocol from `ARCHITECTURE.md`, with a stub Python
  `duck-ear` that answers `hello` and `describe` with fixed text, so the
  Rust side and the "Ear not installed" path can be tested without models.
  Then real listening models behind the same protocol.
* `analyze --ear` and the MCP `analyze` tool add an `ear` section when the
  Ear is available, and `ear: {available: false}` otherwise.
* Accept: tests for handshake, timeout, missing program and a broken line;
  the app never waits on the Ear longer than a set timeout.

## Phase 5: voices from math only

* Formant synthesis (see "Voice (roadmap Phase 5)" in `ARCHITECTURE.md`):
  gibberish and robot voices first, invented natural voices later.
* **Never copy a real person's voice**: no cloning, no recordings of
  people as a source, no "sounds like" a named person.
* Accept for the first step: a recipe makes a short gibberish phrase with
  vowels an analysis can tell apart; every render deterministic.

## Later

* **15. Sample layers:** `source = "sample"` with `[layers.sample]` (`path`,
  `hash`, `start`, `loop`), playback rate from the pitch curve; a missing
  or changed file is an error at `layers[n].sample.path`.
* **Video editor link:** render sounds and music to a video's timeline
  (cue points from the editor, files back at the right frames).
* A `delete` tool for the MCP server (same root rules as `create`, only
  recipes, WAVs and pictures).
* Housekeeping: add a `LICENSE` once chosen (MIT + Apache-2.0 dual was
  discussed); publish crates only after the schema has been stable for a
  phase; move the CI's `actions/checkout`, `upload-artifact` and
  `download-artifact` to their Node 24 versions (GitHub warns that the v4
  ones run on deprecated Node 20).

## Phase 0 record

What was built in Phase 0, with the choices made and the lessons for
later work. The original task texts are in the git history.

### 1. Stereo output and per-layer pan

Done. Decision worth knowing: layer effects run on the mono layer signal
*before* the pan, and only master effects run per channel. Running a
nonlinear layer effect (distortion, bitcrush) after the pan would make its
sound depend on the pan position, because the pan gain changes how hard it
is driven; linear ones (echo, reverb) give the same result either way. A
per-layer `spread` for stereo echo and reverb is still open. `analyze` now
reports `per_channel` levels and sums loudness over channels. Example:
`ricochet.toml`.

### 2. Note names for pitch

Done. Choices made: bad names use a new
`invalid_format` code rather than `unknown_variant` (a note is not an
enum), with suggestions for common slips (`H4`, missing octave, `♯`/`♭`);
note-to-Hz uses a literal semitone-ratio table and exact octave scaling,
so it is platform-independent; lower-case letters are accepted; filter
cutoffs stay Hz-only. Existing examples keep their Hz values because
rewriting them as notes would change their pinned bytes; `chime.toml`
shows note names. For the music phase: `core::note::note_number` gives
MIDI numbers, which the song timeline can reuse.

### 3. Band-limited oscillators

Done: `layers[].aliasing`, `clean` by default. Choices and numbers:

* **A 4-point B-spline BLEP, not the usual 2-point PolyBLEP.** Measured on
  a 2 kHz saw at 44.1 kHz, the worst folded tone (at 20.1 kHz, from the
  24 kHz harmonic) is -21.6 dB raw, -31.0 dB with 2-point PolyBLEP and
  -40.4 dB with the cubic B-spline correction, so only the wider one meets
  the 40 dB goal. A 2 kHz square measures -44.9 dB. The price is a softer
  top end (-3 dB at 10 kHz, -7 dB at 15 kHz). Getting both flat highs and
  40 dB would need oversampling or a long filter with delay; not worth it
  for a light, realtime-ready oscillator.
* **No memory and no delay.** The correction is worked out from the phase
  and pitch step alone (`dsp::blep_near`), so `Voice::process` stays
  allocation-free and nothing is delayed against the envelope.
* **Triangle stays raw:** it has no jumps, so it aliases far less.
* **Examples stay retro, presets go clean.** Every square and saw layer in
  `examples/recipes/` says `aliasing = "retro"`, so the examples still
  match their first renders and pin the retro path. The 14 presets with a
  square or saw layer changed sound and were re-pinned; 8 of them also
  needed a new `normalize_peak_db` to stay within 0.1 LU of their target
  (coin moved most, 0.48 LU, because clean high notes lose the folded
  tones' energy). The other 16 presets did not change.
* Tests: `clean_oscillators.rs` (spectrum, saw and square, and that the
  same measurement catches retro), plus unit tests in `dsp.rs`.

### 6. Richer analysis for agents

Done. `analyze` gained `summary`, `crest_factor_db`, `attack_seconds`,
`decay_seconds`, `spectral_centroid_hz`, `tonality` and `pitch_track`;
`compare` gives b-minus-a deltas, a cross-correlation with lag, and a
summary line. Every field and threshold is defined in `docs/ANALYSIS.md`.
Choices: `tonality` (share of power in prominent spectral peaks) was added
because a dominant frequency means nothing for noise and the summary needs
to say which it is; decay starts at the last moment at 90% so held notes
are not counted; `compare` only gives a loudness delta when both sides use
the same loudness method. The FFT lives in `duck_echo_analyze::fft` for
task 3 to reuse. Both commands accept recipes as well as WAV files. A new
`duck-echo-tools` crate holds operations shared with the MCP server.
Known limits, worth knowing when writing the agent guide: `attack_seconds`
measures time to the loudest moment, so a sound whose second layer is
loudest reads as a slow attack (powerup); naive square and saw waves read
"very bright".

### 7. Presets and `variations`

Done. 30 presets in `presets/` across seven categories (the six planned
plus `movement`), compiled into the tools crate so the CLI and MCP server
find them anywhere. Choices: tags live in the recipe as new metadata
fields `description` and `tags` rather than in comments or an index, so
JSON tool calls see them; presets are levelled to per-category loudness
targets (R10) and trimmed; `preset:NAME` works wherever a recipe path does. Every preset's
render is pinned like the examples. `variations` reports `seeds_matter:
false` with a note when nothing depends on the seed, instead of writing
identical files. Conventions and how to add a preset: `presets/README.md`.
Note: presets were first peak-normalized to -3 dBFS, which left their
loudness from -4 to -17.5 LUFS; R10 re-levelled them by loudness per
category.

### 8. Cross-platform determinism in CI

Done. `.github/workflows/ci.yml` runs on every push: on Linux and Windows
it runs clippy (warnings are errors) and every test (and `cargo fmt` on
Linux), then renders every example and preset with a release build
(`scripts/render-fingerprints.sh`) and uploads the SHA-256 of each WAV; a
last job compares the two lists. The guarantees are written down in
`ARCHITECTURE.md`, "What is guaranteed to match".

First run (run 1, commit 610bc5b): all tests passed on both, and the
Linux and Windows lists (40 files each: 10 example files, 30 presets) were
identical, so no fix in `synth::math` was needed. Windows also passed the
golden tests, which compare unoptimized renders with fingerprints made by
a release build on Linux.

Choices: macOS was left out (the plan said Linux and Windows); add it as
a third matrix entry when needed. `.gitattributes` keeps LF line endings on
Windows checkouts, and the MCP server shows paths with `/` on every
platform, so the same tests pass on both. Lesson: keep the render path to
`+ - * /`, `sqrt`, `floor` and bit tricks (see `synth::math`); one call to
the platform's `sin` or `powf` there would break this check.

### 9. MCP server crate (`duck-echo-mcp`)

Done: ten tools over stdio, documented in `ARCHITECTURE.md`, with setup for
Claude Code and Claude Desktop in the README. Choices that differ from the
plan below:

* **No SDK.** The stdio protocol layer is written directly on `serde_json`
  instead of the official Rust MCP SDK, which is async and would add tokio
  and a large dependency tree. Only `protocol.rs` would change to adopt the
  SDK later, for example to add resources or HTTP transport.
* **`get_preset` added**, and `list_presets` returns metadata only, so a
  listing stays small.
* **`--root`** confines all file access to one directory, since clients
  like Claude Desktop give the server no other sandbox.
* **`export` waits for task 13.**

Shared operations (presets, variations, edits, render-to-file) live in
`duck-echo-tools` so the CLI and server behave the same. For task 10: the
`instructions` string sent at `initialize` is the shortest description of
the agent loop; keep it in step with the guide.

### 17. Loudness normalization in the recipe

Done with the playtest 1 fixes, as `master.normalize_lufs` (a number, so
there is no `normalize = "loudness"` mode). Choices that differ from the
plan below:

* **The peak ceiling is fixed at -1 dBFS** (`core::LOUDNESS_CEILING_DBFS`)
  instead of using `normalize_peak_db`. Presets set `normalize_peak_db` to
  whatever peak hits their target (-9.4 for coin), so using it as the
  ceiling capped an agent who added `normalize_lufs = -16` to a preset at
  the preset's old level: the first MCP test of the feature failed that
  way. `normalize` and `normalize_peak_db` are now unused while
  `normalize_lufs` is set, with an `unused` warning on `normalize = true`.
  A target the ceiling does not allow renders quieter with a `suspicious`
  render warning.
* **It measures the finished render** (scaled, clipped, trimmed) and
  corrects over up to four passes, because trimming and the short-clip
  active span move with the gain; the result is within 0.1 LU of the target
  as `analyze` reads the 16-bit WAV.
* **Presets keep their peak levels** (`normalize_peak_db`), so their pinned
  renders are unchanged; the test
  `normalize_lufs_hits_every_category_target_in_one_render` adds
  `normalize_lufs` to every preset as written and checks it lands on its
  category target. Moving presets to `normalize_lufs` would change every
  preset fingerprint; do it only on purpose.
* The deterministic meter is `synth::loudness`; the test
  `render_loudness_meter_matches_the_analyzer` holds it within 0.001 LU of
  the analyzer on every preset and example, at 44.1 and 48 kHz.

### Phase 0 review fixes (R1 to R10)

Found in a review of Phase 0 and fixed before task 1.

- [x] **R1. Report every problem in one validate call.** Validation used to
  stop at the first structural (serde) error, so a recipe with four typos
  and two bad values took five calls. A schema-driven walker
  (`core::structure`) now reports all unknown fields, enum values, missing
  fields and type mismatches with paths, lines and suggestions, and range
  checks run in the same call on a pruned copy. Range errors now carry
  lines too. Test: `one_call_reports_all_six_mistakes_in_toml_and_json`.
  Lesson for later tasks: new recipe types need no walker changes unless
  schemars starts emitting a new keyword, which a unit test catches.
- [x] **R2. Loudness for short sounds.** `analyze` returned
  `loudness_lufs: null` under 400 ms, which covers most sound effects.
  Clips under 400 ms now get an ungated K-weighted loudness over their
  active span (first to last sample above -60 dBFS), and the output names
  the method in `loudness_method` (`bs1770_gated` or
  `k_weighted_ungated`). For a steady tone the two methods agree within
  0.2 dB. Loudness is computed per channel and summed, ready for stereo.
  Note for task 10: the Phase 0 coin reads about -5 LUFS, far louder than
  typical UI targets, so the guide's targets will drive preset levels.
- [x] **R3. Trailing silence.** Computed lengths left 25 to 28% silence
  at the end of explosion, laser and powerup because effect tails are
  worst-case estimates. Added opt-in `master.trim_silence` (cuts after the
  last sample above -60 dBFS, offline, after clipping). The default path
  is unchanged and pinned by `default_render_path_is_byte_identical_to_phase0`,
  which fingerprints every example's WAV against Phase 0. The three
  examples now use trimming. Use that golden test to prove later changes
  (stereo, note names) leave existing renders alone.
- [x] **R4. Coin comment.** `coin.toml` said "a fifth" for steps
  `[0, 5]`, which is a fourth.
- [x] **R5. Check inside misspelled entries.** An unknown key or effect
  `type` with a confident suggestion (`[[layer]]` for `layers`, `"delay"`
  for `echo`) now has its contents validated as the suggested name in the
  same call, reported at the written path; range checks run on it too.
  Confidence comes from `load::suggest`; weak guesses only suggest.
  Test: `misspelled_entries_are_checked_as_the_suggested_name`.
- [x] **R6. Quoted numbers.** `"440"` or `"440hz"` for a pitch (and a
  quoted number for any numeric field) now suggests the bare number.
- [x] **R7. Top-level keys under a table.** Writing `seed = 4` after
  `[master]` in TOML makes it `master.seed`; the error now says the field
  is top-level and must go above the first table header. Found while
  writing the presets, where the template made exactly this mistake.
- [x] **R8. MCP write paths could leave the root (security).** The
  `preset:` prefix skipped the root guard, so `render` with output
  `preset:/../../x.wav` wrote outside the root, and `render` overwrote an
  unrelated `important.txt`; `create` and `variations` had the same hole,
  and `variations` also used the recipe `name` in file names. Fixed in
  `duck-echo-mcp/src/root.rs`: separate read and write resolvers;
  `preset:` only for reads and only with a known name; no `..` or absolute
  paths; symbolic links resolved and kept inside the root; render writes
  only `.wav`; names made file-safe. Tests:
  `write_paths_cannot_leave_the_root`, `read_paths_cannot_leave_the_root`.
  Lesson: every new tool argument that names a file must go through
  `Root::read` or `Root::write`, never `PathBuf::from`.

- [x] **R9. Summary words on noisy sounds.** Tonality used raw power, so
  a low thump dominated: the shotgun read "tonal" and big_explosion read
  "pitch rising" from a window 45 dB below its loudest. Tonality is now
  A-weighted, measured at 46 ms and 186 ms (the longer frame keeps low
  harmonic tones such as engine_hum tonal), the "tonal" bar moved from 0.7
  to 0.6 because weighting lowers fast sweeps, and pitch words ignore
  windows more than 20 dB below the loudest. Shotgun, both explosions and
  missile now read noisy; hit and land (noise bursts over short tones)
  read "tone and noise"; every pitched preset keeps "tonal". Test:
  `summary_words_match_what_the_presets_are`. Analysis only; no render
  changed.
- [x] **R10. Presets levelled by loudness.** At -3 dBFS peak the presets
  ranged from -4 LUFS (ui_error, alarm) to -17.5 LUFS (explosions), so
  feedback sounds were 13 dB louder than explosions. Each preset's
  `normalize_peak_db` is now set so it hits its category's loudness
  target within 0.25 LU (table in `presets/README.md`, constants in
  `presets::LOUDNESS_TARGETS`), with at least 1 dB of peak headroom. All
  preset fingerprints were re-pinned on purpose; example renders are
  unchanged. Tail-heavy presets needed a second pass because
  `trim_silence` uses an absolute -60 dBFS threshold, so a quieter render
  is trimmed sooner. Follow-up: task 17.

### Playtest 1 fixes (P1 to P7)

An agent made eight platformer sounds through the MCP tools only and
reviewed the experience (`playtest/REVIEW.md` on branch `playtest-1`,
rated 8/10). Its findings, fixed together:

- [x] **P1. A picture of the sound.** `render` and `analyze` take
  `picture: true` (CLI `--picture out.png`) and return a 480×254 PNG as an
  MCP image block: level over time (peak, RMS, onset ticks) above a
  log-frequency spectrogram, with a `picture` object describing the axes.
  Drawn in `analyze::picture`, encoded by the in-house `analyze::png` (no
  new dependency). Every analysis also gained `energy_over_time`, RMS in
  20 slices. The reviewer could not tell whether coin's brightness was
  aliasing; the picture shows the alias lines.
- [x] **P2. Per-layer analysis and onsets.** `per_layer`, `solo` and `mute`
  on `render` and `analyze` (CLI `--per-layer`, `--solo 0,2`, `--mute 1`).
  Layers are isolated after `prepare` (`Voice::keep_layers`) and finished
  with the mix's gain and length (`synth::render_layers`), so a layer reads
  as it sounds in the mix. Every analysis gained `onsets`: the start of
  the sound, then spectral-flux peaks, each with the pitch that starts
  there (`note_hz`, the strongest *new* frequency, so a melody over a bass
  reads as the melody). The summary says "N onsets". On the playtest's
  level_complete jingle, one `analyze` call with `per_layer` now gives six
  onsets in the mix (0.1 s steps, the chord at 0.55 s) and reads the lead
  as C5 E5 G5 C6 E6 and the chord as C6, E6 and G6, which the reviewer
  could not verify. Tests pin the note counts of the presets.
- [x] **P3. `master.normalize_lufs`.** Task 17, below.
- [x] **P4. `list_files`.** Lists the server root or a folder in it with
  kinds, sizes and WAV lengths; hidden files skipped, links not followed.
  Logic in `tools::files`. No delete tool yet (the review asked for one
  too; see Open below).
- [x] **P5. Error polish.** `Code::Other` is gone: new codes
  `duplicate_field`, `not_found`, `already_exists`, `path_refused`, `io`,
  `invalid_argument`, and argument errors reuse `type_mismatch`,
  `out_of_range`, `unknown_variant`. `ToolError::Io(String)` became
  `ToolError::Failed(Issue)`; front ends name the argument with
  `ToolError::at`, so no MCP error has an empty path. Shared messages name
  no CLI command (a missing preset lists every name in `allowed`), a
  missing file suggests the closest name and `list_files`, and the server
  replaces its absolute root in every message. Tool descriptions and the
  server instructions explain `isError` (validate answers a bad recipe
  with `isError: false`, `ok: false`) and the two loudness methods. Test:
  `every_error_has_a_code_a_path_and_no_absolute_path` calls each tool
  wrongly 29 ways.
- [x] **P6. Playtest results are committed.** `.gitignore` no longer
  ignores `playtest/`, and WAVs under it are not ignored either. Give each
  playtest its own folder (README, "Playtest in this repository").

Lessons for later tasks:

* Onsets were tuned on every preset, example and playtest sound; the
  thresholds are in `analyze::timeline` with the numbers that chose them.
  If a new source (pitched noise, LFOs in task 4) adds false onsets, check
  the share of new spectrum and the frame tonality first, and add the
  preset to `onsets_count_the_notes_of_each_preset`.
* Isolating layers by removing them from the `Voice` keeps seed streams
  and jitter only because both are drawn per layer index in `prepare`.
  Keep it that way, or solo renders will stop matching the mix.
* A new failure gets a new `Code`, never a catch-all; and every tool
  argument that names a file goes through `Root::read`, `Root::write` or
  `Root::folder` with its argument name.

- [x] **P7. Two small agent helps** (after the playtest). The summary of
  a recipe with several layers ends with "N layers; per_layer describes
  each", because the rest of the summary describes only the whole mix;
  with `solo`/`mute` it says "K of N layers heard". `list_presets` with a
  tag no preset has returns a `not_found` error suggesting the closest
  real tag, instead of an empty list. Tests: `agent_hints.rs` and
  `tag_typos_and_layer_counts_help_the_agent`.

Open, from the same review:

* A note-list layer for melodies (proposed as task 18; now part of task
  16, the song document, in Phase 1).
* A `delete` tool (it needs the same root rules as `create`, and a way to
  refuse deleting anything but recipes, WAVs and pictures).
* `attack_seconds` still measures the time to the loudest moment, so a
  layered sound whose last part is loudest reads "slow attack"; the first
  onset and `per_layer` now show the real start. Task 10's guide should say
  so.
* "Is this brightness real or aliasing?" is answered by the picture, and
  since task 3 the default `clean` oscillators keep folded tones more than
  40 dB down.
