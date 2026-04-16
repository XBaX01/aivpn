#!/bin/sh
set -eu

CONFIG_DIR="/etc/aivpn"
CONFIG_PATH="$CONFIG_DIR/server.json"
CONFIG_TEMPLATE="/usr/share/aivpn/server.json.example"
KEY_PATH="$CONFIG_DIR/server.key"

mkdir -p "$CONFIG_DIR"

if [ ! -f "$CONFIG_PATH" ]; then
    cp "$CONFIG_TEMPLATE" "$CONFIG_PATH"
    echo "Initialized $CONFIG_PATH from bundled template"
fi

if [ ! -f "$KEY_PATH" ]; then
    umask 077
    head -c 32 /dev/urandom > "$KEY_PATH"
    echo "Generated $KEY_PATH"
fi

exec /usr/local/bin/aivpn-server "$@"