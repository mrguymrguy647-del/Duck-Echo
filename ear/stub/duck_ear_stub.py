#!/usr/bin/env python3
"""The stub Duck Ear: it speaks the Ear's protocol, with no models.

Duck Echo talks to the Duck Ear in JSON Lines: one request per line on
the Ear's stdin, one answer per line on its stdout, matched by "id". This
stub answers like the real Ear will, but never listens: every answer says
"stub": true, so nobody mistakes it for listening. Duck Echo's tests use
it, and so can anyone trying the Ear's tools before the real Ear exists.

It needs only Python's standard library (Python 3.8 or newer).

  score     a text containing "coin" matches 0.9, anything else 0.1
  describe  "A stub Ear: no model listened to this."
  ask       "unsure", with the same reason
  load      done at once
  bye       answers, then exits

Like every Ear, it exits as soon as its stdin closes: that happens
whenever Duck Echo stops, however it stops. Its log goes to stderr.

The protocol is in docs/phase4-design.md, section 3.
"""

import json
import sys

PROTOCOL = 1
VERSION = "0.1.0-stub"
NOT_LISTENING = "A stub Ear: no model listened to this."

# A few words to rank texts among, like the real Ear's list of about 120.
VOCABULARY = ["coin pickup", "explosion", "footsteps", "laser", "calm music"]


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


def hello(params):
    if params.get("protocol") != PROTOCOL:
        raise EarError(
            "protocol",
            "this Ear speaks protocol %d, not %r" % (PROTOCOL, params.get("protocol")),
        )
    log("hello; describe_idle_seconds =", params.get("describe_idle_seconds"))
    return {
        "protocol": PROTOCOL,
        "ear_version": VERSION,
        "capabilities": ["score", "describe", "ask"],
        "device": "none",
        "device_name": "no device: the stub does not listen",
        "models": [
            {"role": "score", "name": "stub", "revision": "0", "loaded": True},
            {"role": "describe", "name": "stub", "revision": "0", "loaded": True},
        ],
        "stub": True,
    }


def load(params):
    roles = params.get("roles", [])
    if not isinstance(roles, list):
        raise EarError("bad_params", '"roles" must be a list, such as ["score"]')
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
                "closest": VOCABULARY[:3],
            }
        )
    return {"model": "stub@0", "scores": scores, "stub": True}


def describe(params):
    read_wav(params)
    return {"model": "stub@0", "text": NOT_LISTENING, "seconds": 0.0, "stub": True}


def ask(params):
    read_wav(params)
    if not isinstance(params.get("question"), str):
        raise EarError("bad_params", '"question" must be text')
    return {
        "model": "stub@0",
        "answer": "unsure",
        "reason": NOT_LISTENING,
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
        try:
            if handler is None:
                raise EarError("unknown_method", "the Ear cannot do %r" % method)
            send({"id": request_id, "result": handler(params)})
        except EarError as error:
            send({"id": request_id, "error": {"code": error.code, "message": error.message}})
    log("stdin closed: exiting")


if __name__ == "__main__":
    main()
