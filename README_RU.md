# AIVPN

Обычные VPN давно мертвы. Провайдеры и GFW (китайский файрвол) палят WireGuard и OpenVPN за доли секунды по размерам пакетов, интервалам и хэндшейкам. Можете шифровать трафик хоть тройным AES — DPI-системам плевать на содержимое, они блокируют саму *форму* соединения.

**AIVPN** — это мой ответ современным системам глубокого анализа трафика (DPI). Мы не просто шифруем пакеты, мы "натягиваем" на них маску реальных приложений. Для провайдера вы сидите в Zoom-колле или листаете TikTok, а на деле — это зашифрованный туннель.

Чтобы проверить это на практике, я разработал собственный эмулятор DPI, воспроизводил реальные сценарии фильтрации и целенаправленно блокировал трафик в разных режимах. Затем прогонял систему под высокой нагрузкой, чтобы оценить устойчивость, скорость переключения масок и стабильность маршрутизации. Для быстрого роутинга внедрено мое запатентованное решение: заявка USPTO (USA) № 19/452,440 от Jan 19, 2026 — *SYSTEM AND METHOD FOR UNSUPERVISED MULTI-TASK ROUTING VIA SIGNAL RECONSTRUCTION RESONANCE*.


## Поддерживаемые платформы

| Платформа | Сервер | Клиент | Полный туннель | Примечания |
|-----------|--------|--------|----------------|------------|
| **Linux** | ✅ | ✅ | ✅ | Основная платформа, TUN через `/dev/net/tun` |
| **macOS** | — | ✅ | ✅ | Через `utun`, автоматическая настройка маршрутов |
| **Windows** | — | ✅ | ✅ | Через [Wintun](https://www.wintun.net/) драйвер |
| **Android** | — | ✅ | ✅ | Kotlin-приложение через `VpnService` API |

### Текущий статус клиентов

- ✅ Приложение macOS: работает
- ✅ CLI-клиент: работает
- ✅ Android-приложение: работает
- 🧪 Windows-клиент: сейчас в тестировании

## 📥 Готовые бинарники

Не нужно ничего компилировать — скачайте и запускайте:

| Платформа | Файл | Размер | Примечания |
|-----------|------|--------|------------|
| **macOS** | [aivpn-macos.dmg](releases/aivpn-macos.dmg) | ~1.8 МБ | Приложение в menu bar с интерфейсом RU/EN |
| **Windows** | [aivpn-client.exe](releases/aivpn-client.exe) | ~6.4 МБ | Требуется [wintun.dll](https://www.wintun.net/) рядом с exe |
| **Android** | [aivpn-client.apk](releases/aivpn-client.apk) | ~6.5 МБ | Установите и вставьте ключ подключения |

### Быстрый старт (macOS)
1. Скачайте и откройте `aivpn-macos.dmg`
2. Перетащите **Aivpn.app** в Applications
3. Запустите — приложение появится в menu bar (без иконки в Dock)
4. Вставьте ключ подключения (`aivpn://...`) и нажмите **Подключить**
5. Нажмите 🇷🇺/🇬🇧 для переключения языка

> ⚠️ VPN-клиенту требуются права root для создания TUN-устройства. Приложение запросит пароль через `sudo`.

### Быстрый старт (Windows)
1. Скачайте `aivpn-client.exe` и [wintun.dll](https://www.wintun.net/)
2. Положите оба файла в одну папку
3. Запустите **от имени администратора** в PowerShell:
   ```powershell
   .\aivpn-client.exe -k "ваш_ключ_подключения"
   ```

### Быстрый старт (Android)
1. Скачайте и установите `aivpn-client.apk`
2. Вставьте ключ подключения (`aivpn://...`) в приложение
3. Нажмите **Подключить**

## ❤️ Поддержать проект

Если проект оказался полезным, вы можете поддержать его развитие донейшеном через Tribute:

👉 https://t.me/tribute/app?startapp=dzX1

Любая поддержка помогает развивать AIVPN дальше. Спасибо! 🙌

## Главная фича: Нейронный Резонанс (AI)

Самое интересное под капотом — это наш ИИ-модуль, который мы называем **Neural Resonance**.
Мы не стали тащить в проект огромные LLM-модели на 400 мегабайт, которые сожрут всю память на дешевом VPS. Вместо этого:

- **Baked Mask Encoder:** Под каждую маску (кодек WebRTC, протокол QUIC) мы натренировали и "запекли" в бинарник микро-нейросеть (MLP 64→128→64). Она весит всего ~66 КБ!
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

> На Windows убедитесь, что установлен [Wintun](https://www.wintun.net/) — скачайте `wintun.dll` и положите рядом с бинарником.

### 3. Сервер (только Linux)

#### Вариант А: Docker (рекомендуется)

Самый простой способ — всё настроено в `docker-compose.yml`.

```bash
# Генерируем ключ сервера
mkdir -p config
openssl rand 32 > config/server.key
chmod 600 config/server.key

# Включаем NAT (нужен для доступа в интернет через VPN)
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o eth0 -j MASQUERADE

# Собираем и запускаем
docker compose up -d aivpn-server
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
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/24 -o eth0 -j MASQUERADE
```

### 3.1 Управление клиентами

AIVPN использует модель регистрации клиентов по аналогии с WireGuard/XRay: у каждого клиента — уникальный PSK, статический VPN IP и статистика трафика.

Вся конфигурация упаковывается в один **ключ подключения** — одну строку, которую пользователь вставляет в приложение или CLI-клиент.

#### Docker

```bash
# Добавить клиента (выводит ключ подключения)
docker exec aivpn-server-aivpn-server-1 aivpn-server \
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
# aivpn://eyJpIjoiMTAuMC4wLjIiLCJrIjoiLi4uIiwicCI6Ii4uLiIsInMiOiIxLjIuMy40OjQ0MyJ9

# Список всех клиентов со статистикой
docker exec aivpn-server-aivpn-server-1 aivpn-server \
    --list-clients --clients-db /etc/aivpn/clients.json

# Показать конкретного клиента (и его ключ подключения)
docker exec aivpn-server-aivpn-server-1 aivpn-server \
    --show-client "Телефон Алисы" \
    --key-file /etc/aivpn/server.key \
    --clients-db /etc/aivpn/clients.json \
    --server-ip ВАШ_ПУБЛИЧНЫЙ_IP:443

# Удалить клиента
docker exec aivpn-server-aivpn-server-1 aivpn-server \
    --remove-client "Телефон Алисы" \
    --clients-db /etc/aivpn/clients.json
```

> **Имя контейнера** зависит от имени директории проекта. Проверьте через `docker ps`. Типичные имена: `aivpn-aivpn-server-1` или `aivpn-server-aivpn-server-1`.

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

### 4. Клиент

#### Ключ подключения (рекомендуется)

Самый простой способ — вставить ключ подключения из `--add-client`:

```bash
sudo ./target/release/aivpn-client -k "aivpn://eyJp..."
```

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

Скачайте и положите `wintun.dll` (от [WireGuard/wintun](https://www.wintun.net/)) рядом с `.exe`:

```
aivpn-client.exe
wintun.dll
```

Запуск из Powershell **с правами администратора**:

```powershell
.\aivpn-client.exe --server IP_ВАШЕГО_VPS:443 --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64
```

Для полного туннеля:

```powershell
.\aivpn-client.exe --server IP_ВАШЕГО_VPS:443 --server-key ПУБЛИЧНЫЙ_КЛЮЧ_BASE64 --full-tunnel
```

> Клиент автоматически настроит маршруты через `route add` и корректно откатит их при завершении.

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

## Структура проекта

```
aivpn/
├── aivpn-common/src/
│   ├── crypto.rs        # X25519, ChaCha20-Poly1305, BLAKE3
│   ├── mask.rs          # Профили мимикрии (WebRTC, QUIC, DNS)
│   └── protocol.rs      # Формат пакетов, inner types
├── aivpn-client/src/
│   ├── client.rs        # Основная логика клиента
│   ├── tunnel.rs        # TUN-интерфейс (Linux / macOS / Windows)
│   └── mimicry.rs       # Движок шейпинга трафика
├── aivpn-server/src/
│   ├── gateway.rs       # UDP-шлюз, MaskCatalog, resonance loop
│   ├── neural.rs        # Baked Mask Encoder, AnomalyDetector
│   ├── nat.rs           # NAT-форвардер (iptables)
│   ├── client_db.rs     # База клиентов (PSK, статический IP, статистика)
│   ├── key_rotation.rs  # Ротация сессионных ключей
│   └── metrics.rs       # Prometheus-мониторинг
├── aivpn-android/       # Android-клиент (Kotlin)
├── Dockerfile
├── docker-compose.yml
└── build.sh
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
