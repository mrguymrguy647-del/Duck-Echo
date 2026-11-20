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
import time
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


class ReadingAYesOrNo(unittest.TestCase):
    def test_the_forced_shape(self):
        from duck_ear.describe import read_answer

        self.assertEqual(read_answer('{"answer": "partly", "reason": "Only  the start fits."}'), ("partly", "Only the start fits."))

    def test_a_plain_answer(self):
        from duck_ear.describe import read_answer

        self.assertEqual(read_answer("Yes, it is a short bright chime."), ("yes", "It is a short bright chime."))
        self.assertEqual(read_answer("**No.** The sound is far too long."), ("no", "The sound is far too long."))
        self.assertEqual(read_answer("I would say no here.")[0], "no")

    def test_no_answer_at_all_is_an_error(self):
        from duck_ear.describe import DescribeError, read_answer

        with self.assertRaises(DescribeError):
            read_answer("It is a beep.")


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


def start_ear(model_folder, **more):
    """Start the Ear with its score model in `model_folder` (and its own
    folder there too, so no installed model is found by accident)."""
    env = dict(os.environ, DUCK_EAR_SCORE_MODEL=str(model_folder), DUCK_ECHO_EAR_HOME=str(model_folder))
    for name in ("DUCK_EAR_DESCRIBE_MODEL", "DUCK_EAR_LLAMA_SERVER", "DUCK_EAR_DEVICE"):
        env.pop(name, None)
    env.update(more)
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

    def test_describe_and_ask_are_not_available_without_a_describe_model(self):
        for i, (method, params) in enumerate(
            [("describe", {"wav": "x.wav"}), ("ask", {"wav": "x.wav"}), ("load", {"roles": ["describe"]})]
        ):
            error = ask(self.ear, i, method, params)["error"]
            self.assertEqual(error["code"], "not_available")
            self.assertIn("scores only", error["message"])

    def test_bad_requests(self):
        self.assertEqual(ask(self.ear, 0, "sing")["error"]["code"], "unknown_method")
        self.assertEqual(ask(self.ear, 1, "score", {"wav": 3})["error"]["code"], "bad_params")

    def test_bye_ends_it(self):
        self.assertEqual(ask(self.ear, 7, "bye")["result"], {})
        self.assertEqual(self.ear.wait(timeout=30), 0)

    def test_it_ends_when_stdin_closes(self):
        self.ear.stdin.close()
        self.assertEqual(self.ear.wait(timeout=30), 0)


FAKE_LLAMA = Path(__file__).resolve().parent / "fake_llama_server.py"


def alive(pid):
    """Is the process `pid` still running?"""
    if os.name == "nt":
        out = subprocess.run(["tasklist", "/FI", "PID eq %d" % pid, "/NH"], capture_output=True, text=True).stdout
        return str(pid) in out
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    # A stopped child the Ear has not collected yet still has a pid.
    try:
        with open("/proc/%d/stat" % pid) as file:
            return file.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return True


def gone(pid, within=10.0):
    deadline = time.monotonic() + within
    while time.monotonic() < deadline:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return False


