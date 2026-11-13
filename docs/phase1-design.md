# Phase 1 design: music

This is the plan for Phase 1 (roadmap tasks 16, 4, 19, 20 and 21). It was
written before any code and approved with "yes to all" on the questions
in section 8, plus three additions (marked **Added on approval** below).

Where the build changed a detail, a note marked **As built** says so. The
exact reference for what exists is the "Songs" part of
`docs/RECIPE_SPEC.md`.

The short version:

* A **song** is a new kind of text document, next to the recipe. It holds
  a tempo, some tracks, and notes on those tracks.
* Each note plays an **instrument**. An instrument is just a recipe, the
  same kind you already use for sound effects.
* A song renders to the same bytes every time, on Linux and Windows, like
  a recipe does.
* A song can be a **loop** that joins up with itself with no click.
* `analyze` can tell an agent which written notes can really be heard.

Section 8 lists the open questions. Each one has my recommendation, so
you can answer with "yes to all" or pick the ones you want changed.

---

## 1. The song file

### 1.1 A full example: four bars, two tracks

```toml
version = 0
name = "four_bar_demo"
tempo = 110            # beats per minute
bars = 4               # the song is 4 bars long (4 beats in a bar by default)

[master]
normalize_lufs = -16   # level the whole song, same field as in recipes

# A pattern is a small group of notes with a name. Write it once, use it
# in many bars. Beats inside a pattern count from the start of its bar.
[patterns.walk]
notes = [
  { beat = 0, note = "C2" },
  { beat = 1, note = "E2" },
  { beat = 2, note = "G2" },
  { beat = 3, note = "E2" },
]

[[tracks]]
name = "bass"
instrument = "bass"
volume = 0.8
play = [
  { pattern = "walk", bars = [1, 2, 4] },
  { pattern = "walk", bars = [3], transpose = 5 },   # 5 semitones up: F
]

[[tracks]]
name = "lead"
instrument = "chip_lead"
pan = 0.2
notes = [
  { bar = 1, beat = 0, note = "E5" },
  { bar = 1, beat = 1, note = "G5" },
  { bar = 1, beat = 2, note = "C6", length = 2 },
  { bar = 2, beat = 0, note = "B5" },
  { bar = 2, beat = 1, note = "G5" },
  { bar = 2, beat = 2, note = "E5", length = 2 },
  { bar = 3, beat = 0, note = "F5" },
  { bar = 3, beat = 1, note = "A5" },
  { bar = 3, beat = 2, note = "C6" },
  { bar = 3, beat = 3, note = "A5" },
  { bar = 4, beat = 0, note = "G5" },
  { bar = 4, beat = 1, note = "E5" },
  { bar = 4, beat = 2, note = "C5", length = 2, velocity = 0.7 },
]
```

The same song as JSON, the form an agent sends in a tool call. It is the
same schema, field for field:

```json
{
  "version": 0,
  "name": "four_bar_demo",
  "tempo": 110,
  "bars": 4,
  "master": { "normalize_lufs": -16 },
  "patterns": {
    "walk": {
      "notes": [
        { "beat": 0, "note": "C2" },
        { "beat": 1, "note": "E2" },
        { "beat": 2, "note": "G2" },
        { "beat": 3, "note": "E2" }
      ]
    }
  },
  "tracks": [
    {
      "name": "bass",
      "instrument": "bass",
      "volume": 0.8,
      "play": [
        { "pattern": "walk", "bars": [1, 2, 4] },
        { "pattern": "walk", "bars": [3], "transpose": 5 }
      ]
    },
    {
      "name": "lead",
      "instrument": "chip_lead",
      "pan": 0.2,
      "notes": [
        { "bar": 1, "beat": 0, "note": "E5" },
        { "bar": 1, "beat": 1, "note": "G5" },
        { "bar": 1, "beat": 2, "note": "C6", "length": 2 },
        { "bar": 2, "beat": 0, "note": "B5" },
        { "bar": 2, "beat": 1, "note": "G5" },
        { "bar": 2, "beat": 2, "note": "E5", "length": 2 },
        { "bar": 3, "beat": 0, "note": "F5" },
        { "bar": 3, "beat": 1, "note": "A5" },
        { "bar": 3, "beat": 2, "note": "C6" },
        { "bar": 3, "beat": 3, "note": "A5" },
        { "bar": 4, "beat": 0, "note": "G5" },
        { "bar": 4, "beat": 1, "note": "E5" },
        { "bar": 4, "beat": 2, "note": "C5", "length": 2, "velocity": 0.7 }
      ]
    }
  ]
}
```

