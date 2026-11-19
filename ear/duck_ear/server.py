"""The Duck Ear's main loop: JSON Lines in on stdin, out on stdout.

The protocol is in docs/phase4-design.md, section 3 (and the stub,
ear/stub/duck_ear_stub.py, answers the same way without models). This
Ear has the score model only (step 3 of the design); `describe` and `ask`
come with step 4, and until then they answer that they are not available
yet. Like every Ear, it exits as soon as its stdin closes, and it writes
its log to stderr, never to stdout.

Where the score model is: the folder `DUCK_EAR_SCORE_MODEL` names, else
`<the Ear's folder>/models/score`, where the Ear's folder is
`DUCK_ECHO_EAR_HOME`, else the usual place (the same rule as Duck Echo's).
"""

import hashlib
import json
import os
import sys
import time
from pathlib import Path

from . import __version__
from . import audio as ear_audio
from . import guard
from .words import WORDS_VERSION

PROTOCOL = 1
NOT_YET = "the describe model comes with step 4 of the Duck Ear (docs/phase4-design.md): not available yet"


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
        return {
            "protocol": PROTOCOL,
            "ear_version": __version__,
            "capabilities": ["score"] if self.info is not None else [],
            "device": "cpu",
            "device_name": "CPU (ONNX Runtime %s)" % onnxruntime.__version__,
            "models": models,
            "not_available": {"describe": NOT_YET, "ask": NOT_YET},
        }

    def load(self, params):
        roles = params.get("roles", [])
        if not isinstance(roles, list):
            raise EarError("bad_params", '"roles" must be a list, such as ["score"]')
        started = time.monotonic()
        for role in roles:
            if role == "score":
                self._score_model()
            elif role == "describe":
                raise EarError("model_missing", NOT_YET)
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

    def not_yet(self, params):
        raise EarError("not_available", NOT_YET)


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
        "describe": ear.not_yet,
        "ask": ear.not_yet,
    }
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
        except EarError as error:
            send({"id": request_id, "error": {"code": error.code, "message": error.message}})
        except Exception as error:  # a bug: say so, and carry on
            log("internal error in %s: %r" % (method, error))
            send({"id": request_id, "error": {"code": "internal", "message": repr(error)[:500]}})
    log("stdin closed: exiting")


if __name__ == "__main__":
    main()
