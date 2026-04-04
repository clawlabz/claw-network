#!/usr/bin/env bash
# ClawNetwork OpenClaw Plugin — Uninstaller
#
# Usage:
#   curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/uninstall.sh | bash
#
# Custom OpenClaw directory:
#   curl -sSf .../uninstall.sh | bash -s ~/.openclaw-myprofile
#
# What this does:
#   1. Removes plugin files from <openclaw-dir>/extensions/clawnetwork/
#   2. Disables the plugin in <openclaw-dir>/openclaw.json (config preserved)
#
# What is NOT removed (your data is safe):
#   - Wallet: <openclaw-dir>/workspace/clawnetwork/wallet.json
#   - Chain data: ~/.clawnetwork/
#   - Node binary: <openclaw-dir>/bin/claw-node
#   - Node logs: <openclaw-dir>/workspace/clawnetwork/node.log

set -euo pipefail

PLUGIN_ID="clawnetwork"

OPENCLAW_DIR="${1:-${OPENCLAW_DIR:-${HOME}/.openclaw}}"

EXTENSIONS_DIR="${OPENCLAW_DIR}/extensions/${PLUGIN_ID}"
CONFIG_FILE="${OPENCLAW_DIR}/openclaw.json"

# Colors (using $'...' for proper escape interpretation)
if [ -t 1 ] || [ -t 0 ]; then
  GREEN=$'\033[0;32m'
  YELLOW=$'\033[1;33m'
  CYAN=$'\033[0;36m'
  RED=$'\033[0;31m'
  NC=$'\033[0m'
else
  GREEN='' YELLOW='' CYAN='' RED='' NC=''
fi

info()  { printf "%s[clawnetwork]%s %s\n" "$CYAN" "$NC" "$1"; }
ok()    { printf "%s[clawnetwork]%s %s\n" "$GREEN" "$NC" "$1"; }
warn()  { printf "%s[clawnetwork]%s %s\n" "$YELLOW" "$NC" "$1"; }

# --- Stop node if running ---

info "Stopping node (if running)..."
pkill -f 'claw-node start' 2>/dev/null || true

UI_PID_FILE="${OPENCLAW_DIR}/workspace/clawnetwork/ui-server.pid"
if [ -f "${UI_PID_FILE}" ]; then
  kill "$(cat "${UI_PID_FILE}")" 2>/dev/null || true
  rm -f "${UI_PID_FILE}"
fi

rm -f "${OPENCLAW_DIR}/clawnetwork-ui-port" 2>/dev/null || true

# --- Remove plugin files ---

if [ -d "${EXTENSIONS_DIR}" ]; then
  rm -rf "${EXTENSIONS_DIR}"
  ok "Removed plugin files: ${EXTENSIONS_DIR}/"
else
  warn "Plugin directory not found (already removed?)"
fi

# --- Disable in config ---

if [ -f "${CONFIG_FILE}" ] && command -v node >/dev/null 2>&1; then
  node -e "
const fs = require('fs');
const cfgPath = '${CONFIG_FILE}';
try {
  const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  if (cfg.plugins && cfg.plugins.entries && cfg.plugins.entries['${PLUGIN_ID}']) {
    cfg.plugins.entries['${PLUGIN_ID}'].enabled = false;
  }
  if (cfg.plugins && Array.isArray(cfg.plugins.allow)) {
    cfg.plugins.allow = cfg.plugins.allow.filter(p => p !== '${PLUGIN_ID}');
  }
  fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n');
} catch {}
"
  ok "Plugin disabled in config"
fi

# --- Done ---

echo ""
ok "ClawNetwork plugin uninstalled."
echo ""
info "The following data was preserved (delete manually if needed):"
printf "  Wallet:     %s${OPENCLAW_DIR}/workspace/clawnetwork/wallet.json%s\n" "$CYAN" "$NC"
printf "  Chain data: %s~/.clawnetwork/%s\n" "$CYAN" "$NC"
printf "  Binary:     %s${OPENCLAW_DIR}/bin/claw-node%s\n" "$CYAN" "$NC"
printf "  Logs:       %s${OPENCLAW_DIR}/workspace/clawnetwork/node.log%s\n" "$CYAN" "$NC"
echo ""
printf "%s[clawnetwork]%s Restart your Gateway: %sopenclaw gateway restart%s\n" "$CYAN" "$NC" "$CYAN" "$NC"
echo ""
printf "%s[clawnetwork]%s To reinstall: %scurl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/install.sh | bash%s\n" "$CYAN" "$NC" "$CYAN" "$NC"
