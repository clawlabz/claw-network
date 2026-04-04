# ClawNetwork OpenClaw Plugin — Uninstaller (Windows)
#
# Usage:
#   irm https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/uninstall.ps1 | iex
#
# Custom OpenClaw directory:
#   & { $env:OPENCLAW_DIR="$env:USERPROFILE\.openclaw-ludis"; irm .../uninstall.ps1 | iex }

$ErrorActionPreference = "Stop"

$PluginId = "clawnetwork"
$OpenClawDir = if ($env:OPENCLAW_DIR) { $env:OPENCLAW_DIR } else { Join-Path $env:USERPROFILE ".openclaw" }
$ExtensionsDir = Join-Path $OpenClawDir "extensions\$PluginId"
$ConfigFile = Join-Path $OpenClawDir "openclaw.json"

function Write-Info($msg)  { Write-Host "[clawnetwork] $msg" -ForegroundColor Cyan }
function Write-Ok($msg)    { Write-Host "[clawnetwork] $msg" -ForegroundColor Green }
function Write-Warn($msg)  { Write-Host "[clawnetwork] $msg" -ForegroundColor Yellow }

# --- Stop node if running ---

Write-Info "Stopping node (if running)..."
Get-Process -Name "claw-node" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$UiPidFile = Join-Path $OpenClawDir "workspace\clawnetwork\ui-server.pid"
if (Test-Path $UiPidFile) {
    try {
        $pid = [int](Get-Content $UiPidFile)
        Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
    } catch {}
    Remove-Item $UiPidFile -Force -ErrorAction SilentlyContinue
}

Remove-Item (Join-Path $OpenClawDir "clawnetwork-ui-port") -Force -ErrorAction SilentlyContinue

# --- Remove plugin files ---

if (Test-Path $ExtensionsDir) {
    Remove-Item -Recurse -Force $ExtensionsDir
    Write-Ok "Removed plugin files: $ExtensionsDir"
} else {
    Write-Warn "Plugin directory not found (already removed?)"
}

# --- Disable in config ---

if ((Test-Path $ConfigFile) -and (Get-Command node -ErrorAction SilentlyContinue)) {
    $NodeScript = @"
const fs = require('fs');
const cfgPath = '$($ConfigFile -replace '\\', '\\\\')';
try {
  const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  if (cfg.plugins && cfg.plugins.entries && cfg.plugins.entries['$PluginId']) {
    cfg.plugins.entries['$PluginId'].enabled = false;
  }
  if (cfg.plugins && Array.isArray(cfg.plugins.allow)) {
    cfg.plugins.allow = cfg.plugins.allow.filter(p => p !== '$PluginId');
  }
  fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n');
} catch {}
"@
    node -e $NodeScript
    Write-Ok "Plugin disabled in config"
}

# --- Done ---

Write-Host ""
Write-Ok "ClawNetwork plugin uninstalled."
Write-Host ""
Write-Info "The following data was preserved (delete manually if needed):"
Write-Host "  Wallet:     ~/.openclaw/workspace/clawnetwork/wallet.json"
Write-Host "  Chain data: ~/.clawnetwork/"
Write-Host "  Binary:     ~/.openclaw/bin/claw-node.exe"
Write-Host "  Logs:       ~/.openclaw/workspace/clawnetwork/node.log"
Write-Host ""
Write-Info "Restart your Gateway: openclaw gateway restart"
Write-Host ""
Write-Info "To reinstall: irm https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/install.ps1 | iex"
