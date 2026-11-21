# Playtest 5: agent notes

Fresh agent, working only through `bridge.py` (the MCP server). Notes written as I go.

## Setup

- `bridge.py list` and `get_guide` read top to bottom.
- `ear_status`: "The Duck Ear 0.4.0 is ready, on CPU (ONNX Runtime 1.32.0): it can score."
  Capabilities: `["score"]` only; `ask` and `describe` not available (no describe model).

## Picking the sounds: a survey with `listen`

I asked the Ear about each preset with one text each, the words I would mean for it
(`listen` with `path: preset:NAME`, one `like`). Results (match, verdict, rank of 103):

| preset | my words | Ear |
|---|---|---|
| jump | a light, springy jump in a cartoon platform game | yes 0.90 (rank 2) |
| footstep | a single footstep on a hard floor | **unsure 0.48** (rank 26) |
| land | a character landing on the ground after a jump | yes 0.95 |
| low_health | a slow heartbeat warning of low health | yes 0.97 |
| warning_beep | three short warning beeps | yes 0.93 |
| ui_hover | a soft tick when hovering over a menu button | yes 0.92 |
| punch | a punch hitting a body | yes 0.83 |
| metal_clang | a metal pipe struck, ringing | yes 0.99 |
| menu_open | a soft upward swish as a menu opens | yes 0.90 |
| gem | a sparkly gem pickup | yes 0.79 |
| coin | a classic coin pickup in a platform game | yes 0.73 |
| thud | a heavy door slamming shut | yes 0.82 |
| ui_click | a short crisp click of a button | yes 0.94 |
| engine_hum | a steady low engine hum | yes 0.99 |
| wind | a gust of wind | yes 0.99 |
| hit | a character taking damage | yes 0.85 |
| ui_error | a low buzz when an action is refused | yes 0.78 |
| explosion | an explosion | yes 0.76 |
| laser | a retro laser shot | yes 0.93 |
| charge_up | energy charging up before a shot | unsure 0.65 |
| powerup | a rising power-up chime | yes 0.87 |
| missile | a missile launching with a whoosh | **unsure 0.35** (rank 36) |
| shotgun | a shotgun blast | yes 0.89 |
| extra_life | a triumphant extra life jingle | yes 0.93 |
| blaster | a punchy retro blaster shot | yes 0.94 |
| alarm | a repeating two-tone alarm | yes 0.93 |
| ui_confirm | two rising notes confirming a choice | unsure 0.51 |
| ui_cancel | two falling notes for cancel | no 0.15 |
| key_pickup | picking up a key | no 0.19 |
| big_explosion | a huge explosion with a long rumble | yes 0.99 |

**Jump was not a sound the Ear is unsure about** on my words: yes 0.90. But it says yes to
almost anything with "jump" in it: "a heavy jump" got yes 0.84 and "a soft, bouncy jump" yes 0.94,
which are opposites. "a rising whistle" (the pitch does rise, 254 to 659 Hz) got no 0.31.
So on jump the Ear's yes is not telling me much; I worked on it anyway, as the task asks,
and used "a heavy jump" as an opposite text to see whether my changes moved the two apart.

I picked:
- **footstep** (unsure 0.48): a short, plain sound, the kind the guide warns the Ear is weak
  at ("A footstep, a jump, a warning beep and the coin were each beaten by a wrong text at least once").
  I wanted to see whether changes could move it, or whether it just cannot hear footsteps.
