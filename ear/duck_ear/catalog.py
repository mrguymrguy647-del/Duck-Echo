"""The describe models the Duck Ear knows, and its three sizes.

Each model is two files from Hugging Face at one pinned revision: the
model itself and the part that hears (`mmproj`). Both are GGUF files for
`llama-server`. Their sizes and SHA-256 are written here so a download
can be checked. The results that chose them are in docs/ear-check.md.

The sizes a person can choose:

* **Small**: scores only. No describe model passed Duck Echo's listening
  test (docs/ear-check.md), so the Ear ships without one.
* **Full**: Qwen2.5-Omni-7B, for a graphics card of 8 GB or more (it
  works on the CPU, slowly). It did not pass the test either: a person
  can add it, but Duck Echo does not recommend it yet.
* **Large**: Qwen3-Omni-30B-A3B, offered only when a graphics card with
  16 GB or more is found. Not measured yet: it needs such a card.

Each model's `listening_test` says how it did, in one line.
"""

# A graphics card must have this much memory before Large is offered.
LARGE_NEEDS_MB = 16000

MODELS = {
    "qwen2.5-omni-7b": {
        "size": "full",
        "maker": "Alibaba (Qwen), GGUF by ggml-org",
        "repo": "ggml-org/Qwen2.5-Omni-7B-GGUF",
        "revision": "89b785438c8901d4635e42f50480ba5985a1bbf1",
        # The original model's own LICENSE file is Apache-2.0 (the GGUF
        # page's label says otherwise, but links to that same file).
        "licence": "Apache-2.0",
        "listening_test": "did not pass (CPU): yes to 45% of true texts (bar 80%), no to 90% of wrong texts",
        "files": {
            "model": {
                "name": "Qwen2.5-Omni-7B-Q4_K_M.gguf",
                "bytes": 4683072384,
                "sha256": "09883dff531dc56923a041c9c99c7c779e26ffde32caa83adeeb7502ec3b50fe",
            },
            "mmproj": {
                "name": "mmproj-Qwen2.5-Omni-7B-Q8_0.gguf",
                "bytes": 1548488480,
                "sha256": "4a7bc5478a2ec8c5d186d63532eb22e75b79ba75ec3c0ce821676157318ef4ad",
            },
        },
        "settings": {"context": 4096, "sample_rate": 16000},
    },
    "qwen3-omni-30b-a3b": {
        "size": "large",
        "needs_card_mb": LARGE_NEEDS_MB,
        "maker": "Alibaba (Qwen), GGUF by ggml-org",
        "repo": "ggml-org/Qwen3-Omni-30B-A3B-Instruct-GGUF",
        "revision": "6e35a28f4a19b18730f8949b0c579c6429649ab8",
        "licence": "Apache-2.0",
        "listening_test": "not measured yet: it needs a graphics card with 16 GB or more",
        "files": {
            "model": {
                "name": "Qwen3-Omni-30B-A3B-Instruct-Q4_K_M.gguf",
                "bytes": 18557053952,
                "sha256": "d9e2876556e7873e02c0359f832432ee2d67ab7dd0cee3efe0f77fd7a1f4dd85",
            },
            "mmproj": {
                "name": "mmproj-Qwen3-Omni-30B-A3B-Instruct-Q8_0.gguf",
                "bytes": 1325020128,
                "sha256": "1104376db833f1e89c84834144ac3863340c2cd1ddaeddb39cb0247fb5c20c8d",
            },
        },
        "settings": {"context": 4096, "sample_rate": 16000},
    },
    # Tried for Small, and did not pass the listening test
    # (docs/ear-check.md). Kept so the test can be run again; not offered
    # as a size. (MiDashengLM-0.6B was tried too and is not here: it
    # needs its makers' fork of llama.cpp, and does not answer yes or no.)
    "gemma-4-e2b": {
        "size": "tried for small",
        "maker": "Google, GGUF by ggml-org",
        "repo": "ggml-org/gemma-4-E2B-it-GGUF",
        "revision": "2a8538c2e2d4687953a86c422c7c623cfd022880",
        "licence": "Apache-2.0",
        "listening_test": "did not pass (CPU): yes to 13% of true texts (bar 80%), no to 96% of wrong texts",
        "files": {
            "model": {
                "name": "gemma-4-E2B-it-Q4_0.gguf",
                "bytes": 2841481184,
                "sha256": "8e30dff3ac4c8434c49a7036fa15564bdbb6044e42bf04550bf1a096ad7e6a52",
            },
            "mmproj": {
                "name": "mmproj-gemma-4-E2B-it-Q8_0.gguf",
                "bytes": 557368096,
                "sha256": "042cc2561806cd3b5cb5efb25cab76911e3100c43cc09d854904f921924b1429",
            },
        },
        # Gemma thinks for hundreds of words before an answer whose shape
        # is forced, even with `--reasoning off`; asked plainly it does not.
        "settings": {
            "context": 4096,
            "sample_rate": 16000,
            "server_arguments": ["--reasoning", "off"],
            "plain_answers": True,
        },
    },
    "gemma-4-e4b": {
        "size": "tried for small",
        "maker": "Google, GGUF by ggml-org",
        "repo": "ggml-org/gemma-4-E4B-it-GGUF",
        "revision": "8903cb408c7df39cde2ed15fac397e6d7be92951",
        "licence": "Apache-2.0",
        "listening_test": "did not pass (CPU): yes to 34% of true texts (bar 80%), no to 86% of wrong texts",
        "files": {
            "model": {
                "name": "gemma-4-E4B-it-Q4_0.gguf",
                "bytes": 4590807392,
                "sha256": "a555b900214b477d8880e7832e0b8925e139b0159640036b09fe472b6f2097f2",
            },
            "mmproj": {
                "name": "mmproj-gemma-4-E4B-it-Q8_0.gguf",
                "bytes": 559874848,
                "sha256": "9257f21a0f5358204b4a612cff83fad2e56412f4f607f0049331ce60c0cf9d34",
            },
        },
        # Gemma thinks for hundreds of words before an answer whose shape
        # is forced, even with `--reasoning off`; asked plainly it does not.
        "settings": {
            "context": 4096,
            "sample_rate": 16000,
            "server_arguments": ["--reasoning", "off"],
            "plain_answers": True,
        },
    },
}

