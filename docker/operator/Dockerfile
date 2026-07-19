# Multi-stage build for the fah-operator binary.
# Build context is the repository root:
#   docker build -t ghcr.io/ewohltman/fah-operator:latest -f docker/operator/Dockerfile .

FROM rust:1-slim-bookworm AS builder
WORKDIR /usr/src/fah-operator

# Build dependencies first for better layer caching.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src src/bin \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && echo 'fn main() {}' > src/bin/crdgen.rs \
    && cargo build --release --bin fah-operator \
    && rm -rf src

# Build the real binary.
COPY src ./src
RUN touch src/main.rs src/lib.rs \
    && cargo build --release --bin fah-operator

# Minimal, non-root runtime image.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /usr/src/fah-operator/target/release/fah-operator /usr/local/bin/fah-operator
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/fah-operator"]
