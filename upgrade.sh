#!/usr/bin/env bash
# TekmerDB Upgrade Script
# Lives in /opt/tekmerdb — no repo checkout or cargo needed to run it.
#
# Workflow:
#   sudo /opt/tekmerdb/upgrade.sh
#
# The script will:
#   - Look up the latest release version on GitHub
#   - Compare it against the currently installed version
#   - Skip if already up to date
#   - Otherwise confirm with you, then download and install the new
#     binaries, and confirm again before restarting services

set -e

REPO="raa82/tekmerdb"
INSTALL_DIR="/opt/tekmerdb"
CONF="$INSTALL_DIR/tekmerdb-server.conf"
SERVICES="tekmerdb-server tekmerdb-mcp tekmerdb-cron"

# ── colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[tekmerdb]${NC} $1"; }
warn()  { echo -e "${YELLOW}[tekmerdb]${NC} $1"; }
error() { echo -e "${RED}[tekmerdb]${NC} $1"; exit 1; }

# ── preflight ─────────────────────────────────────────────────────────────────

info "TekmerDB upgrade checker starting..."

if [ "$EUID" -ne 0 ]; then
    error "Please run as root: sudo $0"
fi

for cmd in wget curl sort; do
    if ! command -v "$cmd" &>/dev/null; then
        error "Required command not found: $cmd — install it first (e.g. apt install $cmd)"
    fi
done

if [ ! -x "$INSTALL_DIR/tekmerdb" ]; then
    error "No installation found at $INSTALL_DIR. Run install.sh first."
fi

# ── determine architecture ────────────────────────────────────────────────────

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ARCH_LABEL="linux-x64" ;;
    aarch64) ARCH_LABEL="linux-arm64" ;;
    *) error "Unsupported architecture: $ARCH" ;;
esac

# ── look up latest version on GitHub ──────────────────────────────────────────

info "Checking latest release on GitHub..."
LATEST_JSON=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest") \
    || error "Could not reach GitHub releases API for $REPO."

