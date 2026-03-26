#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork — 全节点部署脚本
# !! 此脚本包含内部 IP，不得提交到公开仓库 !!
#
# Usage:
#   ./deploy-internal/deploy-all.sh <version>        # 部署指定版本到所有节点
#   ./deploy-internal/deploy-all.sh <version> hetzner # 只部署 Hetzner
#   ./deploy-internal/deploy-all.sh <version> aliyun  # 只部署阿里云
#   ./deploy-internal/deploy-all.sh <version> macmini # 只部署 Mac Mini

VERSION="${1:?Usage: deploy-all.sh <version> [hetzner|aliyun|macmini]}"
TARGET="${2:-all}"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'
info()  { echo -e "${CYAN}==>${NC} $*"; }
ok()    { echo -e "${GREEN}  ✓${NC} $*"; }
err()   { echo -e "${RED}  ✗${NC} $*" >&2; }

TAG="v${VERSION}"
LINUX_BIN="/tmp/claw-node-linux-${VERSION}"
MAC_BIN="/tmp/claw-node-mac-${VERSION}"

# ── Download binaries ──
info "Downloading ${TAG} release binaries..."
cd /tmp
gh release download "$TAG" -R clawlabz/claw-network \
  -p "claw-node-linux-x86_64.tar.gz" \
  -p "claw-node-macos-aarch64.tar.gz" --clobber

tar xzf claw-node-linux-x86_64.tar.gz && cp claw-node "$LINUX_BIN"
tar xzf claw-node-macos-aarch64.tar.gz && cp claw-node "$MAC_BIN"
ok "Downloaded linux + macOS binaries"

# ── Helper: deploy to a Linux systemd node ──
deploy_linux() {
  local HOST="$1" NAME="$2" SERVICE="$3"
  info "Deploying to ${NAME} (${HOST})..."

  scp "$LINUX_BIN" "root@${HOST}:/tmp/claw-node-new" || { err "${NAME}: upload failed"; return 1; }

  ssh "root@${HOST}" "bash -s" <<REMOTE
set -euo pipefail
# CRITICAL: Stop FIRST, then backup. Copying redb while running produces corrupt files.
systemctl stop ${SERVICE}
sleep 3
# Verify process is fully stopped
if pgrep -f "${SERVICE}" >/dev/null 2>&1; then
  echo "WARNING: ${SERVICE} still running after stop, waiting..."
  sleep 5
  pgrep -f "${SERVICE}" >/dev/null 2>&1 && { echo "ABORT: process won't stop"; exit 1; }
fi
# Backup after clean stop
DB="/opt/${SERVICE}/.clawnetwork/chain.redb"
BACKUP="/tmp/chain-${SERVICE}-\$(date +%s).redb"
[ -f "\$DB" ] && cp "\$DB" "\$BACKUP" && echo "Backed up chain.redb → \$BACKUP"
# Verify backup is loadable (quick sanity: file size > 0 and has redb magic)
if [ -f "\$BACKUP" ]; then
  MAGIC=\$(xxd -l4 -p "\$BACKUP")
  [ "\$MAGIC" != "72656462" ] && echo "WARNING: backup does not have redb magic header!"
fi
# Replace binary + start
cp /tmp/claw-node-new /opt/${SERVICE}/bin/claw-node
chmod +x /opt/${SERVICE}/bin/claw-node
chown ${SERVICE}:${SERVICE} /opt/${SERVICE}/bin/claw-node
/opt/${SERVICE}/bin/claw-node --version
systemctl start ${SERVICE}
sleep 8
HEALTH=\$(curl -sf http://127.0.0.1:9710/health 2>/dev/null)
if echo "\$HEALTH" | grep -q '"height"'; then
  echo "Health OK: \$HEALTH"
else
  echo "HEALTH CHECK FAILED — consider rollback: cp \$BACKUP \$DB && systemctl start ${SERVICE}"
  exit 1
fi
REMOTE
  ok "${NAME} deployed"
}

# ── Helper: deploy to Mac Mini ──
deploy_macmini() {
  info "Deploying to Mac Mini (192.168.7.23)..."
  sshpass -p 'build' scp -o PubkeyAuthentication=no -o PreferredAuthentications=password \
    "$MAC_BIN" build@192.168.7.23:/tmp/claw-node-new || { err "Mac Mini: upload failed"; return 1; }

  sshpass -p 'build' ssh -o PubkeyAuthentication=no -o PreferredAuthentications=password \
    build@192.168.7.23 "
echo 'build' | sudo -S kill \$(pgrep -f claw-node) 2>/dev/null; sleep 2
echo 'build' | sudo -S cp /tmp/claw-node-new /usr/local/bin/claw-node
echo 'build' | sudo -S xattr -cr /usr/local/bin/claw-node 2>/dev/null
echo 'build' | sudo -S codesign --sign - --force /usr/local/bin/claw-node 2>&1
echo 'build' | sudo -S launchctl bootout system/com.clawlabz.clawnet-mainnet 2>/dev/null; sleep 1
echo 'build' | sudo -S launchctl bootstrap system /Library/LaunchDaemons/com.clawlabz.clawnet-mainnet.plist
sleep 10
curl -sf http://127.0.0.1:9710/health || echo 'HEALTH CHECK FAILED'
"
  ok "Mac Mini deployed"
}

# ── Deploy ──
case "$TARGET" in
  all)
    deploy_linux "178.156.162.162" "Hetzner" "clawnet-mainnet"
    deploy_linux "39.102.144.231" "Aliyun" "clawnet-mainnet"
    deploy_macmini
    ;;
  hetzner) deploy_linux "178.156.162.162" "Hetzner" "clawnet-mainnet" ;;
  aliyun)  deploy_linux "39.102.144.231" "Aliyun" "clawnet-mainnet" ;;
  macmini) deploy_macmini ;;
  *) err "Unknown target: $TARGET (use hetzner|aliyun|macmini|all)"; exit 1 ;;
esac

echo ""
ok "Deploy complete! ${TAG}"
echo "  curl https://rpc.clawlabz.xyz/health"
