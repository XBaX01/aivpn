# Changelog

## [0.7.0] - 2026-06-13

### Added
- **Advanced Split-Tunneling**: `--include-routes` and `--exclude-routes` CLI flags for fine-grained per-CIDR routing control on Linux, macOS, and Windows. Routes are automatically cleaned up on disconnect.
- **Kill-Switch + Leak Protection**: `--kill-switch` flag installs firewall rules (nftables on Linux, pfctl on macOS, Windows Firewall on Windows) that block all non-VPN traffic. Rules survive unexpected process termination and persist until explicitly cleared with `kill-switch clear`.
- **IPv6 Dual-Stack**: Full NAT66/NPTv6 support on the server (`aivpn-server`). New `ipv6_enabled` and `ipv6_prefix` fields in `VpnNetworkConfig`; clients receive an IPv6 address in `ServerHello`.
- **MTU Auto-Detection**: `mtu: "auto"` in server config triggers PMTUD-based MTU discovery, replacing hardcoded 1400-byte defaults.
- **Mask Validator**: `--validate-mask <path>` server subcommand validates a mask JSON file — checks structure, confidence score, FSM reachability, and IAT distribution consistency.
- **Six New DPI-Evasion Masks**: `avito`, `sber`, `vk`, `sberjazz`, `whatsapp`, and `yandex` traffic profiles added to `mask-assets/`. Each has confidence score ≥ 0.90.
- **Neural Anti-Probing Improvements**: Neural Resonance Module now tracks 64 traffic features including burst pattern, packet direction ratio, IAT periodicity, and entropy variance. Rotation cooldown of 60 s prevents oscillation under sustained active probing.
- **Linux Desktop GUI**: Native Linux application (`aivpn-linux`) built with Iced framework, distributed as AppImage with system tray integration.
- **eBPF/XDP Early Packet Filter**: `aivpn-linux-kernel` module now compiles an XDP BPF program (`xdp_prog.o`). When present, it attaches to the default-route NIC at connect time and drops malformed or replayed UDP packets at NIC level before socket buffer allocation. Configuration is pinned at `/sys/fs/bpf/aivpn/xdp_config`.
- **Threat Model Document**: Added `THREAT_MODEL.md` covering adversary model, cryptographic design, traffic-analysis resistance, kill-switch guarantees, XDP properties, and known limitations.

### Changed
- **`record_traffic` API**: Added `is_rx: bool` parameter for directional traffic analysis (upload vs. download distinction in neural feature extraction).
- **Rust Workspace version**: Bumped to 0.7.0.
- **macOS build**: CFBundleVersion bumped to 5.
- **iOS build**: CFBundleVersion bumped to 3.

### Fixed
- **`resolve_config_path` crash**: Server no longer calls `process::exit(1)` when `/etc/aivpn/server.json` exists but is not readable (e.g. root-owned). Auto-discovery now uses `File::open().is_ok()` instead of `path.exists()`.
- **Test fixture API alignment**: Updated `VpnNetworkConfig`, `ClientNetworkConfig`, and `ServerArgs` test literals in `client_db.rs`, `management_api_tests.rs`, and `main.rs` to match 0.7.0 struct fields.

## [0.6.0] - 2026-06-12

### Added
- **SOCKS5 Proxy Mode**: Introduced userspace proxy mode using `smoltcp` stack in `aivpn-client` and added proxy option to the Windows GUI.
- **Linux Kernel Module (`aivpn.ko`)**: Implemented a kernel-space accelerator module with user-space integration via `KernelAccel` for high-performance packet routing.
- **MikroTik RouterOS 7 support**: Created `aivpn-mikrotik` Docker build environment and detailed guides for RouterOS deployments.

### Fixed
- **Security Audit Corrections**: Resolved deep audit findings in `aivpn-server` (forward_packet boundaries, rate limit pruning, is_expired tracking) and kernel crypto operations.
- **macOS VPN Routing**: Fixed full-tunnel cleanup and route subnet commands on disconnect/reconnect.
- **Build Pipeline**: Resolved iOS cross-compilation target file copying bugs in `musl` build scripts.
- **Rust Workspace version**: Bumped to 0.6.0.

## [0.5.0] - 2026-06-11

