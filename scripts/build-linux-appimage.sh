#!/usr/bin/env bash
# Build aivpn-linux as an AppImage.
# Requires: appimagetool (https://github.com/AppImage/AppImageKit), Rust toolchain.
set -euo pipefail

ARCH="${ARCH:-x86_64}"
APPDIR="AppDir-aivpn-linux"

echo "==> Building aivpn-linux release binary..."
cargo build --release -p aivpn-linux

echo "==> Building aivpn-client release binary..."
cargo build --release -p aivpn-client

echo "==> Setting up AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp target/release/aivpn-linux "$APPDIR/usr/bin/"
cp target/release/aivpn-client "$APPDIR/usr/bin/"

cat > "$APPDIR/usr/share/applications/aivpn.desktop" <<EOF
[Desktop Entry]
Name=AIVPN
Comment=AI-powered VPN for censorship circumvention
Exec=aivpn-linux
Icon=aivpn
Type=Application
Categories=Network;
EOF

cp "$APPDIR/usr/share/applications/aivpn.desktop" "$APPDIR/"

if [ -f "aivpn-linux/assets/icon.png" ]; then
    cp "aivpn-linux/assets/icon.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/aivpn.png"
    cp "aivpn-linux/assets/icon.png" "$APPDIR/aivpn.png"
else
    echo "WARN: no icon at aivpn-linux/assets/icon.png — AppImage will have no icon"
    touch "$APPDIR/aivpn.png"
fi

cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
SELF="$(readlink -f "$0")"
HERE="${SELF%/*}"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/aivpn-linux" "$@"
APPRUN
chmod +x "$APPDIR/AppRun"

echo "==> Packaging AppImage..."
APPIMAGETOOL="${APPIMAGETOOL:-appimagetool}"
if ! command -v "$APPIMAGETOOL" &>/dev/null; then
    echo "ERROR: appimagetool not found. Download from https://github.com/AppImage/AppImageKit/releases"
    exit 1
fi

OUTPUT="aivpn-linux-${ARCH}.AppImage"
ARCH="$ARCH" "$APPIMAGETOOL" "$APPDIR" "$OUTPUT"
echo "==> Done: $OUTPUT"
