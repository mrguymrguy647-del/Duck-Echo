# Duck Ear check

This page records how well the Duck Ear's models hear Duck Echo's own
sounds, on one test set with known answers.

* **The score model** (step 3 of `docs/phase4-design.md`): two models
  were converted and measured, the better one was chosen, and its yes and
  no lines were set. This is most of the page.
* **The describe model** (step 4): four models were measured on the CPU.
  **None passes**, so the Ear ships with scores only. That part is at the
  end: "The describe model". Its numbers on a graphics card are **to be
  measured on the person's PC** (one command per size: "Measuring on a PC
  with a graphics card").

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

## The describe model

A describe model is a language model that hears: it says in words what a
sound is, and answers "does this sound like a coin pickup?" with yes, no
or partly and a reason. The design asks a lot of it: on the same test
set, it must say **yes to 80% of the true texts and no to 80% of the
wrong texts**, counted apart (a model that always says yes gets 100% and
0%), and it must not say yes to a sound when there is only silence or
noise.

**Result: none of the models measured passes.** Three say no far too
often to true texts, and the fourth (MiDashengLM-0.6B) does not answer
yes or no at all. As agreed in the design, the Ear ships with
**scores only**: the Small Ear has no describe model. Full
(Qwen2.5-Omni-7B) and Large (Qwen3-Omni-30B-A3B) can still be added by a
person, to measure them on a graphics card; they are not what Duck Echo
recommends until one passes.

### What was measured, and where

| | |
|---|---|
| PC | the same as above: Linux, Intel Xeon at 2.1 GHz, 4 cores, 15 GB of memory, **no graphics card** |
| llama.cpp | `llama-server` built from source at commit `a956a85` (November 2026), CPU only |
| Full | Qwen2.5-Omni-7B, `ggml-org/Qwen2.5-Omni-7B-GGUF` at `89b785438c89`, Q4_K_M (4.7 GB) with the Q8_0 hearing part (1.5 GB); Apache-2.0 |
| Tried for Small | Gemma 4 E4B, `ggml-org/gemma-4-E4B-it-GGUF` at `8903cb408c7d`, Q4_0 (4.6 GB + 0.6 GB); Apache-2.0 |
| Tried for Small | Gemma 4 E2B, `ggml-org/gemma-4-E2B-it-GGUF` at `2a8538c2e2d4`, Q4_0 (2.8 GB + 0.6 GB); Apache-2.0 |
| Tried for Small | MiDashengLM-0.6B, the design's first choice. Its GGUF (`mispeech/midashenglm-0.6b-gguf`) needs its makers' own fork of llama.cpp, which could not be fetched here, so its original weights were asked with PyTorch instead: `mispeech/midashenglm-0.6b-fp32` at `3969e451d1c6` (2.7 GB, fp32), transformers 4.57.6, not through the Ear; Apache-2.0 |
| Test set | the same 41 sounds: 47 true and 137 wrong pairs of a sound and a text |

Run it again with one command for a size, which downloads the model
(after asking) and writes `ear-check.json` and `ear-check.md`:

```bash
ear/.venv/bin/python ear/tools/measure_describe_model.py full
```

(On Windows: `ear\.venv\Scripts\python.exe ear\tools\measure_describe_model.py full`.)
Under it is `duck-echo ear check --part describe --fresh`: `--part score`
leaves the describe model out, `--part describe` is an error when the
Ear has none, and `--fresh` asks the Ear everything again whatever the
answer cache holds, so the times are that run's own.

### How a question is asked

Evidence first, in two turns of one talk, with temperature 0, the most
likely word each time, and seed 42:

1. The sound, and: *"What do you hear? Describe the sound itself in one
   or two short sentences: what kind of sound it is, its pitch, and how
   it feels. (It is 0.3 seconds long.)"* The length is given because
   these models pad every sound to a fixed window and cannot hear it.
   The answer is the description.
2. Then, after the model's own words: *"This sound was made for a video
   game. Does this sound like a coin pickup? Answer yes, no or partly,
   then say why in one sentence. Say yes only if that describes this
   sound well; say partly if only some of it fits."*

Asked in one turn, the models heard whatever the question named: the
small Gemma heard "a metallic clinking sound" in a coin *and* in an
explosion when asked about a coin, and "a loud bang" in both when asked
about an explosion. That is why the question comes second.

