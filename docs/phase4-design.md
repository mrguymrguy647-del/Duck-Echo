# Phase 4 design: the Duck Ear

Status: **approved**, with questions 1 to 8 as recommended and three
changes, written in below: the same question gets the same answer (a
cache), `llama-server` stops after 2 minutes with no describe question,
and the Ear ships with scores only if no describe model passes its test.
Done so far: steps 1 (the bridge and the stub), 2 (what agents get, with
the stub) and 3 (the score model: LAION-CLAP was chosen, see
`docs/ear-check.md`).

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

## 3. The bridge (task 14) [Rust: `duck-echo-ear`]

### Who starts the Ear, and how long it lives

Loading a model takes seconds (and the big one, tens of seconds), so the
Ear is **started once and kept**, not started for every question.

* **The MCP server and the Studio** each start the Ear the first time
  they need it, and keep it by the same two rules:
  * **The Ear and its score model stay for 10 minutes** with no question.
    Then Duck Echo stops the Ear. (Duck Echo keeps this clock.)
  * **`llama-server`, with the describe model, stays for 2 minutes** with
    no describe or ask question. Then the Ear stops it and the graphics
    card is free again, while the Ear and its score model stay. The next
    describe question starts it again. (The Ear keeps this clock; Duck
    Echo sends the 120 seconds in `hello`, so both numbers live in one
    table in Duck Echo.)
* Because the describe model may have been stopped, Duck Echo sends
  `load` before every `describe` or `ask`. When the model is still
  running, `load` answers at once; when it was stopped, starting it again
  happens under `load`'s own, longer time limit, never under the
  question's.
* **The command line** (`duck-echo listen`, `duck-echo analyze --ear`)
  starts the Ear, asks, and stops it. Slow for many calls, but simple; an
  agent that listens a lot uses the MCP server.
* The Studio starts its Ear for Settings > Duck Ear > Test and for
  measuring your ratings (section 7). Your ratings themselves never need
  the Ear.
* A new crate, **`crates/duck-echo-ear`**, does only this: start the
  program, speak the protocol, keep time, stop it. It knows nothing about
  recipes. `duck-echo-tools` uses it, so the CLI, the MCP server and the
  Studio share one way of asking (the CLAUDE.md rule). It uses
  `serde_json` and the standard library's `process` and `thread`, plus,
  on Windows only, `windows-sys` (already in `Cargo.lock` through other
  crates) for the job object that stops the Ear's own children with it
  (section 5). That is justified in `ARCHITECTURE.md` in step 1.

```
duck-echo-tools ──> duck-echo-ear ──spawn──> duck-ear (Python)
 (listen, analyze)    │  writer thread: requests ──▶ stdin
                      │  reader thread: stdout ──▶ answers, by id
                      │  stderr thread: ──▶ ear.log (last 1 MB kept)
                      └  a clock for every request
```

Three threads read and write, so the Ear can never block Duck Echo by
filling a pipe (a program that writes a lot to stderr while nobody reads
it stops, on Windows and Linux both). Every wait has a deadline.

### The protocol: JSON Lines, version 1

One JSON object per line, UTF-8, `\n` at the end (a `\r\n` from Windows
is accepted). Requests go to the Ear's stdin, answers come back on its
stdout, matched by `id`. stdout carries nothing else; the Ear's logging
goes to stderr. Audio is passed **by path**, never as bytes.

```
→ {"id":0,"method":"hello","params":{"protocol":1,"describe_idle_seconds":120}}
← {"id":0,"result":{"protocol":1,"ear_version":"0.1.0",
     "capabilities":["score","describe","ask"],
     "device":"vulkan","device_name":"NVIDIA GeForce RTX 4060 (8 GB)",
     "models":[{"role":"score","name":"...","revision":"...","loaded":false},
               {"role":"describe","name":"...","revision":"...","loaded":false}]}}

→ {"id":1,"method":"load","params":{"roles":["score"]}}
← {"id":1,"progress":{"message":"loading the score model","done":0.5}}
← {"id":1,"result":{"loaded":["score"],"seconds":3.1}}

→ {"id":2,"method":"score","params":{"wav":"C:\\...\\tmp\\a1.wav",
     "texts":["a soft coin pickup"]}}
← {"id":2,"result":{"model":"...","seconds":0.4,
     "scores":[{"text":"a soft coin pickup","match":0.81,"rank":2,"of":120}],
     "closest":[{"text":"coin pickup","match":0.86},{"text":"bell ding","match":0.71}]}}

→ {"id":3,"method":"ask","params":{"wav":"...","question":
     "Does this sound like a soft coin pickup? ...","answer":"yes_no"}}
← {"id":3,"result":{"model":"...","answer":"yes",
     "reason":"A short, bright two-note chime with a quick fade.","seconds":4.2}}

→ {"id":4,"method":"ask","params":{"wav":"...","question":
     "Is this tune calm?","answer":"free"}}
← {"id":4,"result":{"model":"...","text":"Mostly: slow and soft, but the drums ...","seconds":5.0}}

→ {"id":5,"method":"describe","params":{"wav":"..."}}
← {"id":5,"result":{"model":"...","text":"A short bright chime ...","seconds":3.9}}

→ {"id":6,"method":"bye"}
← {"id":6,"result":{}}            (then the Ear exits)
```

* **Errors:** `{"id":2,"error":{"code":"unreadable_audio","message":"..."}}`.
  Codes: `unknown_method`, `bad_params`, `unreadable_audio`,
  `model_missing`, `out_of_memory`, `internal`. Duck Echo passes the code
  and message on, under the `ear` section's `error`.
* **Progress** lines (`"progress"` instead of `"result"`) may come before
  the answer. They are shown in the Studio; they **do not** give the
  request more time.
* **Versions:** `hello` sends `protocol: 1`. An Ear with another protocol
  number answers `{"error":{"code":"protocol"}}` and Duck Echo reports it
  as not available, "update the Ear". Inside version 1, an answer field
  Duck Echo does not know is skipped, so an Ear can add fields; a missing
  field that Duck Echo needs is a broken answer. (Recipes and tool calls
  stay strict as always; this rule is only for the line between the two
  programs, because an installed Ear and Duck Echo are updated at
  different times.)
* **Size:** a line over 1 MB is a broken answer. Nothing we send needs
  more.
