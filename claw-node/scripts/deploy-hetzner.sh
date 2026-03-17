#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork — Deploy/upgrade nodes on Hetzner (mainnet + testnet)
#
# Usage:
#   ./scripts/deploy-hetzner.sh                # bump version, tag, CI build, deploy both
#   ./scripts/deploy-hetzner.sh --skip-bump    # redeploy current version
#   ./scripts/deploy-hetzner.sh --mainnet-only # upgrade mainnet only
#   ./scripts/deploy-hetzner.sh --testnet-only # upgrade testnet only
#   ./scripts/deploy-hetzner.sh --local        # build locally (cross-compile)
#
# Environment:
#   DEPLOY_HOST   Server IP (default: 178.156.162.162)

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
      sed -n '3,11p' "$0"
      exit 0
      ;;
  esac
done

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
NC='\033[0m'
info()  { echo -e "${CYAN}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }
warn()  { echo -e "${YELLOW}==>${NC} $*"; }
err()   { echo -e "${RED}Error:${NC} $*" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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

  # Wait for the correct run (matching our tag)
  for i in $(seq 1 60); do
    RUN_ID=$(gh run list --workflow=release.yml --limit 5 --json databaseId,headBranch,status \
      -q ".[] | select(.headBranch == \"$TAG\") | .databaseId" | head -1)
    if [ -n "$RUN_ID" ]; then
      break
    fi
    [ "$i" -eq 60 ] && err "Timed out waiting for CI run to appear"
    sleep 5
  done

  info "Found CI run: $RUN_ID, waiting for completion..."
  gh run watch "$RUN_ID" --exit-status || err "CI build failed"
  ok "CI build passed"

  info "Downloading ${ARTIFACT} from release ${TAG}..."
  # Wait a few seconds for release to be published
  sleep 5
  gh release download "$TAG" --pattern "$ARTIFACT" --dir /tmp --clobber
  ok "Downloaded to $LOCAL_PATH"
fi

ls -lh "$LOCAL_PATH"

# ── Step 3: Upload to server ──
info "Uploading to ${DEPLOY_USER}@${DEPLOY_HOST}..."
scp "$LOCAL_PATH" "${DEPLOY_USER}@${DEPLOY_HOST}:/tmp/${ARTIFACT}"
ok "Upload complete"

# ── Step 4: Deploy on server ──
info "Deploying v${NEW_VERSION} on ${DEPLOY_HOST}..."

# Build the list of services to upgrade
SERVICES=""
[ "$MAINNET" = true ] && SERVICES="clawnet-mainnet"
[ "$TESTNET" = true ] && SERVICES="$SERVICES clawnet-testnet"

ssh "${DEPLOY_USER}@${DEPLOY_HOST}" "bash -s" << REMOTE
set -euo pipefail

ARTIFACT="${ARTIFACT}"
SERVICES="${SERVICES}"

echo "==> Stopping services: \$SERVICES"
for svc in \$SERVICES; do
  systemctl stop \$svc || true
done
sleep 1

echo "==> Extracting binary..."
tar xzf /tmp/\${ARTIFACT} -C /tmp/
chmod +x /tmp/claw-node

# Install to each service
for svc in \$SERVICES; do
  USER_DIR="/opt/\${svc}"
  cp /tmp/claw-node \${USER_DIR}/bin/claw-node
  chown \${svc}:\${svc} \${USER_DIR}/bin/claw-node
  echo "==> Installed to \${USER_DIR}/bin/claw-node"
done

rm /tmp/claw-node

echo "==> Version: \$(/opt/clawnet-mainnet/bin/claw-node --version 2>/dev/null || /opt/clawnet-testnet/bin/claw-node --version)"

echo "==> Starting services: \$SERVICES"
for svc in \$SERVICES; do
  systemctl start \$svc
done

sleep 3

echo "==> Health checks:"
if echo "\$SERVICES" | grep -q mainnet; then
  HEALTH=\$(curl -sf http://localhost:9710/health 2>/dev/null || echo '{"status":"FAILED"}')
  echo "  Mainnet: \$HEALTH"
fi
if echo "\$SERVICES" | grep -q testnet; then
  HEALTH=\$(curl -sf http://localhost:9720/health 2>/dev/null || echo '{"status":"FAILED"}')
  echo "  Testnet: \$HEALTH"
fi

echo "==> Deploy complete!"
REMOTE

ok "Deploy complete! v${NEW_VERSION} running on ${DEPLOY_HOST}"
echo ""
echo "  Useful commands:"
echo "    ssh ${DEPLOY_USER}@${DEPLOY_HOST} 'journalctl -u clawnet-mainnet -f'     # mainnet logs"
echo "    ssh ${DEPLOY_USER}@${DEPLOY_HOST} 'journalctl -u clawnet-testnet -f'     # testnet logs"
echo "    curl https://rpc.clawlabz.xyz/health                                      # mainnet health"
echo "    curl https://testnet-rpc.clawlabz.xyz/health                               # testnet health"
echo ""
