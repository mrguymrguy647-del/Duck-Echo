# The Duck Ear

The Duck Ear is an optional program that listens to a sound with
machine-learning models and says what it hears, so an AI assistant can ask
"does this sound like a soft coin pickup?" without a person listening.
Duck Echo starts it, talks to it in JSON Lines over its stdin and stdout,
and never waits for it longer than a set time. Nothing in Duck Echo needs
it. The plan is `docs/phase4-design.md`.

* `stub/duck_ear_stub.py`: the stub Ear. It speaks the protocol with no
  models (Python's standard library only), and every answer says
  `"stub": true`. Duck Echo's tests use it.

The real Ear (a Python package, its models and its installer) comes in
steps 3 to 5 of the design.

Try the stub by hand (one request per line; Ctrl+D, or Ctrl+Z then Enter
on Windows, ends it):

```bash
python3 ear/stub/duck_ear_stub.py
{"id": 0, "method": "hello", "params": {"protocol": 1}}
{"id": 1, "method": "bye"}
```
