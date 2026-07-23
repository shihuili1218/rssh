#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

if [ "$(uname -s)" != "Darwin" ]; then
    echo "Error: iOS builds require macOS."
    exit 1
fi

for cmd in rustup npm npx xcodebuild xcrun pod xcodegen; do
    command -v "$cmd" >/dev/null || { echo "Missing: $cmd"; exit 1; }
done

required_signing_vars=(
    APPLE_DEVELOPMENT_TEAM
    IOS_CERTIFICATE
    IOS_CERTIFICATE_PASSWORD
    IOS_MOBILE_PROVISION
)

for var in "${required_signing_vars[@]}"; do
    if [ -z "${!var:-}" ]; then
        echo "Error: $var is not set."
        echo "This helper expects manual iOS signing credentials from the CI secret store."
        exit 1
    fi
done

if ! xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1; then
    echo "Error: a full Xcode installation with the iOS SDK is required."
    exit 1
fi

rustup target add aarch64-apple-ios

echo "=== 1. Install frontend dependencies ==="
npm ci

if [ ! -f src-tauri/gen/apple/project.yml ]; then
    echo "=== 2. Initialize the Tauri iOS project ==="
    npx tauri ios init --ci --skip-targets-install
else
    echo "=== 2. Tauri iOS project already initialized ==="
fi

cp src-tauri/PrivacyInfo.xcprivacy src-tauri/gen/apple/PrivacyInfo.xcprivacy

echo "=== 3. Build App Store IPA ==="
npx tauri ios build --ci --export-method "${IOS_EXPORT_METHOD:-app-store-connect}" "$@"

IPA_DIR="src-tauri/gen/apple/build/arm64"
if [ -d "$IPA_DIR" ]; then
    echo "=== IPAs ==="
    find "$IPA_DIR" -maxdepth 1 -name "*.ipa" -exec ls -lh {} \;
fi
