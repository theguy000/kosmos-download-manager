# Registers the Kosmos Download Manager native messaging host for Chrome, Edge, Brave and Chromium.
# The executable owns the manifest, allowed origins and registry keys (src/host/registry.rs);
# this script only locates (or builds) it and calls `--register`.
[CmdletBinding()]
param (
    [string]$HostExecutablePath = "",
    # Extra extension IDs to trust, e.g. an unpacked build with a different ID.
    [string[]]$ExtensionId = @()
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$rootDir = Split-Path -Parent $scriptDir

Write-Host "=== Kosmos Download Manager Native Host Installer ===" -ForegroundColor Cyan

# 1. Determine host executable path
if ([string]::IsNullOrWhiteSpace($HostExecutablePath)) {
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
    $HostExecutablePath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1

    if (-not $HostExecutablePath) {
        Write-Host "Could not find a built executable. Building debug target..." -ForegroundColor Yellow
        Push-Location $rootDir
        try {
            cargo build
            if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
        } finally {
            Pop-Location
        }
        $HostExecutablePath = Join-Path $rootDir "target\debug\kosmos-download-manager.exe"
    }
}

if (-not (Test-Path $HostExecutablePath)) {
    throw "Host executable not found: $HostExecutablePath"
}
$HostExecutablePath = (Resolve-Path $HostExecutablePath).Path
Write-Host "Target Executable: $HostExecutablePath" -ForegroundColor Green

# 2. Register via the binary. Start-Process -Wait is used because release builds use the
#    Windows GUI subsystem, which `&` does not wait for and gives no reliable exit code.
$registerArgs = @("--register")
foreach ($id in $ExtensionId) {
    $registerArgs += @("--extension-id", $id)
}
$process = Start-Process -FilePath $HostExecutablePath -ArgumentList $registerArgs -Wait -PassThru
if ($process.ExitCode -ne 0) {
    throw "Native host registration failed with exit code $($process.ExitCode)"
}
Write-Host "Native host registered successfully." -ForegroundColor Green

Write-Host "`nExtension ID: ghnbdddbpdglebhbgiaffnkeioomhfkn" -ForegroundColor Cyan
Write-Host "To install the extension:" -ForegroundColor White
Write-Host "1. Open edge://extensions or chrome://extensions"
Write-Host "2. Enable 'Developer mode'"
Write-Host "3. Click 'Load unpacked' and select the 'extension' directory"
Write-Host "`nInstallation complete!" -ForegroundColor Green
