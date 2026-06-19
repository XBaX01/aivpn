#!/bin/bash
# Build AIVPN Windows package with native GUI (egui)
#
# Prerequisites:
#   - Rust with x86_64-pc-windows-gnu target
#   - mingw-w64 cross compiler
#   - zip (optional — falls back to python3 -m zipfile)
#   - makensis / nsis (optional — skips installer build if absent)
#
# Usage: ./build-windows-gui.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
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

# Copy icon into package dir so NSIS can find it with a simple relative path
cp "aivpn-windows/assets/aivpn.ico" "$PACKAGE_DIR/aivpn.ico"

# Create zip — prefer system zip, fall back to python3
ZIP_NAME="aivpn-windows-gui.zip"
echo "Creating ${ZIP_NAME}..."
if command -v zip &>/dev/null; then
    (cd "$PACKAGE_DIR" && zip -r "../${ZIP_NAME}" ./*)
else
    echo "  zip not found — using python3 zipfile"
    python3 - <<EOF
import zipfile, pathlib
pkg = pathlib.Path("${PACKAGE_DIR}")
with zipfile.ZipFile("${ZIP_NAME}", "w", zipfile.ZIP_DEFLATED) as z:
    for f in pkg.iterdir():
        z.write(f, f.name)
        print(f"  added {f.name}")
EOF
fi

# Build NSIS installer if makensis is available
INSTALLER_NSI="${SCRIPT_DIR}/windows-installer/aivpn-installer.nsi"
INSTALLER_EXE="${SCRIPT_DIR}/releases/aivpn-windows-installer.exe"
APP_VERSION=$(grep -m1 '^version' "${SCRIPT_DIR}/Cargo.toml" | sed 's/.*"\(.*\)"/\1/')
if command -v makensis &>/dev/null && [ -f "$INSTALLER_NSI" ]; then
    echo ""
    echo "Building NSIS installer (v${APP_VERSION})..."
    makensis -V2 \
        "-DAPP_VERSION=${APP_VERSION}" \
        "-DSTAGE_DIR=${SCRIPT_DIR}/${PACKAGE_DIR}" \
        "-DOUTPUT_EXE=${INSTALLER_EXE}" \
        "$INSTALLER_NSI"
    echo "Installer: ${INSTALLER_EXE} ($(du -sh "$INSTALLER_EXE" | cut -f1))"
else
    echo ""
    echo "makensis not found — skipping installer (install nsis to enable)"
fi

# Show result
echo ""
echo "=== Build complete ==="
echo "Package: ${ZIP_NAME}"
echo "Contents:"
ls -lh "$PACKAGE_DIR/"
echo ""
echo "Total size:"
du -sh "$PACKAGE_DIR"
