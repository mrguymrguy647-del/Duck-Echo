#!/usr/bin/env python3
"""Download a describe model for the Duck Ear, and check it.

A person runs this, never the Ear and never an AI assistant: it uses the
network (Hugging Face), and the files are large. It shows the size and
asks before it downloads anything. The installer of step 5 will do this
from the Studio; until then:

    python ear/tools/get_describe_model.py --list
    python ear/tools/get_describe_model.py full
    python ear/tools/get_describe_model.py large

It puts the model in `<the Ear's folder>/models/<name>/` with a
`model.json`, checks every file's SHA-256, and makes it the model the Ear
uses (`models/describe.txt` names it; `--keep-current` leaves that
alone). A stopped download carries on where it stopped.

Large is offered only when `llama-server` finds a graphics card with 16
GB or more, so `llama-server` must be there first: the program
`DUCK_EAR_LLAMA_SERVER` names, or `<the Ear's folder>/llama/llama-server`.

Python's standard library only (and the duck_ear package beside it).
"""

import argparse
import hashlib
import json
import os
import shutil
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from duck_ear import catalog  # noqa: E402
from duck_ear import describe  # noqa: E402
from duck_ear.server import ear_home  # noqa: E402

CHUNK = 1 << 20


def gigabytes(count):
    return "%.1f GB" % (count / 1e9)


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(CHUNK), b""):
            digest.update(block)
    return digest.hexdigest()


def download(url, to, size):
    """Download `url` to `to`, carrying on from a `.part` file if one is
    there."""
    part = Path(str(to) + ".part")
    have = part.stat().st_size if part.exists() else 0
    if have < size:
        request = urllib.request.Request(url, headers={"Range": "bytes=%d-" % have} if have else {})
        with urllib.request.urlopen(request, timeout=60) as answer:
            if have and answer.status != 206:  # the server sent it all again
                have = 0
            with open(part, "ab" if have else "wb") as file:
                done = have
                while True:
                    block = answer.read(CHUNK)
                    if not block:
                        break
                    file.write(block)
                    done += len(block)
                    if done // (256 * CHUNK) != (done - len(block)) // (256 * CHUNK):
                        print("  %s of %s" % (gigabytes(done), gigabytes(size)), flush=True)
    os.replace(part, to)


def cards_found(home):
    program = describe.server_program(home)
    return (describe.cards(program) if program else []), program


def show_list(home):
    cards, program = cards_found(home)
    print("The Ear's folder: %s" % home)
    print("llama-server: %s" % (program or "not found (set DUCK_EAR_LLAMA_SERVER)"))
    print("Graphics cards: %s" % (", ".join("%s (%d MB)" % (c["name"], c["memory_mb"]) for c in cards) or "none found"))
    print()
    for size, name in catalog.SIZES.items():
        if name is None:
            print("%-6s scores only: no small describe model has passed the listening test yet" % size)
            continue
        may, why = catalog.offered(name, cards)
        print(
            "%-6s %-20s %8s  %s: %s"
            % (size, name, gigabytes(catalog.total_bytes(name)), "offered" if may else "not offered", why)
        )
    print()
    print("Candidates for small, for measuring only: gemma-4-e2b, gemma-4-e4b")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("which", nargs="?", help="a size (full, large) or a model's name")
    parser.add_argument("--list", action="store_true", help="show the sizes and what this PC is offered")
    parser.add_argument("--yes", action="store_true", help="do not ask before downloading")
    parser.add_argument("--keep-current", action="store_true", help="do not make this the model the Ear uses")
    args = parser.parse_args()
    home = ear_home()
    if args.list or not args.which:
        show_list(home)
        return 0
    name = catalog.SIZES.get(args.which, args.which)
    if name is None:
        print("The Small Ear has scores only: no small describe model has passed the listening test yet (docs/ear-check.md).")
        return 1
    if name not in catalog.MODELS:
        print("There is no size or model `%s`. Sizes: full, large. Models: %s." % (args.which, ", ".join(catalog.MODELS)))
        return 2
    model = catalog.MODELS[name]
    cards, _ = cards_found(home)
    may, why = catalog.offered(name, cards)
    if not may:
        print("%s is not offered on this PC: it %s." % (name, why))
        return 1
    folder = home / "models" / name
    folder.mkdir(parents=True, exist_ok=True)
    missing = [f for f in model["files"].values() if not (folder / f["name"]).is_file()]
    need = sum(f["bytes"] for f in missing)
    free = shutil.disk_usage(folder).free
    print("%s (%s, %s): %s to download into %s" % (name, model["maker"], model["licence"], gigabytes(need), folder))
    if need > free:
        print("Not enough room: %s free on that disk." % gigabytes(free))
        return 1
    if missing and not args.yes:
        if input("Download %s? [y/N] " % gigabytes(need)).strip().lower() not in ("y", "yes"):
            print("Nothing was downloaded.")
            return 1
    for file in missing:
        print("downloading %s" % file["name"], flush=True)
        url = "https://huggingface.co/%s/resolve/%s/%s" % (model["repo"], model["revision"], file["name"])
        download(url, folder / file["name"], file["bytes"])
    for file in model["files"].values():
        print("checking %s" % file["name"], flush=True)
        if sha256_of(folder / file["name"]) != file["sha256"]:
            print("%s does not match its SHA-256: delete it and run this again." % (folder / file["name"]))
            return 1
    (folder / "model.json").write_text(json.dumps(catalog.model_json(name), indent=2) + "\n")
    if not args.keep_current:
        (home / "models" / "describe.txt").write_text(name + "\n")
        print("The Ear now uses %s." % name)
    print("Done: %s" % folder)
    return 0


if __name__ == "__main__":
    sys.exit(main())
