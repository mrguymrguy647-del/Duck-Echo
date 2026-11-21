#!/usr/bin/env python3
"""The stub Duck Ear: it speaks the Ear's protocol, with no models.

Duck Echo talks to the Duck Ear in JSON Lines: one request per line on
the Ear's stdin, one answer per line on its stdout, matched by "id". This
stub answers like the real Ear will, but never listens: every answer says
"stub": true, so nobody mistakes it for listening. Duck Echo's tests use
it, and so can anyone trying the Ear's tools before the real Ear exists.

It needs only Python's standard library (Python 3.8 or newer).

  score     a text containing "coin" matches 0.9, anything else 0.1;
            the closest words are always the same three
  describe  "A stub Ear: no model listened to this."
  ask       a yes or no question: "yes" when it names a coin, else "no";
            a free question: the same words as describe
  load      done at once
  bye       answers, then exits

Like the real Ear, it lets go of its describe model when no describe or
ask question came for describe_idle_seconds (the real one stops
llama-server then), and answers describe and ask only after "load" with
the role "describe". So Duck Echo must send load before every describe
question, as the design says. The stub checks the time when a request
comes, rather than with a clock of its own; Duck Echo cannot tell the
difference.

Like every Ear, it exits as soon as its stdin closes: that happens
whenever Duck Echo stops, however it stops. Its log goes to stderr.

The protocol is in docs/phase4-design.md, section 3.
"""

import json
import os
import sys
import time

PROTOCOL = 1
VERSION = "0.1.0-stub"
NOT_LISTENING = "A stub Ear: no model listened to this."

# A few words to rank texts among, like the real Ear's list of about 120.
VOCABULARY = ["coin pickup", "explosion", "footsteps", "laser", "calm music"]

# The models' versions. A different revision means different answers, so
# Duck Echo's answer cache keys on them.
SCORE_REVISION = "0"
DESCRIBE_REVISION = "0"


class State:
    """What the stub remembers between requests."""

    describe_idle_seconds = 120.0
    loaded = set()  # the roles loaded: "score", "describe"
    last_describe = 0.0  # when the last describe or ask question came


def let_go_of_an_idle_describe_model():
    """Unload the describe model if no describe question came for a while."""
    if "describe" not in State.loaded:
        return
    idle = time.monotonic() - State.last_describe
    if idle >= State.describe_idle_seconds:
        State.loaded.discard("describe")
        log(
            "stopped the describe model after %.0f s with no describe question"
            % State.describe_idle_seconds
        )


def need_describe_model():
    if "describe" not in State.loaded:
        raise EarError(
            "model_not_loaded",
            'the describe model is not loaded: send load with roles ["describe"] first',
        )
    State.last_describe = time.monotonic()


def log(*words):
    """Write a line to the log (stderr), never to stdout."""
    print("duck-ear-stub:", *words, file=sys.stderr, flush=True)


def send(message):
    """Write one answer line to stdout."""
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


