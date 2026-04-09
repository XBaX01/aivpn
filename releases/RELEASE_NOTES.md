# AIVPN Client Releases - March 30, 2026

## 📦 v0.3.0 — Major Update

### Ключевые изменения:
- 🔐 **macOS: Никаких паролей при подключении!** Привилегированный helper daemon устанавливается один раз через PKG и работает как системная служба
- 📦 **macOS: PKG-установщик** — один пароль при установке, потом подключение без пароля
- 🛡️ **Android: EncryptedSharedPreferences** — ключи шифруются через Android Keystore
- 📱 **Android: ABI splits** — отдельные APK для arm64, armv7, x86, x86_64 + универсальный
- 🏗️ **macOS: LaunchDaemon** — helper автоматически запускается при необходимости
- 🌐 **Unix socket IPC** — GUI общается с helper через `/var/run/aivpn/helper.sock`

---

## 🖥️ macOS

**Файлы:**
- `aivpn-macos.pkg` — Рекомендуемый установщик (5.9 MB)
- `aivpn-macos.dmg` — Для ручной установки (3.7 MB)

**Архитектура:** Universal Binary (arm64 + x86_64)  
**Минимальная версия:** macOS 13.0+

### Состав PKG:
- **Aivpn.app** — Swift UI приложение (Universal Binary)
- **aivpn-helper** — Привилегированный daemon (Universal Binary)
  - Запускается как LaunchDaemon (`com.aivpn.helper`)
  - Управляет aivpn-client через Unix socket
  - Один пароль при установке — никаких диалогов при подключении!
- **aivpn-client** — VPN клиент (Universal Binary)
  - Устанавливается в `/Library/Application Support/AIVPN/`
  - Поддержка Apple Silicon (M1/M2/M3/M4)
  - Поддержка Intel (x86_64)

### Установка (рекомендуемая):
```bash
sudo installer -pkg aivpn-macos.pkg -target /
```
Или дважды кликните на `aivpn-macos.pkg` в Finder.

### Установка (DMG, ручная):
1. Откройте `aivpn-macos.dmg`
2. Перетащите **Aivpn.app** в Applications
3. Запустите из Applications folder

### Архитектура:
```
┌──────────────┐     Unix Socket      ┌──────────────────┐
│  Aivpn.app   │ ◄──────────────────► │  aivpn-helper    │
│  (GUI, user) │  /var/run/aivpn/     │  (root, daemon)  │
│              │   helper.sock        │                  │
└──────────────┘                      │  ┌────────────┐  │
                                      │  │aivpn-client│  │
                                      │  │  (VPN core)│  │
                                      │  └────────────┘  │
                                      └──────────────────┘
```

---

## 🤖 Android

**Файлы:**
- `aivpn-client-arm64-v8a.apk` — ARM64 (современные устройства)
- `aivpn-client-armeabi-v7a.apk` — ARM 32-bit (старые устройства)
- `aivpn-client-x86.apk` — x86 (эмуляторы)
- `aivpn-client-x86_64.apk` — x86_64 (ChromeOS)
- `aivpn-client-universal.apk` — Все архитектуры

**Минимальная версия:** Android 8.0+ (API 26)  
**Разрешения:** VPN, Internet, Foreground Service, Notifications

### Безопасность:
- 🔑 Ключи подключения хранятся в **EncryptedSharedPreferences** (Android Keystore)
- Защита от root-доступа к хранилищу ключей
- Автоматическое шифрование при сохранении

### Установка:
1. Включите "Install from Unknown Sources" в настройках
2. Установите APK (выберите подходящий для вашего устройства)
3. Откройте приложение и вставьте connection key

---

## 🪟 Windows

**Файлы:** `aivpn-windows-package.zip`, `aivpn-client.exe`, `wintun.dll`  
**Размер:** 6.4 MB  
**Архитектура:** x86_64  
**Требования:** Windows 10/11, [wintun.dll](https://www.wintun.net/)

**Основной артефакт для GitHub Releases:** `aivpn-windows-package.zip`

### Установка:
1. Рекомендуется скачать и распаковать `aivpn-windows-package.zip`
2. Либо положить `wintun.dll` рядом с `aivpn-client.exe`
3. Запустите PowerShell **от имени Администратора**

### Быстрый старт:
```powershell
.\aivpn-client.exe -k "aivpn://eyJp..."
```

---

## 🔧 Linux (CLI)

**Файл:** `aivpn-client-linux-x86_64`  
**Требования:** sudo права для TUN устройства

`aivpn-client-universal` в `releases/` это macOS Mach-O Universal Binary (arm64 + x86_64), а Linux-релизный ELF публикуется отдельно как `aivpn-client-linux-x86_64`.

### Сборка из исходников:
```bash
cargo build --release
sudo ./target/release/aivpn-client -k "aivpn://..."
```

### Full tunnel mode:
```bash
sudo ./target/release/aivpn-client -k "aivpn://..." --full-tunnel
```

---

## 📝 Connection Key

Все клиенты используют единый формат connection key:

```
aivpn://BASE64({"s":"server:port","k":"server_pubkey","p":"psk","i":"vpn_ip"})
```

Получить key можно от сервера:
```bash
docker exec aivpn-server aivpn-server \
  --add-client "My Phone" \
  --key-file /etc/aivpn/server.key \
  --server-ip YOUR_PUBLIC_IP
```

---

## 🐛 Известные Проблемы

- ⚠️ Windows: Если выкладывать raw `aivpn-client.exe`, рядом обязательно нужен `wintun.dll`; для релизов предпочтителен `aivpn-windows-package.zip`
- ⚠️ macOS DMG: При первом запуске может потребоваться `xattr -cr` для снятия карантина
- ⚠️ Android: На некоторых устройствах требуется ручное разрешение VPN

---

## 📊 Статистика Релиза

| Платформа | Файл | Размер | Статус |
|-----------|------|--------|--------|
| macOS PKG | aivpn-macos.pkg | 5.9 MB | ✅ Universal (ARM+Intel) |
| macOS DMG | aivpn-macos.dmg | 3.7 MB | ✅ Universal (ARM+Intel) |
| Windows EXE | aivpn-client.exe | 6.4 MB | ✅ Готово |
| Windows DLL | wintun.dll | ~0.4 MB | ✅ Required runtime |
| Linux Binary | aivpn-client-linux-x86_64 | ~4.0 MB | ✅ x86_64 GNU/Linux |
| Android APK | aivpn-client-universal.apk | ~6.5 MB | ✅ Все ABI |
| macOS Binary | aivpn-client-universal | 6.3 MB | ✅ Universal (ARM+Intel, Mach-O) |

---

## 🔐 Проверка Контрольных Сумм

```bash
# macOS PKG
shasum -a 256 releases/aivpn-macos.pkg

# macOS DMG
shasum -a 256 releases/aivpn-macos.dmg

# Windows
certutil -hashfile releases\aivpn-client.exe SHA256

# Android
sha256sum releases/aivpn-client-universal.apk
```

---

**Дата сборки:** March 30, 2026  
**Версия:** 0.3.0  
**Статус:** ✅ Stable
