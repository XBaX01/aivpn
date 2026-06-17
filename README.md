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
./build-ios.sh YOUR_TEAM_ID
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
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl

# Docker server build (outputs to releases/)
./build-server-release.sh

# Windows GUI (cross-compile from Linux)
./build-windows-gui.sh

# iOS (macOS + Xcode 15+)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./build-ios.sh              # unsigned / simulator
./build-ios.sh YOUR_TEAM_ID # signed for device
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
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md
```

---

## Contributing

Pull requests are welcome.

Entry points:

- Mask engine: [`aivpn-common/src/mask.rs`](aivpn-common/src/mask.rs)
- Neural module: [`aivpn-server/src/neural.rs`](aivpn-server/src/neural.rs)
- Cross-platform TUN: [`aivpn-client/src/tunnel.rs`](aivpn-client/src/tunnel.rs)
- Tests: `cargo test` (175+ tests)

---

## License

MIT — see [LICENSE](LICENSE).

---
---

# AIVPN (Русский)

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)

---

## Обзор

AIVPN — VPN-система на базе UDP, совмещающая шифрование туннеля с **мимикрией трафика**: исходящие пакеты маскируются под известные прикладные протоколы (WebRTC, QUIC, DNS-over-UDP), и соединение становится статистически неотличимым от обычного приложения для пассивного наблюдателя.

Ключевые технические характеристики:

- **Zero-RTT** — зашифрованный трафик может пойти с первого пакета, обязательного рукопожатия нет.
- **O(1) поиск сессий** — идентификатор сессии не передаётся в открытом виде. Каждый пакет несёт 8-байтовый *резонансный тег*, выведенный из временной метки и ключа сессии. Сервер находит сессию за константное время через `DashMap`.
- **Совершенная прямая секретность** — ротация ключей сессии по X25519 в режиме рэтчет. Компрометация ключа сервера не раскрывает прошлый трафик.
- **Модуль Neural Resonance** — micro-MLP (~66 КБ) на каждую маску следит за статистикой трафика; высокая ошибка реконструкции (MSE) запускает автоматическую ротацию маски без разрыва соединения клиента.
- **Написан на Rust** — нет GC, нет утечек памяти. Клиентский бинарник ≈ 2,5 МБ. Работает на VPS за $5.

---

## Архитектура

### Структура воркспейса

```
aivpn-common/       — общая крипто, протокол, маски (без I/O)
aivpn-server/       — VPN-шлюз и управляющий CLI (только Linux)
aivpn-client/       — кроссплатформенный клиент (Linux / macOS / Windows)
aivpn-android-core/ — JNI-мост для Android
aivpn-windows/      — Windows GUI (egui/eframe)
aivpn-android/      — Android-приложение на Kotlin
aivpn-macos/        — macOS SwiftUI в строке меню
aivpn-ios-core/     — iOS Rust staticlib (C FFI)
aivpn-ios/          — iOS SwiftUI + NEPacketTunnelProvider
mask-assets/        — встроенные профили мимикрии (JSON)
```

### Ключевые модули

| Модуль | Расположение | Назначение |
|--------|-------------|-----------|
| `crypto.rs` | `aivpn-common` | X25519, ChaCha20-Poly1305, BLAKE3/HMAC, генерация резонансных тегов |
| `protocol.rs` | `aivpn-common` | Wire-формат: `[8-byte tag][pad_len][inner_header][encrypted payload][poly1305 tag]` |
| `mask.rs` | `aivpn-common` | `MaskProfile` — шейпинг трафика: шаблоны заголовков, FSM, IAT-распределения |
| `gateway.rs` | `aivpn-server` | Центральный event loop: UDP-приём, диспетчер сессий, NAT, нейронные проверки |
| `session.rs` | `aivpn-server` | `SessionManager` — O(1) через `DashMap`, окно воспроизведения на 256 записей |
| `neural.rs` | `aivpn-server` | Neural Resonance: MLP 64→128→64 на маску, порог MSE 0,35, авто-ротация |
| `client.rs` | `aivpn-client` | Машина состояний: Unprovisioned → Connecting → Connected |
| `tunnel.rs` | `aivpn-client` | TUN: `/dev/net/tun` (Linux), `utun` (macOS), Wintun (Windows) |
| `mimicry.rs` | `aivpn-client` | `MimicryEngine` — применяет `MaskProfile` к исходящим пакетам |

### Синхронизация пула

Синхронизация клиентских баз между серверами пула использует `ControlPayload::PoolSync` внутри обычных VPN UDP-пакетов — неотличима от клиентского трафика. Отдельный TCP-порт и правило файрволла не нужны.

---

## Поддерживаемые платформы

| Платформа | Сервер | Клиент | GUI | TUN-драйвер |
|-----------|:------:|:------:|:---:|-------------|
| Linux | ✅ | ✅ | ✅ AppImage + трей | `/dev/net/tun` |
| macOS | — | ✅ | ✅ строка меню | `utun` |
| Windows | — | ✅ | ✅ egui | [Wintun](https://www.wintun.net/) |
| Android | — | ✅ | ✅ нативный Kotlin | `VpnService` API |
| iOS | — | ✅ | ✅ SwiftUI | `NetworkExtension` |
| MikroTik RouterOS 7.6+ | — | ✅ | — | контейнер veth + TUN |
| Entware-роутеры (ARMv7 / MIPSel) | — | ✅ | — | статический musl-бинарник |

---

## Быстрый старт

### Сервер (Linux)

#### Docker (рекомендуется)

```bash
mkdir -p config
docker compose up -d aivpn-server
```

Контейнер автоматически генерирует `server.key` и `server.json` при первом запуске. Работает в режиме `network_mode: host`, монтирует `./config` → `/etc/aivpn`.

Открыть UDP-порт 443:

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

Сервер автоматически включает переадресацию IPv4 и устанавливает NAT-правила (nftables при наличии, иначе iptables). Ручная настройка файрволла для туннеля не нужна.

#### Добавить клиента

```bash
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Bare metal
aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443
```

Вывод содержит ключ подключения (`aivpn://…`) — передать клиенту.

