"""The score model: how well a text fits a sound.

A CLAP-style model turns a sound into a list of numbers (an embedding) and
a text into another; the closer the two point, the better the text fits.
The model's folder (made by ear/convert/convert_clap.py) holds:

    model.json        name, source, revision, licence, settings, SHA-256s
    audio.onnx        mel spectrogram -> sound embedding
    text.onnx         token ids -> text embedding
    tokenizer.json    the model's own tokenizer
    mel_filters.npy   the mel filters its audio was trained with
    rope.npy          (CLAP2 only) the table of its position codes
    reference.npy     the embeddings of Duck Echo's presets and examples

The raw closeness of a CLAP model is not a probability and is not easy to
read (docs/phase4-design.md, section 4, "Scores you can read"). Some texts
are close to almost every sound, others to almost none. So each text is
measured against the reference sounds first: how much closer it is to
this sound than to the usual game sound, in standard deviations (a
z-score). `match` is that number put on a 0 to 1 scale (see `match_of`).
The test set (docs/ear-check.md) showed this reads far better than the
raw closeness or a share among the Ear's words.
"""

import json
import math
import os
from pathlib import Path

import numpy as np

from . import audio as ear_audio
from .words import WORDS

# Where yes starts and no ends, when model.json does not say (it does, once
# the test set has been measured: see docs/ear-check.md).
YES_FROM = 0.6
NO_BELOW = 0.3


class Clap2Front:
    """CLAP2's input: a 128-band log-mel at 32 kHz, cut into 16 x 16
    patches (as OpenCLIP's `AudioNaFlexPatchify`), with their position
    codes taken from the table the converter saved."""

    def __init__(self, settings, filters, rope_table):
        self.s = settings
        self.filters = filters
        self.rope = rope_table
        self.freq_rows = settings["mel_bins"] // settings["patch_freq"]
        self.max_time = settings["max_patches"] // self.freq_rows

    def inputs(self, x, rate):
        s = self.s
        x = ear_audio.resample(x, rate, s["sample_rate"])
        if len(x) < s["n_fft"]:
            x = np.pad(x, (0, s["n_fft"] - len(x)))
        mel = ear_audio.log_mel(x, s["n_fft"], s["hop"], self.filters)  # (T, mels)
        pt, pf, rows = s["patch_time"], s["patch_freq"], self.freq_rows
        mel = mel[: self.max_time * pt]
        frames = mel.shape[0]
        tt = max(1, math.ceil(frames / pt))
        fill = tt * pt - frames
        if fill > 0:
            mode = s.get("patch_pad_mode", "floor")
            if mode == "repeat" and frames > 0:
                tail = np.repeat(mel[-1:], fill, axis=0)
            else:
                value = float(mel.min()) if (mode == "floor" and frames > 0) else -100.0
                tail = np.full((fill, mel.shape[1]), value, dtype=np.float32)
            mel = np.concatenate([mel, tail], axis=0)
        grid = mel.reshape(1, tt, pt, rows, pf).transpose(3, 1, 0, 4, 2)  # (F, Tt, C, pf, pt)
        patches = grid.reshape(rows * tt, pf * pt).astype(np.float32)
        freq = np.repeat(np.arange(rows), tt)
        time = np.tile(np.arange(tt), rows)
        coord = np.stack([freq, time], axis=1).astype(np.int64)
        rope = self.rope[freq * self.max_time + time]
        return {
            "patches": patches[None],
            "patch_coord": coord[None],
            "patch_valid": np.ones((1, rows * tt), dtype=bool),
            "rope": rope[None, None].astype(np.float32),
        }


class LaionFront:
    """LAION-CLAP's input: a 64-band log-mel (Slaney filters) at 48 kHz of
    each 10 s window; a shorter sound is repeated, then padded with
    silence, to fill its window (the model's own "repeatpad")."""

    def __init__(self, settings, filters):
        self.s = settings
        self.filters = filters

    def inputs(self, x, rate):
        s = self.s
        x = ear_audio.resample(x, rate, s["sample_rate"])
        window = s["window_seconds"] * s["sample_rate"]
        pieces = [x[i : i + window] for i in range(0, max(len(x), 1), window)] or [x]
        features = []
        for piece in pieces:
            if len(piece) == 0:
                piece = np.zeros(1, dtype=np.float32)
            if len(piece) < window:
                if s.get("padding", "repeatpad") == "repeatpad":
                    piece = np.tile(piece, int(window / len(piece)))
                piece = np.pad(piece, (0, window - len(piece)))
            mel = ear_audio.log_mel(piece, s["n_fft"], s["hop"], self.filters)
            features.append(mel[None])
        return {"features": np.stack(features).astype(np.float32)}


# A sound this quiet (its loudest sample under -60 dBFS) is silence: the
# models were not trained on it and say "music" or "explosion" to it, so
# the Ear answers without them.
SILENT_PEAK = 10 ** (-60 / 20)


def match_of(z, center, steepness):
    """`match`, 0 to 1, from a text's z-score for a sound: 0.5 at `center`,
    rising with `steepness` (a logistic curve)."""
    return float(1.0 / (1.0 + math.exp(-steepness * (z - center))))


