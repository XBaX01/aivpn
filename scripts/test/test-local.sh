#!/bin/bash
# AIVPN Local Test Script

set -e

echo "=== AIVPN Local Test ==="
echo ""

cd /Users/oleg/Documents/aivpn

# Check binaries exist
echo "📦 Checking binaries..."
if [ ! -f "target/release/aivpn-server" ]; then
    echo "❌ Server binary not found! Building..."
    cargo build --release --bin aivpn-server
fi

if [ ! -f "target/release/aivpn-client" ]; then
    echo "❌ Client binary not found! Building..."
    cargo build --release --bin aivpn-client
fi

echo "✅ Binaries found:"
ls -lh target/release/aivpn-*
echo ""

# Show binary info
echo "📊 Binary information:"
file target/release/aivpn-server
file target/release/aivpn-client
echo ""

# Check help
echo "📖 Server help:"
./target/release/aivpn-server --help | head -15
echo ""

echo "📖 Client help:"
./target/release/aivpn-client --help | head -15
echo ""

# Version check
echo "🏷️  Version:"
echo "  Server: $(./target/release/aivpn-server --version 2>&1 | head -1)"
echo "  Client: $(./target/release/aivpn-client --version 2>&1 | head -1)"
echo ""

echo "=== Test Complete ==="
echo ""
echo "Next steps:"
echo "  1. Start server: sudo ./target/release/aivpn-server --listen 0.0.0.0:443"
echo "  2. Get server key from output"
echo "  3. Start client: sudo ./target/release/aivpn-client --server <IP>:443 --server-key <KEY>"
echo ""
