🌐 [English](README.md) | [Русский](README_RU.md)

# AIVPN

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android%20%7C%20iOS%20%7C%20MikroTik-informational)

---

## 概述

AIVPN 是一款基于 UDP 的 VPN 系统，将标准隧道加密与**流量拟态**相结合：出站数据包被重塑为已知应用协议的形态（WebRTC、QUIC、DNS-over-UDP），使连接在统计上与正常应用流量无法区分。

核心技术特性：

- **零 RTT 数据启动** — 加密载荷可从第一个数据包开始传输，无需强制握手往返。
- **O(1) 会话查找** — 明文中不传输会话 ID。每个数据包携带一个 8 字节的*共振标签*，由时间戳和每会话密钥派生。服务器通过 `DashMap` 以常数时间解析会话。
- **完美前向保密** — 通过 X25519 棘轮机制进行飞行中会话密钥轮换。服务器密钥泄露不会暴露过去的流量。
- **神经共振模块** — 每个掩码的微型 MLP（约 66 KB）实时监控流量统计；高重建误差触发自动掩码轮换，不中断客户端连接。
- **Rust 编写** — 内存安全，无 GC 停顿。客户端二进制约 2.5 MB，可在 $5 VPS 上运行。

---

## 架构

### 工作区布局

```
aivpn-common/       — 共享加密、协议、掩码配置（无 I/O）
aivpn-server/       — 仅 Linux VPN 网关和管理 CLI
aivpn-client/       — 跨平台 VPN 客户端（Linux / macOS / Windows）
aivpn-android-core/ — Android JNI 桥接
aivpn-windows/      — Windows GUI（egui/eframe）
aivpn-android/      — Android Kotlin 应用
aivpn-macos/        — macOS SwiftUI 菜单栏应用
aivpn-ios-core/     — iOS Rust 静态库（C FFI）
aivpn-ios/          — iOS SwiftUI 应用 + NEPacketTunnelProvider
mask-assets/        — 捆绑的流量拟态 JSON 配置文件
```

### 核心模块

| 模块 | 位置 | 用途 |
|------|------|------|
| `crypto.rs` | `aivpn-common` | X25519 密钥交换、ChaCha20-Poly1305 AEAD、BLAKE3/HMAC、共振标签生成 |
| `protocol.rs` | `aivpn-common` | 线路格式：`[8字节标签][pad_len][内部头][加密载荷][poly1305标签]` |
| `mask.rs` | `aivpn-common` | `MaskProfile` — 流量整形：头部模板、FSM 状态、IAT 分布 |
| `gateway.rs` | `aivpn-server` | 核心事件循环：UDP 接收、会话分发、NAT 转发、神经检查 |
| `session.rs` | `aivpn-server` | `SessionManager` — `DashMap` O(1) 查找，256 条目重放窗口 |
| `neural.rs` | `aivpn-server` | 神经共振：每掩码 MLP 64→128→64，MSE 阈值 0.35，自动轮换 |
| `client.rs` | `aivpn-client` | 状态机：未配置 → 连接中 → 已连接，密钥交换，重连 |
| `tunnel.rs` | `aivpn-client` | 跨平台 TUN：`/dev/net/tun`（Linux）、`utun`（macOS）、Wintun（Windows） |
| `mimicry.rs` | `aivpn-client` | `MimicryEngine` — 对出站数据包应用 `MaskProfile` |

### 节点池同步

服务器间客户端数据库同步使用 `ControlPayload::PoolSync`，通过普通 VPN UDP 数据包传输 — 与客户端流量无法区分。无需单独 TCP 端口或防火墙规则。

---

## 平台支持