### 1.2 How Duck Echo tells a song from a recipe

A document with `tracks` at the top is a song. A document with `layers`
is a recipe. A document with both, or with neither, is an error that says
so. There is no extra "kind" field to remember.

Because of this, the same commands work on both: `duck-echo render
song.toml`, `validate`, `analyze` and `compare`, and the same MCP tools.

Recipes do not change at all in task 16. Songs get their own schema file,
`schema/song.schema.json`, printed by `duck-echo schema song`.

### 1.3 Every field

Top of the song:

| Field | Meaning | Default |
|---|---|---|
| `version` | Must be `0`, as for recipes. | required |
| `name`, `description`, `tags` | For people and file names. Same as recipes. | empty |
| `tempo` | Beats per minute, 20 to 400. | required |
| `beats_per_bar` | Beats in one bar, 1 to 16. | `4` |
| `bars` | Length of the song in bars. If left out, the song ends at the bar line after its last note. | computed |
| `loop` | Make a seamless loop (section 5). | `false` |
| `seed` | Picks the random parts (noise, small variations). | `0` |
| `sample_rate` | Samples per second. | `44100` |
| `channels` | `1` or `2`. | `2` |
| `master` | The same `master` table as a recipe: `gain`, `normalize`, `normalize_peak_db`, `normalize_lufs`, `trim_silence`. | no change |
| `effects` | Song-wide effects, the same `[[effects]]` list as a recipe (echo, reverb, ...). They run once on the whole mix. | none |
| `instruments` | Your own instruments, by name (section 1.5). | none |
| `patterns` | Named groups of notes. | none |
| `tracks` | The tracks. At least one. | required |

A track:

| Field | Meaning | Default |
|---|---|---|
| `name` | A short unique name, such as `"bass"`. Used in reports and for the track's seed (section 3). | required |
| `instrument` | Which instrument plays this track (section 1.5). | required |
| `volume` | How loud the track is in the mix, 0 to 2. | `1` |
| `pan` | Left to right, -1 to 1. | `0` |
| `notes` | Notes written straight on the track. | none |
| `play` | Patterns placed at bars. | none |

A track may use `notes`, `play`, or both. The two are simply added
together.

A note:

| Field | Meaning | Default |
|---|---|---|
| `beat` | When the note starts, in beats. `0` is on the bar line, `0.5` is half a beat later. | required |
| `bar` | Which bar `beat` is counted from. Bars count from 1. | `1` |
| `note` | The pitch: a name like `"C5"`, a number in Hz, a list of these for a chord, or a drum name like `"kick"` on a drum track. | required |
| `length` | How long the note is held, in beats. | `1` |
| `velocity` | How hard the note is played, 0 to 1. | `1` |

Leaving out `bar` means "bar 1", so `beat` then counts from the start of
the song. `{ beat = 9, note = "A5" }` and `{ bar = 3, beat = 1, note =
"A5" }` are the same note in a 4-beat bar. Use whichever is easier.

A chord is one entry: `{ beat = 0, note = ["C4", "E4", "G4"], length = 4 }`.

A pattern placement (an entry of `play`):

| Field | Meaning | Default |
|---|---|---|
| `pattern` | The name of a pattern. | required |
| `bars` | The bars it starts at, such as `[1, 2, 4]`. | required |
| `transpose` | Move every note up or down by this many semitones. | `0` |

Inside a pattern, notes have `beat`, `note`, `length` and `velocity`, but
no `bar`. A pattern can be longer than one bar: beat 4 of a pattern is
the first beat of the next bar.

### 1.4 Errors

Song errors look and work like recipe errors: all of them at once, each
with an exact path, the allowed values, and a suggestion.

```json
{
  "code": "invalid_format",
  "path": "tracks[1].notes[3].note",
  "message": "`H5` is not a note name",
  "got": "H5",
  "allowed": "a note name such as C5 or F#3, or a number in Hz",
  "suggestion": "B5"
}
```