* **Paths:** Duck Echo writes the WAV it wants heard into a folder of its
  own (the system's temporary folder, `duck-echo-ear-<process id>`), and
  deletes it when the Ear stops. The Ear only reads the paths it is
  given; it writes nothing but its log and its model cache.

### Same question, same answer: the answer cache

Asking the Ear the same thing twice gives the identical answer:

* The describe model runs with **temperature 0 and a fixed seed** (42),
  so it always picks its most likely words, the same way each time.
* Duck Echo keeps **every answer it gets** in a cache in the Ear's folder
  (`ear/answers/`, section 5). The key is the SHA-256 of the sound file,
  the question (the method and its parameters, with the sound's path left
  out), the Ear's version and the revision of every model it reported in
  `hello`. Before asking, Duck Echo looks in the cache; a hit is answered
  at once, without loading a model. (Duck Echo still greets the Ear
  first, which takes well under a second, so the revisions are always
  the Ear's own: step 3 found that a remembered `hello` gave one model's
  answers for another.)
* Every answer says where it came from: `"cached": true` or `false`.
* Only answers are kept, never errors or timeouts: those are asked again.
* A new model or Ear version gives new keys, so it is asked afresh, and
  the old answers stay for comparing.
* One small JSON file per answer (`answers/3f/3f1c....json`, named by the
  key's SHA-256), written whole and then renamed, so two Duck Echo
  programs can share the cache safely. Settings shows its size and has
  **Clear answers**; Remove deletes it with the Ear.
* The cache lives in Duck Echo (the bridge), not in the Ear, so the stub
  is cached the same way and the test "ask twice, get the same answer,
  the second time `cached: true`" runs without models.

### Timeouts: Duck Echo never waits longer than this

| Request | Time allowed | Why |
|---|---|---|
| start + `hello` | 20 s | Python's first start on Windows with antivirus scanning can take 10 s |
| `load` score model | 60 s | measured in step 3; under 1 GB from disk |
| `load` describe model | 180 s | measured in step 4; several GB |
| `score` | 15 s | one clip, ~1 s on CPU |
| `ask`, `describe` | 60 s on a graphics card, 240 s on CPU | measured in step 4 |
| `bye` | 3 s | then the process is killed |

* The times are in one table in `duck-echo-ear`, and one setting changes
  them all, for a slow PC: the environment variable
  `DUCK_ECHO_EAR_TIMEOUT_SCALE` (2 doubles them; 0.25 to 10). No value
  makes them endless. (A command-line option for it was planned here and
  left out in step 1: the variable reaches the CLI, the MCP server and
  the Studio alike.)
* **When time runs out:** Duck Echo kills the Ear (it may be stuck),
  answers with `ear: {"available": true, "ok": false, "error":
  {"code": "timeout", "message": "the Ear took longer than 60 s to
  answer ask; it was stopped. It starts again on the next question."}}`,
  and the rest of the answer (the numbers) is there as always.
* A render, `validate`, `analyze` without `ear`, playing a sound, the
  Studio's screens: **none of them ever calls the Ear.** They cannot wait
  for it.

### A missing or broken Ear

Every way the Ear can fail ends the same way: the `ear` section says so,
with a reason and what to do, and everything else in the answer is as it
always was.

| What happened | `ear` says |
|---|---|
| Not installed | `{"available": false, "reason": "not_installed", "message": "The Duck Ear is not installed. Ask the person to install it: Duck Echo Studio > Settings > Duck Ear, or duck-echo ear install."}` |
| Installed, but the program will not start | `reason: "will_not_start"`, with the last lines of `ear.log` |
| No `hello` in time | `reason: "no_hello"` |
| Other protocol version | `reason: "protocol"`, "update the Ear" |
| A line that is not JSON, or an answer with a wrong or missing `id` | the Ear is stopped; `ok: false`, `error.code: "broken_answer"`, the first 200 characters of the line |
| The Ear exits in the middle | `ok: false`, `error.code: "ear_stopped"`, its exit code and log lines |
| A model is missing or damaged | `ok: false`, `error.code: "model_missing"`, "Repair in Settings" |
| Out of memory on the graphics card | the Ear tries the CPU once by itself; if that fails, `out_of_memory` |

`available: false` means "you cannot ask the Ear on this PC now".
`ok: false` means "it is there, but this question failed".

### Where Duck Echo looks for the Ear

In order: `--ear-program PROGRAM`, the one spelling for "which Ear
program" on the MCP server and the command line (`analyze --ear` is a
switch meaning "ask the Ear too"; the old `--ear PROGRAM` still works,
with a note); the environment variable `DUCK_ECHO_EAR`; the installed Ear
(section 5). These are how tests point at the stub, and how a developer
points at a `duck-ear` they are working on.

### The stub `duck-ear`

`ear/stub/duck_ear_stub.py`: Python's standard library only, no models,
about 100 lines. It answers `hello` and `load` at once, `score` with a
fixed table (a text containing "coin" scores 0.9, anything else 0.1),
`describe` with "A stub Ear: no model listened to this.", `ask` with
"unsure". Its answers are marked `"stub": true`, and so is the `ear`
section, so nobody mistakes it for listening.

For the hard cases the tests also use **a small Rust program** in
`crates/duck-echo-ear/tests/` (built by Cargo, so CI needs no Python for
it), told by its first argument how to misbehave: `silent` (never says
hello), `slow` (answers after 30 s), `garbage` (prints a line that is not
JSON), `wrong_id`, `exit_midway`, `stderr_flood` (writes 50 MB to stderr
before answering), `huge_line` (2 MB), `old_protocol`.

## 4. Listening models

### How this was checked

Web searches on 2026-11-19, by three research agents, plus direct reads of
GitHub pages and PyPI listings. Hugging Face and arXiv pages could not be
opened from the build machine, so numbers from model cards and papers
come from search summaries of those pages. Sizes and speeds marked
**(est.)** are estimates, not published figures. Steps 3 and 4 measure
every one of them on our own sounds before anything ships.

Two jobs, two kinds of model:

* **Score**: "how well does this text fit this sound?" A **CLAP-style**
  model turns a sound and a text into two lists of numbers (embeddings),
  and their closeness is the match. Small and fast; good at *what kind of
  sound* it is, weak at fine words like "soft".
* **Describe and answer**: "describe this sound", "is this tune calm, and
  why?". An **audio language model** hears the sound and writes text. Big
  and slow; can explain itself; can also make things up.

The Ear uses both and lets them check each other (section 6, `verdict`).

### Score models compared

| Model (maker, date) | Licence (code / weights) | Free tool, used commercially? | Size | Speed (est.) | Short sounds and music | Notes |
|---|---|---|---|---|---|---|
| **CLAP2** S / M / L (LAION, Oct–Nov 2026) | MIT / Apache-2.0 | yes | 104M / 153M / 340M params; ~0.4 / 0.6 / 1.4 GB fp32 | S/M under 1 s per sound effect on a 4–8 core CPU; L 1–3 s for 20–30 s of music; tens of ms on a graphics card | takes any length (no 10 s window), so a 0.2 s coin stays a 0.2 s coin; trained on clips up to 30 s; 45% of its training audio is speech; no music numbers yet | best broad scores of the open small models (MAEB-30 53.6 vs 35.2 for LAION-CLAP); only weeks old; ONNX export is a community one |
| **LAION-CLAP** `larger_clap_general` (LAION, 2023) | CC0 / Apache-2.0 | yes | ~200M; ONNX 118 MB audio + 502 MB text (fp32), 34 + 127 MB quantized | 0.2–0.6 s per 10 s window on CPU | 48 kHz, 10 s windows: a short sound must be padded with silence (not repeated, the default); music in 10 s windows, averaged | years of use; ready ONNX and OpenVINO builds; ESC-50 zero-shot ~91% |
| **PE-AV / PE-A-Frame** (Meta, Dec 2025) | Apache-2.0 / Apache-2.0 | yes | 450M to 2B | too slow on CPU for music; fine on a graphics card | best published retrieval (AudioCaps 45.8); can also say *when* a sound happens | a later "high quality" choice for graphics cards |
| MS CLAP 2023 (Microsoft) | MIT / MS-PL | yes | 690 MB | | ESC-50 93.9% | repository archived Sep 2026: no |
| M2D-CLAP (NTT) | evaluation-only licence | **no** | | | | excluded |
| MuQ-MuLan (Tencent) | MIT / CC-BY-NC-4.0 | **no** | | | music only | excluded |
| GLAP (Xiaomi) | Apache-2.0, but its text part comes from Meta's SONAR (CC-BY-NC) | **unclear** | | | | excluded until clarified |

**A score is not a probability.** Studies in 2025–2026 agree that the raw
closeness of a CLAP model matches people's ratings only weakly, and that
music models act like a "bag of words" (they miss small changes in a
sentence). So the Ear never shows the raw number. See "Scores you can
read" below.

**Recommendation: CLAP2-S (every PC), with LAION-CLAP measured beside it
in step 3.**

* Why CLAP2: a clean licence, the best results of the small models, and it
  hears a 0.2 s sound as it is (no padding to 10 s), which is what game
  sounds are. The small size is fast on any CPU, so the score model never
  needs a graphics card.
* Why measure LAION-CLAP too: CLAP2 is weeks old and its numbers are its
  authors' own. Step 3 runs both on our test set; the better one on *our*
  sounds ships. LAION-CLAP is the safe fallback.
* **We convert it to ONNX ourselves, once**, and check the converted model
  gives the same embeddings as the original (closeness 0.999 or more on
  every test sound). Then the Ear runs it with ONNX Runtime (17 to 28 MB)
  and never needs PyTorch (126 MB on Windows, about 3 GB on Linux with
  CUDA). The converted file is published with Duck Echo's releases, with
  its licence (Apache-2.0 allows it).

### Describe models compared

| Model (maker, date) | Weights licence | Free tool, used commercially? | Size at 4 bits (est.) | Runs in | Sound / music (MMAU) | Notes |
|---|---|---|---|---|---|---|
| **Qwen2.5-Omni-7B** (Alibaba, Mar 2025) | Apache-2.0 | yes | 5–6 GB GGUF Q4_K_M + audio part | **llama.cpp** (official GGUF from ggml-org), transformers | 67.9 / 69.2 | follows free questions well; fits an 8 GB graphics card |
| **MiDashengLM-0.6B** (Xiaomi, Apr 2025) | Apache-2.0 | yes | under 1 GB GGUF | llama.cpp (Xiaomi says it runs on CPU) | no scores published | small enough for any CPU; must prove itself on our test set |
| MiDashengLM-7B (Xiaomi, Oct 2025) | Apache-2.0 | yes | ~6 GB (GPTQ 4-bit) | transformers, vLLM; no GGUF | 66.3 overall; the best describer of sounds and music (AudioCaps 62.2, MusicCaps 59.7) | needs PyTorch and CUDA: a later option |
| Qwen3-Omni-30B-A3B (Alibaba, Sep 2025) | Apache-2.0 | yes | ~17–20 GB GGUF Q4 | llama.cpp, transformers | 77.5 overall, the best | too big for 8 GB; a later option for 12 GB + 32 GB RAM |
| Step-Audio 2 mini (StepFun, Aug 2025) | Apache-2.0 | yes | ~16 GB at fp16, no GGUF | transformers, vLLM (Linux) | 76.6 / 71.6 | strong, but no simple Windows or 8 GB path |
| Gemma 4 E2B / E4B (Google, 2026) | Apache-2.0 | yes | small | llama.cpp | speech scores only | easy to run; sounds and music unproven; second CPU candidate |
| Kimi-Audio-7B (Moonshot, Apr 2025) | MIT | yes | ~24 GB at fp16 | own code | 74.2 / 61.1, but a poor describer (AudioCaps 49.0) | no |
| Audio Flamingo 3 / Next (NVIDIA, 2025–26) | NVIDIA non-commercial | **no** | | | 73.1 | excluded |
| Qwen2.5-Omni-3B (Alibaba) | Qwen Research Licence | **no** | | | | excluded |
| Voxtral, Phi-4-multimodal, Nemotron Omni | various | | | | built for speech | not for sound effects |

Speed (est., nothing published for one short clip): a 7B model at 4 bits
in llama.cpp, one clip of up to 30 s, about 150 words out:

* **2 to 5 s** on an RTX 3060 or 4060;
* **30 to 90 s** on an 8-core CPU;
* a 0.6B model on CPU: a few seconds.

**Recommendation: Qwen2.5-Omni-7B (Q4_K_M, from ggml-org) for a PC with
a graphics card of 8 GB or more; MiDashengLM-0.6B for every other PC,
if it passes step 4's check (Gemma 4 E2B if it does not).**

* Why Qwen2.5-Omni-7B: Apache-2.0, good published results on sounds *and*
  music, answers free questions (which "does this sound like ...?" needs),
  and runs in **llama.cpp**, a single program with ready builds for
  Windows and Linux. Only Qwen3-Omni is clearly better, and it is four
  times the size.
* Why llama.cpp: one `llama-server` program (23 MB for CPU, 36 MB for
  **Vulkan**, which uses NVIDIA, AMD and Intel graphics cards alike)
  instead of PyTorch with CUDA (2.5 to 3.5 GB). It has an `--offline`
  switch and listens only on this PC (127.0.0.1).
* Without a graphics card the 7B model still works, just slowly (the 240 s
  timeout in section 3 allows for it), and a person can choose it.

### Scores you can read

What the agent sees as `match` (0 to 1) is worked out like this, and the
same way for every sound, so two scores can be compared:

1. The score model gives the closeness of the sound to the asked text
   **and** to every word of the Ear's list (about 120 game-sound and music
   words, section 6).
