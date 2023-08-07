
FROM rust:bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# We do not need the Rust toolchain to run the binary
FROM debian:bookworm-slim AS runtime
WORKDIR /app
# Workspace means we build in a target dir in the root of the workspace
COPY --from=builder /app/target/release/resticmgr /usr/local/bin
COPY restic_0.16.0_linux_amd64 /usr/local/bin/restic
# FIXME
CMD ["/usr/local/bin/resticmgr", "/resticmgr-config.json"]
