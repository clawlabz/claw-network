#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork Node Installer
# Usage: curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.sh | bash

REPO="clawlabz/claw-network"
VERSION="${CLAW_VERSION:-latest}"
INSTALL_DIR="${CLAW_INSTALL_DIR:-$HOME/.clawnetwork/bin}"
DATA_DIR="${CLAW_DATA_DIR:-$HOME/.clawnetwork}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }
err()   { echo -e "${RED}Error:${NC} $*" >&2; exit 1; }

echo ""
echo "  ╔═══════════════════════════════════╗"
echo "  ║   ClawNetwork Node Installer      ║"
echo "  ║   AI Agent Blockchain             ║"
echo "  ╚═══════════════════════════════════╝"
echo ""

# Detect OS and arch
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)   PLATFORM="linux" ;;
  Darwin)  PLATFORM="macos" ;;
  *)       err "Unsupported OS: $OS. Use Docker instead: docker run clawlabz/claw-node" ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_NAME="x86_64" ;;
  aarch64|arm64) ARCH_NAME="aarch64" ;;
  *)       err "Unsupported architecture: $ARCH" ;;
esac

TARGET="${PLATFORM}-${ARCH_NAME}"
info "Detected: $PLATFORM $ARCH_NAME"

# Resolve version
if [ "$VERSION" = "latest" ]; then
  info "Fetching latest version..."
  VERSION=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
  if [ -z "$VERSION" ]; then
    err "Could not determine latest version. Set CLAW_VERSION=v0.1.0 manually."
  fi
fi

info "Version:     $VERSION"
info "Install dir: $INSTALL_DIR"
info "Data dir:    $DATA_DIR"
echo ""

DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/claw-node-${TARGET}.tar.gz"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/${VERSION}/SHA256SUMS"

info "Downloading claw-node for ${TARGET}..."
mkdir -p "$INSTALL_DIR"

if command -v curl &>/dev/null; then
  HTTP_CODE=$(curl -sSfL -w '%{http_code}' -o /tmp/claw-node.tar.gz "$DOWNLOAD_URL" 2>/dev/null || true)
  if [ "$HTTP_CODE" != "200" ] && [ "$HTTP_CODE" != "302" ]; then
    err "Download failed (HTTP $HTTP_CODE). Check version and platform: $DOWNLOAD_URL"
  fi
  # Download checksums
  curl -sSfL -o /tmp/claw-node-SHA256SUMS "$CHECKSUM_URL" 2>/dev/null || true
elif command -v wget &>/dev/null; then
  wget -qO /tmp/claw-node.tar.gz "$DOWNLOAD_URL"
  wget -qO /tmp/claw-node-SHA256SUMS "$CHECKSUM_URL" 2>/dev/null || true
else
  err "curl or wget required"
fi

# SHA256 verification
if [ -f /tmp/claw-node-SHA256SUMS ]; then
  EXPECTED=$(grep "claw-node-${TARGET}.tar.gz" /tmp/claw-node-SHA256SUMS | awk '{print $1}')
  if [ -n "$EXPECTED" ]; then
    info "Verifying SHA256 checksum..."
    if command -v sha256sum &>/dev/null; then
      ACTUAL=$(sha256sum /tmp/claw-node.tar.gz | awk '{print $1}')
    elif command -v shasum &>/dev/null; then
      ACTUAL=$(shasum -a 256 /tmp/claw-node.tar.gz | awk '{print $1}')
    else
      info "No sha256sum/shasum found — skipping verification"
      ACTUAL="$EXPECTED"
    fi
    if [ "$ACTUAL" != "$EXPECTED" ]; then
      rm -f /tmp/claw-node.tar.gz /tmp/claw-node-SHA256SUMS
      err "SHA256 mismatch! Expected: $EXPECTED, Got: $ACTUAL"
    fi
    ok "Checksum verified"
  else
    info "No checksum entry for ${TARGET} — skipping verification"
  fi
  rm -f /tmp/claw-node-SHA256SUMS
else
  info "No SHA256SUMS file found — skipping verification"
fi

tar xzf /tmp/claw-node.tar.gz -C "$INSTALL_DIR"
rm -f /tmp/claw-node.tar.gz

chmod +x "$INSTALL_DIR/claw-node"

# Verify binary works
if ! "$INSTALL_DIR/claw-node" --version &>/dev/null; then
  # Check if it's a glibc issue
  if ldd "$INSTALL_DIR/claw-node" 2>&1 | grep -q "not found"; then
    err "Missing shared libraries. Try the musl (static) build or use Docker instead."
  fi
  err "Binary verification failed. Your system may need additional libraries."
fi

ACTUAL_VERSION=$("$INSTALL_DIR/claw-node" --version 2>&1 || echo "unknown")
ok "Installed: $ACTUAL_VERSION"

# Initialize if first install
if [ ! -f "$DATA_DIR/key.json" ]; then
  info "Initializing node..."
  "$INSTALL_DIR/claw-node" init --data-dir "$DATA_DIR"
  ADDRESS=$("$INSTALL_DIR/claw-node" key show --data-dir "$DATA_DIR" 2>&1)
  ok "Node address: $ADDRESS"
fi

# PATH setup
SHELL_RC=""
if [ -f "$HOME/.zshrc" ]; then
  SHELL_RC="$HOME/.zshrc"
elif [ -f "$HOME/.bashrc" ]; then
  SHELL_RC="$HOME/.bashrc"
fi

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  # Also check /usr/local/bin (some distros like Alibaba Linux don't include it)
  if [[ "$INSTALL_DIR" != "/usr/local/bin" ]] && [[ ":$PATH:" != *":/usr/local/bin:"* ]]; then
    export PATH="/usr/local/bin:$PATH"
  fi

  if [ -n "$SHELL_RC" ]; then
    echo "" >> "$SHELL_RC"
    echo "# ClawNetwork" >> "$SHELL_RC"
    echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
    ok "Added to PATH in $SHELL_RC (restart shell or run: source $SHELL_RC)"
  else
    echo ""
    info "Add to your PATH manually:"
    echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
  fi
fi

echo ""
echo "  ╔═══════════════════════════════════════════════════╗"
echo "  ║  Installation complete!                           ║"
echo "  ╠═══════════════════════════════════════════════════╣"
echo "  ║                                                   ║"
echo "  ║  Quick start:                                     ║"
echo "  ║    claw-node start --single       (solo testnet)  ║"
echo "  ║    claw-node start --bootstrap <addr>  (join net) ║"
echo "  ║                                                   ║"
echo "  ║  Useful commands:                                 ║"
echo "  ║    claw-node key show             (your address)  ║"
echo "  ║    claw-node status               (node status)   ║"
echo "  ║    curl localhost:9710/health     (RPC health)    ║"
echo "  ║                                                   ║"
echo "  ║  Docs: https://github.com/${REPO}    ║"
echo "  ╚═══════════════════════════════════════════════════╝"
echo ""
