#!/usr/bin/env bash
# Rewrite an AppImage's .DirIcon from an absolute symlink to a relative one and
# repack it.
#
# linuxdeploy bakes .DirIcon as an ABSOLUTE symlink into the build machine's
# path (e.g. /home/runner/work/rssh/rssh/.../RSSH.AppDir/RSSH.png). That target
# does not exist on the user's box, so tools that unpack/validate the AppImage
# (AppManager, `--appimage-extract`) fail with "Symlink target not found". The
# icon file itself sits in the AppDir root, so a relative link is all we need.
# (issue #91)
#
# Usage: fix-appimage-icon.sh <path-to.AppImage> [arch]
#   arch defaults to the host arch (uname -m); pass x86_64 / aarch64 to override.
set -euo pipefail

APPIMAGE="${1:?usage: fix-appimage-icon.sh <path-to.AppImage> [arch]}"
case "${2:-$(uname -m)}" in
    aarch64|arm64) AI_ARCH="aarch64" ;;
    *)             AI_ARCH="x86_64" ;;
esac

# appimagetool repacks the squashfs. --appimage-extract-and-run keeps it
# self-contained (bundled mksquashfs, no FUSE) so it works on bare CI runners.
APPIMAGETOOL="$HOME/.cache/tauri/appimagetool-$AI_ARCH.AppImage"
if [ ! -x "$APPIMAGETOOL" ]; then
    mkdir -p "$(dirname "$APPIMAGETOOL")"
    curl -fsSL -o "$APPIMAGETOOL" \
        "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$AI_ARCH.AppImage"
    chmod +x "$APPIMAGETOOL"
fi

# Absolutize paths so the cd into the bundle dir below stays valid.
APPIMAGE="$(cd "$(dirname "$APPIMAGE")" && pwd)/$(basename "$APPIMAGE")"
APPIMAGETOOL="$(cd "$(dirname "$APPIMAGETOOL")" && pwd)/$(basename "$APPIMAGETOOL")"

( cd "$(dirname "$APPIMAGE")"
  base="$(basename "$APPIMAGE")"
  rm -rf squashfs-root
  "./$base" --appimage-extract >/dev/null
  if [ -L squashfs-root/.DirIcon ]; then
      # absolute -> relative: the icon sits in the same dir as .DirIcon
      ln -sf "$(basename "$(readlink squashfs-root/.DirIcon)")" squashfs-root/.DirIcon
  fi
  ARCH="$AI_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run squashfs-root "$base"
  rm -rf squashfs-root )

echo "Fixed .DirIcon in $(basename "$APPIMAGE")"
