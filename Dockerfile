# PUBLIC-TESTNET-DEPLOY-AND-SEED-V1 — operator-facing reference
# Dockerfile for the DeOpt V2 backend (Rust + axum + sqlx).
#
# This image is host-agnostic: it produces a single-binary runtime
# that any container PaaS (Fly, Railway, Render, Cloud Run, ECS,
# etc.) can run. The operator must supply env vars at runtime —
# this image embeds NO secrets.
#
# Usage:
#   docker build -t deopt-v2-backend .
#   docker run --rm -p 8080:8080 \
#     -e PERSISTENCE_ENABLED=true \
#     -e DATABASE_URL=postgres://... \
#     -e CHAIN_ID=84532 \
#     -e OPTIONS_ENABLED=true \
#     -e CORS_ALLOWED_ORIGINS=https://your-frontend \
#     -e HOST=0.0.0.0 -e PORT=8080 \
#     deopt-v2-backend
#
# The container listens on $PORT (defaults to 8080) and runs DB
# migrations automatically on startup when PERSISTENCE_ENABLED=true.
#
# Health endpoints (for PaaS health checks):
#   GET /health   always 200 when the HTTP server is up
#   GET /ready    200 when ready, 503 SERVICE_UNAVAILABLE otherwise
#
# Build stage uses rust:1.83-bookworm (matches rust-version =
# "1.75" minimum + recent stable features). The final stage is
# debian:bookworm-slim — small but ships glibc so sqlx/openssl link
# cleanly without statically-linked-musl gotchas.

# ---- builder ------------------------------------------------------
FROM rust:1.83-bookworm AS builder
WORKDIR /build

# System deps: pkg-config + openssl headers for sqlx-postgres +
# any rustls-via-openssl crates; libpq is NOT required (sqlx uses
# its own protocol implementation).
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Copy the manifest first so cargo can warm the dependency cache
# (.dockerignore should keep target/ + node_modules/ out of context).
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations

# Build the release binary. SQLX_OFFLINE=true so sqlx macros use
# the committed query metadata rather than a live DB; if your
# build uses an inline migration helper that needs DATABASE_URL,
# adjust here.
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin deopt-v2-backend

# ---- runtime -----------------------------------------------------
FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1001 --home-dir /app deopt

# Bring the binary + migrations dir (sqlx::migrate!() embeds them
# at compile time, but copying the directory keeps the artefact
# self-describing for operators inspecting the image).
COPY --from=builder /build/target/release/deopt-v2-backend /usr/local/bin/deopt-v2-backend
COPY --from=builder /build/migrations /app/migrations

USER deopt
ENV HOST=0.0.0.0 \
    PORT=8080 \
    RUST_LOG=info

EXPOSE 8080

# Health check uses /health (no DB dependency); use /ready in
# PaaS-level readiness probes instead.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD wget -qO- "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/local/bin/deopt-v2-backend"]
