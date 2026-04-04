# ClawNetwork OpenClaw Plugin — One-line Installer (Windows)
# Usage: irm https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/install.ps1 | iex
#
# What this does:
#   1. Downloads the latest plugin from npm (no ClawHub, no rate limits)
#   2. Installs to ~/.openclaw/extensions/clawnetwork/
#   3. Registers the plugin in ~/.openclaw/openclaw.json
#   4. Adds "clawnetwork" to the plugins allow list
#
# Safe to re-run — updates existing installation in place.
# Your wallet and chain data are never touched.

$ErrorActionPreference = "Stop"

$PluginId = "clawnetwork"
$NpmPackage = "@clawlabz/clawnetwork"
$OpenClawDir = Join-Path $env:USERPROFILE ".openclaw"
$ExtensionsDir = Join-Path $OpenClawDir "extensions\$PluginId"
$ConfigFile = Join-Path $OpenClawDir "openclaw.json"

function Write-Info($msg)  { Write-Host "[clawnetwork] $msg" -ForegroundColor Cyan }
function Write-Ok($msg)    { Write-Host "[clawnetwork] $msg" -ForegroundColor Green }
function Write-Warn($msg)  { Write-Host "[clawnetwork] $msg" -ForegroundColor Yellow }
function Write-Fail($msg)  { Write-Host "[clawnetwork] $msg" -ForegroundColor Red; exit 1 }

# --- Pre-checks ---

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    Write-Fail "npm is required but not found. Install Node.js first: https://nodejs.org"
}
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Fail "node is required but not found. Install Node.js first: https://nodejs.org"
}

if (-not (Test-Path $OpenClawDir)) {
    Write-Warn "~/.openclaw/ not found. Creating directory structure..."
    New-Item -ItemType Directory -Path (Join-Path $OpenClawDir "extensions") -Force | Out-Null
}

# --- Detect install vs update ---

$IsUpdate = $false
$OldVersion = ""
$PkgJsonPath = Join-Path $ExtensionsDir "package.json"
if (Test-Path $PkgJsonPath) {
    $IsUpdate = $true
    try {
        $OldVersion = (Get-Content $PkgJsonPath | ConvertFrom-Json).version
    } catch {}
}

# --- Download from npm ---

Write-Info "Downloading latest $NpmPackage from npm..."

$TmpDir = Join-Path $env:TEMP "clawnetwork-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    Push-Location $TmpDir
    npm pack "$NpmPackage@latest" --silent 2>$null
    if ($LASTEXITCODE -ne 0) { Write-Fail "Failed to download from npm. Check your network connection." }

    $Tarball = Get-ChildItem "clawlabz-clawnetwork-*.tgz" | Select-Object -First 1
    if (-not $Tarball) { Write-Fail "Downloaded tarball not found" }

    $Version = $Tarball.Name -replace 'clawlabz-clawnetwork-', '' -replace '\.tgz$', ''
    Write-Info "Downloaded version: $Version"

    # --- Install ---

    if ($IsUpdate) {
        Write-Info "Updating from v$OldVersion to v$Version..."
    } else {
        Write-Info "Installing to $ExtensionsDir/..."
    }

    New-Item -ItemType Directory -Path $ExtensionsDir -Force | Out-Null

    # Extract tarball
    tar xzf $Tarball.FullName 2>$null
    if (-not (Test-Path "package")) {
        # Fallback: use node to extract if tar not available
        node -e "require('child_process').execSync('tar xzf $($Tarball.Name)', {stdio:'inherit'})"
    }

    # Copy plugin files
    Copy-Item "package\index.ts" $ExtensionsDir -Force
    Copy-Item "package\openclaw.plugin.json" $ExtensionsDir -Force
    Copy-Item "package\package.json" $ExtensionsDir -Force
    if (Test-Path "package\README.md") { Copy-Item "package\README.md" $ExtensionsDir -Force }
    if (Test-Path "package\skills") {
        $SkillsDir = Join-Path $ExtensionsDir "skills"
        New-Item -ItemType Directory -Path $SkillsDir -Force | Out-Null
        Copy-Item "package\skills\*" $SkillsDir -Force
    }

    Write-Ok "Plugin files installed"
} finally {
    Pop-Location
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}

# --- Register in openclaw.json ---

Write-Info "Updating $ConfigFile..."

if (-not (Test-Path $ConfigFile)) {
    '{"plugins":{"entries":{},"allow":[]}}' | Set-Content $ConfigFile -Encoding UTF8
}

$NodeScript = @"
const fs = require('fs');
const cfgPath = '$($ConfigFile -replace '\\', '\\\\')';
let cfg = {};
try { cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8')); } catch {}
if (!cfg.plugins) cfg.plugins = {};
if (!cfg.plugins.entries) cfg.plugins.entries = {};
if (!cfg.plugins.allow) cfg.plugins.allow = [];
if (!cfg.plugins.entries['$PluginId']) {
  cfg.plugins.entries['$PluginId'] = {
    enabled: true,
    config: {
      network: 'mainnet', autoStart: true, autoDownload: true,
      autoRegisterAgent: true, rpcPort: 9710, p2pPort: 9711,
      syncMode: 'light', healthCheckSeconds: 30, uiPort: 19877
    }
  };
} else {
  cfg.plugins.entries['$PluginId'].enabled = true;
}
if (!cfg.plugins.allow.includes('$PluginId')) {
  cfg.plugins.allow.push('$PluginId');
}
fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n');
"@

node -e $NodeScript
Write-Ok "Plugin registered in config"

# --- Done ---

Write-Host ""
if ($IsUpdate) {
    Write-Ok "ClawNetwork plugin updated: v$OldVersion -> v$Version"
    Write-Host ""
    Write-Info "Restart your Gateway to apply the update:"
    Write-Host ""
    Write-Host "  openclaw gateway restart" -ForegroundColor Cyan
    Write-Host ""
    Write-Info "Your wallet, chain data, and config are unchanged."
} else {
    Write-Ok "ClawNetwork plugin v$Version installed successfully!"
    Write-Host ""
    Write-Info "Restart your OpenClaw Gateway to activate the plugin:"
    Write-Host ""
    Write-Host "  openclaw gateway restart" -ForegroundColor Cyan
    Write-Host ""
    Write-Info "After restart, the plugin will automatically:"
    Write-Host "  1. Download the claw-node binary (SHA256 verified)"
    Write-Host "  2. Start a light node and join mainnet"
    Write-Host "  3. Generate a wallet (if first time)"
    Write-Host "  4. Register your Agent and Miner identity on-chain"
    Write-Host "  5. Begin mining and earning rewards"
}
Write-Host ""
Write-Info "Dashboard:  http://127.0.0.1:19877"
Write-Info "Status:     openclaw clawnetwork status"
Write-Host ""
Write-Info "To uninstall: irm https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/uninstall.ps1 | iex"