- **missile** (unsure 0.35, rank 36): the lowest "unsure" of the lot, and a longer sound
  (so the guide's "short sounds" excuse does not apply). The preset is described as "Missile launch
  whoosh", my words say exactly that, yet the Ear is unsure.
  (I did not pick ui_cancel or key_pickup, where the Ear said a firm "no", because I suspect my words
  "two falling notes for cancel" and "picking up a key" are not things a sound model can hear.)

The verdict thresholds seem to be about: no below ~0.3, unsure ~0.3 to ~0.67, yes above ~0.68
(a laser shot 0.29 = no, missile 0.35 = unsure, charge_up 0.65 = unsure, cartoon boing 0.69 = yes).
The guide does not say where they are.

## Sound 1: jump (`sfx/jump.toml`)

**My words:** "a light, springy jump in a cartoon platform game". Opposite text, to see the
Ear separate them: "a heavy jump".

**Before** (`preset:jump` copied with `create`, rendered to `sfx/jump_before.wav`):
`0.27 s, ~-20.0 LUFS, mid, sharp attack, tonal, pitch rising from 254 to 659 Hz, no clipping, ends cleanly`.
Ear: my words yes 0.90 (rank 2 of 103); "a heavy jump" yes 0.84.

Step 0 (setup, as the guide says to do first): level by loudness, `master.normalize_lufs = -20`
(the movement target), removing `normalize` and `normalize_peak_db`. The preset was already at
~-20.0 LUFS, so nothing audible changed (Ear: about the same on both texts). I do not count it as one
of my five.

For each change I rendered to `sfx/work/jump_cN.wav`, ran `compare` against the before WAV, and
`listen` with `compare_with: sfx/jump_before.wav` on both texts.

| # | change | numbers | Ear (light / heavy) | kept? |
|---|---|---|---|---|
| 1 | `pitch.end` 660 -> 990 (bigger, higher leap) | 0.27 s, ~-20.0 LUFS, peak -10.7, pitch 267 -> 989 Hz, no clipping, ends cleanly | 0.89 / 0.76 (light same, heavy -0.08) | kept: the sound moved away from "heavy" and the numbers are fine |
| 2 | `pitch.sweep_time` 0.18 -> 0.12 (quicker spring) | same length and loudness, pitch reaches 966 Hz by the 3rd point | 0.87 / 0.68 | kept, but a judgement call: my text went down 0.025 ("about the same"), the opposite went down 0.16. The guide's rule ("the match went up") says undo; I kept it because the gap between the two texts grew, and the numbers show exactly the snappier rise I meant |
| 3 | `envelope.decay` 0.22 -> 0.15 (shorter, lighter) | 0.20 s, ~-20.0 LUFS, peak -11.2, ends cleanly; tonality fell 0.62 -> 0.54, summary now says "tone and noise" | **0.93** / 0.64 (heavy now unsure) | kept: best on both texts |
| 4 | `duty` 0.3 -> 0.5 (rounder tone) | same length/loudness, peak -13.8, tonality back to 0.69 | 0.91 / 0.72 | **undone** (edit_undo): worse than step 3 on both texts |
| 5 | `filter.resonance` 1.2 -> 3 (more "boing") | 16% brighter, sharpness low -> medium, and the pitch track's last point jumped to **1971 Hz** (the resonance made the 2nd harmonic loudest) | 0.92 / 0.65 | **undone**: Ear about the same as step 3, but the numbers show an octave jump in the tail I did not want. Numbers trusted over the Ear here |

**After** (`sfx/jump_after.wav`): `0.20 s, ~-20.0 LUFS, mid, sharp attack, tone and noise,
pitch rising from 294 to 989 Hz, no clipping, ends cleanly`; peak -11.2 dBFS. Ear on my words
**0.90 -> 0.93** (yes both times); "a heavy jump" 0.84 -> 0.64 (yes -> unsure).
`compare`: "same loudness, 0.07 s shorter, waveforms differ (0.18)". The picture shows the rise
finishing at about 0.12 s and a clean held tone fading out, as intended.

Honest verdict: the Ear's score on my words hardly moved (0.90 to 0.93, under its own 0.05
"about the same" line). What moved was the opposite text. The real evidence for the change is the
numbers (shorter, faster, higher rise), which are what I asked for.

## Sound 2: footstep (`sfx/footstep.toml`)

**My words:** "a single footstep on a hard floor". Opposite text: "a hand clap" (the Ear's
favourite for the preset).

**Why:** in the survey the Ear was **unsure (0.48)** on my words. On the before WAV it was sure
of the wrong things: "a hand clap" yes 0.97 (rank 3), "a short burst of static noise" yes 0.92,
while plain "a footstep" got **no 0.12** and "a shoe stepping on a wooden floor" no 0.31.
The numbers agree with the Ear's complaint: `0.06 s, ~-20.0 LUFS, bright, sharp attack, noisy`,
centroid 4449 Hz, and `per_layer` showed the low "tap" layer 12 LU under the scuff
(`~-32.3 LUFS, dark ... booming`): the heel is nearly inaudible.

Setup: `normalize_lufs = -20` as in the guide (Ear and waveform unchanged, correlation 1.00).

