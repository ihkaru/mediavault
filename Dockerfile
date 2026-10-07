# ==============================================================================
# Multi-Stage Dockerfile with Cargo-Chef Layer Caching
# Designed for Fast GitHub App Auto-Deployments in Coolify
# ==============================================================================

# Stage 1: Cargo Chef Planner
FROM lukemathwalker/cargo-chef:latest-rust-1.93-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Cache & Build Dependencies Only
FROM chef AS builder
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this layer is cached by Docker across deployments
RUN cargo chef cook --release --recipe-path recipe.json

# Copy application source code and compile binary
COPY . .
RUN cargo build --release --bin mediavault

# Stage 3: Minimal, Secure Runtime Image
FROM debian:bookworm-slim AS runtime

# Install CA certificates and curl for container healthchecks
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root unprivileged user
RUN groupadd -g 10001 appuser && \
    useradd -u 10001 -g appuser -s /bin/false -m appuser

WORKDIR /app

# Copy compiled binary from builder stage
COPY --from=builder /app/target/release/mediavault /usr/local/bin/mediavault

# Create data directories with appropriate permissions
RUN mkdir -p /app/data/storage /app/data/temp && \
    chown -R appuser:appuser /app

USER appuser

ENV HOST=0.0.0.0 \
    PORT=8080 \
    DATABASE_URL=sqlite:/app/data/mediavault.db?mode=rwc \
    STORAGE_LOCAL_DIR=/app/data/storage \
    STORAGE_TEMP_DIR=/app/data/temp \
    RUST_LOG=mediavault=info,tower_http=info

EXPOSE 8080

HEALTHCHECK --interval=20s --timeout=5s --retries=3 --start-period=10s \
    CMD curl -f http://localhost:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/mediavault"]
