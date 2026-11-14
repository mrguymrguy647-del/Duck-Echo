# Phase 2 design: Duck Echo Studio

This is the plan for Phase 2 (roadmap tasks 25, 22, 23, 11, 12 and 24).
It was written before any code and approved with "yes to all", except
question 6 (Linux), plus four changes on approval. The changes are
marked **Changed on approval**.

The short version:

* **Duck Echo Studio** is an app for Windows and Linux. On Windows it
  installs with one `setup.exe`; on Linux it is one AppImage file. It
  needs no Rust, no terminal and no build. It ships ready-made programs.
* It looks and installs like its two siblings. It uses **egui** like Duck
  Quill and Duck Forge Studio, the same brand colours and theme menu, and
  the Home screen, installer, update banner and named commands the two
  share.
* **Every action is a named command with Undo.** A button, a key, a
  script and an agent's MCP tool all run the same command. A person and
  an agent can edit the same sound and undo each other's steps.
* **Sounds play live.** Moving a slider changes the sound while it plays,
  with no clicks. The audio thread never allocates memory and never
  waits on a lock.
* Everything the Studio does without a window lives in plain Rust
  libraries with tests. The window only draws and turns clicks into
  commands.

Section 10 lists the open questions, each with my recommendation, so you
can answer "yes to all" or pick the ones to change.

---

## 1. What the family already does, and what we reuse

I read both sibling repositories before writing this.

Duck Quill's installer, Home screen and update checks are on its
unmerged Phase 6 branch, `claude/stoic-cannon-vyu6qr`
(`docs/phase6-design.md` and `release/installer.iss` there). I read both
that branch and Duck Forge Studio. Where the two agree, Duck Echo does
the same. Where they differ, it follows Duck Forge Studio, unless Duck
Quill's way is plainly safer.

| Thing | Duck Quill (Phase 6 branch) | Duck Forge Studio | Duck Echo Studio |
|---|---|---|---|
| Window and widgets | egui 0.37 (eframe) | egui 0.37 (eframe) | **egui 0.37**, same |
| Where the app code lives | `crates/app` beside the engine | its own workspace, `studio/` | **its own workspace, `studio/`** |
| Logic without a window | the engine crate | `studio_core` | `duck-echo-tools` and a new `duck-echo-live` |
| Every action is a command | one list of every command, used for menus, shortcuts and Ctrl+K search | one `Command` type, `{"do": "level.paint"}` | **one `Command` type and table**, also behind Ctrl+K search |
| Undo | commands in memory | steps that name files, before and after | **steps that name files, kept on disk** |
| Installer | Inno Setup, per user, no admin, Start menu, uninstall entry, desktop icon ticked off by default, optional "add to PATH" box for agents | Inno Setup, per user, no admin, Start menu, uninstall entry, desktop icon ticked on | **Inno Setup**, per user, no admin; desktop icon **off** by default and an **"add to PATH" box**, as in Duck Quill |
| Install folder | `%LOCALAPPDATA%\Programs\Duck Quill` | `%LOCALAPPDATA%\Programs\Duck Forge Studio` | `%LOCALAPPDATA%\Programs\Duck Echo Studio` |
| Home screen | at start: big cards of recent work, New buttons, update banner top right | at start: cards, starters, rename, duplicate, delete, update banner | **at start: cards, a gallery, New buttons, update banner top right** |
| Update check | GitHub releases; skips drafts and pre-releases; never an older or equal version; nothing about you is sent; a Settings tick box; a SHA-256 check of the setup before it runs | GitHub releases through Windows' own `curl.exe`; the same rules; checks the download's size | **the rules both share, `curl` as in Duck Forge, and Duck Quill's SHA-256 check** |
| Linux | not shipped | not shipped | **an AppImage**, built and tested in CI |
| Sound out loud | none | `tinyaudio` (Windows, macOS), ALSA on Linux | **`tinyaudio` on both systems**, plus a lock-free mixer |
| Screenshots with no window | none | `egui_kittest` | **`egui_kittest`**, same |
| Brand | night indigo, duck yellow, ink navy | the same | **the same**, plus an Echo icon |

**Changed on approval:** the first version of this table said Duck Quill
had no installer. It has one, on the Phase 6 branch.

---

## 2. Where the code goes

```
Duck-Echo/
  crates/                 the library workspace, as today (Rust 1.80, no window)
    duck-echo-core          recipe and song types (+ ranges for sliders)
    duck-echo-synth         renderer (+ smoothed live changes)
    duck-echo-analyze       analysis (+ click check, song summary)
    duck-echo-tools         shared operations (+ commands, project, history)
    duck-echo-live   NEW    the realtime mixer: voice pool, queues, no window
    duck-echo-cli           duck-echo.exe (+ `run` for command scripts)
    duck-echo-mcp           duck-echo-mcp.exe (+ one tool per command)
  studio/          NEW    its own workspace (Rust 1.95, egui)
    crates/app              DuckEchoStudio.exe: the window only
    release/                DuckEchoStudio.iss, READ ME FIRST.txt,
                            linux/ (AppRun, .desktop file, icon)
    brand/                  icon and logo
```

Why `studio/` is its own workspace (as in Duck Forge):

* egui 0.37 needs Rust 1.95. The libraries promise Rust 1.80 today, and
  agents and CI build them all the time. Keeping egui out means
  `cargo test --workspace` stays as fast as it is, and the command-line
  tool and the MCP server are untouched.
* The Studio uses the libraries through path dependencies, so it always
  runs the very same code an agent's MCP server runs.

Everything a command does lives in `duck-echo-tools`, as CLAUDE.md asks.
The Studio app holds no logic of its own that an agent cannot reach.

---

## 3. The command layer (task 25)

### 3.1 One type, one table