| # | change | numbers | Ear (footstep / clap), vs before | kept? |
|---|---|---|---|---|
| 1 | `layers[1].volume` 0.6 -> 1.5 (louder low tap) | centroid 4449 -> 4267 Hz only, waveforms 0.92 alike | 0.48 / 0.93: about the same | **undone**: neither moved; with loudness normalising, a low layer barely counts (K-weighting) |
| 2 | `layers[0].filter.cutoff` 1200 -> 500 (darker scuff) | "21% darker", centroid 3495 Hz, peak -8.0, sharpness medium -> low | **0.71 yes** / 0.82 (footstep +0.23, clap -0.15) | kept |
| 3 | `layers[0].envelope.decay` 0.05 -> 0.09 (longer scuff) | 0.10 s, roughness low -> medium, attack now "quick" | **0.32 no** / 0.86 | **undone**: big drop |
| 4 | `layers[0].envelope.decay` 0.05 -> 0.035 (tighter, harder) | 0.04 s, 23% darker, peak -8.3, ends cleanly | **0.85 yes** / 0.85 | kept |
| 5 | `layers[1].volume` 0.6 -> 1.2 (now that the scuff is dark, try the heel again) | "30% darker", centroid 3112 Hz, `boom` low -> **medium**, pitch track now 69 Hz, peak -6.8, attack "quick" | 0.88 / 0.91 (vs step 4: footstep +0.03, clap +0.06) | kept: trusted the numbers. The Ear held my words and moved a little towards "clap", but the numbers show exactly the heel thump I meant |

To check whether the Ear's jump from 0.48 to 0.85 was luck of one seed (the preset has
`variation = 0.3`), I asked it with `seed` 1 to 5 (`quick: true`) about the preset and my version:
preset 0.38 to 0.48 every time, mine 0.76 to 0.99 every time. So the improvement on my words is real
for the Ear, not one lucky render. But "a hand clap" stayed 0.84 to 0.99 for mine too.

**After** (`sfx/footstep_after.wav`): `0.04 s, ~-20.0 LUFS, bright, quick attack, noisy, no clipping,
ends cleanly`; per layer: scuff ~-21.4, heel ~-22.4 LUFS (was 12 LU apart, now 1). Ear on my words
**0.48 unsure -> 0.88 yes** (`compare_with`: after is more like it by 0.40). "a hand clap" 0.97 -> 0.91,
"a short burst of static noise" 0.92 -> 0.95 (still yes).

