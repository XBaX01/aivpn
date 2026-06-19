🌐 [English](README.md) | [中文](README_CN.md)

# AIVPN

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)
![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android%20%7C%20iOS%20%7C%20MikroTik-informational)

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

### Таблица функциональных возможностей

| Функция | CLI | Win | Mac | Android | iOS |
|---------|:---:|:---:|:---:|:-------:|:---:|
| Маскировка трафика | ✅ | ✅ | ✅ | ✅ | ✅ |
| Адаптивный режим (4 уровня) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Качество соединения (live) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Split Tunnel | ✅ | ✅ | ✅ | ✅ | ✅ |
| DNS Proxy | ✅ | ✅ | ✅ | ❌ | ❌ |
| Kill Switch | ✅ | ✅ | ✅ | ✅ | ✅ |
| mTLS сертификат | ✅ | ✅ | ✅ | ✅ | ✅ |
| FEC (помехоустойчивость) | ✅ | ✅ | ✅ | ✅ | ✅ |
| Запись трафика | ✅ | ✅ | ✅ | ✅ | ✅ |
| Device Key / JIT | ✅ | ✅ | ✅ | ✅ | ✅ |
| SOCKS5 Proxy | ✅ | ✅ | ✅ | ❌ | ❌ |
| Полный туннель | ✅ | ✅ | ✅ | ✅ | ✅ |
| Диагностика / тест | ✅ | ✅ | ✅ | ✅ | ✅ |

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
./scripts/build-ios.sh ВАШ_TEAM_ID
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
./scripts/build-musl-release.sh server armv7-unknown-linux-musleabihf
./scripts/build-musl-release.sh client mipsel-unknown-linux-musl

# Docker-сборка сервера (результат в releases/)
./scripts/build-server-release.sh

# Windows GUI (кросс-компиляция с Linux)
./scripts/build-windows-gui.sh

# iOS (требуется macOS + Xcode 15+)
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./scripts/build-ios.sh              # без подписи / симулятор
./scripts/build-ios.sh ВАШ_TEAM_ID  # с подписью для устройства
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
├── scripts/               # Скрипты сборки и деплоя
├── docker/                # Dockerfiles и точка входа
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md
```

---

## Лицензия

MIT — см. [LICENSE](LICENSE).
