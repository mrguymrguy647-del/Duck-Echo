#!/bin/sh
# Playtest 4: plays the part of the person running the game. From the
# Duck Forge folder, Duck Craft replays its recorded playthrough headless
# and writes its log.json (and audio.wav) into this playtest's out/
# folder, inside the Duck Echo project, for mix_check.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
DUCK_FORGE=${DUCK_FORGE:-/home/user/df-playtest}
cd "$DUCK_FORGE"
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/home/user/duck-forge/target} \
  cargo run -q --example duck_craft -- --headless \
  --inputs playthroughs/duck_craft.json --out "$HERE/out/duck_craft"