What I am not sure of: the Ear says yes to footstep, clap and static all at once, so it does not
really tell them apart; its score swung from 0.71 to 0.32 to 0.85 for decays of 50, 90 and 35 ms,
which is a lot for a small change. The summary still says "bright" although the picture shows most
energy under 1 kHz (centroid 3112 Hz, pulled up by the noise's high tail). 0.04 s is short for a footstep;
the numbers say it is clean, but a person should listen.

## Sound 3: missile (`sfx/missile.toml`)

**My words:** "a missile launching with a whoosh". Also asked: "a rocket taking off" (the same thing in
other words) and "a low rumble" (an opposite: what I did not want to lead).

**Why:** the lowest "unsure" in the survey (0.35, rank 36 of 103) for a sound whose own description is
"Missile launch whoosh", and at 0.70 s it is long enough that "short sounds" is no excuse. On the before
WAV the Ear said "a whoosh" yes 0.73 and "rushing air rising in pitch" yes 0.89, but "a rocket taking off"
**no 0.04**, "a jet engine" no 0.02. `per_layer` showed why the low end might confuse it: the falling saw
"rumble" (-18.1 LUFS alone) was louder than the "whoosh" noise (-23.5 LUFS alone).

Before: `0.70 s, -17.0 LUFS, bright, slow attack, noisy, moving, no clipping, ends cleanly, 2 layers`, peak -5.7.
Setup: `normalize_lufs = -17` (weapon target); nothing changed (correlation 1.00, Ear identical).

| # | change | numbers | Ear (missile / rocket / rumble), vs before | kept? |
|---|---|---|---|---|
| 1 | `layers[1].volume` (rumble) 0.4 -> 0.2 | 19% brighter, peak -4.4, boom medium -> low, roughness medium -> low | 0.41 / 0.05 / 0.09 (missile +0.07) | kept |
| 2 | `layers[0].envelope.hold` (whoosh) 0.35 -> 0.6 | 0.95 s, -17.0 LUFS, peak -6.1, 28% brighter, ends cleanly | **0.47** / 0.05 / 0.07 (missile +0.12) | kept |
| 3 | `layers[0].filter.resonance` 2 -> 1 (airier) | 53% brighter, sharpness low -> medium | 0.32 no | **undone** |
| 4 | `layers[0].filter.cutoff_end` 2500 -> 1500 (less bright end) | 15% brighter than before (darker than step 2) | 0.31 no | **undone** |
| 5 | `layers[0].envelope.decay` 0.3 -> 0.5 (longer tail, flying away) | 1.15 s, ends cleanly | 0.40 (asked with `compare_with` step 2: step 2 more like it by 0.07) | **undone** |

**After** (`sfx/missile_after.wav`): `0.95 s, -17.0 LUFS, bright, slow attack, noisy, moving, no clipping,
ends cleanly, 2 layers`, peak -6.1 dBFS; whoosh now leads (-18.5 vs rumble -21.5 LUFS alone).
`compare`: "same loudness, 28% brighter, 0.25 s longer, attack 111 ms faster, waveforms partly similar (0.82)".
Ear on my words **0.35 -> 0.47, still unsure** (after more like it by 0.12). "a rocket taking off" stayed
no (0.04 -> 0.05). "rushing air rising in pitch" 0.89 -> 0.96.

My reading: the Ear clearly hears "rushing air rising" and "whoosh", but no change I tried makes it hear a
*missile* or a *rocket*. That may be the model's vocabulary, not the sound: a synthesized rising noise is
what the preset is. I stopped at five changes. Every change I kept was also sensible by the numbers.
"attack 111 ms faster" in the compare line surprised me since I did not touch an attack: that is the
documented "attack is time to the loudest moment" effect (the whoosh now peaks earlier in relation to
its longer body).

## The tunes' mood: `duck_craft_day` and `duck_craft_night`

Copied with `create` (`from: example:...`) to `music/`, rendered to `music/duck_craft_day.ogg` and
`music/duck_craft_night.ogg` with `per_track: true`. I did not change the songs.

A caveat first: the copied files keep their comments, and they state the intended mood
("a calm, bright loop for the daytime", "a dark, slow-moving loop for the night", tags "calm", "dark").
So my judgement is not blind; I tried to reach it from the tools' numbers and the Ear before
trusting the comments.

### What the numbers said

| | day | night |
|---|---|---|
| summary | 8 bars looping at 84 BPM, 4 tracks; 22.86 s, -18.0 LUFS, no clipping, loop joins cleanly, 113 of 113 notes heard | 8 bars looping at 84 BPM, 5 tracks; 22.86 s, -18.0 LUFS, no clipping, loop joins cleanly, 92 of 92 notes heard |
| tempo | 84 BPM | 84 BPM |
| spectral centroid | 1225 Hz | 775 Hz (`compare`: "34% darker") |
| melody (keys) range, from `note_check` | C5 to D6, "mid" | G#3 to D5, "dark" |
| notes per beat (from `note_check`, 32 beats) | keys 0.78, bass 0.50, drums 1.50 | keys 0.69, bass 0.31, drums 0.75, stars 0.38 |
| mix onsets | 36 | 10 |
| energy over time | -18.9 to -22.8 dBFS (steady) | -18.0 to -21.9 dBFS (steady) |
| feel | boom medium, movement medium, roughness low, sharpness low | same |
| pitch classes heard (my count from `note_check` with `notes: true`) | G 15, C 14, D 9, A 8, E 7, B 6, F 6: no sharps, C and G most: **C major** | A 17, E 14, F 12, C 9, D 8, **G# 5**, B 3: **A minor** (harmonic, with the E-major chord) |
| quietest track | drums 6 LU under keys | stars **14 LU** under keys (the guide says 10 LU or more is "lost in the mix"; here it is meant to be "far-off", but nothing flags it) |

Duck Echo does not report a key, a mood or notes per beat itself; I worked those out from
`note_check` (see Problems below).

### What the Ear said

`listen` on each OGG with mood texts, and `compare_with` the other song:

- On day, with "calm, peaceful music", "tense, suspenseful music", "happy, cheerful music",
  "sad, melancholy music", "bright daytime music", "dark night-time music": **yes to all six**
  (0.90 to 1.00). "sad, melancholy music" was rank 1, "dark night-time music" rank 2 for the *day* tune.
- On night, with "calm", "tense", "happy", "sad", "relaxing", "scary", "a cheerful tune", "a gloomy tune":
  yes to all eight; "calm", "tense", "sad" and "scary" were all rank 1 at once.
- The only signal is in `compare_with`: day more like "happy, cheerful music" by **0.38**, day more like
  "a cheerful tune" by 0.11, night more like "scary" by 0.08. Everything else "about the same".
- `ask` ("Is this tune calm or tense, happy or sad?") is not available (no describe model); its summary
  was just "Ear (advice): heard".

### Conclusion

Both are **calm**: slow (84 BPM), few notes (at most 1.5 drum hits per beat, under 1 pitched note per beat
per track), steady level, low roughness and sharpness. **Day is happy**: major key, brighter (1225 Hz),
melody an octave higher, busier light beat. **Night is sad/melancholy, a little uneasy rather than tense**:
minor key with a G# leading tone, 34% darker, melody low, sparse heartbeat drum, a bigger reverb. I trusted
the numbers for this; the Ear's absolute answers were useless here (it said yes to every mood) and only its
relative answers (happier: day, scarier: night) pointed the same way as the numbers, by small margins.

## Where the Ear helped, and where it misled me

- **Helped most on footstep**: it heard "hand clap / static" in the preset, which matched the numbers
  (bright, 12 LU between layers), and it rewarded the darker, tighter version strongly (0.48 -> 0.88), steadily
  across five seeds. That steered me to the scuff's filter and decay, which the numbers alone would not
  have ranked.
- **Helped as a veto**: it marked the longer footstep decay (0.32 no) and two missile changes as worse, and
  the numbers did not object to any of them, so without the Ear I might have kept them.
- **Misled / not believable**:
  - It says yes to opposites: "a heavy jump" 0.84 and "a soft, bouncy jump" 0.94 on the same sound;
    footstep, hand clap and static all yes on my final footstep. A single yes means little.
  - On music its scores saturate near 1.0 for any mood text (see above).
  - On jump step 5 it was happy (0.92) with a version whose pitch track showed an unwanted octave jump
    to 1971 Hz. The numbers caught it; the Ear did not.
  - Its footstep score swung 0.71 -> 0.32 -> 0.85 for scuff decays of 50, 90 and 35 ms. I could not tell
    whether that is real or noise, so I checked with several seeds before believing it.
  - For missile it never got past "unsure", whatever I did, while "rushing air rising in pitch" was 0.96:
    I think it hears the sound well but has no idea what a synthesized missile is.
- **Where I disagreed and trusted the numbers**: jump step 2 (my text slightly down, kept because the gap to
  "heavy" grew and the numbers showed the faster rise I meant); footstep step 5 (Ear moved a little towards
  "clap", kept because the numbers show the heel thump I meant: boom low -> medium, 69 Hz).

## Confusions and problems in the guide and the tools (most important first)

1. **The `listen` tool description contradicts the guide and the Ear.** The tool says it returns
   "the describe model's yes or no with a reason, and a `verdict` (yes, no or unsure: the two models must
   agree)", and offers `ask` ("One free question ... The answer is the describe model's words"). The guide
   and `ear_status` say: "The Ear has a score model only ... So `ask` (a free question) and `description`
   say they are not available, and every verdict comes from the score alone". No `answer` or `reason` came
   back in any `like` entry. The tool description should say scores only, or say "when a describe model is
   installed". The `ask` answer's summary, "Ear (advice): heard", says nothing; it should say "ask is not
   available: no describe model".
