#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "=== 1. npm install ==="
npm ci

echo "=== 2. Build CLI ==="
cd src-tauri
cargo build --release --features cli --bin rssh-cli --target aarch64-apple-darwin
mkdir -p bin
cp target/aarch64-apple-darwin/release/rssh-cli bin/
cd ..

echo "=== 3. Build Tauri app ==="
npx tauri build --target aarch64-apple-darwin

echo "=== Done ==="
ls -lh src-tauri/target/aarch64-apple-darwin/release/bundle/macos/*.app
ls -lh src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null || true
