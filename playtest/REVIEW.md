# Duck Echo playtest 1: retro platformer sound set

**Rating: 8 / 10** as a tool for an agent that cannot hear.

I made eight sounds in `sfx/` (recipe `.toml` plus rendered `.wav` each): jump, land, coin, powerup, enemy_hit, player_death, menu_select, level_complete. All eight validate with no warnings and none clip.

## How I tested (read this first)

- The brief was "MCP tools only; don't read source or docs". My session did not have Duck Echo registered as native tools, so I built the server from the zip and spoke MCP JSON-RPC to it over stdio with a small throwaway script. Every sound was made through the server's nine tools (`get_schema`, `list_presets`, `get_preset`, `validate`, `create`, `edit`, `render`, `analyze`, `compare`, plus `variations` listed but unused).
- What I opened: the directory listing and `.mcp.json` (to know how to launch the server). What I did not open: any source, `README.md`, `CLAUDE.md`, `docs/`, or `presets/*.toml`. Everything I know came from the server's own instructions, tool descriptions, schema and results.
- The repo's `CLAUDE.md` was injected into my context by the harness when I attached the repo, after all sounds were made. I did not open it myself, and it did not shape any sound.
- Untested: `variations`, stereo/pan, `pcm24`/`float32` output.

## What was easy

- **The loop is obvious from the server instructions alone.** `list_presets`, `get_preset`, `create`, `edit`, `render`, `analyze`. I never had to guess the order.
- **Presets are real starting points.** Jump, land, coin, powerup, hit, ui_confirm and extra_life gave me working recipes in one call each. Presets are labelled with a category and a loudness target, which turned out to be the most useful idea in the tool.
- **`render` returns the analysis in the same call**, with a one-line `summary`. That saved a round trip on every iteration.
- **Note names and arpeggios** (`"B5"`, `steps: [0, 5]`) made the chip-tune sounds quick to write. `create` makes missing folders (`sfx/`) itself, and refuses to overwrite without `overwrite: true`.
- **`edit` by path** (`master.normalize_peak_db`, `layers[0].filter`) is pleasant and returns `written: true`.
- **Same seed, same bytes** is stated in the schema, so I could re-render without worrying about drift.

## Errors: which helped

Helped a lot (I probed these on purpose):
- `validate` reports **every problem in one call** with exact paths, the allowed options, and a suggestion. `"sawtooth"` suggested `saw`, `"volum"` suggested `volume`, the note `"H4"` suggested `B4` and explained the note grammar, and `effects[0].type: "chorus"` listed all four real effect types. Out-of-range values gave the range (`0..=1`). It even raised a warning for a suspicious sustain setting.
- `get_preset` on a typo said "Did you mean `jump`?".
- `create` over an existing file named the fix (`pass "overwrite": true`). `..` in an output path was refused with a clear reason.
- `edit` with a bad index said exactly what was wrong ("index 5 is past the end (length 2)").

Helped less:
- **Most failures have `code: "other"`**, and several have `path: ""`, so a program cannot branch on them. `unknown_variant`, `out_of_range` and `invalid_format` are good; the rest are not categorised.
- **The `get_preset` typo error tells me to run `duck-echo presets list`**, a CLI command I cannot reach from MCP. It should say "call `list_presets`".
- **`analyze` on a missing file leaks the server's absolute path** (`/home/.../playtest/sfx/nope.wav`) while other errors use root-relative paths.
- `validate` on a bad recipe returns `isError: false` with `ok: false`, whereas `create` returns `isError: true`. Defensible (validate did its job), but I had to check both flags.
- Neither `render` with no recipe nor `analyze` with a missing file hinted at what `path` values are legal beyond the one-line description.

## What confused me