A sound shorter than 1 second is padded with silence to 1 second. In the
check, silence and noise go to the model like any sound (in normal use
the Ear answers silence itself, without the model).

### The results

| Measure | Bar | Qwen2.5-Omni-7B (Full) | Gemma 4 E4B | Gemma 4 E2B |
|---|---|---|---|---|
| Yes on true texts | 80% | 21 of 47 (**45%**) | 16 of 47 (34%) | 6 of 47 (13%) |
| No on wrong texts | 80% | 124 of 137 (90%) | 118 of 137 (86%) | 132 of 137 (96%) |
| Silence: no yes to a wrong text | 100% | 5 of 5 | 5 of 5 | 5 of 5 |
| Noise: no yes to a wrong text | 100% | 4 of 4 | 4 of 4 | 4 of 4 |
| Same words when asked again | 100% | 8 of 8 | 8 of 8 | 8 of 8 |
| A true text said no | fewer is better | 18 of 47 (38%) | 27 of 47 (57%) | 41 of 47 (87%) |
| A wrong text said yes | fewer is better | 3 of 137 (2%) | 16 of 137 (12%) | 5 of 137 (4%) |

Every model reaches the bar for wrong texts and misses the bar for true
texts by a wide margin. They are no-sayers: the opposite of what the
research warned of, and just as useless for a yes.

**MiDashengLM-0.6B is not in the table**, because it gave no yes or no
to measure. Asked the same two questions with its original weights, it
answered 154 of the 184 yes or no questions with its description again
("It is a bell ringing.") or the question's own words, and the other 30
with yes: 8 for true texts, 22 for wrong ones, and never a no. It writes
captions; it does not answer questions. See "What the models hear" for
its captions.

**The silence and noise rows pass, but the models are not listening to
silence.** The measure only counts a plain yes. Given pure silence,
Qwen2.5-Omni-7B described "a high-pitched, electronic, and synthesized
tone", said no to "silence", and said "partly" to "a coin pickup" and to
"an alarm". Both Gemma models heard "a low, resonant hum" or "a low,
rumbling bass note" in silence. So the Ear's own silence guard (a sound
whose peak is below -60 dBFS is answered without the model) stays.

**Same words twice**: after the run, the Ear and `llama-server` were
stopped and started again, and the first four sounds were asked again
with no answer cache: all 8 answers came back word for word, for every
model. A second run of four sounds with Qwen2.5-Omni-7B later gave the
same four descriptions and the same ten answers as the first.

### The verdict of both models

The `verdict` an agent sees needs both models to agree for a yes or a
no. With Qwen2.5-Omni-7B beside the score model:

| Who decides | Yes on true texts | No on wrong texts | A true text said no | A wrong text said yes |
|---|---|---|---|---|
| The score model alone | 36 of 47 (77%) | 103 of 137 (75%) | 4 (9%) | 13 (9%) |
| The describe model alone | 21 of 47 (45%) | 124 of 137 (90%) | 18 (38%) | 3 (2%) |
| Both must agree (the verdict) | 16 of 47 (34%) | 93 of 137 (68%) | 3 (6%) | **0 (0%)** |

This is the one useful thing the describe model does here: **with it,
the verdict never said yes to a wrong text** (the score alone did 13
times), and a yes was right 16 times out of 16. But it turns most true
yeses into `unsure`: 36 down to 16. A person who adds Full gets fewer
wrong answers and far fewer answers.

One example, the calm Duck Craft night theme: the score model gave
"tense music" a match of 0.98 (a clear yes, and wrong). The describe
model heard "a slow, soft, and dreamy electronic piece" and said no, so
the verdict was `unsure` instead of a wrong yes.

### What the models hear

Qwen2.5-Omni-7B hears the big differences: a beep from a low boom, music
from a sound effect, calm music from fast music (the calm loop: yes to
"calm music" and "slow gentle music", no to "tense music"; the tense
loop: yes to "fast exciting music", but no to "tense music"). It does not
hear what a game sound *is*: 14 of the 41 sounds are "a high-pitched,
electronic beep" to it (every pickup, both jumps, the menu click, the
alarm and the warning beep), a footstep and a landing are "a low, continuous hum", and a
metal clang is "a low, dark, and heavy electronic synth". It cannot tell
a soft coin from a harsh one (no to all of those texts).