2. `match` comes from the text's **z-score**: how far its closeness to
   this sound is above its usual closeness to Duck Echo's own sounds (the
   presets and examples, saved with the model), put through a logistic
   curve so it runs from 0 to 1 and is 0.5 halfway between the yes and no
   lines. It means: *how much more this text fits this sound than it fits
   Duck Echo's sounds in general.* `rank` says where the text came among
   the words. (Step 3 first tried a softmax over the words, as planned
   here: it gave 0.00 for everything.) A silent sound is not given to the
   model at all.
3. The list, the reference sounds and the curve are pinned with the
   model, and `hello` reports their version in the model's revision, so a
   score today means the same tomorrow.
4. Where `yes` starts and `no` ends is set from the test set, reported in
   `hello`, and printed in `docs/ear-check.md` (LAION-CLAP: yes from
   0.662, no below 0.338).

The describe model runs with temperature 0 and a fixed seed, so the same
question gets the same words (and the answer cache in section 3 makes
that exact). Its yes or no is asked **evidence first**: "What do you
hear? List the sounds. Then: does it sound like *a soft coin pickup*?
Answer yes, no or partly." Research in 2026 found these models lean
towards yes, and miss short events. So:

* every answer is checked against the score (`verdict` is `unsure` when
  they disagree);
* a silent clip and a noise clip go through the same questions in the
  test set: a model that says "yes, a coin" to silence is not listening;
* a sound shorter than 1 s is padded with silence to 1 s for the describe
  model (not for the score model).

## 5. Installing the Ear

### What is installed

`duck-ear` stays a Python program, as `ARCHITECTURE.md` planned, but a
light one: **no PyTorch**. It reads the WAV, gets it ready for the models,
runs the score model with ONNX Runtime, and drives `llama-server` for the
describe model. Why Python and not more Rust: each model's own
preparation code (spectrograms, the text tokenizer) is in Python, new
models appear there first, the later fine-tuning is Python, and Duck
Echo's Rust workspace stays free of large machine-learning libraries (the
"keep dependencies few" rule). The Ear is updated apart from Duck Echo.

```
 Duck Echo (Rust)
   └─ starts ─▶ duck-ear (Python 3.12, private, in its own folder)
                  ├─ onnxruntime ─▶ score model (ONNX, ~0.4 GB)
                  └─ starts ─▶ llama-server (llama.cpp, Vulkan or CPU build)
                                 └─ describe model (GGUF, 0.6–6 GB)
```

