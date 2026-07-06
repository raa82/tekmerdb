#!/usr/bin/env bash
set -euo pipefail

DOMAIN="${DOMAIN:-General}"

# engine_host must be 0.0.0.0 *inside* the container for `docker run -p` to
# reach it at all — Docker's port publishing NATs to the container's bridge
# interface, which a process bound to the container's own 127.0.0.1 can never
# see. This does not weaken the "never expose 0.0.0.0 on bare metal" advice in
# tekmerdb-server.conf: the container's network namespace is isolated, and the
# actual host-level exposure is controlled entirely by how -p is invoked (see
# Dockerfile header — engine port should only ever be published to
# 127.0.0.1 on the host).
sed -e "s/^domain = .*/domain = \"${DOMAIN}\"/" \
    -e 's/^engine_host = .*/engine_host = "0.0.0.0"/' \
    tekmerdb-server.conf.template > tekmerdb-server.conf

./tekmerdb &
engine_pid=$!

for _ in $(seq 1 30); do
    curl -fs http://127.0.0.1:3000/status >/dev/null 2>&1 && break
    sleep 1
done

./tekmerdb-mcp --sse &
mcp_pid=$!

term() {
    kill -TERM "$engine_pid" "$mcp_pid" 2>/dev/null || true
}
trap term SIGTERM SIGINT

wait -n "$engine_pid" "$mcp_pid"
term
wait