Other things validation catches: a pattern name that does not exist
(`tracks[0].play[1].pattern`, with the closest name suggested), a bar past
the end of the song, `bar = 0` ("bars count from 1"), two tracks with the
same name, an instrument that cannot be found, and a note too high or too
low for its instrument (section 2.1).

### 1.5 Instruments

A track's `instrument` is a string. It can be:

* A built-in instrument: `"piano"`, `"pad"`, `"bass"`, `"drums"`,
  `"chip_lead"` (section 4).
* A built-in preset: `"preset:coin"`. Any sound effect can be played as
  notes.
* A recipe file: `"my_lead.toml"`. The path is relative to the folder the
  song file is in.
* A name from the song's own `instruments` table.

The `instruments` table lets a song carry its own sounds, so the song is
one file with nothing else to send along:

```toml
[instruments.blip]          # a full recipe, written inline
version = 0
[[instruments.blip.layers]]
source = "square"
envelope = { attack = 0.005, decay = 0.1, sustain = 0.5, release = 0.05 }
pitch = { start = "C4" }

[[tracks]]
name = "melody"
instrument = "blip"
```

A **drum kit** is an instrument made of named pieces, each a recipe. On a
kit track, `note` is the name of a piece:

```toml
[patterns.beat]
notes = [
  { beat = 0, note = "kick" },
  { beat = 1, note = "snare" },
  { beat = 2, note = "kick" },
  { beat = 3, note = "snare" },
  { beat = 0.5, note = "hat", velocity = 0.5 },
  { beat = 1.5, note = "hat", velocity = 0.5 },
]

[[tracks]]
name = "drums"
instrument = "drums"
play = [{ pattern = "beat", bars = [1, 2, 3, 4] }]
```

You can build your own kit in its own table: `[kits.mykit]` with
`kick = "preset:thud"`, `snare = "mysnare"` and so on. Each piece names an
instrument: a preset, a recipe file, or a name from `instruments`.

**As built:** the first plan put kits inside `instruments`
(`[instruments.mykit.kit]`). They moved to a top-level `kits` table,
because two kinds of table under one name cannot be told apart cleanly
when checking the file, and one string per piece is shorter to write.

---

## 2. How a note plays an instrument

For every note, Duck Echo makes one `Voice` from the instrument recipe.
`Voice` is the same renderer that plays sound effects today. Four things
are changed for the note: pitch, length, loudness and seed.

### 2.1 Pitch

Every instrument has a **home note**: the pitch its recipe plays as
written. The home note is the `pitch.start` of the first layer that is
not plain noise. The built-in piano is written at C4, so its home note is
C4.

To play another note, every layer's pitch is multiplied by the same
number: `note / home`. So if the piano is asked for C5, every layer plays
twice as high. This keeps the layers in step with each other. A piano
made of layers at 1, 2 and 3 times the base pitch stays a piano at every
note.

Pitch sweeps, vibrato and arpeggios in the instrument still work. They
are all written relative to the pitch, so they move with it.

Two rules for the edges:

* A layer that would go above 20 kHz is left out of that note. Nobody can
  hear it, and it would fold back down as a false tone.
* If the first pitched layer itself would leave the allowed range (1 Hz
  to 20 kHz), that is a validation error on the note, with the range of
  notes this instrument can play.

Filters do **not** move with the note in Phase 1. A low-pass at 2 kHz
stays at 2 kHz for every note. See open question 8.

### 2.2 Length

Today an envelope has fixed stages: attack, hold, decay, sustain time,
release. A sound effect plays them from start to end.

A note works like a key on a keyboard:

1. While the note is held, the envelope plays as written. If it reaches
   the sustain level, it stays there for as long as the note is held. (The
   recipe's own `sustain_time` is not used for notes. The note's length
   decides.)
2. When the note ends, the envelope falls to silence over `release`,
   starting **from whatever level it is at**.
3. If the envelope already faded to silence by itself, nothing more
   happens. A short pluck is a short pluck however long the note is.

Rule 2 matters. A piano note decays slowly for more than a second. A
short piano note must stop when the key is let go, not ring for its full
decay. So the renderer needs one small new thing: an envelope can be told
"let go at this sample". When no one tells it, it behaves exactly as
today, so every existing render stays the same (the golden tests prove
this).

Drum pieces are different: a drum hit always plays its whole recipe, and
`length` is not used. You cannot hold a snare.

