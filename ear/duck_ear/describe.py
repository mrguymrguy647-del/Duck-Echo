"""The describe model: `llama-server`, started and stopped by the Ear.

The describe model is a language model that hears: it says in words what
a sound is, and answers questions about it. It runs in `llama-server`
(llama.cpp), a separate program the Ear starts on this PC only
(127.0.0.1, `--offline`) and talks to over HTTP.

How a question is asked (docs/phase4-design.md, section 4):

* **Evidence first.** The model is first asked what it hears, with no
  hint of what we hope for. Only then, as a second turn of the same
  talk, comes the question ("Does this sound like a coin pickup?"). Asked
  the question straight away, these models hear whatever the question
  names.
* **The same words each time**: temperature 0, the most likely word each
  time, and a fixed seed.
* **A sound shorter than 1 second is padded with silence to 1 second.**
* **The graphics card is let go**: after `idle_seconds` with no describe
  question, `llama-server` is stopped, and started again by the next
  `load`.

Where things are: `llama-server` is the program `DUCK_EAR_LLAMA_SERVER`
names, else `llama-server` in `<the Ear's folder>/llama/` (or in a folder
inside it). The model is in the
folder `DUCK_EAR_DESCRIBE_MODEL` names, else `<the Ear's
folder>/models/<the name in models/describe.txt>`, with a `model.json`
(both written by `ear/tools/get_describe_model.py`). `DUCK_EAR_DEVICE=cpu` keeps the model
off the graphics card; `DUCK_EAR_DEVICE=Vulkan1` (a name from
`llama-server --list-devices`) picks a card.
"""

import base64
import hashlib
import http.client
import io
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import wave
from pathlib import Path

import numpy as np

from . import audio as ear_audio

# The version of the questions below. It is part of the model's revision
# in `hello`, so new wording misses Duck Echo's answer cache.
PROMPTS_VERSION = "1"
SEED = 42

# The first turn: what is heard, before any question names a sound. The
# length is a fact Duck Echo knows, and these models cannot hear it (they
# pad every sound to a fixed window).
HEAR = (
    "What do you hear? Describe the sound itself in one or two short sentences: "
    "what kind of sound it is, its pitch, and how it feels. (It is {seconds} seconds long.)"
)
# The second turn, around Duck Echo's yes or no question.
YES_NO = (
    "This sound was made for a video game. {question} "
    "Say yes only if that describes this sound well; say partly if only some of it fits."
)
# The second turn, around a free question.
FREE = "{question} Answer in at most three short sentences."

ANSWER_SHAPE = {
    "type": "object",
    "properties": {
        "answer": {"enum": ["yes", "no", "partly"]},
        "reason": {"type": "string", "maxLength": 200},
    },
    "required": ["answer", "reason"],
}

SILENT_PEAK = 10 ** (-60 / 20)
SILENT_TEXT = "Silence: nothing can be heard."
SILENT_REASON = "The sound is silent (its peak is below -60 dBFS), so the model was not asked."
SHORTEST = 1.0  # seconds: shorter sounds are padded with silence
STARTING = 170.0  # seconds to wait for llama-server to load the model


def starting_seconds():
    """How long to wait for llama-server to load the model: STARTING,
    made longer or shorter by `DUCK_ECHO_EAR_TIMEOUT_SCALE`, as Duck
    Echo's own time limits are (0.25 to 10)."""
    try:
        scale = float(os.environ.get("DUCK_ECHO_EAR_TIMEOUT_SCALE", "1"))
    except ValueError:
        scale = 1.0
    return STARTING * min(10.0, max(0.25, scale))


# Graphics built into the processor share the PC's memory: they report a
# lot of it and are slow. Names like these are not counted as a graphics
# card (a guess from the name; `DUCK_EAR_DEVICE` can still pick one).
SHARED_MEMORY_NAMES = ("uhd graphics", "iris", "hd graphics", "radeon(tm) graphics", "radeon graphics", "llvmpipe", "basic render")


