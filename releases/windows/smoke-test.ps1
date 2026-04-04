param(
    [Parameter(Mandatory = $true)]
    [string]$ConnectionKey,

    [switch]$FullTunnel,

    [int]$WarmupSeconds = 12
)

$ErrorActionPreference = "Stop"

function Write-Step($msg) {
    Write-Host "[STEP] $msg" -ForegroundColor Cyan
}

function Write-Ok($msg) {
    Write-Host "[OK]   $msg" -ForegroundColor Green
}

function Write-Warn($msg) {
    Write-Host "[WARN] $msg" -ForegroundColor Yellow
}

function Write-Fail($msg) {
    Write-Host "[FAIL] $msg" -ForegroundColor Red
}

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run PowerShell as Administrator"
    }
}

function Show-Signature($path, $requiredValid) {
    $sig = Get-AuthenticodeSignature -FilePath $path
    $status = $sig.Status.ToString()
    Write-Host "Signature $path -> $status"

    if ($requiredValid -and $status -ne "Valid") {
        throw "Signature is not valid for required file: $path"
    }

    if (-not $requiredValid -and $status -ne "Valid") {
        Write-Warn "EXE is not Authenticode-signed (SmartScreen warning is expected)."
    }
}

function Get-PublicIp {
    try {
        return (Invoke-RestMethod -Uri "https://api.ipify.org" -TimeoutSec 5)
    }
    catch {
        return "unknown"
    }
}

Write-Step "Checking Administrator privileges"
Assert-Admin
Write-Ok "Running as Administrator"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $root "aivpn-client.exe"
$dll = Join-Path $root "wintun.dll"

Write-Step "Checking required files"
if (-not (Test-Path $exe)) { throw "Missing file: $exe" }
if (-not (Test-Path $dll)) { throw "Missing file: $dll" }
Write-Ok "Files found"

Write-Step "Checking file signatures"
Show-Signature -path $exe -requiredValid:$false
Show-Signature -path $dll -requiredValid:$true
Write-Ok "Signature checks completed"

Write-Step "Starting client"
$args = @("-k", $ConnectionKey)
if ($FullTunnel) {
    $args += "--full-tunnel"
}

$logPath = Join-Path $root "smoke-test-client.log"
if (Test-Path $logPath) { Remove-Item $logPath -Force }

$proc = Start-Process -FilePath $exe -ArgumentList $args -WorkingDirectory $root -PassThru -RedirectStandardOutput $logPath -RedirectStandardError $logPath
Start-Sleep -Seconds $WarmupSeconds

if ($proc.HasExited) {
    Get-Content $logPath -ErrorAction SilentlyContinue | Select-Object -Last 50
    throw "Client exited too early with code $($proc.ExitCode)"
}
Write-Ok "Client process is running (PID=$($proc.Id))"

Write-Step "Running connectivity checks"
$ip = Get-PublicIp
Write-Host "Public IP (while running): $ip"

$dnsCheck = Test-NetConnection -ComputerName 8.8.8.8 -Port 53 -WarningAction SilentlyContinue
if (-not $dnsCheck.TcpTestSucceeded) {
    Write-Warn "TCP 53 check failed (this may be network-policy related)"
} else {
    Write-Ok "TCP connectivity check passed"
}

Write-Step "Stopping client"
try {
    Stop-Process -Id $proc.Id -Force -ErrorAction Stop
    Write-Ok "Client process stopped"
}
catch {
    Write-Warn "Could not stop process cleanly: $($_.Exception.Message)"
}

Write-Step "Tail of client log"
Get-Content $logPath -ErrorAction SilentlyContinue | Select-Object -Last 40

Write-Ok "Smoke test finished"