### 2.3 Velocity

`velocity` multiplies the note's volume. `1` plays the instrument as
written, `0.5` is half as loud (6 dB quieter). It does not change the
tone in Phase 1.

### 2.4 Notes overlap freely

Each note has its own `Voice`, so a new note never cuts off the last one.
A release tail rings on under the next note. Chords are just notes that
start together.

Each note starts on its exact sample. The start is worked out with plain
arithmetic on 64-bit numbers (`beats * 60 * sample_rate / tempo`, then
rounded), which gives the same answer on every computer.

### 2.5 Why instrument effects run once per track

An instrument recipe can have a top-level `[[effects]]` list, for example
a reverb. In a song, these effects run **once for the whole track**, on
the sum of all its notes. They do not run once per note.

Three reasons:

* **It sounds right.** One reverb per track is one room with one player
  in it. An echo keeps repeating across the next notes. A distortion sees
  the whole chord at once, like a real amp does.
* **Tails join up.** With one reverb per note, every note would start a
  new, separate tail, and a fast melody would turn to mud.
* **It is much cheaper.** A melody of 200 notes would otherwise run 200
  reverbs.

Effects on a single layer (`layers[].effects`) are part of the tone of
the sound, so they stay per note.

The whole signal path, in order:

```
note voices -> summed -> instrument effects -> track volume and pan
                                                   |
all tracks summed -> song effects -> (loop wrap) -> song master -> file
```

### 2.6 What a song uses from an instrument recipe

| Part of the recipe | In a song |
|---|---|
| `layers` | Played for every note. |
| `effects` | Run once per track. |
| `seed` | Mixed into every note's seed (section 3). |
| `variation` | Works per note, so each note differs a little. A cheap way to sound less like a machine. |
| `master.gain` and normalize settings | Used once, to set the instrument's level (see below). |
| `sample_rate`, `channels`, `duration`, `master.trim_silence` | Not used. The song decides the rate, the channels and the length. |

**Levelling.** Normalizing every note on its own would make every note
equally loud and undo `velocity`. So an instrument is levelled once: Duck
Echo works out the gain the recipe's own `master` would apply if the
recipe were rendered alone, and uses that one fixed gain for the whole
track. In plain words: an instrument is as loud in a song as it is on its
own. This is what makes the built-in loudness targets carry over into
songs.

---

## 3. Seeds

The rule to keep: **the same song gives the same file, and changing one
thing only changes that thing.**

Recipes already do this. Each layer gets its own stream of random numbers,
forked from the recipe seed, so muting a layer does not change the
others. Songs go two levels further.

```
song seed
  -> one stream per track, picked by the track's NAME
       -> one stream per note, picked by the note's START SAMPLE
            (and, for notes starting together, its place in pitch order)
            -> this number, mixed with the instrument's own seed,
               becomes the seed of that note's Voice
```

What this buys:

* **Adding a track never changes the others.** A track's stream depends
  on its name, not on its place in the file. You can add, remove or
  reorder tracks freely. (Renaming a track changes that one track's
  random parts, and nothing else.)
* **Changing a note never changes the others.** A note's stream depends
  on where it starts, not on how many notes come before it. Insert a note
  in bar 2 and bar 3 still renders the same samples.
* **Every drum hit is a little different.** Two hi-hats at different
  times get different noise, as real ones do. The same hi-hat at the same
  time is always the same.
* **A solo track is exactly the track in the mix**, as with layers today.
  This is what makes per-track analysis honest.

Track names are turned into numbers with a small fixed hash (FNV-1a,
already used for the fingerprints). No hash maps are walked on the audio
path. Patterns and instruments are stored in sorted maps (`BTreeMap`), so
their order is always the same.

Tracks are added into the mix in file order and notes in time order, so
the sums come out the same on every computer.

---

## 4. Starter instruments

Five instruments live in a new `instruments/` folder. They are compiled
into the program, like the presets. Each is a normal recipe (the drum kit
is four), each has tags and a loudness target, and each is pinned by a
fingerprint test. `duck-echo instruments list` and an MCP
`list_instruments` tool show them. `instrument:piano` names one anywhere a
recipe is accepted, the same way `preset:coin` does, so you can render or
analyze an instrument by itself.