The Gemma models hear less. Gemma 4 E2B gave only 18 different
descriptions for 41 sounds, 12 of them word for word "A low, resonant
hum. It feels deep and steady." (the coin among them), and silence is "a
low, resonant hum" to it too. Gemma
4 E4B called the coin "a sharp, high-pitched metallic clang" and silence
"a low, rumbling bass note".

MiDashengLM-0.6B is the quickest by far (about 1 s a caption on 4 CPU
cores, with PyTorch) and its captions are short. On music they are good:
the Duck Craft day theme is "a soft, ambient, and mellow piano song",
the tense loop "a fast-paced electronic music piece with a strong
bassline". On sound effects they are a lottery: the coin, the gem and
the chime are "A bell ringing", the shotgun "A gun fire" and the wind
"Waves and wind", but the menu click is "A cat meowing", the missile "A
sneeze", the footstep "Thunderstorm" and a jump "Bass drum". Silence is
"A loud, high-pitched, metallic sound that is repeated."

**Is a Small candidate worth keeping? No.** Gemma 4 E2B hears a hum in
nearly everything (yes to 13% of true texts). MiDashengLM-0.6B cannot
answer a question, its sound effect captions are wrong too often to
trust, and running it in the Ear would mean building and shipping its
makers' fork of llama.cpp ourselves. Its music captions are the one
thing worth remembering if a later version answers questions.

What Qwen2.5-Omni-7B heard in every sound, with its wrong answers:

