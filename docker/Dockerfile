# Self-contained TekmerDB engine + MCP server image, scoped to a single
# domain (EU AI Act risk category) chosen at container start via the DOMAIN
# env var. Models are baked in at build time so the container needs no
# internet access to start.
#
# Build (from repo root):
#   docker build -f docker/Dockerfile -t tekmerdb:latest .
#
# Run (one instance per domain; engine port stays host-loopback-only,
# mcp port is meant to be reachable by remote MCP clients):
#   docker run -d --name tekmerdb-healthcare \
#     -e DOMAIN=Healthcare \
#     -p 127.0.0.1:3100:3000 \
#     -p 0.0.0.0:3101:3001 \
#     tekmerdb:latest

# ---------- build stage ----------
FROM almalinux:10 AS builder

RUN dnf install -y gcc gcc-c++ make curl openssl-devel zlib-devel && dnf clean all

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --bin tekmerdb --bin tekmerdb-mcp

# Same model set install.sh downloads onto bare-metal installs.
RUN mkdir -p models \
    && curl -fL -o models/miniLM.onnx https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx \
    && curl -fL -o models/tokenizer.json https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json \
    && curl -fL -o models/nli.onnx https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/onnx/model.onnx \
    && curl -fL -o models/nli_tokenizer.json https://huggingface.co/cross-encoder/nli-MiniLM2-L6-H768/resolve/main/tokenizer.json

# ---------- runtime stage ----------
FROM almalinux:10 AS runtime

RUN dnf install -y openssl-libs zlib curl-minimal && dnf clean all \
    && useradd --system --no-create-home --shell /sbin/nologin tekmerdb

WORKDIR /opt/tekmerdb
COPY --from=builder /build/target/release/tekmerdb /build/target/release/tekmerdb-mcp ./
COPY --from=builder /build/models ./models
COPY tekmerdb-server.conf ./tekmerdb-server.conf.template
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x tekmerdb tekmerdb-mcp /usr/local/bin/entrypoint.sh \
    && chown -R tekmerdb:tekmerdb /opt/tekmerdb

USER tekmerdb
ENV DOMAIN=General

# 3000 = engine (publish to host-loopback only, never 0.0.0.0 — see entrypoint.sh)
# 3001 = MCP SSE server (meant for public/remote reachability)
EXPOSE 3000 3001

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fs http://127.0.0.1:3000/status || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