| Instrument | Built from | Useful range |
|---|---|---|
| `piano` | Six sine layers at 1, 2, 3, 4, 5 and 6 times the note. Each higher one is quieter and fades faster. A 15 ms burst of filtered noise is the hammer. | C2 to C7 |
| `pad` | Three clean saws: one on the note, one a little sharp and panned left, one a little flat and panned right. Slow attack, long release, a low-pass filter that an LFO moves slowly, and a reverb. | C3 to C6 |
| `bass` | A clean saw through a low-pass that closes quickly (the pluck), plus a sine on the same note for weight. | E1 to C4 |
| `drums` | A kit. `kick`: a sine that drops fast from about 150 Hz to 45 Hz, with a click. `snare`: filtered noise plus a short triangle body. `hat`: very short high-passed pitched noise. `clap`: three noise bursts 10 ms apart, with a small reverb. | not pitched |
| `chip_lead` | A clean square wave with a narrow pulse, a little vibrato, and an LFO slowly changing the pulse width. | C3 to C7 |

The pad, the hat and the chip lead need task 4 (LFOs and pitched noise).
That is why task 4 comes before task 19.

### 4.1 Does the piano need a new sound source?

**No, not in Phase 1.** I tested it before writing this.

The prototype is the recipe in the table: six sine layers, each tuned a
tiny bit sharp of an exact multiple the way real piano strings are, plus
the noise hammer. I rendered it at C2, C4 and C6 and read the notes back
with `analyze`:

| Note | Wanted | Read back | Off by |
|---|---|---|---|
| C2 | 65.41 Hz | 65.44 Hz | 0.9 cents |
| C4 | 261.63 Hz | 261.66 Hz | 0.2 cents |
| C6 | 1046.50 Hz | 1046.72 Hz | 0.4 cents |

That is well inside the 10 cent limit. It works with the sources we have
because of the rule in 2.1: every layer moves by the same ratio.

To be honest about the sound: it will be closer to an electric piano or a
music box than to a grand piano. It is clearly "a piano-like keyboard",
which is what a retro game needs.

If a later playtest says it is not good enough, the smallest new source
to add is a **partials list**: one layer that is a sum of sines, each
with its own ratio, volume and fade time. I would pick that over FM
because:

* It is the same sound we already have, in one layer instead of seven,
  so it is cheaper to render and easier to write.
* Its pitch is easy to predict, so `analyze` reads it back correctly. FM
  makes side tones that can fool a pitch reader, and they can fold back
  at high notes.
* It needs only the sine we already have in `synth::math`. No new math.

I recommend **not** adding it now. It goes on the roadmap as a possible
follow-up.

### 4.2 Task 4 in short

Both parts are new optional fields on a layer. With the defaults, nothing
changes, so no existing render changes.

* `noise_mode = "white" | "pitched"` (default `white`). Pitched noise
  picks a new random value `pitch` times per second and holds it in
  between. Low pitch gives a rough rumble, high pitch gives hiss. This is
  how old game consoles made drums. It follows the layer's pitch, so it
  follows the note in a song.
* `[layers.lfo]`: a slow wave that moves one setting. `target` is
  `volume`, `cutoff` or `duty`. `rate` is in Hz. `shape` is `sine`,
  `triangle` or `square`. `delay` is the time in seconds before it starts.
  `depth` says how far it moves: for `volume` a part from 0 to 1, for
  `cutoff` a number of octaves, for `duty` an amount added and taken away.
  One LFO per layer. It starts again with each note.

The new example for task 4 is a siren whose filter is moved by an LFO. I
will put it in `examples/recipes/siren.toml`, next to the other examples,
so the existing fingerprint test and the CI script pick it up.

---

## 5. Seamless loops

### 5.1 How it works

A song with `loop = true` renders to **exactly** its length in bars, to
the sample. Four bars at 120 BPM and 44.1 kHz is exactly 352,800 samples.

The trouble is the end. When the last bar stops, things are still
ringing: a reverb, an echo, the release of the last note. Cutting them
off makes a click and a dry hole at the start of the next round.

So Duck Echo does this:

1. Render the song the long way, until everything has gone quiet. Call
   the part after the last bar line the **tail**.
2. Cut the tail off.
3. Add the tail onto the **start** of the song.

