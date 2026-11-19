"""Tests for the duck_ear package, without its models.

Run them from the `ear` folder, in a Python that has the package's
libraries (numpy, onnxruntime, tokenizers):

    python -m unittest discover -s tests -v

The test that needs a real score model runs only when
DUCK_EAR_SCORE_MODEL names one (see ear/convert/README.md).
"""

import json
import math
import os
import socket
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

EAR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(EAR))

from duck_ear import audio  # noqa: E402
from duck_ear import score  # noqa: E402
from duck_ear.words import WORDS  # noqa: E402


def write_wav(path, frames, rate, kind, bits):
    """A WAV file of `frames` (float, shaped (frames, channels)), written by
    hand so the test does not trust the reader to check itself."""
    frames = np.asarray(frames, dtype=np.float64)
    channels = frames.shape[1]
    if kind == 3:
        data = frames.astype("<f4").tobytes()
    elif bits == 16:
        data = np.round(frames * 32767).astype("<i2").tobytes()
    elif bits == 24:
        ints = np.round(frames * 8388607).astype(np.int64).reshape(-1)
        data = b"".join(int(i).to_bytes(3, "little", signed=True) for i in ints)
    else:
        data = np.round(frames * 2147483647).astype("<i4").tobytes()
    block = channels * bits // 8
    fmt = struct.pack("<HHIIHH", kind, channels, rate, rate * block, block, bits)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    Path(path).write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)


