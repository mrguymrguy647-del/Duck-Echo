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
* `[master]` has `normalize = true`, `normalize_peak_db = -3` and
  `trim_silence = true`, so every preset peaks at -3 dBFS, never clips and
  ends without silence. Set game mix levels in the game, not here.
* It validates with no warnings, renders clean, and sounds different from
  every other preset.
* Top-level keys (`seed`, `variation`, `channels`) go above `[master]`;
  in TOML every key after a table header belongs to that table.

## Adding or changing a preset

1. Write `presets/<name>.toml` following the conventions.
2. Add the name to the `presets!` list in
   `crates/duck-echo-tools/src/presets.rs`.
3. Render it, check `duck-echo analyze preset:<name>` reads as intended, and
   add its fingerprint to `PRESET_GOLDEN` in the presets test (the failing
   test prints the new value). Changing an existing preset's sound means
   updating its fingerprint on purpose, and saying so in the commit.