Why this is right: think of the loop playing for the tenth time. At its
start, the tail of the ninth time is still ringing under it. The wrapped
file is exactly that. It is one turn of a song that has been playing
forever.

Details:

* The wrap happens after the song effects and before the song master. So
  loudness is measured on the finished loop, and the peak limit applies
  to it.
* A note that is held across the end wraps too. A pad chord can start in
  the last bar and carry over the join.
* If the tail is longer than the whole loop, it wraps round more than
  once.
* `loop = true` together with `master.trim_silence = true` is a
  validation error, because trimming would change the length.

### 5.2 How we prove there is no click

Four tests, from simple to strict:

1. **Exact length.** The file has exactly `bars * beats_per_bar * 60 /
   tempo * sample_rate` samples.
2. **No jump at the join.** Play the loop twice in a row. The step from
   the last sample to the first is no bigger than the biggest step between
   two neighbouring samples anywhere else in the file. A click is a jump,
   so no jump means no click.
3. **No false note at the join.** `analyze` the loop played twice. A test
   song with a long reverb and nothing written on beat 0 must show no
   onset at the join.
4. **It really is "one turn of forever".** Write the song out twice in a
   row as one long song, render it without looping, and take the second
   half. It must match the looped render sample for sample (within a tiny
   rounding allowance). This is the strict one. It proves the wrapped tail
   is the true tail and not just something smooth.

And one check for the agent, every time: `analyze` on a looped song
reports the join, for example
`"loop": { "seam_step": 0.004, "largest_step": 0.21, "clean": true }`.

---

## 6. Checking a song without ears

The question an agent needs answered is: **"I wrote these notes. Can each
one really be heard, at the pitch I wrote?"**

A note can go missing in ways the song text does not show. It can be too
high or low for its instrument's filter. It can be buried under the tail
of the note before it. It can be too short to sound. An instrument can
have a low layer that is louder than the note itself, so the ear hears
the wrong octave.

### 6.1 How it is measured

`analyze` on a song (or `per_track: true` in the MCP tool) renders each
track alone. This is exact, because a solo track is the same samples as
that track in the mix (section 3).

Then, for every written note, it listens at the place the note should be:

* It looks at the stretch of sound while the note is held.
* It finds the strongest pitch there.
* It compares that with the written pitch, in cents.

This is better than only looking for onsets (sudden starts). A pad with a
slow attack has no sudden start, but its notes are still there. Onsets
are used for the other direction: a clear onset where no note was written
is reported as `extra`.

Drum hits are checked by time only: is there a hit within a few
milliseconds of where it was written?

### 6.2 What the report looks like

```json
{
  "summary": "8 bars at 96 BPM, 20.0 s, -16.1 LUFS, 3 tracks, 61 of 62 notes heard",
  "tempo": 96,
  "bars": 8,
  "tracks": [
    {
      "name": "lead",
      "instrument": "chip_lead",
      "notes_written": 30,
      "notes_heard": 29,
      "problems": [
        {
          "problem": "wrong_pitch",
          "path": "tracks[1].notes[12]",
          "bar": 4, "beat": 2.0,
          "written": "A5", "written_hz": 880.0,
          "heard": "A4", "heard_hz": 440.3,
          "cents_off": -1198.8,
          "level_dbfs": -19.2
        }
      ]
    }
  ]
}
```

`problem` is one of `missing` (nothing there), `quiet` (far below the
rest of the track), `wrong_pitch` (more than half a semitone off) or
`extra`. Very short low notes, where pitch cannot be read well, are
marked `uncertain` instead of wrong.

Only problems are listed by default, so the answer stays short. An option
lists every note with what was heard.

The picture (`picture: true`) gets thin lines at every bar, so the
spectrogram can be read against the song.

