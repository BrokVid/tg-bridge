#!/usr/bin/env bash
# Tg Bridge installation wizard for Ubuntu/Debian servers.
#
# One-line install (run as root on the bridge host):
#   curl -fsSL https://raw.githubusercontent.com/BrokVid/tg-bridge/main/install.sh | bash
#
# The wizard asks a few questions, generates secrets, writes the config,
# installs a hardened systemd unit and prints the env values to give clients.
set -euo pipefail

REPO="BrokVid/tg-bridge"
BIN="/usr/local/bin/tg-bridge"
ENV_FILE="/etc/tg-bridge/env"
CONF="/etc/tg-bridge/tg-bridge.toml"
UNIT="/etc/systemd/system/tg-bridge.service"

log() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ $(id -u) -eq 0 ]] || die "run as root: sudo bash install.sh"
command -v systemctl >/dev/null || die "systemd is required"

# --- prompts ----------------------------------------------------------------
# Every answer can be pre-set via environment variable for unattended
# installs: TGB_LISTEN, TGB_ALIAS, TGB_BOT_TOKEN, TGB_CLIENT, TGB_CLIENT_IP,
# TGB_USE_RELEASE.
ask() { # ask <prompt> <default> -> answer in $REPLY
    local prompt="$1" def="${2:-}"
    # opening /dev/tty fails when there is no controlling terminal
    # (e.g. `curl | bash` over ssh), which means unattended mode
    if exec 3</dev/tty 2>/dev/null; then
        read -r -p "$(printf '\033[1m%s\033[0m%s: ' "$prompt" "${def:+ [$def]}")" REPLY <&3 || {
            exec 3<&-; die "aborted";
        }
        exec 3<&-
        REPLY="${REPLY:-$def}"
    else
        # fall back to the provided default, which may come from the environment
        REPLY="$def"
    fi
}

secret() { openssl rand -hex 32; }

# --- questions --------------------------------------------------------------
log "Tg Bridge setup"
ask "Listen address (127.0.0.1 behind TLS/reverse proxy, 0.0.0.0 for direct)" "${TGB_LISTEN:-127.0.0.1:8080}"
LISTEN="$REPLY"
ask "Bot alias (clients use it in URLs, e.g. mybot)" "${TGB_ALIAS:-mybot}"
ALIAS="$REPLY"
ask "Telegram bot token from @BotFather" "${TGB_BOT_TOKEN:-}"
BOT_TOKEN="$REPLY"
[ -n "$BOT_TOKEN" ] || die "bot token is required (set TGB_BOT_TOKEN for unattended installs)"
ask "First client name (e.g. myapp)" "${TGB_CLIENT:-myapp}"
CLIENT="$REPLY"
ask "Restrict client to an IP/CIDR? (empty = any IP)" "${TGB_CLIENT_IP:-}"
ALLOWED="$REPLY"
ask "Download prebuilt binary from GitHub Releases? (no = build with cargo)" "${TGB_USE_RELEASE:-yes}"
USE_RELEASE="$REPLY"

# --- files & user -----------------------------------------------------------
log "creating user and directories"
id tg-bridge >/dev/null 2>&1 || useradd --system --home /var/lib/tg-bridge --shell /usr/sbin/nologin tg-bridge
install -d -o tg-bridge -g tg-bridge -m 700 /etc/tg-bridge /var/lib/tg-bridge

CLIENT_SECRET="$(secret)"

log "writing $ENV_FILE"
cat >"$ENV_FILE" <<EOF
TGB_BOT_${ALIAS^^}_TOKEN=$BOT_TOKEN
TGB_CLIENT_${CLIENT^^}_SECRET=$CLIENT_SECRET
EOF
chmod 600 "$ENV_FILE"; chown tg-bridge:tg-bridge "$ENV_FILE"

log "writing $CONF"
{
    cat <<EOF
[server]
listen = "$LISTEN"
max_body_bytes = 65536
max_upload_bytes = 52428800
request_timeout_secs = 35
timestamp_window_secs = 60
replay_protection = true

[bots.$ALIAS]
token = "env:TGB_BOT_${ALIAS^^}_TOKEN"

[[clients]]
name = "$CLIENT"
secret = "env:TGB_CLIENT_${CLIENT^^}_SECRET"
EOF
    if [ -n "$ALLOWED" ]; then printf 'allowed_ips = ["%s"]\n' "$ALLOWED"; fi
} >"$CONF"
chown tg-bridge:tg-bridge "$CONF"

# --- binary -----------------------------------------------------------------
if [[ "$USE_RELEASE" =~ ^[Yy] ]]; then
    log "downloading latest release binary (x86_64)"
    URL="https://github.com/${REPO}/releases/latest/download/tg-bridge-x86_64-linux-musl.tar.gz"
    TMP="$(mktemp -d)"
    curl -fsSL "$URL" | tar xz -C "$TMP"
    install -m 755 "$TMP/tg-bridge" "$BIN"
    rm -rf "$TMP"
else
    command -v cargo >/dev/null || die "cargo not found; install rustup or choose release download"
    log "building with cargo (a few minutes)"
    TMP="$(mktemp -d)"
    git clone --depth 1 "https://github.com/${REPO}.git" "$TMP/src"
    (cd "$TMP/src" && cargo build --release --locked)
    install -m 755 "$TMP/src/target/release/tg-bridge" "$BIN"
    rm -rf "$TMP"
fi

# --- systemd ----------------------------------------------------------------
log "installing systemd unit"
cat >"$UNIT" <<'EOF'
[Unit]
Description=Tg Bridge - Telegram Bot API relay
After=network-online.target
Wants=network-online.target

[Service]
User=tg-bridge
Group=tg-bridge
Environment=TGB_CONFIG=/etc/tg-bridge/tg-bridge.toml
EnvironmentFile=/etc/tg-bridge/env
ExecStart=/usr/local/bin/tg-bridge
Restart=on-failure
RestartSec=2
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6
LockPersonality=true
SystemCallArchitectures=native
CapabilityBoundingSet=
AmbientCapabilities=

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now tg-bridge
sleep 1
systemctl is-active tg-bridge >/dev/null || { journalctl -u tg-bridge -n 20 --no-pager; die "service failed to start"; }

LISTEN_PORT="${LISTEN##*:}"
curl -fsS "http://127.0.0.1:${LISTEN_PORT}/healthz" >/dev/null || die "healthz check failed"

cat <<EOF

Done. Bridge is running.

Give these values to the client machine (keep them private!):

  TGB_URL=http://<bridge-host>:${LISTEN_PORT}
  TGB_CLIENT=${CLIENT}
  TGB_SECRET=${CLIENT_SECRET}
  BOT_ALIAS=${ALIAS}

Client integration guide: docs/INTEGRATION.md
Config reference:         docs/PROTOCOL.md
EOF