Другие команды управления: `--list-clients`, `--show-client`, `--remove-client`.

---

### Клиент — Linux

```bash
sudo ./aivpn-client -k "aivpn://..."
# Полный туннель (весь трафик через VPN)
sudo ./aivpn-client -k "aivpn://..." --full-tunnel
```

### Клиент — macOS

Скачать `aivpn-macos.dmg` из [Releases](https://github.com/infosave2007/aivpn/releases), перетащить **Aivpn.app** в Applications, запустить — появится в строке меню. Вставить ключ подключения и нажать **Connect**.

CLI:
```bash
sudo ./aivpn-client -k "aivpn://..."
```

> Приложение запрашивает пароль через `sudo` для создания интерфейса `utun`.

### Клиент — Windows

**Установщик (рекомендуется):** скачать `aivpn-windows-installer.exe`, запустить от имени Администратора, открыть **AIVPN** из меню Пуск.

**Portable:** извлечь `aivpn-windows-package.zip` (содержит `aivpn.exe`, `aivpn-client.exe`, `wintun.dll`). Запустить `aivpn.exe` от Администратора.

CLI (PowerShell, с правами Администратора):
```powershell
.\aivpn-client.exe -k "aivpn://..."
```

> Требуются права Администратора для создания сетевого адаптера Wintun.

### Клиент — Android

1. Установить `aivpn-client.apk`
2. Вставить ключ подключения (`aivpn://…`)
3. Нажать **Connect**

### Клиент — iOS

Сборка на macOS (требуется Xcode 15+):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./build-ios.sh ВАШ_TEAM_ID
```

Установка `releases/aivpn-ios.ipa`:
```bash
xcrun devicectl device install app --device <UDID> releases/aivpn-ios.ipa
```

> Достаточно бесплатного Apple Developer аккаунта. Сайдлоад-сборки истекают через 7 дней.

### Клиент — Entware-роутеры (ARMv7 / MIPSel)

```bash
scp aivpn-client-linux-armv7-musleabihf root@router:/opt/bin/aivpn-client
ssh root@router 'chmod +x /opt/bin/aivpn-client && /opt/bin/aivpn-client -k "aivpn://..."'
```

### Клиент — MikroTik RouterOS 7.6+

```routeros
/system/device-mode/update container=yes   # затем перезагрузка
/interface/veth/add name=veth-aivpn address=172.31.0.2/30 gateway=172.31.0.1
/ip/address/add address=172.31.0.1/30 interface=veth-aivpn
/container/mounts/add name=aivpn-tun src=/dev/net/tun dst=/dev/net/tun type=bind
/container/envs/add list=aivpn-env name=AIVPN_KEY value="aivpn://..."
/container/add remote-image=infosave2007/aivpn-mikrotik:latest \
    interface=veth-aivpn start-on-boot=yes envlist=aivpn-env mounts=aivpn-tun
/container/start [find remote-image~"aivpn-mikrotik"]
/ip/route/add dst-address=0.0.0.0/0 gateway=172.31.0.2
```

Подробнее: [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md).

### Режим SOCKS5-прокси (без root)

```bash
aivpn-client -k "aivpn://..." --proxy-listen 127.0.0.1:1080
```

Настроить Firefox / Chrome / curl на `SOCKS5 127.0.0.1:1080`. TUN-устройство и права Администратора не нужны.

---

## Формат ключа подключения

Ключ подключения кодирует все параметры сервера и клиента в одну строку:

```
aivpn://<base64url(JSON)>
```

Поля JSON:

| Поле | Тип | Описание |
|------|-----|---------|
| `s` | `string` | Адрес сервера, напр. `"1.2.3.4:443"` |
| `k` | `string` | Публичный ключ X25519 сервера (base64) |
| `p` | `string` | Предварительный общий ключ (PSK) клиента (base64) |
| `i` | `string` | Статический VPN-IP клиента, напр. `"10.0.0.2"` |
| `n` | `object` | *(необязательно)* Bootstrap `network_config` (см. ниже) |

Объект `network_config` (`n`):

| Поле | Описание |
|------|---------|
| `client_ip` | TUN-IP клиента |
| `server_vpn_ip` | TUN-IP сервера |
| `prefix_len` | Длина префикса подсети |
| `mtu` | Внутренний MTU |

Приоритет при подключении:

1. Параметры из `ServerHello` (авторитетный источник)
2. Bootstrap `network_config` из ключа
3. Устаревший фолбэк `10.0.0.0/24`

Ключи без `network_config` полностью поддерживаются.

Выпустить ключ:
```bash
aivpn-server --add-client "Имя" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

Повторно показать существующий ключ:
```bash
aivpn-server --show-client "Имя" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

---

## Справочник конфигурации сервера

Пути конфига: `config/server.json` (локально) или `/etc/aivpn/server.json`. CLI-флаги перекрывают значения файла.

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

| Параметр | По умолчанию | Описание |
|----------|-------------|---------|
| `listen_addr` | `0.0.0.0:443` | UDP-адрес. Порт автоматически встраивается в ключи подключения |
| `tun_name` | случайное | Имя TUN-интерфейса |
| `tun_mtu` | _(не задан)_ | `"auto"` = физический MTU минус 64 байта накладных расходов (фолбэк 1346); или целое число |
| `mask_dir` | `/var/lib/aivpn/masks` | Директория с `.json` профилями масок |
| `bootstrap_mask_files` | `[]` | Маски, предзагружаемые при старте |
| `session_timeout_secs` | `0` | Жёсткий лимит сессии; `0` = без лимита |
| `idle_timeout_secs` | `300` | Разрыв молчащих сессий (секунды) |
| `allow_peer_routing` | `false` | Маршрутизация пакетов между VPN-клиентами |
| `network_config.server_vpn_ip` | `10.0.0.1` | TUN-IP сервера |
| `network_config.prefix_len` | `24` | Префикс VPN-подсети |
| `network_config.mtu` | `1346` | Внутренний MTU, отправляемый клиентам в `ServerHello` |
| `network_config.keepalive_secs` | `8` | Интервал keepalive |
| `network_config.ipv6_enabled` | `false` | Включить IPv6 NAT66 |
| `network_config.ipv6_prefix` | `fd10:cafe::/48` | ULA /48 префикс для клиентских IPv6-адресов |
| `pool.peers` | `[]` | Адреса узлов пула для синхронизации БД |
| `pool.sync_key` | `""` | Общий 32-байтный ключ BLAKE3 (base64). Генерация: `openssl rand -base64 32` |

### Опциональные возможности (Cargo features)

| Feature | Что включает |
|---------|-------------|
| `neural` | Модуль Neural Resonance (ротация маски по MSE) |
| `management-api` | HTTP API на Unix-сокете `/run/aivpn/api.sock` |
| `metrics` | Экспортёр Prometheus |
| `passive-distribution` | Каналы распространения bootstrap-дескрипторов |

```bash
cargo build --release --bin aivpn-server --features "management-api,metrics,neural"
```

---

## Сборка из исходников

Требования: Rust 1.75+, `cargo`.

```bash
git clone https://github.com/infosave2007/aivpn.git
cd aivpn

# Все компоненты воркспейса
cargo build --release

# Отдельные бинарники
cargo build --release --bin aivpn-server
cargo build --release --bin aivpn-client

# Тесты
cargo test

# Статические musl-сборки (ARMv7 / MIPSel)
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl

# Docker-сборка сервера (результат в releases/)
./build-server-release.sh

# Windows GUI (кросс-компиляция с Linux)
./build-windows-gui.sh

# iOS (требуется macOS + Xcode 15+)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./build-ios.sh              # без подписи / симулятор
./build-ios.sh ВАШ_TEAM_ID  # с подписью для устройства
```

### Android

```bash
export ANDROID_SDK_ROOT=/opt/android-sdk
export ANDROID_NDK_ROOT=/opt/android-ndk
echo "sdk.dir=$ANDROID_SDK_ROOT" > aivpn-android/local.properties

cd aivpn-android
./build-rust-android.sh release
```

Подписанная сборка: создать `aivpn-android/keystore.properties` перед запуском скрипта.

### Установка из crates.io

```bash
cargo install aivpn-client
cargo install aivpn-server
```

---

## Расширенные возможности

### Привязка устройства (JIT-зачисление)

Ключ подключения может быть одноразовым: первое подключившееся устройство привязывает свой статический X25519-ключ, последующие подключения с другого устройства отклоняются.

```bash
# Создать слот зачисления
aivpn-server --add-client-one-time "Alice-Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip IP:PORT

# Сбросить привязку (повторное зачисление)
aivpn-server --reset-device "Alice-Phone" \
    --clients-db /etc/aivpn/clients.json
```

Хранение ключа устройства:

| Платформа | Путь |
|-----------|------|
| Linux / macOS | `~/.config/aivpn/device.key` (режим 600, автогенерация) |
| Windows | `%APPDATA%\aivpn\device.key` |
| Android | Android Keystore через `EncryptedSharedPreferences` |
| iOS | Keychain, `kSecAttrAccessibleAfterFirstUnlock` |

### Оценка качества соединения и адаптивный режим

AIVPN непрерывно вычисляет **оценку качества 0–100** из RTT (40 пт), джиттера (20 пт), потерь пакетов (30 пт) и Neural MSE (10 пт). Адаптивный режим автоматически регулирует keepalive и размер FEC-группы:

| Оценка | Уровень | Keepalive | FEC-группа |
|--------|---------|-----------|-----------|
| 80–100 | Выкл. | 8 с | выключено |
| 50–79 | Лёгкий | 6 с | 1/16 |
| 20–49 | Агрессивный | 4 с | 1/8 |
| 0–19 | Спутниковый | 15 с | 1/4 |

```bash
aivpn-client -k "aivpn://..." --adaptive
```

### Прямая коррекция ошибок (FEC)

Каждые N uplink-пакетов отправляется один XOR-ремонтный пакет. При потере ровно одного пакета из группы сервер восстанавливает его немедленно без повторной передачи. N управляется адаптивным режимом. На качественном канале FEC отключён.

### Синхронизация пула (multi-server)

```json
{
  "pool": {
    "peers": ["node2.example.com:443"],
    "sync_key": "<base64-32-byte-key>"
  }
}
```

### Многоузловая цепочка (multi-hop)

Клиент подключается только к входному узлу; интернет видит IP выходного узла.

**Входной узел:**
```json
{ "pool": { "sync_key": "<ключ>", "exit_node": "exit.example.com:443" } }
```
**Выходной узел:**
```json
{ "pool": { "sync_key": "<тот же ключ>", "exit_node_enabled": true } }
```

### Локальный DNS-прокси

```bash
aivpn-client -k "aivpn://..." --dns-proxy 127.0.0.1:5300 --dns-upstream 1.1.1.1:53
```

### Запись трафика — создание собственных масок

```bash
aivpn-client record start --service myapp
# ... работать с приложением 60+ секунд ...
aivpn-client record stop
```

Сервер анализирует гистограммы размеров пакетов и IAT, генерирует `MaskProfile`, валидирует через самотестирование и распространяет на активные сессии.

### Бенчмарк соединения

```bash
aivpn-client bench -k "aivpn://..."
# P50: 12ms  P95: 28ms  Up: 47 Mbps  Down: 52 Mbps  Score: 94/100
aivpn-client bench -k "aivpn://..." --json
```

---

## Модель безопасности

| Свойство | Механизм |
|----------|---------|
| Шифрование | ChaCha20-Poly1305 AEAD |
| Обмен ключами | X25519 ECDH |
| Аутентификация сессии | PSK на клиента (опционально — привязка устройства) |
| Прямая секретность | X25519 рэтчет в полёте |
| Защита от повтора | Скользящее окно на 256 записей на сессию |
| Анонимность сессии | 8-байтовый BLAKE3-тег; идентификатор сессии не передаётся |
| Мимикрия трафика | FSM `MaskProfile`: инъекция заголовков, IAT-шейпинг |
| Целостность маски | Neural Resonance MSE 0,35; авто-ротация |
| NAT | Сервер: nftables/iptables; клиент: `SO_REUSEPORT` |

Подробная модель угроз и анализ: [THREAT_MODEL.md](THREAT_MODEL.md).

---

## Структура проекта

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs          # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs            # Профили мимикрии (WebRTC, QUIC, DNS)
│   ├── protocol.rs        # Формат пакетов и управляющий протокол
│   └── fec.rs             # XOR Forward Error Correction
├── aivpn-client/src/
│   ├── client.rs          # Ядро машины состояний
│   ├── tunnel.rs          # Кроссплатформенный TUN
│   ├── kill_switch.rs     # Kill-switch (nftables / pfctl / netsh)
│   └── mimicry.rs         # Движок шейпинга трафика
├── aivpn-server/src/
│   ├── gateway.rs         # UDP-шлюз, диспетчер сессий
│   ├── neural.rs          # Модуль Neural Resonance
│   ├── nat.rs             # NAT (IPv4 + IPv6 NAT66)
│   ├── client_db.rs       # База клиентов
│   └── pool_sync.rs       # Внутрипротокольная синхронизация пула
├── aivpn-android/         # Android Kotlin-приложение
├── aivpn-ios/             # iOS SwiftUI + NEPacketTunnelProvider
├── aivpn-windows/         # Windows egui GUI
├── aivpn-macos/           # macOS SwiftUI в строке меню
├── mask-assets/           # Встроенные профили мимикрии (JSON)
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md
```

---

## Участие в разработке

Pull request'ы приветствуются.

Точки входа:

- Движок масок: [`aivpn-common/src/mask.rs`](aivpn-common/src/mask.rs)
- Нейронный модуль: [`aivpn-server/src/neural.rs`](aivpn-server/src/neural.rs)
- Кроссплатформенный TUN: [`aivpn-client/src/tunnel.rs`](aivpn-client/src/tunnel.rs)
- Тесты: `cargo test` (175+ тестов)

---

## Лицензия

MIT — см. [LICENSE](LICENSE).