class ReadingWavFiles(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        t = np.arange(441) / 44100
        self.frames = np.stack([0.5 * np.sin(2 * math.pi * 440 * t), -0.25 * np.ones_like(t)], axis=1)

    def check(self, kind, bits, closeness):
        path = os.path.join(self.folder.name, "sound.wav")
        write_wav(path, self.frames, 44100, kind, bits)
        x, rate = audio.read_wav(path)
        self.assertEqual(rate, 44100)
        self.assertEqual(x.shape, (2, 441))
        self.assertEqual(x.dtype, np.float32)
        self.assertLess(np.abs(x.T - self.frames).max(), closeness)

    def test_16_bit(self):
        self.check(1, 16, 1e-4)

    def test_24_bit(self):
        self.check(1, 24, 1e-6)

    def test_32_bit(self):
        self.check(1, 32, 1e-7)

    def test_32_bit_float(self):
        self.check(3, 32, 1e-7)

    def test_mono_is_the_mean_of_the_channels(self):
        x = np.array([[1.0, 0.0], [0.0, 0.0]], dtype=np.float32)
        self.assertEqual(list(audio.mono(x)), [0.5, 0.0])

    def test_not_a_wav_file(self):
        path = os.path.join(self.folder.name, "words.wav")
        Path(path).write_text("not a sound")
        with self.assertRaises(audio.AudioError):
            audio.read_wav(path)

    def test_a_missing_file(self):
        with self.assertRaises(audio.AudioError):
            audio.read_wav(os.path.join(self.folder.name, "nothing.wav"))


class Resampling(unittest.TestCase):
    def sine(self, rate, hz=1000.0, seconds=0.5):
        t = np.arange(int(rate * seconds)) / rate
        return np.sin(2 * math.pi * hz * t).astype(np.float32)

    def test_the_same_rate_changes_nothing(self):
        x = self.sine(48000)
        self.assertTrue(np.array_equal(audio.resample(x, 48000, 48000), x))

    def test_the_length_is_torchaudios(self):
        for orig, new, length in [(44100, 48000, 22050), (44100, 32000, 22050), (48000, 32000, 1001)]:
            y = audio.resample(np.zeros(length, dtype=np.float32), orig, new)
            self.assertEqual(len(y), math.ceil(new * length / orig))

    def test_a_tone_keeps_its_pitch_and_loudness(self):
        for new in (48000, 32000):
            y = audio.resample(self.sine(44100), 44100, new)
            middle = y[len(y) // 4 : 3 * len(y) // 4]  # away from the ends
            spectrum = np.abs(np.fft.rfft(middle * np.hanning(len(middle))))
            hz = np.argmax(spectrum) * new / len(middle)
            self.assertAlmostEqual(hz, 1000.0, delta=new / len(middle))
            self.assertAlmostEqual(float(np.abs(middle).max()), 1.0, delta=0.01)


class MelSpectrogram(unittest.TestCase):
    def test_shape_and_where_a_tone_is(self):
        n_fft, hop, rate = 1024, 320, 32000
        filters = np.eye(n_fft // 2 + 1, dtype=np.float32)  # one "mel" per FFT bin
        t = np.arange(rate) / rate
        x = np.sin(2 * math.pi * 1000 * t)
        mel = audio.log_mel(x, n_fft, hop, filters)
        self.assertEqual(mel.shape, (1 + len(x) // hop, n_fft // 2 + 1))
        self.assertEqual(int(np.argmax(mel[50])), round(1000 * n_fft / rate))

    def test_silence_is_the_floor(self):
        mel = audio.log_mel(np.zeros(4000), 512, 160, np.ones((257, 8), dtype=np.float32))
        self.assertTrue(np.all(mel == -100.0))


class ScoresYouCanRead(unittest.TestCase):
    def test_match_is_one_half_at_the_center(self):
        self.assertAlmostEqual(score.match_of(0.6, 0.6, 2.0), 0.5)
        self.assertGreater(score.match_of(1.6, 0.6, 2.0), 0.85)
        self.assertLess(score.match_of(-0.4, 0.6, 2.0), 0.15)

    def test_the_word_list(self):
        labels = [label for label, _ in WORDS]
        self.assertEqual(len(labels), len(set(labels)), "every word once")
        self.assertIn("silence", labels)
        self.assertGreaterEqual(len(WORDS), 80)


def start_ear(model_folder):
    env = dict(os.environ, DUCK_EAR_SCORE_MODEL=str(model_folder))
    return subprocess.Popen(
        [sys.executable, "-m", "duck_ear.server"],
        cwd=str(EAR),
        env=env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )


def ask(ear, request_id, method, params=None):
    ear.stdin.write(json.dumps({"id": request_id, "method": method, "params": params or {}}) + "\n")
    ear.stdin.flush()
    answer = json.loads(ear.stdout.readline())
    assert answer["id"] == request_id, answer
    return answer


class TheEarWithNoModel(unittest.TestCase):
    """The protocol, with no score model in its folder."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.ear = start_ear(self.folder.name)
        self.addCleanup(self.stop)

    def stop(self):
        if self.ear.poll() is None:
            self.ear.kill()
        self.ear.wait(timeout=30)
        for pipe in (self.ear.stdin, self.ear.stdout, self.ear.stderr):
            pipe.close()

    def test_hello_says_what_it_cannot_do(self):
        result = ask(self.ear, 0, "hello", {"protocol": 1})["result"]
        self.assertEqual(result["protocol"], 1)
        self.assertEqual(result["capabilities"], [])
        self.assertEqual(result["models"], [])
        self.assertIn("describe", result["not_available"])
        self.assertIn("ask", result["not_available"])
        self.assertNotIn("stub", result)

    def test_another_protocol_is_refused(self):
        self.assertEqual(ask(self.ear, 0, "hello", {"protocol": 99})["error"]["code"], "protocol")

    def test_score_with_no_model_says_where_it_looked(self):
        wav = os.path.join(self.folder.name, "a.wav")
        write_wav(wav, np.zeros((100, 1)), 44100, 1, 16)
        error = ask(self.ear, 1, "score", {"wav": wav, "texts": ["a coin"]})["error"]
        self.assertEqual(error["code"], "model_missing")
        self.assertIn("DUCK_EAR_SCORE_MODEL", error["message"])

    def test_describe_and_ask_are_not_available_yet(self):
        for i, method in enumerate(["describe", "ask"]):
            self.assertEqual(ask(self.ear, i, method, {"wav": "x.wav"})["error"]["code"], "not_available")
        self.assertEqual(ask(self.ear, 2, "load", {"roles": ["describe"]})["error"]["code"], "model_missing")

    def test_bad_requests(self):
        self.assertEqual(ask(self.ear, 0, "sing")["error"]["code"], "unknown_method")
        self.assertEqual(ask(self.ear, 1, "score", {"wav": 3})["error"]["code"], "bad_params")

    def test_bye_ends_it(self):
        self.assertEqual(ask(self.ear, 7, "bye")["result"], {})
        self.assertEqual(self.ear.wait(timeout=30), 0)

    def test_it_ends_when_stdin_closes(self):
        self.ear.stdin.close()
        self.assertEqual(self.ear.wait(timeout=30), 0)


GUARDED = """
import socket, sys
sys.path.insert(0, %r)
from duck_ear import guard
guard.refuse_the_network()
for name, try_it in [
    ("connect", lambda: socket.create_connection(("192.0.2.1", 80), timeout=2)),
    ("lookup", lambda: socket.getaddrinfo("example.com", 443)),
]:
    try:
        try_it()
        print(name, "allowed")
    except PermissionError:
        print(name, "refused")
    except OSError:
        print(name, "allowed")  # the guard let it through; the network said no
try:
    socket.create_connection(("127.0.0.1", 9), timeout=2)
    print("local", "allowed")
except PermissionError:
    print("local", "refused")
except OSError:
    print("local", "allowed")  # nothing listens there, or no network at all
"""


class NothingLeavesThePC(unittest.TestCase):
    def test_the_guard_refuses_the_network_but_not_this_pc(self):
        out = subprocess.run(
            [sys.executable, "-c", GUARDED % str(EAR)], capture_output=True, text=True, timeout=60
        )
        self.assertEqual(out.stdout.split("\n")[:3], ["connect refused", "lookup refused", "local allowed"], out.stderr)


@unittest.skipUnless(os.environ.get("DUCK_EAR_SCORE_MODEL"), "needs a score model (DUCK_EAR_SCORE_MODEL)")
class TheRealScoreModel(unittest.TestCase):
    def test_silence_and_noise(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        silent = os.path.join(folder.name, "silent.wav")
        noise = os.path.join(folder.name, "noise.wav")
        write_wav(silent, np.zeros((44100, 1)), 44100, 1, 16)
        write_wav(noise, np.random.default_rng(1).uniform(-0.5, 0.5, (44100, 1)), 44100, 1, 16)
        ear = start_ear(os.environ["DUCK_EAR_SCORE_MODEL"])
        try:
            hello = ask(ear, 0, "hello", {"protocol": 1})["result"]
            self.assertEqual(hello["capabilities"], ["score"])
            self.assertIn("yes_from", hello["models"][0])
            said = ask(ear, 1, "score", {"wav": silent, "texts": ["silence", "an explosion"]})["result"]
            self.assertEqual([s["match"] for s in said["scores"]], [1.0, 0.0])
            said = ask(ear, 2, "score", {"wav": noise, "texts": ["white noise", "a coin pickup"]})["result"]
            noise_match, coin_match = (s["match"] for s in said["scores"])
            self.assertGreater(noise_match, coin_match)
        finally:
            ear.stdin.close()
            ear.wait(timeout=60)
            for pipe in (ear.stdout, ear.stderr):
                pipe.close()


if __name__ == "__main__":
    unittest.main()
