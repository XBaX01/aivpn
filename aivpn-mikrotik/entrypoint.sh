#!/bin/sh
set -e

if [ -z "${AIVPN_KEY}" ]; then
    echo "[aivpn-mikrotik] ERROR: AIVPN_KEY environment variable is required" >&2
    echo "[aivpn-mikrotik] Set it in the RouterOS container envlist:" >&2
    echo "[aivpn-mikrotik]   /container/envs/add list=aivpn-env name=AIVPN_KEY value=\"aivpn://...\"" >&2
    exit 1
fi

if ! (exec 3>/dev/net/tun) 2>/dev/null; then
    echo "[aivpn-mikrotik] ERROR: Cannot open /dev/net/tun — ensure cap=net-admin is set and the tun module is loaded" >&2
    exit 1
fi

# Enable IP forwarding and set up NAT for gateway mode
sysctl -w net.ipv4.ip_forward=1 >/dev/null 2>&1 || true
iptables -t nat -C POSTROUTING -o tun0 -j MASQUERADE 2>/dev/null || \
    iptables -t nat -A POSTROUTING -o tun0 -j MASQUERADE || true
iptables -C FORWARD -i eth0 -o tun0 -j ACCEPT 2>/dev/null || \
    iptables -A FORWARD -i eth0 -o tun0 -j ACCEPT || true
iptables -C FORWARD -i tun0 -o eth0 -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || \
    iptables -A FORWARD -i tun0 -o eth0 -m state --state RELATED,ESTABLISHED -j ACCEPT || true

# Optional: full tunnel mode. Default: false (gateway mode — RouterOS handles routing).
# Set AIVPN_FULL_TUNNEL=true only for client-mode containers.
FULL_TUNNEL="${AIVPN_FULL_TUNNEL:-false}"

echo "[aivpn-mikrotik] Starting aivpn-client (full-tunnel=${FULL_TUNNEL})"

while true; do
    if [ "${FULL_TUNNEL}" = "true" ]; then
        /usr/local/bin/aivpn-client --connection-key "${AIVPN_KEY}" --full-tunnel
    else
        /usr/local/bin/aivpn-client --connection-key "${AIVPN_KEY}"
    fi
    echo "[aivpn-mikrotik] aivpn-client exited ($?), restarting in 5s..."
    sleep 5
done
