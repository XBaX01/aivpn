# AIVPN

[![CI](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/infosave2007/aivpn/actions/workflows/ci.yml)
[![Crates.io Server](https://img.shields.io/crates/v/aivpn-server.svg?label=aivpn-server)](https://crates.io/crates/aivpn-server)
[![Crates.io Client](https://img.shields.io/crates/v/aivpn-client.svg?label=aivpn-client)](https://crates.io/crates/aivpn-client)
![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)

Обычные VPN давно мертвы. Провайдеры и GFW (китайский файрвол) палят WireGuard и OpenVPN за доли секунды по размерам пакетов, интервалам и хэндшейкам. Можете шифровать трафик хоть тройным AES — DPI-системам плевать на содержимое, они блокируют саму *форму* соединения.

**AIVPN** — это мой ответ современным системам глубокого анализа трафика (DPI). Мы не просто шифруем пакеты, мы "натягиваем" на них маску реальных приложений. Для провайдера вы сидите в Zoom-колле или листаете TikTok, а на деле — это зашифрованный туннель.

Чтобы проверить это на практике, я разработал собственный эмулятор DPI, воспроизводил реальные сценарии фильтрации и целенаправленно блокировал трафик в разных режимах. Затем прогонял систему под высокой нагрузкой, чтобы оценить устойчивость, скорость переключения масок и стабильность маршрутизации. Для быстрого роутинга внедрено мое запатентованное решение: заявка USPTO (USA) № 19/452,440 от Jan 19, 2026 — *SYSTEM AND METHOD FOR UNSUPERVISED MULTI-TASK ROUTING VIA SIGNAL RECONSTRUCTION RESONANCE*.


## Поддерживаемые платформы

| Платформа | Сервер | Клиент | Полный туннель | Примечания |
|-----------|--------|--------|----------------|------------|
| **Linux** | ✅ | ✅ | ✅ | Основная платформа, TUN через `/dev/net/tun`; GUI-приложение (AppImage + трей) |
| **macOS** | — | ✅ | ✅ | Через `utun`, автоматическая настройка маршрутов |
| **Windows** | — | ✅ | ✅ | Через [Wintun](https://www.wintun.net/) драйвер |
| **Android** | — | ✅ | ✅ | Kotlin-приложение через `VpnService` API |
| **iOS** | — | ✅ | ✅ | Нативное SwiftUI-приложение через `NetworkExtension` API |
| **MikroTik RouterOS** | — | ✅ | ✅ | Контейнер RouterOS 7.6+, arm64/armv7/amd64 |

### Текущий статус клиентов

- ✅ Приложение macOS: работает
- ✅ CLI-клиент: работает
- ✅ Android-приложение: работает
- ✅ iOS-приложение: работает (сборка требует macOS + Xcode 15+)
- ✅ Windows-клиент: работает (GUI + CLI)
- ✅ MikroTik RouterOS контейнер: работает (arm64/armv7/amd64)

## 📥 Загрузки

Готовые бинарные файлы для всех поддерживаемых платформ автоматически собираются и прикрепляются к каждому релизу. Вы можете скачать последние версии на странице [GitHub Releases](https://github.com/infosave2007/aivpn/releases).


### Быстрый старт (macOS)
1. Скачайте `aivpn-macos.dmg` со страницы [GitHub Releases](https://github.com/infosave2007/aivpn/releases) и откройте его
2. Перетащите **Aivpn.app** в Applications
3. Запустите — приложение появится в menu bar (без иконки в Dock)
4. Вставьте ключ подключения (`aivpn://...`) и нажмите **Подключить**
5. Нажмите 🇷🇺/🇬🇧 для переключения языка
> ⚠️ VPN-клиенту требуются права root для создания TUN-устройства. Приложение запросит пароль через `sudo`.

### Быстрый старт (Windows)

#### Вариант А: Установщик (рекомендуется)
1. Скачайте [aivpn-windows-installer.exe](https://github.com/infosave2007/aivpn/releases)
2. Правой кнопкой мыши → **Запустить от имени администратора**, следуйте инструкциям установщика
3. Запустите **AIVPN** из меню «Пуск» (запускается с правами администратора автоматически)
4. Вставьте ключ подключения (`aivpn://...`) и нажмите **Подключить**

> ⚠️ VPN-клиенту требуются права администратора для создания сетевого адаптера Wintun. Всегда запускайте от имени администратора.

#### Вариант Б: Портативный архив
1. Скачайте и распакуйте [aivpn-windows-package.zip](https://github.com/infosave2007/aivpn/releases)
2. Убедитесь, что `aivpn.exe`, `aivpn-client.exe` и `wintun.dll` лежат в одной папке
3. Правой кнопкой на `aivpn.exe` → **Запустить от имени администратора** для GUI, или через CLI:
   ```powershell
   .\aivpn-client.exe -k "ваш_ключ_подключения"
   ```

### Быстрый старт (Linux)
1. Скачайте [aivpn-client-linux-x86_64](https://github.com/infosave2007/aivpn/releases)
2. Сделайте файл исполняемым и запустите от root:
    ```bash
    chmod +x ./aivpn-client-linux-x86_64
    sudo ./aivpn-client-linux-x86_64 -k "ваш_ключ_подключения"
    ```

### Быстрый старт (Entware роутеры)
1. Скачайте `aivpn-client-linux-mipsel-musl` или `aivpn-client-linux-armv7-musleabihf` со страницы [GitHub Releases](https://github.com/infosave2007/aivpn/releases).
2. Скопируйте бинарник на роутер, например в `/opt/bin/aivpn-client`.
3. Сделайте файл исполняемым и запустите из Entware shell от root:
    ```sh
    chmod +x /opt/bin/aivpn-client
    /opt/bin/aivpn-client -k "ваш_ключ_подключения"
    ```
4. Эти musl-сборки статически слинкованы, поэтому на роутере не нужен Rust toolchain и дополнительные shared libraries.

### Быстрый старт (MikroTik RouterOS)
1. Включите поддержку контейнеров: `/system/device-mode/update container=yes` и перезагрузите роутер
2. Выполните команды настройки (см. [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md)):
   ```routeros
   /interface/veth/add name=veth-aivpn address=172.31.0.2/30 gateway=172.31.0.1
   /ip/address/add address=172.31.0.1/30 interface=veth-aivpn
   /container/mounts/add name=aivpn-tun src=/dev/net/tun dst=/dev/net/tun type=bind
   /container/envs/add list=aivpn-env name=AIVPN_KEY value="aivpn://..."
   /container/add remote-image=infosave2007/aivpn-mikrotik:latest interface=veth-aivpn start-on-boot=yes envlist=aivpn-env mounts=aivpn-tun
   /container/start [find remote-image~"aivpn-mikrotik"]
   ```
3. Добавьте маршрут по умолчанию через контейнер: `/ip/route/add dst-address=0.0.0.0/0 gateway=172.31.0.2`

Полная документация с настройкой policy routing и решением типичных проблем — в [aivpn-mikrotik/README.md](aivpn-mikrotik/README.md).

### Быстрый старт (Android)
1. Скачайте и установите `aivpn-client.apk`
2. Вставьте ключ подключения (`aivpn://...`) в приложение
3. Нажмите **Подключить**

### 📦 Установка через Cargo (crates.io)

Если у вас установлен Rust, вы можете легко установить клиент или сервер напрямую из crates.io:

```bash
cargo install aivpn-client
cargo install aivpn-server
```

### Быстрый старт (iOS)
1. Соберите на macOS (требуется Xcode 15+, `xcodegen`):
   ```bash
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
   cargo install xcodegen
   ./build-ios.sh ВАШ_TEAM_ID
   ```
2. Установите `releases/aivpn-ios.ipa` на устройство:
   - Перетащите в **Xcode → Window → Devices and Simulators**, или
   - `xcrun devicectl device install app --device <UDID> releases/aivpn-ios.ipa`
3. Откройте приложение, вставьте ключ подключения (`aivpn://...`) и нажмите **Подключить**

> Бесплатный Apple ID (personal team) достаточен — платный Developer Program не нужен. Установки истекают через 7 дней.

## ❤️ Поддержать проект

Если проект оказался полезным, вы можете поддержать его развитие донейшеном через Tribute:

👉 https://t.me/tribute/app?startapp=dzX1

Любая поддержка помогает развивать AIVPN дальше. Спасибо! 🙌

## Главная фича: Нейронный Резонанс (AI)

Самое интересное под капотом — это наш ИИ-модуль, который мы называем **Neural Resonance**.
Мы не стали тащить в проект огромные LLM-модели на 400 мегабайт, которые сожрут всю память на дешевом VPS. Вместо этого:

- **Baked Mask Encoder:** Под каждую маску (кодек WebRTC, протокол QUIC) мы детерминированно выводим микро-нейросеть (MLP 64→128→64) напрямую из 64-float вектора подписи маски — засеянного BLAKE3-хэшем этой подписи. Уникальна для каждой маски, ~66 КБ, никаких внешних файлов обучения не требуется.
- **Анализ в реальном времени:** Эта нейронка на лету анализирует энтропию и IAT (тайминги) прилетающих UDP-пакетов.
- **Охота на цензоров:** Если DPI-система провайдера пытается прощупать наш сервер (Active Probing) или начинает задерживать пакеты, нейромодуль видит рост ошибки реконструкции (MSE).
- **Авто-ротация масок:** Как только ИИ понимает, что текущая маска скомпрометирована (например, `webrtc_zoom` спалили), сервер и клиент *без разрыва соединения* перестраивают шейпинг трафика под резервную маску (например, на `dns_over_udp`). Никаких дисконнектов!

## Что ещё крутого

- **Zero-RTT и PFS:** Нет классического рукопожатия (handshake), которое так любят ловить снифферы. Данные льются с первого же пакета. При этом работает Perfect Forward Secrecy — ключи ротируются на лету, так что если сервак когда-нибудь изымут, расшифровать старый дамп трафика не выйдет.
- **O(1) криптотеги сессий:** Мы не передаем ID сессии в открытом виде. Вместо этого в каждый пакет вшивается динамический криптографический тег, зависящий от таймстемпа и секретного ключа. Сервер находит нужного клиента моментально, а для стороннего наблюдателя это просто белый шум.
- **Написан на Rust:** Быстрый, безопасный, без утечек памяти. Весь бинарник клиента весит около 2.5 МБ. Спокойно крутится на серверах за пару баксов.

## Как поднять всё это добро

### 1. Клонируем репозиторий

```bash
git clone https://github.com/infosave2007/aivpn.git
cd aivpn
```

### 2. Сборка (потребуется Rust 1.75+)

Проект разбит на воркспейсы: `aivpn-common` (шифры и маски), `aivpn-server` и `aivpn-client`.

```bash
# Все плафтормы — одна команда:
cargo build --release
```

Чтобы обновить Linux-артефакт сервера без установки Rust на хост:

```bash
./build-server-release.sh
```

Для статических musl-сборок под ARMv7 серверы и MIPSel/Entware роутеры:

```bash
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
```

Для сборки iOS-приложения (требуется macOS + Xcode 15+):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo install xcodegen
./build-ios.sh              # неподписанная сборка (CI / симулятор)
./build-ios.sh ВАШ_TEAM_ID  # подписанная для устройства (бесплатный Apple ID)
```

Артефакт копируется в `releases/aivpn-ios.ipa`.

Чтобы развернуть последнюю опубликованную Linux-версию сервера на VPS одной командой:

```bash
./deploy-server-release.sh
```

> Для GitHub Releases серверным Linux-артефактом по умолчанию должен оставаться `aivpn-server-linux-x86_64`, основным Windows-артефактом — `aivpn-windows-package.zip`, а для ARM/Entware нужно прикладывать musl-артефакты `aivpn-server-linux-armv7-musleabihf`, `aivpn-server-linux-mipsel-musl`, `aivpn-client-linux-armv7-musleabihf` и `aivpn-client-linux-mipsel-musl`. Отдельный `aivpn-client.exe` безопасно выкладывать только вместе с `wintun.dll` рядом.

Автоматизация GitHub Releases: workflow в `.github/workflows/server-release-asset.yml` собирает `aivpn-server-linux-x86_64`, а также ARMv7 и MIPSel musl-артефакты для сервера и клиента при публикации Release и автоматически прикладывает их к релизу.

Для Docker-backed кросс-сборки без локального тулчейна используйте:

```bash
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
```

Эти артефакты рассчитаны на ARM Linux-серверы/SBC и MIPSel-роутеры с Entware.

### 3. Сервер (только Linux)

#### Вариант А: Docker (рекомендуется)

Самый простой способ — всё настроено в `docker-compose.yml`.

```bash
# Определяем Compose-команду, которая есть именно на вашей системе
if docker compose version >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
    AIVPN_COMPOSE="docker-compose"
else
    echo "Установите Docker Compose v2 (`docker-compose-v2` или `docker-compose-plugin`) либо legacy `docker-compose`."
    exit 1
fi

# Генерируем ключ сервера
mkdir -p config
openssl rand 32 > config/server.key
chmod 600 config/server.key

# Включаем NAT (нужен для доступа в интернет через VPN)
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE

# Быстрый старт из готового Linux-бинарника
AIVPN_SERVER_DOCKERFILE=Dockerfile.prebuilt $AIVPN_COMPOSE up -d aivpn-server

# Или оставить исходный путь со сборкой из исходников
$AIVPN_COMPOSE up -d aivpn-server
```

Быстрый путь ожидает локальный файл `releases/aivpn-server-linux-x86_64`. Его можно собрать командой `./build-server-release.sh` или скачать из Releases перед запуском Docker.

Для быстрого деплоя на VPS одной командой используйте `./deploy-server-release.sh`. Скрипт скачивает релизный артефакт, создаёт `config/server.key` при необходимости, включает IPv4 forwarding, добавляет NAT-правило для интерфейса по умолчанию и запускает Docker через `Dockerfile.prebuilt`.

Если у вас включён firewall, откройте `443/udp` тем инструментом, который есть в системе:

```bash
# UFW (Ubuntu/Debian)
sudo ufw allow 443/udp

# firewalld (RHEL/CentOS/Fedora)
sudo firewall-cmd --add-port=443/udp --permanent
sudo firewall-cmd --reload
```

> Контейнер запускается с `network_mode: "host"` и монтирует `./config` → `/etc/aivpn` внутри контейнера.

#### Вариант Б: На голом железе

Заходите на свой VPS, генерите ключ:

```bash
sudo mkdir -p /etc/aivpn
openssl rand 32 | sudo tee /etc/aivpn/server.key > /dev/null
sudo chmod 600 /etc/aivpn/server.key
```

Поднимаем:

```bash
sudo ./target/release/aivpn-server --listen 0.0.0.0:443 --key-file /etc/aivpn/server.key
```

Включаем NAT:

```bash
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE
```

Если VPN-подсеть у вас не legacy `10.0.0.0/24`, держите её в `config/server.json` как единственный авторитетный источник:

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

И NAT-правило тоже должно соответствовать этой подсети, например:

```bash
DEFAULT_IFACE=$(ip route show default | awk '/default/ {print $5; exit}')
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -C POSTROUTING -s 10.150.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE 2>/dev/null || \
sudo iptables -t nat -A POSTROUTING -s 10.150.0.0/24 -o "$DEFAULT_IFACE" -j MASQUERADE
```

`listen_addr` управляет портом (по умолчанию: 443). Чтобы использовать другой порт:

```json
{
  "listen_addr": "0.0.0.0:8443",
  ...
}
```

Порт автоматически вшивается в ключи подключения — клиентам не нужна ручная настройка. Переменная окружения `AIVPN_LISTEN` или флаг `--listen` переопределяют значение из `server.json`.

#### Полный справочник server.json

```json
{
  "listen_addr": "0.0.0.0:443",
  "tun_name": "aivpn0",
  "tun_mtu": "auto",
  "mask_dir": "/var/lib/aivpn/masks",
  "bootstrap_mask_files": ["/etc/aivpn/masks/custom.json"],
  "session_timeout_secs": 0,
  "idle_timeout_secs": 300,
  "network_config": {
    "server_vpn_ip": "10.0.0.1",
    "prefix_len": 24,
    "mtu": 1346,
    "keepalive_secs": 8,
    "ipv6_enabled": false,
    "ipv6_prefix": "fd10:cafe::/48"
  }
}
```

| Поле | По умолчанию | Описание |
|------|-------------|----------|
| `listen_addr` | `0.0.0.0:443` | UDP-адрес и порт прослушивания |
| `tun_name` | случайный | Имя TUN-устройства (`aivpn0`, `tun0`, …) |
| `tun_mtu` | _(не задан)_ | `"auto"` — автоопределение MTU по физическому интерфейсу (−64 байта накладных расходов, при ошибке — 1346); или целое число, например `1400` |
| `mask_dir` | `/var/lib/aivpn/masks` | Папка с `.json`-файлами профилей масок |
| `bootstrap_mask_files` | `[]` | Дополнительные маски, загружаемые при старте (снижают задержку первого подключения) |
| `session_timeout_secs` | `0` | Жёсткий лимит сессии в секундах; `0` = без ограничений |
| `idle_timeout_secs` | `300` | Отключить клиента после N секунд тишины |
| `network_config.server_vpn_ip` | `10.0.0.1` | IP сервера в TUN-интерфейсе |
| `network_config.prefix_len` | `24` | Длина префикса VPN-подсети |
| `network_config.mtu` | `1346` | Внутренний MTU туннеля, передаваемый клиенту в `ServerHello` |
| `network_config.keepalive_secs` | `8` | Интервал keepalive, согласуемый с клиентами (сек.) |
| `network_config.ipv6_enabled` | `false` | Включить IPv6 NAT66 — каждому клиенту назначается IPv6-адрес из `ipv6_prefix` |
| `network_config.ipv6_prefix` | `fd10:cafe::/48` | ULA /48-префикс для клиентских IPv6-адресов |

### 3.1 Управление клиентами

AIVPN использует модель регистрации клиентов по аналогии с WireGuard/XRay: у каждого клиента — уникальный PSK, статический VPN IP и статистика трафика.

Вся конфигурация упаковывается в один **ключ подключения** — одну строку, которую пользователь вставляет в приложение или CLI-клиент.

Теперь ключ подключения несёт не только legacy-поле VPN IP, но и необязательный блок `network_config` для начальной сетевой конфигурации. Новый клиент берёт сетевые параметры из этого блока и затем подтверждает их через `ServerHello`. Старые ключи без `network_config` продолжают работать.

#### Docker

```bash
# Используйте ту же Compose-команду, что определили выше
# Добавить клиента (выводит ключ подключения)
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --add-client "Телефон Алисы" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Вывод:
# ✅ Client 'Телефон Алисы' created!
#    ID:     a1b2c3d4e5f67890
#    VPN IP: 10.0.0.2
#
# ══ Connection Key (paste into app) ══
#
# aivpn://eyJpIjoiMTAuMC4wLjIiLCJrIjoiLi4uIiwibiI6eyJjbGllbnRfaXAiOiIxMC4wLjAuMiIsInNlcnZlcl92cG5faXAiOiIxMC4wLjAuMSIsInByZWZpeF9sZW4iOjI0LCJtdHUiOjEzNDZ9LCJwIjoiLi4uIiwicyI6IjEuMi4zLjQ6NDQzIn0

# Список всех клиентов со статистикой
docker compose exec aivpn-server aivpn-server \
    --list-clients --clients-db /etc/aivpn/clients.json

# Показать конкретного клиента (и его ключ подключения)
$AIVPN_COMPOSE exec aivpn-server aivpn-server \
    --show-client "Телефон Алисы" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Удалить клиента
docker compose exec aivpn-server aivpn-server \
    --remove-client "Телефон Алисы" \
    --clients-db /etc/aivpn/clients.json
```

> Используется имя сервиса Compose, поэтому команда не зависит от сгенерированного имени контейнера.

#### На голом железе

```bash
# Добавить клиента
aivpn-server \
    --add-client "Телефон Алисы" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Список всех клиентов со статистикой
aivpn-server --list-clients --clients-db /etc/aivpn/clients.json

# Показать конкретного клиента (и его ключ подключения)
aivpn-server \
    --show-client "Телефон Алисы" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Удалить клиента
aivpn-server \
    --remove-client "Телефон Алисы" \
    --clients-db /etc/aivpn/clients.json
```

### 3.2 Запись собственных масок

AIVPN поддерживает автоматическую запись трафика реальных приложений для создания новых профилей мимикрии. Это позволяет адаптировать систему под конкретные сервисы, которые не блокируются в вашей сети.

#### Как работает запись

Система записи работает через **аутентифицированное клиентское подключение**:

1. **Создать admin-клиента**: Сгенерировать специальный админский ключ на сервере
2. **Подключить клиент**: Запустить AIVPN-клиент с админским ключом подключения
3. **Начать запись**: Отправить команду `record start <service>` через VPN-туннель
4. **Использовать сервис**: Система захватывает метаданные пакетов (размеры, интервалы, заголовки)
5. **Остановить запись**: Отправить `record stop` для генерации маски и самотестирования

Серверный конвейер:
- **Запись**: Перехват UDP-пакетов из VPN-сессии
- **Анализ**: Построение гистограммы размеров, вычисление периодов IAT, вывод FSM
- **Генерация**: Создание полного `MaskProfile` с `HeaderSpec`
- **Самотестирование**: Проверка воспроизведения статистических свойств
- **Сохранение**: Сохранение в хранилище и регистрация в каталоге

#### Пошаговая инструкция

**1. Создать admin-клиента на сервере:**

```bash
# Docker
docker compose exec aivpn-server aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_IP_СЕРВЕРА:443

# На голом железе
aivpn-server \
    --add-client "recording-admin" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_IP_СЕРВЕРА:443
```

Сохраните выходной ключ подключения (начинается с `aivpn://`).

**2. Подключить клиент с админским ключом:**

```bash
sudo ./target/release/aivpn-client -k "aivpn://..."
```

**3. Начать запись для сервиса:**

```bash
# Отправить команду начала записи через VPN-туннель
aivpn record start --service zoom
```

**4. Использовать сервис нормально** в течение нескольких минтут для захвата разнообразных паттернов трафика.

**5. Остановить запись:**

```bash
aivpn record stop
```

Сервер проанализирует захваченные пакеты и сгенерирует новую маску. Вы увидите вывод:

```
✅ Mask generated and tested!

   Mask ID:     zoom_custom_abc123
   Service:     zoom
   Confidence:  0.87

   Broadcasting to all clients...
```

#### Требования к хорошим маскам

- **Минимум 500 пакетов** для статистической значимости
- **Минимум 60 секунд** записи (требование системы) лучше больше
- **Разнообразный трафик**: разные типы операций в сервисе
- **Стабильное соединение**: без разрывов и ретрансмиссий

Каждая маска — отдельный JSON-файл с именем `{mask_id}.json`.

### 4. Клиент

#### Ключ подключения (рекомендуется)

Самый простой способ — вставить ключ подключения из `--add-client`:

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..."
```

Приоритет у новых клиентов такой:

1. Сетевые параметры, подтверждённые сервером в `ServerHello`
2. Bootstrap `network_config` из ключа подключения
3. Legacy fallback `10.0.0.0/24`

Важно для миграции: старые клиенты продолжают работать со старыми ключами и legacy `/24`, но если вы переносите сервер в другую подсеть или меняете префикс, клиентов нужно обновить, а ключи подключения лучше перевыпустить.

Полный туннель:

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..." --full-tunnel
```

#### Ручной режим

Также можно указать адрес и ключ сервера вручную (без PSK — для работы без регистрации):

#### Linux

```bash
sudo ./target/release/aivpn-client \
    --server IP_ВАШЕГО_VPS:443 \
    --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64
```

Для полного туннеля (весь трафик через VPN):

```bash
sudo ./target/release/aivpn-client \
    --server IP_ВАШЕГО_VPS:443 \
    --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64 \
    --full-tunnel
```

#### macOS

Точно так же, `cargo build --release` соберет нативный бинарник:

```bash
sudo ./target/release/aivpn-client \
    --server IP_ВАШЕГО_VPS:443 \
    --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64
```

> macOS автоматически настроит `utun`-интерфейс и маршруты через `ifconfig` / `route`.

#### Windows

Для пользователей предпочтительна установка через [aivpn-windows-installer.exe](https://github.com/infosave2007/aivpn/releases) (включает GUI-приложение, CLI-клиент и Wintun драйвер).

Альтернативно можно скачать и распаковать [aivpn-windows-package.zip](https://github.com/infosave2007/aivpn/releases). Архив содержит:

```
aivpn.exe          # GUI-приложение
aivpn-client.exe   # CLI-клиент
wintun.dll         # Сетевой драйвер Wintun
```

> ⚠️ **Требуются права администратора.** VPN-клиенту нужны права администратора для создания сетевого адаптера Wintun. Всегда запускайте через правую кнопку мыши → «Запуск от имени администратора» или из PowerShell с повышенными привилегиями.

**GUI-режим** (рекомендуется): правой кнопкой на `aivpn.exe` → **Запуск от имени администратора**, вставьте ключ подключения и нажмите «Подключить».

**CLI-режим** из PowerShell **от имени администратора**:

```powershell
.\aivpn-client.exe --server IP_ВАШЕГО_VPS:443 --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64
```

Для полного туннеля:

```powershell
.\aivpn-client.exe --server IP_ВАШЕГО_VPS:443 --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64 --full-tunnel
```

> Клиент автоматически настроит маршруты через `route add` и корректно откатит их при завершении.

### 4.1 Прокси-режим (SOCKS5, без root)

Вместо TUN-устройства клиент может работать как локальный **SOCKS5-прокси**. Это позволяет пустить конкретный браузер или приложение через VPN без прав администратора/root и без установки драйвера ядра.

```bash
# Запустить SOCKS5-прокси на порту 1080 (sudo не нужен)
aivpn-client -k "aivpn://eyJp..." --proxy-listen 127.0.0.1:1080
```

Настройте своё приложение на использование `SOCKS5` по адресу `127.0.0.1:1080`:

| Приложение | Настройка |
|------------|-----------|
| **Firefox** | Настройки → Параметры сети → Ручная настройка прокси → SOCKS5 `127.0.0.1:1080`, включить «Проксировать DNS» |
| **Chrome / Chromium** | Запуск с флагом `--proxy-server=socks5://127.0.0.1:1080` |
| **curl** | `curl --proxy socks5h://127.0.0.1:1080 https://example.com` |
| **git** | `git config --global http.proxy socks5h://127.0.0.1:1080` |

**Ограничения:**
- IPv6-адреса назначения не поддерживаются (используйте имена хостов или IPv4)
- UDP-трафик не проксируется (только TCP CONNECT)
- DNS разрешается локально через системный резолвер (запросы не идут через VPN)

### 5. Android

1. Установите APK (`aivpn-android/app/build/outputs/apk/debug/app-debug.apk`)
2. Вставьте свой **ключ подключения** (`aivpn://...`) в поле ввода
3. Нажмите **Подключить**

Ключ подключения содержит всё: адрес сервера, публичный ключ, ваш PSK и VPN IP. Никакой ручной настройки.

## Кросс-компиляция

Можно собирать клиент под любую платформу прямо со своей машины:

```bash
# Для Linux из macOS/Windows
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# Для Windows из Linux/macOS
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
```

Для статических musl-кросс-сборок без локального тулчейна используйте Docker-backed release builds:

```bash
./build-musl-release.sh client armv7-unknown-linux-musleabihf
./build-musl-release.sh client mipsel-unknown-linux-musl
./build-musl-release.sh server armv7-unknown-linux-musleabihf
./build-musl-release.sh server mipsel-unknown-linux-musl
```

Эти артефакты рассчитаны на ARM Linux-серверы/SBC и MIPSel-роутеры с Entware.

Для Entware-роутеров обычный поток такой: собрать или скачать musl-артефакт, скопировать его в `/opt/bin`, выдать `chmod +x` и запускать прямо из shell роутера.

## Расширенная конфигурация сервера

### Пул-синхронизация между серверами (встроена в протокол)

Серверы автоматически синхронизируют базу клиентов. Синхронизация встроена в основной VPN-протокол как управляющее сообщение `PoolSync` — неотличима от клиентского трафика. Отдельный TCP-порт и дополнительные правила брандмауэра не нужны.

`server.json`:
```json
{
  "pool": {
    "peers": ["node2.example.com:443", "node3.example.com:443"],
    "sync_key": "<base64-ключ 32 байта>"
  }
}
```
Генерация ключа: `openssl rand -base64 32`

### Резервное копирование / Миграция

```bash
# Экспорт (БД клиентов, маски, конфиг сервера)
aivpn-server --export /tmp/aivpn-backup.tar.gz

# Предпросмотр и восстановление
aivpn-server --import /tmp/aivpn-backup.tar.gz --dry-run
aivpn-server --import /tmp/aivpn-backup.tar.gz --target-dir /etc/aivpn
```

### QoS на уровне клиента

```bash
aivpn-server --set-client-qos "Alice" --bw-up 10M --bw-down 50M --dscp EF
```

Применяется через eBPF TC при наличии ядра, иначе — программный токен-баккет.

### Бенчмарк и диагностика

```bash
# По умолчанию: 10-секундный тест
aivpn-client bench -k "aivpn://..."
# P50: 12ms  P95: 28ms  Up: 47 Mbps  Down: 52 Mbps  Score: 94/100

# Задать продолжительность
aivpn-client bench -k "aivpn://..." --duration 30

# Вывод в JSON
aivpn-client bench -k "aivpn://..." --json
```

Доступен из CLI и в панели диагностики всех GUI-клиентов (Windows, macOS, iOS, Android).

Проверить статус записи маски:

```bash
aivpn-client record status -k "aivpn://..."
```

Удалить зависшие правила kill-switch после некорректного завершения:

```bash
aivpn-client kill-switch clear
```

### Адаптивный режим

Автоматическая настройка MTU и keepalive на основе измерения потерь пакетов в реальном времени:

```bash
aivpn-client -k "aivpn://..." --adaptive
```

### OpenWRT / LuCI

Нативный пакет для OpenWRT: procd init-скрипт, UCI-конфигурация и веб-интерфейс LuCI. Смотри `aivpn-openwrt/docs/openwrt-setup.md`.

### Журнал аудита администратора

Все операции управления записываются в `/var/log/aivpn/audit.log` (JSONL, путь задаётся через `--audit-log`) с полями: актор, действие, объект, результат, ISO-8601 метка времени.

### Многоскачковая цепочка (Multi-hop)

Маршрутизация трафика клиента через два узла AIVPN без изменений на стороне клиента. Входной узел пересылает зашифрованные IP-пакеты на выходной узел. Клиент общается только с входным узлом.

```
[Клиент] ──(зашифровано)──► [Входной узел] ──(ChainForward)──► [Выходной узел] ──► Интернет
```

Входной узел: `pool.exit_node = "exit.example.com:443"`. Выходной узел: `pool.exit_node_enabled = true`. Оба узла используют одинаковый `pool.sync_key`.

### DNS-over-HTTPS прокси

Направляет DNS-трафик клиентов через зашифрованный DoH-резолвер. Сборка с `--features "dns"`. `block_plain_dns: true` блокирует UDP/53 вне VPN-интерфейса.

```json
{ "dns": { "upstream_doh": "https://cloudflare-dns.com/dns-query", "block_plain_dns": true } }
```

### Сеть сайт-сайт

Соединяет подсети нескольких серверов AIVPN без клиентского ПО. Маршруты устанавливаются автоматически при получении объявлений от пиров.

```json
{ "site_to_site": { "local_subnets": ["192.168.1.0/24"], "peers": [{ "name": "b", "endpoint": "b.example.com:443", "sync_key": "...", "remote_subnets": ["192.168.2.0/24"] }] } }
```

### mTLS-сертификаты клиентов

Опциональный ed25519-сертификат (104 байта) поверх X25519 + PSK. `required: false` (по умолчанию) — обратная совместимость. `required: true` — блокирует данные до предъявления сертификата.

**1. Генерация CA-ключа на сервере:**

```bash
aivpn-server --gen-ca --key-file /etc/aivpn/server.key
# Выводит: публичный и приватный ключи CA в hex
```

**2. Выпуск клиентского сертификата:**

```bash
aivpn-server --issue-cert "Alice Phone" \
    --key-file /etc/aivpn/server.key \
    --ca-key <CA_PRIVATE_KEY_HEX> \
    --days 365
# Выводит 104-байтовый сертификат (base64) для передачи клиенту
```

**3. Настройка сервера** (`server.json`):

```json
{
  "mtls": {
    "ca_public_key_hex": "<CA_PUBLIC_KEY_HEX>",
    "required": false
  }
}
```

**4. Подключение с сертификатом (CLI-клиент):**

```bash
aivpn-client -k "aivpn://..." --mtls-cert /path/to/client.cert
```

GUI-клиенты (Windows, macOS, iOS, Android) имеют отдельное поле mTLS в настройках профиля.

### Телеметрия дропов eBPF XDP

Счётчики дропов по причинам через BPF-кольцевой буфер, события `XdpDrop` в `EventBus`. Активируется при наличии `/sys/fs/bpf/aivpn/drop_stats`.

---

## Структура проекта

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs          # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs            # Профили мимикрии (WebRTC, QUIC, DNS)
│   ├── protocol.rs        # Формат пакетов, inner types
│   └── kernel_accel.rs    # API /dev/aivpn + XDP-хелперы
├── aivpn-client/src/
│   ├── client.rs          # Логика клиента (split-tunnel, kill-switch, XDP)
│   ├── tunnel.rs          # TUN-интерфейс (Linux / macOS / Windows)
│   ├── kill_switch.rs     # Kill-switch (nftables / pfctl / netsh)
│   └── mimicry.rs         # Движок шейпинга трафика
├── aivpn-server/src/
│   ├── gateway.rs         # UDP-шлюз, MaskCatalog, resonance loop
│   ├── neural.rs          # Baked Mask Encoder, AnomalyDetector
│   ├── nat.rs             # NAT-форвардер (IPv4 + IPv6 NAT66)
│   ├── client_db.rs       # База клиентов (PSK, статический IP, статистика)
│   ├── key_rotation.rs    # Ротация сессионных ключей
│   └── metrics.rs         # Prometheus-мониторинг
├── aivpn-common/mask-assets/   # 11 профилей мимикрии трафика (JSON)
├── aivpn-linux/           # Linux Iced GUI (AppImage + системный трей)
├── aivpn-linux-kernel/    # Опциональный ядерный модуль (aivpn.ko) + XDP-фильтр
├── aivpn-android/         # Android-клиент (Kotlin)
├── aivpn-ios-core/        # iOS Rust staticlib (C FFI)
├── aivpn-ios/             # iOS SwiftUI + NEPacketTunnelProvider
├── Dockerfile
├── docker-compose.yml
└── THREAT_MODEL.md        # Модель угроз и безопасность протокола
```

## Разработка и контрибы

Хотите поковыряться в коде или обучить свою маску для нейронки? Залетайте:

- Движок масок: [`aivpn-common/src/mask.rs`](aivpn-common/src/mask.rs)
- Обученные веса и детектор аномалий: [`aivpn-server/src/neural.rs`](aivpn-server/src/neural.rs)
- Кроссплатформенный TUN-модуль: [`aivpn-client/src/tunnel.rs`](aivpn-client/src/tunnel.rs)
- Тесты (больше сотни): `cargo test`

Буду рад пулл-реквестам! Особо ищем спецов по анализу трафика, чтобы снимать дампы с реальных приложений и обучать новые профили для Neural Resonance.

---

Лицензия — MIT. Пользуйтесь, форкайте, обходите блокировки с умом.
