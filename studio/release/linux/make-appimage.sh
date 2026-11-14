#!/bin/bash
# Build DuckEchoStudio.AppImage from three built programs.
#
#   studio/release/linux/make-appimage.sh BIN_DIR STUDIO_BIN OUT_FILE
#
# BIN_DIR holds duck-echo and duck-echo-mcp (the libraries' workspace,
# target/release); STUDIO_BIN is DuckEchoStudio (studio/target/release).
# appimagetool must be on the PATH, or named by APPIMAGETOOL.
#
# The AppImage holds the three programs, the menu entry and the icon. The
# programs use the system's own libraries for the window, the graphics and
# the sound (libasound.so.2, which every Linux desktop has), so nothing is
# copied in for those.
set -euo pipefail
bin_dir=$1
studio_bin=$2
out=$3
here="$(cd "$(dirname "$0")" && pwd)"
brand="$here/../../brand"
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/../../Cargo.toml" | head -n 1)

appdir=$(mktemp -d)/DuckEchoStudio.AppDir
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" \
         "$appdir/usr/share/icons/hicolor/256x256/apps"
install -m 755 "$studio_bin" "$appdir/usr/bin/DuckEchoStudio"
install -m 755 "$bin_dir/duck-echo" "$appdir/usr/bin/duck-echo"
install -m 755 "$bin_dir/duck-echo-mcp" "$appdir/usr/bin/duck-echo-mcp"
install -m 755 "$here/AppRun" "$appdir/AppRun"
install -m 644 "$here/duck-echo-studio.desktop" "$appdir/duck-echo-studio.desktop"
install -m 644 "$here/duck-echo-studio.desktop" "$appdir/usr/share/applications/"
install -m 644 "$brand/duck-echo-icon-256.png" "$appdir/duck-echo-studio.png"
install -m 644 "$brand/duck-echo-icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/duck-echo-studio.png"
ln -s duck-echo-studio.png "$appdir/.DirIcon"

tool=${APPIMAGETOOL:-appimagetool}
# Run appimagetool (itself an AppImage) without FUSE, as on CI machines.
export APPIMAGE_EXTRACT_AND_RUN=1
# --no-appstream: there is no AppStream metadata file to check.
ARCH=x86_64 VERSION="$version" "$tool" --no-appstream "$appdir" "$out"
rm -rf "$(dirname "$appdir")"
ls -l "$out"