### Added
- **iOS Client application**: Native Swift application with a Network Extension (`PacketTunnelProvider`) and integrated Rust core (`aivpn-ios-core`).
- **Android Quick Settings tile**: One-tap quick settings tile for toggling the VPN connection easily.
- **ED25519 descriptor verification**: Verification of `BootstrapDescriptor` signatures using ed25519 trusted keys.
- **Neural core auto-calibration**: Added auto-calibration for MSE and O(1) time complexity optimization using sliding window in `VecDeque`.
- **CI/CD build automation**: Added automated release builds for Windows client binaries, NSIS installers, and iOS unsigned IPAs directly in GitHub Actions.

### Changed
- **Apksigner integration**: Switch from deprecated `jarsigner` to `apksigner` for Android APK v2/v3 signing.
- **Improved Windows installer**: Enhanced NSIS-based cross-compilation packaging.
- **Rust workspace version**: Bumped to 0.5.0.

### Fixed
- **Helper daemon security**: Fixed world-writable socket permissions in macOS client helper.
- **Key rotation logic**: Fixed key rotation ratchet no-op bug.
- **Deadlock resolved**: Fixed server handshake retry deadlock on Android.
- **Layout & Docs**: Stability fixes for macOS layout, secure fields, and post-connect sync.

## [0.4.0] - 2026-04-18

### Added
- **PSK-based bootstrap mask selection**: Deterministic initial mask selection based on PSK hash (blake3)
- **Multi-channel bootstrap loader**: Load descriptors from CDN, Telegram, GitHub, IPFS
- **Background descriptor refresh**: Automatic bootstrap descriptor updates
- **Neural resonance check**: Resonance verification system for detecting compromised masks
- **Mask recording mode**: Traffic recording mode for generating new masks from captured traffic
- **PFS ratchet**: Perfect Forward Secrecy with automatic key rotation
- **Linux arm64 support**: Full aarch64 support for server and client (Keenetic KN1012, OpenWrt, NanoPi R3S)
- **New mask presets**: Added QUIC over HTTPS v2 mask for improved traffic mimicry

### Changed
- **Optimized binary sizes**: Reduced binary sizes by 3-5x (release build)
- **Universal macOS binaries**: All macOS components built as universal (x86_64 + arm64)
- **Improved session management**: Better handling of sessions and reconnections
- **Removed 24h hard session timeout**: `HARD_TIMEOUT` now defaults to `Duration::ZERO` (unlimited). PFS ratchet handles key rotation, forced expiration caused reconnect failures (Issue #33)
- **Enhanced error handling**: More detailed connection error diagnostics

### Fixed
- **macOS helper daemon**: Fixed privileged helper daemon issues
- **Android JNI stability**: Improved JNI call stability
- **Bootstrap mask rotation**: Correct mask rotation on compromise
- **Session tag window**: Fixed edge cases in tag handling
- **Bootstrap mask loading** (Issue #38): Fixed parsing of bootstrap mask files - now supports both single MaskProfile objects and arrays of MaskProfile objects, as well as empty files
- **Bootstrap file reference removed from example config**: The `bootstrap_mask_files` entry has been removed from `config/server.json.example` since the bootstrap mask file is no longer created automatically. Users who need custom bootstrap masks can add the `bootstrap_mask_files` entry manually.

### Platform Updates
- **macOS**: v0.4.0 (build 4)
  - Installer: aivpn-macos.pkg (15 MB)
  - DMG: aivpn-macos.dmg (15 MB)
  - CLI: aivpn-client-macos-universal (17 MB)
- **Android**: API level 26+, universal APK 7 MB
- **Windows**: Rebuild required
- **Linux Server**:
  - x86_64 (4.7 MB)
  - arm64/aarch64 (5.0 MB) - **NEW** for Keenetic KN1012, OpenWrt, NanoPi R3S
  - armv7 (3.5 MB)
  - mipsel (4.5 MB)
- **Linux Client**:
  - x86_64 (3.8 MB)
  - arm64/aarch64 (9.6 MB) - **NEW** for Keenetic, OpenWrt, NanoPi
  - armv7 (3.5 MB)
  - mipsel (4.5 MB)

### Technical Details
- Rust workspace version: 0.4.0
- Protocol version: compatible with 0.3.x
- Minimum macOS: 13.0
- Minimum Android: 8.0 (API 26)
