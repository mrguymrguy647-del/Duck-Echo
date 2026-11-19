"""Convert the Duck Ear's score models to ONNX, and check them.

Run once, by Duck Echo's builders, not by people using the Ear:

    cd ear/convert
    uv sync
    HF_HUB_DISABLE_XET=1 uv run python convert_clap.py --sounds DIR --out DIR

`--sounds` is a folder of WAV files to check with (Duck Echo's presets and
examples, rendered: see ear/convert/README.md). For each model it:

1. downloads the original from Hugging Face, at one pinned revision;
2. exports its audio part and its text part to ONNX. The audio part starts
   from the mel spectrogram, which the Ear makes itself in numpy
   (duck_ear/audio.py), so the Ear needs no PyTorch;
3. saves what the Ear needs beside them: the mel filters, the tokenizer,
   and for CLAP2 the table of its position codes (RoPE);
4. checks the Ear's whole path (numpy front end + ONNX) against the
   original (PyTorch front end + PyTorch model) on every sound and on a
   list of texts: each embedding must point the same way, closeness
   (cosine) 0.999 or more. Below that, the conversion fails.

It writes `<out>/<name>/model.json` with the model's name, source,
revision, licence, settings, every file's SHA-256 and the check's results.

With `--install` instead of `--out`, it converts only the chosen model
(CHOSEN, below) and puts it where the Ear looks for its score model:
`<the Ear's folder>/models/score`.
"""

import argparse
import hashlib
import json
import math
import os
import shutil
import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch
import torch.nn.functional as F

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))  # the duck_ear package, for its front end
from duck_ear import audio as ear_audio  # noqa: E402
from duck_ear import score as ear_score  # noqa: E402

# `lines` and `calibration` come from the test set (docs/ear-check.md):
# on each model's z-scores, yes from `yes_z` and no below `no_z` gave the
# most yes on true texts and no on wrong ones together, with each kind of
# mistake at most 10% (what `duck-echo ear check` suggests). `match` is
# 0.5 halfway between them, with steepness 2.
MODELS = {
    "clap2-small": {
        "kind": "clap2",
        "repo": "laion/clap2-small-audioxl-s1B",
        "revision": "56bdd36775531b9b22ee7681437da32830bb50dd",
        "licence": "Apache-2.0",
        "yes_z": 0.57,
        "no_z": 0.23,
    },
    "laion-clap-general": {
        "kind": "laion-clap",
        "repo": "laion/larger_clap_general",
        "revision": "ada0c23a36c4e8582805bb38fec3905903f18b41",
        "licence": "Apache-2.0",
        "yes_z": 0.98,
        "no_z": 0.31,
    },
}

STEEPNESS = 2.0

# The model the Ear uses: it did better on Duck Echo's sounds
# (docs/ear-check.md).
CHOSEN = "laion-clap-general"

CHECK_TEXTS = [
    "A coin is collected in a video game.",
    "a soft coin pickup",
    "A loud explosion with a long rumble.",
    "Footsteps on gravel.",
    "Calm, slow piano music.",
    "Tense, fast music with drums.",
    "silence",
]

ENOUGH = 0.999


def rms_norm(x, normalized_shape, weight=None, eps=None):
    """RMSNorm written out, so the ONNX exporter knows every step."""
    eps = 1e-6 if eps is None else eps
    dims = tuple(range(-len(normalized_shape), 0))
    y = x * torch.rsqrt(x.pow(2).mean(dim=dims, keepdim=True) + eps)
    return y * weight if weight is not None else y


F.rms_norm = rms_norm
torch.rms_norm = rms_norm


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def cosine(a, b):
    a = np.asarray(a, dtype=np.float64).ravel()
    b = np.asarray(b, dtype=np.float64).ravel()
    return float(a @ b / (np.linalg.norm(a) * np.linalg.norm(b)))


def download(spec):
    from huggingface_hub import snapshot_download

    return Path(snapshot_download(spec["repo"], revision=spec["revision"]))


def sounds_in(folder):
    files = sorted(Path(folder).glob("*.wav"))
    if not files:
        sys.exit("no WAV files in %s" % folder)
    return files


# ---------------------------------------------------------------------------
# CLAP2 (OpenCLIP).
# ---------------------------------------------------------------------------


