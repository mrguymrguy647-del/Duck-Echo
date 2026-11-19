# Duck Ear check: the score model

This page records how well the Duck Ear's score model hears Duck Echo's
own sounds. It covers step 3 of `docs/phase4-design.md`: two models were
converted, both were measured on the same test set, the better one was
chosen, and its yes and no lines were set. Step 4 adds the describe
model's results here.

**Chosen: LAION-CLAP (`laion/larger_clap_general`).** It tells which of
two sounds a text fits much more often than CLAP2-S (98% against 88%),
and it is level with CLAP2-S on everything else. CLAP2-S is twice as
fast and half the size, so it stays in the converter as the second
choice.

## How to run it again

```bash
cd ear && uv sync && cd ..
duck-echo ear check --ear-program ear/.venv/bin/duck-ear
```

(On Windows the program is `ear\.venv\Scripts\duck-ear.exe`.)
`ear check` renders every sound in `ear/testset/testset.toml`, asks the
Ear to score every text, and writes `ear-check/ear-check.json` and
`ear-check/ear-check.md`. The model is found as the Ear always finds it
(`DUCK_EAR_SCORE_MODEL`, else `<the Ear's folder>/models/score`); making
it is in `ear/convert/README.md`.

## Where these numbers come from

| | |
|---|---|
| PC | Linux, Intel Xeon at 2.1 GHz, 4 cores, no graphics card |
| The Ear | duck-ear 0.3.0, Python 3.13, ONNX Runtime 1.32.0, numpy 2.5.5, tokenizers 0.23.3 |
| The converter | PyTorch 2.15.0 (CPU), open_clip 4.2.2, transformers 5.23.0 |
| Test set | `ear/testset/testset.toml`: 41 sounds, 36 different texts (47 true and 137 wrong pairs of a sound and a text) |
| Reference sounds | the 43 renders of Duck Echo's 30 presets, 10 example recipes and 3 example songs |

## The conversion

Both models were converted to ONNX by `ear/convert/convert_clap.py`, at
pinned revisions, and checked against the originals: the Ear's whole path
(its numpy front end, then ONNX Runtime) against the original's (its
PyTorch front end, then PyTorch). Each embedding must point the same way
as the original's: closeness (cosine) 0.999 or more, on every sound and
every text, or the conversion fails.

| Model | Revision | Licence | Size | Lowest closeness, sounds (of 43) | Lowest closeness, texts (of 7) |
|---|---|---|---|---|---|
| CLAP2-S (`laion/clap2-small-audioxl-s1B`) | `56bdd3677553` | Apache-2.0 | 421 MB | 0.999999 | 1.000000 |
| LAION-CLAP (`laion/larger_clap_general`) | `ada0c23a36c4` | Apache-2.0 | 784 MB | 0.999664 (the chime) | 1.000000 |

Both are well above the bar. LAION-CLAP is a little further from its
original (its lowest is the chime), but the difference is far below what
changes an answer.

## Scores you can read

The raw closeness of a sound and a text means little by itself: both
models give every sound much the same closeness to "music", for
example. So the Ear compares: each text's closeness to the sound is
turned into a **z-score**, how many spreads it is above that text's
usual closeness to Duck Echo's 43 reference sounds. Then

    match = 1 / (1 + e^(-2 (z - center)))

so `match` is 0 to 1, and 0.5 halfway between the yes and no lines. A
silent sound (peak below -60 dBFS) is not given to the model: it matches
"silence" and nothing else. The reference sounds, the center and the
lines are saved with the model (`reference.npy`, `model.json`), and the
model's revision in `hello` includes the word list's version and a hash
of them, so a score today means the same tomorrow, and a new calibration
misses the answer cache.

## The yes and no lines

`ear check` suggests the lines that give the most "yes" on true texts
and "no" on wrong texts together, while each kind of mistake (a true
text said no, a wrong text said yes) stays at most 10%. The lines below
are those, for each model, and are what the Ear reports in `hello`:

| Model | Yes from | No below | Center (z) |
|---|---|---|---|
| LAION-CLAP | 0.662 | 0.338 | 0.645 |
| CLAP2-S | 0.584 | 0.416 | 0.400 |

Between the lines the answer is `unsure`.

## The results

| Measure | What it means | Bar | LAION-CLAP | CLAP2-S |
|---|---|---|---|---|
| Kind of sound | every true text scores above every wrong one | 90% | 36 of 41 (88%) | 36 of 41 (88%) |
| Pairs | for one text, the right sound scores above the wrong one | 90% | **209 of 214 (98%)** | 189 of 214 (88%) |
| Soft and harsh | soft and harsh versions told apart | 75% | 3 of 3 | 3 of 3 |
| Calm and tense | calm and tense music told apart, both ways | 80% | 3 of 4 (75%) | 3 of 4 (75%) |
| Yes on true texts | a true text gets yes | 80% | 36 of 47 (77%) | 37 of 47 (79%) |
| No on wrong texts | a wrong text gets no | 80% | 103 of 137 (75%) | 107 of 137 (78%) |
| Silence | silence matches silence, and nothing else | 100% | 1 of 1 | 1 of 1 |
| Time | seconds per sound, score only, on the CPU | recorded | 0.26 s | 0.13 s |
| Loading | from disk, and memory in use after | recorded | 4.0 s, 1.1 GB | 2.2 s, 0.6 GB |

