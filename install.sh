#!/usr/bin/env bash
# TekmerDB Install Script
# Installs TekmerDB to /opt/tekmerdb
#
# Workflow:
#   1. Clone or download the repo
#   2. sudo ./install.sh
#
# The script will:
#   - Read the version from Cargo.toml
#   - Download the matching binaries from GitHub releases
#   - Copy config and other files from the local repo
#   - Download ML models from HuggingFace

set -e

REPO="raa82/tekmerdb"
INSTALL_DIR="/opt/tekmerdb"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[tekmerdb]${NC} $1"; }
warn()  { echo -e "${YELLOW}[tekmerdb]${NC} $1"; }
error() { echo -e "${RED}[tekmerdb]${NC} $1"; exit 1; }

# ── preflight ─────────────────────────────────────────────────────────────────

info "TekmerDB installer starting..."

if [ "$EUID" -ne 0 ]; then
    error "Please run as root: sudo ./install.sh"
fi

for cmd in wget curl; do
    if ! command -v "$cmd" &>/dev/null; then
        error "Required command not found: $cmd — install it first (e.g. apt install $cmd)"
    fi
done

# check we're running from the repo root
if [ ! -f "$SCRIPT_DIR/Cargo.toml" ] || [ ! -f "$SCRIPT_DIR/tekmerdb-server.conf" ]; then
    error "Run this script from the root of the tekmerdb repository."
fi

# ── create system user ────────────────────────────────────────────────────────

if ! id -u tekmerdb &>/dev/null; then
    useradd --system --no-create-home --shell /sbin/nologin tekmerdb
    info "Created system user: tekmerdb"
else
    info "System user tekmerdb already exists."
fi

# ── read version from Cargo.toml ──────────────────────────────────────────────

VERSION=$(grep '^version' "$SCRIPT_DIR/Cargo.toml" | head -1 | sed -E 's/version = "(.*)"/\1/' | tr -d ' ')
if [ -z "$VERSION" ]; then
    error "Could not read version from Cargo.toml"
fi

TAG="$VERSION"
info "Version: $TAG"

# ── check for existing installation ───────────────────────────────────────────

if [ -x "$INSTALL_DIR/tekmerdb" ]; then
    info "TekmerDB is already installed at $INSTALL_DIR — skipping installation."
    exit 0
fi

info "No existing installation found — installing TekmerDB $TAG."

# ── detect architecture ───────────────────────────────────────────────────────

ARCH=$(uname -m)
case "$ARCH" in
    x86_64)  ARCH_LABEL="linux-x64" ;;
    aarch64) ARCH_LABEL="linux-arm64" ;;
    *) error "Unsupported architecture: $ARCH" ;;
esac

info "Architecture: $ARCH_LABEL"

# ── download binaries from GitHub releases ────────────────────────────────────

TARBALL="tekmerdb-v${VERSION}-${ARCH_LABEL}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$TAG/$TARBALL"
TMP_DIR=$(mktemp -d)

info "Downloading binaries from GitHub releases..."
info "URL: $DOWNLOAD_URL"
wget --progress=bar:force -O "$TMP_DIR/$TARBALL" "$DOWNLOAD_URL" 2>&1 \
    || error "Download failed.
  Make sure release $TAG exists at:
  https://github.com/$REPO/releases/tag/$TAG"

info "Extracting binaries..."
tar -xzf "$TMP_DIR/$TARBALL" -C "$TMP_DIR"

# find extracted binaries — handle both flat and subdirectory extraction
TEKMERDB_BIN=$(find "$TMP_DIR" -name "tekmerdb" -not -name "tekmerdb-mcp" -not -name "tekmerdb-ingest" -not -name "tekmerdb-cron" -type f | head -1)
TEKMERDB_MCP_BIN=$(find "$TMP_DIR" -name "tekmerdb-mcp" -type f | head -1)
TEKMERDB_CRON_BIN=$(find "$TMP_DIR" -name "tekmerdb-cron" -type f | head -1)
TEKMERDB_INGEST_BIN=$(find "$TMP_DIR" -name "tekmerdb-ingest" -type f | head -1)