| Sound | What the describe model heard | True texts not given a yes | Wrong texts given a yes |
|---|---|---|---|
| `preset:coin` | I hear a high pitched, electronic beep. |  |  |
| `examples/recipes/coin.toml` | The sound is a high-pitched, electronic beep with a short duration. |  |  |
| `preset:gem` | The sound is a high-pitched, electronic beep. It feels like a quick, sharp tone. |  |  |
| `preset:key_pickup` | The sound is a high-pitched, electronic beep with a clear and sharp quality. |  |  |
| `preset:extra_life` | The sound is a high-pitched, electronic beep with a clear and sharp quality. | a cheerful rising jingle (no) |  |
| `preset:powerup` | The sound is a high pitched, electronic beep with a long decay. It feels like a digital signal or notification. | a rising power-up sound (partly) |  |
| `examples/recipes/fanfare.toml` | I hear a short, high-pitched, electronic beep. It feels like a quick, sharp sound. | a short victory fanfare (partly) |  |
| `coin_soft.toml` | The sound is a high-pitched, electronic beep. It feels like a short, sharp tone. | a gentle chime (no), a soft coin pickup (no) |  |
| `coin_harsh.toml` | I hear a short, high-pitched beep. It feels like a quick electronic signal. | a harsh buzzy coin sound (no) |  |
| `preset:laser` | I hear a high pitched, electronic, synthesized sound that feels like a quick, sharp zap. |  |  |
| `examples/recipes/laser.toml` | I hear a high pitched, electronic, synthesized sound that feels like a quick, sharp, and sudden burst. |  |  |
| `preset:blaster` | The sound is a low, deep, and resonant electronic beep. | a laser shot (no) |  |
| `preset:shotgun` | I hear a short, sharp, and high-pitched sound. It feels like a quick, sudden strike. | a gunshot (partly) |  |
| `preset:explosion` | I hear a sound that is a bit like a gun firing, but it's not a real gun. It's a synthesized sound, and it's quite loud. | an explosion (partly) |  |
| `preset:big_explosion` | I hear a loud, deep, metallic bang that reverberates and echoes. It feels like a heavy object hitting a hard surface. | a big explosion with a long rumble (partly) |  |
| `examples/recipes/explosion.toml` | I hear a sound that is a combination of a whoosh and a pop, with a high pitch and a feeling of intensity. |  |  |
| `preset:missile` | I hear a sound that is a low, deep, and resonant electronic tone, it feels like a powerful and vibrating sound. | a whoosh (partly) |  |
| `preset:punch` | The sound is a low, deep, and resonant bass drum hit, with a slow decay and a sense of power. | a punch (no) |  |
| `preset:hit` | The sound is a low, deep, and resonant electronic beep. It feels like a low-frequency electronic signal. | a hit (no) |  |
| `preset:thud` | I hear a low, deep, and resonant sound that feels heavy and powerful. |  |  |
| `preset:land` | The sound is a low, continuous hum, like a distant engine or a soft, vibrating background noise. | a heavy thud (no) |  |
| `preset:metal_clang` | The sound is a low, dark, and heavy electronic synth with a slow, pulsating rhythm. | a metal clang (no) |  |
| `preset:footstep` | The sound is a low, continuous hum, like a distant engine or a soft, ambient noise. It has a low pitch and feels steady and unchanging. | a footstep (no) |  |
| `preset:jump` | The sound is a high-pitched, electronic beep that feels like a quick, sharp signal. | a jump in a platform game (partly) |  |
| `examples/recipes/jump.toml` | The sound is a high-pitched, electronic beep that feels like a short, sharp, and clear signal. | a jump in a platform game (partly) |  |
| `preset:ui_click` | The sound is a high-pitched, electronic beep that feels like a digital signal. |  |  |
| `preset:ui_confirm` | I hear a high pitched, short, and sharp sound. |  |  |
| `preset:ui_error` | The sound is a low, electronic beep with a slow decay. It feels like a simple, clean tone. | an error buzz (no) |  |
| `examples/recipes/chime.toml` | The sound is a high-pitched, clear, and resonant tone that feels sharp and crisp. | a gentle chime (no) | a harsh buzzy coin sound (yes) |
| `preset:alarm` | The sound is a high-pitched, electronic beep with a smooth, clean quality. | an alarm (no) | a coin pickup (yes) |
| `examples/recipes/siren.toml` | The sound is a high-pitched, electronic, and synthesized tone that feels like a futuristic or sci-fi sound effect. | a siren (no) |  |
| `preset:warning_beep` | I hear a short, high-pitched beep. |  |  |
| `preset:engine_hum` | The sound is a low, dark, and heavy synthesizer bass with a slow, pulsating rhythm. It has a deep, rich, and resonant quality, creating a sense of depth and intensity. | an engine hum (no) |  |
| `preset:wind` | The sound is a whoosh, it is high pitched and feels like it is moving quickly. |  | a laser shot (yes) |
| `calm_loop.toml` | I hear a slow, soft, and dreamy synth melody with a high pitch. |  |  |
| `tense_loop.toml` | The sound is a fast-paced, electronic music track with a strong beat and a repetitive synth melody. It has a high pitch and feels energetic and upbeat. | tense music (no) |  |
| `example:duck_craft_day` | The sound is a synth melody with a slow tempo, low pitch, and a dreamy, ethereal quality. |  |  |
| `example:duck_craft_night` | I hear a slow, soft, and dreamy electronic piece with a repetitive melody and a slow, steady beat. |  |  |
| `example:four_bars` | The sound is a synth melody with a retro vibe, it's high pitched and feels nostalgic. |  |  |
| `silence` | The sound is a high-pitched, electronic, and synthesized tone that feels like a futuristic or sci-fi sound effect. | silence (no) |  |
| `noise.toml` | The sound is a high-pitched, electronic whoosh that feels fast and sharp. | white noise (no) |  |

### The times, on the CPU

Measured on 4 CPU cores with nothing else running (four sounds):

| | Qwen2.5-Omni-7B | Gemma 4 E4B | Gemma 4 E2B |
|---|---|---|---|
| Starting `llama-server` and loading the model (files already read once) | 4 to 8 s | 5 s | 4 s |
| Hearing a sound the first time (`describe`) | 32 s (longest 34 s) | 3.5 s | 1.6 s |
| A yes or no question about a sound already heard | 8 s (longest 10 s) | 3.0 s | 1.5 s |
| `listen` with two texts and one free question, from nothing | 67 s | | |

Qwen2.5-Omni-7B pads every sound to 30 seconds (750 pieces of sound for
the model), so a coin costs as much as a tune. `llama-server` keeps what
it worked out for a sound, so the second question about the same sound
costs a quarter of the first. Duck Echo's time limits (240 s for a
describe question on the CPU, 180 s to load) leave room for a PC a few
times slower. The Gemma times are from the full run, where other work
ran beside it now and then.

### To be measured on the person's PC

