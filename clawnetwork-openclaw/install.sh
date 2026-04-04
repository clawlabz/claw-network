#!/usr/bin/env bash
# ClawNetwork OpenClaw Plugin — One-line Installer
#
# Usage:
#   curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/install.sh | bash
#
# Custom OpenClaw directory (for named profiles like ~/.openclaw-myprofile):
#   curl -sSf .../install.sh | bash -s ~/.openclaw-myprofile
#
# What this does:
#   1. Downloads the latest plugin from npm (no ClawHub, no rate limits)
#   2. Installs to <openclaw-dir>/extensions/clawnetwork/
#   3. Registers the plugin in <openclaw-dir>/openclaw.json
#   4. Adds "clawnetwork" to the plugins allow list
#
# Safe to re-run — updates existing installation in place.
# Your wallet and chain data are never touched.

set -euo pipefail

PLUGIN_ID="clawnetwork"
NPM_PACKAGE="@clawlabz/clawnetwork"

# First positional arg = custom openclaw dir; fallback to env var; fallback to default
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
fail()  { printf "%s[clawnetwork]%s %s\n" "$RED" "$NC" "$1" >&2; exit 1; }

# --- Pre-checks ---

command -v npm >/dev/null 2>&1 || fail "npm is required but not found. Install Node.js first: https://nodejs.org"
command -v node >/dev/null 2>&1 || fail "node is required but not found. Install Node.js first: https://nodejs.org"

if [ ! -d "${OPENCLAW_DIR}" ]; then
  warn "${OPENCLAW_DIR} not found. Creating directory structure..."
  mkdir -p "${OPENCLAW_DIR}/extensions"
fi

# --- Download from npm ---

info "Downloading latest ${NPM_PACKAGE} from npm..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "${TMPDIR}"' EXIT

cd "${TMPDIR}"
npm pack "${NPM_PACKAGE}@latest" --silent 2>/dev/null || fail "Failed to download from npm. Check your network connection."

TARBALL=$(ls clawlabz-clawnetwork-*.tgz 2>/dev/null | head -1)
[ -n "${TARBALL}" ] || fail "Downloaded tarball not found"

VERSION=$(echo "${TARBALL}" | sed 's/clawlabz-clawnetwork-//;s/\.tgz//')
info "Downloaded version: ${VERSION}"

# --- Detect install vs update ---

IS_UPDATE=false
OLD_VERSION=""
if [ -f "${EXTENSIONS_DIR}/package.json" ]; then
  IS_UPDATE=true
  OLD_VERSION=$(node -e "try{console.log(require('${EXTENSIONS_DIR}/package.json').version)}catch{}" 2>/dev/null || true)
fi

# --- Install ---

if [ "${IS_UPDATE}" = true ]; then
  info "Updating from v${OLD_VERSION} to v${VERSION}..."
else
  info "Installing to ${EXTENSIONS_DIR}/..."
fi
mkdir -p "${EXTENSIONS_DIR}"

tar xzf "${TARBALL}"

cp package/index.ts "${EXTENSIONS_DIR}/"
cp package/openclaw.plugin.json "${EXTENSIONS_DIR}/"
cp package/package.json "${EXTENSIONS_DIR}/"
cp package/README.md "${EXTENSIONS_DIR}/" 2>/dev/null || true
if [ -d package/skills ]; then
  mkdir -p "${EXTENSIONS_DIR}/skills"
  cp -r package/skills/* "${EXTENSIONS_DIR}/skills/"
fi

ok "Plugin files installed"

# --- Register in openclaw.json ---

info "Updating ${CONFIG_FILE}..."

if [ ! -f "${CONFIG_FILE}" ]; then
  cat > "${CONFIG_FILE}" << 'INITJSON'
{
  "plugins": {
    "entries": {},
    "allow": []
  }
}
INITJSON
fi

node -e "
const fs = require('fs');
const cfgPath = '${CONFIG_FILE}';
let cfg = {};
try { cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8')); } catch {}

if (!cfg.plugins) cfg.plugins = {};
if (!cfg.plugins.entries) cfg.plugins.entries = {};
if (!cfg.plugins.allow) cfg.plugins.allow = [];

if (!cfg.plugins.entries['${PLUGIN_ID}']) {
  cfg.plugins.entries['${PLUGIN_ID}'] = {
    enabled: true,
    config: {
      network: 'mainnet',
      autoStart: true,
      autoDownload: true,
      autoRegisterAgent: true,
      rpcPort: 9710,
      p2pPort: 9711,
      syncMode: 'light',
      healthCheckSeconds: 30,
      uiPort: 19877
    }
  };
} else {
  cfg.plugins.entries['${PLUGIN_ID}'].enabled = true;
}

if (!cfg.plugins.allow.includes('${PLUGIN_ID}')) {
  cfg.plugins.allow.push('${PLUGIN_ID}');
}

fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n');
"

ok "Plugin registered in config"

# --- Done ---

echo ""
if [ "${IS_UPDATE}" = true ]; then
  ok "ClawNetwork plugin updated: v${OLD_VERSION} -> v${VERSION}"
  echo ""
  info "Restart your Gateway to apply the update:"
  printf "\n  %sopenclaw gateway restart%s\n\n" "$CYAN" "$NC"
  info "Your wallet, chain data, and config are unchanged."
else
  ok "ClawNetwork plugin v${VERSION} installed successfully!"
  echo ""
  info "Restart your OpenClaw Gateway to activate the plugin:"
  printf "\n  %sopenclaw gateway restart%s\n\n" "$CYAN" "$NC"
  info "After restart, the plugin will automatically:"
  echo "  1. Download the claw-node binary (SHA256 verified)"
  echo "  2. Start a light node and join mainnet"
  echo "  3. Generate a wallet (if first time)"
  echo "  4. Register your Agent and Miner identity on-chain"
  echo "  5. Begin mining and earning rewards"
fi
echo ""
printf "%s[clawnetwork]%s Dashboard:  %shttp://127.0.0.1:19877%s\n" "$CYAN" "$NC" "$CYAN" "$NC"
printf "%s[clawnetwork]%s Status:     %sopenclaw clawnetwork status%s\n" "$CYAN" "$NC" "$CYAN" "$NC"
echo ""
printf "%s[clawnetwork]%s To uninstall: %scurl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/clawnetwork-openclaw/uninstall.sh | bash%s\n" "$CYAN" "$NC" "$CYAN" "$NC"
