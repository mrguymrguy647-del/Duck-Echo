"""The Duck Ear's main loop: JSON Lines in on stdin, out on stdout.

The protocol is in docs/phase4-design.md, section 3 (and the stub,
ear/stub/duck_ear_stub.py, answers the same way without models). The
score model answers `score`; the describe model, when one is installed
with `llama-server`, answers `describe` and `ask` (see describe.py).
Without one they answer `not_available`, and `hello` says why. Like every
Ear, it exits as soon as its stdin closes (and stops `llama-server`
first), and it writes its log to stderr, never to stdout.

Where the score model is: the folder `DUCK_EAR_SCORE_MODEL` names, else
`<the Ear's folder>/models/score`, where the Ear's folder is
`DUCK_ECHO_EAR_HOME`, else the usual place (the same rule as Duck Echo's).
Where the describe model and `llama-server` are: see describe.py.
"""

import hashlib
import json
import os
import sys
import time
from pathlib import Path

from . import __version__
from . import audio as ear_audio
from . import describe as ear_describe
from . import guard
from .words import WORDS_VERSION

PROTOCOL = 1
# Why there may be no describe model: the Small Ear has none.
SCORES_ONLY = (
    "no describe model is installed. The Ear ships with scores only: no describe model "
    "passed Duck Echo's listening test (docs/ear-check.md)"
)


def log(*words):
    print("duck-ear:", *words, file=sys.stderr, flush=True)


def ear_home():
    if os.environ.get("DUCK_ECHO_EAR_HOME"):
        return Path(os.environ["DUCK_ECHO_EAR_HOME"])
    if os.name == "nt":
        return Path(os.environ.get("LOCALAPPDATA", Path.home())) / "DuckEcho" / "ear"
    data = os.environ.get("XDG_DATA_HOME")
    base = Path(data) if data and Path(data).is_absolute() else Path.home() / ".local" / "share"
    return base / "duck-echo" / "ear"


def score_model_folder():
    return Path(os.environ.get("DUCK_EAR_SCORE_MODEL") or ear_home() / "models" / "score")


