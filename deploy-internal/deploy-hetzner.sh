#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork — 内部部署脚本：升级 Hetzner 节点 (mainnet + testnet)
# !! 此脚本包含内部 IP，不得提交到公开仓库 !!
#
# Usage:
#   ./deploy-internal/deploy-hetzner.sh                # bump + CI + 升级两个节点
#   ./deploy-internal/deploy-hetzner.sh --skip-bump    # 重部署当前版本
#   ./deploy-internal/deploy-hetzner.sh --mainnet-only  # 只升级主网
#   ./deploy-internal/deploy-hetzner.sh --testnet-only  # 只升级测试网
#   ./deploy-internal/deploy-hetzner.sh --local         # 本地编译

DEPLOY_HOST="${DEPLOY_HOST:-178.156.162.162}"
DEPLOY_USER="${DEPLOY_USER:-root}"
TARGET="linux-x86_64"
SKIP_BUMP=false
LOCAL_BUILD=false
MAINNET=true
TESTNET=true

for arg in "$@"; do
  case "$arg" in
    --skip-bump)    SKIP_BUMP=true ;;
    --local)        LOCAL_BUILD=true ;;
    --mainnet-only) TESTNET=false ;;
    --testnet-only) MAINNET=false ;;
    --help|-h)
      sed -n '3,10p' "$0"
      exit 0
      ;;
  esac
