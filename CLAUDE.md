# Duck Echo: rules for builders

Read `docs/ARCHITECTURE.md`, `docs/RECIPE_SPEC.md` and `docs/ROADMAP.md`
before changing anything.

## Non-negotiable

- The recipe is the source of truth. Never add behaviour that is not
  expressed in the recipe or in a documented CLI/tool option.
- Renders are deterministic: same recipe + same seed = identical bytes. No
  wall clock, no global state, no hash-map iteration, no platform libm on
  the audio path (use `duck_echo_synth::math`).
- TOML files and JSON tool calls deserialize into the same Rust types.
  One schema. Unknown fields are errors, never ignored.
- Every error carries an exact field path and, where possible, the allowed
  range or options and a suggestion. Agents read errors as JSON.
- Nothing is silently deferred. If a field exists in the schema it works;
  if it does not work yet it is not in the schema and is listed in
  `docs/RECIPE_SPEC.md` under Deferred.
- `Voice::process` must stay allocation-free and lock-free.
- Keep dependencies minimal; justify each new one in `ARCHITECTURE.md`.
- Never implement voice cloning or anything that imitates a real person.
- The CLI and the MCP server share their operations through
  `duck-echo-tools`; add behaviour there, not in one front end only.
- Renders of every example and preset are pinned by fingerprint tests.
  Change a fingerprint only when the sound is meant to change, and say so
  in the commit.

## Workflow

- `cargo test --workspace`, `cargo clippy --workspace --all-targets` and
  `cargo fmt --all -- --check` must pass before a commit.
- After changing recipe types: `cargo run -p duck-echo-cli -- schema >
  schema/recipe.schema.json` and update `docs/RECIPE_SPEC.md`. After
  changing song types: the same with `schema song >
  schema/song.schema.json`.
- New features get an example recipe or a test that renders it.
- Keep `docs/ROADMAP.md` current: tick off what you did, add what you
  learned the next builder needs.
