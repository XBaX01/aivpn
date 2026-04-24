# AIVPN

传统VPN已经消亡。ISP和国家级别的防火墙（如GFW）只需查看数据包大小、时间间隔和握手模式，就能在几毫秒内检测到WireGuard和OpenVPN。你可以使用任何加密算法来加密载荷——DPI系统并不关心内容，它们阻止的是连接本身的"形状"。

**AIVPN**是我对现代深度包检测（DPI）的回应。我们不仅加密数据包——我们将它们伪装成真实的应用流量。当你的ISP看到一个Zoom通话或TikTok滚动时，实际上它是一个完全加密的隧道。

为了在实践中验证这一点，我构建了自己的DPI模拟器，重现了真实的过滤场景，并在不同模式下故意阻止流量。然后我在重负载下对系统进行了压力测试，以测量弹性、掩码切换速度和路由稳定性。为了实现快速路由，我实现了我的专利方法：USPTO（美国）申请号19/452,440，日期为2026年1月19日——《通过信号重建共振实现无监督多任务路由的系统和方法》。

## 支持的平台

| 平台 | 服务器 | 客户端 | 全隧道 | 备注 |
|------|--------|--------|--------|------|
| **Linux** | ✅ | ✅ | ✅ | 主要平台，通过`/dev/net/tun`的TUN |
| **macOS** | — | ✅ | ✅ | 通过`utun`内核接口，自动路由配置 |
| **Windows** | — | ✅ | ✅ | 通过[Wintun](https://www.wintun.net/)驱动程序 |
| **Android** | — | ✅ | ✅ | 通过`VpnService`API的原生Kotlin应用 |

### 当前客户端状态

- ✅ macOS应用：正常工作
- ✅ CLI客户端：正常工作
- ✅ Android应用：正常工作
- 🧪 Windows客户端：测试中

## 📥 下载（预编译二进制）

无需编译——下载并运行：

| 平台 | 文件 | 大小 | 备注 |
|------|------|------|------|
| **macOS** | [aivpn-macos.dmg](releases/aivpn-macos.dmg) | ~1.8 MB | 菜单栏应用，支持俄/英界面 |
| **Linux** | [aivpn-client-linux-x86_64](releases/aivpn-client-linux-x86_64) | ~4.0 MB | 原生x86_64 GNU/Linux CLI二进制 |
| **Linux ARMv7** | [aivpn-client-linux-armv7-musleabihf](releases/aivpn-client-linux-armv7-musleabihf) | ~4-5 MB | 用于ARMv7服务器和SBC的静态musl客户端二进制 |
| **Entware / MIPSel** | [aivpn-client-linux-mipsel-musl](releases/aivpn-client-linux-mipsel-musl) | ~4-5 MB | 用于支持Entware路由器的静态musl客户端二进制 |
| **Windows** | [aivpn-windows-package.zip](releases/aivpn-windows-package.zip) | ~7 MB | 包含`aivpn-client.exe` + `wintun.dll` |
| **Android** | [aivpn-client.apk](releases/aivpn-client.apk) | ~6.5 MB | 安装并粘贴你的连接密钥 |
| **Linux服务器** | [aivpn-server-linux-x86_64](releases/aivpn-server-linux-x86_64) | ~4.0 MB | 预编译的x86_64 GNU/Linux服务器二进制，用于VPS或快速Docker部署 |
| **Linux服务器 ARMv7** | [aivpn-server-linux-armv7-musleabihf](releases/aivpn-server-linux-armv7-musleabihf) | ~4-5 MB | 用于ARMv7 Linux主机的静态musl服务器二进制 |
| **Linux服务器 MIPSel** | [aivpn-server-linux-mipsel-musl](releases/aivpn-server-linux-mipsel-musl) | ~4-5 MB | 用于轻量级MIPSel/Entware系统的静态musl服务器二进制 |

### 快速开始（macOS）

1. 下载并打开`aivpn-macos.dmg`
2. 将**Aivpn.app**拖拽到应用程序文件夹
3. 启动——应用出现在菜单栏（无坞图标）
4. 粘贴你的连接密钥（`aivpn://...`）并点击**连接**
5. 切换🇷🇺/🇬🇧以切换语言

> ⚠️ VPN客户端需要root权限来访问TUN设备。应用将通过`sudo`提示输入密码。

### 快速开始（Windows）

1. 下载并解压[aivpn-windows-package.zip](releases/aivpn-windows-package.zip)
2. 确保`aivpn-client.exe`和`wintun.dll`保留在同一文件夹中
3. 在PowerShell中**以管理员身份**运行：
   ```powershell
   .\aivpn-client.exe -k "your_connection_key_here"
   ```

### 快速开始（Linux）

1. 下载[aivpn-client-linux-x86_64](releases/aivpn-client-linux-x86_64)
2. 使其可执行并作为root运行：
   ```bash
   chmod +x ./aivpn-client-linux-x86_64
   sudo ./aivpn-client-linux-x86_64 -k "your_connection_key_here"
   ```

### 快速开始（Entware路由器）

1. 为MIPSel路由器下载[aivpn-client-linux-mipsel-musl](releases/aivpn-client-linux-mipsel-musl)，或为ARMv7路由器下载[aivpn-client-linux-armv7-musleabihf](releases/aivpn-client-linux-armv7-musleabihf)
2. 将二进制文件复制到路由器，例如`/opt/bin/aivpn-client`
3. 使其可执行并从Entware shell作为root运行：
   ```sh
   chmod +x /opt/bin/aivpn-client
   /opt/bin/aivpn-client -k "your_connection_key_here"
   ```
4. 由于这些musl构建是静态链接的，路由器上不需要Rust工具链或额外的共享库。

### 快速开始（Android）

1. 下载并安装`aivpn-client.apk`
2. 在应用中粘贴你的连接密钥（`aivpn://...`）
3. 点击**连接**

### Android发布签名

对于生产签名的Android APK，创建`aivpn-android/keystore.properties`：

```properties
storeFile=/absolute/path/to/aivpn-release.jks
storePassword=your-store-password
keyAlias=aivpn
keyPassword=your-key-password
```

然后使用Java 21构建：

```bash
cd aivpn-android
export JAVA_HOME="$(/usr/libexec/java_home -v 21)"
export PATH="$JAVA_HOME/bin:$PATH"
./build-rust-android.sh release
```

如果`keystore.properties`不存在，脚本将回退到未签名的发布APK，然后仅使用调试keystore签名作为本地可安装的后备方案。

## ❤️ 支持项目

如果你觉得这个项目有帮助，可以通过Tribute捐款来支持其开发：

👉 https://t.me/tribute/app?startapp=dzX1

每一笔捐款都有助于AIVPN的持续发展。谢谢！🙌

## 主要功能：神经共振（AI）

最有趣的核心功能是我们的AI模块——**神经共振**。

我们没有在项目中拖入一个会耗尽廉价VPS所有内存的400MB大语言模型，而是：

- **预训练掩码编码器：**对于每个掩码配置文件（WebRTC编解码器、QUIC协议），我们训练了一个微型神经网络（MLP 64→128→64）并直接"烘焙"到二进制文件中。它仅重约66KB！
- **实时分析：**这个神经网络实时分析传入UDP数据包的熵和IAT（到达间隔时间）。
- **追踪审查者：**如果ISP的DPI系统试图探测我们的服务器（主动探测）或开始限制数据包，神经模块会检测到重建误差（MSE）的峰值。
- **自动掩码轮换：**一旦AI确定当前掩码已泄露（例如`webrtc_zoom`被标记），服务器和客户端会*无缝*地将流量重塑为备用掩码（例如`dns_over_udp`）。零断开连接！

## 其他很酷的功能

- **零RTT和PFS：**没有经典握手供嗅探器捕获。数据从第一个数据包就开始流动。完美前向保密（PFS）内置——密钥实时轮换，因此即使服务器被查封，旧流量转储也无法解密。
- **O(1)加密会话标签：**我们从不在明文中传输会话ID。相反，每个数据包都携带一个从时间戳和密钥派生的动态加密标签。服务器可以立即找到正确的客户端，但对任何观察者来说这只是噪声。
- **用Rust编写：**快速、内存安全、无泄漏。整个客户端二进制文件约2.5MB。在5美元的VPS上舒适运行。

## 入门指南

### 1. 克隆仓库

```bash
git clone https://github.com/infosave2007/aivpn.git
cd aivpn
```

### 2. 构建（需要Rust 1.75+）

项目分为工作区：`aivpn-common`（加密和掩码）、`aivpn-server`和`aivpn-client`。

```bash
# 在所有平台上命令相同：
cargo build --release
```

要在不在主机上安装Rust的情况下刷新Linux服务器发布工件：

```bash
./build-server-release.sh
```

对于ARMv7服务器和Entware级MIPSel路由器的静态musl构建：

```bash
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
```

要将最新的已发布Linux服务器版本一键部署到VPS：

```bash
./deploy-server-release.sh
```

> 对于GitHub Releases，将`aivpn-server-linux-x86_64`发布为默认Linux服务器资产，将`aivpn-windows-package.zip`作为主要Windows资产，并附加musl工件`aivpn-server-linux-armv7-musleabihf`、`aivpn-server-linux-mipsel-musl`、`aivpn-client-linux-armv7-musleabihf`和`aivpn-client-linux-mipsel-musl`用于ARM/Entware目标。原始`aivpn-client.exe`仅在`wintun.dll`与之一起提供时才是安全的。

GitHub Releases自动化：`.github/workflows/server-release-asset.yml`中的工作流程在每个发布的Release上构建`aivpn-server-linux-x86_64`以及ARMv7和MIPSel musl服务器/客户端资产，并自动上传它们。

### 3. 服务器（仅Linux）

#### 选项A：Docker（推荐）

最简单的方式——一切都在`docker-compose.yml`中预配置。

```bash
# 选择你系统上可用的Compose命令
if docker compose version >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker-compose"
else
    echo "安装Docker Compose v2（`docker-compose-v2`或`docker-compose-plugin`）或旧版`docker-compose`。"
    exit 1
fi

# 可选：在此处预创建config/server.json或config/server.key。
# 如果它们缺失，容器现在会自动引导两者。
mkdir -p config

# 启用NAT（VPN互联网访问所需）
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE

# 从预编译的Linux发布二进制文件快速启动
AIVPN_SERVER_DOCKERFILE=Dockerfile.prebuilt $AIVPN_COMPOSE up -d aivpn-server

# 或保留原始源码构建路径
$AIVPN_COMPOSE up -d aivpn-server
```

快速路径需要`releases/aivpn-server-linux-x86_64`本地存在。使用`./build-server-release.sh`构建或从Releases下载后再启动Docker。

对于VPS一键快速部署，运行`./deploy-server-release.sh`。它会下载发布资产，在需要时创建`config/server.key`，启用IPv4转发，为默认接口添加NAT规则，并使用`Dockerfile.prebuilt`启动Docker。

如果启用了防火墙，还使用系统工具允许`443/udp`：

```bash
# UFW (Ubuntu/Debian)
sudo ufw allow 443/udp

# firewalld (RHEL/CentOS/Fedora)
sudo firewall-cmd --add-port=443/udp --permanent
sudo firewall-cmd --reload
```

> 容器以`network_mode: "host"`运行，并在容器内挂载`./config` → `/etc/aivpn`。
> 首次启动时，它会从捆绑的示例自动创建`server.json`，并在任一文件缺失时生成`server.key`。

#### 选项B：裸金属

SSH到你的VPS，生成密钥：

```bash
sudo mkdir -p /etc/aivpn
openssl rand 32 | sudo tee /etc/aivpn/server.key > /dev/null
sudo chmod 600 /etc/aivpn/server.key
```

启动：

```bash
sudo ./target/release/aivpn-server --listen 0.0.0.0:443 --key-file /etc/aivpn/server.key
```

启用NAT：

```bash
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE
```

如果你使用不同于旧版`10.0.0.0/24`的VPN子网，请在`config/server.json`中将其保留为权威来源：

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

然后将NAT规则匹配到该子网，例如：

```bash
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.150.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.150.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE
```

### 3.1 客户端管理

AIVPN使用类似于WireGuard/XRay的客户端注册模型：每个客户端获得唯一的PSK、静态VPN IP和流量统计。

所有配置都打包在一个**连接密钥**中——用户将其粘贴到应用或CLI客户端的一个字符串。

连接密钥现在同时携带旧版顶级VPN IP字段和可选的引导`network_config`块。新客户端使用此块中的服务器提供的网络设置，然后从`ServerHello`确认它们。没有`network_config`的旧密钥仍然有效。

#### Docker

```bash
# 重用上面检测到的相同Compose命令
# 添加新客户端（打印连接密钥）
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# 输出：
# ✅ 客户端'Alice Phone'已创建！
#    ID:     a1b2c3d4e5f67890
#    VPN IP: 10.0.0.2
#
# ══ 连接密钥（粘贴到应用） ══
#
# aivpn://eyJpIjoiMTAuMC4wLjIiLCJrIjoiLi4uIiwibiI6eyJjbGllbnRfaXAiOiIxMC4wLjAuMiIsInNlcnZlcl92cG5faXAiOiIxMC4wLjAuMSIsInByZWZpeF9sZW4iOjI0LCJtdHUiOjEzNDZ9LCJwIjoiLi4uIiwicyI6IjEuMi4zLjQ6NDQzIn0

# 列出所有客户端及其流量统计
docker compose exec aivpn-server aivpn-server \
    --list-clients --clients-db /etc/aivpn/clients.json

# 显示特定客户端（及其连接密钥）
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --show-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# 删除客户端
docker compose exec aivpn-server aivpn-server \
    --remove-client "Alice Phone" \
    --clients-db /etc/aivpn/clients.json
```

> 使用Compose服务名称，因此无论生成的容器名称如何都能工作。

#### 裸金属

```bash
# 添加新客户端
aivpn-server \
    --add-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# 列出所有客户端及其流量统计
aivpn-server --list-clients --clients-db /etc/aivpn/clients.json

# 显示特定客户端（及其连接密钥）
aivpn-server \
    --show-client "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_PUBLIC_IP:443

# 删除客户端
aivpn-server \
    --remove-client "Alice Phone" \
    --clients-db /etc/aivpn/clients.json
```

### 3.2 录制自定义掩码

AIVPN支持从真实应用自动录制流量以创建新的伪装配置文件。这允许系统适应你网络中未被阻止的特定服务。

#### 录制工作原理

录制系统通过**认证的客户端连接**工作：

1. **创建管理员客户端**：在服务器上生成特殊的管理员密钥
2. **连接客户端**：使用管理员连接密钥启动AIVPN客户端
3. **开始录制**：通过VPN隧道发送`record start <service>`命令
4. **使用服务**：系统捕获数据包元数据（大小、间隔、头部）
5. **停止录制**：发送`record stop`以触发掩码生成和自测试

服务器端管道：
- **录制**：拦截来自VPN会话的UDP数据包
- **分析**：构建大小直方图，计算IAT周期，推断FSM
- **生成**：创建包含`HeaderSpec`的完整`MaskProfile`
- **自测试**：验证统计重现性
- **存储**：保存到掩码存储并在目录中注册

#### 分步指南

**1. 在服务器上创建管理员客户端：**

```bash
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_SERVER_IP:443

# 裸金属
aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip YOUR_SERVER_IP:443
```

保存输出的连接密钥（以`aivpn://`开头）。

**2. 使用管理员密钥连接客户端：**

```bash
sudo ./target/release/aivpn-client -k "aivpn://..."
```

**3. 开始录制服务：**

```bash
# 通过VPN隧道发送录制开始命令
aivpn record start --service zoom
```

**4. 正常使用服务30-60秒**以捕获多样的流量模式。

**5. 停止录制：**

```bash
aivpn record stop
```

服务器将分析捕获的数据包并生成新掩码。你将看到类似输出：

```
✅ 掩码已生成并测试！

   掩码ID:     zoom_custom_abc123
   服务:       zoom
   置信度:     0.87

   广播到所有客户端...
```

#### 良好掩码的要求

- **至少500个数据包**以获得统计显著性
- **最少60秒**的录制时间（系统要求）
- **多样化流量**：服务中的不同操作类型
- **稳定连接**：无断开连接或重传

每个掩码是一个单独的JSON文件，命名为`{mask_id}.json`。

### 4. 客户端

#### 连接密钥（推荐）

最简单的方式——粘贴来自`--add-client`的连接密钥：

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..."
```

现代客户端的优先级是：

1. 由`ServerHello`确认的网络设置
2. 来自连接密钥的引导`network_config`
3. 旧版回退`10.0.0.0/24`

迁移说明：旧客户端继续使用旧密钥和旧版`/24`默认值工作，但如果你将服务器移动到不同的子网或前缀，必须更新客户端并重新签发连接密钥。

全隧道：

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..." --full-tunnel
```

#### 手动模式

你也可以手动指定服务器地址和密钥（不使用PSK——用于旧版/无认证模式）：

##### Linux

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64
```

全隧道模式（通过VPN路由所有流量）：

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64 \
    --full-tunnel
```

##### macOS

同样，`cargo build --release`生成原生二进制文件：

```bash
sudo ./target/release/aivpn-client \
    --server YOUR_VPS_IP:443 \
    --server-key SERVER_PUBLIC_KEY_BASE64
```

> macOS将通过`ifconfig`/`route`自动配置`utun`接口和路由。

##### Windows

用户首选：下载并解压`releases/aivpn-windows-package.zip`。

如果改为分发原始文件，请将`wintun.dll`保留在`.exe`旁边：

```
aivpn-client.exe
wintun.dll
```

从PowerShell**以管理员身份**运行：

```powershell
.\aivpn-client.exe --server YOUR_VPS_IP:443 --server-key SERVER_PUBLIC_KEY_BASE64
```

全隧道：

```powershell
.\aivpn-client.exe --server YOUR_VPS_IP:443 --server-key SERVER_PUBLIC_KEY_BASE64 --full-tunnel
```

> 客户端将通过`route add`自动配置路由，并在退出时清理它们。

### 5. Android

1. 安装APK（`aivpn-android/app/build/outputs/apk/debug/app-debug.apk`）
2. 在单个输入字段中粘贴你的**连接密钥**（`aivpn://...`）
3. 点击**连接**

连接密钥包含一切：服务器地址、公钥、你的PSK和VPN IP。无需手动配置。

## 交叉编译

从当前机器为任何平台构建客户端：

```bash
# 从macOS/Windows构建Linux目标
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# 从Linux/macOS构建Windows目标
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
```

对于不需要安装本地交叉工具链的静态musl交叉构建，使用Docker支持的发布构建：

```bash
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
```

这些工件适用于ARM Linux服务器/SBC和支持Entware的MIPSel路由器。

对于Entware路由器，通常的流程是：构建或下载musl工件，将其复制到`/opt/bin`，`chmod +x`，然后直接从路由器shell运行。

## 项目结构

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs        # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs          # 伪装配置文件（WebRTC, QUIC, DNS）
│   └── protocol.rs      # 数据包格式，内部类型
├── aivpn-client/src/
│   ├── client.rs        # 核心客户端逻辑
│   ├── tunnel.rs        # TUN接口（Linux / macOS / Windows）
│   └── mimicry.rs       # 流量整形引擎
├── aivpn-server/src/
│   ├── gateway.rs       # UDP网关，MaskCatalog，共振循环
│   ├── neural.rs        # 预训练掩码编码器，异常检测器
│   ├── nat.rs           # NAT转发器（iptables）
│   ├── client_db.rs     # 客户端数据库（PSK，静态IP，统计）
│   ├── key_rotation.rs  # 会话密钥轮换
│   └── metrics.rs       # Prometheus监控
├── aivpn-android/       # Android客户端（Kotlin）
├── Dockerfile
├── docker-compose.yml
└── build.sh
```

## 贡献

想深入研究代码或为你的神经模块训练自己的掩码？加入：

- 掩码引擎：[`aivpn-common/src/mask.rs`](aivpn-common/src/mask.rs)
- 神经权重和异常检测器：[`aivpn-server/src/neural.rs`](aivpn-server/src/neural.rs)
- 跨平台TUN模块：[`aivpn-client/src/tunnel.rs`](aivpn-client/src/tunnel.rs)
- 测试（100+）：`cargo test`

欢迎PR！我们特别寻找有流量分析经验的人来捕获流行应用的转储并为神经共振训练新的配置文件。

---

许可证 — MIT。使用它，fork它，负责任地绕过审查。
