"""Reading a WAV file and getting it ready for a model, with numpy only.

The models were trained on audio prepared by PyTorch code (OpenCLIP's
`audio/dsp.py`, and transformers' CLAP feature extractor). The Ear does
the same sums here in numpy, so it needs no PyTorch: the converter checks
that both give the same numbers (see ear/convert/convert_clap.py).
"""

import math
import struct

import numpy as np


class AudioError(Exception):
    """A file that is not a WAV file the Ear can read."""


def read_wav(path):
    """The sound in a WAV file: (samples as float32 in -1..1, shaped
    (channels, frames), sample rate). Reads 16, 24 and 32-bit PCM and 32-bit
    float, which is everything Duck Echo writes."""
    try:
        with open(path, "rb") as file:
            data = file.read()
    except OSError as error:
        raise AudioError("cannot read %s: %s" % (path, error))
    if len(data) < 12 or data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise AudioError("%s is not a WAV file" % path)
    fmt = None
    samples = None
    at = 12
    while at + 8 <= len(data):
        chunk, size = struct.unpack_from("<4sI", data, at)
        body = data[at + 8 : at + 8 + size]
        if chunk == b"fmt ":
            fmt = list(struct.unpack_from("<HHIIHH", body, 0))
            if fmt[0] == 0xFFFE and len(body) >= 26:
                # WAVE_FORMAT_EXTENSIBLE: the real kind starts the sub-format.
                fmt[0] = struct.unpack_from("<H", body, 24)[0]
        elif chunk == b"data":
            samples = body
        at += 8 + size + (size & 1)
    if fmt is None or samples is None:
        raise AudioError("%s has no sound in it (no fmt or data chunk)" % path)
    kind, channels, rate, _, _, bits = fmt
    if channels < 1:
        raise AudioError("%s has no channels" % path)
    if kind == 1 and bits == 16:
        x = np.frombuffer(samples[: len(samples) // 2 * 2], dtype="<i2").astype(np.float32) / 32768.0
    elif kind == 1 and bits == 24:
        raw = np.frombuffer(samples[: len(samples) // 3 * 3], dtype=np.uint8).reshape(-1, 3)
        ints = (raw[:, 0].astype(np.int32) | (raw[:, 1].astype(np.int32) << 8) | (raw[:, 2].astype(np.int32) << 16))
        ints = np.where(ints >= 1 << 23, ints - (1 << 24), ints)
        x = ints.astype(np.float32) / float(1 << 23)
    elif kind == 1 and bits == 32:
        x = np.frombuffer(samples[: len(samples) // 4 * 4], dtype="<i4").astype(np.float32) / float(1 << 31)
    elif kind == 3 and bits == 32:
        x = np.frombuffer(samples[: len(samples) // 4 * 4], dtype="<f4").astype(np.float32)
    else:
        raise AudioError("%s is %d-bit audio of kind %d, which the Ear cannot read" % (path, bits, kind))
    frames = len(x) // channels
    x = x[: frames * channels].reshape(frames, channels).T
    return np.ascontiguousarray(x), rate


def mono(x):
    """One channel: the mean of all channels."""
    return x.mean(axis=0) if x.ndim == 2 else x


def _sinc_kernel(orig, new, width_zeros=6, rolloff=0.99):
    """The resampling filter: a Hann-windowed sinc, as torchaudio's
    `sinc_interp_hann` (OpenCLIP's `_sinc_resample_kernel`), in float64."""
    base = min(orig, new) * rolloff
    width = math.ceil(width_zeros * orig / base)
    idx = np.arange(-width, width + orig, dtype=np.float64)[None, :] / orig
    t = np.arange(0, -new, -1, dtype=np.float64)[:, None] / new + idx
    t *= base
    t = np.clip(t, -width_zeros, width_zeros)
    window = np.cos(t * math.pi / width_zeros / 2) ** 2
    t *= math.pi
    with np.errstate(invalid="ignore", divide="ignore"):
        kernel = np.where(t == 0, 1.0, np.sin(t) / t)
    kernel *= window * (base / orig)
    return kernel, width


def resample(x, orig_rate, new_rate):
    """`x` (one channel) from `orig_rate` to `new_rate`, the same way as
    OpenCLIP's `resample` (and torchaudio's)."""
    if orig_rate == new_rate:
        return x.astype(np.float32)
    gcd = math.gcd(int(orig_rate), int(new_rate))
    orig, new = int(orig_rate) // gcd, int(new_rate) // gcd
    kernel, width = _sinc_kernel(orig, new)
    length = len(x)
    padded = np.pad(x.astype(np.float64), (width, width + orig))
    span = kernel.shape[1]
    count = (len(padded) - span) // orig + 1
    frames = np.lib.stride_tricks.sliding_window_view(padded, span)[::orig][:count]
    out = (frames @ kernel.T).reshape(-1)
    # The same length as torchaudio: ceil in float32.
    keep = int(np.ceil(np.float32(new * length / orig)))
    return out[:keep].astype(np.float32)


def log_mel(x, n_fft, hop, filters):
    """A power mel spectrogram in dB, shaped (frames, mels): a centred STFT
    with reflect padding and a periodic Hann window, |X|^2, the mel
    `filters` (shaped (n_fft // 2 + 1, mels)), then 10 * log10(max(., 1e-10)).
    The same sums as OpenCLIP's `log_mel_spectrogram` and transformers'
    `spectrogram(..., log_mel="dB")`."""
    x = x.astype(np.float64)
    half = n_fft // 2
    if len(x) <= half:
        x = np.pad(x, (0, half + 1 - len(x)))
    padded = np.pad(x, (half, half), mode="reflect")
    window = 0.5 - 0.5 * np.cos(2 * math.pi * np.arange(n_fft) / n_fft)
    count = 1 + (len(padded) - n_fft) // hop
    frames = np.lib.stride_tricks.sliding_window_view(padded, n_fft)[::hop][:count]
    power = np.abs(np.fft.rfft(frames * window, axis=1)) ** 2
    mel = power @ filters.astype(np.float64)
    return (10.0 * np.log10(np.maximum(mel, 1e-10))).astype(np.float32)