def convert_clap2(name, spec, source, out, sounds):
    import open_clip
    from open_clip.audio.io import load_audio
    from timm.models.naflexvit import NaFlexVit

    model, preprocess = open_clip.create_model_from_pretrained("local-dir:" + str(source))
    tokenizer = open_clip.get_tokenizer("local-dir:" + str(source))
    model.eval()
    cfg = json.loads((source / "open_clip_config.json").read_text())["model_cfg"]["audio_cfg"]
    vit = next(m for m in model.audio.modules() if isinstance(m, NaFlexVit))

    # The position codes (RoPE) of the largest grid, 8 x 188 (30 s): a clip's
    # codes are rows of this table, checked below for every clip length.
    freq_rows = cfg["mel_bins"] // cfg["patch_freq"]
    max_time = cfg["audio_seq_len"] // freq_rows
    table = vit.rope.get_embed(shape=(freq_rows, max_time)).detach().numpy().astype(np.float32)
    for tt in (1, 2, 7, 63, max_time):
        small = vit.rope.get_embed(shape=(freq_rows, tt)).detach().numpy()
        rows = [f * max_time + t for f in range(freq_rows) for t in range(tt)]
        assert np.allclose(small, table[rows], atol=1e-6), "the RoPE codes depend on the clip length"

    # The ONNX graph takes the codes as an input, so that nothing in it
    # depends on the clip's length (the exporter cannot follow that).
    holder = {}
    vit._generate_rope_naflex = lambda x, coord: holder["rope"]

    class Audio(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.model = model

        def forward(self, patches, patch_coord, patch_valid, rope):
            holder["rope"] = rope
            return self.model.encode_audio(
                {"patches": patches, "patch_coord": patch_coord, "patch_valid": patch_valid},
                normalize=True,
            )

    class Text(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.model = model

        def forward(self, ids):
            return self.model.encode_text(ids, normalize=True)

    filters = open_clip.audio.dsp.mel_filterbank(
        cfg["window_size"] // 2 + 1, float(cfg["fmin"]), float(cfg["fmax"]), cfg["mel_bins"], cfg["sample_rate"]
    ).numpy().astype(np.float32)
    out.mkdir(parents=True, exist_ok=True)
    np.save(out / "mel_filters.npy", filters)
    np.save(out / "rope.npy", table)
    shutil.copy(source / "tokenizer.json", out / "tokenizer.json")
    settings = {
        "kind": "clap2",
        "sample_rate": cfg["sample_rate"],
        "n_fft": cfg["window_size"],
        "hop": cfg["hop_size"],
        "mel_bins": cfg["mel_bins"],
        "patch_freq": cfg["patch_freq"],
        "patch_time": cfg["patch_time"],
        "patch_pad_mode": cfg.get("patch_pad_mode", "floor"),
        "max_patches": cfg["audio_seq_len"],
        "logit_scale": float(model.logit_scale.exp().item()),
        "text_pad_to": None,
    }

    # Export from one real clip.
    example = ear_score.Clap2Front(settings, filters, table)
    wave, rate = ear_audio.read_wav(sounds[0])
    feed = example.inputs(ear_audio.mono(wave), rate)
    torch.onnx.export(
        Audio(),
        tuple(torch.from_numpy(feed[k]) for k in ("patches", "patch_coord", "patch_valid", "rope")),
        str(out / "audio.onnx"),
        input_names=["patches", "patch_coord", "patch_valid", "rope"],
        output_names=["embedding"],
        dynamic_axes={k: {1: "n"} for k in ("patches", "patch_coord", "patch_valid")} | {"rope": {2: "n"}},
        opset_version=18,
        dynamo=False,
    )
    ids = torch.tensor([[50259, 32, 10752, 13, 50257]])
    torch.onnx.export(
        Text(), (ids,), str(out / "text.onnx"), input_names=["ids"], output_names=["embedding"],
        dynamic_axes={"ids": {0: "texts", 1: "tokens"}}, opset_version=18, dynamo=False,
    )

    def original_audio(path):
        a = preprocess()(load_audio(str(path)))
        holder.pop("rope", None)
        with torch.no_grad():
            # The original path, with timm's own RoPE codes.
            vit._generate_rope_naflex = original_rope
            try:
                return model.encode_audio({k: v[None] for k, v in a.items()}, normalize=True).numpy()[0]
            finally:
                vit._generate_rope_naflex = lambda x, coord: holder["rope"]

    original_rope = NaFlexVit._generate_rope_naflex.__get__(vit)

    def original_text(text):
        with torch.no_grad():
            return model.encode_text(tokenizer([text]), normalize=True).numpy()[0]

    return settings, original_audio, original_text


# ---------------------------------------------------------------------------
# LAION-CLAP (transformers).
# ---------------------------------------------------------------------------


def convert_laion(name, spec, source, out, sounds):
    from transformers import AutoTokenizer, ClapFeatureExtractor, ClapModel
    from open_clip.audio.dsp import resample as torch_resample
    import soundfile

    model = ClapModel.from_pretrained(str(source)).eval()
    extractor = ClapFeatureExtractor.from_pretrained(str(source))
    tokenizer = AutoTokenizer.from_pretrained(str(source))

    class Audio(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.model = model

        def forward(self, features):
            # features: [windows, 1, frames, mels]; one embedding per window
            longer = torch.zeros(features.shape[0], 1, dtype=torch.bool)
            out = self.model.get_audio_features(input_features=features, is_longer=longer)
            out = getattr(out, "pooler_output", out)
            return F.normalize(out, dim=-1)

    class Text(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.model = model

        def forward(self, ids, mask):
            out = self.model.get_text_features(input_ids=ids, attention_mask=mask)
            out = getattr(out, "pooler_output", out)
            return F.normalize(out, dim=-1)

    out.mkdir(parents=True, exist_ok=True)
    filters = extractor.mel_filters_slaney.astype(np.float32)
    np.save(out / "mel_filters.npy", filters)
    shutil.copy(source / "tokenizer.json", out / "tokenizer.json")
    settings = {
        "kind": "laion-clap",
        "sample_rate": extractor.sampling_rate,
        "n_fft": extractor.fft_window_size,
        "hop": extractor.hop_length,
        "mel_bins": extractor.feature_size,
        "window_seconds": extractor.max_length_s,
        "padding": extractor.padding,
        "logit_scale": float(model.logit_scale_a.exp().item()),
        "text_pad_to": None,
    }
    example = ear_score.LaionFront(settings, filters)
    wave, rate = ear_audio.read_wav(sounds[0])
    feed = example.inputs(ear_audio.mono(wave), rate)
    torch.onnx.export(
        Audio(), (torch.from_numpy(feed["features"]),), str(out / "audio.onnx"),
        input_names=["features"], output_names=["embedding"],
        dynamic_axes={"features": {0: "windows"}}, opset_version=18, dynamo=False,
    )
    enc = tokenizer(["A coin."], return_tensors="pt")
    torch.onnx.export(
        Text(), (enc["input_ids"], enc["attention_mask"]), str(out / "text.onnx"),
        input_names=["ids", "mask"], output_names=["embedding"],
        dynamic_axes={"ids": {0: "texts", 1: "tokens"}, "mask": {0: "texts", 1: "tokens"}},
        opset_version=18, dynamo=False,
    )

    window = settings["window_seconds"] * settings["sample_rate"]
    # The reference is a fresh copy of the original: tracing a transformers
    # model for ONNX can leave the traced copy changed (its text answers
    # moved by up to 0.12 in closeness when this check used it).
    model = ClapModel.from_pretrained(str(source)).eval()

    def original_audio(path):
        # The original feature extractor, on the same windows as the Ear
        # (a clip longer than one window is cut into windows, and their
        # embeddings averaged: the original picks one window at random).
        data, sr = soundfile.read(str(path), dtype="float32", always_2d=True)
        x = torch.from_numpy(data.T.mean(axis=0, keepdims=True))
        x = torch_resample(x, sr, settings["sample_rate"]).squeeze(0).numpy()
        pieces = [x[i : i + window] for i in range(0, max(len(x), 1), window)]
        pieces = [p for p in pieces if len(p) > 0]
        embeds = []
        with torch.no_grad():
            for piece in pieces:
                feats = extractor(piece, sampling_rate=settings["sample_rate"], return_tensors="pt")
                e = model.get_audio_features(input_features=feats["input_features"], is_longer=feats["is_longer"])
                e = getattr(e, "pooler_output", e)
                embeds.append(F.normalize(e, dim=-1).numpy()[0])
        mean = np.mean(embeds, axis=0)
        return mean / np.linalg.norm(mean)

    def original_text(text):
        with torch.no_grad():
            enc = tokenizer([text], return_tensors="pt")
            e = model.get_text_features(input_ids=enc["input_ids"], attention_mask=enc["attention_mask"])
            e = getattr(e, "pooler_output", e)
            return F.normalize(e, dim=-1).numpy()[0]

    return settings, original_audio, original_text


# ---------------------------------------------------------------------------
# Checking, and model.json.
# ---------------------------------------------------------------------------


def check(out, original_audio, original_text, sounds):
    """The Ear's path against the original's, on every sound and text."""
    ear = ear_score.ScoreModel(out, words=[])
    audio = []
    for path in sounds:
        wave, rate = ear_audio.read_wav(path)
        got = ear.embed_audio(ear_audio.mono(wave), rate)
        audio.append((path.name, cosine(got, original_audio(path))))
    texts = [(t, cosine(ear.embed_texts([t])[0], original_text(t))) for t in CHECK_TEXTS]
    worst_audio = min(audio, key=lambda a: a[1])
    worst_text = min(texts, key=lambda t: t[1])
    print("  audio: %d sounds, lowest closeness %.6f (%s)" % (len(audio), worst_audio[1], worst_audio[0]))
    print("  text: %d texts, lowest closeness %.6f (%r)" % (len(texts), worst_text[1], worst_text[0]))
    return {
        "sounds": len(audio),
        "lowest_audio_closeness": round(worst_audio[1], 6),
        "lowest_audio_sound": worst_audio[0],
        "texts": len(texts),
        "lowest_text_closeness": round(worst_text[1], 6),
        "enough": ENOUGH,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--sounds", required=True, help="a folder of WAV files to check with")
    where = parser.add_mutually_exclusive_group(required=True)
    where.add_argument("--out", help="where to write each model's folder")
    where.add_argument(
        "--install", action="store_true", help="convert the chosen model into the Ear's folder, as models/score"
    )
    parser.add_argument("--model", action="append", choices=sorted(MODELS), help="only this model (repeatable)")
    args = parser.parse_args()
    sounds = sounds_in(args.sounds)
    names = [CHOSEN] if args.install else args.model or sorted(MODELS)
    for name in names:
        spec = MODELS[name]
        print("%s: %s @ %s" % (name, spec["repo"], spec["revision"][:12]))
        source = download(spec)
        if args.install:
            from duck_ear.server import ear_home

            out = ear_home() / "models" / "score"
            out.parent.mkdir(parents=True, exist_ok=True)
        else:
            out = Path(args.out) / name
        if out.exists():
            shutil.rmtree(out)
        convert = convert_clap2 if spec["kind"] == "clap2" else convert_laion
        settings, original_audio, original_text = convert(name, spec, source, out, sounds)
        # model.json first without the check, so the Ear can load the model.
        info = {
            "name": name,
            "source": "https://huggingface.co/%s" % spec["repo"],
            "revision": spec["revision"],
            "licence": spec["licence"],
            "converted_by": "ear/convert/convert_clap.py",
            "settings": settings,
        }
        center = (spec["yes_z"] + spec["no_z"]) / 2
        logistic = lambda z: round(1.0 / (1.0 + math.exp(-STEEPNESS * (z - center))), 3)
        info["calibration"] = {
            "reference": "reference.npy: the embeddings of the sounds in --sounds (Duck Echo's presets and examples)",
            "center": center,
            "steepness": STEEPNESS,
        }
        info["lines"] = {"yes_from": logistic(spec["yes_z"]), "no_below": logistic(spec["no_z"])}
        (out / "model.json").write_text(json.dumps(info, indent=2) + "\n")
        # The reference sounds, through the Ear's own path.
        reference = ear_score.ScoreModel(out, words=[])
        np.save(out / "reference.npy", np.stack([
            reference.embed_audio(ear_audio.mono(w), r) for w, r in (ear_audio.read_wav(p) for p in sounds)
        ]).astype(np.float32))
        info["calibration"]["sounds"] = [p.name for p in sounds]
        result = check(out, original_audio, original_text, sounds)
        info["checked"] = result
        info["files"] = {f.name: sha256(f) for f in sorted(out.iterdir()) if f.name != "model.json"}
        info["bytes"] = sum(f.stat().st_size for f in out.iterdir() if f.name != "model.json")
        (out / "model.json").write_text(json.dumps(info, indent=2) + "\n")
        if min(result["lowest_audio_closeness"], result["lowest_text_closeness"]) < ENOUGH:
            sys.exit("%s: the conversion is not close enough to the original (needs %.3f)" % (name, ENOUGH))
        print("  ok: %s (%.0f MB)" % (out, info["bytes"] / 1e6))


if __name__ == "__main__":
    main()