Nothing here ran on a graphics card: the build machine has none. These
are filled in from the `ear-check.json` and `ear-check.md` that
`ear/tools/measure_describe_model.py` writes on a PC with a big card
(the first one: an NVIDIA RTX 4090, 24 GB, with 36 GB of memory):

| | Full (Qwen2.5-Omni-7B) | Large (Qwen3-Omni-30B-A3B) |
|---|---|---|
| Yes on true texts (bar 80%) | 45% on the CPU; on the card: to be measured on the person's PC | to be measured on the person's PC |
| No on wrong texts (bar 80%) | 90% on the CPU; on the card: to be measured on the person's PC | to be measured on the person's PC |
| Silence and noise | to be measured on the person's PC | to be measured on the person's PC |
| Verdict: yes on true, no on wrong, mistakes | 34%, 68%, 6% and 0% on the CPU; on the card: to be measured on the person's PC | to be measured on the person's PC |
| Same words when asked again | 8 of 8 on the CPU; on the card: to be measured on the person's PC | to be measured on the person's PC |
| Loading the model | to be measured on the person's PC | to be measured on the person's PC |
| Hearing a sound the first time | to be measured on the person's PC | to be measured on the person's PC |
| A further yes or no question | to be measured on the person's PC | to be measured on the person's PC |
| The card's memory in use | to be measured on the person's PC | to be measured on the person's PC |

What is known and not known before that:

* **Every time on a graphics card** is unmeasured: loading, hearing a
  sound, a question, for Full and for Large. The 60 s limit for a
  describe question on a card is a guess until then.
* **Large (Qwen3-Omni-30B-A3B) is not measured at all**: its yes and no
  answers and its times. It was downloaded here and both files match
  their SHA-256, but it could not be loaded: it is 19.9 GB, the build
  machine has 15 GB of memory and no card, and the kernel stopped
  `llama-server` for lack of memory about 30 s into loading, twice. So
  it is also not yet known that this `llama-server` answers the Ear's
  questions with this model; the checklist's first Large step finds
  out.
* **Full's answers on a card** may differ in a word here and there from
  the CPU's (the same model, but sums done in another order). The
  measures should be close to the ones above; "same words twice" must be
  checked again on the card.
* **The fall back to the CPU** when a card has too little memory, and
  that the card's memory is free again 2 minutes after the last describe
  question, are tested with a pretend `llama-server` only (and the
  2-minute rule with the real one, on the CPU).

### The quick set, through the one command

`ear/tools/measure_describe_model.py full --quick` was run on the build
machine, to try the whole way a person will go: it found `llama-server`,
`duck-echo` and the Ear, downloaded Qwen2.5-Omni-7B (6.2 GB), checked
both files' SHA-256, asked about the 10 sounds of `ear/testset/quick.toml`
(52 yes or no questions, silence and noise among them) with no answer
cache, and wrote `ear-check.json` and `ear-check.md`.

| Measure, on the quick set (CPU) | Qwen2.5-Omni-7B |
|---|---|
| Describe: yes on true texts | 7 of 13 |
| Describe: no on wrong texts | 32 of 39 |
| Silence: no yes to a wrong text | 5 of 5 |
| Noise: no yes to a wrong text | 4 of 4 |
| Verdict: yes on true texts | 5 of 13 |
| Verdict: no on wrong texts | 25 of 39 |
| Verdict: a wrong text said yes | 0 of 39 |
| Same words when asked again | 8 of 8 |