2. **The guide promises mood numbers that do not exist.** "For a tune's mood, trust Duck Echo's own numbers
   instead: tempo, key, loudness, how many notes per beat." There is no key and no notes-per-beat field in
   `analyze` or `render` (I checked every key of `analysis` and the `analysis` reference page: the words
   "key", "major", "minor", "mood" do not appear in it). I had to work out the key and the density myself by
   counting `note_check` notes (`notes: true`). Either add `key` and `notes_per_beat` to a song's analysis
   or change the sentence.
3. **The Ear's `match` saturates on music, and the guide undersells it.** The guide says the Ear is weak at
   "Calm against tense music. It gave a tense loop 'calm music' above 'fast exciting music'". In fact on both
   example songs it gave *every* mood text 0.82 to 1.00 and "yes" (calm, tense, sad, happy, scary at rank 1
   together). "match (0 to 1: how much better the text fits this sound than it usually fits Duck Echo's
   sounds)" seems to mean "than it fits sound effects", so anything musical scores near 1. The guide should
   say: for songs, ignore `match` and `verdict`; only `compare_with` differences mean anything.
4. **`compared.by` is 0.0 whenever the difference is under 0.05**, even when `this` and `other` differ
   (jump step 2: this 0.873, other 0.898, `by: 0.0`). The guide says "under 0.05 is 'about the same', so do
   not chase it", but reporting 0.0 hides the direction; I had to read `this`/`other` myself. Report the real
   difference and let `more_like` say "about the same".
