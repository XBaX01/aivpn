# AIVPN

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)

Traditional VPNs are dead. ISPs and state-level firewalls (like GFW) detect WireGuard and OpenVPN in milliseconds just by looking at packet sizes, timing intervals, and handshake patterns. You can encrypt your payload with whatever cipher you want — DPI systems don't care about the content, they block the *shape* of the connection itself.

**AIVPN** is my answer to modern deep packet inspection. We don't just encrypt packets — we disguise them as real application traffic. Your ISP sees a Zoom call or TikTok scrolling, when in reality it's a fully encrypted tunnel.

To validate this in practice, I built my own DPI emulator, reproduced real filtering scenarios, and intentionally blocked traffic across different modes. I then stress-tested the system under heavy load to measure resilience, mask-switching speed, and routing stability. For fast routing, I implemented my patented approach: USPTO (USA) application No. 19/452,440 dated Jan 19, 2026 — *SYSTEM AND METHOD FOR UNSUPERVISED MULTI-TASK ROUTING VIA SIGNAL RECONSTRUCTION RESONANCE*.

## Supported Platforms

| Platform | Server | Client | Full Tunnel | Notes |
|----------|--------|--------|-------------|-------|
| **Linux** | ✅ | ✅ | ✅ | Primary platform, TUN via `/dev/net/tun`; GUI app (AppImage + tray) |
| **macOS** | — | ✅ | ✅ | Via `utun` kernel interface, auto route config |
| **Windows** | — | ✅ | ✅ | Via [Wintun](https://www.wintun.net/) driver |
| **Android** | — | ✅ | ✅ | Native Kotlin app via `VpnService` API |
| **iOS** | — | ✅ | ✅ | Native SwiftUI app via `NetworkExtension` API |
| **MikroTik RouterOS** | — | ✅ | ✅ | RouterOS 7.6+ container, arm64/armv7/amd64 |

### Current Client Status

- ✅ macOS app: working
- ✅ CLI client: working
- ✅ Android app: working
- ✅ iOS app: working (build requires macOS + Xcode 15+)
- ✅ Windows client: working (GUI + CLI)
- ✅ MikroTik RouterOS container: working (arm64/armv7/amd64)

## 📥 Downloads

Pre-built binaries for all supported platforms are automatically built and attached to each release. You can download the latest versions from the [GitHub Releases](https://github.com/infosave2007/aivpn/releases) page.


### Quick Start (macOS)
1. Download `aivpn-macos.dmg` from the [Releases](https://github.com/infosave2007/aivpn/releases) page and open it
2. Drag **Aivpn.app** to Applications
3. Launch — the app appears in the menu bar (no dock icon)
4. Paste your connection key (`aivpn://...`) and click **Connect**
5. Toggle 🇷🇺/🇬🇧 to switch language
> ⚠️ The VPN client requires root privileges for TUN device. The app will prompt for password via `sudo`.

### Quick Start (Windows)

#### Option A: Installer (recommended)
1. Download [aivpn-windows-installer.exe](https://github.com/infosave2007/aivpn/releases)
2. Right-click → **Run as Administrator** and follow the installer
3. Launch **AIVPN** from the Start Menu (runs as Administrator automatically)
4. Paste your connection key (`aivpn://...`) and click **Connect**

> ⚠️ The VPN client requires Administrator privileges to create the Wintun network adapter. Always run as Administrator.

#### Option B: Portable archive
1. Download and extract [aivpn-windows-package.zip](https://github.com/infosave2007/aivpn/releases)
2. Ensure `aivpn.exe`, `aivpn-client.exe`, and `wintun.dll` remain in the same folder
3. Right-click `aivpn.exe` → **Run as Administrator** for the GUI, or use CLI:
   ```powershell
   .\aivpn-client.exe -k "your_connection_key_here"
   ```

### Quick Start (Linux)
1. Download [aivpn-client-linux-x86_64](https://github.com/infosave2007/aivpn/releases)
2. Make it executable and run as root:
    ```bash
    chmod +x ./aivpn-client-linux-x86_64
    sudo ./aivpn-client-linux-x86_64 -k "your_connection_key_here"
    ```

### Quick Start (Entware Routers)
1. Download `aivpn-client-linux-mipsel-musl` or `aivpn-client-linux-armv7-musleabihf` from the [Releases](https://github.com/infosave2007/aivpn/releases) page.
2. Copy the binary to the router, for example into `/opt/bin/aivpn-client`.
3. Make it executable and run it from Entware shell as root:
    ```sh
    chmod +x /opt/bin/aivpn-client
    /opt/bin/aivpn-client -k "your_connection_key_here"
    ```
4. Because these musl builds are statically linked, no Rust toolchain or extra shared libraries are required on the router.

### Quick Start (MikroTik RouterOS)
1. Enable containers: `/system/device-mode/update container=yes` and reboot
2. Run the setup commands (see [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md)):
   ```routeros
   /interface/veth/add name=veth-aivpn address=172.31.0.2/30 gateway=172.31.0.1
   /ip/address/add address=172.31.0.1/30 interface=veth-aivpn
   /container/mounts/add name=aivpn-tun src=/dev/net/tun dst=/dev/net/tun type=bind
   /container/envs/add list=aivpn-env name=AIVPN_KEY value="aivpn://..."
   /container/add remote-image=infosave2007/aivpn-mikrotik:latest interface=veth-aivpn start-on-boot=yes envlist=aivpn-env mounts=aivpn-tun
   /container/start [find remote-image~"aivpn-mikrotik"]
   ```
3. Add a default route through the container: `/ip/route/add dst-address=0.0.0.0/0 gateway=172.31.0.2`

See [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md) for full documentation including policy routing and troubleshooting.

### Quick Start (Android)
1. Download and install `aivpn-client.apk`
2. Paste your connection key (`aivpn://...`) into the app
3. Tap **Connect**

### Quick Start (iOS)
1. Build on macOS (requires Xcode 15+, `xcodegen`):
   ```bash
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
   cargo install xcodegen
   ./build-ios.sh YOUR_TEAM_ID
   ```
2. Install `releases/aivpn-ios.ipa` on device:
   - Drag into **Xcode → Window → Devices and Simulators**, or
   - `xcrun devicectl device install app --device <UDID> releases/aivpn-ios.ipa`
3. Open the app, paste your connection key (`aivpn://...`) and tap **Connect**

> A free Apple ID (personal team) is sufficient — no paid Developer Program required. Device installs expire after 7 days and must be rebuilt to renew.

### Android Release Signing

For a production-signed Android APK, create `aivpn-android/keystore.properties`:

```properties
storeFile=/absolute/path/to/aivpn-release.jks
storePassword=your-store-password
keyAlias=aivpn
keyPassword=your-key-password
```

Then build with Java 21:

```bash
cd aivpn-android
export JAVA_HOME="$(/usr/libexec/java_home -v 21)"
export PATH="$JAVA_HOME/bin:$PATH"
./build-rust-android.sh release
```

If `keystore.properties` is absent, the script falls back to an unsigned release APK and then signs it with the debug keystore only as a local installable fallback.

### 📦 Install via Cargo (crates.io)

If you have Rust installed, you can easily install the client or server binaries directly from crates.io:

```bash
cargo install aivpn-client
cargo install aivpn-server
```

## ❤️ Support the Project

If you find this project helpful, you can support its development with a donation via Tribute:

👉 https://t.me/tribute/app?startapp=dzX1

Every donation helps keep AIVPN evolving. Thank you! 🙌

## The Main Feature: Neural Resonance (AI)

The most interesting thing under the hood is our AI module called **Neural Resonance**.
We didn't drag a 400 MB LLM into the project that would eat all the RAM on a cheap VPS. Instead:

- **Baked Mask Encoder:** For each mask profile (WebRTC codec, QUIC protocol) we deterministically derive a micro neural network (MLP 64→128→64) directly from the mask's 64-float signature vector — seeded by a BLAKE3 hash of that signature. Structurally unique per mask, ~66 KB, no external training files needed.
- **Real-time analysis:** This neural net analyzes entropy and IAT (inter-arrival times) of incoming UDP packets on the fly.
- **Hunting censors:** If the ISP's DPI system tries to probe our server (Active Probing) or starts throttling packets, the neural module detects a spike in reconstruction error (MSE).
- **Auto mask rotation:** As soon as the AI determines the current mask is compromised (e.g. `webrtc_zoom` got flagged), the server and client *seamlessly* reshape traffic to a backup mask (e.g. `dns_over_udp`). Zero disconnects!

## Other Cool Stuff

- **Zero-RTT & PFS:** No classic handshake for sniffers to catch. Data flows from the very first packet. And Perfect Forward Secrecy is built in — keys rotate on the fly, so even if the server gets seized, old traffic dumps can't be decrypted.
- **O(1) cryptographic session tags:** We never transmit a session ID in the clear. Instead, every packet carries a dynamic cryptographic tag derived from a timestamp and a secret key. The server finds the right client instantly, but to any observer it's just noise.
- **Written in Rust:** Fast, memory-safe, no leaks. The entire client binary is ~2.5 MB. Runs comfortably on a $5 VPS.

## Getting Started

### 1. Clone the repo

```bash
git clone https://github.com/infosave2007/aivpn.git
cd aivpn
```

### 2. Build (requires Rust 1.75+)

The project is split into workspaces: `aivpn-common` (crypto & masks), `aivpn-server`, and `aivpn-client`.

```bash
# Same command on all platforms:
cargo build --release
```

To refresh the Linux server release artifact without installing Rust on the host:

```bash
./build-server-release.sh
```

For static musl builds for ARMv7 servers and Entware-class MIPSel routers:

```bash
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
```

For the iOS app (macOS + Xcode 15+ required):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./build-ios.sh              # unsigned (CI / simulator)
./build-ios.sh YOUR_TEAM_ID # signed for real device (free Apple ID)
```

The `.ipa` is copied to `releases/aivpn-ios.ipa`.

To deploy the latest published Linux server release to a VPS in one command:

```bash
./deploy-server-release.sh
```

> For GitHub Releases, publish `aivpn-server-linux-x86_64` as the default Linux server asset, keep `aivpn-windows-package.zip` as the primary Windows asset, and attach the musl artifacts `aivpn-server-linux-armv7-musleabihf`, `aivpn-server-linux-mipsel-musl`, `aivpn-client-linux-armv7-musleabihf`, and `aivpn-client-linux-mipsel-musl` for ARM/Entware targets. Raw `aivpn-client.exe` is only safe when `wintun.dll` is shipped next to it.

GitHub Releases automation: the workflow in `.github/workflows/server-release-asset.yml` builds `aivpn-server-linux-x86_64` plus the ARMv7 and MIPSel musl server/client assets on each published Release and uploads them automatically.

### 3. Server (Linux only)

#### Option A: Docker (recommended)

The easiest way — everything is preconfigured in `docker-compose.yml`.

```bash
# Pick the Compose command available on your system
if docker compose version >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker-compose"
else
    echo "Install Docker Compose v2 (`docker-compose-v2` or `docker-compose-plugin`) or legacy `docker-compose`."
    exit 1
fi

# Optional: pre-create config/server.json or config/server.key here.
# If they are missing, the container now bootstraps both automatically.
mkdir -p config

# Fast start from the prebuilt Linux release binary
AIVPN_SERVER_DOCKERFILE=Dockerfile.prebuilt $AIVPN_COMPOSE up -d aivpn-server

# Or keep the original source build path
$AIVPN_COMPOSE up -d aivpn-server
```

The fast path expects `releases/aivpn-server-linux-x86_64` to be present locally. Build it with `./build-server-release.sh` or download it from Releases before starting Docker.

For a VPS one-command fast deploy, run `./deploy-server-release.sh`. It downloads the release asset, creates `config/server.key` if needed, and starts Docker with `Dockerfile.prebuilt`. The server manages IPv4 forwarding and NAT automatically on startup.

If your firewall is enabled, also allow `443/udp` using the tool your system uses:

```bash
# UFW (Ubuntu/Debian)
sudo ufw allow 443/udp

# firewalld (RHEL/CentOS/Fedora)
sudo firewall-cmd --add-port=443/udp --permanent
sudo firewall-cmd --reload
```

> The container runs with `network_mode: "host"` and mounts `./config` → `/etc/aivpn` inside the container.
> On first start it auto-creates `server.json` from the bundled example and generates `server.key` if either file is missing.

#### Option B: Bare metal

SSH into your VPS, generate a key:

```bash
sudo mkdir -p /etc/aivpn
openssl rand 32 | sudo tee /etc/aivpn/server.key > /dev/null
sudo chmod 600 /etc/aivpn/server.key
```

Start it up:

```bash
sudo ./target/release/aivpn-server --listen 0.0.0.0:443 --key-file /etc/aivpn/server.key
```

> The server automatically enables IPv4 forwarding and installs NAT masquerade rules on startup (using nftables if available, otherwise iptables). No manual `iptables` configuration is required.

If you use a VPN subnet other than the legacy `10.0.0.0/24`, keep it in `config/server.json` as the authoritative source:

```json
{
    "listen_addr": "0.0.0.0:443",
    "tun_name": "aivpn0",
    "network_config": {
        "server_vpn_ip": "10.150.0.1",
        "prefix_len": 24,
        "mtu": 1346
    }
}
```

The server reads `network_config` from `server.json` and automatically installs the NAT rule for the correct subnet on startup.

`listen_addr` controls the port (default: 443). To use a different port:

```json
{
  "listen_addr": "0.0.0.0:8443",
  ...
}
```

The port is automatically embedded in connection keys — clients don't need manual configuration. The `AIVPN_LISTEN` environment variable or `--listen` CLI flag override `server.json`.

### 3.1 Client Management

AIVPN uses a client registration model similar to WireGuard/XRay: each client gets a unique PSK, a static VPN IP, and traffic statistics.

All config is packed into a single **connection key** — one string that the user pastes into the app or CLI client.

The connection key now carries both the legacy top-level VPN IP field and an optional bootstrap `network_config` block. New clients use server-provided network settings from this block, then confirm them from `ServerHello`. Older keys without `network_config` still work.

#### Docker

```bash
# Reuse the same Compose command detected above
# Add a new client (prints a connection key)
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# Output:
# ✅ Client 'Alice Phone' created!
#    ID:     a1b2c3d4e5f67890
#    VPN IP: 10.0.0.2
#
# ══ Connection Key (paste into app) ══
#
# aivpn://eyJpIjoiMTAuMC4wLjIiLCJrIjoiLi4uIiwibiI6eyJjbGllbnRfaXAiOiIxMC4wLjAuMiIsInNlcnZlcl92cG5faXAiOiIxMC4wLjAuMSIsInByZWZpeF9sZW4iOjI0LCJtdHUiOjEzNDZ9LCJwIjoiLi4uIiwicyI6IjEuMi4zLjQ6NDQzIn0

# List all clients with traffic stats
docker compose exec aivpn-server aivpn-server \
    --list-clients --clients-db /etc/aivpn/clients.json

# Show a specific client (and its connection key)
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --show-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# Remove a client
docker compose exec aivpn-server aivpn-server \
    --remove-client "Alice Phone" \
    --clients-db /etc/aivpn/clients.json
```

> Uses the Compose service name, so it works regardless of the generated container name.

#### Bare metal

```bash
# Add a new client
aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# List all clients with traffic stats
aivpn-server --list-clients --clients-db /etc/aivpn/clients.json

# Show a specific client (and its connection key)
aivpn-server \
    --show-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# Remove a client
aivpn-server \
    --remove-client "Alice Phone" \
    --clients-db /etc/aivpn/clients.json
```

### 3.2 Recording Custom Masks

AIVPN supports automatic traffic recording from real applications to create new mimicry profiles. This allows adapting the system to specific services that aren't blocked in your network.

#### How Recording Works

The recording system works through an **authenticated client connection**:

1. **Create admin client**: Generate a special admin key on the server
2. **Connect client**: Start the AIVPN client with the admin connection key
3. **Start recording**: Send `record start <service>` command through the VPN tunnel
4. **Use the service**: The system captures packet metadata (sizes, intervals, headers)
5. **Stop recording**: Send `record stop` to trigger mask generation and self-testing

The server-side pipeline:
- **Record**: Intercepts UDP packets from the VPN session
- **Analyze**: Builds size histogram, computes IAT periods, infers FSM
- **Generate**: Creates a full `MaskProfile` with `HeaderSpec`
- **Self-test**: Validates statistical reproduction
- **Store**: Saves to mask storage and registers in catalog

#### Step-by-Step Guide

**1. Create an admin client on the server:**

```bash
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_SERVER_IP:443

# Bare metal
aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_SERVER_IP:443
```

Save the output connection key (starts with `aivpn://`).

**2. Connect the client with admin key:**

```bash
sudo ./target/release/aivpn-client -k "aivpn://..."
```

**3. Start recording for a service:**

```bash
# Send record start command through the VPN tunnel
aivpn-client record start --service zoom
```

**4. Use the service normally** for 30-60 seconds to capture diverse traffic patterns.

**5. Stop recording:**

```bash
aivpn-client record stop
```

The server analyzes the captured packets and generates a new mask. Progress appears in server logs:

```
INFO Analysis complete for 'zoom': 1243 packets, up=621 down=622, confidence=0.87
INFO Self-test passed for 'zoom': up(size=0.923,iat=0.891) down(size=0.908,iat=0.876) header=0.94 fsm=0.89, confidence=0.87
INFO Mask 'zoom_custom_abc123' stored and broadcast to active sessions
```

#### Requirements for Good Masks

- **At least 500 packets** for statistical significance
- **Minimum 60 seconds** of recording (system requirement)
- **Diverse traffic**: different operation types in the service
- **Stable connection**: no disconnects or retransmissions

Each mask is a separate JSON file named `{mask_id}.json`.

### 4. Client

#### Connection Key (recommended)

The easiest way — paste the connection key from `--add-client`:

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..."
```

Priority on modern clients is:

1. Network settings confirmed by `ServerHello`
2. Bootstrap `network_config` from the connection key
3. Legacy fallback `10.0.0.0/24`

Migration note: old clients continue to work with old keys and legacy `/24` defaults, but if you move the server to a different subnet or prefix, clients must be updated and connection keys should be reissued.

Full tunnel:

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..." --full-tunnel
```

#### Manual mode

You can also specify the server address and key manually (without PSK — for legacy/no-auth mode):

#### Linux

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64
```

Full tunnel mode (route all traffic through VPN):

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64 \
    --full-tunnel
```

#### macOS

Same deal, `cargo build --release` produces a native binary:

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64
```

> macOS will auto-configure the `utun` interface and routes via `ifconfig` / `route`.

#### Windows

Preferred for users: install via [aivpn-windows-installer.exe](https://github.com/infosave2007/aivpn/releases) (includes GUI app, CLI client, and Wintun driver).

Alternatively, download and extract [aivpn-windows-package.zip](https://github.com/infosave2007/aivpn/releases). The archive contains:

```
aivpn.exe          # GUI application
aivpn-client.exe   # CLI client
wintun.dll         # Wintun network driver
```

> ⚠️ **Administrator privileges required.** The VPN client needs Administrator rights to create the Wintun network adapter. Always right-click → "Run as Administrator" or launch from an elevated PowerShell.

**GUI mode** (recommended): right-click `aivpn.exe` → **Run as Administrator**, paste your connection key and click Connect.

**CLI mode** from PowerShell **as Administrator**:

```powershell
.\aivpn-client.exe --server YOUR_VPS_IP:443 --server-key SERVER_PUBLIC_KEY_BASE64
```

Full tunnel:

```powershell
.\aivpn-client.exe --server YOUR_VPS_IP:443 --server-key SERVER_PUBLIC_KEY_BASE64 --full-tunnel
```

> The client auto-configures routes via `route add` and cleans them up on exit.

### 4.1 Proxy Mode (SOCKS5, no root required)

Instead of a TUN device, the client can run as a local **SOCKS5 proxy**. This lets you route a specific browser or application through the VPN without administrator/root privileges and without any kernel driver.

```bash
# Start the SOCKS5 proxy on port 1080 (no sudo needed)
aivpn-client -k "aivpn://eyJp..." --proxy-listen 127.0.0.1:1080
```

Configure your application to use `SOCKS5` at `127.0.0.1:1080`:

| Application | How to configure |
|-------------|-----------------|
| **Firefox** | Settings → Network Settings → Manual proxy → SOCKS5 `127.0.0.1:1080`, enable "Proxy DNS" |
| **Chrome / Chromium** | Launch with `--proxy-server=socks5://127.0.0.1:1080` |
| **curl** | `curl --proxy socks5h://127.0.0.1:1080 https://example.com` |
| **git** | `git config --global http.proxy socks5h://127.0.0.1:1080` |

**Limitations:**
- IPv6 target addresses are not supported (use hostnames or IPv4)
- UDP traffic is not proxied (TCP CONNECT only)
- DNS is resolved locally via system resolver (queries bypass the VPN)

### 5. Android

1. Install the APK (`aivpn-android/app/build/outputs/apk/debug/app-debug.apk`)
2. Paste your **connection key** (`aivpn://...`) into the single input field
3. Tap **Connect**

The connection key contains everything: server address, public key, your PSK, and VPN IP. No manual configuration needed.

## Cross-compilation

Build the client for any platform from your current machine:

```bash
# Linux target from macOS/Windows
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# Windows target from Linux/macOS
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
```

For static musl cross-builds without installing a local cross toolchain, use Docker-backed release builds:

```bash
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
```

These artifacts are intended for ARM Linux servers/SBCs and Entware-capable MIPSel routers.

For Entware routers, the usual flow is: build or download the musl artifact, copy it into `/opt/bin`, `chmod +x`, and run it directly from the router shell.

## What's New in v0.8.0

### Multi-server Pool Sync (in-protocol)

Run AIVPN as a pool of nodes that automatically share their client databases. Sync is carried **inside the existing VPN protocol** as a `PoolSync` control message — indistinguishable from regular client traffic. No extra TCP port, no extra firewall rule.

`server.json`:
```json
{
  "pool": {
    "peers": ["node2.example.com:443", "node3.example.com:443"],
    "sync_key": "<base64-encoded 32-byte key>"
  }
}
```
Generate a key: `openssl rand -base64 32`

### Backup / Migration

```bash
# Export (clients DB, masks, server config)
aivpn-server --export /tmp/aivpn-backup.tar.gz

# Dry-run preview, then restore
aivpn-server --import /tmp/aivpn-backup.tar.gz --dry-run
aivpn-server --import /tmp/aivpn-backup.tar.gz --target-dir /etc/aivpn
```

### Per-client QoS

```bash
aivpn-server --set-client-qos "Alice" --bw-up 10M --bw-down 50M --dscp EF
```

Enforced via eBPF TC when available, automatic userspace token-bucket fallback otherwise.

### Benchmarking & Diagnostics

```bash
aivpn-client bench -k "aivpn://..."
# P50: 12ms  P95: 28ms  Up: 47 Mbps  Down: 52 Mbps  Score: 94/100
```

Available from CLI and from the diagnostics panel in all GUI clients (Windows, macOS, iOS, Android).

### Adaptive Mode

Automatic MTU and keepalive tuning based on live per-connection packet-loss measurement:

```bash
aivpn-client -k "aivpn://..." --adaptive
```

### OpenWRT / LuCI

Native OpenWRT package with procd init script, UCI config, and a LuCI web UI. See `aivpn-openwrt/docs/openwrt-setup.md`.

### Admin Audit Log

Every management operation logged to `/var/log/aivpn/audit.log` (JSONL, configurable via `--audit-log`) with actor, action, target, result, and ISO-8601 timestamp.

---

## Project Structure

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs          # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs            # Mimicry profiles (WebRTC, QUIC, DNS)
│   ├── protocol.rs        # Packet format, inner types
│   └── kernel_accel.rs    # /dev/aivpn ioctl API + XDP attach helpers
├── aivpn-client/src/
│   ├── client.rs          # Core client logic (split-tunnel, kill-switch, XDP)
│   ├── tunnel.rs          # TUN interface (Linux / macOS / Windows)
│   ├── kill_switch.rs     # Kill-switch (nftables/pfctl/netsh)
│   └── mimicry.rs         # Traffic shaping engine
├── aivpn-server/src/
│   ├── gateway.rs         # UDP gateway, MaskCatalog, resonance loop
│   ├── neural.rs          # Baked Mask Encoder, AnomalyDetector
│   ├── nat.rs             # NAT forwarder (IPv4 + IPv6 NAT66)
│   ├── client_db.rs       # Client database (PSK, static IP, stats)
│   ├── key_rotation.rs    # Session key rotation
│   └── metrics.rs         # Prometheus monitoring
├── aivpn-common/mask-assets/   # 11 traffic mimicry profiles (JSON)
├── aivpn-linux/           # Linux Iced GUI (AppImage + system tray)
├── aivpn-linux-kernel/    # Optional kernel module (aivpn.ko) + XDP filter
├── aivpn-android/         # Android client (Kotlin)
├── aivpn-ios-core/        # iOS Rust staticlib (C FFI)
├── aivpn-ios/             # iOS SwiftUI app + NEPacketTunnelProvider
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md        # Protocol security & adversary model
```

## Contributing

Want to dig into the code or train your own mask for the neural module? Jump in:

- Mask engine: [`aivpn-common/src/mask.rs`](aivpn-common/src/mask.rs)
- Neural weights & anomaly detector: [`aivpn-server/src/neural.rs`](aivpn-server/src/neural.rs)
- Cross-platform TUN module: [`aivpn-client/src/tunnel.rs`](aivpn-client/src/tunnel.rs)
- Tests (100+): `cargo test`

PRs are welcome! We're especially looking for people with traffic analysis experience to capture dumps from popular apps and train new profiles for Neural Resonance.

---

License — MIT. Use it, fork it, bypass censorship responsibly.
