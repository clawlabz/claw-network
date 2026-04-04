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

# --- Kill stale processes + upgrade binary ---

info "Stopping old node and dashboard processes..."
pkill -f 'claw-node start' 2>/dev/null || true
pkill -f 'ui-server.js' 2>/dev/null || true
sleep 2
# Clean up ALL possible PID/port files (default + custom profile paths)
for DIR in "${OPENCLAW_DIR}" "${HOME}/.openclaw"; do
  rm -f "${DIR}/clawnetwork-ui-port" 2>/dev/null
  rm -f "${DIR}/workspace/clawnetwork/node.pid" 2>/dev/null
  rm -f "${DIR}/workspace/clawnetwork/stop.signal" 2>/dev/null
done

# Check if binary needs download/upgrade
BINARY_PATH="${OPENCLAW_DIR}/bin/claw-node"
DEFAULT_BINARY="${HOME}/.openclaw/bin/claw-node"
CURRENT_BIN=""
if [ -f "${BINARY_PATH}" ]; then CURRENT_BIN="${BINARY_PATH}"
elif [ -f "${DEFAULT_BINARY}" ]; then CURRENT_BIN="${DEFAULT_BINARY}"
fi

LATEST_VER=$(curl -sf https://api.github.com/repos/clawlabz/claw-network/releases/latest 2>/dev/null | grep '"tag_name"' | sed 's/.*"v\(.*\)".*/\1/' || true)
CURRENT_VER=""
if [ -n "${CURRENT_BIN}" ]; then
  CURRENT_VER=$("${CURRENT_BIN}" --version 2>/dev/null | awk '{print $2}' || true)
fi

NEED_DOWNLOAD=false
if [ -z "${CURRENT_BIN}" ]; then
  NEED_DOWNLOAD=true
elif [ -n "${LATEST_VER}" ] && [ "${CURRENT_VER}" != "${LATEST_VER}" ]; then
  NEED_DOWNLOAD=true
fi

if [ "${NEED_DOWNLOAD}" = true ] && [ -n "${LATEST_VER}" ]; then
  info "Downloading node binary v${LATEST_VER}..."
  PLATFORM_TARGET=""
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)  PLATFORM_TARGET="linux-x86_64" ;;
    Linux-aarch64) PLATFORM_TARGET="linux-aarch64" ;;
    Darwin-arm64)  PLATFORM_TARGET="macos-aarch64" ;;
    Darwin-x86_64) PLATFORM_TARGET="macos-x86_64" ;;
  esac
  if [ -n "${PLATFORM_TARGET}" ]; then
    DL_URL="https://github.com/clawlabz/claw-network/releases/download/v${LATEST_VER}/claw-node-${PLATFORM_TARGET}.tar.gz"
    BIN_TMP=$(mktemp -d)
    DEST="${CURRENT_BIN:-${DEFAULT_BINARY}}"
    if curl -sSfL -o "${BIN_TMP}/claw-node.tar.gz" "${DL_URL}" 2>/dev/null; then
      tar xzf "${BIN_TMP}/claw-node.tar.gz" -C "${BIN_TMP}"
      mkdir -p "$(dirname "${DEST}")"
      cp "${BIN_TMP}/claw-node" "${DEST}"
      chmod +x "${DEST}"
      ok "Node binary: v${LATEST_VER}"
    else
      warn "Failed to download binary (will be downloaded on gateway start)"
    fi
    rm -rf "${BIN_TMP}"
  fi
else
  ok "Node binary: v${CURRENT_VER:-unknown}"
fi

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
