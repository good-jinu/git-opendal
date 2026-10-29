# syntax=docker/dockerfile:1

# Default BUILD_MODE is 'source' for local `docker build .`
# In CI, pass `--build-arg BUILD_MODE=prebuilt` to package pre-compiled musl binaries.
ARG BUILD_MODE=source

# ── Stage 1: Build from source (used when BUILD_MODE=source) ──
FROM rust:1-alpine AS builder-source
WORKDIR /src
RUN apk add --no-cache musl-dev
COPY . .
RUN cargo build --release --locked --bins
RUN mkdir -p /out && \
    cp target/release/git-remote-opendal /out/git-remote-opendal && \
    cp target/release/git-opendal /out/git-opendal

# ── Stage 2: Pre-compiled binaries (used when BUILD_MODE=prebuilt) ──
FROM scratch AS builder-prebuilt
ARG TARGETARCH
COPY docker-bin/${TARGETARCH}/git-remote-opendal /out/git-remote-opendal
COPY docker-bin/${TARGETARCH}/git-opendal /out/git-opendal

# ── Stage 3: Selector ──
FROM builder-${BUILD_MODE} AS binaries

# ── Stage 4: Final runtime image ──
FROM alpine:3.20

# git-remote-opendal invokes git (bundle, fast-import), so git is required.
# ca-certificates is required for HTTPS calls to cloud storage services.
# openssh-client is standard for git environments.
RUN apk add --no-cache git ca-certificates openssh-client

COPY --from=binaries /out/git-remote-opendal /usr/local/bin/git-remote-opendal
COPY --from=binaries /out/git-opendal /usr/local/bin/git-opendal

RUN chmod +x /usr/local/bin/git-remote-opendal /usr/local/bin/git-opendal

ENTRYPOINT ["git-opendal"]
CMD ["--help"]
