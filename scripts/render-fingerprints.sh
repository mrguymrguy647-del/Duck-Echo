#!/usr/bin/env bash
# Render every example recipe and every preset with the release build, and
# write one line per WAV file: its SHA-256 hash and its name. CI runs this
# on Linux and on Windows and checks that the two lists are identical:
# the same recipe must give the same file on every platform.
#
# Usage: scripts/render-fingerprints.sh [output-folder]   (default: renders)
# Run it from the repository root after `cargo build --release -p duck-echo-cli`.
set -euo pipefail

out="${1:-renders}"
bin="target/release/duck-echo"
mkdir -p "$out"

# Examples: coin.toml and coin.json are the same recipe, so keep the
# extension in the name to render both.
for recipe in examples/recipes/*.toml examples/recipes/*.json; do
  base="$(basename "$recipe")"
  "$bin" render "$recipe" -o "$out/example-${base//./-}.wav" 2>/dev/null
done

# Presets: the first word of each line of `presets list` is a name.
for name in $("$bin" presets list | cut -d' ' -f1); do
  "$bin" render "preset:$name" -o "$out/preset-$name.wav" 2>/dev/null
done

# `sha256sum` prints "hash  name" on Linux but may print "hash *name" on
# Windows, so print the two columns ourselves.
for wav in "$out"/*.wav; do
  hash="$(sha256sum "$wav" | cut -d' ' -f1)"
  echo "$hash  $(basename "$wav")"
done | LC_ALL=C sort -k2 > "$out/SHA256SUMS"

echo "$(wc -l < "$out/SHA256SUMS") files fingerprinted in $out/SHA256SUMS" >&2