All 10 descriptions and all 52 answers are word for word those of the
full run a day earlier (another start of the machine, another
`llama-server`). Ten sounds are too few to judge a model by; the quick
set is there to show that the driver and the verdict work, and it does.
(Its times, 34 s to hear a sound and 11 s a question, were taken while
Duck Echo's tests ran beside it; the times above are the quiet ones.)

### Measuring on a PC with a graphics card

One command per size. It finds `llama-server`, the graphics card,
`duck-echo` and the Ear; shows the model's size and asks before it
downloads; checks every file's SHA-256; runs `duck-echo ear check --part
describe --fresh` on the whole test set; and writes `ear-check.json` and
`ear-check.md` into `ear-check-full\` or `ear-check-large\`. It does not
change which model the Ear uses.

On Windows, in PowerShell, from the repository's top folder. Before the
first time: `cargo build --release -p duck-echo-cli`, `uv sync` in `ear\`,
and the score model made (`ear/convert/README.md`; `duck-echo ear status
--ear-program ear\.venv\Scripts\duck-ear.exe` then names
`laion-clap-general`).

1. Get `llama-server` (llama.cpp's newest Windows build for any graphics
   card, the Vulkan one) into the Ear's folder:

   ```powershell
   $ear = "$env:LOCALAPPDATA\DuckEcho\ear"
   $release = Invoke-RestMethod https://api.github.com/repos/ggml-org/llama.cpp/releases/latest
   $zip = $release.assets | Where-Object name -like "*bin-win-vulkan-x64.zip" | Select-Object -First 1
   Invoke-WebRequest $zip.browser_download_url -OutFile "$env:TEMP\llama.zip"
   Expand-Archive "$env:TEMP\llama.zip" -DestinationPath "$ear\llama" -Force
   ```

   (These five lines could not be tried on the build machine, which
   cannot reach GitHub. If they fail, download the file whose name ends
   in `bin-win-vulkan-x64.zip` from
   https://github.com/ggml-org/llama.cpp/releases by hand and unpack it
   into `%LOCALAPPDATA%\DuckEcho\ear\llama`.)

2. See that the card is found and Large is offered:

   ```powershell
   ear\.venv\Scripts\python.exe ear\tools\get_describe_model.py --list
   ```

3. A quick try first (10 sounds, a few minutes with the 6.2 GB
   download): it shows that everything works.

   ```powershell
   ear\.venv\Scripts\python.exe ear\tools\measure_describe_model.py full --quick
   ```

4. Measure Full (6.2 GB):

   ```powershell
   ear\.venv\Scripts\python.exe ear\tools\measure_describe_model.py full
   ```

5. Measure Large (19.9 GB to download):

   ```powershell
   ear\.venv\Scripts\python.exe ear\tools\measure_describe_model.py large
   ```

6. While step 4 and step 5 run, note in Task Manager (Performance, GPU)
   the "Dedicated GPU memory" in use. After each ends, Task Manager
   shows no `llama-server.exe`.

7. Send back four files: `ear-check-full\ear-check.json`,
   `ear-check-full\ear-check.md`, `ear-check-large\ear-check.json` and
   `ear-check-large\ear-check.md` (and the two memory numbers). They hold
   the model, the card, every yes and no with its reason, the verdicts,
   and the times.

If a run stops with a time limit while a model loads from a slow disk,
set `$env:DUCK_ECHO_EAR_TIMEOUT_SCALE = "6"` and run it again (the tool
uses 3 by itself). The Ear's log is `ear.log` and `llama-server`'s is
`llama-server.log`, both in `%LOCALAPPDATA%\DuckEcho\ear`.

On Linux the commands are the same with `ear/.venv/bin/python` and
`ear/tools/...`, and `llama-server` goes into
`~/.local/share/duck-echo/ear/llama/`.

### Honest notes

* **One wording was measured in full.** Four wordings of the questions
  were tried by hand first, on two to six sounds each. A kinder second
  question ("judge if a player would take it for ...") made
  Qwen2.5-Omni-7B say yes to "a coin pickup" for wind; the plain
  question with no word about games made it say no to "a coin pickup"
  for the coin. The wording used is between the two. Another wording
  could move some answers between yes, partly and no. But 18 true texts
  got a plain no, mostly for sounds whose description was already wrong
  (a footstep heard as a hum), and wording does not fix hearing.
* **Counting "partly" as yes would not pass either**: 29 of 47 true texts
  (62%) for Qwen2.5-Omni-7B.
* **Naming the sound's maker changes what is heard.** Telling
  Qwen2.5-Omni-7B that the sounds were "made with a synthesizer" before
  it listened turned an explosion into "a short, sharp electronic beep".
  So the first question says nothing about the sound.
* **Gemma is asked plainly.** Asked for an answer in a forced shape, both
  Gemma models think for hundreds of words first, even with thinking
  switched off (about 30 s for an answer on Gemma 4 E2B). They are asked
  without the shape, and the Ear reads "Yes, ..." itself. Qwen answers in the forced
  shape.
* **The sounds are short and made by a synthesizer**, and these models
  learned from speech, music and recordings. The music results are
  better than the sound effect results for that reason.

