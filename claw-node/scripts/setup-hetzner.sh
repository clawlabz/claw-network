#!/usr/bin/env bash
set -euo pipefail

# ClawNetwork — Initial server setup for Hetzner (run once on fresh Ubuntu 24.04)
#
# This script:
#   1. Installs dependencies (nginx, certbot, ufw)
#   2. Creates isolated users (clawnet-mainnet, clawnet-testnet)
#   3. Downloads and installs the node binary
#   4. Creates systemd services with security hardening
#   5. Configures log rotation
#   6. Sets up firewall
#   7. Configures Nginx reverse proxy
#   8. Obtains Let's Encrypt SSL certificates
#
# Usage:
#   ssh root@<IP> 'bash -s' < scripts/setup-hetzner.sh
#
# Prerequisites:
#   - Fresh Ubuntu 24.04 server
#   - DNS A records pointing to server IP:
#     - rpc.clawlabz.xyz → <server-ip>
#     - testnet-rpc.clawlabz.xyz → <server-ip>

BINARY_URL="${BINARY_URL:-}"
SSL_EMAIL="${SSL_EMAIL:-admin@clawlabz.xyz}"
MAINNET_DOMAIN="rpc.clawlabz.xyz"
TESTNET_DOMAIN="testnet-rpc.clawlabz.xyz"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'
info()  { echo -e "${CYAN}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }
err()   { echo -e "${RED}Error:${NC} $*" >&2; exit 1; }

# ── 1. System packages ──
info "Installing dependencies..."
apt-get update -qq
apt-get install -y -qq nginx certbot python3-certbot-nginx ufw curl jq logrotate > /dev/null 2>&1
ok "Packages installed"

# ── 2. Create users ──
info "Creating isolated users..."
for NET in mainnet testnet; do
  USER="clawnet-${NET}"
  if ! id "$USER" &>/dev/null; then
    useradd -r -m -d "/opt/${USER}" -s /usr/sbin/nologin "$USER"
    ok "Created user: $USER"
  else
    ok "User $USER already exists"
  fi
  mkdir -p "/opt/${USER}"/{bin,data,logs}
  chown -R "${USER}:${USER}" "/opt/${USER}"
done

# ── 3. Install binary ──
if [ -n "$BINARY_URL" ]; then
  info "Downloading binary from $BINARY_URL..."
  curl -fsSL "$BINARY_URL" -o /tmp/claw-node-linux-x86_64.tar.gz
elif [ -f /tmp/claw-node-linux-x86_64.tar.gz ]; then
  info "Using existing binary at /tmp/claw-node-linux-x86_64.tar.gz"
else
  err "No binary found. Either set BINARY_URL or place claw-node-linux-x86_64.tar.gz in /tmp/"
fi

tar xzf /tmp/claw-node-linux-x86_64.tar.gz -C /opt/clawnet-mainnet/bin/
chmod +x /opt/clawnet-mainnet/bin/claw-node
chown clawnet-mainnet:clawnet-mainnet /opt/clawnet-mainnet/bin/claw-node

cp /opt/clawnet-mainnet/bin/claw-node /opt/clawnet-testnet/bin/claw-node
chown clawnet-testnet:clawnet-testnet /opt/clawnet-testnet/bin/claw-node

VERSION=$(/opt/clawnet-mainnet/bin/claw-node --version)
ok "Installed: $VERSION"

# ── 4. Initialize nodes ──
info "Initializing nodes..."
cd /opt/clawnet-mainnet
sudo -u clawnet-mainnet /opt/clawnet-mainnet/bin/claw-node init --network mainnet 2>&1 || true

cd /opt/clawnet-testnet
sudo -u clawnet-testnet /opt/clawnet-testnet/bin/claw-node init --network testnet 2>&1 || true

# ── 5. systemd services ──
info "Creating systemd services..."

create_service() {
  local NET="$1"
  local RPC_PORT="$2"
  local P2P_PORT="$3"

  cat > "/etc/systemd/system/clawnet-${NET}.service" << EOF
[Unit]
Description=ClawNetwork ${NET^} Node
Documentation=https://github.com/clawlabz/claw-network
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=clawnet-${NET}
Group=clawnet-${NET}
WorkingDirectory=/opt/clawnet-${NET}
ExecStart=/opt/clawnet-${NET}/bin/claw-node start \\
  --network ${NET} \\
  --rpc-port ${RPC_PORT} \\
  --p2p-port ${P2P_PORT}
Restart=always
RestartSec=5
StartLimitIntervalSec=60
StartLimitBurst=5

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/clawnet-${NET}
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true

# Resource limits
LimitNOFILE=65535
MemoryMax=2G
CPUQuota=80%

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=clawnet-${NET}

[Install]
WantedBy=multi-user.target
EOF
}

create_service "mainnet" "9710" "9711"
create_service "testnet" "9720" "9721"
ok "systemd services created"

# ── 6. Log rotation ──
info "Configuring log rotation..."
cat > /etc/logrotate.d/clawnet << 'EOF'
/opt/clawnet-mainnet/logs/*.log
/opt/clawnet-testnet/logs/*.log
{
    daily
    rotate 14
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
}
EOF
ok "Log rotation configured"

# ── 7. Firewall ──
info "Configuring firewall..."
ufw --force reset > /dev/null 2>&1
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp     # SSH
ufw allow 80/tcp     # HTTP (certbot)
ufw allow 443/tcp    # HTTPS (Nginx)
ufw allow 9711/tcp   # Mainnet P2P
ufw allow 9721/tcp   # Testnet P2P
ufw --force enable
ok "Firewall enabled"

# ── 8. Start services ──
info "Starting nodes..."
systemctl daemon-reload
systemctl enable clawnet-mainnet clawnet-testnet
systemctl start clawnet-mainnet clawnet-testnet
sleep 4

for NET in mainnet testnet; do
  STATUS=$(systemctl is-active "clawnet-${NET}" || true)
  if [ "$STATUS" = "active" ]; then
    ok "clawnet-${NET}: active"
  else
    err "clawnet-${NET}: $STATUS — check: journalctl -u clawnet-${NET} -n 30"
  fi
done

# ── 9. Nginx reverse proxy ──
info "Configuring Nginx..."

create_nginx_site() {
  local NAME="$1"
  local DOMAIN="$2"
  local PORT="$3"

  cat > "/etc/nginx/sites-available/${NAME}" << EOF
server {
    listen 80;
    server_name ${DOMAIN};

    location / {
        proxy_pass http://127.0.0.1:${PORT};
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;

        # CORS
        add_header Access-Control-Allow-Origin "*" always;
        add_header Access-Control-Allow-Methods "GET, POST, OPTIONS" always;
        add_header Access-Control-Allow-Headers "Content-Type" always;
        if (\$request_method = OPTIONS) {
            return 204;
        }

        proxy_read_timeout 30s;
        proxy_connect_timeout 5s;
    }
}
EOF

  ln -sf "/etc/nginx/sites-available/${NAME}" "/etc/nginx/sites-enabled/"
}

create_nginx_site "clawnet-mainnet" "$MAINNET_DOMAIN" "9710"
create_nginx_site "clawnet-testnet" "$TESTNET_DOMAIN" "9720"
rm -f /etc/nginx/sites-enabled/default

nginx -t
systemctl reload nginx
ok "Nginx configured"

# ── 10. SSL certificates ──
info "Obtaining SSL certificates..."
certbot --nginx \
  -d "$MAINNET_DOMAIN" \
  -d "$TESTNET_DOMAIN" \
  --non-interactive \
  --agree-tos \
  --email "$SSL_EMAIL" \
  --redirect

ok "SSL certificates installed"

# ── Done ──
echo ""
echo "============================================="
echo "  ClawNetwork Deployment Complete!"
echo "============================================="
echo ""
echo "  Mainnet RPC:  https://${MAINNET_DOMAIN}"
echo "  Testnet RPC:  https://${TESTNET_DOMAIN}"
echo ""
echo "  Mainnet health: curl https://${MAINNET_DOMAIN}/health"
echo "  Testnet health: curl https://${TESTNET_DOMAIN}/health"
echo ""
echo "  Logs:   journalctl -u clawnet-mainnet -f"
echo "          journalctl -u clawnet-testnet -f"
echo ""
echo "  Manage: systemctl {start|stop|restart|status} clawnet-{mainnet|testnet}"
echo ""

# Final health check
echo "=== Health Check ==="
curl -sf "http://localhost:9710/health" 2>/dev/null | jq . || echo "Mainnet: starting..."
curl -sf "http://localhost:9720/health" 2>/dev/null | jq . || echo "Testnet: starting..."
