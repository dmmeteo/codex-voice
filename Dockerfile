# syntax=docker/dockerfile:1.7

FROM rust:1.98-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config && \
    rm -rf /var/lib/apt/lists/*

ARG RUST_SILK_VERSION=0.1.3

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked --bin codex-voice && \
    cp /app/target/release/codex-voice /usr/local/bin/codex-voice

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install rust-silk --version "${RUST_SILK_VERSION}" --locked --root /usr/local

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --gid nogroup --home-dir /nonexistent --shell /usr/sbin/nologin codex-voice

COPY LICENSE-MIT THIRD_PARTY_NOTICES.md /usr/share/doc/codex-voice/
COPY --from=builder /usr/local/bin/codex-voice /usr/local/bin/codex-voice
COPY --from=builder /usr/local/bin/rust-silk /usr/local/bin/rust-silk

ENV CODEX_VOICE_SILK_DECODER=/usr/local/bin/rust-silk
USER 10001:65534
EXPOSE 2600

# The container's own interface; publish it on host loopback only, for
# example `docker run -p 127.0.0.1:2600:2600 ...`.
ENTRYPOINT ["codex-voice"]
CMD ["serve", "--host", "0.0.0.0", "--port", "2600"]
