# ClawNetwork Node Installer for Windows
# Usage: irm https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "clawlabz/claw-network"
$Version = if ($env:CLAW_VERSION) { $env:CLAW_VERSION } else { "latest" }
$InstallDir = if ($env:CLAW_INSTALL_DIR) { $env:CLAW_INSTALL_DIR } else { "$env:USERPROFILE\.clawnetwork\bin" }
$DataDir = if ($env:CLAW_DATA_DIR) { $env:CLAW_DATA_DIR } else { "$env:USERPROFILE\.clawnetwork" }

Write-Host ""
Write-Host "  +===================================+" -ForegroundColor Cyan
Write-Host "  |   ClawNetwork Node Installer      |" -ForegroundColor Cyan
Write-Host "  |   AI Agent Blockchain             |" -ForegroundColor Cyan
Write-Host "  +===================================+" -ForegroundColor Cyan
Write-Host ""

# Check architecture
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -ne "AMD64") {
    # Fallback detection
    try {
        $Arch2 = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {
        $Arch2 = ""
    }
    if ($Arch -ne "AMD64" -and $Arch2 -ne "X64") {
        Write-Host "Error: Only x86_64 (AMD64) Windows is supported. Got: $Arch / $Arch2" -ForegroundColor Red
        exit 1
    }
}

$Target = "windows-x86_64"
Write-Host "==> Detected: Windows x86_64" -ForegroundColor Cyan

# Resolve version
if ($Version -eq "latest") {
    Write-Host "==> Fetching latest version..." -ForegroundColor Cyan
    $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -ErrorAction Stop
    $Version = $Release.tag_name
}

Write-Host "==> Version:     $Version" -ForegroundColor Cyan
Write-Host "==> Install dir: $InstallDir" -ForegroundColor Cyan
Write-Host "==> Data dir:    $DataDir" -ForegroundColor Cyan
Write-Host ""

$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/claw-node-$Target.zip"
$TempZip = "$env:TEMP\claw-node.zip"

# Download
Write-Host "==> Downloading claw-node..." -ForegroundColor Cyan
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
try {
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZip -UseBasicParsing
} catch {
    Write-Host "Error: Download failed. Check version: $DownloadUrl" -ForegroundColor Red
    exit 1
}

# Extract
Expand-Archive -Path $TempZip -DestinationPath $InstallDir -Force
Remove-Item $TempZip -Force

# Verify
$Binary = Join-Path $InstallDir "claw-node.exe"
if (-not (Test-Path $Binary)) {
    Write-Host "Error: Binary not found after extraction" -ForegroundColor Red
    exit 1
}

$ActualVersion = & $Binary --version 2>&1
Write-Host "==> Installed: $ActualVersion" -ForegroundColor Green

# Initialize
$KeyFile = Join-Path $DataDir "key.json"
if (-not (Test-Path $KeyFile)) {
    Write-Host "==> Initializing node..." -ForegroundColor Cyan
    & $Binary init --data-dir $DataDir
    $Address = & $Binary key show --data-dir $DataDir 2>&1
    Write-Host "==> Node address: $Address" -ForegroundColor Green
}

# Add to PATH
$CurrentPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($CurrentPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$CurrentPath", "User")
    $env:Path = "$InstallDir;$env:Path"
    Write-Host "==> Added to user PATH (restart terminal to take effect)" -ForegroundColor Green
}

Write-Host ""
Write-Host "  +===================================================+" -ForegroundColor Green
Write-Host "  |  Installation complete!                            |" -ForegroundColor Green
Write-Host "  |                                                    |" -ForegroundColor Green
Write-Host "  |  Quick start:                                      |" -ForegroundColor Green
Write-Host "  |    claw-node start --single       (solo testnet)   |" -ForegroundColor Green
Write-Host "  |    claw-node start --bootstrap <addr>  (join net)  |" -ForegroundColor Green
Write-Host "  |                                                    |" -ForegroundColor Green
Write-Host "  |  Useful commands:                                  |" -ForegroundColor Green
Write-Host "  |    claw-node key show             (your address)   |" -ForegroundColor Green
Write-Host "  |    claw-node status               (node status)    |" -ForegroundColor Green
Write-Host "  +===================================================+" -ForegroundColor Green
Write-Host ""