def shares_memory(name):
    lower = name.lower()
    return any(part in lower for part in SHARED_MEMORY_NAMES)


class DescribeError(Exception):
    """A describe request failed: becomes an "error" answer."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def server_program(home):
    """Where `llama-server` is, or None."""
    named = os.environ.get("DUCK_EAR_LLAMA_SERVER")
    if named:
        return Path(named) if Path(named).is_file() else None
    name = "llama-server.exe" if os.name == "nt" else "llama-server"
    folder = Path(home) / "llama"
    if (folder / name).is_file():
        return folder / name
    # An unpacked download may keep its own folders (`llama/build/bin/`).
    try:
        inside = sorted(folder.rglob(name))
    except OSError:
        inside = []
    return inside[0] if inside else None


def model_folder(home):
    """Where the describe model is: the folder `DUCK_EAR_DESCRIBE_MODEL`
    names; else the folder in `models/` that `models/describe.txt` names
    (written by ear/tools/get_describe_model.py); else `models/describe`."""
    named = os.environ.get("DUCK_EAR_DESCRIBE_MODEL")
    if named:
        return Path(named)
    models = Path(home) / "models"
    try:
        chosen = (models / "describe.txt").read_text().strip()
    except OSError:
        chosen = ""
    # A plain folder name only: never a path out of the models folder.
    if chosen and chosen == Path(chosen).name and chosen not in (".", ".."):
        return models / chosen
    return models / "describe"


def read_info(folder):
    """The describe model's `model.json`, or None when there is no model."""
    try:
        info = json.loads((Path(folder) / "model.json").read_text())
    except (OSError, ValueError):
        return None
    files = info.get("files", {})
    if not all((Path(folder) / files.get(part, "?")).is_file() for part in ("model", "mmproj")):
        return None
    return info


def command_for(program):
    """How to run `program`: a `.py` file (the tests' pretend llama-server)
    runs in this Python."""
    program = str(program)
    return [sys.executable, program] if program.endswith(".py") else [program]


def environment_for(program):
    """The environment for llama-server: its own folder is where its
    libraries are (on Linux, a source build needs that said)."""
    env = dict(os.environ)
    if os.name != "nt":
        folder = str(Path(program).resolve().parent)
        env["LD_LIBRARY_PATH"] = folder + os.pathsep + env.get("LD_LIBRARY_PATH", "")
    return env


