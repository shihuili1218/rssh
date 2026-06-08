#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Linux ships on both x86_64 and arm64 boxes, so derive the target triple from
# the host instead of hardcoding it (build-mac.sh can hardcode — Apple Silicon
# only). Override by exporting TARGET=... before running.
case "$(uname -m)" in
    x86_64)          ARCH_TRIPLE="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64)   ARCH_TRIPLE="aarch64-unknown-linux-gnu" ;;
    *)               echo "Unsupported arch: $(uname -m)"; exit 1 ;;
esac
TARGET="${TARGET:-$ARCH_TRIPLE}"
echo "Target: $TARGET"
rustup target add "$TARGET" 2>/dev/null || true

echo "=== 1. npm install ==="
npm ci

echo "=== 2. Build CLI ==="
cd src-tauri
cargo build --release --features cli --bin rssh-cli --target "$TARGET"
mkdir -p bin
cp "target/$TARGET/release/rssh-cli" bin/
cd ..

echo "=== 3. Build Tauri app ==="
npx tauri build --target "$TARGET"

BUNDLE="src-tauri/target/$TARGET/release/bundle"

echo "=== 4. Repack AppImage with a relocatable icon symlink (issue #91) ==="
APPIMAGE="$(ls "$BUNDLE"/appimage/*.AppImage 2>/dev/null | head -1 || true)"
if [ -n "$APPIMAGE" ]; then
    bash .github/scripts/fix-appimage-icon.sh "$APPIMAGE"
else
    echo "No AppImage produced; skipping repack."
fi

echo "=== Done ==="
ls -lh "$BUNDLE"/deb/*.deb 2>/dev/null || true
ls -lh "$BUNDLE"/rpm/*.rpm 2>/dev/null || true
ls -lh "$BUNDLE"/appimage/*.AppImage 2>/dev/null || true
