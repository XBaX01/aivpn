# Сборка Windows-инсталлятора AIVPN

## Требования

| Компонент | Версия | Установка |
|-----------|--------|-----------|
| Rust | stable | [rustup.rs](https://rustup.rs) |
| Target | `x86_64-pc-windows-msvc` | `rustup target add x86_64-pc-windows-msvc` |
| NSIS | 3.x | `winget install NSIS.NSIS` |
| Python 3 + Pillow | 3.10+ | `pip install Pillow` |
| Visual Studio Build Tools | 2019+ | [visualstudio.microsoft.com](https://visualstudio.microsoft.com/visual-cpp-build-tools/) |

## Структура

```
aivpn-windows/           — исходники GUI (egui/wgpu)
  assets/
    aivpn.ico            — иконка (генерируется скриптом)
    generate_icon.py     — генератор иконки из Android-дизайна
  build.rs               — встраивает .ico в .exe через winres
  src/main.rs            — точка входа GUI

windows-installer/
  aivpn-installer.nsi    — NSIS-скрипт инсталлятора
  build-installer.ps1    — PowerShell-скрипт сборки
  BUILD.md               — эта инструкция

releases/
  aivpn-windows-package/ — промежуточная директория с бинарниками
  aivpn-windows-installer.exe — готовый инсталлятор
```

## Пошаговая сборка

### 1. Перегенерировать иконку (если менялся дизайн)

```powershell
python aivpn-windows\assets\generate_icon.py
```

Скрипт создаёт `aivpn-windows/assets/aivpn.ico` — точная копия Android-иконки
(Material Security shield, `#6C5CE7` на `#121218`).

### 2. Собрать бинарники

```powershell
# GUI-приложение (debug — обходит проблему OpenGL на VirtIO GPU)
cargo build -p aivpn-windows --target x86_64-pc-windows-msvc

# VPN-клиент (release)
cargo build --release -p aivpn-client --target x86_64-pc-windows-msvc
```

> **Примечание:** GUI собирается в debug-профиле, потому что release-сборка
> не работает на машинах без аппаратного OpenGL 2.0+ (виртуальные машины с
> VirtIO GPU). В debug-режиме wgpu корректно определяет программный рендерер
> (WARP/D3D12). Для машин с нормальным GPU можно собирать `--release`.

### 3. Подготовить пакет

```powershell
$pkg = "releases\aivpn-windows-package"
New-Item -ItemType Directory -Path $pkg -Force

# GUI
Copy-Item "target\x86_64-pc-windows-msvc\debug\aivpn.exe" "$pkg\aivpn.exe"

# VPN-клиент
Copy-Item "target\x86_64-pc-windows-msvc\release\aivpn-client.exe" "$pkg\aivpn-client.exe"

# WinTUN драйвер (скачать с https://www.wintun.net если нет)
Copy-Item "path\to\wintun.dll" "$pkg\wintun.dll"
```

### 4. Собрать инсталлятор

```powershell
cd windows-installer
powershell -ExecutionPolicy Bypass -File .\build-installer.ps1
```

Скрипт:
1. Читает версию из корневого `Cargo.toml`
2. Копирует бинарники и иконку во временную директорию `C:\AIVPNBUILD\stage`
3. Вызывает `makensis` с параметрами версии
4. Сохраняет результат в `releases\aivpn-windows-installer.exe`

### 5. Проверить

```powershell
# Тихая установка
Start-Process "releases\aivpn-windows-installer.exe" -ArgumentList '/S' -Wait

# Проверить файлы
Get-ChildItem "C:\Program Files\AIVPN"

# Проверить ярлык
$sh = New-Object -ComObject WScript.Shell
$lnk = $sh.CreateShortcut("$env:PUBLIC\Desktop\AIVPN.lnk")
$lnk.TargetPath    # → C:\Program Files\AIVPN\aivpn.exe
$lnk.IconLocation  # → C:\Program Files\AIVPN\aivpn.exe,0

# Запустить
Start-Process "C:\Program Files\AIVPN\aivpn.exe"
```

## Что устанавливает инсталлятор

- `C:\Program Files\AIVPN\aivpn.exe` — GUI-приложение
- `C:\Program Files\AIVPN\aivpn-client.exe` — VPN-клиент
- `C:\Program Files\AIVPN\wintun.dll` — WinTUN драйвер
- `C:\Program Files\AIVPN\uninstall-aivpn.exe` — деинсталлятор
- Ярлык на рабочем столе и в меню «Пуск»
- Запись в реестре `HKLM\...\Uninstall\AIVPN`

## Тихая установка / удаление

```powershell
# Установка
aivpn-windows-installer.exe /S

# Удаление
"C:\Program Files\AIVPN\uninstall-aivpn.exe" /S
```
