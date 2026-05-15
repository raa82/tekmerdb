#!/usr/bin/env bash
# TekmerDB Install Script
# Installs tekmerdb to /opt/tekmerdb
#
# Prerequisites on the target machine:
#   - wget
#   - Pre-built binaries in target/release/ (build on dev machine with: cargo build --release)
#
# Usage:
#   sudo ./install.sh

set -e

INSTALL_DIR="/opt/tekmerdb"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── colours ───────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()    { echo -e "${GREEN}[tekmerdb]${NC} $1"; }
warn()    { echo -e "${YELLOW}[tekmerdb]${NC} $1"; }
error()   { echo -e "${RED}[tekmerdb]${NC} $1"; exit 1; }

# ── preflight ─────────────────────────────────────────────────────────────────

info "TekmerDB installer starting..."

# must be run as root or with sudo
if [ "$EUID" -ne 0 ]; then
    error "Please run as root: sudo ./install.sh"
fi

# check dependencies
for cmd in wget; do
    if ! command -v "$cmd" &>/dev/null; then
        error "Required command not found: $cmd — install it with: apt install wget"
    fi
done

# check pre-built binaries exist
if [ ! -f "$REPO_DIR/target/release/tekmerdb" ] || [ ! -f "$REPO_DIR/target/release/tekmerdb-mcp" ]; then
    error "Pre-built binaries not found in target/release/
  Build them on your development machine first:
    cargo build --release
  Then run this installer."
fi

info "Pre-built binaries found."

# ── create install directory ──────────────────────────────────────────────────

info "Creating install directory at $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR/models"
mkdir -p "$INSTALL_DIR/data"
mkdir -p "$INSTALL_DIR/log"

# ── copy binaries ─────────────────────────────────────────────────────────────

info "Installing binaries..."
cp "$REPO_DIR/target/release/tekmerdb"     "$INSTALL_DIR/tekmerdb"
cp "$REPO_DIR/target/release/tekmerdb-mcp" "$INSTALL_DIR/tekmerdb-mcp"
chmod +x "$INSTALL_DIR/tekmerdb"
chmod +x "$INSTALL_DIR/tekmerdb-mcp"

# ── copy config file ──────────────────────────────────────────────────────────

if [ -f "$REPO_DIR/tekmerdb-server.conf" ]; then
    if [ -f "$INSTALL_DIR/tekmerdb-server.conf" ]; then
        warn "tekmerdb-server.conf already exists in $INSTALL_DIR — skipping to preserve your settings."
        warn "New default config saved as $INSTALL_DIR/tekmerdb-server.conf.new"
        cp "$REPO_DIR/tekmerdb-server.conf" "$INSTALL_DIR/tekmerdb-server.conf.new"
    else
        cp "$REPO_DIR/tekmerdb-server.conf" "$INSTALL_DIR/tekmerdb-server.conf"
        info "Config file installed."
    fi
fi

# ── download models ───────────────────────────────────────────────────────────

info "Checking models..."

download_model() {
    local name="$1"
    local url="$2"
    local dest="$3"

    if [ -f "$dest" ]; then
        info "Model already present: $name — skipping download."
    else
        info "Downloading $name (~$(echo $4) MB)..."
        wget --progress=bar:force -O "$dest" "$url" 2>&1 || error "Failed to download $name"
        info "$name downloaded."
    fi
}

download_model \
    "miniLM.onnx (sentence embedding model)" \
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx" \
    "$INSTALL_DIR/models/miniLM.onnx" \
    "90"

download_model \
    "tokenizer.json (MiniLM vocabulary)" \
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json" \
    "$INSTALL_DIR/models/tokenizer.json" \
    "1"

download_model \
    "nli.onnx (NLI contradiction classifier)" \
    "https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/onnx/model.onnx" \
    "$INSTALL_DIR/models/nli.onnx" \
    "328"

download_model \
    "nli_tokenizer.json (NLI vocabulary)" \
    "https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/tokenizer.json" \
    "$INSTALL_DIR/models/nli_tokenizer.json" \
    "1"

# ── set permissions ───────────────────────────────────────────────────────────

# allow the installing user (not just root) to write to data/ and log/
REAL_USER="${SUDO_USER:-$USER}"
chown -R "$REAL_USER:$REAL_USER" "$INSTALL_DIR/data" "$INSTALL_DIR/log" 2>/dev/null || true

# ── verify install ────────────────────────────────────────────────────────────

info "Verifying installation..."

MISSING=0
for f in tekmerdb tekmerdb-mcp tekmerdb-server.conf \
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

# ── done ──────────────────────────────────────────────────────────────────────

echo ""
info "Installation complete."
echo ""
echo "  Install location : $INSTALL_DIR"
echo "  Engine binary    : $INSTALL_DIR/tekmerdb"
echo "  MCP binary       : $INSTALL_DIR/tekmerdb-mcp"
echo "  Config file      : $INSTALL_DIR/tekmerdb-server.conf"
echo "  Models           : $INSTALL_DIR/models/"
echo "  Data             : $INSTALL_DIR/data/  (created on first run)"
echo "  Logs             : $INSTALL_DIR/log/   (created on first run)"
echo ""
echo "  To start the engine:"
echo "    cd $INSTALL_DIR && ./tekmerdb"
echo ""
echo "  To configure:"
echo "    edit $INSTALL_DIR/tekmerdb-server.conf"
echo "    then restart the engine"
echo ""