# The model of each size a person can choose. Small has none.
SIZES = {"small": None, "full": "qwen2.5-omni-7b", "large": "qwen3-omni-30b-a3b"}


def total_bytes(name):
    return sum(part["bytes"] for part in MODELS[name]["files"].values())


def offered(name, cards):
    """(may this model be offered on this PC?, why or why not, in words).
    `cards` is what `describe.devices` found, the biggest first."""
    needs = MODELS[name].get("needs_card_mb")
    if not needs:
        return True, "works on any PC (a graphics card of 8 GB or more makes it fast)"
    if cards and cards[0]["memory_mb"] >= needs:
        return True, "found %s with %d MB" % (cards[0]["name"], cards[0]["memory_mb"])
    found = "found %s with %d MB" % (cards[0]["name"], cards[0]["memory_mb"]) if cards else "found none"
    return False, "needs a graphics card with %d GB or more (%s)" % (needs // 1000, found)


def model_json(name):
    """What goes into the model's folder as `model.json`."""
    model = MODELS[name]
    return {
        "name": name,
        "size": model["size"],
        "source": "https://huggingface.co/%s" % model["repo"],
        "revision": model["revision"],
        "licence": model["licence"],
        "listening_test": model["listening_test"],
        "files": {part: file["name"] for part, file in model["files"].items()},
        "sha256": {file["name"]: file["sha256"] for file in model["files"].values()},
        "bytes": total_bytes(name),
        "settings": model["settings"],
    }
