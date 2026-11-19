# Phase 4 design: the Duck Ear

Status: **draft, waiting for approval.** Nothing in it is built yet.

## 1. What we want, and why

Playtest 4 ended with an agent saying: "Duck Echo cannot say whether the
tune is good, and neither can I: a person should listen." Every number
Duck Echo gives (loudness, feel, notes heard, the mix check) says what a
sound *measures*, not what it *sounds like*.

The goal of Phase 4: an agent asks **"does this sound like a soft coin
pickup?"** or **"is this tune calm?"** and gets an honest answer, with a
score and a reason, so it can fix a sound without a person listening
every time.

* The numbers stay. The Ear adds listening on top of them. It never
  replaces a number.
* The Ear is **advice, not truth**. It can be wrong. Its answers carry a
  score, its models' names and a plain "unsure" when it is unsure.
* The Ear is **optional**. Every part of Duck Echo works without it. A
  render never waits for it, never changes because of it, and its answers
  are never written into a sound file or its facts.
* The Ear runs **on your own PC**. When it listens, nothing is sent to
  the internet. It downloads its programs and models once, when you ask
  it to, and never again until you ask for an update.
* You can **rate sounds** (thumbs up or down) in the Studio, so we can
  measure how often the Ear agrees with you, and later teach it your
  taste.

```
   agent / person
        │  "does this sound like a soft coin pickup?"
        ▼
 ┌──────────────┐  render (deterministic, as today)   ┌──────────────┐
 │  Duck Echo   │ ──────────────────────────────────▶ │  coin.wav    │
 │ (Rust: CLI,  │                                     └──────┬───────┘
 │  MCP, Studio)│  JSON Lines over stdin/stdout              │ path
 │              │ ◀────────────────────────────────▶ ┌──────▼───────┐
 │ duck-echo-ear│   {"id":3,"method":"score",...}     │  duck-ear    │
 │  (the bridge)│   {"id":3,"result":{...}}           │ (Python, on  │
 └──────────────┘                                     │  your PC)    │
        │                                             │  ├ CLAP model│
        ▼                                             │  └ audio LLM │
  analyze / listen answer:                            └──────────────┘
  numbers (as today) + "ear": {score, verdict, reason, models}
```

## 2. Where we start

* `docs/ARCHITECTURE.md` ("Duck Ear bridge (design)") already sets the
  shape: a Python program `duck-ear`, started by Duck Echo, JSON Lines over
  stdin and stdout, files passed by path, `ear: {"available": false}` when
  it is missing, and nothing in core, synth or analyze knows it exists.
  This design keeps all of that and fills it in.
* `docs/ROADMAP.md` task 14 says: the protocol and a stub `duck-ear`
  first, with tests for the handshake, a timeout, a missing program and a
  broken line; then the `ear` section in `analyze`; then real models.
* What agents have today: `analyze` (summary, `feel`, `expect`,
  `note_check`, `loop_seam`), `compare`, `set_check`, `mix_check`. The
  `expect` table checks numbers only (`roughness = "high"`,
  `loudness_lufs = [-20, -16]`). Songs have no `expect` yet.
* The Studio already downloads a file safely: `studio/crates/app/src/update.rs`
  fetches a release file and its `SHA256SUMS` and refuses a file whose
  SHA-256 does not match. The Ear's installer reuses that idea.

DESIGN-SECTIONS-GO-HERE