| 平台 | 服务器 | 客户端 | GUI | TUN 驱动 |
|------|:------:|:------:|:---:|---------|
| Linux | ✅ | ✅ | ✅ AppImage + 托盘 | `/dev/net/tun` |
| macOS | — | ✅ | ✅ 菜单栏 | `utun` |
| Windows | — | ✅ | ✅ egui | [Wintun](https://www.wintun.net/) |
| Android | — | ✅ | ✅ 原生 Kotlin | `VpnService` API |
| iOS | — | ✅ | ✅ SwiftUI | `NetworkExtension` |
| MikroTik RouterOS 7.6+ | — | ✅ | — | 容器 veth + TUN |
| Entware 路由器（ARMv7 / MIPSel） | — | ✅ | — | musl 静态二进制 |

### 功能能力矩阵

| 功能 | CLI | Win | Mac | Android | iOS |
|------|:---:|:---:|:---:|:-------:|:---:|
| 流量伪装 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 自适应模式（4 级） | ✅ | ✅ | ✅ | ✅ | ✅ |
| 实时连接质量 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 分流隧道 | ✅ | ✅ | ✅ | ✅ | ✅ |
| DNS 代理 | ✅ | ✅ | ✅ | ❌ | ❌ |
| Kill Switch | ✅ | ✅ | ✅ | ✅ | ✅ |
| mTLS 证书 | ✅ | ✅ | ✅ | ✅ | ✅ |
| FEC（前向纠错） | ✅ | ✅ | ✅ | ✅ | ✅ |
| 流量录制 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 设备密钥 / JIT | ✅ | ✅ | ✅ | ✅ | ✅ |
| SOCKS5 代理 | ✅ | ✅ | ✅ | ❌ | ❌ |
| 全流量隧道 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 诊断 / 基准测试 | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## 快速入门

### 服务器（Linux）

#### Docker（推荐）

```
mkdir -p config
docker compose up -d aivpn-server
```

容器首次启动时自动生成 `server.key` 和 `server.json`，以 `network_mode: host` 运行，挂载 `./config` → `/etc/aivpn`。

开放防火墙 UDP 443 端口：

```
# UFW
sudo ufw allow 443/udp
# firewalld
sudo firewall-cmd --add-port=443/udp --permanent && sudo firewall-cmd --reload
```

#### 裸机

```
sudo mkdir -p /etc/aivpn
openssl rand 32 | sudo tee /etc/aivpn/server.key > /dev/null
sudo chmod 600 /etc/aivpn/server.key
sudo ./aivpn-server --listen 0.0.0.0:443 --key-file /etc/aivpn/server.key
```

服务器自动启用 IPv4 转发并安装 NAT 伪装规则（优先 nftables，回退 iptables）。隧道本身无需手动配置防火墙。

#### 添加客户端

```
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip 您的公网IP:443

# 裸机
aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip 您的公网IP:443
```

输出包含连接密钥（`aivpn://…`）— 分发给客户端。

其他管理命令：`--list-clients`、`--show-client`、`--remove-client`。

---

### 客户端 — Linux

```
sudo ./aivpn-client -k "aivpn://..."
# 全隧道（所有流量通过 VPN）
sudo ./aivpn-client -k "aivpn://..." --full-tunnel
```

### 客户端 — macOS

从 [Releases](https://github.com/infosave2007/aivpn/releases) 下载 `aivpn-macos.dmg`，将 **Aivpn.app** 拖入 Applications，启动 — 出现在菜单栏。粘贴连接密钥并点击 **Connect**。

命令行：
```
sudo ./aivpn-client -k "aivpn://..."
```

> 应用通过 `sudo` 请求密码以创建 `utun` 接口。

### 客户端 — Windows

**安装程序（推荐）：** 下载 `aivpn-windows-installer.exe`，以管理员身份运行，从开始菜单启动 **AIVPN**。

**便携版：** 解压 `aivpn-windows-package.zip`（包含 `aivpn.exe`、`aivpn-client.exe`、`wintun.dll`），以管理员身份运行 `aivpn.exe`。

命令行（PowerShell，管理员权限）：
```
.\aivpn-client.exe -k "aivpn://..."
```

> 创建 Wintun 网络适配器需要管理员权限。

### 客户端 — Android

1. 安装 `aivpn-client.apk`
2. 粘贴连接密钥（`aivpn://…`）
3. 点击 **Connect**

### 客户端 — iOS

在 macOS 上构建（需要 Xcode 15+）：

```
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./scripts/build-ios.sh 您的TEAM_ID
```

安装 `releases/aivpn-ios.ipa`：
```
xcrun devicectl device install app --device <UDID> releases/aivpn-ios.ipa
```

> 免费 Apple 开发者账户即可。侧载构建 7 天后过期。

### 客户端 — Entware 路由器（ARMv7 / MIPSel）

```
scp aivpn-client-linux-armv7-musleabihf root@router:/opt/bin/aivpn-client
ssh root@router 'chmod +x /opt/bin/aivpn-client && /opt/bin/aivpn-client -k "aivpn://..."'
```

### 客户端 — MikroTik RouterOS 7.6+

```
/system/device-mode/update container=yes
/interface/veth/add name=veth-aivpn address=172.31.0.2/30 gateway=172.31.0.1
/ip/address/add address=172.31.0.1/30 interface=veth-aivpn
/container/mounts/add name=aivpn-tun src=/dev/net/tun dst=/dev/net/tun type=bind
/container/envs/add list=aivpn-env name=AIVPN_KEY value="aivpn://..."
/container/add remote-image=infosave2007/aivpn-mikrotik:latest interface=veth-aivpn start-on-boot=yes envlist=aivpn-env mounts=aivpn-tun
/container/start [find remote-image~"aivpn-mikrotik"]
/ip/route/add dst-address=0.0.0.0/0 gateway=172.31.0.2
```

详见 [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md)。

### SOCKS5 代理模式（无需 root）

```
aivpn-client -k "aivpn://..." --proxy-listen 127.0.0.1:1080
```

配置 Firefox / Chrome / curl 使用 `SOCKS5 127.0.0.1:1080`，无需 TUN 设备或管理员权限。

---

## 连接密钥格式

连接密钥将所有服务器和客户端参数编码为单个可移植字符串：

```
aivpn://<base64url(JSON)>
```

JSON 字段：

| 字段 | 类型 | 描述 |
|------|------|------|
| `s` | `string` | 服务器地址，如 `"1.2.3.4:443"` |
| `k` | `string` | 服务器 X25519 公钥（base64） |
| `p` | `string` | 客户端预共享密钥 / PSK（base64） |
| `i` | `string` | 客户端静态 VPN IP，如 `"10.0.0.2"` |
| `n` | `object` | *（可选）* 引导 `network_config`（见下文） |

`network_config` 对象（`n`）：

| 字段 | 描述 |
|------|------|
| `client_ip` | 客户端 TUN IP |
| `server_vpn_ip` | 服务器 TUN IP |
| `prefix_len` | 子网前缀长度 |
| `mtu` | 内部 MTU |

连接时的优先级：

1. `ServerHello` 确认的设置（权威）
2. 密钥中的引导 `network_config`
3. 传统回退 `10.0.0.0/24`

不含 `network_config` 的密钥完全兼容。

生成密钥：
```
aivpn-server --add-client "姓名" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

重新打印现有密钥：
```
aivpn-server --show-client "姓名" --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json --server-ip IP:PORT
```

---

## 服务器配置参考

默认配置路径：`config/server.json`（本地）或 `/etc/aivpn/server.json`。CLI 标志覆盖文件值。

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

| 字段 | 默认值 | 描述 |
|------|--------|------|
| `listen_addr` | `0.0.0.0:443` | UDP 绑定地址，端口自动嵌入连接密钥 |
| `tun_name` | 随机 | TUN 接口名称 |
| `tun_mtu` | _（未设置）_ | `"auto"` = 物理 MTU 减去 64 字节开销（回退 1346）；或固定整数 |
| `mask_dir` | `/var/lib/aivpn/masks` | 扫描 `.json` 掩码配置文件的目录 |
| `bootstrap_mask_files` | `[]` | 启动时预加载的掩码文件，降低首次连接延迟 |
| `session_timeout_secs` | `0` | 会话硬性上限；`0` = 无限制 |
| `idle_timeout_secs` | `300` | 断开静默超时（秒） |
| `allow_peer_routing` | `false` | 在子网内路由 VPN 客户端间的数据包 |
| `network_config.server_vpn_ip` | `10.0.0.1` | 服务器 TUN IP |
| `network_config.prefix_len` | `24` | VPN 子网前缀 |
| `network_config.mtu` | `1346` | 通过 `ServerHello` 发送给客户端的内部 MTU |
| `network_config.keepalive_secs` | `8` | 与客户端协商的心跳间隔 |
| `network_config.ipv6_enabled` | `false` | 启用 IPv6 NAT66 |
| `network_config.ipv6_prefix` | `fd10:cafe::/48` | 客户端 IPv6 地址的 ULA /48 前缀 |
| `pool.peers` | `[]` | 数据库同步的对等服务器地址 |
| `pool.sync_key` | `""` | 共享 32 字节 BLAKE3 密钥（base64）。生成：`openssl rand -base64 32` |

### 可选功能（Cargo）

| 功能 | 启用内容 |
|------|---------|
| `neural` | 神经共振模块（基于 MSE 的掩码轮换） |
| `management-api` | Unix 套接字 HTTP API，位于 `/run/aivpn/api.sock` |
| `metrics` | Prometheus 导出器 |
| `passive-distribution` | 引导描述符分发渠道 |

```
cargo build --release --bin aivpn-server --features "management-api,metrics,neural"
```

---

## 从源码构建

要求：Rust 1.75+、`cargo`。

```
git clone https://github.com/infosave2007/aivpn.git
cd aivpn

# 构建所有工作区成员
cargo build --release

# 单独构建
cargo build --release --bin aivpn-server
cargo build --release --bin aivpn-client

# 运行测试
cargo test

# 静态 musl 交叉构建（ARMv7 / MIPSel）
./scripts/build-musl-release.sh server armv7-unknown-linux-musleabihf
./scripts/build-musl-release.sh client mipsel-unknown-linux-musl

# Docker 服务器构建（输出到 releases/）
./scripts/build-server-release.sh

# Windows GUI（从 Linux 交叉编译）
./scripts/build-windows-gui.sh

# iOS（macOS + Xcode 15+）
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./scripts/build-ios.sh              # 未签名 / 模拟器
./scripts/build-ios.sh 您的TEAM_ID # 设备签名
```

### Android

```
export ANDROID_SDK_ROOT=/opt/android-sdk
export ANDROID_NDK_ROOT=/opt/android-ndk
echo "sdk.dir=$ANDROID_SDK_ROOT" > aivpn-android/local.properties

cd aivpn-android
./build-rust-android.sh release
```

签名构建：运行脚本前创建 `aivpn-android/keystore.properties`。

### 从 crates.io 安装

```
cargo install aivpn-client
cargo install aivpn-server
```

---

## 高级功能

### 设备绑定（JIT 注册）

连接密钥可指定为*一次性*：第一个连接的设备绑定其 X25519 静态密钥，来自不同设备的后续连接将被拒绝。

```
# 创建注册槽位
aivpn-server --add-client-one-time "Alice-Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip IP:PORT

# 重置绑定（重新启用注册）
aivpn-server --reset-device "Alice-Phone" \
    --clients-db /etc/aivpn/clients.json
```

各平台设备密钥存储位置：

| 平台 | 位置 |
|------|------|
| Linux / macOS | `~/.config/aivpn/device.key`（权限 600，自动生成） |
| Windows | `%APPDATA%\aivpn\device.key` |
| Android | 通过 `EncryptedSharedPreferences` 使用 Android Keystore |
| iOS | Keychain，`kSecAttrAccessibleAfterFirstUnlock` |

### 连接质量评分与自适应模式

AIVPN 持续计算 **0–100 质量评分**，来源：RTT（40 分）、抖动（20 分）、丢包（30 分）、Neural MSE（10 分）。自适应模式自动调整心跳间隔和 FEC 组大小：

| 评分 | 自适应级别 | 心跳 | FEC 组 |
|------|-----------|------|--------|
| 80–100 | 关闭 | 8 秒 | 禁用 |
| 50–79 | 轻度 | 6 秒 | 1/16 |
| 20–49 | 积极 | 4 秒 | 1/8 |
| 0–19 | 卫星 | 15 秒 | 1/4 |

```
aivpn-client -k "aivpn://..." --adaptive
```

### 前向纠错（FEC）

每 N 个上行数据包发送一个 XOR 修复包。如果一组中恰好丢失一个包，服务器立即重建 — 无需重传往返。N 由自适应模式控制。清洁链路上 FEC 禁用。

### 多服务器节点池同步

```json
{
  "pool": {
    "peers": ["node2.example.com:443"],
    "sync_key": "<base64-32字节密钥>"
  }
}
```

### 多跳链式转发

客户端流量通过两个 AIVPN 节点路由。客户端仅连接入口节点；互联网看到出口节点的 IP。

**入口节点：**
```json
{ "pool": { "sync_key": "<密钥>", "exit_node": "exit.example.com:443" } }
```
**出口节点：**
```json
{ "pool": { "sync_key": "<相同密钥>", "exit_node_enabled": true } }
```

### 本地 DNS 代理

```
aivpn-client -k "aivpn://..." --dns-proxy 127.0.0.1:5300 --dns-upstream 1.1.1.1:53
```

### 流量录制 — 自定义掩码创建

```
aivpn-client record start --service myapp
# ... 使用应用程序 60+ 秒 ...
aivpn-client record stop
```

服务器分析数据包大小直方图和到达间隔时间，生成 `MaskProfile`，通过自测验证，并分发到活跃会话。

### 基准测试

```
aivpn-client bench -k "aivpn://..."
# P50: 12ms  P95: 28ms  Up: 47 Mbps  Down: 52 Mbps  Score: 94/100
aivpn-client bench -k "aivpn://..." --json
```

---

## 安全模型

| 属性 | 机制 |
|------|------|
| 加密 | ChaCha20-Poly1305 AEAD |
| 密钥交换 | X25519 ECDH |
| 会话认证 | 每客户端 PSK（可选设备绑定） |
| 前向保密 | 飞行中 X25519 密钥棘轮 |
| 重放保护 | 每会话 256 条目滑动窗口 |
| 会话匿名性 | 8 字节 BLAKE3 派生共振标签；明文中无会话 ID |
| 流量拟态 | `MaskProfile` FSM：头部注入、IAT 整形 |
| 掩码完整性 | 神经共振 MSE 阈值（0.35）；自动轮换 |
| NAT 穿透 | 服务器端 nftables/iptables，客户端 `SO_REUSEPORT` |

详细对手模型和威胁分析：[THREAT_MODEL.md](THREAT_MODEL.md)。

---

## 项目结构

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs
│   ├── mask.rs
│   ├── protocol.rs
│   └── fec.rs
├── aivpn-client/src/
│   ├── client.rs
│   ├── tunnel.rs
│   ├── kill_switch.rs
│   └── mimicry.rs
├── aivpn-server/src/
│   ├── gateway.rs
│   ├── neural.rs
│   ├── nat.rs
│   ├── client_db.rs
│   └── pool_sync.rs
├── aivpn-android/
├── aivpn-ios/
├── aivpn-windows/
├── aivpn-macos/
├── mask-assets/
├── scripts/
├── docker/
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md
```

---

## 许可证

MIT — 见 [LICENSE](LICENSE)。
