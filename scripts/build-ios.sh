#!/usr/bin/env bash
# build-ios.sh — Build the AIVPN iOS app and copy the .ipa to releases/
#
# Prerequisites (macOS only):
#   Xcode 15+  (xcodebuild, xcodegen)
#   rustup targets: aarch64-apple-ios  aarch64-apple-ios-sim  x86_64-apple-ios
#   cargo install xcodegen  (https://github.com/yonaskolb/XcodeGen)
#
# Usage:
#   ./build-ios.sh                # unsigned build (no real-device install)
#   ./build-ios.sh AD12XXXXXX     # your 10-char Apple Team ID → development export

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
IOS_DIR="$REPO_ROOT/aivpn-ios"
CORE_DIR="$REPO_ROOT/aivpn-ios-core"
RELEASES_DIR="$REPO_ROOT/releases"
LIB_DIR="$CORE_DIR/lib"

TEAM_ID="${1:-}"
CONFIGURATION="${CONFIGURATION:-Release}"
TARGET_DIR="$REPO_ROOT/target"

# ── 1. Rust: device + both simulator slices ───────────────────────────────────

echo "==> Building Rust core for aarch64-apple-ios (device) …"
cargo build --release -p aivpn-ios-core --target aarch64-apple-ios

echo "==> Building Rust core for aarch64-apple-ios-sim (Apple Silicon simulator) …"
cargo build --release -p aivpn-ios-core --target aarch64-apple-ios-sim

echo "==> Building Rust core for x86_64-apple-ios (Intel simulator) …"
cargo build --release -p aivpn-ios-core --target x86_64-apple-ios

# ── 2. Combine simulator slices into a fat lib, then XCFramework ──────────────

mkdir -p "$LIB_DIR"

DEVICE_LIB="$TARGET_DIR/aarch64-apple-ios/release/libaivpn_core.a"
SIM_ARM_LIB="$TARGET_DIR/aarch64-apple-ios-sim/release/libaivpn_core.a"
SIM_X86_LIB="$TARGET_DIR/x86_64-apple-ios/release/libaivpn_core.a"
SIM_FAT_LIB="$LIB_DIR/libaivpn_core_sim.a"

echo "==> Lipo: universal simulator lib …"
lipo -create "$SIM_ARM_LIB" "$SIM_X86_LIB" -output "$SIM_FAT_LIB"

echo "==> Creating XCFramework …"
XCFRAMEWORK="$LIB_DIR/AivpnCore.xcframework"
rm -rf "$XCFRAMEWORK"
xcodebuild -create-xcframework \
    -library "$DEVICE_LIB" -headers "$CORE_DIR/include" \
    -library "$SIM_FAT_LIB" -headers "$CORE_DIR/include" \
    -output "$XCFRAMEWORK"

# Also keep a plain device .a for the direct linker flag in project.yml
cp "$DEVICE_LIB" "$LIB_DIR/libaivpn_core.a"

# ── 3. XcodeGen ──────────────────────────────────────────────────────────────

echo "==> Generating Xcode project …"
cd "$IOS_DIR"
xcodegen generate --spec project.yml

# ── 4. Archive ───────────────────────────────────────────────────────────────

ARCHIVE_PATH="$IOS_DIR/build/Aivpn.xcarchive"
mkdir -p "$IOS_DIR/build"

if [ -n "$TEAM_ID" ]; then
    SIGN_ARGS="DEVELOPMENT_TEAM=$TEAM_ID CODE_SIGN_STYLE=Automatic"
else
    # Unsigned — useful for CI and simulator testing
    SIGN_ARGS="CODE_SIGN_IDENTITY=- CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO"
fi

echo "==> Archiving (${CONFIGURATION}) …"
# shellcheck disable=SC2086
xcodebuild archive \
    -project Aivpn.xcodeproj \
    -scheme Aivpn \
    -configuration "$CONFIGURATION" \
    -destination "generic/platform=iOS" \
    -archivePath "$ARCHIVE_PATH" \
    $SIGN_ARGS \
    SKIP_INSTALL=NO \
    BUILD_LIBRARY_FOR_DISTRIBUTION=NO

# ── 5. Export / package ───────────────────────────────────────────────────────

EXPORT_PATH="$IOS_DIR/build/export"
mkdir -p "$EXPORT_PATH"
DEST="$RELEASES_DIR/aivpn-ios.ipa"
mkdir -p "$RELEASES_DIR"

if [ -n "$TEAM_ID" ]; then
    # Signed: export a proper .ipa
    EXPORT_OPTIONS="$IOS_DIR/build/ExportOptions.plist"
    cat > "$EXPORT_OPTIONS" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>method</key>
    <string>development</string>
    <key>teamID</key>
    <string>${TEAM_ID}</string>
    <key>compileBitcode</key>
    <false/>
    <key>stripSwiftSymbols</key>
    <true/>
</dict>
</plist>
PLIST

    xcodebuild -exportArchive \
        -archivePath "$ARCHIVE_PATH" \
        -exportPath "$EXPORT_PATH" \
        -exportOptionsPlist "$EXPORT_OPTIONS"

    IPA_SRC="$(find "$EXPORT_PATH" -name "*.ipa" | head -1)"
    if [ -z "$IPA_SRC" ]; then
        echo "ERROR: .ipa not found in $EXPORT_PATH"
        exit 1
    fi
    cp "$IPA_SRC" "$DEST"
else
    # Unsigned: package the .app from the archive into a zip-based .ipa manually
    APP_PATH="$(find "$ARCHIVE_PATH/Products" -name "*.app" | head -1)"
    if [ -z "$APP_PATH" ]; then
        echo "ERROR: .app not found in archive $ARCHIVE_PATH"
        exit 1
    fi
    PAYLOAD_DIR="$IOS_DIR/build/Payload"
    rm -rf "$PAYLOAD_DIR"
    mkdir -p "$PAYLOAD_DIR"
    cp -r "$APP_PATH" "$PAYLOAD_DIR/"
    (cd "$IOS_DIR/build" && zip -qr "$DEST" Payload)
    rm -rf "$PAYLOAD_DIR"
fi

echo "==> Artifact: $DEST  ($(du -sh "$DEST" | cut -f1))"
echo ""
if [ -n "$TEAM_ID" ]; then
    echo "Install on device:"
    echo "  ios-deploy --bundle $DEST"
    echo "  or: xcrun devicectl device install app --device <UDID> $DEST"
else
    echo "Unsigned build — for device install, re-run with your Team ID:"
    echo "  ./build-ios.sh YOUR_TEAM_ID"
fi