class EarError(Exception):
    """A request failed: becomes an "error" answer."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def read_wav(params):
    """Check that params["wav"] names a WAV file. Its sound is not used."""
    path = params.get("wav")
    if not isinstance(path, str):
        raise EarError("bad_params", '"wav" must be the path of a WAV file')
    try:
        with open(path, "rb") as file:
            start = file.read(12)
    except OSError as error:
        raise EarError("unreadable_audio", "cannot read %s: %s" % (path, error))
    if len(start) < 12 or start[:4] != b"RIFF" or start[8:12] != b"WAVE":
        raise EarError("unreadable_audio", "%s is not a WAV file" % path)
    return path


def score_revision():
    """The score model's revision: SCORE_REVISION, or the text of a file
    `stub_revision.txt` beside the stub. With that file a test can change
    the model while the program stays the same, as when a real Ear is
    pointed at another model folder."""
    beside = os.path.join(os.path.dirname(os.path.abspath(__file__)), "stub_revision.txt")
    try:
        with open(beside) as file:
            return file.read().strip()
    except OSError:
        return SCORE_REVISION


def hello(params):
    if params.get("protocol") != PROTOCOL:
        raise EarError(
            "protocol",
            "this Ear speaks protocol %d, not %r" % (PROTOCOL, params.get("protocol")),
        )
    idle = params.get("describe_idle_seconds")
    if isinstance(idle, (int, float)) and idle > 0:
        State.describe_idle_seconds = float(idle)
    log("hello; describe_idle_seconds =", idle)
    return {
        "protocol": PROTOCOL,
        "ear_version": VERSION,
        "capabilities": ["score", "describe", "ask"],
        "device": "none",
        "device_name": "no device: the stub does not listen",
        "models": [
            {"role": "score", "name": "stub", "revision": score_revision(),
             "loaded": "score" in State.loaded},
            {"role": "describe", "name": "stub", "revision": DESCRIBE_REVISION,
             "loaded": "describe" in State.loaded},
        ],
        "stub": True,
    }


def load(params):
    roles = params.get("roles", [])
    if not isinstance(roles, list):
        raise EarError("bad_params", '"roles" must be a list, such as ["score"]')
    for role in roles:
        if role not in ("score", "describe"):
            raise EarError("bad_params", "there is no model for the role %r" % role)
        if role not in State.loaded:
            State.loaded.add(role)
            log("loaded the %s model" % role)
        if role == "describe":
            State.last_describe = time.monotonic()
    return {"loaded": roles, "seconds": 0.0, "stub": True}


def score(params):
    read_wav(params)
    texts = params.get("texts")
    if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
        raise EarError("bad_params", '"texts" must be a list of texts')
    scores = []
    for text in texts:
        match = 0.9 if "coin" in text.lower() else 0.1
        scores.append(
            {
                "text": text,
                "match": match,
                "rank": 1 if match > 0.5 else len(VOCABULARY),
                "of": len(VOCABULARY) + 1,
            }
        )
    closest = [
        {"text": VOCABULARY[0], "match": 0.5},
        {"text": VOCABULARY[1], "match": 0.2},
        {"text": VOCABULARY[2], "match": 0.1},
    ]
    return {
        "model": "stub@" + SCORE_REVISION,
        "scores": scores,
        "closest": closest,
        "seconds": 0.0,
        "stub": True,
    }


def describe(params):
    read_wav(params)
    need_describe_model()
    return {
        "model": "stub@" + DESCRIBE_REVISION,
        "text": NOT_LISTENING,
        "seconds": 0.0,
        "stub": True,
    }


def ask(params):
    read_wav(params)
    question = params.get("question")
    if not isinstance(question, str):
        raise EarError("bad_params", '"question" must be text')
    kind = params.get("answer", "yes_no")
    if kind not in ("yes_no", "free"):
        raise EarError("bad_params", '"answer" is "yes_no" or "free"')
    need_describe_model()
    if kind == "free":
        return {
            "model": "stub@" + DESCRIBE_REVISION,
            "text": NOT_LISTENING,
            "seconds": 0.0,
            "stub": True,
        }
    return {
        "model": "stub@" + DESCRIBE_REVISION,
        "answer": "yes" if "coin" in question.lower() else "no",
        "reason": NOT_LISTENING + " It says yes to any question about a coin.",
        "seconds": 0.0,
        "stub": True,
    }


METHODS = {
    "hello": hello,
    "load": load,
    "score": score,
    "describe": describe,
    "ask": ask,
}


def main():
    # Text in and out as UTF-8 with plain \n line ends, on Windows too.
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
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
        handler = METHODS.get(method)
        let_go_of_an_idle_describe_model()
        try:
            if handler is None:
                raise EarError("unknown_method", "the Ear cannot do %r" % method)
            send({"id": request_id, "result": handler(params)})
            log("answered", method)
        except EarError as error:
            send({"id": request_id, "error": {"code": error.code, "message": error.message}})
    log("stdin closed: exiting")


if __name__ == "__main__":
    main()