class EarError(Exception):
    """A request failed: becomes an "error" answer."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


class Ear:
    def __init__(self):
        self.folder = score_model_folder()
        self.info = self._read_info()
        self.model = None
        self.describer, self.no_describer = self._find_describer()

    def _find_describer(self):
        """The describe model's keeper, or why there is none."""
        home = ear_home()
        folder = ear_describe.model_folder(home)
        info = ear_describe.read_info(folder)
        if info is None:
            # Said to agents: only a person can try a bigger model.
            return None, (
                "%s. Ask with texts in `like`. (Only a person can try a bigger model: "
                "Duck Echo Studio, Settings > Duck Ear, on a PC with a big graphics card; "
                "it looked in %s.)" % (SCORES_ONLY, folder))
        program = ear_describe.server_program(home)
        if program is None:
            return None, (
                "there is a describe model, but no llama-server to run it "
                "(set DUCK_EAR_LLAMA_SERVER to the program, or put it in %s)" % (home / "llama")
            )
        return ear_describe.Describer(program, folder, info, home, log=log), None

    def close(self):
        if self.describer is not None:
            self.describer.close()

    def _read_info(self):
        try:
            return json.loads((self.folder / "model.json").read_text())
        except (OSError, ValueError):
            return None

    def hello(self, params):
        if params.get("protocol") != PROTOCOL:
            raise EarError("protocol", "this Ear speaks protocol %d, not %r" % (PROTOCOL, params.get("protocol")))
        import onnxruntime

        models = []
        if self.info is not None:
            lines = self.info.get("lines", {})
            # The revision names everything that changes an answer: the
            # model's own revision, the Ear's word list and its calibration
            # (the answer cache keys on it).
            calibration = json.dumps(
                [self.info.get("calibration", {}), self.info.get("lines", {})], sort_keys=True
            )
            model = {
                "role": "score",
                "name": self.info["name"],
                "revision": "%s+words%s+cal%s"
                % (self.info["revision"][:12], WORDS_VERSION, hashlib.sha256(calibration.encode()).hexdigest()[:8]),
                "loaded": self.model is not None,
            }
            if "yes_from" in lines:
                model["yes_from"] = lines["yes_from"]
                model["no_below"] = lines["no_below"]
            models.append(model)
        log("hello; score model:", self.folder if self.info else "none at %s" % self.folder)
        answer = {
            "protocol": PROTOCOL,
            "ear_version": __version__,
            "capabilities": ["score"] if self.info is not None else [],
            # The score model always runs on the CPU. `device` is where
            # the describe model runs: it is the slow one.
            "device": "cpu",
            "device_name": "CPU (ONNX Runtime %s)" % onnxruntime.__version__,
            "models": models,
        }
        if self.describer is None:
            answer["not_available"] = {"describe": self.no_describer, "ask": self.no_describer}
            return answer
        idle = params.get("describe_idle_seconds")
        if isinstance(idle, (int, float)) and idle > 0:
            self.describer.idle_seconds = float(idle)
        device, name, memory = self.describer.device()
        answer["capabilities"] += ["describe", "ask"]
        answer["device"] = device
        answer["device_name"] = "%s for the describe model; the score model on the CPU" % name
        if memory is not None:
            answer["device_memory_mb"] = memory
        info = self.describer.info
        models.append(
            {
                "role": "describe",
                "name": info.get("name", "describe"),
                "revision": self.describer.revision(),
                "loaded": self.describer.loaded(),
                "size": info.get("size"),
            }
        )
        log("hello; describe model: %s on %s" % (info.get("name"), name))
        return answer

    def load(self, params):
        roles = params.get("roles", [])
        if not isinstance(roles, list):
            raise EarError("bad_params", '"roles" must be a list, such as ["score"]')
        started = time.monotonic()
        for role in roles:
            if role == "score":
                self._score_model()
            elif role == "describe":
                self._describer().load()
            else:
                raise EarError("bad_params", "there is no model for the role %r" % role)
        return {"loaded": roles, "seconds": round(time.monotonic() - started, 3)}

    def _score_model(self):
        if self.model is None:
            if self.info is None:
                raise EarError(
                    "model_missing",
                    "no score model at %s (convert one with ear/convert/convert_clap.py, "
                    "or set DUCK_EAR_SCORE_MODEL)" % self.folder,
                )
            from .score import ScoreModel

            started = time.monotonic()
            self.model = ScoreModel(self.folder)
            log("loaded the score model %s in %.1f s" % (self.info["name"], time.monotonic() - started))
        return self.model

    def score(self, params):
        path = params.get("wav")
        texts = params.get("texts")
        if not isinstance(path, str):
            raise EarError("bad_params", '"wav" must be the path of a WAV file')
        if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
            raise EarError("bad_params", '"texts" must be a list of texts')
        model = self._score_model()
        try:
            wave, rate = ear_audio.read_wav(path)
        except ear_audio.AudioError as error:
            raise EarError("unreadable_audio", str(error))
        started = time.monotonic()
        answer = model.score(ear_audio.mono(wave), rate, texts)
        answer["model"] = "%s@%s" % (model.name, model.revision[:12])
        answer["seconds"] = round(time.monotonic() - started, 3)
        return answer

    def _describer(self):
        if self.describer is None:
            raise EarError("not_available", self.no_describer)
        return self.describer

    def _wav_of(self, params):
        path = params.get("wav")
        if not isinstance(path, str):
            raise EarError("bad_params", '"wav" must be the path of a WAV file')
        return path

    def _described(self, answer, started):
        describer = self.describer
        answer["model"] = "%s@%s" % (describer.info.get("name", "describe"), describer.revision())
        answer["seconds"] = round(time.monotonic() - started, 3)
        return answer

    def describe(self, params):
        describer = self._describer()
        path = self._wav_of(params)
        started = time.monotonic()
        return self._described(describer.describe(path, guard=params.get("guard", True) is not False), started)

    def ask(self, params):
        describer = self._describer()
        path = self._wav_of(params)
        question = params.get("question")
        if not isinstance(question, str) or not question.strip():
            raise EarError("bad_params", '"question" must be text')
        kind = params.get("answer", "yes_no")
        if kind not in ("yes_no", "free"):
            raise EarError("bad_params", '"answer" is "yes_no" or "free"')
        started = time.monotonic()
        said = describer.ask(path, question.strip(), kind, guard=params.get("guard", True) is not False)
        return self._described(said, started)


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def main():
    guard.refuse_the_network()
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    ear = Ear()
    methods = {
        "hello": ear.hello,
        "load": ear.load,
        "score": ear.score,
        "describe": ear.describe,
        "ask": ear.ask,
    }
    try:
        serve(ear, methods)
    finally:
        ear.close()  # llama-server never outlives the Ear


def serve(ear, methods):
    for line in sys.stdin:  # ends when stdin closes
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
            request_id = request["id"]
            method = request["method"]
            params = request.get("params") or {}
        except (ValueError, KeyError, TypeError):
            log("skipped a line that is not a request:", line[:200])
            continue
        if method == "bye":
            send({"id": request_id, "result": {}})
            log("bye")
            return
        handler = methods.get(method)
        try:
            if handler is None:
                raise EarError("unknown_method", "the Ear cannot do %r" % method)
            send({"id": request_id, "result": handler(params)})
        except (EarError, ear_describe.DescribeError, ear_audio.AudioError) as error:
            code = getattr(error, "code", "unreadable_audio")
            send({"id": request_id, "error": {"code": code, "message": str(error)}})
        except Exception as error:  # a bug: say so, and carry on
            log("internal error in %s: %r" % (method, error))
            send({"id": request_id, "error": {"code": "internal", "message": repr(error)[:500]}})
    log("stdin closed: exiting")


if __name__ == "__main__":
    main()
