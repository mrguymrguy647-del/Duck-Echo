# Presets

Starting-point recipes, compiled into `duck-echo` and the MCP server so
they work from any directory. Use one anywhere a recipe path goes:

```bash
duck-echo presets list --tag impact
```

```bash
duck-echo render preset:coin -o coin.wav
```

```bash
duck-echo presets show coin > my_coin.toml
```

## Conventions

Tests in `crates/duck-echo-tools/tests/presets.rs` enforce these:

* The file name is the recipe's `name`.
* `description` is one sentence; `tags` starts with exactly one category:
  `ui`, `pickup`, `weapon`, `impact`, `movement`, `alert` or `ambient`.
  More descriptive tags may follow.
* It is levelled to its category's loudness target (below), within
  0.25 LU: `[master]` has `normalize = true` and a `normalize_peak_db`
  chosen so the measured loudness hits the target, never above -1 dBFS.
  It also has `trim_silence = true`, so it ends without silence.
* It validates with no warnings, renders clean, and sounds different from
  every other preset.
* Top-level keys (`seed`, `variation`, `channels`) go above `[master]`;
  in TOML every key after a table header belongs to that table.

## Loudness targets

Presets are levelled by loudness, not peak, so they sit together in a game
without one category drowning another. Loudness is what `duck-echo
analyze` reports: gated BS.1770 for clips of 400 ms or more, the ungated
short-clip method below that.

| Category | Target | Why |
|---|---|---|
| impact | -16 LUFS | Loudest category. An explosion peaks about 14 dB above its loudness, so -16 LUFS is as loud as it can be with 1 dB of peak headroom. |
| weapon | -17 LUFS | Just under impacts: the shot should not hide what it hits. |
| alert | -18 LUFS | Must be noticed over gameplay, but not startle. |
| pickup | -19 LUFS | Rewarding but frequent; slightly under alerts. |
| movement | -20 LUFS | Repeats constantly (steps, jumps); kept modest. |
| ui | -22 LUFS | Feedback that should never mask the game. |
| ambient | -24 LUFS | Beds that sit under everything. |

These are starting levels, not a final mix; set mix levels in the game.
At peak normalization alone, the same presets ranged from -4 LUFS (alarm,
ui_error) to -17.5 LUFS (explosions), 13 dB of difference the wrong way
round. The targets are in `duck_echo_tools::presets::LOUDNESS_TARGETS` and
`list_presets` reports each preset's `target_lufs`.

Presets are levelled by peak (`normalize_peak_db`), so an edited preset
drifts off its target. To keep the target through edits, set
`master.normalize_lufs` to the category target (for example `-19` for a
pickup): it levels the sound by loudness in every render, with the peak
kept under -1 dBFS. `normalize` and `normalize_peak_db` then have no effect,
and validation warns about `normalize = true` so it can be removed. The
presets themselves keep their peak settings so their pinned renders stay
the same; a test checks that each one lands on its target with
`normalize_lufs` added.

## Adding or changing a preset

1. Write `presets/<name>.toml` following the conventions.
2. Add the name to the `presets!` list in
   `crates/duck-echo-tools/src/presets.rs`.
3. Set `normalize_peak_db`: the presets test fails with the value that
   hits the category target (`set normalize_peak_db to about -9.4`). Run it
   again after changing it, since trimming can shift a tail-heavy preset a
   little.
4. Render it, check `duck-echo analyze preset:<name>` reads as intended, and
   add its fingerprint to `PRESET_GOLDEN` in the presets test (the failing
   test prints the new value). Changing an existing preset's sound means
   updating its fingerprint on purpose, and saying so in the commit.
