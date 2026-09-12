# --- Build stage ---
# Pinned to a toolchain that supports edition 2024 (required by transitive deps).
FROM rust:1.93.1-slim AS builder
WORKDIR /app

# Cache dependencies: build against a dummy main first, then the real sources.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY src ./src
RUN touch src/main.rs && cargo build --release

# --- Runtime stage ---
# Match the builder's Debian release (trixie) so glibc versions line up.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/speck-server /usr/local/bin/speck-server
# Railway injects PORT at runtime; SPECK_PORT is the local/default fallback.
ENV SPECK_PORT=8080
EXPOSE 8080
CMD ["speck-server"]
