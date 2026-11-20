#!/usr/bin/env python3
"""A pretend `duck-echo`, for the tests of ear/tools/measure_describe_model.py.

It writes what it was started with (its arguments and the Ear's settings
from the environment) into the file `FAKE_DUCK_ECHO_LOG` names, and, like
`duck-echo ear check`, an `ear-check.json` and `ear-check.md` into `--out`.
"""

import json
import os
import sys
from pathlib import Path

arguments = sys.argv[1:]
kept = ("DUCK_EAR_DESCRIBE_MODEL", "DUCK_EAR_DEVICE", "DUCK_ECHO_EAR_TIMEOUT_SCALE")
Path(os.environ["FAKE_DUCK_ECHO_LOG"]).write_text(
    json.dumps({"arguments": arguments, "environment": {name: os.environ.get(name) for name in kept}})
)
out = Path(arguments[arguments.index("--out") + 1])
out.mkdir(parents=True, exist_ok=True)
report = {
    "describe": {
        "model": "pretend-model@0",
        "device": "cpu",
        "measures": [{"name": "Describe: yes on true texts", "right": 1, "of": 2, "bar": 0.8}],
        "times": {"load": 1.0, "describe_mean": 2.0, "ask_mean": 0.5},
    }
}
(out / "ear-check.json").write_text(json.dumps(report))
(out / "ear-check.md").write_text("# Duck Ear check\n")