`duck-ear` starts `llama-server` itself, on 127.0.0.1 with `--offline`,
and stops it when it stops. Duck Echo starts `duck-ear` inside a
**Windows job object** (on Linux, its own **process group**), so that when
Duck Echo stops the Ear, or the Ear crashes, `llama-server` goes too and
never sits on the graphics card's memory.

### Three sizes, chosen by the person

(Two were planned. Step 4 added Large, for PCs with a big graphics card,
and no small describe model has passed its test yet: `docs/ear-check.md`
gets the results.)

| | Small | Full | Large |
|---|---|---|---|
| For | every PC | a PC with a graphics card of 8 GB or more (works on CPU, slowly) | offered only when a graphics card with 16 GB or more is found |
| Gives | `score` and the `verdict` from the score. No describe model: none has passed step 4's test yet | the same, and `describe` and `ask` from Qwen2.5-Omni-7B | the same, from Qwen3-Omni-30B-A3B |
| Programs: uv, Python, packages, llama.cpp | ~150–200 MB (est.) | same | same |
| Models | 0.8 GB score (LAION-CLAP) | + 6.2 GB | + 19.9 GB |
| **Total on disk** | **about 1 GB** | **about 7.2 GB** | **about 21 GB** |

The model sizes are measured (steps 3 and 4); the programs' size is
known after step 5. Sizes are shown before anything is downloaded.