done

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'
info()  { echo -e "${CYAN}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }
err()   { echo -e "${RED}Error:${NC} $*" >&2; exit 1; }

# Find repo root (claw-node/)
REPO_ROOT="$(cd "$(dirname "$0")/../claw-node" && pwd)"
cd "$REPO_ROOT"

# ── Step 1: Version bump + tag ──
if [ "$SKIP_BUMP" = false ]; then
  CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
  MAJOR=$(echo "$CURRENT" | cut -d. -f1)
  MINOR=$(echo "$CURRENT" | cut -d. -f2)
  PATCH=$(echo "$CURRENT" | cut -d. -f3)
  NEW_PATCH=$((PATCH + 1))
  NEW_VERSION="${MAJOR}.${MINOR}.${NEW_PATCH}"

  info "Bumping version: $CURRENT → $NEW_VERSION"
  sed -i '' "s/version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" Cargo.toml 2>/dev/null \
    || sed -i "s/version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" Cargo.toml

  git add Cargo.toml
  git commit -m "chore: bump version to $NEW_VERSION"
  git tag "v${NEW_VERSION}"
  git push origin main --tags
  ok "Tagged v${NEW_VERSION} and pushed"
else
  NEW_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
  info "Redeploying current version: $NEW_VERSION"
fi

TAG="v${NEW_VERSION}"
ARTIFACT="claw-node-${TARGET}.tar.gz"
LOCAL_PATH="/tmp/${ARTIFACT}"

# ── Step 2: Get binary ──
if [ "$LOCAL_BUILD" = true ]; then
  info "Building locally for ${TARGET}..."
  command -v cross &>/dev/null || err "'cross' not installed. Run: cargo install cross"
  cross build --release --target x86_64-unknown-linux-musl
  tar czf "$LOCAL_PATH" -C target/x86_64-unknown-linux-musl/release claw-node
  ok "Local build complete"
else
  info "Waiting for CI release build (tag: $TAG)..."
  for i in $(seq 1 60); do
    RUN_ID=$(gh run list --workflow=release.yml --limit 5 --json databaseId,headBranch,status \
      -q ".[] | select(.headBranch == \"$TAG\") | .databaseId" | head -1)
    [ -n "$RUN_ID" ] && break
    [ "$i" -eq 60 ] && err "Timed out waiting for CI run"
    sleep 5
  done

  info "Found CI run: $RUN_ID, waiting..."
  gh run watch "$RUN_ID" --exit-status || err "CI build failed"
  ok "CI build passed"

  sleep 5
  gh release download "$TAG" --pattern "$ARTIFACT" --dir /tmp --clobber
  ok "Downloaded to $LOCAL_PATH"
fi

ls -lh "$LOCAL_PATH"

# ── Step 3: Upload ──
info "Uploading to ${DEPLOY_USER}@${DEPLOY_HOST}..."
scp "$LOCAL_PATH" "${DEPLOY_USER}@${DEPLOY_HOST}:/tmp/${ARTIFACT}"
ok "Upload complete"

# ── Step 4: Deploy ──
SERVICES=""
[ "$MAINNET" = true ] && SERVICES="clawnet-mainnet"
[ "$TESTNET" = true ] && SERVICES="$SERVICES clawnet-testnet"

info "Deploying v${NEW_VERSION} → ${DEPLOY_HOST} (${SERVICES})..."

ssh "${DEPLOY_USER}@${DEPLOY_HOST}" "bash -s" << REMOTE
set -euo pipefail

ARTIFACT="${ARTIFACT}"
SERVICES="${SERVICES}"

# CRITICAL: Stop FIRST, then backup. Copying redb while running produces corrupt files.
echo "==> Stopping: \$SERVICES"
for svc in \$SERVICES; do
  systemctl stop \$svc || true
done
sleep 3
# Verify all processes fully stopped
for svc in \$SERVICES; do
  if pgrep -f "\$svc" >/dev/null 2>&1; then
    echo "WARNING: \$svc still running, waiting..."
    sleep 5
    pgrep -f "\$svc" >/dev/null 2>&1 && { echo "ABORT: \$svc won't stop"; exit 1; }
  fi
done

echo "==> Backing up chain data (after clean stop)..."
for svc in \$SERVICES; do
  DB="/opt/\${svc}/.clawnetwork/chain.redb"
  if [ -f "\$DB" ]; then
    BACKUP="/tmp/chain-\${svc}-backup-\$(date +%s).redb"
    cp "\$DB" "\$BACKUP"
    echo "  Backed up \$DB → \$BACKUP (\$(du -h "\$BACKUP" | cut -f1))"
  fi
done

echo "==> Extracting..."
tar xzf /tmp/\${ARTIFACT} -C /tmp/
chmod +x /tmp/claw-node

for svc in \$SERVICES; do
  cp /tmp/claw-node /opt/\${svc}/bin/claw-node
  chown \${svc}:\${svc} /opt/\${svc}/bin/claw-node
  echo "==> Installed → /opt/\${svc}/bin/"
done
rm /tmp/claw-node

echo "==> Version: \$(/opt/clawnet-mainnet/bin/claw-node --version 2>/dev/null || /opt/clawnet-testnet/bin/claw-node --version)"

echo "==> Starting: \$SERVICES"
for svc in \$SERVICES; do
  systemctl start \$svc
done
sleep 8

echo "==> Health:"
DEPLOY_OK=true
if echo "\$SERVICES" | grep -q mainnet; then
  MH=\$(curl -sf http://localhost:9710/health 2>/dev/null || echo FAILED)
  echo "  Mainnet: \$MH"
  echo "\$MH" | grep -q '"height"' || DEPLOY_OK=false
fi
if echo "\$SERVICES" | grep -q testnet; then
  TH=\$(curl -sf http://localhost:9720/health 2>/dev/null || echo FAILED)
  echo "  Testnet: \$TH"
  echo "\$TH" | grep -q '"height"' || DEPLOY_OK=false
fi
if [ "\$DEPLOY_OK" = false ]; then
  echo "==> WARNING: Health check failed! Backups are in /tmp/chain-*-backup-*.redb"
  echo "==> To rollback: systemctl stop <svc> && cp /tmp/chain-<svc>-backup-<ts>.redb /opt/<svc>/.clawnetwork/chain.redb && systemctl start <svc>"
fi
echo "==> Done!"
REMOTE

ok "Deploy complete! v${NEW_VERSION} on ${DEPLOY_HOST}"
echo ""
echo "  journalctl -u clawnet-mainnet -f     # mainnet logs"
echo "  journalctl -u clawnet-testnet -f     # testnet logs"
echo "  curl https://rpc.clawlabz.xyz/health  # mainnet"
echo "  curl https://testnet-rpc.clawlabz.xyz/health  # testnet"
