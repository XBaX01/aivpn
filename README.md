🌐 [Русский](README_RU.md) | [中文](README_CN.md)

# AIVPN

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android%20%7C%20iOS%20%7C%20MikroTik-informational)

---

## Overview

AIVPN is a UDP-based VPN system that combines standard tunnel encryption with **traffic mimicry**: outbound packets are reshaped to resemble known application protocols (WebRTC, QUIC, DNS-over-UDP), making the connection statistically indistinguishable from regular application traffic to passive observers.

Key technical properties:

- **Zero-RTT data start** — encrypted payload can flow in the first packet; no mandatory handshake round-trip.
- **O(1) session lookup** — no session ID is transmitted in the clear. Every packet carries an 8-byte *resonance tag* derived from a timestamp and a per-session secret. The server resolves the session in constant time via a `DashMap`.
- **Perfect Forward Secrecy** — in-flight session key rotation via X25519 ratchet. Compromising the server key does not expose past traffic.
- **Neural Resonance module** — per-mask micro-MLP (~66 KB) monitors live traffic statistics; high reconstruction error triggers automatic mask rotation without disconnecting clients.
- **Written in Rust** — memory-safe, no GC pauses. Client binary ≈ 2.5 MB. Runs on a $5 VPS.

---

## Architecture

### Workspace layout

```
aivpn-common/       — shared crypto, protocol, mask profiles (no I/O)
aivpn-server/       — Linux-only VPN gateway and management CLI
aivpn-client/       — cross-platform VPN client (Linux / macOS / Windows)
aivpn-android-core/ — JNI bridge for Android
aivpn-windows/      — Windows GUI (egui/eframe)
aivpn-android/      — Android Kotlin app
aivpn-macos/        — macOS SwiftUI menu bar app
aivpn-ios-core/     — iOS Rust staticlib (C FFI)
aivpn-ios/          — iOS SwiftUI app + NEPacketTunnelProvider
mask-assets/        — bundled traffic mimicry JSON profiles
```

### Key modules

| Module | Location | Purpose |
|--------|----------|---------|
| `crypto.rs` | `aivpn-common` | X25519 key exchange, ChaCha20-Poly1305 AEAD, BLAKE3/HMAC, resonance tag generation |
| `protocol.rs` | `aivpn-common` | Wire format: `[8-byte tag][pad_len][inner_header][encrypted payload][poly1305 tag]` |
| `mask.rs` | `aivpn-common` | `MaskProfile` — traffic shaping: header templates, FSM states, IAT distributions |
| `gateway.rs` | `aivpn-server` | Central event loop: UDP receive, session dispatch, NAT forwarding, neural checks |
| `session.rs` | `aivpn-server` | `SessionManager` — `DashMap`-based O(1) tag lookup, 256-entry replay window, 500-session cap |
| `neural.rs` | `aivpn-server` | Neural Resonance: per-mask MLP 64→128→64, MSE threshold 0.35, auto mask rotation |
| `client.rs` | `aivpn-client` | State machine: Unprovisioned → Connecting → Connected, key exchange, reconnection |
| `tunnel.rs` | `aivpn-client` | Cross-platform TUN: `/dev/net/tun` (Linux), `utun` (macOS), Wintun (Windows) |
| `mimicry.rs` | `aivpn-client` | `MimicryEngine` — applies `MaskProfile` to outbound packets |

### Pool sync

Server-to-server client database synchronization uses `ControlPayload::PoolSync` carried inside ordinary VPN UDP packets — indistinguishable from client traffic. No separate TCP port or firewall rule required.

---

## Platform Support