if [ -z "$TEKMERDB_BIN" ] || [ -z "$TEKMERDB_MCP_BIN" ] || [ -z "$TEKMERDB_CRON_BIN" ] || [ -z "$TEKMERDB_INGEST_BIN" ]; then
    error "Binaries not found in release tarball."
fi

# ── create install directory ──────────────────────────────────────────────────

info "Creating install directory at $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR/models"
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$INSTALL_DIR/log"

# ── install binaries ──────────────────────────────────────────────────────────

info "Installing binaries..."
cp "$TEKMERDB_BIN"        "$INSTALL_DIR/tekmerdb"
cp "$TEKMERDB_MCP_BIN"    "$INSTALL_DIR/tekmerdb-mcp"
cp "$TEKMERDB_CRON_BIN"   "$INSTALL_DIR/tekmerdb-cron"
cp "$TEKMERDB_INGEST_BIN" "$INSTALL_DIR/tekmerdb-ingest"
chmod +x "$INSTALL_DIR/tekmerdb"
chmod +x "$INSTALL_DIR/tekmerdb-mcp"
chmod +x "$INSTALL_DIR/tekmerdb-cron"
chmod +x "$INSTALL_DIR/tekmerdb-ingest"

rm -rf "$TMP_DIR"

# ── install config from local repo ───────────────────────────────────────────

if [ -f "$INSTALL_DIR/tekmerdb-server.conf" ]; then
    warn "tekmerdb-server.conf already exists — preserving your settings."
    warn "New default config saved as $INSTALL_DIR/tekmerdb-server.conf.new"
    cp "$SCRIPT_DIR/tekmerdb-server.conf" "$INSTALL_DIR/tekmerdb-server.conf.new"
else
    cp "$SCRIPT_DIR/tekmerdb-server.conf" "$INSTALL_DIR/tekmerdb-server.conf"
    info "Config file installed."
fi

cp "$SCRIPT_DIR/system_jobs.json" "$INSTALL_DIR/system_jobs.json"
info "system_jobs.json installed."

if [ -f "$INSTALL_DIR/user_jobs.json" ]; then
    warn "user_jobs.json already exists — preserving your jobs."
    warn "New default saved as $INSTALL_DIR/user_jobs.json.new"
    cp "$SCRIPT_DIR/user_jobs.json" "$INSTALL_DIR/user_jobs.json.new"
else
    cp "$SCRIPT_DIR/user_jobs.json" "$INSTALL_DIR/user_jobs.json"
    info "user_jobs.json installed."
fi

# ── install management scripts ────────────────────────────────────────────────

for script in uninstall.sh upgrade.sh; do
    if [ -f "$SCRIPT_DIR/$script" ]; then
        cp "$SCRIPT_DIR/$script" "$INSTALL_DIR/$script"
        chmod +x "$INSTALL_DIR/$script"
        info "$script copied to $INSTALL_DIR."
    fi
done

# ── download models ───────────────────────────────────────────────────────────

info "Checking models..."

download_model() {
    local name="$1"
    local url="$2"
    local dest="$3"
    local size="$4"

    if [ -f "$dest" ]; then
        info "Model already present: $name — skipping."
    else
        info "Downloading $name (~${size}MB)..."
        wget --progress=bar:force -O "$dest" "$url" 2>&1 \
            || error "Failed to download $name"
        info "$name downloaded."
    fi
}

download_model \
    "miniLM.onnx" \
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx" \
    "$INSTALL_DIR/models/miniLM.onnx" \
    "90"

download_model \
    "tokenizer.json" \
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json" \
    "$INSTALL_DIR/models/tokenizer.json" \
    "1"

