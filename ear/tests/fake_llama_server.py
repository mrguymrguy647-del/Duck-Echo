#!/usr/bin/env python3
"""A pretend `llama-server`, for the Duck Ear's tests: no model at all.

It takes the options the Ear gives the real one, answers `/health` and
`/v1/chat/completions` on 127.0.0.1, and writes what it was asked into a
log the tests read (one JSON object per line, in the file
`FAKE_LLAMA_LOG` names). Python's standard library only.

What it "hears": nothing. A yes or no question gets `yes` when it names a
coin and `no` otherwise; anything else gets fixed words.

Switches, all environment variables:

  FAKE_LLAMA_LOG          the log file
  FAKE_LLAMA_DEVICES      what `--list-devices` prints, one device per
                          line, such as "Vulkan0: Pretend Card (24564 MiB,
                          24000 MiB free)"; none when unset
  FAKE_LLAMA_FAIL_ON_CARD exit at once unless started with `--device none`
                          (a graphics card with too little memory)
"""

import base64
import io
import json
import os
import sys
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def note(event):
    path = os.environ.get("FAKE_LLAMA_LOG")
    if path:
        with open(path, "a") as file:
            file.write(json.dumps(event) + "\n")


def heard_seconds(part):
    """The length and rate of the WAV in an `input_audio` part."""
    data = base64.b64decode(part["input_audio"]["data"])
    with wave.open(io.BytesIO(data), "rb") as file:
        return {
            "seconds": file.getnframes() / float(file.getframerate()),
            "rate": file.getframerate(),
            "channels": file.getnchannels(),
        }


def shown(messages):
    """The messages with each sound replaced by its length (the log stays
    small)."""
    out = []
    for message in messages:
        content = message["content"]
        if isinstance(content, list):
            content = [
                {"audio": heard_seconds(part)} if part.get("type") == "input_audio" else part["text"]
                for part in content
            ]
        out.append({"role": message["role"], "content": content})
    return out


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):  # quiet
        pass

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/health":
            self.reply(200, {"status": "ok"})
        else:
            self.reply(404, {"error": {"message": "not found"}})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        messages = shown(body["messages"])
        note({"event": "chat", "messages": messages, **{k: v for k, v in body.items() if k != "messages"}})
        last = messages[-1]["content"]
        last = last if isinstance(last, str) else " ".join(p for p in last if isinstance(p, str))
        if "response_format" in body:
            answer = "yes" if "coin" in last.lower() else "no"
            text = json.dumps({"answer": answer, "reason": "A pretend reason."})
        elif len(messages) == 1:
            text = "  A pretend   description. "
        else:
            text = "A pretend answer."
        self.reply(200, {"choices": [{"message": {"role": "assistant", "content": text}}]})


def main():
    arguments = sys.argv[1:]
    if "--list-devices" in arguments:
        print("Available devices:")
        listed = os.environ.get("FAKE_LLAMA_DEVICES")
        for line in listed.split("\n") if listed else ["(none)"]:
            print("  " + line)
        return
    note({"event": "start", "pid": os.getpid(), "arguments": arguments})
    on_cpu = "--device" in arguments and arguments[arguments.index("--device") + 1] == "none"
    if os.environ.get("FAKE_LLAMA_FAIL_ON_CARD") and not on_cpu:
        print("pretend: not enough memory on the graphics card", file=sys.stderr)
        sys.exit(1)
    port = int(arguments[arguments.index("--port") + 1])
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