The roadmap test ("a song with one wrong note is reported with its track,
beat and both pitches") is done by checking one song's audio against a
copy of the song with one note changed.

**As built:** the report is its own block, `note_check`, next to
`tracks`, so it is there without asking for the full per-track analysis.
`analyze` always includes it; `render` includes it with `per_track` or
`notes`. Extra onsets are listed separately as `extra_onsets` and are
information, not problems. `docs/ANALYSIS.md` ("Songs") is the
reference.

---

## 7. Fitting the future: Studio and Duck Forge

### 7.1 Duck Echo Studio (a timeline you edit)

The song file already **is** a timeline:

| In the Studio | In the song file |
|---|---|
| A lane | A track |
| A block on a lane | A note, with `beat` and `length` |
| A reusable clip | A pattern, placed with `play` |
| Dragging a block | Changing `beat` or `bar` |
| The play head in bars and beats | The same bars and beats |

Choices made now so the Studio does not need a redesign:

* **Positions are in beats, not seconds.** Change the tempo and the music
  stays in time.
* **Edits are small and local.** The existing `edit` tool changes one
  field by its path, such as `tracks[1].notes[3].note`. A Studio edit is
  the same operation.
* **Changing one note changes only that note's sound** (section 3). The
  Studio can re-render just the track that was edited and keep the rest.
  A track render depends only on the track, the tempo and the seed.
* **The song player works block by block**, a few hundred samples at a
  time, the same way `Voice` does. That is the shape live playback needs.
  See risk 10 for what is still missing for it.
* **Solo and mute per track** exist from the start, as options on
  `render` and `analyze`.
* **Stems add up to the mix. (Added on approval.)** A solo or per-track
  render uses the full song's master gain, the same rule as `per_layer`.
  So a stem lines up with the mix in both time and level, and all the
  stems added together give the mix. A test checks this.
* **Songs have `version = 0`** like recipes, so the format can grow with
  the same rules.

Things the Studio will want later, and where they will go. None of them
are in Phase 1. All are listed in the spec under Deferred:

* Tempo that changes during a song.
* Volume or filter changes over time on a track (automation).
* Swing and small timing shifts.
* Sample (recorded audio) tracks.

### 7.2 Duck Forge and Duck Craft (day and night music)

What a game needs from looping music, and how the design covers it:

* **Loops with no seam.** Section 5.
* **Day and night that can cross-fade.** If both songs have the same
  tempo and the same number of bars, they are the same length to the
  sample. The game can start both together and fade between them at any
  moment, and they stay in time. The two demo tracks will be made this
  way: same tempo, 8 bars each, in keys that fit each other (C major for
  day, A minor for night).
* **Layers the game can switch on and off.** Rendering one track alone
  (`--solo drums`) gives stems that line up exactly, for example drums
  that only play at night.
* **Facts for the engine.** `render --json` reports `tempo`, `bars`,
  `beats_per_bar`, `loop` and the exact length in samples, so an engine
  can switch music on a bar line.
* **Small, plain files.** 44.1 kHz stereo WAV.

**Added on approval:** Duck Forge already reads 44.1 kHz WAV, so songs
stay WAV. One thing to know: the engine currently mixes stereo down to
mono. So a track's `pan` is lost inside a game until the Phase 3 engine
fixes. The files themselves are true stereo and will sound right as soon
as the engine plays both channels. Until then, do not rely on pan to keep
two parts apart; use different pitch ranges and instruments.

---

## 8. Risks and open questions

Each has my recommendation. "Yes to all" is a fine answer.

**1. Do bars count from 1 and beats from 0?**
Bars are names, like page numbers, and musicians say "bar 1". Beats are
distances from the bar line, like `offset` in a recipe. `bar = 0` is an
error with a clear message, so the mix-up is caught.
*Recommendation: yes.*

**2. Must every track have a name?**
The name makes reports readable ("lead, bar 4") and gives the track a
seed that does not move when tracks are reordered. The cost is one short
line per track.
*Recommendation: yes, required and unique.*

**3. Is a drum kit one instrument with named pieces (`note = "kick"`)?**
The other way is one track per drum. One track is far shorter to write
and reads like a drum pattern.
*Recommendation: yes, one kit instrument.*

**4. Are chords written as a list (`note = ["C4", "E4", "G4"]`)?**
Pads are nearly all chords. Without this every chord is three or four
lines.
*Recommendation: yes.*

**5. Is the layered piano good enough?**
It passes the pitch test (section 4.1) but will sound like an electric
piano.
*Recommendation: ship it, say so plainly, and add the partials source
only if a playtest asks for it.*

**6. Should each note get its own random stream?**
It makes drum hits differ slightly, and makes edits local. The other
choice is every note of an instrument sounding exactly the same.
*Recommendation: yes, per note.*

**7. Should an instrument be levelled once, from its own `master`?**
Section 2.6. Without it, a preset used as an instrument has whatever raw
level its layers happen to have.
*Recommendation: yes.*

**8. Should filters follow the note?**
Real instruments get brighter as they go up. In Phase 1 a filter stays
where the recipe put it, so an instrument sounds duller at the top of its
range and each one has a "useful range". Doing this well is a design of
its own.
*Recommendation: not now. List it under Deferred.*

**9. Risk: speed.**
One 1.7 second piano note (7 layers) takes about 0.13 seconds to render
in a release build. An 8-bar song with a few tracks will take a few
seconds. Tests run in debug builds, which are much slower.
*Recommendation: build only the synth crate with optimization in test
builds. This cannot change the bytes, because Rust never reorders
floating point math. Look for speed-ups only if a 30 second song takes
more than about 10 seconds.*

**10. Risk: live playback.**
`Voice::process` stays allocation-free, as the rules demand. But the song
player makes a new `Voice` when each note starts, which allocates memory.
That is fine for rendering to a file, and not fine for live audio.
*Recommendation: accept it for Phase 1. Phase 2 adds a pool of voices
made ahead of time. The player's shape (a sorted list of notes, worked
through block by block) does not change.*

