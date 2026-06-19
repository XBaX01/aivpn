#!/bin/bash
# AIVPN Build Script

set -e

echo "=== AIVPN Build Script ==="
echo ""

# Check for Rust
if ! command -v cargo &> /dev/null; then
    echo "Error: cargo (Rust) is not installed"
    echo "Please install Rust from https://rustup.rs/"
    exit 1
fi

echo "Rust version: $(rustc --version)"
echo ""

# Build all targets
echo "Building workspace..."
cargo build --release

echo ""
echo "=== Build Complete ==="
echo ""
echo "Binaries:"
echo "  - Server: target/release/aivpn-server"
echo "  - Client: target/release/aivpn-client"
echo ""

# Show binary sizes
if [ -f "target/release/aivpn-server" ]; then
    echo "Server binary size: $(du -h target/release/aivpn-server | cut -f1)"
fi
if [ -f "target/release/aivpn-client" ]; then
    echo "Client binary size: $(du -h target/release/aivpn-client | cut -f1)"
fi

echo ""
echo "=== Installing client binary system-wide ==="

HELPER_PLIST="/Library/LaunchDaemons/com.aivpn.helper.plist"
CLIENT_DST="/Library/Application Support/AIVPN/aivpn-client"

# Остановить helper-демон (если запущен)
if [ -f "$HELPER_PLIST" ]; then
    echo "Stopping AIVPN helper..."
    sudo launchctl unload "$HELPER_PLIST" || true
fi

# Копировать новый бинарник клиента
echo "Copying client binary to $CLIENT_DST"
sudo mkdir -p "/Library/Application Support/AIVPN"
sudo cp -f target/release/aivpn-client "$CLIENT_DST"
sudo chmod +x "$CLIENT_DST"

# Запустить helper-демон обратно
if [ -f "$HELPER_PLIST" ]; then
    echo "Starting AIVPN helper..."
    sudo launchctl load "$HELPER_PLIST"
fi

echo "=== Install complete ==="
