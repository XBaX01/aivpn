#!/bin/sh
set -e

if [ -z "${AIVPN_KEY}" ]; then
    echo "[aivpn-mikrotik] ERROR: AIVPN_KEY environment variable is required" >&2
    echo "[aivpn-mikrotik] Set it in the RouterOS container envlist:" >&2
    echo "[aivpn-mikrotik]   /container/envs/add list=aivpn-env name=AIVPN_KEY value=\"aivpn://...\"" >&2
    exit 1
fi

if [ ! -c /dev/net/tun ]; then
    echo "[aivpn-mikrotik] ERROR: /dev/net/tun not found. Mount it from the RouterOS host:" >&2
    echo "[aivpn-mikrotik]   /container/mounts/add name=tun src=/dev/net/tun dst=/dev/net/tun type=bind" >&2
    exit 1
fi

# Optional: full tunnel mode (routes all traffic through VPN). Default: true.
FULL_TUNNEL="${AIVPN_FULL_TUNNEL:-true}"

ARGS="--connection-key ${AIVPN_KEY}"
if [ "${FULL_TUNNEL}" = "true" ]; then
    ARGS="${ARGS} --full-tunnel"
fi

echo "[aivpn-mikrotik] Starting aivpn-client (full-tunnel=${FULL_TUNNEL})"
exec /usr/local/bin/aivpn-client ${ARGS}
