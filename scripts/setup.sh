#!/bin/bash
# AIVPN Development Setup Script

set -e

echo "=== AIVPN Development Setup ==="
echo ""

# Check for Rust
if ! command -v cargo &> /dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    source $HOME/.cargo/env
fi

echo "Rust version: $(rustc --version)"

# Install development dependencies
echo ""
echo "Installing development tools..."

# cargo-watch for hot reloading
if ! command -v cargo-watch &> /dev/null; then
    cargo install cargo-watch
    echo "  ✓ cargo-watch installed"
fi

# cargo-audit for security audits
if ! command -v cargo-audit &> /dev/null; then
    cargo install cargo-audit
    echo "  ✓ cargo-audit installed"
fi

# Run clippy
echo ""
echo "Running clippy..."
cargo clippy --all-targets --all-features -- -D warnings

# Run tests
echo ""
echo "Running tests..."
cargo test --workspace

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Quick start commands:"
echo "  Build:          ./build.sh"
echo "  Run server:     cargo run --bin aivpn-server -- --listen 0.0.0.0:443"
echo "  Run client:     cargo run --bin aivpn-client -- --server <SERVER_IP>:443 --server-key <KEY>"
echo "  Watch & run:    cargo watch -x 'run --bin aivpn-server'"
echo "  Run tests:      cargo test --workspace"
echo "  Security audit: cargo audit"
echo ""
