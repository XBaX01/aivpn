#!/bin/bash

set -euo pipefail

ARTIFACT_PATH="releases/aivpn-server-linux-arm64"

echo "=== Building Linux server arm64 release ==="
echo ""
echo "Building Docker builder image with cross-compilation..."

# Use a Docker image with aarch64 cross-compilation tools
docker run --rm -v "$(pwd)":/aivpn -w /aivpn \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  -e CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
  debian:bookworm bash -c "
    apt-get update && 
    apt-get install -y curl build-essential gcc-aarch64-linux-gnu &&
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable &&
    . \$HOME/.cargo/env &&
    rustup target add aarch64-unknown-linux-gnu &&
    cargo build --release -p aivpn-server --target aarch64-unknown-linux-gnu
  "

mkdir -p releases
cp target/aarch64-unknown-linux-gnu/release/aivpn-server "$ARTIFACT_PATH"
chmod +x "$ARTIFACT_PATH"

echo ""
echo "=== Artifact Ready ==="
ls -lh "$ARTIFACT_PATH"
file "$ARTIFACT_PATH" || true
