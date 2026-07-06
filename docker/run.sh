#!/usr/bin/env bash
# Launches one domain-scoped tekmerdb container with hard resource limits.
#
# Docker has no way to bake resource limits into an image — they're cgroup
# limits applied at `docker run` time — so this script is the single place
# those limits live, instead of relying on whoever runs `docker run`
# remembering the right flags each time.
#
# Limits are sized for 3 concurrent containers on this box (2 vCPU / 3.5GB
# RAM, ~2.8GB available with Docker/Caddy/demo-site-web already running):
#   - memory 800m: measured idle usage is ~490MB per container; 800m leaves
#     growth headroom (HNSW index / Parquet buffers grow with PFO count)
#     while 3x800m = 2.4GB stays under the ~2.8GB available.
#   - cpus 0.5: 3x0.5 = 1.5 of the 2 cores, leaving 0.5 core for the host.
#   Docker enforces neither of these as a hard cap on *how many* containers
#   you can start — nothing stops a 4th — this just keeps 3 safely within
#   budget. If you need a 4th, lower these first and re-check headroom.
#
# Usage: docker/run.sh <domain> <engine-host-port> <mcp-host-port> [name]
# Example: docker/run.sh Healthcare 3100 3101 my-healthcare-demo
#
# [name] is the docker container name a demo visitor picked in the UI. It's
# validated by the Flask app (demo_site/app.py) before reaching this script,
# not here. Since it's caller-controlled, containers are discovered by the
# tekmerdb.demo=1 label below rather than by name prefix, so any name works.

set -euo pipefail

DOMAIN="${1:?usage: run.sh <domain> <engine-host-port> <mcp-host-port> [name]}"
ENGINE_PORT="${2:?usage: run.sh <domain> <engine-host-port> <mcp-host-port> [name]}"
MCP_PORT="${3:?usage: run.sh <domain> <engine-host-port> <mcp-host-port> [name]}"
NAME="${4:-tekmerdb-$(echo "$DOMAIN" | tr '[:upper:]' '[:lower:]')}"

docker run -d \
    --name "$NAME" \
    --label tekmerdb.demo=1 \
    --memory 800m \
    --memory-swap 800m \
    --cpus 0.5 \
    --restart unless-stopped \
    -e DOMAIN="$DOMAIN" \
    -p "127.0.0.1:${ENGINE_PORT}:3000" \
    -p "0.0.0.0:${MCP_PORT}:3001" \
    tekmerdb:latest