**If no describe model passes step 4's test set**, the Ear ships with
scores only: one size, without `llama-server` or a describe model.
`describe` and `ask` answer "not available yet", with the reason ("No
describe model passed Duck Echo's listening test yet: see
docs/ear-check.md"), and `verdict` comes from the score alone. Step 4
writes its results into `docs/ear-check.md` either way, so the next try
starts from them.

### How it is installed

Never by itself, and never by an agent. A person says yes, after seeing
the size:

* **In the Studio**: Settings > Duck Ear > Install. Also, the first time
  an AI assistant asks to listen and the Ear is missing, the Studio shows
  a note: "An AI assistant wanted to listen to a sound. Install the Duck
  Ear (1.5 GB)? [Install] [Not now]".
* **On the command line**: `duck-echo ear install` (or `--full`) prints
  the size and asks "Install? [y/N]"; `--yes` skips the question.
* **An agent** that asks while the Ear is missing gets `not_installed`
  with these two ways, to pass on to the person.

The steps, all in `duck-echo-tools` (so the CLI and the Studio share
them):

1. **The list of files** (`ear/manifest.toml`, built into Duck Echo): for
   each file its address, size and SHA-256. uv, llama.cpp's two builds and
   each model file are pinned to one version.
2. **Download** each file with `curl`, as the Studio's update check does
   (Windows 10 and 11 have `curl.exe` built in, so Duck Echo needs no new
   download library). Into `<name>.part`, resumed if stopped (`curl
   --continue-at -`), renamed only when its SHA-256 matches. A file that
   does not match is deleted and fetched once more, then the install
   stops with the file's name.
3. **Python**: the downloaded `uv` installs a pinned Python into the
   Ear's own folder and the `duck-ear` package with
   `uv sync --frozen` from the lockfile in `ear/uv.lock` (every package
   checked by its hash). Nothing touches a Python already on the PC.
4. **Which llama.cpp**: the Vulkan build if `llama-server
   --list-devices` finds a graphics card, otherwise the CPU build.
5. **Self-test**: start the Ear, `hello`, score a built-in test sound.
   Only then is the Ear marked installed in `ear.json`.

**Where it lives** (a short path, because of Windows' 260-character
limit):

* Windows: `%LOCALAPPDATA%\DuckEcho\ear` (for example
  `C:\Users\you\AppData\Local\DuckEcho\ear`)
* Linux: `$XDG_DATA_HOME/duck-echo/ear` (usually
  `~/.local/share/duck-echo/ear`)

```
ear/
  ear.json        what is installed: size, versions, SHA-256s, device
  uv/             the uv program
  python/         the private Python
  env/            duck-ear and its packages
  llama/          llama-server (Vulkan or CPU)
  models/<name>/<revision>/   each model's files, checked
  ear.log         the Ear's last 1 MB of logging
```

Models are kept apart from the programs, so updating the Ear does not
download them again.

**Where the models come from**: the score model, converted by us, from a
Duck Echo GitHub release; the describe models from Hugging Face, pinned to
one commit (`.../resolve/<commit>/<file>`). If a model is ever taken down,
the next Duck Echo release points at our own copy (Apache-2.0 allows it).

### With and without a graphics card

| PC | Score model | Describe model |
|---|---|---|
| NVIDIA, AMD or Intel graphics card, 8 GB or more | CPU (fast enough) | Full: on the card, through Vulkan |
| Graphics card under 8 GB | CPU | Small: on the card; Full: does not fit, offered on CPU |
| No graphics card | CPU | Small on CPU; Full on CPU if chosen (about a minute an answer) |

* "Use: Automatic / CPU only" in Settings (and `--ear-device cpu`).
* If the describe model runs out of memory on the card, the Ear tries the
  CPU once by itself and says so in its answer (`device: "cpu (fell
  back)"`).
* Linux needs the graphics card's Vulkan driver (Mesa for AMD and Intel,
  NVIDIA's own driver), as games do. Without it, the CPU build is used and
  Settings says why.
* CUDA (about 620 MB more) is left out unless step 4 shows Vulkan is much
  slower on NVIDIA cards (open question 4).

### Nothing leaves your PC while listening

Several walls, because one is not enough:

1. Installing and listening are separate. Only `ear install` and `ear
   update` download, and they only fetch the files in the manifest.
2. The Ear is started with `HF_HUB_OFFLINE=1`, `HF_HUB_DISABLE_TELEMETRY=1`
   and `UV_OFFLINE=1`, by its own Python directly (not `uv run`, which may
   check for updates).
3. `llama-server` gets `--offline`, listens only on 127.0.0.1, and is sent
   the audio as data, never as an address.
4. Inside `duck-ear`, a Python audit hook refuses any network connection
   that is not to 127.0.0.1 and logs it. (A guard, not a sandbox.)
5. **A test proves it**: on Linux CI the Ear's protocol tests run inside a
   network namespace with no network at all (`unshare --net`), and pass.

### How the Studio shows it: Settings > Duck Ear

```
┌ Duck Ear: listening (optional) ──────────────────────────────────────┐
│ Lets AI assistants ask "does this sound like a soft coin pickup?".   │
│ It runs on this PC only: nothing is sent anywhere when it listens.   │
│                                                                      │
│ Not installed.                                                       │
│ (•) Small: about 1.5 GB. Any PC. Scores and short answers.           │
│ ( ) Full: about 7.5 GB. Best with a graphics card of 8 GB or more.   │
│     This PC: NVIDIA GeForce RTX 4060, 8 GB. Free on C: 212 GB.       │
│                                                     [ Install ]      │
└──────────────────────────────────────────────────────────────────────┘

   while installing:
│ Downloading the describe model: 2.1 of 5.4 GB  [██████░░░░░░] 39%    │
│ It carries on where it stopped if you close the Studio. [ Stop ]     │

   when ready:
│ Ready: Full, on NVIDIA GeForce RTX 4060 (Vulkan). 7.4 GB in          │
│ C:\Users\you\AppData\Local\DuckEcho\ear              [ Open folder ] │
│ Use: (•) Automatic  ( ) CPU only                                     │
│ [ Test ]  coin: "a coin pickup": yes (0.86), 0.4 s;                  │
│           night loop: "calm music": yes, 6.1 s                       │
│ Your ratings: 34 sounds rated.        [ Measure how often it agrees ]│
│ [ Check for an Ear update ]                          [ Remove (7.4 GB) ] │
```

* A broken install says what broke in words ("Your antivirus blocked
  uv.exe", "The disk is full: 3.1 GB more needed") and offers **Repair**
  (the install again; files already there and checked are kept).
* Remove deletes the whole `ear` folder, after asking.

## 6. What agents get [Duck Echo: `duck-echo-tools`, the CLI, the MCP server]

### `analyze` with `ear: true`

`analyze` is unchanged unless it is asked (`"ear": true` in the tool,
`--ear` on the command line). Then its answer gets one more section. The
numbers come first and are the same as without the Ear.

```json
"ear": {
  "available": true,
  "ok": true,
  "advice": "The Ear's answers come from listening models and can be wrong. Check them against the numbers.",
  "models": {"score": "<score model>@<revision>", "describe": "<describe model>@<revision>"},
  "device": "vulkan",
  "heard": {"from_seconds": 0.0, "to_seconds": 0.31},
  "description": "A short, bright two-note chime with a quick fade, like collecting a coin.",
  "closest": [
    {"text": "coin pickup", "match": 0.86},
    {"text": "bell ding", "match": 0.71},
    {"text": "ui confirm", "match": 0.64}
  ],
  "sounds_like": {
    "text": "a soft coin pickup", "match": 0.81, "rank": 2, "of": 120,
    "answer": "yes", "reason": "A gentle, short chime; not harsh.",
    "verdict": "yes"
  },
  "seconds": 5.1,
  "cached": false
}
```

* `description`: the describe model's words (section 4). When no describe
  model is installed (or none passed its test, section 5), it is
  `{"available": false, "reason": "..."}` instead, and so is `answer`.
* `cached`: `true` when the whole answer came from the answer cache
  (section 3); the Ear was not asked.
* `closest`: the score model's best matches from a built-in list of about
  120 game-sound and music words (`coin pickup`, `explosion`,
  `footsteps on gravel`, `calm music`, `tense music` ...), kept in the
  Ear's package so it travels with the model.
* `sounds_like`: only when the recipe's `expect` has `sounds_like` (below).
* `match` is 0 to 1: how much more this text fits the sound than the
  other words in the list do (section 4, "Scores you can read"). `rank`
  is where it came among them.
* `answer` and `reason`: the describe model asked "Does this sound like
  ...? Answer yes, no or partly, then say why in one sentence." It is
  **not** told Duck Echo's numbers, so it cannot repeat them back: its
  reason comes from listening.
* `verdict` puts the two together: `yes` when both say yes, `no` when both
  say no, `unsure` when they disagree or the match is in the middle. When
  no describe model answers (`quick`, the model is busy, or none passed
  its check on this PC), the verdict comes from the match alone, and says
  so (`"from": ["score"]`).
* `heard`: what part was listened to. A clip longer than the models take
  (30 s, section 4) is heard from the start; a looping song once round.
* `summary` gets one more part at its end, after the numbers, marked as
  advice: `...; Ear (advice): sounds like "a soft coin pickup": yes
  (0.81)`.

### `expect.sounds_like`

```toml
expect = { sounds_like = "a soft coin pickup", sharpness = "low", duration_seconds = [0.1, 0.4] }
```

* A new `expect` line: text, 1 to 200 characters.
* With the Ear (`analyze` with `ear: true`), its `pass` is `true` for the
  verdict `yes`, `false` for `no`, and `null` for `unsure`.
* Without the Ear, the line is listed with `"pass": null, "note": "needs
  the Ear: analyze with ear: true"`, and the summary says `expect: all 2
  met (sounds_like not checked)`. An unchecked line is never a failure.
* **Songs get `expect` too**, the same table (a song's `feel` and
  loudness are measured already): `expect = { sounds_like = "a calm,
  warm tune", loudness_lufs = [-19, -17] }`.
* The renderer ignores `expect`, as now. Like any other field, changing it
  changes the recipe's `recipe_hash` (so `game_send` sends again).

### The `listen` tool (and `duck-echo listen`)

For questions that are not in the recipe:

```json
{"path": "sfx/coin.toml", "like": ["a soft coin pickup", "a harsh alarm"]}
{"path": "music/day.toml", "ask": "Is this tune calm? What makes it tense, if anything?"}
{"path": "sfx/coin.toml", "compare_with": "sfx/coin_old.wav", "like": ["a soft coin pickup"]}
{"path": "music/day.toml", "solo": ["drums"], "ask": "Are the drums too busy for a calm tune?"}
```

* `path` or `recipe`, `seed`, `solo` and `mute` work as in `analyze`.
* `like`: up to 8 texts; each gets `match`, `rank`, `answer`, `reason`,
  `verdict`.
* `ask`: one free question; the answer is the describe model's text.
* `compare_with`: a second sound, scored against the same texts. This is
  the most reliable use of the Ear: "is B more like this than A?"
  (`"more_like": "sfx/coin.toml"`, `"by": 0.17`). A difference under 0.05
  is reported as "about the same".
* `quick: true`: the score model only (about a second, no describe model).
* Not an Undo step: it changes nothing.
* The answer has `ear` (as above), `like`, `answer`, `compared`, and a
  `summary` such as `Ear (advice): "a soft coin pickup": yes (0.81), "a
  harsh alarm": no (0.07)`.

### `ear_status` (and `duck-echo ear status`)

Fast, never loads a model: installed or not, which models, their size,
the device it would use, and the last error. An agent calls it once to
know whether to plan with the Ear.

### How the guide teaches it (a new chapter, "Listening with the Ear")

* **Numbers first, then the Ear.** Fix clipping, loudness, notes heard and
  loop joins with the numbers; then ask the Ear what it sounds like.
* **The Ear is advice.** Write "the Ear thinks it sounds like a coin", not
  "it sounds like a coin". Never tell a person a sound is good because the
  Ear said yes.
* **When the Ear and a number disagree, the number wins for what it
  measures.** If `sharpness` is `low` and the Ear says "harsh", say both
  and ask the person; don't chase the Ear.
* **Compare rather than trust one score.** Change one thing, then
  `listen` with `compare_with` the old version. A match of 0.81 means
  little alone; "0.81 now, 0.64 before" means the change helped.
* **Small differences are noise.** Under 0.05 is "about the same".
* **Stop after three rounds** of changes driven by the Ear. If it still
  says no, tell the person what you tried and ask them to listen.
* **`unsure` is an honest answer.** Treat it as "ask a person".
* What the Ear is good and weak at, from the test set (section 8), in one
  table, so agents know where to trust it.

## 7. Your ratings [Studio, `duck-echo-tools`]

### Rating a sound in the Studio

On the sound screen and the song screen, beside Play:

```
 ▶ Play    Does it sound like "a soft coin pickup"?   [ 👍 ] [ 👎 ]  [ a word: too long    ]
```

* The question uses the recipe's `expect.sounds_like`, or else its
  `description`. With neither, it asks "Do you like it?".
* One click saves a rating. Clicking the other thumb changes it; clicking
  the same one again clears it. The word is optional ("harsh", "too
  long", "perfect").
* A rating is about **the sound you just heard**: the exact render, with
  its seed. If the recipe changes, the thumbs are empty again for the new
  sound (your old rating stays, for the old sound).
* Ratings are not Undo steps (they change no sound). They are yours:
  agents can read them, never write them.

### Where they are kept: in the project

```
<project>/ratings/
  ratings.jsonl            one line per click, never rewritten, only added to
  sounds/<sha256>.wav      the exact sound rated (a song over 10 s as .ogg)
  recipes/<hash>.toml      the recipe text it came from
```

One line of `ratings.jsonl`:

```json
{"version":1,"file":"sfx/coin.toml","kind":"recipe","question":"sounds_like",
 "text":"a soft coin pickup","rating":"up","word":"too long",
 "sound":"sha256:3f1c...","recipe_hash":"sha256:9a0b...","seed":0,
 "date":"2026-11-20"}
```

* The last line for the same `sound` and `text` wins; `"rating": "none"`
  clears one. Keeping every click means nothing is lost, and the file is
  easy to read and to merge.
* The sound is stored once however often it is rated (named by its
  SHA-256). Sound effects are a few KB; a 20 s song as OGG about 300 KB.
* The date is the day, from the computer's clock. It is a record, not
  audio, so the "no wall clock" rule does not apply.
* Nothing in `ratings/` ever leaves your PC.

### What agents see

`ratings_list` (`duck-echo ratings list`): your ratings, newest first,
for one file or all, with the word. So an agent can read "the person said
*too harsh* about version 3" before it changes the sound again. The guide
says: a person's rating outranks the Ear.

### Measuring how often the Ear agrees with you

`ear_agreement` (`duck-echo ear agreement`, and a button in Settings >
Duck Ear):

1. For each sound you rated up or down (the latest rating), ask the Ear
   the same question about the stored sound: `sounds_like` for a
   description, "Is this a good game sound?" for "Do you like it?". The
   answer cache (section 3) keeps them, so measuring again with the same
   models is instant.
2. Count. The Ear's `yes` with your 👍, or its `no` with your 👎, agree.
   `unsure` is counted on its own, not as right or wrong.
3. Report it honestly:

```
You rated 34 sounds. The Ear was sure about 30 of them and agreed with
you on 25 (83%; with this few ratings, somewhere between 70% and 91%).
Always saying "yes" would have agreed 24 times out of 34 (71%), because
you liked most of them.
Sound effects: 20 of 22 (91%). Music: 5 of 8 (63%).
Where it disagreed most: music/night.toml (you 👎 "too busy", Ear yes) ...
```

* The range is a Wilson interval at 90%, said in words. With fewer than
  20 ratings it says "too few to measure yet; rate N more".
* The "always yes" line is there because a model that says yes to
  everything looks good when you like most sounds. The Ear only helps if
  it beats that line.
* The models' revisions are in the report, so a new model can be
  measured against the old one on the same ratings.

### Fine-tuning, later (not in Phase 4)

The ratings hold exactly what fine-tuning needs: the sound, the text and
your answer. Two later steps, in order:

1. **Your thresholds** (from about 50 ratings): fit the `match` value
   where `yes` starts to your ratings. No training, seconds to compute,
   easy to undo.
2. **A small adapter on the score model** (from a few hundred ratings,
   with the test set to check nothing got worse): needs a graphics card
   and an hour. The measure above decides whether it helped.

## 8. How we prove the Ear helps

### A test set with known answers

`ear/testset/testset.toml`: about 40 sounds, **all made by Duck Echo**
(presets, examples and a few recipes written for it), so no audio is
stored and anyone can rebuild them. Each lists texts that are true and
texts that are wrong for it:

```toml
[[sound]]
file = "preset:coin"
true = ["a coin pickup", "a short bright chime"]
wrong = ["an explosion", "footsteps", "a calm tune", "a harsh alarm"]

[[sound]]
file = "testset/coin_harsh.toml"      # a coin made loud, long and buzzy on purpose
true = ["a harsh coin sound"]
wrong = ["a soft coin pickup"]

[[sound]]
file = "example:duck_craft_night"
true = ["calm music", "slow dark music"]
wrong = ["tense music", "fast exciting music"]
```

* Sound effects: coin, explosion, laser, jump, footstep, door, hit,
  power-up, UI click, alarm, magic, engine, water, wind (and soft and
  harsh versions of three of them, to test words like "soft").
* Music: calm and tense versions of the same tune (tempo, key, drums),
  the Duck Craft day and night loops, a happy chiptune, a scary drone.

What `duck-echo ear check` measures, and the bar each must pass before we
tell agents to rely on that part:

| Measure | What it means | Bar |
|---|---|---|
| Kind of sound | the true text scores above every wrong one | 90% |
| Pairs | for one text, the right sound scores above the wrong one | 90% |
| Words like "soft", "harsh" | soft and harsh versions told apart | 75% |
| Calm or tense music | answered right, both ways | 80% |
| Yes and no | `ask` right on true **and** on wrong texts, counted apart (a yes-sayer gets 100% and 0%) | 80% each |
| Time | seconds per answer, CPU and graphics card | recorded |

* The check writes `ear-check.json` and a table. We commit its results as
  `docs/ear-check.md`, with the models' revisions and the PC they ran on.
  A part that misses its bar is listed in the guide as a weak spot.
* CI runs only the stub (the models are too big to download for every
  run). A manual workflow runs the score model's check on Linux (CPU),
  because that model is small enough.

### Playtest 5: an agent fixes sounds with the Ear

A fresh agent with only the guide and the MCP server (with the Ear) gets
three sounds that are wrong on purpose and the words they should match:

1. A "soft coin pickup" that is harsh and too long.
2. An "explosion" that is thin and short.
3. A "calm" loop that is fast and busy.

It may change each at most three rounds. Then **you** listen, without
being told which is which, to the before and after of each (in the
Studio, in random order) and rate them. The Ear helped if:

* you prefer the after version in at least 2 of 3;
* the agent's notes show it used the Ear's reasons, not only its scores;
* it never broke a number to please the Ear (no clipping, loudness kept).

Its notes and a review go in `playtest/playtest-5/`, as before.

## 9. Build steps

Each step fits in one session and leaves everything working. The
protocol and the stub come first; real models come only after the Rust
side is proven with the stub. Steps marked **(stronger model)** have
traps that are easy to miss (processes on two systems, model numbers).

### Step 1: the bridge crate and the stub (stronger model) [done]

* `crates/duck-echo-ear`: the protocol types, starting the program (in a
  job object on Windows, a process group on Linux), the three threads, the
  timeout table, stopping (`bye`, then kill the whole tree), starting
  again after a failure.
* `ear/stub/duck_ear_stub.py` (standard library only), and the
  misbehaving test program.
* Tests: handshake; each timeout; a missing program; a line that is not
  JSON; a wrong `id`; the Ear exiting in the middle; 50 MB of stderr; a
  2 MB line; an old protocol; killing the tree leaves no process behind;
  a path with spaces and non-English letters.
* ARCHITECTURE.md's Ear section updated to match.

What you test on Windows:
- [ ] `cargo test -p duck-echo-ear` passes.
- [ ] Task Manager shows no `python.exe` left after the tests.
- [ ] With the stub in a folder named `C:\Temp\Dück Ëar\`, the handshake works.

What you test on Linux:
- [ ] `cargo test -p duck-echo-ear` passes.
- [ ] `ps aux | grep duck_ear` shows nothing left after the tests.
- [ ] The tests pass inside `unshare --net` (no network).

### Step 2: what agents get, with the stub [done]

* `analyze` with `ear: true` and `--ear`; the `listen` tool and `duck-echo
  listen`; `ear_status`; the summary's advice part.
* `expect.sounds_like` for recipes; `expect` (the whole table) for songs.
  Schema, `RECIPE_SPEC.md` and the song spec updated; an example recipe
  with `sounds_like`.
* The MCP server keeps one Ear and stops it after 10 minutes with no
  question; `load` before every describe question; temporary WAVs in
  their own folder, deleted after.
* The answer cache (section 3) in the bridge.
* Tests (all with the stub): every field of the `ear` section; `pass:
  null` without the Ear; `compare_with`; the idle stop; a render and
  `analyze` without `ear` never start the Ear; **asking twice gives the
  identical answer, the second `cached: true`, with the stub not asked
  again**; a new model revision in `hello` misses the cache.

What you test on Windows:
- [ ] `duck-echo listen preset:coin --like "a coin" --ear-program ear\stub\duck_ear_stub.py` answers, marked `stub`.
- [ ] `duck-echo analyze preset:coin --ear` with no Ear says `not_installed` and still gives the numbers.
- [ ] In Claude Code with the project's MCP server, ask it to listen to a sound: it gets the stub's answer.

What you test on Linux:
- [ ] The same three.

### Step 3: the real Ear, part 1: the score model (stronger model) [done]

* `ear/duck_ear/`: a Python package with `pyproject.toml` and `uv.lock`:
  reading WAVs, resampling, the score model through ONNX Runtime, the
  word list, `match` and `rank`, the network guard.
* `ear/convert/convert_clap.py` (run once, by us, with PyTorch): converts
  CLAP2-S and LAION-CLAP to ONNX and checks them against the originals.
* The test set (`ear/testset/`) and `duck-echo ear check` (score part).
  Both models measured; the better one on our sounds chosen; the yes and
  no lines set; results in `docs/ear-check.md`.
* Run from the source with `--ear-program ear/.venv/bin/duck-ear` or
  `DUCK_ECHO_EAR` (no installer yet).
* As built: LAION-CLAP was chosen, not CLAP2-S (pairs 98% against 88%);
  `match` uses z-scores, not a softmax (above); `ear check` has the score
  part only, so it has no `--part` yet.

What you test on Windows:
- [ ] `uv sync` in `ear/`, the model converted with `--install` (`ear/convert/README.md`); `duck-echo listen preset:coin --like "a coin pickup" --like "an explosion" --ear-program ear\.venv\Scripts\duck-ear.exe` says yes to the coin (about 0.74) and no to the explosion. It takes a few seconds, most of it loading the model; with `--json`, `seconds` (the listening itself) is under 2.
- [ ] `duck-echo ear check --ear-program ear\.venv\Scripts\duck-ear.exe` finishes and prints its table.
- [ ] Pull the network cable (or turn Wi-Fi off): listening still works.

What you test on Linux:
- [ ] The same, and `ear check` gives the same table as on Windows (to 2 decimals).

### Step 4: the real Ear, part 2: the describe model (stronger model)

* `duck-ear` starts and stops `llama-server` (`--offline`, 127.0.0.1),
  with temperature 0 and seed 42; stops it after `describe_idle_seconds`
  with no describe question; sends audio as data, asks evidence first,
  pads short sounds, falls back to the CPU when the card runs out of
  memory.
* `describe`, `ask`, and the `verdict` that combines the two models.
* Qwen2.5-Omni-7B measured on a graphics card and on CPU; MiDashengLM-0.6B
  and Gemma 4 E2B measured on CPU; the small one chosen; the timeouts set
  from the measured times; silence and noise checks pass.
* The same question asked twice of the real model (with the cache off for
  the test) gives the same words on one PC.
* `docs/ear-check.md` gets the describe part and the times, whether a
  model passed or not. **If none passed**, the Ear ships with scores only
  (section 5), and `describe` and `ask` say "not available yet" with the
  reason.

What you test on Windows (your PC with its graphics card):
- [ ] `duck-echo listen example:duck_craft_night --ask "Is this tune calm?"` answers with a reason in under 10 s.
- [ ] With "CPU only", the same answers in under 4 minutes.
- [ ] After the Ear stops, Task Manager shows no `llama-server.exe` and the card's memory is free again.
- [ ] Ask one describe question, wait 2 minutes: `llama-server.exe` is gone and the card's memory is free, but a score question still answers at once. Ask another describe question: it starts again.

What you test on Linux:
- [ ] The same on CPU (and on a graphics card if the PC has one).

### Step 5: installing the Ear (stronger model)

* `ear/manifest.toml`; downloads with resume and SHA-256; uv, Python, `uv
  sync --frozen`; Vulkan or CPU llama.cpp; the models; the self-test;
  `ear.json`.
* `duck-echo ear install [--full] [--yes]`, `ear status`, `ear update`,
  `ear remove`.
* Errors in words: no space, antivirus, a file that does not match, no
  internet.
* Tests: against a small local test server with fake files (a stopped
  download resumes; a wrong SHA-256 is refused; no space is caught); a
  manual workflow installs the real Small Ear on Windows and Linux.

What you test on Windows:
- [ ] `duck-echo ear install` on a PC with no Python: it asks, shows the size, installs, the self-test passes.
- [ ] Stop it in the middle (close the window); run it again: it carries on, it does not start over.
- [ ] `ear remove` leaves no `DuckEcho\ear` folder.

What you test on Linux:
- [ ] The same, and on a PC with no graphics card the CPU build of llama.cpp is chosen.

### Step 6: the Studio's Settings > Duck Ear

* The box from section 5: sizes, install with progress, stop and carry
  on, Test, device, Remove, Repair, Clear answers, the "an assistant
  wanted to listen" note.
* The Studio's Ear follows the same two rules as the MCP server's (10
  minutes for the Ear, 2 for `llama-server`).
* Screen tests with a fake installer, and pictures in CI.

What you test on Windows:
- [ ] Install Small from Settings; close the Studio halfway; open it: it carries on.
- [ ] Test shows the coin and the night loop answers.
- [ ] Remove frees the space shown.

What you test on Linux (AppImage):
- [ ] The same.

### Step 7: your ratings, and measuring the Ear

* Thumbs and the word on the sound and song screens; `ratings/`;
  `ratings_list`; `ear_agreement` and its line in Settings.
* Tests: a rating's line and its stored sound; the last rating wins; the
  measure on made-up ratings gives the right counts, range and "always
  yes" line; agents have no way to write a rating.

What you test on Windows:
- [ ] Rate five sounds; `ratings/ratings.jsonl` has five lines and the sounds are stored.
- [ ] Change a recipe: the thumbs are empty again.
- [ ] "Measure" with fewer than 20 ratings says how many more are needed.

What you test on Linux:
- [ ] The same.

### Step 8: the guide, playtest 5, the roadmap

* `AGENT_GUIDE.md`: the chapter "Listening with the Ear" (section 6),
  with what the Ear is good and weak at from `docs/ear-check.md`, and its
  test.
* Playtest 5 (section 8), its review and fixes.
* `ROADMAP.md`: Phase 4 ticked off with lessons; this design marked done.

What you test on Windows:
- [ ] Blind-rate playtest 5's before and after pairs in the Studio.

What you test on Linux:
- [ ] Run playtest 5's agent part with the Small Ear only, and read its notes.

## 10. Risks (and what we do about each)

1. **The Ear is confidently wrong.** CLAP models act like a "bag of
   words"; audio language models lean towards "yes" and miss short events.
   *We* ask evidence first, make the two models check each other, count
   right answers on true and wrong texts apart, test silence and noise,
   show `unsure`, and teach agents to compare versions rather than trust
   one score.
2. **Our sounds are made by synthesis; the models learned from
   recordings.** A beep-built coin may not sound like a coin to them.
   *We* build the test set only from Duck Echo's own sounds, and list what
   the Ear is weak at in the guide.
3. **CLAP2 is weeks old, and its ONNX export is not official.** *We*
   convert it ourselves, check it against the original, and measure
   LAION-CLAP beside it; the better one on our sounds ships.
4. **Some numbers in section 4 were read from search summaries**, because
   Hugging Face and arXiv could not be opened from the build machine.
   *We* measure sizes, speeds and quality ourselves in steps 3 and 4
   before anything is promised in Settings.
5. **Licences change, or a model is taken down.** *We* pin every model to
   one commit, record each licence in `ear.json` and in the Studio's
   About, and keep our own copy of anything Apache-2.0 we depend on.
6. **Big downloads and full disks.** *We* offer two sizes, show the size
   and free space first, carry on stopped downloads, and give Remove.
7. **Antivirus and Windows' long paths.** Defender has flagged `uv.exe`
   before. *We* keep the folder path short, check every file's SHA-256,
   and say in words which file was blocked.
8. **`llama-server` left running, holding the graphics card's memory.**
   *We* use a job object (Windows) and a process group (Linux), and test
   that nothing is left.
9. **Two Ears at once** (the MCP server's and the Studio's) loading the
   big model twice. *We* stop `llama-server` after 2 minutes with no
   describe question, and a lock file in the `ear` folder lets only one
   Ear load the describe model at a time; a second gets `describe: busy`
   and still scores. The shared answer cache means a question one of them
   asked is not asked again by the other.
10. **Slow on CPU** (a minute an answer with the 7B model). *We* offer
    `quick`, set timeouts from measured times, and recommend Small on
    CPU-only PCs.
11. **Agents chase the score** and change a good sound to please the Ear.
    *We* teach "numbers win for what they measure", "under 0.05 is noise"
    and "three rounds, then ask a person", and playtest 5 checks it.
12. **Answers differ a little between PCs** (graphics card or CPU). The Ear
    is not part of the "same recipe, same bytes" promise: its answers are
    never written into a sound or its facts, they are rounded to 2
    decimals, and they name the models and the device. Duck Echo's own
    tests use the stub.
13. **The private Python breaks** (an update, a moved folder). *We* run
    the self-test when Settings opens the Ear box, and Repair re-installs
    without downloading what is already there.

## 11. Open questions (answered: 1 to 8 as recommended, 9 changed)

1. **Python for the Ear, or Rust only?** Recommend **Python**, light (no
   PyTorch): ONNX Runtime and `llama-server` do the heavy work. Rust only
   would mean writing each model's preparation code again in Rust and
   adding big libraries to the workspace.
2. **When is the Ear downloaded?** Recommend **only when a person says
   yes**, after seeing the size, either in Settings or from the note the
   Studio shows the first time an assistant asks. Never by itself, never
   by an agent.
3. **Which size by default?** Recommend **Small** for everyone, with Full
   offered (not chosen) when a graphics card of 8 GB or more is found.
4. **Vulkan only, or CUDA too?** Recommend **Vulkan only** (36 MB, all
   three makes of card), and CUDA (about 620 MB) only if step 4 measures
   Vulkan as much slower on NVIDIA.
5. **Do songs get the whole `expect` table?** Recommend **yes**: their
   loudness and feel are measured already, and `sounds_like = "a calm
   tune"` is what playtest 4 was missing.
6. **Where are ratings kept?** Recommend **in the project, `ratings/`**:
   they belong with the sounds, move with the project, and can go in your
   Git if you want. Never uploaded.
7. **Is the describe model told Duck Echo's numbers?** Recommend **no**:
   it would repeat them back as if it heard them. The agent puts the two
   together.
8. **Does `analyze` listen by default?** Recommend **no**: only with `ear:
   true`, so `analyze` stays fast and its answer stays the same on every
   PC.
9. **The idle time before the MCP server stops the Ear.** *Decided:* the
   Ear and its score model stay 10 minutes; `llama-server` stops after 2
   minutes with no describe question, so the graphics card is given back
   quickly. The Studio's Ear follows the same rules.

## 12. Sources

Checked on 2026-11-19. The research agents could read GitHub and PyPI
directly; Hugging Face and arXiv only through search summaries.

* CLAP2: https://github.com/LAION-AI/clap2 ,
  https://huggingface.co/laion/clap2-large-audioxl-s1B ,
  https://arxiv.org/html/2611.11592
* LAION-CLAP: https://github.com/LAION-AI/CLAP ,
  https://huggingface.co/laion/larger_clap_general ,
  ONNX: https://huggingface.co/Xenova/clap-htsat-unfused
* PE-AV: https://github.com/facebookresearch/perception_models ,
  https://arxiv.org/abs/2512.19687
* Excluded on licence: M2D-CLAP https://github.com/nttcslab/m2d (LICENSE.pdf),
  MuQ-MuLan https://huggingface.co/OpenMuQ/MuQ-MuLan-large ,
  GLAP https://github.com/xiaomi-research/dasheng-glap with SONAR
  https://github.com/facebookresearch/SONAR ,
  Audio Flamingo https://github.com/NVIDIA/audio-flamingo
* Scores are not probabilities: Human-CLAP https://arxiv.org/pdf/2506.23553 ,
  XACLE challenge https://arxiv.org/html/2602.00604 ,
  "Don't CLAP" https://arxiv.org/pdf/2609.30540
* Qwen2.5-Omni: https://github.com/QwenLM/Qwen2.5-Omni ,
  https://huggingface.co/Qwen/Qwen2.5-Omni-7B
* MiDashengLM: https://github.com/xiaomi-research/dasheng-lm ,
  https://arxiv.org/html/2508.03983v2
* Qwen3-Omni: https://github.com/QwenLM/Qwen3-Omni ;
  Step-Audio 2: https://huggingface.co/stepfun-ai/Step-Audio-2-mini ;
  Kimi-Audio: https://github.com/MoonshotAI/Kimi-Audio ;
  Gemma 4: https://ai.google.dev/gemma/docs/core/model_card_4
* Yes-leaning and missed events: https://arxiv.org/html/2604.19300 ,
  https://arxiv.org/pdf/2410.16130v1
* llama.cpp audio models and server:
  https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md ,
  https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md ,
  builds and sizes: https://github.com/ggml-org/llama.cpp/releases
* uv and PyTorch: https://github.com/astral-sh/uv/blob/main/docs/guides/integration/pytorch.md ;
  sizes: https://pypi.org/project/uv/#files ,
  https://pypi.org/project/torch/#files ,
  https://pypi.org/project/onnxruntime/#files
* Folders: https://platformdirs.readthedocs.io/en/stable/platforms.html ;
  Windows path limit:
  https://learn.microsoft.com/windows/win32/fileio/maximum-file-path-limitation
* Offline: https://huggingface.co/docs/huggingface_hub/package_reference/environment_variables ;
  Python audit hooks: https://peps.python.org/pep-0578/
* Lessons from other apps: ComfyUI Desktop https://github.com/Comfy-Org/desktop ,
  Krita AI Diffusion https://github.com/Acly/krita-ai-diffusion/wiki/Common-Issues ,
  LM Studio https://lmstudio.ai/changelog ;
  Defender flagging uv: https://github.com/NousResearch/hermes-agent/issues/48411