| Platform | Server | Client | GUI | TUN driver |
|----------|:------:|:------:|:---:|------------|
| Linux | ✅ | ✅ | ✅ AppImage + tray | `/dev/net/tun` |
| macOS | — | ✅ | ✅ menu bar | `utun` |
| Windows | — | ✅ | ✅ egui | [Wintun](https://www.wintun.net/) |
| Android | — | ✅ | ✅ native Kotlin | `VpnService` API |
| iOS | — | ✅ | ✅ SwiftUI | `NetworkExtension` |
| MikroTik RouterOS 7.6+ | — | ✅ | — | container veth + TUN |
| Entware routers (ARMv7 / MIPSel) | — | ✅ | — | musl static binary |

### Feature Capability Matrix

| Feature | CLI | Win | Mac | Android | iOS |
|---------|:---:|:---:|:---:|:-------:|:---:|
| Traffic Mimicry | ✅ | ✅ | ✅ | ✅ | ✅ |
| Adaptive Mode (4 levels) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Live Quality Score | ✅ | ✅ | ✅ | ✅ | ✅ |
| Split Tunnel | ✅ | ✅ | ✅ | ✅ | ✅ |
| DNS Proxy | ✅ | ✅ | ✅ | ❌ | ❌ |
| Kill Switch | ✅ | ✅ | ✅ | ✅ | ✅ |
| mTLS Certificate | ✅ | ✅ | ✅ | ✅ | ✅ |
| FEC (forward error correction) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Traffic Recording | ✅ | ✅ | ✅ | ✅ | ✅ |
| Device Key / JIT | ✅ | ✅ | ✅ | ✅ | ✅ |
| SOCKS5 Proxy | ✅ | ✅ | ✅ | ❌ | ❌ |
| Full Tunnel | ✅ | ✅ | ✅ | ✅ | ✅ |
| Diagnostics / Benchmark | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## Quick Start

### Server (Linux)

#### Docker (recommended)

```bash
mkdir -p config
docker compose up -d aivpn-server
```

The container auto-generates `server.key` and `server.json` on first start. It runs with `network_mode: host` and mounts `./config` → `/etc/aivpn`.

Open UDP port 443 in your firewall:

```bash
# UFW
sudo ufw allow 443/udp
# firewalld
sudo firewall-cmd --add-port=443/udp --permanent && sudo firewall-cmd --reload
```

#### Bare metal

```bash
sudo mkdir -p /etc/aivpn
openssl rand 32 | sudo tee /etc/aivpn/server.key > /dev/null
sudo chmod 600 /etc/aivpn/server.key
sudo ./aivpn-server --listen 0.0.0.0:443 --key-file /etc/aivpn/server.key
```

The server automatically enables IPv4 forwarding and installs NAT masquerade rules (nftables preferred, iptables fallback). No manual firewall configuration is required for the tunnel itself.

#### Add a client

```bash
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# Bare metal
aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443
```

Output includes the connection key (`aivpn://…`) — distribute it to the client.

Other management commands: `--list-clients`, `--show-client`, `--remove-client`.

---

### Client — Linux

```bash
sudo ./aivpn-client -k "aivpn://..."
# Full tunnel (route all traffic through VPN)
sudo ./aivpn-client -k "aivpn://..." --full-tunnel
```

### Client — macOS

Download `aivpn-macos.dmg` from [Releases](https://github.com/infosave2007/aivpn/releases), drag **Aivpn.app** to Applications, launch — appears in the menu bar. Paste the connection key and click **Connect**.

CLI:
```bash
sudo ./aivpn-client -k "aivpn://..."
```

> The app prompts for a password via `sudo` to create the `utun` interface.

### Client — Windows

**Installer (recommended):** download `aivpn-windows-installer.exe`, run as Administrator, launch **AIVPN** from the Start Menu.

**Portable:** extract `aivpn-windows-package.zip` (contains `aivpn.exe`, `aivpn-client.exe`, `wintun.dll`). Run `aivpn.exe` as Administrator.

CLI (PowerShell, elevated):
```powershell
.\aivpn-client.exe -k "aivpn://..."
```

> Administrator privileges are required to create the Wintun network adapter.

### Client — Android

1. Install `aivpn-client.apk`
2. Paste the connection key (`aivpn://…`)
3. Tap **Connect**

### Client — iOS

Build on macOS (Xcode 15+ required):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./scripts/build-ios.sh YOUR_TEAM_ID
```

Install `releases/aivpn-ios.ipa`:
```bash
xcrun devicectl device install app --device <UDID> releases/aivpn-ios.ipa
```

> A free Apple Developer account is sufficient. Sideloaded builds expire after 7 days.

### Client — Entware routers (ARMv7 / MIPSel)

```bash
# Copy the static musl binary to the router
scp aivpn-client-linux-armv7-musleabihf root@router:/opt/bin/aivpn-client
ssh root@router 'chmod +x /opt/bin/aivpn-client && /opt/bin/aivpn-client -k "aivpn://..."'
```

### Client — MikroTik RouterOS 7.6+

```routeros
/system/device-mode/update container=yes   # then reboot
/interface/veth/add name=veth-aivpn address=172.31.0.2/30 gateway=172.31.0.1
/ip/address/add address=172.31.0.1/30 interface=veth-aivpn
/container/mounts/add name=aivpn-tun src=/dev/net/tun dst=/dev/net/tun type=bind
/container/envs/add list=aivpn-env name=AIVPN_KEY value="aivpn://..."
/container/add remote-image=infosave2007/aivpn-mikrotik:latest \
    interface=veth-aivpn start-on-boot=yes envlist=aivpn-env mounts=aivpn-tun
/container/start [find remote-image~"aivpn-mikrotik"]
/ip/route/add dst-address=0.0.0.0/0 gateway=172.31.0.2
```

See [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md) for policy routing and troubleshooting.

### SOCKS5 proxy mode (no root)

```bash
aivpn-client -k "aivpn://..." --proxy-listen 127.0.0.1:1080
```

Configure Firefox / Chrome / curl to use `SOCKS5 127.0.0.1:1080`. No TUN device or administrator privileges required.

---

## Connection Key Format

Connection keys encode all server and client parameters in a single portable string:

```
aivpn://<base64url(JSON)>
```

JSON fields:

| Field | Type | Description |
|-------|------|-------------|
| `s` | `string` | Server address, e.g. `"1.2.3.4:443"` |
| `k` | `string` | Server X25519 public key (base64) |
| `p` | `string` | Client pre-shared key / PSK (base64) |
| `i` | `string` | Client static VPN IP, e.g. `"10.0.0.2"` |
| `n` | `object` | *(optional)* Bootstrap `network_config` (see below) |

`network_config` object (`n`):

| Field | Description |
|-------|-------------|
| `client_ip` | Client TUN IP |
| `server_vpn_ip` | Server TUN IP |
| `prefix_len` | Subnet prefix length |
| `mtu` | Inner MTU |

Priority order when connecting:

1. Settings confirmed by `ServerHello` (authoritative)
2. Bootstrap `network_config` from the key
3. Legacy fallback `10.0.0.0/24`

Keys without `network_config` remain fully supported.

Generate a key:
```bash
aivpn-server --add-client "Name" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

Reprint an existing key:
```bash
aivpn-server --show-client "Name" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

---

## Server Configuration Reference

Default config path: `config/server.json` (local) or `/etc/aivpn/server.json`. CLI flags override file values.

```json
{
  "listen_addr": "0.0.0.0:443",
  "tun_name": "aivpn0",
  "tun_mtu": "auto",
  "mask_dir": "/var/lib/aivpn/masks",
  "bootstrap_mask_files": [],
  "session_timeout_secs": 0,
  "idle_timeout_secs": 300,
  "allow_peer_routing": false,
  "network_config": {
    "server_vpn_ip": "10.0.0.1",
    "prefix_len": 24,
    "mtu": 1346,
    "keepalive_secs": 8,
    "ipv6_enabled": false,
    "ipv6_prefix": "fd10:cafe::/48"
  },
  "pool": {
    "peers": [],
    "sync_key": ""
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `listen_addr` | `0.0.0.0:443` | UDP bind address. Port is embedded in connection keys automatically |
| `tun_name` | random | TUN interface name |
| `tun_mtu` | _(unset)_ | `"auto"` = physical MTU minus 64-byte overhead (fallback 1346); or a fixed integer |
| `mask_dir` | `/var/lib/aivpn/masks` | Directory scanned for `.json` mask profiles |
| `bootstrap_mask_files` | `[]` | Mask files pre-loaded at startup to reduce first-connection latency |
| `session_timeout_secs` | `0` | Hard session cap; `0` = unlimited |
| `idle_timeout_secs` | `300` | Disconnect sessions silent for this many seconds |
| `allow_peer_routing` | `false` | Route packets between VPN clients inside the subnet |
| `network_config.server_vpn_ip` | `10.0.0.1` | Server TUN IP |
| `network_config.prefix_len` | `24` | VPN subnet prefix |
| `network_config.mtu` | `1346` | Inner MTU sent to clients in `ServerHello` |
| `network_config.keepalive_secs` | `8` | Keepalive interval negotiated with clients |
| `network_config.ipv6_enabled` | `false` | Enable IPv6 NAT66 |
| `network_config.ipv6_prefix` | `fd10:cafe::/48` | ULA /48 prefix for client IPv6 addresses |
| `pool.peers` | `[]` | Peer server addresses for database sync |
| `pool.sync_key` | `""` | Shared 32-byte BLAKE3 key (base64). Generate: `openssl rand -base64 32` |

### Optional features (Cargo)

| Feature | What it enables |
|---------|----------------|
| `neural` | Neural Resonance module (MSE-based mask rotation) |
| `management-api` | Unix socket HTTP API at `/run/aivpn/api.sock` |
| `metrics` | Prometheus exporter |
| `passive-distribution` | Bootstrap descriptor distribution channels |

```bash
cargo build --release --bin aivpn-server --features "management-api,metrics,neural"
```

---

## Build from Source

Requires: Rust 1.75+, `cargo`.

```bash
git clone https://github.com/infosave2007/aivpn.git
cd aivpn

# Build all workspace members
cargo build --release

# Individual binaries
cargo build --release --bin aivpn-server
cargo build --release --bin aivpn-client

# Run tests
cargo test

# Static musl cross-builds (ARMv7 / MIPSel)
./scripts/build-musl-release.sh server armv7-unknown-linux-musleabihf
./scripts/build-musl-release.sh client mipsel-unknown-linux-musl

# Docker server build (outputs to releases/)
./scripts/build-server-release.sh

# Windows GUI (cross-compile from Linux)
./scripts/build-windows-gui.sh

# iOS (macOS + Xcode 15+)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./scripts/build-ios.sh              # unsigned / simulator
./scripts/build-ios.sh YOUR_TEAM_ID # signed for device
```

### Android

```bash
export ANDROID_SDK_ROOT=/opt/android-sdk
export ANDROID_NDK_ROOT=/opt/android-ndk
echo "sdk.dir=$ANDROID_SDK_ROOT" > aivpn-android/local.properties

cd aivpn-android
./build-rust-android.sh release
```

Signed build: create `aivpn-android/keystore.properties` before running the script.

### Install from crates.io

```bash
cargo install aivpn-client
cargo install aivpn-server
```

---

## Advanced Features

### Device Binding (JIT enrollment)

A connection key can be designated as *one-time*: the first device to connect binds its X25519 static key, and subsequent connections from a different device are rejected.

```bash
# Create enrollment slot
aivpn-server --add-client-one-time "Alice-Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip IP:PORT

# Reset binding (re-enable enrollment)
aivpn-server --reset-device "Alice-Phone" \
    --clients-db /etc/aivpn/clients.json
```

Device key storage per platform:

| Platform | Location |
|----------|----------|
| Linux / macOS | `~/.config/aivpn/device.key` (mode 600, auto-generated) |
| Windows | `%APPDATA%\aivpn\device.key` |
| Android | Android Keystore via `EncryptedSharedPreferences` |
| iOS | Keychain, `kSecAttrAccessibleAfterFirstUnlock` |

### Connection Quality Score and Adaptive Mode

AIVPN continuously computes a **0–100 quality score** from RTT (40 pts), jitter (20 pts), packet loss (30 pts), and Neural MSE (10 pts). Adaptive Mode adjusts keepalive interval and FEC group size automatically:

| Score | Adaptive Level | Keepalive | FEC group |
|-------|---------------|-----------|-----------|
| 80–100 | Off | 8 s | disabled |
| 50–79 | Light | 6 s | 1/16 |
| 20–49 | Aggressive | 4 s | 1/8 |
| 0–19 | Satellite | 15 s | 1/4 |

Enable Adaptive Mode:
```bash
aivpn-client -k "aivpn://..." --adaptive
```

### Forward Error Correction (FEC)

Every N uplink data packets, one XOR repair packet is emitted. If exactly one packet from a group is lost, the server reconstructs it immediately — no retransmit round-trip. N is controlled by Adaptive Mode. FEC is disabled on clean links.

### Multi-server Pool Sync

Nodes in a pool share their client databases in real time over the standard VPN port:

```json
{
  "pool": {
    "peers": ["node2.example.com:443"],
    "sync_key": "<base64-32-byte-key>"
  }
}
```

### Multi-hop Chain Forwarding

Route client traffic through two AIVPN nodes. The client connects only to the entry node; the internet sees the exit node's IP.

**Entry node:**
```json
{ "pool": { "sync_key": "<key>", "exit_node": "exit.example.com:443" } }
```
**Exit node:**
```json
{ "pool": { "sync_key": "<same-key>", "exit_node_enabled": true } }
```

### Local DNS Proxy

Forward all DNS queries through the VPN tunnel (Linux):

```bash
aivpn-client -k "aivpn://..." --dns-proxy 127.0.0.1:5300 --dns-upstream 1.1.1.1:53
```

### Traffic Recording — Custom Mask Creation

Record real application traffic to generate new mimicry profiles:

```bash
# Connect with an admin key, then:
aivpn-client record start --service myapp
# ... use the application for 60+ seconds ...
aivpn-client record stop
```

The server analyzes packet size histograms and inter-arrival times, generates a `MaskProfile`, validates it via self-test, and distributes it to active sessions.

### Benchmarking

```bash
aivpn-client bench -k "aivpn://..."
# P50: 12ms  P95: 28ms  Up: 47 Mbps  Down: 52 Mbps  Score: 94/100
aivpn-client bench -k "aivpn://..." --json
```

---

## Security Model

| Property | Mechanism |
|----------|-----------|
| Encryption | ChaCha20-Poly1305 AEAD |
| Key exchange | X25519 ECDH |
| Session authentication | Per-client PSK (optional device binding) |
| Forward secrecy | In-flight X25519 key ratchet |
| Replay protection | 256-entry sliding window per session |
| Session anonymity | 8-byte BLAKE3-derived resonance tag; no session ID in the clear |
| Traffic mimicry | `MaskProfile` FSM: header injection, IAT shaping |
| Mask integrity | Neural Resonance MSE threshold (0.35); automatic rotation |
| NAT traversal | Server-side nftables/iptables, client-side `SO_REUSEPORT` |

Detailed adversary model and threat analysis: [THREAT_MODEL.md](THREAT_MODEL.md).

---

## Project Structure

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs          # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs            # Mimicry profiles (WebRTC, QUIC, DNS)
│   ├── protocol.rs        # Packet format and control plane
│   └── fec.rs             # XOR Forward Error Correction
├── aivpn-client/src/
│   ├── client.rs          # Core state machine
│   ├── tunnel.rs          # Cross-platform TUN interface
│   ├── kill_switch.rs     # Kill-switch (nftables / pfctl / netsh)
│   └── mimicry.rs         # Traffic shaping engine
├── aivpn-server/src/
│   ├── gateway.rs         # UDP gateway, session dispatch
│   ├── neural.rs          # Neural Resonance module
│   ├── nat.rs             # NAT forwarder (IPv4 + IPv6 NAT66)
│   ├── client_db.rs       # Client database
│   └── pool_sync.rs       # In-protocol pool synchronization
├── aivpn-android/         # Android Kotlin app
├── aivpn-ios/             # iOS SwiftUI app + NEPacketTunnelProvider
├── aivpn-windows/         # Windows egui GUI
├── aivpn-macos/           # macOS SwiftUI menu bar app
├── mask-assets/           # Bundled traffic mimicry profiles (JSON)
├── scripts/               # Build and deployment scripts
├── docker/                # Dockerfiles and entrypoint
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md
```

---

## License

MIT — see [LICENSE](LICENSE).
