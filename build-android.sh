#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# ── Prerequisites check ──
for cmd in rustup npm npx java; do
    command -v "$cmd" >/dev/null || { echo "Missing: $cmd"; exit 1; }
done

if [ -z "${ANDROID_HOME:-}" ] && [ -z "${ANDROID_SDK_ROOT:-}" ]; then
    # Common default paths
    if [ -d "$HOME/Library/Android/sdk" ]; then
        export ANDROID_HOME="$HOME/Library/Android/sdk"
    elif [ -d "$HOME/Android/Sdk" ]; then
        export ANDROID_HOME="$HOME/Android/Sdk"
    else
        echo "Error: ANDROID_HOME not set and SDK not found in default locations."
        exit 1
    fi
fi
export ANDROID_SDK_ROOT="${ANDROID_HOME:-$ANDROID_SDK_ROOT}"
export NDK_HOME="${NDK_HOME:-$ANDROID_SDK_ROOT/ndk/$(ls "$ANDROID_SDK_ROOT/ndk/" 2>/dev/null | sort -V | tail -1)}"

if [ ! -d "$NDK_HOME" ]; then
    echo "Error: NDK not found at $NDK_HOME"
    echo "Install via: sdkmanager --install 'ndk;27.0.12077973'"
    exit 1
fi

echo "SDK: $ANDROID_SDK_ROOT"
echo "NDK: $NDK_HOME"

# ── Rust Android targets ──
TARGETS=(aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android)
for t in "${TARGETS[@]}"; do
    rustup target add "$t" 2>/dev/null || true
done

echo "=== 1. npm install ==="
npm ci

echo "=== 2. Build Tauri Android (APK) ==="
npx tauri android build

echo "=== Done ==="
echo ""

# Find output APKs
APK_DIR="src-tauri/gen/android/app/build/outputs/apk"
if [ -d "$APK_DIR" ]; then
    echo "APKs:"
    find "$APK_DIR" -name "*.apk" -exec ls -lh {} \;
fi

# Find output AABs
AAB_DIR="src-tauri/gen/android/app/build/outputs/bundle"
if [ -d "$AAB_DIR" ]; then
    echo ""
    echo "AABs:"
    find "$AAB_DIR" -name "*.aab" -exec ls -lh {} \;
fi