Every action is one value of one Rust type, `Command`, in
`duck-echo-tools/src/commands.rs`. A command has a name, arguments, and a
plain title for Undo. It returns what changed in a few words, or a
`Report` with paths and suggestions, like every tool today.

```json
{"do": "sound_set", "file": "sfx/coin.toml", "path": "layers[0].volume", "value": 0.6}
{"do": "song_add_note", "file": "music/tune.toml", "track": "lead", "bar": 3, "beat": 1, "note": "E5"}
{"do": "file_copy_from", "from": "preset:coin", "to": "sfx/my_coin.toml"}
{"do": "edit_undo"}
```

Names are `group_action`, with an underscore. Duck Forge uses a dot
(`level.paint`), but the Claude API only accepts tool names that match
`^[a-zA-Z0-9_-]{1,128}$` (letters, digits, `_` and `-`; its "Define
tools" page). One name everywhere (script, button and MCP tool) matters
more than matching a dot. See question 4.

The first command list (more come with each step):

| Group | Commands | Undo? |
|---|---|---|
| file | `file_new_sound`, `file_new_song`, `file_copy_from` (a preset, an instrument or a file), `file_rename`, `file_duplicate`, `file_delete` (to the Studio's recycle bin), `file_restore` | yes |
| sound | `sound_set` (one field by path: every slider uses it), `sound_remove`, `sound_add_layer`, `sound_remove_layer`, `sound_move_layer`, `sound_add_effect`, `sound_remove_effect`, `sound_set_text` (the whole text, from the text box) | yes |
| song | `song_set` (by path), `song_add_track`, `song_remove_track`, `song_add_note`, `song_move_note`, `song_resize_note`, `song_remove_note`, `song_place_pattern`, `song_move_pattern`, `song_remove_pattern`, `song_set_loop` | yes |
| edit | `edit_undo`, `edit_redo`, `edit_history` (the list of steps) | no |
| play | `play_start`, `play_stop`, `play_loop` | Studio only, no |

A table beside the enum gives each command its name, title, argument
schema (made by `schemars`, like the recipe schema) and whether Undo
covers it. Three things are made from that one table, so a new command is
one new entry:

* the **MCP tool list**: each command becomes a tool with the same name
  and arguments, added to the 13 tools there are today;
* `duck-echo commands`, which lists them, and `duck-echo run script.json`,
  which runs a list of them with no window (like Duck Forge's `--run`);
* the Studio's buttons, which build the same values.

The existing tools (`create`, `edit`, `render`, ...) keep working exactly
as today. `edit` and `create` also go through the history from step 1,
so an agent's old-style edit can be undone too.

### 3.2 Undo that a person and an agent share

A person works in the Studio and an agent works through the MCP server.
These are two separate programs, so Undo cannot live in one program's
memory. It lives on disk, in the project folder:

```
Documents/Duck Echo/              the project folder (the MCP root)
  sfx/coin.toml
  music/tune.toml
  .duck-echo/history/
    000041.json   {"title": "Set layer 1 volume to 0.6", "by": "person",
                   "files": [{"path": "sfx/coin.toml",
                              "before": "<text>", "after": "<text>"}]}
    000042.json   {"title": "Add note E5 to lead, bar 3 beat 1", "by": "agent", ...}
    undone/       steps that were undone, newest last, for Redo
```

The rules, in plain words:

1. **A change is never lost.** (**Changed on approval.**) A command
   reads the file, makes its change in memory, and checks the result. It
   writes the file only if the file still holds the exact text it read.
   If the text changed in between (another program saved first), it reads
   the file again and makes its change on the new text, once. If the text
   changed again, it gives up and says so in plain words: "coin.toml kept
   changing while I tried to save. Nothing was changed; try again." The
   other program's change is always kept.
2. **No program ever reads a half-written file.** Every file Duck Echo
   writes (sounds, songs, history steps, WAV files, pictures) is first
   written to a temporary file in the same folder, then renamed into
   place in one go.
3. **The check and the rename happen together.** "Is the text still the
   same?" and "rename" are two actions. Two programs could both check,
   both see the old text, and then both write, losing one change. So a
   command holds a tiny lock while it checks and renames: a file
   `.duck-echo/lock`, created with "fail if it exists" and removed right
   after (a few milliseconds). A program that finds the lock waits a
   moment and tries again. A lock older than 10 seconds is left over from
   a crash and is removed. Slow work (checking the recipe, rendering) is
   done before the lock is taken. The audio thread never sees it.
4. **Undo puts back the "before" text of the newest step**, under the
   same rules: only if the file still holds that step's "after" text.
   The history is one line for the whole project, so the newest step is
   undone first, whoever made it. The History panel shows who made each
   step, so a person can see they are undoing the agent's step.
5. **A slider drag is one step.** Edits with the same "stroke" number
   (one press of the mouse) join the last step, as in Duck Forge.
6. **A change made outside both** (a text editor, a git pull) becomes a
   step called "Changed outside the Studio" the next time either program
   looks at the file. Undo never jumps over it by surprise.
7. **Steps say who made them**: `person` or `agent`. The History panel
   shows "You" or "AI assistant" beside each step, so a person can undo
   an agent's change on purpose, and an agent can undo its own.
8. **The history is trimmed**: the newest 500 steps per project are kept.
   It is in the project folder, so it can be deleted safely and never goes
   to git (`.duck-echo/` is skipped by `list_files`).

How the two programs see each other's changes: the Studio looks at the
files of the open sound or song twice a second, by size and time. When one
changed (a new history step by the agent), it reads it again and redraws.
That is Duck Forge's pattern, and it needs no new library.

The test that proves rules 1 to 3 (**Changed on approval**): the
command-line tool and the MCP server, as separate programs, add many
layers to the same file at the same moment, over and over. At the end,
every layer from both is in the file, the file is valid every time it is
read in between, and the history has one step for each change.

### 3.3 What the person sees

* **Undo** and **Redo** buttons, `Ctrl+Z` and `Ctrl+Y`, with the step's
  title in the tooltip: "Undo: Add note E5 to lead".
* A **History** panel listing the steps, newest first, with "You" or "AI
  assistant" and the time.
* When an agent changes the open sound, a small note appears for three
  seconds: "AI assistant: Set layer 1 cutoff to 1200". Undo is right there.
* **Ctrl+K** opens a search over every command, as in Duck Quill. It is
  made from the same command table, so a new command is in the search the
  moment it exists.

---

## 4. The libraries: what and why

Every new dependency, as CLAUDE.md asks. All of them go only into the
`studio/` workspace or the new `duck-echo-live` crate. The command-line
tool and the MCP server gain **no** new libraries.

| Crate or tool | Version | Used by | Why this one |
|---|---|---|---|
| `eframe` (egui) | 0.37 | studio app | The window and every widget. The same version as both siblings, so the three apps look alike and share know-how. Immediate mode suits sliders that redraw as they move. |
| `egui_kittest` | 0.37 | studio tests only | Draws every screen to a PNG with no window, as Duck Forge does. CI checks screens by their words, and pictures go in the docs. |
| `rfd` | 0.18 | studio app | The system's own "Save As" window, for "Export WAV": Windows' standard one, and on Linux the desktop's own (through the XDG portal, which needs no extra system library). The siblings use 0.17; 0.18 is current, and either works. |
| `winresource` | 0.1 | studio build only | Puts the icon and version into `DuckEchoStudio.exe`, as both siblings do. |
| `windows-sys` | 0.61 | studio app, Windows only | Asks Windows where Documents really is (OneDrive often moves it), as Duck Forge does. |
| `tinyaudio` | 2.0 | studio app | Plays sound through the speakers. The lightest crate that does it; Duck Forge uses it. On Windows it needs nothing else installed. On Linux it links to ALSA in the normal way, with no `unsafe` code of ours, and ALSA reaches PipeWire and PulseAudio too. |
| `rtrb` | 0.4 | duck-echo-live | A lock-free ring buffer with one writer and one reader, made for audio threads. The audio thread can take messages without ever waiting on a lock or allocating. It has no dependencies of its own. Writing our own needs `unsafe` code, which the workspace forbids. |
| `assert_no_alloc` | 1.1 | duck-echo-live tests only | Makes the test fail if the audio code allocates or frees memory. This is the "debug counter" the roadmap asks for. |
| Inno Setup 6 | (a program) | release and CI | Builds `DuckEchoStudioSetup.exe` from a short text script. The same choice and almost the same script as Duck Forge Studio: one person, no administrator rights, a desktop icon, an entry in Windows' Apps list, and upgrades in place. |
| `curl` | (comes with Windows 10 and later, and with nearly every Linux) | studio app | The update check and download, as Duck Forge does: no web library. |
| `sha2` | 0.10 | studio app | Checks the downloaded setup or AppImage against the release's list of SHA-256 fingerprints before it runs, as Duck Quill does. A refused download is deleted. |
| `libasound2-dev` | (a system package) | Linux build only | ALSA's files for building. Installed in CI and listed in the README's Linux build steps. A person running the AppImage needs nothing: every Linux desktop has ALSA. |
| `appimagetool` | (a program) | release and CI | Packs the Linux programs, the icon and the menu file into one `.AppImage`. Downloaded in CI; nothing to install for a person. |

Why not the alternatives:

* **cpal** for sound is bigger and has more options than we need. `tinyaudio`
  gives one callback with float samples, which is exactly our need, and it
  matches Duck Forge.
* **Tauri** or a web page for the window would mean a second language,
  JavaScript, and a different look from the siblings.
* **WiX or MSI** installers need an extra step to be one `.exe` and are
  awkward for per-user installs. Duck Forge Studio already weighed these.

**Linux** (**changed on approval**: the first version only built it).
The Studio works fully on Linux too:

* Sound plays through `tinyaudio` and ALSA.
* It ships as one AppImage with a menu entry and an icon.
* It keeps your files in the XDG Documents folder and its settings in
  the XDG config folder.
* Update checks, Save As and "Connect an AI assistant" work there as on
  Windows.
* The click test and the no-allocation test run on both systems in CI.

---

## 5. The screens

The Studio has three screens and a few panels. Every sketch is roughly
1280 x 800.

### 5.1 Home (task 23)

```
+--------------------------------------------------------------------------+
| [duck] Duck Echo Studio                     Theme v   Help v   Settings  |
+--------------------------------------------------------------------------+
|  ( New sound )   ( New song )          A new version, 0.2.0, is out.     |
|                                        What's new   [ Update ]  Not now  |
|  MY SOUNDS AND SONGS                                     Search: [     ] |
|  +------------+  +------------+  +------------+  +------------+          |
|  | ~~/\/\~~~~ |  | ~/\~~~~~~~ |  | ||| |||| | |  | ~~~~/\~~~~ |          |
|  | coin       |  | hit        |  | day theme  |  | explosion  |          |
|  | 0.31 s  ▶  |  | 0.12 s  ▶  |  | 8 bars  ▶  |  | 1.12 s  ▶  |          |
|  | ... v      |  | ... v      |  | ... v      |  | ... v      |          |
|  +------------+  +------------+  +------------+  +------------+          |
|                                                                          |
|  START FROM   [ Presets ] [ Instruments ] [ Example songs ]              |
|  Pickup  Impact  Weapon  Alert  Movement  UI  Ambient                    |
|  +------------+  +------------+  +------------+  +------------+          |
|  | coin       |  | gem        |  | key_pickup |  | powerup    |          |
|  | ▶ Play     |  | ▶ Play     |  | ▶ Play     |  | ▶ Play     |          |
|  | Make a copy|  | Make a copy|  | Make a copy|  | Make a copy|          |
|  +------------+  +------------+  +------------+  +------------+          |
+--------------------------------------------------------------------------+
```

* **A card** shows the name, the length (or bars for a song), a small
  picture of the sound (its level over time, made by the existing picture
  code and kept as a thumbnail), and **Play**. Clicking the card opens it.
  Its `...` menu has Rename, Duplicate, Export WAV and Delete; each one is
  a command, and Delete goes to the recycle bin and can be undone.
* **The gallery** shows the 30 presets by category, the 9 instruments and
  the example songs. **Play** plays it at once. **Make a copy** runs
  `file_copy_from`, opens the copy, and gives it a fresh name and
  description, so nothing says "Classic coin pickup." by mistake.
* "A new user can find, play and copy a preset in under a minute": the
  gallery is on the first screen, and each card needs one click to play
  and one to copy.

### 5.2 The sound editor (task 11)

```
+--------------------------------------------------------------------------+
| < Home   coin.toml   [Undo] [Redo]   [ ▶ Play ] [ ⟳ Loop ] [■]  History |
+-----------------------------------+--------------------------------------+
| TEXT                              | LAYER 1  square                 [x] |
|  1 version = 0                    |  volume     [=====|-----]  0.80     |
|  2 name = "coin"                  |  duty       [===|-------]  0.50     |
|  3                                |  ENVELOPE                           |
|  4 [master]                       |  attack     [|----------]  0.002 s  |
|  5 normalize_lufs = -19           |  hold       [=|---------]  0.06 s   |
|  6                                |  decay      [===|-------]  0.25 s   |
|  7 [[layers]]                     |  PITCH                              |
|  8 source = "square"              |  start      [ B5      v ]  988 Hz   |
|  9 volme = 0.8   <- unknown field |  arpeggio   steps [0, 5]  0.07 s    |
|    `volme`; did you mean `volume`?|  ( + Add a layer )  ( + Add effect ) |
+-----------------------------------+--------------------------------------+
| PICTURE (level over time and spectrogram, the analyze picture)           |
| ▁▃█▆▅▄▃▂▁                                                                 |
| 0.31 s, ~-19.0 LUFS, bright, sharp attack, tonal, ends cleanly           |
+--------------------------------------------------------------------------+
```

* **The text and the sliders always agree**, because both edit the same
  file through commands. Typing in the text box runs `sound_set_text` a
  moment after you stop typing. Moving a slider runs `sound_set`, and the
  text shows the new value at once.
* **Sliders are made from the schema.** Each number field becomes a
  slider with its range, each enum a drop-down, each note a note picker.
  Today the ranges are written by hand inside `validate.rs`, and the
  schema only knows some of them. Step 3 moves them into one table in
  core that both `validate` and the schema read, so the schema gains
  `minimum` and `maximum` for every field. Agents get them too. This
  changes the schema file but no render.
* **Errors show inline**, under the line they belong to, with the
  suggestion as a button: "Use `volume`".
* **The picture and the summary line** are the same ones agents read,
  updated a moment after each change.
* **Play** plays the sound once. **Loop** plays it over and over, which
  is how you hear a slider move. **■** stops.

### 5.3 The song timeline (task 24)

```
+--------------------------------------------------------------------------+
| < Home  tune.toml  [Undo][Redo]  [▶ Play][■]  140 BPM  4/4  8 bars  Loop ✓ |
+-------------+------------------------------------------------------------+
|             | 1        2        3        4        5        6     7    8  |
|             | [===================== loop =============================]  |
| lead        | E5 G5 C6  B5 G5  D5   A5 C6 E6  F5 A5 C6  ...                |
| chip_lead   | ▭▭ ▭▭ ▭▭▭▭ ▭▭▭ ▭▭ ▭▭▭  ▭ ▭ ▭▭▭  ▭▭ ▭▭ ▭▭▭▭                  |
| vol 1.4 M S |                    ↑ play head                              |
| chords      | [C E G      ] [B D G     ] [A C E     ] [A C F    ] ...      |
| pad         |                                                            |
| bass        | ╔root╗    ╔root+7╗  ╔root-3╗  ╔root+5╗  ╔root╗ ...          |
| bass        |  clips: one pattern placed at a bar                         |
| drums       | ╔beat╗ ╔beat╗ ╔beat╗ ╔beat╗ ╔beat╗ ╔beat╗ ╔beat╗ ╔beat╗       |
| drums       |                                                            |
| ( + Track ) |                                                            |
+-------------+------------------------------------------------------------+
| 8 bars looping at 140 BPM, 4 tracks; -18.0 LUFS, loop joins cleanly,     |
| 155 of 155 notes heard; loudest track lead, quietest drums               |
+--------------------------------------------------------------------------+
```

* **Tracks are lanes.** The header shows the name, instrument, volume,
  pan, **M** (mute) and **S** (solo).
* **Notes are blocks**, placed by bar and beat, with their name on them.
  Inside a lane, higher notes sit higher, like a small piano roll.
* **Patterns are clips**: one framed block per placement, with the
  pattern's name and its transpose. Double-clicking a clip opens the
  pattern's own notes.
* **The play head** moves while it plays. **Loop markers** above the bar
  numbers show and set `loop` and `bars`.
* **The note check is drawn on the notes.** A missing or wrong note is
  outlined in red, with the reason in its tooltip.
* Editing (add, drag, stretch and delete notes, place and move clips) runs
  `song_*` commands. "A song made in the timeline renders the same bytes as
  the same song written as text" holds by design: the timeline only ever
  writes the song file through commands, and the renderer reads that file.

---

## 6. Live playback (tasks 11 and 12)

### 6.1 Two threads, two one-way queues

```
 window / control thread                       audio thread (tinyaudio)
 -----------------------                       ------------------------
 command changes the recipe                    every ~23 ms, fill 1024 frames:
   -> prepare(recipe) -> Voice  ---- to_audio ---->  take messages (never waits)
      (all allocation here)       (rtrb ring)        mix the voices in the pool
   ParamUpdate {slot, what, value} ------------->    smooth changes, clip to -1..1
                                                     write to the speakers
   drop old voices here  <------ to_control -----   send finished voices back
                                  (rtrb ring)
```

* **The voice pool** is a fixed array of 32 slots, made before the audio
  starts. A slot holds a `Voice`, or is empty.
* **A new sound** is prepared on the control thread, where allocating is
  fine. It moves to the audio thread as a message. Moving a value does not
  allocate.
* **A finished or replaced voice is never freed on the audio thread.** It
  is sent back on the second queue and dropped on the control thread.
  Freeing memory can take a lock inside the system allocator.
* **The audio thread clips its own output** to -1..1, since
  `Voice::process` does not.
* **The test that proves it**: `duck-echo-live` runs its mixer in a plain
  test, with `assert_no_alloc` around every block. It plays, changes and
  stops voices for many seconds of sound, and the test fails on any
  allocation or free inside a block.

The mixer lives in `duck-echo-live`, a library with no window and no
speaker. The Studio connects it to `tinyaudio`. So the realtime rules are
tested on Linux in CI, with no sound card at all.

### 6.2 Smooth changes with no clicks (task 12)

Two kinds of change:

* **A number that can change while playing** (a layer's volume, the
  filter cutoff, the master gain). The slider sends a small `ParamUpdate`
  with plain numbers. The voice moves to the new value over 10 ms, a
  little each sample, instead of jumping. A jump in volume is a click; a
  10 ms slide is not heard as one.
* **A change in structure** (a new layer, a different wave, a new
  envelope). This needs a new `Voice`. The new voice starts at the same
  place in the sound, and the old and new voices cross-fade over 10 ms:
  the old one fades out while the new one fades in.

The smoothing is new code in `duck-echo-synth`, used only by live
playback. The offline renderer never sends a `ParamUpdate`, so every
render and fingerprint stays byte for byte the same. A test checks this
on every example and preset.

**The click test** (the roadmap's "recorded loopback test"): a test plays
a looping sound through the mixer, drags the volume from 0.2 to 1.0 and
back in 30 steps, records what comes out, and checks that no step between
two samples is bigger than the music's own biggest step. This is the same
measure as `loop_seam`. Without smoothing the test fails, and it checks
that too.

### 6.3 Songs

`SongPlayer` (from Phase 1) makes a new `Voice` when each note starts, so
it allocates. For Phase 2 the Studio plays a song from a **background
render**:

* Pressing Play renders the song on a worker thread (an 8-bar song takes
  about 2 seconds today) and plays the result as one long voice.
* After an edit, the song is rendered again in the background. The new
  render takes over at the play head with a 10 ms cross-fade, so you hear
  the change within a couple of seconds, without stopping.
* The first render shows a small "Getting the song ready..." under Play.

True live song voices (voices made ahead of the play head, from a pool)
are a Phase 3 job. See question 5.

---

## 7. The four playtest-2 items

| Item from playtest 2 | Where it goes |
|---|---|
| Make a copy straight from a preset | The `file_copy_from` command (step 1): `{"from": "preset:coin", "to": "sfx/my_coin.toml"}`. It is an MCP tool, and the existing `create` tool gains an optional `from` argument too. The copy gets a fresh `name` and a description of "Copy of the coin preset". Home's **Make a copy** runs it. |
| Song summary words | Step 2. A song's summary drops the sound-effect words ("pitch rising from 110 to 784 Hz", the onset count) and says what matters for a mix: `8 bars looping at 140 BPM, 4 tracks; 13.71 s, -18.0 LUFS, no clipping, loop joins cleanly, 155 of 155 notes heard; loudest track lead (-22.1 LUFS), quietest drums (-27.4 LUFS)`. The other fields stay. |
| A click check for one-shot sounds | Step 2. A new analysis field, `ending`: the jump at the very end of the file, the level of the last 5 ms, and `clean` (true when the last sample is near silence or the jump is no bigger than the sound's own steps). The summary says "ends cleanly" or "ENDS WITH A CLICK". This is analysis only, so no render changes. |
| A tool that returns the guide and the specs | Step 2. The MCP tool `get_guide`, with `topic` set to `guide`, `recipe_spec`, `song_spec`, `analysis`, `instruments` or `presets`. It returns the text, built into the program. The server's instructions say to call it first. The Studio's Help menu shows the same pages. |

---

## 8. The slow first start (no building, ever)

Today `.mcp.json` runs `cargo run --release`. The first start of a fresh
copy takes longer than an MCP client waits (about 30 seconds), so the
server fails to connect. This happened in the last two sessions. The
Studio fixes this for people and for agents:

* **The installer ships ready-made programs**: `DuckEchoStudio.exe`,
  `duck-echo.exe` and `duck-echo-mcp.exe`, built by GitHub's Windows
  machine. Nothing is built on the person's computer, and no Rust is
  needed. The installer's optional "add to PATH" box (as in Duck Quill)
  lets terminals and agents find `duck-echo` and `duck-echo-mcp` by name.
* **The AppImage holds the same three programs**, built by GitHub's Linux
  machine. One file runs all three: started plainly it opens the Studio;
  started as `DuckEchoStudio.AppImage mcp --root ...` it is the MCP
  server; started with `cli` it is the command-line tool. A small `AppRun`
  script picks which by the first word. So an agent can always use the
  AppImage, wherever the person keeps it.
* **Settings > Connect an AI assistant** shows, with a **Copy** button,
  the lines that start the installed MCP server with its root set to the
  project folder: `duck-echo-mcp.exe` on Windows, the AppImage with `mcp`
  on Linux. The agent then starts in under a second, and works on the same
  files the person sees.
* **For developers and cloud sessions**, the README gains a short "build
  once first" note. I will also suggest a setup script for this cloud
  environment that builds the release programs before a session starts.
  That is your choice, so it is not part of the build steps.

Where things are kept, as in Duck Forge Studio on Windows, and in the
standard XDG places on Linux:

| What | Windows | Linux |
|---|---|---|
| The program | `%LOCALAPPDATA%\Programs\Duck Echo Studio` (from the installer) | the `.AppImage` file, wherever you put it |
| The app menu entry and icon | Start menu (from the installer) | `~/.local/share/applications/duck-echo-studio.desktop` and `~/.local/share/icons/...`, added on first start (see below) |
| Your sounds and songs | `Documents\Duck Echo` (the real Documents, even when OneDrive moved it) | `Duck Echo` in the XDG Documents folder (`xdg-user-dir DOCUMENTS`, usually `~/Documents`) |
| Undo history | `Documents\Duck Echo\.duck-echo\history` | the same, inside the Linux folder |
| Settings | `%APPDATA%\Duck Echo Studio\settings.json` | `$XDG_CONFIG_HOME/duck-echo-studio/settings.json` (usually `~/.config/...`) |

**The Linux menu entry.** An AppImage has no installer, so the first
time it starts, the Studio asks: "Add Duck Echo Studio to your app
menu?" **Add** writes a standard `.desktop` file and the icon into your
own folders (nothing system-wide), pointing at the AppImage. If you move
the AppImage, the Studio notices at its next start and fixes the entry.
Settings has **Remove from app menu**.

**Updates on Linux.** The same check as on Windows. **Update** downloads
the new AppImage and the checksum list, checks the SHA-256, puts the new
file in place of the old one (a rename, so a failed download leaves the
old one working), and restarts.

---

## 9. Build steps

Each step is one session and one commit, in this order. After each step
with a checklist, I stop and tell you what to try. Each such step has a
"What you test on Windows" and a "What you test on Linux" checklist. CI
on Linux and Windows stays green at every step. From step 4 on, CI also
builds the `setup.exe` and installs it silently on Windows, and builds the
AppImage and smoke-tests it on Linux.

**Steps that need a stronger model** (**changed on approval**) are marked
**[strong model]**. They hold the hard parts: code that two programs run
at once, the realtime audio rules, and the two installers. The other steps
are mostly screens and wiring, built on what the marked steps prove.

**Getting the installer from CI, with no terminal:** open the pull request
on GitHub, click **Checks**, click the newest run, scroll to
**Artifacts**, and download `DuckEchoStudioSetup`. It comes as a zip:
open it and double-click the `.exe`. Windows may say "Windows protected
your PC": click **More info**, then **Run anyway**. The program is not
signed with a paid certificate, as with the siblings.

**Getting the AppImage from CI, with no terminal:** the same page has a
download called `DuckEchoStudio-AppImage`. Open the zip, drag
`DuckEchoStudio.AppImage` into your home folder, right-click it, choose
**Properties**, and turn on **Allow executing file as program** (the
words differ a little between desktops). Then double-click it.

### Step 1: commands, the project folder, shared Undo (task 25) [strong model]

* `Command`, its table, and `run` in `duck-echo-tools`, with the `file`,
  `sound`, `song` and `edit` groups from section 3.1.
* The on-disk history from section 3.2, with strokes and outside
  changes.
* "No change is lost": write only if the text is unchanged, retry once,
  then refuse; the short project lock; every file written to a temporary
  file and renamed into place.
* Every command as an MCP tool with the same name and arguments.
  `duck-echo commands` and `duck-echo run script.json` on the command
  line.
* `create` and `edit` also record history.
* The path guard (`Root`) moves from the MCP server into
  `duck-echo-tools`, so the command-line tool's new `run` and `do` use the
  very same checks. Every new file argument goes through `Root::read` or
  `Root::write`, and nothing may touch `.duck-echo/`.
* Tests: every command, undone and redone, gives back the exact file;
  the CLI and the MCP server add to one file at the same moment many
  times and no change is lost; a change that keeps changing is refused,
  never forced; a stale undo is refused.
* No checklist: there is no window yet.

### Step 2: the playtest-2 items for agents

* `get_guide`, `ending` (the click check), song summary words, and
  `from` on `create`. Tests, docs and the agent guide updated.
* No checklist.

### Step 3: the realtime mixer, smooth changes, slider ranges (tasks 11, 12 without a window) [strong model]

* `duck-echo-live`: the voice pool, the two queues, clipping, the 10 ms
  smoothing and the cross-fade, the background song player.
* Ranges in one table in core; the schema gains `minimum` and `maximum`.
* Tests: no allocation in the audio block (`assert_no_alloc`), the click
  test, and every fingerprint unchanged, on Linux and on Windows.
* No checklist.

### Step 4: the app, sound out loud, the installers, update checks (task 22) [strong model]

* `studio/` workspace, the window with the brand and the theme menu, an
  About box, Settings, the project folder in Documents, the speaker
  (`tinyaudio`), and a first simple list of presets with **Play**.
* `DuckEchoStudio.iss`, the Linux AppImage (`AppRun`, the `.desktop`
  file, the icon, and the first-start "Add to your app menu"), the
  release workflow (a **Make a release** button in the Actions tab, as in
  the siblings, publishing the setup, the AppImage and the SHA-256 list),
  and the update banner on both systems.
* CI: build, test and screenshot the app on Linux and Windows. On
  Windows: build the installer, install it silently, start the program,
  check that it runs, install a newer copy over it, and uninstall it. On
  Linux: build the AppImage, run it with `--version`, run its `mcp` and
  `cli` modes, and play a sound into ALSA's "null" device (no sound card
  needed). On both: the update check against pretend release lists, as
  Duck Forge does.

**What you test on Windows:**

1. Download the installer as described above and double-click it.
2. Click through the installer. It should not ask for an administrator
   password. Tick "Put a Duck Echo Studio icon on the desktop" (it is off
   by default, as in Duck Quill).
3. At the end, leave "Start Duck Echo Studio now" ticked and click
   **Finish**. The Studio opens with no black terminal window.
4. Click **Play** next to `coin`. You hear the coin from your speakers.
5. Close the Studio. Double-click the new desktop icon: it opens again.
6. Open **Theme** and try Dark, Light and Same as Windows.
7. Open **Help > About**: it shows the version number.
8. Open Windows **Settings > Apps > Installed apps**, find Duck Echo
   Studio, and click **Uninstall**. The desktop icon and the program go,
   and your `Documents\Duck Echo` folder stays.

**What you test on Linux:**

1. Download the AppImage as described above and double-click it.
2. The Studio asks "Add Duck Echo Studio to your app menu?". Click
   **Add**.
3. Click **Play** next to `coin`. You hear the coin from your speakers.
4. Close the Studio. Open your desktop's app menu and search for "Duck
   Echo": it is there, with the duck icon. Start it from there.
5. Open **Theme** and try Dark, Light and Same as the system.
6. Open **Help > About**: it shows the version number.
7. Open **Settings** and click **Remove from app menu**. Search the app
   menu again: it is gone. Your `Documents/Duck Echo` folder stays.

### Step 5: Home (task 23)

* Cards with thumbnails, the gallery of presets, instruments and example
  songs, Play everywhere, Make a copy, New sound, New song, Rename,
  Duplicate, Delete (with the recycle bin), Search, Undo and Redo.

**What you test on Windows:**

1. Start the Studio. Home shows the gallery, with the categories along the
   top.
2. Click **Impact**, then **Play** on `explosion`. Then try a few others.
3. Click **Make a copy** on `coin`. The copy appears under "My sounds and
   songs" with a picture and a length. Time this from opening the Studio:
   it should take under a minute.
4. On your copy, open `...` and choose **Rename**. Call it `my coin`.
5. Choose **Delete** on it. It goes. Press **Ctrl+Z**: it comes back.
6. Close the Studio and open your `Documents\Duck Echo` folder in File
   Explorer. Your coin is a `.toml` file there.

**What you test on Linux:**

1. Start the Studio from the app menu. Home shows the gallery.
2. Click **Impact**, then **Play** on `explosion`, and a few others.
3. Click **Make a copy** on `coin`: it appears under "My sounds and
   songs" in under a minute from opening the Studio.
4. Rename it to `my coin`, then **Delete** it, then press **Ctrl+Z**: it
   comes back.
5. Open `Documents/Duck Echo` in your file manager. Your coin is a
   `.toml` file there.

### Step 6: the sound editor (task 11)

* The text with inline errors, sliders made from the schema, the picture,
  the summary line, Play, Loop and Stop, and the History panel.

**What you test on Windows:**

1. Open your coin. The text is on the left and sliders are on the right.
2. Press **Loop**. The coin repeats.
3. Drag the **decay** slider while it repeats. You hear the coin get
   longer or shorter, and the number in the text changes as you drag.
4. In the text, change `0.25` after `decay` to `0.6`. The slider moves to
   match.
5. In the text, misspell `volume` as `volme`. A message appears under
   that line with **Use `volume`**. Click it, and the text is fixed.
6. Press **Ctrl+Z** a few times. Each change goes back, one at a time.
7. Open **History**: every change is listed with "You".

**What you test on Linux:** the same seven steps as on Windows. Also:

8. Open **...** on your coin and choose **Export WAV**. Your desktop's own
   "Save As" window opens. Save it in your Music folder, then play the
   file there with your usual music player.

### Step 7: smooth live changes (task 12)

* `ParamUpdate` from the volume, cutoff and gain sliders; the cross-fade
  for other changes; the recorded click test passes in CI.

**What you test on Windows:**

1. Open the `wind` preset's copy (make one with **Make a copy**) and
   press **Loop**.
2. Drag its **volume** slider quickly up and down. The sound gets louder
   and quieter smoothly, with no clicks or crackles.
3. Drag the filter **cutoff**. The sound gets brighter and darker
   smoothly.
4. Change the wave from `noise` to `saw` in the drop-down. The sound
   changes with no pop.

**What you test on Linux:** the same four steps, with headphones if you
can: clicks are easier to hear there. If you hear crackles even without
touching a slider, open **Help > About** and note what it says under
"Sound output" for me.

### Step 8: the song timeline, to look at and play (task 24, part 1)

* Lanes, note blocks, pattern clips, the play head, loop markers,
  mute and solo, the note check drawn on the notes, and background song
  playback.

**What you test on Windows:**

1. On Home, open **Example songs** and click **Make a copy** on
   `duck_craft_day`. It opens in the timeline.
2. Press **Play**. The music plays, and the play head moves along.
3. Press **S** (solo) on the drums lane: you hear only the drums. Press it
   again to hear everything.
4. The music loops without a click where it starts again.
5. Hover over a note: its name, bar and beat show.

**What you test on Linux:** the same five steps.

### Step 9: editing the song (task 24, part 2)

* Add, drag, stretch and delete notes; place, move and remove pattern
  clips; track volume and pan; add a track; tempo and bars. All through
  commands, with Undo. A test makes a song by timeline commands and checks
  it renders the same bytes as the same song written as text.

**What you test on Windows:**

1. In your copy of the day theme, double-click an empty spot in the
   `lead` lane. A note appears there and you hear it.
2. Drag the note up: it plays the new pitch as you move it.
3. Drag its right edge to make it longer.
4. Press **Play**. Within a couple of seconds you hear your note in the
   music, without stopping.
5. Press **Ctrl+Z** until the note is gone.

**What you test on Linux:** the same five steps.

### Step 10: a person and an agent together (task 25, finished)

* The Studio notices an agent's changes to the open file, shows the
  three-second note, and lists the agent's steps in History. Settings >
  **Connect an AI assistant** with the Copy button. A short playtest
  (playtest 3) where an agent and the Studio edit the same sound.
* The roadmap ticked off, with lessons.

**What you test on Windows:**

1. Open **Settings > Connect an AI assistant** and click **Copy**.
2. Paste it into Claude Desktop's settings, as the window explains, and
   restart Claude Desktop.
3. Open a copy of `coin` in the Studio and press **Loop**.
4. Ask Claude: "Make my coin in Duck Echo a little lower." Within a
   second the coin you hear changes, and a note says "AI assistant: ...".
5. Press **Ctrl+Z** in the Studio. The coin is back as it was, and
   History shows both steps.

**What you test on Linux:** there is no Claude Desktop for Linux, so
this tests the same sharing with a text editor, and the Copy button:

1. Open **Settings > Connect an AI assistant** and click **Copy**. Paste
   it into a text file: it names your AppImage with `mcp`. (If your AI
   assistant app on Linux takes MCP servers, paste it there and do the
   Windows steps 3 to 5.)
2. Open a copy of `coin` in the Studio and press **Loop**.
3. Open the same `.toml` file in your text editor, change the number
   after `decay`, and save. Within a second you hear the change, and
   History shows "Changed outside the Studio".
4. Press **Ctrl+Z** in the Studio. The coin is back as it was.

---

## 10. Risks and open questions

Each has my recommendation. "Yes to all" is a fine answer.

**1. Following both siblings.** *Answered: Duck Quill's installer,
Home and update checks are on its Phase 6 branch. Follow Duck Forge
Studio, and match Duck Quill where the two agree (section 1). Where they
differ, Duck Echo takes Duck Quill's desktop-icon default, its "add to
PATH" box and its SHA-256 check, and Duck Forge Studio's `curl` and its
command type.*

**2. A brand and an icon for Duck Echo.** There is no icon yet.
*Recommendation: a simple placeholder icon in the family style (a duck
with sound waves) in step 4, as Duck Quill did at first (its decision 7).
Replace it when you have art.*

**3. Where projects live.** One folder, `Documents\Duck Echo`, for the
Studio and the MCP server.
*Recommendation: yes. One folder means a person and an agent always see
the same files. Settings can change it later.*

**4. Command names with underscores** (`sound_set`), not dots
(`sound.set`) as in Duck Forge.
*Recommendation: underscores. The Claude API refuses tool names with
dots (they must match `^[a-zA-Z0-9_-]{1,128}$`), and the same name for a
button, a script and a tool matters most. Duck Forge will meet the same
problem when it adds MCP tools.*

**5. Songs play from a background render in Phase 2.** You hear an edit
about two seconds later, without stopping.
*Recommendation: yes. True live song voices are bigger, and they fit with
Phase 3's work for the game engine.*

**6. Linux.** *Answered (changed on approval): the Studio works fully
on Linux. Sound through ALSA linked normally (no `unsafe` code), with
`libasound2-dev` in CI and the README; an AppImage with a menu entry and
icon, built and smoke-tested in CI next to the setup; XDG Documents and
config folders; update checks, Save As and Connect an AI assistant on
Linux; the click and no-allocation tests on both systems; a Linux
checklist for every step that has a Windows one.*

**7. Versions and releases.** The Studio gets its own version, starting
at 0.1.0, and releases tagged `studio-v0.1.0`, as Duck Forge does. The
command-line tool keeps its own version.
*Recommendation: yes.*

**8. Undo on disk, shared by the Studio and agents.**
*Recommendation: yes. It is the only way the two programs can undo each
other's steps, and it survives a crash. It costs one small file per
step.*

**9. The program is not signed.** Windows shows "Windows protected your
PC" the first time, as for the siblings.
*Recommendation: accept it for now. Signing needs a paid certificate.*

**10. Risk: the text box.** egui's text box has no line numbers or error
underlines built in.
*Recommendation: draw both ourselves with egui's layout hook (a few
hundred lines), in step 6.*

**11. Risk: sound on Windows.** Gaps or crackles if the audio thread is
late.
*Recommendation: start where Duck Forge is: it asks `tinyaudio` for 1024
samples at a time (about 23 ms), which feels instant. Add the
no-allocation test, and hear it first in step 4's checklist.*

**12. Risk: CI time.** egui and wgpu (for screenshots) make the first
build of the Studio slow.
*Recommendation: a separate CI job for `studio/` with its own cache, so
the library jobs stay as fast as now.*

**13. Not in Phase 2:** recording a microphone, a mixer of many sounds
at once in a scene, MIDI keyboards, a version for macOS, and code
signing. All stay on the roadmap.

**14. Risk: comments in a file are lost when a command edits it.**
Commands change a file through its data and write it out again, as the
`edit` tool does today, so `#` comments go. Typing in the text box keeps
them, since it saves the text as written.
*Recommendation: accept it for step 1, and in step 6 (the sound editor)
add `toml_edit`, which changes values inside the text and keeps comments
and layout. Justify it in ARCHITECTURE.md then.*

**15. Risk: a Linux desktop with no XDG portal.** "Save As" uses the
desktop portal, which nearly every desktop has.
*Recommendation: if it is missing, save into `Documents/Duck Echo/exports`
and say where. No extra system library.*
