# The Duck Ear

The Duck Ear is an optional program that listens to a sound with
machine-learning models and says what it hears, so an AI assistant can ask
"does this sound like a soft coin pickup?" without a person listening.
Duck Echo starts it, talks to it in JSON Lines over its stdin and stdout,
and never waits for it longer than a set time. Nothing in Duck Echo needs
it. The plan is `docs/phase4-design.md`; how well it hears is
`docs/ear-check.md`.

* `duck_ear/`: the real Ear, a Python package. Its score model scores how
  well texts fit a sound. Its describe model, when one is installed, says
  what a sound is and answers questions about it; it runs in
  `llama-server` (llama.cpp), which the Ear starts and stops.
* `tools/get_describe_model.py`: downloads a describe model and checks
  it. A person runs it; the Ear itself never uses the network.
* `tools/measure_describe_model.py`: downloads a describe model (after
  asking) and measures it on this PC, in one command.
* `convert/`: run once, by Duck Echo's builders: converts the score
  models to ONNX and checks them against the originals (see
  `convert/README.md`). It needs PyTorch; the Ear does not.
* `testset/`: sounds with known answers, for `duck-echo ear check`
  (`testset.toml`, 41 sounds; `quick.toml`, 10 of them).
* `tests/`: the Ear's own tests (`python -m unittest discover -s tests`).
* `stub/duck_ear_stub.py`: the stub Ear. It speaks the protocol with no
  models (Python's standard library only), and every answer says
  `"stub": true`. Duck Echo's tests use it.

## Running the real Ear from the source

The installer comes with step 5; until then, with
[uv](https://docs.astral.sh/uv/):

```bash
cd ear
uv sync
```

That makes `ear/.venv` with the Ear and its three libraries (numpy, ONNX
Runtime and tokenizers; no PyTorch). Then put a score model where the Ear
looks for it (`convert/README.md`), and point Duck Echo at the Ear:

```bash
duck-echo listen preset:coin --like "a coin pickup" --like "an explosion" --ear-program ear/.venv/bin/duck-ear
```

On Windows the program is `ear\.venv\Scripts\duck-ear.exe`. Or set
`DUCK_ECHO_EAR` to it once.

The Ear looks for its score model in the folder `DUCK_EAR_SCORE_MODEL`
names, else in `models/score` inside the Ear's folder
(`DUCK_ECHO_EAR_HOME`, else `%LOCALAPPDATA%\DuckEcho\ear` on Windows and
`~/.local/share/duck-echo/ear` on Linux). It never uses the network: a
guard refuses every connection that is not to this PC.

## The describe model (Full and Large)

The Ear comes in three sizes:

| Size | Describe model | Download | For | Listening test |
|---|---|---|---|---|
| Small | none: scores only | | every PC | |
| Full | Qwen2.5-Omni-7B | 6.2 GB | a graphics card of 8 GB or more (works on the CPU, slowly) | did not pass |
| Large | Qwen3-Omni-30B-A3B | 19.9 GB | offered only when a graphics card of 16 GB or more is found | not measured yet |

**No describe model has passed Duck Echo's listening test** (they say no
to too many true texts: `docs/ear-check.md`), so the Ear ships with
scores only, and nothing needs a describe model. Full and Large can be
added to measure them, and Full does one useful thing: with it beside
the score model, the verdict never said yes to a wrong text.

The describe model needs two things:

1. **`llama-server`**, from llama.cpp's releases
   (https://github.com/ggml-org/llama.cpp/releases): take the newest
   build for your PC (on Windows with a graphics card, the Vulkan or the
   CUDA zip; with none, the CPU zip) and unpack it. Tell the Ear where it
   is: set `DUCK_EAR_LLAMA_SERVER` to the `llama-server` program, or put
   the unpacked files in `llama/` inside the Ear's folder.
2. **A model**:

   ```bash
   python ear/tools/get_describe_model.py --list
   python ear/tools/get_describe_model.py full
   ```

   `--list` shows the sizes, the graphics card that was found, and
   whether Large is offered. The download is checked (SHA-256), goes into
   `models/` in the Ear's folder, and becomes the model the Ear uses.
   To try another model without changing that, set
   `DUCK_EAR_DESCRIBE_MODEL` to its folder.

To download a model **and measure it** on this PC in one command
(`docs/ear-check.md` has the Windows steps):

```bash
ear/.venv/bin/python ear/tools/measure_describe_model.py full
```

It writes `ear-check.json` and `ear-check.md` into `ear-check-full/`, and
does not change which model the Ear uses. `--quick` asks about 10 sounds
only.

Then `duck-echo ear status` names the describe model and where it runs,
and `listen` gives a description and a yes or no for each `--like`:

```bash
duck-echo listen examples/songs/duck_craft_night.toml --like "calm music" --ask "Is this tune calm?" --ear-program ear/.venv/bin/duck-ear
```

`DUCK_EAR_DEVICE=cpu` keeps the describe model off the graphics card.
`llama-server` runs on this PC only (127.0.0.1, `--offline`), its log is
`llama-server.log` in the Ear's folder, and the Ear stops it after 2
minutes with no describe question, so the card's memory is free again.

## Trying the stub by hand

One request per line; Ctrl+D, or Ctrl+Z then Enter on Windows, ends it:

```bash
python3 ear/stub/duck_ear_stub.py
{"id": 0, "method": "hello", "params": {"protocol": 1}}
{"id": 1, "method": "bye"}
```

The real Ear answers the same requests: `uv run duck-ear` in `ear/`.
