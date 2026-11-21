# Playtest 5: review

Phase 4, step 8. A fresh agent, with only the guide (`get_guide`) and the
MCP server, used the real Duck Ear to improve three sounds and to judge
two tunes' mood.

## How it was played

* The Ear: the real one, installed by the person with `duck-echo ear
  install` from the release `ear-score-v1` (LAION-CLAP, scores only, on
  the CPU), in the usual Ear folder. The agent never installed anything.
* The server: `target/release/duck-echo-mcp --root playtest/`, through
  `bridge.py` (one session per call). Every call is in `calls.jsonl`;
  every answer from the Ear is whole in `ear_answers.jsonl` (70 answers).
* The task: start with `preset:jump`, pick two more sounds the Ear is
  unsure about, improve each one change at a time (at most five), and
  judge the mood of `example:duck_craft_day` and `example:duck_craft_night`.
* 157 tool calls in about 13 minutes, none failed: 69 `listen` (4.4 s
  each on average, the Ear started anew for each call), 24 `render`, 20
  `compare`, 15 `sound_set`, 7 `edit_undo`.
* The agent's own account is `AGENT_NOTES.md`. Its recipes are in `sfx/`
  (`<name>.toml`, `<name>_before.wav`, `<name>_after.wav`), every
  in-between render in `sfx/work/`, the songs in `music/`.

## What happened

The agent first asked the Ear about every preset with the words it meant
for each (a table in its notes), and picked the two lowest it thought a
model could hear: the footstep (unsure, 0.48) and the missile (unsure,
0.35). It left out `ui_cancel` and `key_pickup` (both "no"), judging
that "two falling notes for cancel" and "picking up a key" are not things
a sound model can hear: a good call.

| Sound | Its words | Ear before, after | Changes kept, undone | After, by the numbers |
|---|---|---|---|---|
| jump | "a light, springy jump in a cartoon platform game" | yes 0.90, yes 0.93 | 3, 2 | 0.20 s, -20.0 LUFS, peak -11.2 dBFS, pitch 294 to 989 Hz, no clipping |
| footstep | "a single footstep on a hard floor" | unsure 0.48, yes 0.88 | 3, 2 | 0.04 s, -20.0 LUFS, the heel 1 LU under the scuff (was 12), no clipping |
| missile | "a missile launching with a whoosh" | unsure 0.35, unsure 0.47 | 2, 3 | 0.95 s, -17.0 LUFS, peak -6.1 dBFS, the whoosh now leads, no clipping |

