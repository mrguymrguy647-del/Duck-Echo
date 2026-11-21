#!/usr/bin/env python3
"""Measure a describe model on this PC, in one command.

A person runs this (it downloads, after asking):

    python ear/tools/measure_describe_model.py full
    python ear/tools/measure_describe_model.py large

Run it with the Ear's own Python, from the repository's top folder: on
Windows `ear\\.venv\\Scripts\\python.exe`, elsewhere `ear/.venv/bin/python`.

What it does, for one size (`full`, `large`) or one model's name:

1. finds `llama-server`, the graphics card, `duck-echo` and the Ear;
2. downloads the model if it is not here yet, after showing its size and
   asking, and checks every file's SHA-256;
3. runs `duck-echo ear check --part describe --fresh` on Duck Echo's test
   set (41 sounds, 184 yes or no questions) with that model;
4. writes `ear-check.json` and `ear-check.md` into `ear-check-<size>/`
   (or `--out`): the model, the graphics card, every yes and no, the
   verdicts and the times. Those two files are what to send back.

It does not change which model the Ear uses. `--cpu` measures on the CPU
(slow: about an hour for the 7B model on 4 cores). `--quick` asks about
10 sounds only, to see that everything works. `--model-folder` measures a
model that is already in a folder, with nothing downloaded.

Python's standard library only (and the duck_ear package beside it).
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import get_describe_model as getter  # noqa: E402
from duck_ear import describe  # noqa: E402
from duck_ear.server import ear_home, score_model_folder  # noqa: E402

EXE = ".exe" if os.name == "nt" else ""


def find_duck_echo(named):
    """The `duck-echo` program: the one named, `DUCK_ECHO`, this
    repository's build (release first), or the one on the PATH."""
    places = [named, os.environ.get("DUCK_ECHO")]
    places += [REPO / "target" / kind / ("duck-echo" + EXE) for kind in ("release", "debug")]
    for place in places:
        if place and Path(place).is_file():
            return Path(place)
    found = shutil.which("duck-echo")
    return Path(found) if found else None


def find_ear():
    """The Ear's program: `duck-ear` beside this Python (the Ear's own
    environment), else what `DUCK_ECHO_EAR` names."""
    beside = Path(sys.executable).parent / ("duck-ear" + EXE)
    if beside.is_file():
        return beside
    named = os.environ.get("DUCK_ECHO_EAR")
    return Path(named) if named and Path(named).is_file() else None


def stop(message):
    print(message)
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("which", nargs="?", help="a size (full, large) or a model's name")
    parser.add_argument("--model-folder", help="measure the model in this folder; download nothing")
    parser.add_argument("--yes", action="store_true", help="do not ask before downloading")
    parser.add_argument("--cpu", action="store_true", help="measure on the CPU, not the graphics card")
    parser.add_argument("--out", help="where to write ear-check.json and ear-check.md")
    parser.add_argument("--duck-echo", help="the duck-echo program (default: this repository's build, or the PATH)")
    parser.add_argument("--testset", help="another test set (default: ear/testset/testset.toml)")
    parser.add_argument(
        "--quick", action="store_true", help="10 sounds only (ear/testset/quick.toml): a check that it all works"
    )
    args = parser.parse_args()
    if not args.which and not args.model_folder:
        parser.print_help()
        return 2
    home = ear_home()

    # 1. Everything the measurement needs, checked before a long download.
    server = describe.server_program(home)
    if server is None:
        return stop(
            "There is no llama-server. Download llama.cpp's newest build for this PC from\n"
            "https://github.com/ggml-org/llama.cpp/releases (on Windows with a graphics card: the Vulkan zip),\n"
            "unpack it into %s (so llama-server%s is right inside), and run this again.\n"
            "Or set DUCK_EAR_LLAMA_SERVER to the program." % (home / "llama", EXE)
        )
    duck_echo = find_duck_echo(args.duck_echo)
    if duck_echo is None:
        return stop("There is no duck-echo. Build it (cargo build --release -p duck-echo-cli) or name it with --duck-echo.")
    ear = find_ear()
    if ear is None:
        return stop(
            "There is no duck-ear beside this Python. Run `uv sync` in the ear folder, then run this with\n"
            "the Ear's own Python (ear/.venv/bin/python, or ear\\.venv\\Scripts\\python.exe on Windows)."
        )
    if not (score_model_folder() / "model.json").is_file():
        return stop(
            "There is no score model at %s. The verdict needs it: make it first (ear/convert/README.md)."
            % score_model_folder()
        )
    if args.cpu:
        os.environ["DUCK_EAR_DEVICE"] = "cpu"
    cards = describe.cards(server)
    print("llama-server: %s" % server)
    print("duck-echo:    %s" % duck_echo)
    print("the Ear:      %s" % ear)
    print("measured on:  %s" % ("%s (%d MB)" % (cards[0]["name"], cards[0]["memory_mb"]) if cards else "the CPU"))

    # 2. The model.
    if args.model_folder:
        folder = Path(args.model_folder).resolve()
        if describe.read_info(folder) is None:
            return stop("%s holds no describe model (a model.json and its two files)." % folder)
        label = describe.read_info(folder).get("name", folder.name)
    else:
        code, folder = getter.fetch(args.which, home, yes=args.yes, keep_current=True)
        if folder is None:
            return code
        label = args.which
    testset = args.testset or str(REPO / "ear" / "testset" / ("quick.toml" if args.quick else "testset.toml"))
    out = Path(args.out or "ear-check-%s%s%s" % (label, "-cpu" if args.cpu else "", "-quick" if args.quick else ""))

    # 3. The check. Fresh: no answer cache, so the times are this run's.
    env = dict(os.environ, DUCK_EAR_DESCRIBE_MODEL=str(folder))
    env.setdefault("DUCK_ECHO_EAR_TIMEOUT_SCALE", "3")  # a big model's first load from a slow disk
    command = describe.command_for(duck_echo) + [
        "ear", "check", "--part", "describe", "--fresh",
        "--testset", testset, "--out", str(out), "--ear-program", str(ear),
    ]  # fmt: skip
    print()
    if args.quick:
        print("Measuring %s on the quick set: 10 sounds and 52 questions." % label)
    else:
        print("Measuring %s: 41 sounds and 184 questions. Minutes on a graphics card, about an hour on a CPU." % label)
    print(" ".join(command), flush=True)
    done = subprocess.run(command, env=env)
    if done.returncode != 0:
        print()
        print("The check did not finish (exit code %d). The Ear's log is %s," % (done.returncode, home / "ear.log"))
        print("and llama-server's is %s." % (home / "llama-server.log"))
        return done.returncode

    # 4. What to send back.
    print()
    try:
        report = json.loads((out / "ear-check.json").read_text())
        measured = report["describe"]
        print("%s on %s" % (measured["model"], measured.get("device_name") or measured["device"]))
        for measure in measured["measures"]:
            print("  %-30s %3d of %-3d (bar %.0f%%)" % (measure["name"], measure["right"], measure["of"], measure["bar"] * 100))
        times = measured["times"]
        print(
            "  load %.1f s, hearing a sound %.1f s, a further question %.1f s"
            % (times["load"], times["describe_mean"], times["ask_mean"])
        )
    except (OSError, ValueError, KeyError):
        pass
    print("Send back these two files:")
    print("  %s" % (out / "ear-check.json").resolve())
    print("  %s" % (out / "ear-check.md").resolve())
    return 0


if __name__ == "__main__":
    sys.exit(main())
