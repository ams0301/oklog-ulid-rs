# Dockerfile for the oklog-ulid-rs Port-Mortem submission.
#
# Single-command build of the working binary in an environment
# judges can run with one command. The image embeds the Rust toolchain
# and produces:
#   - the library (liboklog_ulid.rlib)
#   - the CLI binary (oklog-ulid)
#   - the differential fuzz harness (diff_fuzz)
#   - the bench harness (ulid_bench)
#
# Usage:
#   docker build -t oklog-ulid-rs .
#   docker run --rm oklog-ulid-rs cargo run --release --bin oklog-ulid
#   docker run --rm oklog-ulid-rs cargo test --release
#   docker run --rm oklog-ulid-rs ./target/release/diff_fuzz 60
#   docker run --rm oklog-ulid-rs ./target/release/ulid_bench

FROM rust:1.97-slim-bookworm AS builder

# Pre-create the workspace so we can cache the build artifacts.
WORKDIR /port

# Copy manifest first so cargo can resolve & download deps early when the
# manifests change. This port has zero external deps so it's a no-op, but
# kept for forward compatibility if deps are added later.
COPY Cargo.toml ./

# Now copy the source tree.
COPY src/ ./src/
COPY tests/ ./tests/
COPY fuzz/  ./fuzz/
COPY bench/ ./bench/
COPY reference/ ./reference/
COPY README.md ./README.md
COPY DECISIONS.md ./DECISIONS.md
COPY LICENSE-APACHE ./LICENSE-APACHE
COPY .port-mortem.toml ./.port-mortem.toml

# Single build command — the rules require this. Builds lib + 3 bins with
# `--release` so judges see the same numbers we report in bench/results.json.
RUN cargo build --release

# Tests can be re-run inside the image if judges want green test output:
# RUN cargo test --release

# Stamp a small banner image.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /port
COPY --from=builder /port/target/release/oklog-ulid    ./oklog-ulid
COPY --from=builder /port/target/release/diff_fuzz    ./diff_fuzz
COPY --from=builder /port/target/release/ulid_bench    ./ulid_bench
COPY --from=builder /port/target/release/liboklog_ulid.*  ./ 2>/dev/null || true
COPY --from=builder /port/reference ./reference
COPY --from=builder /port/DECISIONS.md ./DECISIONS.md
COPY --from=builder /port/.port-mortem.toml ./.port-mortem.toml

# Default entry point: drop into a shell so judges can run any of the
# artifacts (./oklog-ulid --help, ./diff_fuzz 60, ./ulid_bench, etc).
CMD ["/bin/bash"]
