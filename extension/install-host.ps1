# PowerShell script to register Kosmos Download Manager Native Messaging Host for Edge, Chrome, and Brave
[CmdletBinding()]
param (
    [string]$HostExecutablePath = ""
)

$ErrorActionPreference = "Stop"

Write-Host "=== Kosmos Download Manager Native Host Installer ===" -ForegroundColor Cyan

# 1. Determine Host Executable Path
if ([string]::IsNullOrWhiteSpace($HostExecutablePath)) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $rootDir = Split-Path -Parent $scriptDir
    $candidates = @(
        (Join-Path $rootDir "target\release\kosmos-download-manager.exe"),
        (Join-Path $rootDir "target\debug\kosmos-download-manager.exe"),
        (Join-Path $rootDir "target\release\kosmos-downloader.exe"),
        (Join-Path $rootDir "target\debug\kosmos-downloader.exe"),
        (Join-Path $rootDir "target\release\kosmos-native-host.exe"),
        (Join-Path $rootDir "target\debug\kosmos-native-host.exe"),
        (Join-Path $scriptDir "kosmos-download-manager.exe"),
        (Join-Path $scriptDir "kosmos-downloader.exe")
    )
    foreach ($cand in $candidates) {
        if (Test-Path $cand) {
            $HostExecutablePath = (Resolve-Path $cand).Path
            break
        }
    }
}

if (-not (Test-Path $HostExecutablePath)) {
    Write-Host "Could not find built executable automatically. Building debug target..." -ForegroundColor Yellow
    Push-Location (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path))
    cargo build
    Pop-Location
    $HostExecutablePath = (Resolve-Path (Join-Path $rootDir "target\debug\kosmos-download-manager.exe")).Path
}

Write-Host "Target Executable: $HostExecutablePath" -ForegroundColor Green

# 2. Register Host via binary if supported
try {
    & $HostExecutablePath --register
    Write-Host "Native host registered successfully via executable." -ForegroundColor Green
} catch {
    Write-Host "Falling back to direct registry installation..." -ForegroundColor Yellow

    $localAppData = [Environment]::GetFolderPath("LocalApplicationData")
    $kosmosDir = Join-Path $localAppData "Kosmos Download Manager"
    if (-not (Test-Path $kosmosDir)) {
        New-Item -ItemType Directory -Path $kosmosDir -Force | Out-Null
    }

    $manifestPath = Join-Path $kosmosDir "com.kosmos.downloader.json"
    $manifestJson = @{
        "name" = "com.kosmos.downloader"
        "description" = "Kosmos Download Manager Native Messaging Host"
        "path" = $HostExecutablePath
        "type" = "stdio"
        "allowed_origins" = @(
            "chrome-extension://ghnbdddbpdglebhbgiaffnkeioomhfkn/",
            "chrome-extension://ngpampappnmepgilojfohadhhmbhlaek/"
        )
    } | ConvertTo-Json -Depth 5

    Set-Content -Path $manifestPath -Value $manifestJson -Encoding UTF8

    $regKeys = @(
        "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.kosmos.downloader",
        "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\com.kosmos.downloader",
        "HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.kosmos.downloader"
    )

    foreach ($key in $regKeys) {
        if (-not (Test-Path $key)) {
            New-Item -Path $key -Force | Out-Null
        }
        Set-ItemProperty -Path $key -Name "(Default)" -Value $manifestPath
        Write-Host "Registered in: $key" -ForegroundColor Gray
    }

    Write-Host "Native Messaging Host manifest created at: $manifestPath" -ForegroundColor Green
}

Write-Host "`nExtension ID: ghnbdddbpdglebhbgiaffnkeioomhfkn" -ForegroundColor Cyan
Write-Host "To install the extension:" -ForegroundColor White
Write-Host "1. Open edge://extensions or chrome://extensions"
Write-Host "2. Enable 'Developer mode'"
Write-Host "3. Click 'Load unpacked' and select the 'extension' directory"
Write-Host "`nInstallation complete!" -ForegroundColor Green
