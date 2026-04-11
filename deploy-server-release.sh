#!/bin/bash

set -euo pipefail

REPO_SLUG="${AIVPN_REPO_SLUG:-infosave2007/aivpn}"
RELEASE_TAG="${AIVPN_RELEASE_TAG:-latest}"
ASSET_NAME="aivpn-server-linux-x86_64"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RELEASES_DIR="$SCRIPT_DIR/releases"
ARTIFACT_PATH="$RELEASES_DIR/$ASSET_NAME"

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Error: required command '$1' is not installed" >&2
        exit 1
    fi
}

run_privileged() {
    if [ "${EUID:-$(id -u)}" -eq 0 ]; then
        "$@"
    else
        require_command sudo
        sudo "$@"
    fi
}

download_latest_asset() {
    local url
    url="https://github.com/$REPO_SLUG/releases/latest/download/$ASSET_NAME"
    curl -fL "$url" -o "$ARTIFACT_PATH"
}

download_tagged_asset() {
    local api_url download_url
    api_url="https://api.github.com/repos/$REPO_SLUG/releases/tags/$RELEASE_TAG"
    download_url="$(
        curl -fsSL "$api_url" | python3 -c '
import json, sys
asset_name = sys.argv[1]
data = json.load(sys.stdin)
for asset in data.get("assets", []):
    if asset.get("name") == asset_name:
        print(asset.get("browser_download_url", ""))
        break
' "$ASSET_NAME"
    )"

    if [ -z "$download_url" ]; then
        echo "Error: asset $ASSET_NAME not found in release tag $RELEASE_TAG" >&2
        exit 1
    fi

    curl -fL "$download_url" -o "$ARTIFACT_PATH"
}

echo "=== AIVPN VPS fast deploy ==="

require_command curl
require_command docker
require_command python3

if ! docker compose version >/dev/null 2>&1; then
    echo "Error: docker compose plugin is required" >&2
    exit 1
fi

mkdir -p "$RELEASES_DIR" "$SCRIPT_DIR/config"

if [ ! -f "$SCRIPT_DIR/config/server.key" ]; then
    require_command openssl
    echo "Generating config/server.key"
    openssl rand 32 > "$SCRIPT_DIR/config/server.key"
    chmod 600 "$SCRIPT_DIR/config/server.key"
fi

echo "Downloading server release asset: $ASSET_NAME"
if [ "$RELEASE_TAG" = "latest" ]; then
    download_latest_asset
else
    download_tagged_asset
fi
chmod +x "$ARTIFACT_PATH"

echo "Enabling IPv4 forwarding"
run_privileged sysctl -w net.ipv4.ip_forward=1 >/dev/null

DEFAULT_IFACE="$(ip route show default 2>/dev/null | awk '/default/ {print $5; exit}')"
if [ -n "$DEFAULT_IFACE" ]; then
    echo "Ensuring NAT rule on interface $DEFAULT_IFACE"
    if ! run_privileged iptables -t nat -C POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE >/dev/null 2>&1; then
        run_privileged iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE
    fi
else
    echo "Warning: default network interface not detected; skipping NAT rule setup" >&2
fi

if command -v ufw >/dev/null 2>&1 && run_privileged ufw status | grep -q '^Status: active'; then
    echo "Ensuring UFW allows UDP 443"
    run_privileged ufw allow 443/udp >/dev/null
fi

echo "Starting server from prebuilt release binary"
cd "$SCRIPT_DIR"
AIVPN_SERVER_DOCKERFILE=Dockerfile.prebuilt docker compose up -d aivpn-server

echo ""
echo "Server deployed."
echo "Manage clients with: docker compose exec aivpn-server aivpn-server --help"