download_model \
    "nli.onnx" \
    "https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/onnx/model.onnx" \
    "$INSTALL_DIR/models/nli.onnx" \
    "328"

download_model \
    "nli_tokenizer.json" \
    "https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/tokenizer.json" \
    "$INSTALL_DIR/models/nli_tokenizer.json" \
    "1"

# ── permissions ───────────────────────────────────────────────────────────────

chown -R tekmerdb:tekmerdb "$INSTALL_DIR"

# ── systemd services ──────────────────────────────────────────────────────────

info "Installing systemd service units..."

cat > /etc/systemd/system/tekmerdb-server.service <<EOF
[Unit]
Description=TekmerDB Engine
After=network.target

[Service]
Type=simple
User=tekmerdb
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/tekmerdb
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/tekmerdb-mcp.service <<EOF
[Unit]
Description=TekmerDB MCP Server
After=tekmerdb-server.service
# Deliberately no Requires=: it only cascades a stop, never a restart — an
# operator restarting tekmerdb-server (e.g. a scheduled data reset) would take
# this unit down with it and it would not come back up on its own.

[Service]
Type=simple
User=tekmerdb
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/tekmerdb-mcp --sse
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/tekmerdb-cron.service <<EOF
[Unit]
Description=TekmerDB Cron Scheduler
After=tekmerdb-server.service
# Deliberately no Requires=: it only cascades a stop, never a restart — an
# operator restarting tekmerdb-server (e.g. a scheduled data reset) would take
# this unit down with it and it would not come back up on its own.

[Service]
Type=simple
User=tekmerdb
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/tekmerdb-cron
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable tekmerdb-server tekmerdb-mcp tekmerdb-cron
info "Services enabled (not started — run: systemctl start tekmerdb-server tekmerdb-mcp tekmerdb-cron)"

# ── verify ────────────────────────────────────────────────────────────────────

info "Verifying installation..."
MISSING=0
for f in tekmerdb tekmerdb-mcp tekmerdb-cron tekmerdb-ingest tekmerdb-server.conf \
          system_jobs.json user_jobs.json \
          models/miniLM.onnx models/tokenizer.json \
          models/nli.onnx models/nli_tokenizer.json; do
    if [ ! -f "$INSTALL_DIR/$f" ]; then
        warn "Missing: $INSTALL_DIR/$f"
        MISSING=1
    fi
done

if [ "$MISSING" -eq 1 ]; then
    error "Installation incomplete — some files are missing."
fi

echo "$VERSION" > "$INSTALL_DIR/VERSION"
chown tekmerdb:tekmerdb "$INSTALL_DIR/VERSION"

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
info "TekmerDB $TAG installed successfully."
echo ""
echo "  Location  : $INSTALL_DIR"
echo "  Engine    : $INSTALL_DIR/tekmerdb"
echo "  MCP       : $INSTALL_DIR/tekmerdb-mcp"
echo "  Cron      : $INSTALL_DIR/tekmerdb-cron"
echo "  Ingestor  : $INSTALL_DIR/tekmerdb-ingest"
echo "  Config    : $INSTALL_DIR/tekmerdb-server.conf"
echo "  Jobs      : $INSTALL_DIR/system_jobs.json / user_jobs.json"
echo "  Models    : $INSTALL_DIR/models/"
echo "  Data      : $INSTALL_DIR/data/"
echo "  Logs      : $INSTALL_DIR/log/"
echo "  Uninstall : $INSTALL_DIR/uninstall.sh"
echo "  Upgrade   : $INSTALL_DIR/upgrade.sh"
echo ""
echo "  Start all services:"
echo "    systemctl start tekmerdb-server tekmerdb-mcp tekmerdb-cron"
echo ""
echo "  Configure:"
echo "    edit $INSTALL_DIR/tekmerdb-server.conf"
echo "    systemctl restart tekmerdb-server"
echo ""