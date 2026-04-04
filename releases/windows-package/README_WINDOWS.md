# AIVPN Windows Test Guide

This folder contains tooling to run a real smoke test in a Windows VM.

## Package Contents

- `aivpn-client.exe` - Windows client binary
- `wintun.dll` - Wintun runtime (must be next to the exe)
- `smoke-test.ps1` - automated test script

## Build Package On macOS/Linux

Run from repository root:

```bash
./build-windows-package.sh
```

It will generate:

- `releases/windows-package/` (unpacked files)
- `releases/aivpn-windows-package.zip`

## Real Test In Windows VM

1. Copy `releases/aivpn-windows-package.zip` into your Windows VM.
2. Unzip to a folder, for example `C:\aivpn`.
3. Open PowerShell as Administrator.
4. Run:

```powershell
Set-ExecutionPolicy -Scope Process Bypass -Force
cd C:\aivpn
.\smoke-test.ps1 -ConnectionKey "aivpn://..." -FullTunnel
```

## What The Smoke Test Checks

- Administrator privileges
- Presence and signatures of `aivpn-client.exe` and `wintun.dll`
- Client process starts correctly
- Basic connectivity check while client is running
- Graceful stop and log output summary

## Note About EXE Signature

`aivpn-client.exe` may be unsigned unless you sign it with an Authenticode certificate.
Unsigned exe can trigger SmartScreen warnings. This is expected for self-built binaries.
