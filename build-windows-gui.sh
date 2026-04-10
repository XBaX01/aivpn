#!/bin/bash
# Build AIVPN Windows package with native GUI (egui)
#
# Prerequisites:
#   - Rust with x86_64-pc-windows-gnu target
#   - mingw-w64 cross compiler
#
# Usage: ./build-windows-gui.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

TARGET="x86_64-pc-windows-gnu"
RELEASE_DIR="target/${TARGET}/release"
PACKAGE_DIR="aivpn-windows-gui-package"

echo "=== Building AIVPN Windows GUI ==="

# Check toolchain
if ! rustup target list --installed | grep -q "$TARGET"; then
    echo "Installing target ${TARGET}..."
    rustup target add "$TARGET"
fi

# Build both binaries
echo "Building aivpn-client.exe..."
cargo build --release --target "$TARGET" -p aivpn-client

echo "Building aivpn.exe (GUI)..."
cargo build --release --target "$TARGET" -p aivpn-windows

# Create package
echo "Creating package..."
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR"

cp "${RELEASE_DIR}/aivpn.exe" "$PACKAGE_DIR/"
cp "${RELEASE_DIR}/aivpn-client.exe" "$PACKAGE_DIR/"

# Download wintun.dll if not present
WINTUN_DLL="$PACKAGE_DIR/wintun.dll"
if [ ! -f "$WINTUN_DLL" ]; then
    echo "Downloading wintun.dll..."
    WINTUN_ZIP="/tmp/wintun-0.14.1.zip"
    if [ ! -f "$WINTUN_ZIP" ]; then
        curl -L -o "$WINTUN_ZIP" "https://www.wintun.net/builds/wintun-0.14.1.zip"
    fi
    unzip -o "$WINTUN_ZIP" "wintun/bin/amd64/wintun.dll" -d /tmp/
    cp /tmp/wintun/bin/amd64/wintun.dll "$WINTUN_DLL"
fi

# Create zip
ZIP_NAME="aivpn-windows-gui.zip"
echo "Creating ${ZIP_NAME}..."
cd "$PACKAGE_DIR"
zip -r "../${ZIP_NAME}" ./*
cd ..

# Show result
echo ""
echo "=== Build complete ==="
echo "Package: ${ZIP_NAME}"
echo "Contents:"
ls -lh "$PACKAGE_DIR/"
echo ""
echo "Total size:"
du -sh "$PACKAGE_DIR"