LATEST_VERSION=$(echo "$LATEST_JSON" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
if [ -z "$LATEST_VERSION" ]; then
    error "Could not parse latest release version from GitHub response."
fi
info "Latest release: $LATEST_VERSION"

# ── determine currently installed version ─────────────────────────────────────
# Prefer the live /status endpoint (authoritative — it's the build-time version
# of whatever is actually running). Fall back to the on-disk VERSION marker
# this script writes after every upgrade, for when the server isn't running.

ENGINE_HOST="127.0.0.1"
ENGINE_PORT="3000"
if [ -f "$CONF" ]; then
    CFG_HOST=$(grep -E '^\s*engine_host\s*=' "$CONF" | head -1 | sed -E 's/.*=\s*"?([^"[:space:]]+)"?.*/\1/')
    CFG_PORT=$(grep -E '^\s*engine_port\s*=' "$CONF" | head -1 | sed -E 's/.*=\s*"?([^"[:space:]]+)"?.*/\1/')
    [ -n "$CFG_HOST" ] && ENGINE_HOST="$CFG_HOST"
    [ -n "$CFG_PORT" ] && ENGINE_PORT="$CFG_PORT"
fi

INSTALLED_VERSION=""
STATUS_JSON=$(curl -fsSL --max-time 5 "http://$ENGINE_HOST:$ENGINE_PORT/status" 2>/dev/null) || true
if [ -n "$STATUS_JSON" ]; then
    INSTALLED_VERSION=$(echo "$STATUS_JSON" | grep -m1 '"version"' | sed -E 's/.*"version":"?([^",}]+)"?.*/\1/')
fi

if [ -z "$INSTALLED_VERSION" ] && [ -f "$INSTALL_DIR/VERSION" ]; then
    warn "Engine not reachable at http://$ENGINE_HOST:$ENGINE_PORT — using on-disk VERSION marker."
    INSTALLED_VERSION=$(tr -d ' \n' < "$INSTALL_DIR/VERSION")
fi

if [ -z "$INSTALLED_VERSION" ]; then
    error "Could not determine the installed version (engine unreachable, no VERSION marker). Start tekmerdb-server and retry."
fi

info "Installed version: $INSTALLED_VERSION"

# ── compare versions ──────────────────────────────────────────────────────────

if [ "$INSTALLED_VERSION" == "$LATEST_VERSION" ]; then
    info "TekmerDB $INSTALLED_VERSION is already up to date. Nothing to do."
    exit 0
fi

NEWER=$(printf '%s\n%s\n' "$INSTALLED_VERSION" "$LATEST_VERSION" | sort -V | tail -1)
if [ "$NEWER" != "$LATEST_VERSION" ]; then
    warn "Installed version ($INSTALLED_VERSION) is newer than the latest release ($LATEST_VERSION). Nothing to do."
    exit 0
fi

# ── confirm the upgrade ───────────────────────────────────────────────────────

echo ""
warn "Upgrade available: $INSTALLED_VERSION -> $LATEST_VERSION"
read -r -p "  Proceed with the upgrade? [y/N] " UPGRADE_CONFIRM
if [[ ! "$UPGRADE_CONFIRM" =~ ^[Yy]$ ]]; then
    info "Upgrade cancelled."
    exit 0
fi

# ── download and stage new binaries ───────────────────────────────────────────

TARBALL="tekmerdb-v${LATEST_VERSION}-${ARCH_LABEL}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_VERSION/$TARBALL"
TMP_DIR=$(mktemp -d)

info "Downloading $LATEST_VERSION binaries..."
info "URL: $DOWNLOAD_URL"
wget --progress=bar:force -O "$TMP_DIR/$TARBALL" "$DOWNLOAD_URL" 2>&1 \
    || error "Download failed.
  Make sure release $LATEST_VERSION exists at:
  https://github.com/$REPO/releases/tag/$LATEST_VERSION"

info "Extracting binaries..."
tar -xzf "$TMP_DIR/$TARBALL" -C "$TMP_DIR"

TEKMERDB_BIN=$(find "$TMP_DIR" -name "tekmerdb" -not -name "tekmerdb-mcp" -not -name "tekmerdb-ingest" -not -name "tekmerdb-cron" -type f | head -1)
TEKMERDB_MCP_BIN=$(find "$TMP_DIR" -name "tekmerdb-mcp" -type f | head -1)
TEKMERDB_CRON_BIN=$(find "$TMP_DIR" -name "tekmerdb-cron" -type f | head -1)
TEKMERDB_INGEST_BIN=$(find "$TMP_DIR" -name "tekmerdb-ingest" -type f | head -1)

if [ -z "$TEKMERDB_BIN" ] || [ -z "$TEKMERDB_MCP_BIN" ] || [ -z "$TEKMERDB_CRON_BIN" ] || [ -z "$TEKMERDB_INGEST_BIN" ]; then
    error "Binaries not found in release tarball."
fi

# ── which services are currently running (to restart after upgrade) ──────────

RUNNING_SERVICES=""
for svc in $SERVICES; do
    if systemctl is-active --quiet "$svc" 2>/dev/null; then
        RUNNING_SERVICES="$RUNNING_SERVICES $svc"
    fi
done

# ── install new binaries ──────────────────────────────────────────────────────

info "Installing binaries..."
cp "$TEKMERDB_BIN"        "$INSTALL_DIR/tekmerdb"
cp "$TEKMERDB_MCP_BIN"    "$INSTALL_DIR/tekmerdb-mcp"
cp "$TEKMERDB_CRON_BIN"   "$INSTALL_DIR/tekmerdb-cron"
cp "$TEKMERDB_INGEST_BIN" "$INSTALL_DIR/tekmerdb-ingest"
chmod +x "$INSTALL_DIR/tekmerdb" "$INSTALL_DIR/tekmerdb-mcp" "$INSTALL_DIR/tekmerdb-cron" "$INSTALL_DIR/tekmerdb-ingest"
chown tekmerdb:tekmerdb "$INSTALL_DIR/tekmerdb" "$INSTALL_DIR/tekmerdb-mcp" "$INSTALL_DIR/tekmerdb-cron" "$INSTALL_DIR/tekmerdb-ingest"

rm -rf "$TMP_DIR"

echo "$LATEST_VERSION" > "$INSTALL_DIR/VERSION"
chown tekmerdb:tekmerdb "$INSTALL_DIR/VERSION"

info "Binaries upgraded to $LATEST_VERSION."

# ── confirm the restart ───────────────────────────────────────────────────────

if [ -n "$RUNNING_SERVICES" ]; then
    echo ""
    warn "These services are still running the old binaries:$RUNNING_SERVICES"
    read -r -p "  Restart them now to load $LATEST_VERSION? [y/N] " RESTART_CONFIRM
    if [[ "$RESTART_CONFIRM" =~ ^[Yy]$ ]]; then
        info "Restarting services:$RUNNING_SERVICES"
        systemctl restart $RUNNING_SERVICES
        info "Done."
    else
        warn "Skipped restart — old binaries still running. Restart manually when ready:"
        warn "  systemctl restart$RUNNING_SERVICES"
    fi
else
    info "No services were running — nothing to restart."
fi

echo ""
info "TekmerDB upgraded: $INSTALLED_VERSION -> $LATEST_VERSION"
