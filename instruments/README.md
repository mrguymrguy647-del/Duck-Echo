# Instruments

The starter instruments a song can play. They are compiled into
`duck-echo` and the MCP server, so a song needs nothing else on disk:

```toml
[[tracks]]
name = "keys"
instrument = "piano"
notes = [{ beat = 0, note = ["C4", "E4", "G4"], length = 2 }]
```

```bash
duck-echo instruments list
```

```bash
duck-echo render instrument:piano -o piano.wav
```

```bash
duck-echo instruments show bass > my_bass.toml
```

Each instrument is a normal recipe. To change one, copy it into your
song's `[instruments.NAME]` table (or a file next to the song) and edit
the copy. A song's own instrument wins over a built-in one with the same
name.

## The instruments

| Name | What it is built from | Written at | Useful range | Loudness alone |
|---|---|---|---|---|
| `piano` | Six sine layers at about 1, 2, 3, 4, 5 and 6 times the note, each quieter and fading faster, plus a 20 ms noise "hammer" and a small reverb. | C4 | C2 to C7 | -18 LUFS |
| `pad` | Three clean saws: one on the note and louder, one 5 cents sharp on the left, one 5 cents flat on the right. Slow attack, long release, a low-pass moved slowly by an LFO, and a reverb. | C4 | C3 to C6 | -22 LUFS |
| `bass` | A clean saw through a low-pass that closes quickly (the pluck), plus a sine on the same note for weight. | C2 | E1 to C4 | -18 LUFS |
| `chip_lead` | A clean pulse wave (25% duty) with light vibrato after 0.2 s and an LFO slowly changing the pulse width. | C5 | C3 to C7 | -18 LUFS |

The drum kit is `drums`. On a track that plays it, a note names a piece:
`note = "kick"`, `"snare"`, `"hat"` or `"clap"`. A piece always plays its
whole sound; `length` and pitch are not used. Each piece is also an
instrument of its own (`instrument = "kick"` plays it at a pitch).

| Piece | What it is built from | Loudness alone |
|---|---|---|
| `kick` | A sine dropping fast from 150 Hz to 45 Hz, plus a short noise click. | -16 LUFS |
| `snare` | Band-passed noise over a short falling triangle tone. | -18 LUFS |
| `hat` | 45 ms of pitched noise at 9 kHz, high-passed. | -24 LUFS |
| `clap` | Three noise bursts 11 ms apart, the last one longer, in a small room. | -20 LUFS |

## How they behave in a song

* **Pitch.** Every layer moves with the note, so an instrument keeps its
  character across its range. Filters do not move, so each instrument
  gets duller towards the top of its range and thinner below it. That is
  what "useful range" means; notes outside it still play.
* **Length.** A note is held for its `length`, then released. The piano
  fades by itself, so a long piano note is not louder than a short one.
  The pad swells in over a third of a second: give it notes of two beats
  or more.
* **Loudness.** Each instrument is levelled once, to the loudness in the
  table, played alone at the note it is written at. With every track at
  `volume = 1` the starter instruments already sit together; use `volume`
  to balance from there, and the song's `master.normalize_lufs` for the
  whole mix.
* **Effects.** The piano's, pad's and clap's reverbs run once per track,
  on the sum of the notes.

## Conventions

Tests in `crates/duck-echo-tools/tests/instruments.rs` enforce these:

* The file name is the recipe's `name`; `description` is one sentence;
  `tags` starts with `instrument`.
* `variation` is 0 and every wave is `clean`, so a note lands exactly on
  its pitch.
* `[master]` has a `normalize_lufs` target, and the recipe reaches it
  without clipping.
* Every pitched instrument plays a C major scale in each octave of its
  range, plus the two ends of the range, and each note reads back within
  10 cents.
* Every kit piece is found at the time it was written.
* Each one is pinned by a fingerprint. Change one only when the sound is
  meant to change, and say so in the commit.

To add an instrument: put the `.toml` here, add it to the `instruments!`
list in `crates/duck-echo-tools/src/instruments.rs` with its range, and
pin it in the test.
