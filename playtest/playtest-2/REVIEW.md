# Playtest 2: the agent guide

**Question:** can a fresh agent, given only `docs/AGENT_GUIDE.md` and the
Duck Echo MCP server, make a usable coin, hit and explosion, and a
looping eight-bar tune, in one session? (Roadmap task 10.)

**Answer: yes.** All four were made in one session of 38 tool calls,
none of which failed, and each one passes every check the guide gives.
Nobody has listened to them yet.

## How it was run

* The agent was a fresh subagent that had not seen any of this work. It
  was told to read only the guide and to use only the MCP server, and
  not to read the repository or run `duck-echo` directly.
* **The server was reached through a bridge.** The project's MCP server
  could not connect in this session (its first start needs a cold
  release build, which takes longer than the 30 s start timeout).
  `bridge.py` starts the same `duck-echo-mcp` binary with root
  `playtest/`, as `.mcp.json` does, and sends one request per call. The
  tools, their descriptions, the instructions and the errors are what any
  MCP client sees. Every call is in `calls.jsonl`.
* **One caveat, found afterwards:** the server binary was built a little
  before the last edits of the branch's first two commits. It lacked
  three things: the "above 5 kHz is not a pitch" rule in the note check,
  the eighth-beat rounding of `extra_onsets[].beat`, and the line in the
  server's instructions that points at the guide. The agent was handed
  the guide directly, so the last one did not matter. The first explains
  two of the three false extra notes it saw (see below).
* The agent's own notes are in `AGENT_NOTES.md`. It kept its "before"
  renders (`*_base.wav`) and two pictures as evidence.

## What it made

All numbers re-measured with the final code of this branch.

| | Started from | What it changed, and why | Final summary | Calls |
|---|---|---|---|---|
| `sfx/coin` | `preset:coin` | Second note sooner (`step_time` 0.07 to 0.05) for a snappier pickup | 0.31 s, ~-19.0 LUFS, bright, sharp attack, tonal, 988 to 1319 Hz, 2 onsets, no clipping | 9 |
| `sfx/hit` | `preset:hit` | `per_layer` showed its noise layer 14 LU under the tone, so the strike was lost; raised it (`volume` 0.6 to 1.5) | 0.12 s, ~-16.0 LUFS, bright, sharp attack, noisy, no clipping, 2 layers | 7 |
| `sfx/explosion` | `preset:explosion` | Filter cutoff halved (5000 to 2500 Hz) for a deeper boom; brightness went from "bright" to "mid" | 1.12 s, -16.1 LUFS, mid, sharp attack, noisy, no clipping, 2 layers | 13 |
| `music/tune` | the guide's song example | Written from scratch: C major, 140 BPM, C G Am F, C G F G; chip lead tune, pad chords, a bass pattern moved with `transpose`, drums. Raised the tune's volume to 1.4 so it leads | 8 bars looping, 13.71 s, -18.0 LUFS, no clipping, loop joins cleanly, 155 of 155 notes heard | 9 |

How good is it?

* **Every target was met.** Each sound is at its category's loudness (the
  explosion 0.08 LU under: the -1 dBFS peak ceiling stopped it, and the
  render said so). Nothing clips. The tune loops cleanly, is exactly
  604,800 samples, and every one of its 155 notes is heard, with no extra
  notes.
* **The sound effects are careful, small changes.** The agent took "change
  one thing" literally: each is its preset with one or two edits. They are
  usable, but not much more than the presets themselves. Its best move was
  on the hit: it read `per_layer`, saw a buried layer, and fixed it.
* **The tune is the most original piece.** Its melody uses the notes of
  each chord, moves mostly by step, and ends on G to lead back into the
  loop. Whether it is cheerful needs a person to listen.
* **The first try worked.** The song validated and rendered right the
  first time, from the guide's one-screen example alone, without reading
  `get_schema`.

## What confused it, and what was done

Fixed in this branch:

| # | What happened | Fix |
|---|---|---|
| 1 | The guide called the coin example "one fifth higher". B5 to E6 is a fourth. | Guide corrected. |
| 2 | The guide said `loop_seam.clean` is "in the analysis". In a `render` result it sits next to `analysis`, not inside it, so the agent's script found nothing. | Guide says where it is, for `render` and for `analyze`, and the same for `note_check`. |
| 3 | The levelling ops were a bare JSON array; the agent had to guess they are the `ops` of `edit`. | Guide says so. |
| 4 | `compare` called a correlation of -0.86 "partly similar". | The summary now says "upside down" when the correlation is negative (`analyze/src/compare.rs`, with a test). |
| 5 | The guide promised `compare` "says what changed", but it never mentions notes or onsets. | Guide lists what `compare` covers and says to `analyze` both for notes and pitch. |
| 6 | "Soft attack" did not move when the agent shortened the envelope's attack. | Guide explains that `attack_seconds` is the time to the **loudest** moment, and what to read instead. |
| 7 | No tolerance for a loudness target. | Guide: within 0.5 LU is fine. |
| 8 | With every track at volume 1, the pad was louder than the tune. | Guide: the melody should be the loudest pitched track; raise it (1.3 to 1.5). |
| 9 | False extra notes on the tune's bass: two with "pitches" near 14 to 18 kHz, one at 247 Hz. | The first two were already fixed in this branch (the stale binary, above). The 247 Hz one was the bass's resonant filter sweeping past: the note check now ignores a pitch that was already sounding before the onset. A test uses this tune. |
| 10 | Default `beats_per_bar` and stereo output were not stated. | Guide states both. |
| 11 | The copied preset kept the preset's name and description ("Medium explosion."). | Guide says to edit them after `create`. |
| 12 | The guide pointed at files and a CLI flag an MCP agent cannot use. | Guide lists the tools that answer the same questions, marks repository files as such, and gives the MCP form first. |

Bigger ones, now on the roadmap (Phase 2 list under task 10):

* Copying a preset means pasting the whole recipe back into `create`.
  `create` from `preset:NAME`, or `edit` saving to a new path, would be
  one call.
* A song's summary uses sound-effect words that mean little for a mix
  ("pitch rising from 110 to 784 Hz", an onset count that changes with a
  track's volume).
* No number says whether a one-shot sound ends with a click, the way
  `loop_seam` does for loops.
* The reference documents cannot be read through the MCP server. A tool
  that returns the guide and the specs would help agents that are not in
  the repository.

Not tested: the agent made no failing call, so it never saw an error
report, and it met none of the `missing`, `quiet`, `wrong_pitch` or
`uncertain` note statuses. A playtest that starts from broken files would
cover those.
