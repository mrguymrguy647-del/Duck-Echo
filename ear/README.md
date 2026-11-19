# The Duck Ear

The Duck Ear is an optional program that listens to a sound with
machine-learning models and says what it hears, so an AI assistant can ask
"does this sound like a soft coin pickup?" without a person listening.
Duck Echo starts it, talks to it in JSON Lines over its stdin and stdout,
and never waits for it longer than a set time. Nothing in Duck Echo needs
it. The plan is `docs/phase4-design.md`; how well it hears is
`docs/ear-check.md`.

* `duck_ear/`: the real Ear, a Python package. It has the score model so
  far (step 3 of the design): it scores how well texts fit a sound.
  Describing a sound comes with step 4.
* `convert/`: run once, by Duck Echo's builders: converts the score
  models to ONNX and checks them against the originals (see
  `convert/README.md`). It needs PyTorch; the Ear does not.
* `testset/`: sounds with known answers, for `duck-echo ear check`.
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

## Trying the stub by hand

One request per line; Ctrl+D, or Ctrl+Z then Enter on Windows, ends it:

```bash
python3 ear/stub/duck_ear_stub.py
{"id": 0, "method": "hello", "params": {"protocol": 1}}
{"id": 1, "method": "bye"}
```

The real Ear answers the same requests: `uv run duck-ear` in `ear/`.