The tunes: it did not change them. By the numbers it found both calm (84
BPM, few notes per beat, a steady level, low roughness), the day tune
happy (C major, brighter, a high melody) and the night tune melancholy (A
minor with G#, 34% darker, a low melody). The Ear said yes to every mood
on both songs (0.82 to 1.00); only its comparisons pointed the same way
as the numbers (day more "happy, cheerful" by 0.38, night more "scary"
by 0.08). It noted that the copied songs' comments name their mood, so
its judgement was not blind.

## What the Ear helped with

* **Steering, where the numbers do not rank.** On the footstep it heard
  "a hand clap" and "static" in the preset, which agreed with the
  numbers (bright, the heel 12 LU under the scuff). It rewarded a darker,
  tighter scuff strongly (0.48 to 0.88), and the agent checked that this
  held over five seeds (preset 0.38 to 0.48, its version 0.76 to 0.99).
* **As a veto.** It marked a longer footstep decay (0.32, no) and two
  missile changes as worse, where no number objected. Without it the
  agent might have kept them.
* **The gap to an opposite text.** On the jump, the agent's own words
  hardly moved (0.90 to 0.93, "about the same"), but "a heavy jump" fell
  from 0.84 to 0.64, which matched the faster, higher rise the numbers
  showed.

## Where it misled the agent, and whether the numbers caught it

* **A yes to opposites.** "a heavy jump" 0.84 and "a soft, bouncy jump"
  0.94 on the same sound; footstep, hand clap and static all yes on the
  final footstep. A single yes means little. The numbers did not need to
  catch this: the agent stopped trusting single yeses.
* **An octave jump it did not hear.** Jump change 5 (more resonance)
  scored 0.92, about the same as before, but the pitch track jumped to
  1971 Hz in the tail. **The numbers caught it**; the change was undone.
* **Music.** Every mood was yes on both songs. The numbers (and the
  song's own notes) decided the mood; the Ear only agreed in its
  comparisons.
* **Big swings for small changes.** The footstep went 0.71, 0.32, 0.85 for
  scuff decays of 50, 90 and 35 ms. The agent checked with several seeds
  before believing it; that is the right habit, and now the guide's
  advice to ask about an opposite text helps here too.
* **A missile it never names.** It hears "rushing air rising in pitch"
  (0.96) but never "a missile" or "a rocket". No change got the missile
  past unsure; every change the agent kept was also sensible by the
  numbers, so nothing was broken to please the Ear.

In two places the agent trusted the numbers over the Ear (jump change 2,
footstep change 5), each time for a reason the numbers showed. It never
broke a number to raise a match: no clipping, loudness kept at the
category's target, every sound ends cleanly.

## Found and fixed

1. **`listen`'s description promised a describe model.** It said each
   text gets "the describe model's yes or no with a reason" and that both
   models must agree. It now says the Ear ships with a score model only,
   and what changes when a describe model is there.
2. **A question that could not be answered said "heard".** With scores
   only, the summary now says "the question was not answered (scores
   only: no describe model)".
3. **The Ear's own words pointed agents at a developer's script**
   (`ear/tools/get_describe_model.py`). They now say to ask with texts in
   `like`, and that only a person can try a bigger model, in the Studio.
4. **`compared.by` was 0.0 under 0.05**, hiding which way it moved. It is
   now the real difference; `more_like` still says "about the same", and
   the summary gives both matches.
5. **The guide named mood numbers that do not exist** (a key, notes per
   beat). It now names what there is: the song's tempo and notes, the
   scale the notes use, the melody's height, `spectral_centroid_hz` and
   `loudness_lufs`.
6. **The guide undersold music.** It now says the Ear may say yes to every
   mood on a whole song, and that only the difference between two songs
   on one text means anything.
7. **The yes and no lines were not in the guide.** They are now: yes from
   0.66, no below 0.34.
8. **What to compare with after the first change, and opposite texts.**
   The guide now says to render each kept version over the "before" file,
   and to ask about an opposite text and read the gap.
9. **`docs/ear-check.md` could not be read through the tools.** It is now
   the guide page `ear_check` (`get_guide` with `topic: "ear_check"`).
10. **`compare` on two songs said "attack 20027 ms faster".** Attack is
    left out of the summary when either sound's loudest moment comes
    after 1 s (the number stays in `delta`).
11. **Levelling and louder layers.** The guide now says the levelling
    setup is not one of the changes, and that with loudness normalising,
    a louder layer makes the others quieter (a low layer counts for less).

Each fix has a test: `agent_guide.rs` (the chapter's words, numbers and
the `ear_check` page), `ear.rs` in `duck-echo-tools` (the summary for a
question with scores only, and "about the same" with both matches),
`features.rs` in `duck-echo-analyze` (no attack words for two tunes).

## Not fixed (noted for later)

* **A pitch for pure noise.** The noisy footstep had a `pitch_track` of
  1445 Hz at a tonality of 0.0015. It changes the analysis, so it is for a
  later step with its own tests.
* **`edit_history` numbers a new step like the one undone** (both 16).
  Not wrong, but confusing to read.
* **"bright" on a footstep whose energy is mostly under 1 kHz**: the noise's
  high tail pulls the centroid up. As documented.
* **No `key` or notes-per-beat in a song's analysis.** The agent worked them
  out from `note_check`. Worth adding if agents keep judging mood.
* **The Ear's limits** (yes to opposites, no word for a synthesized
  missile, music saturating): these are the score model's, and the guide
  now says how to work with them. A describe model that passes its test,
  or the Large one on a big graphics card, is the way past them.

## Listening blind

`blind/` has each sound as `A` and `B` in an order that changes from
sound to sound, and `blind/KEY.md` says which is which. The person's
verdict is the real test: whether the after versions are better, by ear.