**11. Risk: loops and distortion.**
Wrapping the tail is exact for reverb and echo. With distortion or
bitcrush on a whole track or the whole song, the wrapped file is still
smooth at the join, but it is not sample-for-sample what a forever-playing
song would give.
*Recommendation: accept and document it. Test 4 in section 5.2 uses
reverb and echo only.*

**12. What audio format does Duck Forge want?**
*Answered: Duck Forge reads 44.1 kHz WAV, so keep WAV. The engine mixes
stereo to mono for now, so pan is lost in games until Phase 3 (see 7.2).*

**13. Should a loop also come with an intro?**
The first time a loop plays, it starts with the tail of a round that
never happened. In practice this is a soft wash at the start and is
normal for game loops. A separate "first time" file without it is
possible later.
*Recommendation: not now.*

**14. Instrument files and the MCP root.**
A song can name a recipe file as an instrument. That is a new way to
read a file, so in the MCP server it goes through `Root::read` like every
other path: no absolute paths, no `..`, no links leading out. The path is
relative to the song file's folder in both the CLI and the MCP server.
*Recommendation: yes, as described.*

**Added on approval:** `instrument:NAME` (a built-in instrument given
where a recipe path is expected) goes through the same known-name check
as `preset:` in `Root::read`: it is read-only, only a known name is
accepted, and it is never treated as a path.

**15. Limits.**
A song sent to the MCP server should not be able to run for an hour.
*Recommendation: at most 10 minutes, 20,000 notes and 64 tracks, each
with a clear error.*

**16. Not in Phase 1.**
Tempo changes, swing, automation, sample tracks, filters that follow the
note, velocity changing the tone, and voices. All go in `RECIPE_SPEC.md`
under Deferred, so nothing is silently missing.

---

## 9. Build order

One commit per step. Every step ends with tests, clippy and fmt clean,
the schema and spec updated, and the roadmap ticked off.

1. **Task 16: songs.** Song types and validation in `duck-echo-core`. The
   note-playing change to the envelope and the song player in
   `duck-echo-synth`. Loading, rendering and measuring in
   `duck-echo-tools`, so the CLI and the MCP server share it. A pinned
   4-bar, two-track demo in `examples/songs/`, using instruments written
   inside the song (the built-in ones come in step 3). The CI script
   renders songs too, so Linux and Windows are compared.
2. **Task 4: pitched noise and LFOs**, with `siren.toml`.
3. **Task 19: the five instruments**, each pinned, each scale read back
   within 10 cents, each drum hit found at its time.
4. **Task 20: loops**, with the four tests from section 5.2.
5. **Task 21: notes written against notes heard**, and bar lines in the
   picture.
6. **Duck Craft demo:** `duck_craft_day.toml` and `duck_craft_night.toml`,
   8 bars each, looping, same tempo, 44.1 kHz stereo, pinned.

Task 10 (the agent guide) is left for the next session, as you asked.

Promises kept the whole way:

* No existing example or preset render changes. The golden tests stay as
  they are.
* The recipe schema only grows in step 2, and only by optional fields.
* No new dependencies.
* Nothing on the audio path uses the platform's math library.