class TheDescribeModel(unittest.TestCase):
    """The Ear driving llama-server: here a pretend one, with no model."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        home = Path(self.folder.name)
        model = home / "models" / "describe"
        model.mkdir(parents=True)
        (model / "pretend.gguf").write_bytes(b"GGUF")
        (model / "mmproj-pretend.gguf").write_bytes(b"GGUF")
        (model / "model.json").write_text(
            json.dumps(
                {
                    "name": "pretend-model",
                    "size": "full",
                    "revision": "0123456789abcdef",
                    "files": {"model": "pretend.gguf", "mmproj": "mmproj-pretend.gguf"},
                }
            )
        )
        self.log = home / "fake-llama.log"
        self.coin = str(home / "coin.wav")
        t = np.arange(4410) / 44100  # a tenth of a second
        write_wav(self.coin, (0.5 * np.sin(2 * math.pi * 1000 * t))[:, None], 44100, 1, 16)
        self.ear = None

    def start(self, **more):
        self.ear = start_ear(
            self.folder.name, DUCK_EAR_LLAMA_SERVER=str(FAKE_LLAMA), FAKE_LLAMA_LOG=str(self.log), **more
        )
        self.addCleanup(self.stop)
        return self.ear

    def stop(self):
        # Politely (closing stdin), so the Ear stops its llama-server too.
        # Killed outright, only Duck Echo's process group or job object
        # would stop that one.
        if not self.ear.stdin.closed:
            self.ear.stdin.close()
        try:
            self.ear.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.ear.kill()
            self.ear.wait(timeout=30)
        for pipe in (self.ear.stdout, self.ear.stderr):
            pipe.close()

    def events(self, kind):
        if not self.log.exists():
            return []
        lines = [json.loads(line) for line in self.log.read_text().splitlines()]
        return [e for e in lines if e["event"] == kind]

    def test_hello_names_the_describe_model_and_its_size(self):
        hello = ask(self.start(), 0, "hello", {"protocol": 1})["result"]
        self.assertEqual(hello["capabilities"], ["describe", "ask"])
        self.assertNotIn("not_available", hello)
        self.assertEqual(hello["device"], "cpu")
        (model,) = hello["models"]
        self.assertEqual((model["role"], model["name"], model["size"]), ("describe", "pretend-model", "full"))
        self.assertEqual(model["revision"], "0123456789ab+pretend+prompts1")
        self.assertFalse(model["loaded"])
        self.assertEqual(self.events("start"), [], "hello starts no llama-server")

    def test_describe_needs_load_first(self):
        error = ask(self.start(), 0, "describe", {"wav": self.coin})["error"]
        self.assertEqual(error["code"], "model_not_loaded")

    def test_llama_server_is_started_for_this_pc_only_with_a_fixed_seed(self):
        ear = self.start()
        self.assertEqual(ask(ear, 0, "load", {"roles": ["describe"]})["result"]["loaded"], ["describe"])
        (start,) = self.events("start")
        arguments = start["arguments"]
        for option, value in [("--host", "127.0.0.1"), ("--seed", "42"), ("-np", "1"), ("--device", "none")]:
            self.assertEqual(arguments[arguments.index(option) + 1], value, option)
        self.assertIn("--offline", arguments)
        self.assertTrue(arguments[arguments.index("-m") + 1].endswith("pretend.gguf"))
        self.assertTrue(arguments[arguments.index("--mmproj") + 1].endswith("mmproj-pretend.gguf"))
        self.assertTrue(ask(ear, 1, "hello", {"protocol": 1})["result"]["models"][0]["loaded"])
        # A second load starts nothing new.
        ask(ear, 2, "load", {"roles": ["describe"]})
        self.assertEqual(len(self.events("start")), 1)

    def test_describe_asks_what_is_heard_with_no_hint(self):
        ear = self.start()
        ask(ear, 0, "load", {"roles": ["describe"]})
        said = ask(ear, 1, "describe", {"wav": self.coin})["result"]
        self.assertEqual(said["text"], "A pretend description.")
        self.assertEqual(said["model"], "pretend-model@0123456789ab+pretend+prompts1")
        (chat,) = self.events("chat")
        self.assertEqual((chat["temperature"], chat["top_k"], chat["seed"]), (0, 1, 42))
        (turn,) = chat["messages"]
        sound, words = turn["content"]
        # A tenth of a second is padded to one second, at 16 kHz, mono.
        self.assertEqual(sound["audio"], {"seconds": 1.0, "rate": 16000, "channels": 1})
        self.assertIn("What do you hear?", words)
        self.assertIn("0.1 seconds long", words)

    def test_a_question_comes_only_after_the_evidence(self):
        ear = self.start()
        ask(ear, 0, "load", {"roles": ["describe"]})
        question = "Does this sound like a coin pickup? Answer yes, no or partly, then say why in one sentence."
        said = ask(ear, 1, "ask", {"wav": self.coin, "question": question})["result"]
        self.assertEqual((said["answer"], said["reason"]), ("yes", "A pretend reason."))
        self.assertEqual(said["heard"], "A pretend description.")
        first, second = self.events("chat")
        # First the model says what it hears, and the question is not there.
        self.assertEqual(len(first["messages"]), 1)
        self.assertNotIn("coin", json.dumps(first["messages"]))
        # Then the question, after its own words.
        roles = [m["role"] for m in second["messages"]]
        self.assertEqual(roles, ["user", "assistant", "user"])
        self.assertEqual(second["messages"][0], first["messages"][0])
        self.assertEqual(second["messages"][1]["content"], "A pretend description.")
        self.assertIn(question, second["messages"][2]["content"])
        self.assertIn("response_format", second)
        # Another question about the same sound does not ask what is heard again.
        said = ask(ear, 2, "ask", {"wav": self.coin, "question": "Does this sound like an explosion?"})["result"]
        self.assertEqual(said["answer"], "no")
        self.assertEqual(len(self.events("chat")), 3)
        said = ask(ear, 3, "ask", {"wav": self.coin, "question": "Is it soft?", "answer": "free"})["result"]
        self.assertEqual(said["text"], "A pretend answer.")

    def test_silence_is_not_given_to_the_model_unless_asked(self):
        silent = str(Path(self.folder.name) / "silent.wav")
        write_wav(silent, np.zeros((22050, 1)), 44100, 1, 16)
        ear = self.start()
        ask(ear, 0, "load", {"roles": ["describe"]})
        said = ask(ear, 1, "describe", {"wav": silent})["result"]
        self.assertEqual((said["text"], said["silent"]), ("Silence: nothing can be heard.", True))
        said = ask(ear, 2, "ask", {"wav": silent, "question": "Does this sound like a coin pickup?"})["result"]
        self.assertEqual(said["answer"], "no")
        said = ask(ear, 3, "ask", {"wav": silent, "question": "Does this sound like silence?"})["result"]
        self.assertEqual(said["answer"], "yes")
        self.assertEqual(self.events("chat"), [])
        # The check turns the guard off, to see what the model itself says.
        said = ask(ear, 4, "describe", {"wav": silent, "guard": False})["result"]
        self.assertEqual(said["text"], "A pretend description.")
        self.assertEqual(len(self.events("chat")), 1)

    def test_the_model_is_let_go_after_a_while_and_comes_back(self):
        ear = self.start()
        ask(ear, 0, "hello", {"protocol": 1, "describe_idle_seconds": 0.4})
        ask(ear, 1, "load", {"roles": ["describe"]})
        ask(ear, 2, "describe", {"wav": self.coin})
        pid = self.events("start")[0]["pid"]
        self.assertTrue(alive(pid))
        self.assertTrue(gone(pid), "llama-server still runs after the idle time")
        self.assertFalse(ask(ear, 3, "hello", {"protocol": 1})["result"]["models"][0]["loaded"])
        self.assertEqual(ask(ear, 4, "describe", {"wav": self.coin})["error"]["code"], "model_not_loaded")
        ask(ear, 5, "load", {"roles": ["describe"]})
        self.assertEqual(ask(ear, 6, "describe", {"wav": self.coin})["result"]["text"], "A pretend description.")
        self.assertEqual(len(self.events("start")), 2)

    def test_llama_server_stops_when_the_ear_stops(self):
        ear = self.start()
        ask(ear, 0, "load", {"roles": ["describe"]})
        pid = self.events("start")[0]["pid"]
        ear.stdin.close()
        self.assertEqual(ear.wait(timeout=30), 0)
        self.assertTrue(gone(pid), "llama-server outlived the Ear")

    def test_a_graphics_card_is_used_when_there_is_one(self):
        card = "Vulkan0: Pretend Card (24564 MiB, 24000 MiB free)"
        ear = self.start(FAKE_LLAMA_DEVICES=card)
        hello = ask(ear, 0, "hello", {"protocol": 1})["result"]
        self.assertEqual((hello["device"], hello["device_memory_mb"]), ("vulkan", 24564))
        self.assertIn("Pretend Card", hello["device_name"])
        ask(ear, 1, "load", {"roles": ["describe"]})
        arguments = self.events("start")[0]["arguments"]
        self.assertEqual(arguments[arguments.index("--device") + 1], "Vulkan0")

    def test_graphics_that_share_the_pcs_memory_are_not_a_card(self):
        listed = "Vulkan0: Intel(R) UHD Graphics 770 (32000 MiB, 31000 MiB free)\nVulkan1: Pretend Card (8192 MiB, 8000 MiB free)"
        ear = self.start(FAKE_LLAMA_DEVICES=listed)
        hello = ask(ear, 0, "hello", {"protocol": 1})["result"]
        self.assertEqual(hello["device_memory_mb"], 8192)
        ask(ear, 1, "load", {"roles": ["describe"]})
        arguments = self.events("start")[0]["arguments"]
        self.assertEqual(arguments[arguments.index("--device") + 1], "Vulkan1")

    def test_only_shared_graphics_means_the_cpu_unless_one_is_named(self):
        listed = "Vulkan0: Intel(R) UHD Graphics 770 (32000 MiB, 31000 MiB free)"
        ear = self.start(FAKE_LLAMA_DEVICES=listed)
        self.assertEqual(ask(ear, 0, "hello", {"protocol": 1})["result"]["device"], "cpu")
        self.stop()
        ear = self.start(FAKE_LLAMA_DEVICES=listed, DUCK_EAR_DEVICE="vulkan0")
        self.assertEqual(ask(ear, 0, "hello", {"protocol": 1})["result"]["device"], "vulkan")

    def test_cpu_only_can_be_asked_for(self):
        card = "Vulkan0: Pretend Card (24564 MiB, 24000 MiB free)"
        ear = self.start(FAKE_LLAMA_DEVICES=card, DUCK_EAR_DEVICE="cpu")
        self.assertEqual(ask(ear, 0, "hello", {"protocol": 1})["result"]["device"], "cpu")
        ask(ear, 1, "load", {"roles": ["describe"]})
        arguments = self.events("start")[0]["arguments"]
        self.assertEqual(arguments[arguments.index("--device") + 1], "none")

    def test_a_card_that_fails_falls_back_to_the_cpu(self):
        card = "Vulkan0: Small Card (2048 MiB, 100 MiB free)"
        ear = self.start(FAKE_LLAMA_DEVICES=card, FAKE_LLAMA_FAIL_ON_CARD="1")
        self.assertEqual(ask(ear, 0, "hello", {"protocol": 1})["result"]["device"], "vulkan")
        self.assertEqual(ask(ear, 1, "load", {"roles": ["describe"]})["result"]["loaded"], ["describe"])
        first, second = self.events("start")
        device = lambda arguments: arguments[arguments.index("--device") + 1]
        self.assertEqual((device(first["arguments"]), device(second["arguments"])), ("Vulkan0", "none"))
        hello = ask(ear, 2, "hello", {"protocol": 1})["result"]
        self.assertEqual(hello["device"], "cpu")
        self.assertIn("could not be used", hello["device_name"])
        self.assertEqual(ask(ear, 3, "describe", {"wav": self.coin})["result"]["text"], "A pretend description.")

    def test_llama_server_is_found_in_the_ears_folder_or_a_folder_inside(self):
        from duck_ear.describe import server_program

        home = Path(self.folder.name)
        name = "llama-server.exe" if os.name == "nt" else "llama-server"
        saved = os.environ.pop("DUCK_EAR_LLAMA_SERVER", None)
        try:
            self.assertIsNone(server_program(home))
            inside = home / "llama" / "build" / "bin"
            inside.mkdir(parents=True)
            (inside / name).write_bytes(b"")
            self.assertEqual(server_program(home), inside / name)
            (home / "llama" / name).write_bytes(b"")
            self.assertEqual(server_program(home), home / "llama" / name)
        finally:
            if saved is not None:
                os.environ["DUCK_EAR_LLAMA_SERVER"] = saved

    def test_a_model_with_no_llama_server_says_so(self):
        self.ear = start_ear(self.folder.name)
        self.addCleanup(self.stop)
        hello = ask(self.ear, 0, "hello", {"protocol": 1})["result"]
        self.assertEqual(hello["capabilities"], [])
        self.assertIn("no llama-server", hello["not_available"]["describe"])


TOOLS = EAR / "tools"
FAKE_DUCK_ECHO = Path(__file__).resolve().parent / "fake_duck_echo.py"


class MeasuringInOneCommand(unittest.TestCase):
    """ear/tools/measure_describe_model.py, with a pretend llama-server, a
    pretend duck-echo and a pretend model: nothing is downloaded."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.home = Path(self.folder.name)
        self.model = self.home / "a-model"
        self.model.mkdir()
        (self.model / "pretend.gguf").write_bytes(b"GGUF")
        (self.model / "mmproj-pretend.gguf").write_bytes(b"GGUF")
        (self.model / "model.json").write_text(
            json.dumps({"name": "pretend-model", "files": {"model": "pretend.gguf", "mmproj": "mmproj-pretend.gguf"}})
        )
        score = self.home / "models" / "score"
        score.mkdir(parents=True)
        (score / "model.json").write_text("{}")
        self.log = self.home / "duck-echo.log"

    def measure(self, *arguments, **more):
        env = dict(
            os.environ,
            DUCK_ECHO_EAR_HOME=str(self.home),
            DUCK_EAR_LLAMA_SERVER=str(FAKE_LLAMA),
            DUCK_ECHO_EAR=str(EAR / "stub" / "duck_ear_stub.py"),
            FAKE_DUCK_ECHO_LOG=str(self.log),
        )
        for name in ("DUCK_EAR_SCORE_MODEL", "DUCK_EAR_DESCRIBE_MODEL", "DUCK_EAR_DEVICE", "DUCK_ECHO_EAR_TIMEOUT_SCALE"):
            env.pop(name, None)
        env.update(more)
        return subprocess.run(
            [sys.executable, str(TOOLS / "measure_describe_model.py"), "--duck-echo", str(FAKE_DUCK_ECHO), *arguments],
            env=env,
            cwd=str(self.home),
            capture_output=True,
            text=True,
            timeout=120,
            stdin=subprocess.DEVNULL,
        )

    def started_with(self):
        return json.loads(self.log.read_text())

    def test_it_runs_the_describe_check_fresh_and_says_what_to_send(self):
        done = self.measure("--model-folder", str(self.model))
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        started = self.started_with()
        arguments = started["arguments"]
        self.assertEqual(arguments[:5], ["ear", "check", "--part", "describe", "--fresh"])
        self.assertTrue(arguments[arguments.index("--testset") + 1].endswith("testset.toml"))
        out = Path(arguments[arguments.index("--out") + 1])
        self.assertEqual(out.name, "ear-check-pretend-model")
        self.assertEqual(Path(started["environment"]["DUCK_EAR_DESCRIBE_MODEL"]), self.model.resolve())
        self.assertIsNone(started["environment"]["DUCK_EAR_DEVICE"])
        self.assertEqual(started["environment"]["DUCK_ECHO_EAR_TIMEOUT_SCALE"], "3")
        self.assertIn("measured on:  the CPU", done.stdout)
        self.assertIn("Describe: yes on true texts", done.stdout)
        self.assertIn("Send back these two files:", done.stdout)
        self.assertIn(str((self.home / out / "ear-check.json").resolve()), done.stdout)
        # It does not change the model the Ear uses.
        self.assertFalse((self.home / "models" / "describe.txt").exists())

    def test_quick_and_cpu(self):
        card = "Vulkan0: Pretend Card (24564 MiB, 24000 MiB free)"
        done = self.measure("--model-folder", str(self.model), "--quick", "--cpu", FAKE_LLAMA_DEVICES=card)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        started = self.started_with()
        arguments = started["arguments"]
        self.assertTrue(arguments[arguments.index("--testset") + 1].endswith("quick.toml"))
        self.assertEqual(Path(arguments[arguments.index("--out") + 1]).name, "ear-check-pretend-model-cpu-quick")
        self.assertEqual(started["environment"]["DUCK_EAR_DEVICE"], "cpu")
        self.assertIn("measured on:  the CPU", done.stdout)

    def test_it_names_the_graphics_card(self):
        card = "Vulkan0: Pretend Card (24564 MiB, 24000 MiB free)"
        done = self.measure("--model-folder", str(self.model), FAKE_LLAMA_DEVICES=card)
        self.assertIn("measured on:  Pretend Card (24564 MB)", done.stdout)

    def test_no_llama_server_is_said_before_anything_is_downloaded(self):
        done = self.measure("full", DUCK_EAR_LLAMA_SERVER=str(self.home / "nothing-here"))
        self.assertEqual(done.returncode, 1)
        self.assertIn("There is no llama-server", done.stdout)
        self.assertIn("llama.cpp/releases", done.stdout)
        self.assertFalse(self.log.exists())

    def test_no_score_model_is_said(self):
        (self.home / "models" / "score" / "model.json").unlink()
        done = self.measure("--model-folder", str(self.model))
        self.assertEqual(done.returncode, 1)
        self.assertIn("There is no score model", done.stdout)

    def test_large_is_not_offered_without_a_big_card(self):
        done = self.measure("large", "--yes")
        self.assertEqual(done.returncode, 1)
        self.assertIn("needs a graphics card with 16 GB or more (found none)", done.stdout)
        self.assertFalse(self.log.exists(), "nothing was measured")
        self.assertFalse((self.home / "models" / "qwen3-omni-30b-a3b" / "model.json").exists())


class TheQuickTestSet(unittest.TestCase):
    def test_it_is_ten_sounds_of_the_whole_set_with_the_same_texts(self):
        try:
            import tomllib
        except ImportError:  # Python 3.10
            self.skipTest("needs Python 3.11 or newer to read TOML")

        read = lambda name: tomllib.loads((EAR / "testset" / name).read_text())["sound"]
        whole = {sound["file"]: sound for sound in read("testset.toml")}
        quick = read("quick.toml")
        self.assertEqual(len(quick), 10)
        for sound in quick:
            self.assertEqual(sound, whole[sound["file"]])
        self.assertTrue({"silence", "noise.toml"} <= {sound["file"] for sound in quick})


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