5. **`compare_with` only compares against one file.** To do the guide's loop (compare the new step with the
   previous kept step, not with the original), I had to keep rendering each step to its own WAV and choose
   which to compare with. The guide's example compares with `jump_before.wav` only; it does not say what to
   do from the second change on. A line saying "compare with your last kept version, not only the
   original" would help.
6. **The verdict thresholds are not documented.** From the answers: no below about 0.3, yes above about 0.68
   (0.29 no, 0.35 unsure, 0.65 unsure, 0.69 yes). The guide says what yes/no/unsure mean but not where they
   fall, so I could not tell whether 0.65 was nearly a yes.
7. **The guide points to a file I cannot read**: "The measured results are in `docs/ear-check.md`" and
   `ear_status` says "(docs/ear-check.md)" and "ear/tools/get_describe_model.py gets one". Through the MCP
   server there is no way to read either (`get_guide` topics are guide, recipe_spec, song_spec, analysis,
   instruments, presets). Make it a `get_guide` topic or drop the reference.
8. **`compare` on two songs says "attack 20027 ms faster"**. For a 22.9 s loop, attack (time to the loudest
   moment) is meaningless, and a 20-second "attack" change in the one-line summary looks like a bug. Leave
   attack out of a song comparison's summary.
9. **"Keep a change only when ... the match went up"** is hard to follow with an opposite text. When my words
   stayed level and the opposite text fell a lot (jump step 2), the rule says undo; the sound had moved where I
   meant. The guide could suggest asking with an opposite text too and reading the gap.
10. **Small things:**
    - The guide's leveling step ("the first edit to a copy should always be to level it by loudness") does not
      say whether it counts as a change; for jump, footstep and missile it changed nothing (presets are already
      at their target), so it is harmless but worth saying.
    - On a two-layer recipe, raising a low layer's volume (footstep heel 0.6 -> 1.5) barely moved anything,
      because `normalize_lufs` turns the whole thing back down and K-weighting counts lows less. The guide
      could warn that with loudness normalising, "louder layer" means "the other layers get quieter".
    - The summary word "bright" stayed on my footstep although its spectrogram is mostly under 1 kHz
      (centroid 3112 Hz, pulled up by noise). Documented (2500 to 6000 Hz is "bright") but surprising for noise.
    - A noise-only sound (footstep before) had a `pitch_track` of [1445, 1445] Hz at tonality 0.0015; a pitch
      for pure noise is misleading.
    - The copied example songs keep comments that state the intended mood, which gives the answer away in a
      blind mood test (fine for people; worth knowing for tests like this one).
    - `edit_history` step numbers are reused after an undo (the undone step 16 and the next new step are both
      16). Not wrong, but confusing when reading the history.

## Files

- `sfx/jump.toml`, `sfx/jump_before.wav`, `sfx/jump_after.wav`
- `sfx/footstep.toml`, `sfx/footstep_before.wav`, `sfx/footstep_after.wav`
- `sfx/missile.toml`, `sfx/missile_before.wav`, `sfx/missile_after.wav`
- `sfx/work/*_cN.wav`: every intermediate render (one per change), kept as evidence
- `music/duck_craft_day.toml/.ogg`, `music/duck_craft_night.toml/.ogg` (unchanged copies)
- `pictures/`: pictures saved by `picture: true` renders
