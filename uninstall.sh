#!/usr/bin/env bash
# TekmerDB Uninstall Script
# Removes all traces of TekmerDB from the system.
# Must be run as root: sudo ./uninstall.sh

set -e

INSTALL_DIR="/opt/tekmerdb"
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

if [ "$EUID" -ne 0 ]; then
    error "Please run as root: sudo ./uninstall.sh"
fi

echo ""
warn "This will permanently remove TekmerDB and all its data from this machine."
warn "This includes: services, binaries, models, data, logs, and the tekmerdb user."
echo ""
read -r -p "  Type YES to confirm: " CONFIRM
if [ "$CONFIRM" != "YES" ]; then
    echo "Aborted."
    exit 0
fi
echo ""

# ── stop and disable services ─────────────────────────────────────────────────

info "Stopping and disabling services..."
for svc in $SERVICES; do
    if systemctl is-active --quiet "$svc" 2>/dev/null; then
        systemctl stop "$svc"
        info "  stopped $svc"
    fi
    if systemctl is-enabled --quiet "$svc" 2>/dev/null; then
        systemctl disable "$svc"
        info "  disabled $svc"
    fi
done

# ── remove systemd unit files ─────────────────────────────────────────────────

info "Removing systemd unit files..."
for svc in $SERVICES; do
    UNIT="/etc/systemd/system/${svc}.service"
    if [ -f "$UNIT" ]; then
        rm -f "$UNIT"
        info "  removed $UNIT"
    fi
done
systemctl daemon-reload
systemctl reset-failed 2>/dev/null || true

# ── remove install directory ──────────────────────────────────────────────────

if [ -d "$INSTALL_DIR" ]; then
    info "Removing $INSTALL_DIR..."
    rm -rf "$INSTALL_DIR"
    info "  removed $INSTALL_DIR"
else
    warn "$INSTALL_DIR not found — skipping."
fi

# ── remove system user and group ──────────────────────────────────────────────

if id -u tekmerdb &>/dev/null; then
    info "Removing system user: tekmerdb"
    userdel tekmerdb
fi

if getent group tekmerdb &>/dev/null; then
    info "Removing group: tekmerdb"
    groupdel tekmerdb 2>/dev/null || true
fi

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
info "TekmerDB has been fully removed from this system."
echo ""