The mistakes, at these lines:

| | LAION-CLAP | CLAP2-S |
|---|---|---|
| A true text said no | 4 of 47 (9%) | 6 of 47 (13%) |
| A wrong text said yes | 13 of 137 (9%) | 13 of 137 (9%) |

(CLAP2-S's lines were rounded after `ear check` suggested them, which
pushed its "true text said no" over 10%.)

### What each model got wrong

"Kind of sound" misses: the sound, its true texts, and the wrong texts
that scored at least as high (each with its `match`).

LAION-CLAP:

| Sound | True text (match) | Wrong text at least as high (match) |
|---|---|---|
| `preset:coin` | a coin pickup (0.74), a short bright game pickup sound (0.36) | wind blowing (0.65), an engine hum (0.56) |
| `preset:footstep` | a footstep (0.12) | an explosion (0.19) |
| `examples/recipes/jump.toml` | a jump in a platform game (0.62) | footsteps (0.70) |
| `preset:warning_beep` | warning beeps (0.84) | footsteps (0.85) |
| `tense_loop.toml` | fast exciting music (0.56), tense music (0.73) | calm music (0.57) |

CLAP2-S:

| Sound | True text (match) | Wrong text at least as high (match) |
|---|---|---|
| `examples/recipes/coin.toml` | a coin pickup (0.62), a short bright game pickup sound (0.80) | calm music (0.76) |
| `preset:metal_clang` | a metal clang (0.62) | calm music (0.73) |
| `preset:ui_click` | a menu click (0.61) | footsteps (0.76) |
| `calm_loop.toml` | calm music (0.14), slow gentle music (0.99) | tense music (0.22) |
| `tense_loop.toml` | fast exciting music (0.41), tense music (0.02) | slow gentle music (0.02) |

## Why LAION-CLAP

* **Pairs, 98% against 88%.** This is the question an agent asks most
  when it changes a sound: "is the new one more like *a soft coin
  pickup* than the old one?" LAION-CLAP got 5 of 214 wrong; CLAP2-S got
  25 wrong.
* Everything else is level: the same kind-of-sound score (36 of 41), the
  same soft-and-harsh and calm-and-tense results, and yes and no within
  a few points of each other.
* CLAP2-S's misses are odder: it hears "calm music" in a coin and a metal
  clang, and it says "tense music" fits a calm loop better than "calm
  music" does.
* What it costs: LAION-CLAP takes twice as long (0.26 s a sound on 4
  CPU cores, still far inside the 15 s limit) and needs about twice the
  disk and memory (784 MB, 1.1 GB in use). Both load in a few seconds,
  inside the 60 s limit.

The design guessed CLAP2-S would win, from its authors' numbers on
broad tests. On Duck Echo's own sounds it did not.

## Weak spots (for the guide)

These parts miss their bar, so agents should not rely on them yet:

* **Kind of sound (88%, bar 90%).** Short, plain sounds are the hardest:
  a footstep, a jump, a warning beep and the coin preset were each beaten
  by a wrong text. Ask with two or three texts and compare their ranks,
  rather than trusting one `match`.
* **Calm or tense music (75%, bar 80%).** "Tense music" is weak in both
  models: LAION-CLAP gives the tense loop "calm music" (0.57) above
  "fast exciting music" (0.56). Use Duck
  Echo's own numbers (tempo, key, loudness) for this question.
* **Yes and no (77% and 75%, bar 80% each).** About one answer in ten is
  wrong in each direction, and about one in seven is `unsure`. A yes or a
  no from the score alone is a hint; step 4's describe model adds a
  second opinion, and both must agree.

* **The closest words** (`closest` in an answer) are not measured by the
  check, and are often odd for short sounds: for the coin preset the
  first three are "speech", "footsteps on gravel" and "birds", and "coin
  pickup" comes fourth. Ask about the texts you mean with `--like`
  rather than reading the closest words.

What it does well: telling which of two sounds fits a text (98%), soft
from harsh (3 of 3), and silence (it never says a silent sound is
anything but silence).

## Honest notes

* **The set is small**: 41 sounds and 184 pairs of a sound and a text.
  One sound more or less right moves "kind of sound" by more than 2
  points.
* **The lines were chosen on this same set**, so these yes and no
  numbers are a little better than on new sounds. A cross-check that left
  each sound out of the reference in turn gave the same kind-of-sound
  score (88%) for both models.
* **The reference sounds are Duck Echo's presets and examples**, and
  many test sounds are those same presets. A `match` says "fits better
  than the text usually fits Duck Echo's sounds", so it is about Duck
  Echo's kind of sounds, not about all sound.
* **All sounds were made by Duck Echo**: no recordings. That is on
  purpose (anyone can rebuild them), but it means nothing here says how
  the Ear hears recorded audio.
* **The numbers are from one PC**, the CPU only. ONNX Runtime may round a
  little differently on another PC; the step 3 checklist compares Windows
  and Linux to 2 decimals.