class ScoreModel:
    """A score model, loaded from its folder, ready to embed sounds and
    texts and to score texts against a sound."""

    def __init__(self, folder, words=None):
        import onnxruntime as ort
        import tokenizers

        folder = Path(folder)
        info = json.loads((folder / "model.json").read_text())
        self.info = info
        self.name = info["name"]
        self.revision = info["revision"]
        self.settings = info["settings"]
        self.scale = float(self.settings["logit_scale"])
        lines = info.get("lines", {})
        self.yes_from = float(lines.get("yes_from", YES_FROM))
        self.no_below = float(lines.get("no_below", NO_BELOW))
        calibration = info.get("calibration", {})
        self.center = float(calibration.get("center", 0.6))
        self.steepness = float(calibration.get("steepness", 2.0))
        options = ort.SessionOptions()
        options.intra_op_num_threads = max(1, min(4, os.cpu_count() or 1))
        options.log_severity_level = 3
        providers = ["CPUExecutionProvider"]
        self.audio_session = ort.InferenceSession(str(folder / "audio.onnx"), options, providers=providers)
        self.text_session = ort.InferenceSession(str(folder / "text.onnx"), options, providers=providers)
        self.audio_inputs = [i.name for i in self.audio_session.get_inputs()]
        self.text_inputs = [i.name for i in self.text_session.get_inputs()]
        self.tokenizer = tokenizers.Tokenizer.from_file(str(folder / "tokenizer.json"))
        filters = np.load(folder / "mel_filters.npy")
        if self.settings["kind"] == "clap2":
            self.front = Clap2Front(self.settings, filters, np.load(folder / "rope.npy"))
        else:
            self.front = LaionFront(self.settings, filters)
        reference = folder / "reference.npy"
        self.reference = np.load(reference) if reference.exists() else None
        self._texts = {}
        self.words = list(WORDS if words is None else words)
        self.word_embeddings = self.embed_texts([w[1] for w in self.words]) if self.words else None

    def embed_audio(self, x, rate):
        """The sound's embedding (one channel `x` at `rate`), length 1."""
        feed = self.front.inputs(x, rate)
        # Only what the graph takes (the exporter drops inputs it never
        # uses, such as CLAP2's patch positions once RoPE is an input).
        feed = {k: v for k, v in feed.items() if k in self.audio_inputs}
        out = self.audio_session.run(None, feed)[0]
        e = out.mean(axis=0) if out.shape[0] > 1 else out[0]
        return e / np.linalg.norm(e)

    def embed_texts(self, texts):
        """Each text's embedding, length 1, kept for the next time."""
        result = []
        for text in texts:
            if text not in self._texts:
                ids = np.array([self.tokenizer.encode(text).ids], dtype=np.int64)
                feed = {"ids": ids}
                if "mask" in self.text_inputs:
                    feed["mask"] = np.ones_like(ids)
                e = self.text_session.run(None, feed)[0][0]
                self._texts[text] = e / np.linalg.norm(e)
            result.append(self._texts[text])
        return np.stack(result) if result else np.zeros((0, 1), dtype=np.float32)

    def z_scores(self, sound, embeddings):
        """How much closer each text is to `sound` than to the reference
        sounds, in standard deviations."""
        closeness = embeddings @ sound
        if self.reference is None:
            return closeness, closeness
        usual = embeddings @ self.reference.T  # texts x reference sounds
        z = (closeness - usual.mean(axis=1)) / (usual.std(axis=1) + 1e-6)
        return z, closeness

    def score(self, x, rate, texts, closest=5):
        """Each text's `match` and `rank` among the Ear's words, and the
        words that fit the sound best. A silent sound gets no match for
        anything but silence, without asking the model."""
        if len(x) == 0 or float(np.max(np.abs(x))) < SILENT_PEAK:
            return self._silent(texts)
        sound = self.embed_audio(x, rate)
        word_z, _ = self.z_scores(sound, self.word_embeddings)
        scores = []
        if texts:
            embeddings = self.embed_texts(texts)
            text_z, closeness = self.z_scores(sound, embeddings)
            for i, text in enumerate(texts):
                # Words that mean the same as the text are left out of its
                # rank: they would only tie with it.
                same = self.word_embeddings @ embeddings[i] >= 0.9
                others = word_z[~same]
                scores.append(
                    {
                        "text": text,
                        "match": round(match_of(text_z[i], self.center, self.steepness), 4),
                        "rank": int((others > text_z[i]).sum()) + 1,
                        "of": int(len(others)) + 1,
                        "z": round(float(text_z[i]), 3),
                        "closeness": round(float(closeness[i]), 4),
                    }
                )
        order = np.argsort(-word_z)[:closest]
        best = [
            {"text": self.words[i][0], "match": round(match_of(word_z[i], self.center, self.steepness), 4)}
            for i in order
        ]
        return {"scores": scores, "closest": best}

    def _silent(self, texts):
        silence = self.embed_texts([w[1] for w in self.words if w[0] == "silence"])[0]
        scores = []
        for text, embedding in zip(texts, self.embed_texts(texts) if texts else []):
            means_silence = float(embedding @ silence) >= 0.9 or "silen" in text.lower()
            scores.append({"text": text, "match": 1.0 if means_silence else 0.0, "rank": None, "of": None})
        return {"scores": scores, "closest": [{"text": "silence", "match": 1.0}], "silent": True}
