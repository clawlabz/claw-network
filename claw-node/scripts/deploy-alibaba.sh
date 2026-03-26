#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork — One-click deploy to Alibaba Cloud
# Usage:
#   ./scripts/deploy-alibaba.sh              # bump patch, tag, build via CI, deploy
#   ./scripts/deploy-alibaba.sh --skip-bump  # deploy current version (re-deploy)
#   ./scripts/deploy-alibaba.sh --local      # build locally instead of waiting for CI
#   DEPLOY_HOST=1.2.3.4 ./scripts/deploy-alibaba.sh  # custom host

DEPLOY_HOST="${DEPLOY_HOST:-39.102.144.231}"
DEPLOY_USER="${DEPLOY_USER:-root}"
DEPLOY_DIR="${DEPLOY_DIR:-/root/.clawnetwork}"
TARGET="linux-x86_64"
SKIP_BUMP=false
LOCAL_BUILD=false

for arg in "$@"; do
  case "$arg" in
    --skip-bump) SKIP_BUMP=true ;;
    --local)     LOCAL_BUILD=true ;;
    --help|-h)
      echo "Usage: $0 [--skip-bump] [--local]"
      echo ""
      echo "  --skip-bump   Skip version bump, redeploy current version"
      echo "  --local       Cross-compile locally instead of waiting for CI"
      echo ""
      echo "Env vars:"
      echo "  DEPLOY_HOST   Server IP (default: 39.102.144.231)"
      echo "  DEPLOY_USER   SSH user (default: root)"
      echo "  DEPLOY_DIR    Remote data dir (default: /root/.clawnetwork)"
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
  if ! command -v cross &>/dev/null; then
    err "'cross' not installed. Run: cargo install cross"
  fi
  cross build --release --target x86_64-unknown-linux-musl
  tar czf "$LOCAL_PATH" -C target/x86_64-unknown-linux-musl/release claw-node
  ok "Local build complete"
else
  info "Waiting for CI release build..."
  # Wait for the release workflow to finish
  RUN_ID=$(gh run list --workflow=release.yml --limit 1 --json databaseId -q '.[0].databaseId')
  if [ -z "$RUN_ID" ]; then
    err "No release workflow run found"
  fi
  gh run watch "$RUN_ID" --exit-status || err "CI build failed"
  ok "CI build passed"

  info "Downloading ${ARTIFACT} from release ${TAG}..."
  gh release download "$TAG" --pattern "$ARTIFACT" --dir /tmp --clobber
  ok "Downloaded to $LOCAL_PATH"
fi

ls -lh "$LOCAL_PATH"

# ── Step 3: Upload to server ──
info "Uploading to ${DEPLOY_USER}@${DEPLOY_HOST}..."
scp "$LOCAL_PATH" "${DEPLOY_USER}@${DEPLOY_HOST}:/tmp/${ARTIFACT}"
ok "Upload complete"

# ── Step 4: Deploy on server (preserve data) ──
info "Deploying on server..."
ssh "${DEPLOY_USER}@${DEPLOY_HOST}" "bash -s" << REMOTE
set -euo pipefail

# Stop old process
pkill -f claw-node || true
sleep 1

# Extract new binary
tar xzf /tmp/${ARTIFACT} -C ${DEPLOY_DIR}/bin/
chmod +x ${DEPLOY_DIR}/bin/claw-node

# Verify version
echo "==> Version: \$(${DEPLOY_DIR}/bin/claw-node --version)"

# Verify data preserved
if [ -f "${DEPLOY_DIR}/chain.redb" ]; then
  echo "==> Data: chain.redb exists (preserved)"
else
  echo "==> Warning: no chain.redb found (fresh node)"
fi

# Start node (NO init — preserves chain data)
nohup ${DEPLOY_DIR}/bin/claw-node start \
  --network devnet \
  --rpc-port 9710 \
  > ${DEPLOY_DIR}/node.log 2>&1 &

sleep 3

# Health check
HEALTH=\$(curl -sf http://localhost:9710/health 2>/dev/null || echo "FAILED")
echo "==> Health: \$HEALTH"

if echo "\$HEALTH" | grep -q '"height"'; then
  echo "==> Deploy SUCCESS"
else
  echo "==> Deploy may have issues, check: tail -50 ${DEPLOY_DIR}/node.log"
  exit 1
fi
REMOTE

ok "Deploy complete! v${NEW_VERSION} running on ${DEPLOY_HOST}"
echo ""
echo "  Useful commands:"
echo "    ssh ${DEPLOY_USER}@${DEPLOY_HOST} 'tail -f ${DEPLOY_DIR}/node.log'    # logs"
echo "    ssh ${DEPLOY_USER}@${DEPLOY_HOST} 'curl -s localhost:9710/health'      # health"
echo "    curl http://${DEPLOY_HOST}:9710/health                                  # remote check"
echo ""