def devices(program):
    """The graphics cards llama-server can use: a list of
    `{"id", "name", "memory_mb", "free_mb"}`, the biggest first. Empty when
    there is none, or when llama-server cannot say."""
    try:
        out = subprocess.run(
            command_for(program) + ["--list-devices"],
            capture_output=True,
            text=True,
            timeout=30,
            env=environment_for(program),
            stdin=subprocess.DEVNULL,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    found = []
    for line in out.splitlines():
        match = re.match(r"\s+(\S+): (.*) \((\d+) MiB, (\d+) MiB free\)\s*$", line)
        if match:
            found.append(
                {
                    "id": match.group(1),
                    "name": match.group(2),
                    "memory_mb": int(match.group(3)),
                    "free_mb": int(match.group(4)),
                }
            )
    # Real cards first, then the biggest.
    return sorted(found, key=lambda d: (shares_memory(d["name"]), -d["memory_mb"]))


def cards(program):
    """The graphics cards worth using: `devices` without graphics that
    share the PC's memory. With `DUCK_EAR_DEVICE` naming one device, that
    one only; with `DUCK_EAR_DEVICE=cpu`, none."""
    wanted = os.environ.get("DUCK_EAR_DEVICE", "").strip()
    if wanted.lower() == "cpu":
        return []
    found = devices(program)
    if wanted:
        return [d for d in found if d["id"].lower() == wanted.lower()]
    return [d for d in found if not shares_memory(d["name"])]


def kind_of(device_id):
    """`Vulkan0` is `vulkan`, `CUDA0` is `cuda`."""
    return re.sub(r"\d+$", "", device_id).lower()


def prepared(path, rate=16000):
    """The sound as the model gets it: one channel at `rate`, padded with
    silence to at least one second, as 16-bit WAV bytes. Also its real
    length in seconds, and whether it is silent."""
    wave_in, rate_in = ear_audio.read_wav(path)
    x = ear_audio.mono(wave_in)
    seconds = len(x) / float(rate_in) if rate_in else 0.0
    silent = len(x) == 0 or float(np.max(np.abs(x))) < SILENT_PEAK
    x = ear_audio.resample(x, rate_in, rate) if len(x) else np.zeros(0, dtype=np.float32)
    shortest = int(SHORTEST * rate)
    if len(x) < shortest:
        x = np.pad(x, (0, shortest - len(x)))
    pcm = np.round(np.clip(x, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()
    out = io.BytesIO()
    with wave.open(out, "wb") as file:
        file.setnchannels(1)
        file.setsampwidth(2)
        file.setframerate(rate)
        file.writeframes(pcm)
    return out.getvalue(), seconds, silent


class Describer:
    """Keeps one `llama-server`, and asks it."""

    def __init__(self, program, folder, info, home, idle_seconds=120.0, log=print):
        self.program = Path(program)
        self.folder = Path(folder)
        self.info = info
        self.home = Path(home)
        self.idle_seconds = float(idle_seconds)
        self.log = log
        self.lock = threading.RLock()
        self.process = None
        self.port = None
        self.last_used = 0.0
        self.heard = {}  # what was heard, by the sound's SHA-256
        self.fell_back = None  # why the CPU is used though a card was found
        self.cards = None  # found when first needed
        self.closing = threading.Event()
        self.watcher = threading.Thread(target=self._watch, name="describe-idle", daemon=True)
        self.watcher.start()

    # -- what hello says ------------------------------------------------

    def found_cards(self):
        if self.cards is None:
            self.cards = cards(self.program)
        return self.cards

    def device(self):
        """(`cpu`, `vulkan` ..., a name in words, the card's memory in MB
        or None)."""
        cards = self.found_cards()
        if not cards or self.fell_back:
            name = "CPU"
            if self.fell_back:
                name += " (the graphics card could not be used: %s)" % self.fell_back
            elif os.environ.get("DUCK_EAR_DEVICE", "").strip():
                name += " (DUCK_EAR_DEVICE=%s)" % os.environ["DUCK_EAR_DEVICE"].strip()
            return "cpu", name, None
        card = cards[0]
        return kind_of(card["id"]), "%s (%d MB)" % (card["name"], card["memory_mb"]), card["memory_mb"]

    def revision(self):
        """Everything that changes an answer: the model's revision, its
        file, and the questions' version."""
        return "%s+%s+prompts%s" % (
            str(self.info.get("revision", "?"))[:12],
            Path(self.info["files"]["model"]).stem.split("-")[-1].lower(),
            PROMPTS_VERSION,
        )

    def loaded(self):
        with self.lock:
            return self.process is not None and self.process.poll() is None

    # -- starting and stopping -----------------------------------------

    def _watch(self):
        """Stop llama-server after `idle_seconds` with no describe
        question: the graphics card's memory is free again."""
        while not self.closing.wait(min(1.0, max(0.05, self.idle_seconds / 4))):
            with self.lock:
                if self.loaded() and time.monotonic() - self.last_used >= self.idle_seconds:
                    self.log(
                        "stopped the describe model after %.0f s with no describe question" % self.idle_seconds
                    )
                    self._stop()

    def _arguments(self, on_cpu):
        files = self.info["files"]
        settings = self.info.get("settings", {})
        arguments = [
            "-m", str(self.folder / files["model"]),
            "--mmproj", str(self.folder / files["mmproj"]),
            "--host", "127.0.0.1",
            "--port", str(self.port),
            "--offline",
            "--no-ui",
            "-c", str(settings.get("context", 4096)),
            "-np", "1",
            "--seed", str(SEED),
            "--cache-ram", str(settings.get("cache_ram_mb", 2048)),
        ]  # fmt: skip
        if on_cpu:
            arguments += ["--device", "none", "--no-mmproj-offload"]
        else:
            # One card, named: never spread over the processor's own graphics.
            arguments += ["--device", self.found_cards()[0]["id"]]
        return arguments + [str(a) for a in settings.get("server_arguments", [])]

    def _start(self, on_cpu):
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            self.port = probe.getsockname()[1]
        log_path = self.home / "llama-server.log"
        try:
            self.home.mkdir(parents=True, exist_ok=True)
            log_file = open(log_path, "ab")
        except OSError:
            log_file = subprocess.DEVNULL
        # Its own stdout and stderr (a file): it must never hold the Ear's
        # pipes, or Duck Echo could not tell that the Ear has stopped.
        try:
            self.process = subprocess.Popen(
                command_for(self.program) + self._arguments(on_cpu),
                stdin=subprocess.DEVNULL,
                stdout=log_file,
                stderr=subprocess.STDOUT,
                env=environment_for(self.program),
            )
        except OSError as error:
            raise DescribeError("model_missing", "cannot start %s: %s" % (self.program, error))
        finally:
            if log_file is not subprocess.DEVNULL:
                log_file.close()
        waiting = starting_seconds()
        deadline = time.monotonic() + waiting
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                code = self.process.returncode
                self.process = None
                return "llama-server stopped while loading the model (exit code %s; see %s)" % (code, log_path)
            try:
                status, _ = self._http("GET", "/health", None, 5.0)
                if status == 200:
                    return None
            except OSError:
                pass
            time.sleep(0.1)
        self._stop()
        return "llama-server did not load the model in %.0f s (see %s)" % (waiting, log_path)

    def load(self):
        """Start llama-server if it is not running. On a graphics card
        first; if that fails (too little memory, a bad driver), on the CPU."""
        with self.lock:
            self.last_used = time.monotonic()
            if self.loaded():
                return
            started = time.monotonic()
            on_cpu = not self.found_cards() or bool(self.fell_back)
            problem = self._start(on_cpu)
            if problem and not on_cpu:
                self.log("the graphics card could not be used (%s): trying the CPU" % problem)
                self.fell_back = problem
                problem = self._start(True)
            if problem:
                raise DescribeError("model_missing", problem)
            self.last_used = time.monotonic()
            self.log(
                "started llama-server with %s on %s in %.1f s"
                % (self.info.get("name"), self.device()[1], time.monotonic() - started)
            )

    def _stop(self):
        process, self.process = self.process, None
        if process is None or process.poll() is not None:
            return
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)

    def close(self):
        """Stop llama-server and the idle watcher: the Ear is ending."""
        self.closing.set()
        with self.lock:
            self._stop()

    # -- asking ----------------------------------------------------------

    def _http(self, method, path, body, timeout):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        try:
            data = None if body is None else json.dumps(body).encode()
            connection.request(method, path, data, {"Content-Type": "application/json"})
            answer = connection.getresponse()
            return answer.status, answer.read()
        finally:
            connection.close()

    def _chat(self, messages, shape=None, most_tokens=120):
        body = {
            "messages": messages,
            "temperature": 0,
            "top_k": 1,
            "seed": SEED,
            "max_tokens": most_tokens,
            # Keep what was worked out for this sound: the next question
            # about it does not listen to it all again.
            "cache_prompt": True,
        }
        if shape is not None:
            body["response_format"] = {"type": "json_schema", "json_schema": {"name": "answer", "schema": shape}}
        try:
            status, raw = self._http("POST", "/v1/chat/completions", body, 600.0)
        except OSError as error:
            raise DescribeError("describe_failed", "llama-server did not answer: %s" % error)
        try:
            said = json.loads(raw)
        except ValueError:
            said = {}
        if status != 200:
            message = said.get("error", {}).get("message") if isinstance(said, dict) else None
            raise DescribeError("describe_failed", "llama-server said %s: %s" % (status, message or raw[:200]))
        try:
            return said["choices"][0]["message"]["content"].strip()
        except (KeyError, IndexError, TypeError, AttributeError):
            raise DescribeError("describe_failed", "llama-server's answer has no text")

    def _first_turn(self, wav, seconds):
        return {
            "role": "user",
            "content": [
                {"type": "input_audio", "input_audio": {"data": base64.b64encode(wav).decode(), "format": "wav"}},
                {"type": "text", "text": HEAR.format(seconds="%.1f" % seconds)},
            ],
        }

    def _hear(self, wav, seconds):
        """What the model hears in the sound: the evidence, asked for
        before any question. Remembered while the Ear runs."""
        key = hashlib.sha256(wav).hexdigest()
        if key not in self.heard:
            if len(self.heard) >= 64:
                self.heard.pop(next(iter(self.heard)))
            self.heard[key] = " ".join(self._chat([self._first_turn(wav, seconds)]).split())
        return self.heard[key]

    def _ready(self):
        if not self.loaded():
            raise DescribeError(
                "model_not_loaded",
                'the describe model is not loaded: send load with roles ["describe"] first',
            )
        self.last_used = time.monotonic()

    def describe(self, path, guard=True):
        with self.lock:
            self._ready()
            wav, seconds, silent = prepared(path, self.info.get("settings", {}).get("sample_rate", 16000))
            if silent and guard:
                return {"text": SILENT_TEXT, "silent": True}
            text = self._hear(wav, seconds)
            self.last_used = time.monotonic()
            return {"text": text}

    def ask(self, path, question, kind="yes_no", guard=True):
        with self.lock:
            self._ready()
            wav, seconds, silent = prepared(path, self.info.get("settings", {}).get("sample_rate", 16000))
            if silent and guard:
                if kind == "free":
                    return {"text": SILENT_TEXT, "silent": True}
                answer = "yes" if "silen" in question.lower() else "no"
                return {"answer": answer, "reason": SILENT_REASON, "heard": SILENT_TEXT, "silent": True}
            heard = self._hear(wav, seconds)
            turns = [self._first_turn(wav, seconds), {"role": "assistant", "content": heard}]
            if kind == "free":
                turns.append({"role": "user", "content": FREE.format(question=question)})
                text = " ".join(self._chat(turns, most_tokens=160).split())
                self.last_used = time.monotonic()
                return {"text": text, "heard": heard}
            turns.append({"role": "user", "content": YES_NO.format(question=question)})
            # The answer's shape is forced (JSON), unless the model's
            # settings say `plain_answers`: some models think for hundreds
            # of words before a shaped answer, whatever they are told, but
            # answer "Yes, ..." at once when asked plainly.
            plain = bool(self.info.get("settings", {}).get("plain_answers"))
            said = self._chat(turns, None if plain else ANSWER_SHAPE, most_tokens=100)
            self.last_used = time.monotonic()
            answer, reason = read_answer(said)
            return {"answer": answer, "reason": reason, "heard": heard}


def read_answer(said):
    """The yes, no or partly and the reason in the model's words: from
    the forced shape (JSON), or from a plain answer that starts with one
    of the three words ("Yes, it is a short bright chime."). Failing
    both, the first yes, no or partly found anywhere counts."""
    try:
        parsed = json.loads(said)
        answer = str(parsed["answer"]).lower()
        reason = " ".join(str(parsed.get("reason", "")).split())
        if answer in ("yes", "no", "partly"):
            return answer, reason
    except (ValueError, KeyError, TypeError):
        pass
    plain = " ".join(said.split())
    start = re.match(r"\W*(yes|no|partly)\b[\s,.:;!*_-]*(.*)", plain, re.IGNORECASE)
    if start:
        reason = start.group(2).strip()
        return start.group(1).lower(), reason[:1].upper() + reason[1:]
    found = re.search(r"\b(yes|no|partly)\b", plain.lower())
    if not found:
        raise DescribeError("describe_failed", "the describe model gave no yes, no or partly: %r" % said[:200])
    return found.group(1), plain