- **Loudness comparisons are slippery.** The analysis uses `k_weighted_ungated` LUFS for short sounds and `bs1770_gated` for longer ones (the `~` in the summary hints at this). Targets like `-19 LUFS` are therefore not measured the same way for a 0.1 s blip and a 1 s jingle. I treated them as close enough, but a note in the tool description would help.
- **`normalize` only targets a peak, not loudness.** To hit each category's `target_lufs` I rendered, read the LUFS, and nudged `normalize_peak_db` by the difference. It converged in one step each (coin -16.8 to -14.3 and so on), but it is a manual loop for something the tool already knows about.
- **Envelope arithmetic.** Layer length is the sum of five times, and `hold` vs `decay` is not what I would call "sustain". For the jingle I had to add up `hold + decay` so the lead run (5 steps at 0.1 s) would end before the chord at 0.55 s.
- **The summary can mislead for layered sounds.** For `level_complete` it says "slow attack, pitch rising from 131 to 262 Hz". The 131 to 262 Hz is the bass (C3 to C4), the "attack" is the time to the loudest point (the chord at 0.55 s), and the melody is invisible. For `player_death` it says "soft attack" even though I put a noise burst at t=0; `attack_seconds` was 0.055.
- **`pitch_track` is monophonic**: one dominant pitch per window. Fine for a jump, useless for a chord.

## What I wanted but could not do

1. **Solo or mute a layer in `analyze`/`render`** (or an onset list / multi-pitch track). I got around this by analyzing the lead layer as a separate inline recipe, which showed C5, E5, G5, C6, E6 (524, 659, 784, 1046, 1319 Hz) as intended. That workaround worked but I had to build it by hand, and I could not verify the chord notes the same way.
2. **`normalize_lufs` / "level to my category"** (see above).
3. **A note-list or sequence layer** (pitch, start, length, per-note envelope). The arpeggio has one step time and one envelope for all notes, so a real melody needs one layer per note or per phrase.
4. **A file listing tool** for the server root, and a delete. I could not browse `sfx/` through MCP and had to remember what I had created.
5. **Some picture of the sound**: a spectrogram, envelope curve or a coarse "energy over time" list. The pitch track and centroid are good, but they do not show texture such as grit.
6. **A hint about harshness.** Coin was reported at 7.4 kHz centroid ("very bright") with a plain square wave. I cannot tell whether that is real brightness or aliasing from a non-band-limited square. I could not resolve it, so I softened it with a 6.5 kHz lowpass (centroid 3.3 kHz, via `compare`). `powerup` is still at 7.2 kHz and I left it alone.

## How sure am I that each sound is good, without hearing it

These are my guesses about whether each would pass a listen in a game. The checkable parts (pitch direction and range, length, loudness, clipping) were verified by number; the subjective parts were not.

| Sound | Confidence | What I verified | What I cannot tell |
|---|---|---|---|
| jump | 85% | Rises 319 to 784 Hz in 0.19 s, -20.0 LUFS (movement target) | Whether the 0.25 duty timbre feels "boingy" enough |
| land | 70% | Falls 105 to 71 Hz, dark (737 Hz centroid), -19.9 LUFS | At 0.09 s it may read as a tap, not a landing; a longer tail could be better |
| coin | 80% | B5 to E6 (988 to 1319 Hz), -19.0 LUFS | Tone after I softened it; could now be slightly dull |
| powerup | 70% | Rising 540 to 2059 Hz, 1.04 s, -19.0 LUFS | Very bright (7.2 kHz centroid), possibly shrill; echo amount unjudged |
| enemy_hit | 65% | 0.14 s, tone plus noise, -16.0 LUFS (impact target), peak -5.6 dBFS | Whether the bitcrush reads as "crunchy" or "broken"; it is the loudest peak in the set |
| player_death | 70% | Falls 424 to 110 Hz over 0.9 s (A4 to A2 as written), -15.4 LUFS | The vibrato wobble and noise burst are unverified; might sound more like a slide whistle than a death |
| menu_select | 80% | E6 to A6 (1319 to 1760 Hz), 0.12 s, -22.0 LUFS (ui target) | Nothing major; it is simple |
| level_complete | 75% for notes, 50% for feel | Lead run verified separately; bass 131 to 262 Hz; 1.39 s, -19.0 LUFS | Chord (C6/E6/G6) and reverb unverified by number; whether it feels like a win vs a phone ring |

Overall, I would trust the set to be roughly right in pitch, length and level, and I would not ship it without one listen.

## Rating: 8 / 10

It does what it says. The loop is discoverable, the error reports are well above average, and the numbers let a deaf agent get pitch, loudness and clipping right in a few calls. It loses points for monophonic analysis on layered sounds (misleading summaries), peak-only normalisation, no sequence/melody layer, and the small error-message rough edges above.
