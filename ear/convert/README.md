# Converting the Duck Ear's score models

`convert_clap.py` is run once, by Duck Echo's builders, not by people
using the Ear. It downloads a score model from Hugging Face at a pinned
revision, converts it to ONNX, and checks the Ear's path (numpy and ONNX
Runtime) against the original (PyTorch): every sound and text must give
an embedding with closeness 0.999 or more to the original's, or it
fails. It also saves the calibration that makes `match` readable (see
`docs/ear-check.md`). This folder is its own project, because it needs
PyTorch and the Ear does not.

It needs [uv](https://docs.astral.sh/uv/), about 3 GB of disk for
PyTorch and the downloads, and the network (Hugging Face and PyTorch's
download site).

## 1. Render Duck Echo's sounds

The converter checks with, and calibrates on, Duck Echo's presets and
examples. Render them into one folder, from the repository's top folder.

On Linux:

```bash
mkdir -p ear-sounds
for f in presets/*.toml; do n=$(basename "$f" .toml); duck-echo render "preset:$n" -o "ear-sounds/preset-$n.wav"; done
for f in examples/recipes/*.toml examples/songs/*.toml; do n=$(basename "$f" .toml); duck-echo render "$f" -o "ear-sounds/example-$n.wav"; done
```

On Windows (PowerShell):

```powershell
mkdir ear-sounds
Get-ChildItem presets\*.toml | ForEach-Object { duck-echo render "preset:$($_.BaseName)" -o "ear-sounds\preset-$($_.BaseName).wav" }
Get-ChildItem examples\recipes\*.toml, examples\songs\*.toml | ForEach-Object { duck-echo render $_.FullName -o "ear-sounds\example-$($_.BaseName).wav" }
```

That is 43 sounds (WAV files are not committed).

## 2. Convert

The chosen model, straight into the Ear's folder as `models/score`:

```bash
cd ear/convert
uv sync
uv run python convert_clap.py --sounds ../../ear-sounds --install
```

Or both models, side by side, to measure them:

```bash
uv run python convert_clap.py --sounds ../../ear-sounds --out ../../ear-models
```

Set `HF_HUB_DISABLE_XET=1` first if the download stops (some networks
block Hugging Face's newer download service). Each model's folder gets
`model.json`: its source, revision, licence, settings, the check's
results, the lines, and every file's SHA-256.

## 3. Measure

```bash
duck-echo ear check --ear-program ear/.venv/bin/duck-ear
```

with `DUCK_EAR_SCORE_MODEL` set to a model's folder to measure one that
is not installed. `ear check` prints the lines it suggests; to change
the lines, put the matching z values in `MODELS` in `convert_clap.py`
and convert again.
