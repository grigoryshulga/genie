# syntax=docker/dockerfile:1.7
#
# Genie server: the Rust server, separately served web assets, and the pi harness that runs the agents.
# Guide: docs/platform/docker.md
#
#   docker build -t genie -f docker/Dockerfile .
#   docker compose -f docker/docker-compose.yml up -d
#   Local tracker: docker compose -p genie-tracker -f docker/docker-compose.tracker.yml up -d --build
#
# Build arguments (all optional):
#   EXTRA_APT_PACKAGES         toolchains your agents need, e.g. "python3 python3-venv build-essential"
#   RUST_TOOLCHAIN             a Rust toolchain for agents on Rust projects ("stable", "1.94"): rustup's
#                              toolchain with clippy and rustfmt, a C compiler and the mold linker; the
#                              agents share one build cache under /data/cache (docker/configure.mjs)

ARG NODE_VERSION=24
ARG RUST_VERSION=1.94
ARG DEBIAN_RELEASE=bookworm

# ---- web UI -------------------------------------------------------------------------------------
FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS web
WORKDIR /src
COPY package.json package-lock.json tsconfig.json ./
COPY web/package.json ./web/package.json
RUN --mount=type=cache,target=/root/.npm \
    npm ci --workspace genie-web --include-workspace-root=false --ignore-scripts --no-audit --no-fund
COPY web ./web
RUN npm run build:web \
 && node web/compress-assets.mjs web/dist

# ---- pi -----------------------------------------------------------------------------------------
# The harness agents run: Pi-Bolt, the Pi coding agent compiled ahead of time into one native
# executable — the same commands, sessions, settings, providers and extensions, with agents
# starting ~13× sooner and long sessions using a fraction of the memory. The release's npm
# package is pinned by its SHA-256; only registry.npmjs.org is contacted (the wrapper package's
# own installer would also fetch GitHub, which a build network may not reach).
FROM debian:${DEBIAN_RELEASE}-slim AS pi
ARG PI_BOLT_VERSION=0.8.5
ARG PI_BOLT_SHA256=409dcca13b31474e3f66890d3ce3f194efad5c8f1cc4e800d126bdb712608542
WORKDIR /opt/genie/pi
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl xz-utils \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL "https://registry.npmjs.org/pi-bolt-linux-x64/-/pi-bolt-linux-x64-${PI_BOLT_VERSION}.tgz" -o bolt.tgz \
 && echo "${PI_BOLT_SHA256}  bolt.tgz" | sha256sum -c - \
 && tar xzf bolt.tgz package/pi-bolt-linux-x64.tar.xz \
 && tar xJf package/pi-bolt-linux-x64.tar.xz \
 && rm -rf bolt.tgz package \
 && pi-bolt-linux-x64/pi --version

# ---- server -------------------------------------------------------------------------------------
# Debian-based on purpose: rusqlite (bundled SQLite) and aws-lc need a C toolchain, and glibc here
# matches the runtime image.
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS server
WORKDIR /src
# Only the manifests and the build script are inputs here: `Cargo.toml`, `Cargo.lock`, the workspace
# members and the genie build script, whose `brotli`/`flate2` build-dependencies are compiled too.
# Adding a workspace member to the root Cargo.toml without its manifest here fails loudly below.
COPY Cargo.toml Cargo.lock ./
COPY crates/genie-core/Cargo.toml ./crates/genie-core/Cargo.toml
COPY crates/genie/Cargo.toml ./crates/genie/Cargo.toml
COPY crates/genie/build.rs ./crates/genie/build.rs
# Compiled dependencies (around 180 crates: aws-lc, bundled SQLite, axum) go into an ordinary layer,
# not a BuildKit cache mount: `cache-to: type=gha` exports layers and preserves this one between CI
# runs, while Docker does not preserve cache mounts in GitHub Actions. The workspace members are
# stubbed so this layer holds third-party artifacts only, and their artifacts are then cleaned, so the
# real sources can never be mistaken for the stubs. Without that clean, a source copied with an older
# mtime than the stub artifact in this layer is considered up to date and a stub binary is linked.
# `cargo clean -p` is deliberate: matching artifact paths by hand would miss fingerprints.
RUN set -eux; \
    mkdir -p crates/genie-core/src crates/genie/src; \
    : > crates/genie-core/src/lib.rs; \
    : > crates/genie/src/lib.rs; \
    printf 'fn main() {}\n' > crates/genie/src/main.rs; \
    cargo build --release --locked -p genie; \
    cargo clean --release -p genie -p genie-core; \
    test ! -e target/release/genie
# config/, agents/ and crates/genie/pi/ are embedded into the binary.
# Keep web assets in the runtime layer so UI-only edits reuse the entire Rust build.
COPY config ./config
COPY agents ./agents
COPY crates ./crates
RUN cargo build --release --locked -p genie \
 && install -D -m 0755 target/release/genie /out/genie \
 && test -n "$(/out/genie --version)" \
 && test -n "$(/out/genie --help)"

# Strip Node before copying it into the runtime: stripping in a later runtime layer
# would retain the original binary underneath and increase the image size.
FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS node-runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends binutils \
 && strip --strip-unneeded /usr/local/bin/node

# ---- runtime ------------------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime
COPY --from=node-runtime /usr/local/ /usr/local/
COPY --from=node-runtime /opt/ /opt/

ARG EXTRA_APT_PACKAGES=""
ARG RUST_TOOLCHAIN=""

# What agents (pi and the shell commands it runs) and the server itself need:
#   tini            PID 1 (after the privilege drop): reaps processes agents leave behind, forwards signals
#   git, ssh        worktrees, commits, pushes; the vault is a git repository
#   ripgrep         pi's search tool
#   bubblewrap      the agent sandbox (needs docker/docker-compose.sandbox.yml, see docs/platform/docker.md)
#   procps          `kill`, used by the server to stop agent processes
#   curl, jq        health check; everyday shell tooling for agents
#   ca-certificates TLS to model providers, Telegram, SMTP, MCP servers
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash bubblewrap ca-certificates curl git jq less libstdc++6 openssh-client procps ripgrep tini tzdata ${EXTRA_APT_PACKAGES} \
 && rm -rf /var/lib/apt/lists/*

# A Rust toolchain for agents (RUST_TOOLCHAIN): read-only under /opt/rust, its binaries on PATH without
# rustup's proxies, so it works for any user and HOME. Without it agents would install their own.
RUN if [ -n "$RUST_TOOLCHAIN" ]; then \
      apt-get update \
   && apt-get install -y --no-install-recommends build-essential pkg-config mold \
   && rm -rf /var/lib/apt/lists/* \
   && curl -fsSL https://sh.rustup.rs | RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo \
        sh -s -- -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN" -c clippy,rustfmt \
   && ln -s /opt/rust/rustup/toolchains/*/bin/* /usr/local/bin/ \
   && rm -rf /opt/rust/cargo /opt/rust/rustup/downloads /opt/rust/rustup/tmp \
   && chmod -R a+rX /opt/rust \
   && cargo --version; \
    fi

# The service user. Its ids are remapped at start (GENIE_UID / GENIE_GID) to match mounted repositories.
# The Debian base has no application user.
RUN groupadd --gid 1000 genie \
 && useradd --uid 1000 --gid genie --home-dir /data/home --no-create-home --shell /bin/bash genie

# Pi-Bolt, from the stage above: one native executable with its WASM helper, pinned by the
# SHA-256 in that stage, exposed as `pi` — genie's command templates and doctor find it under
# the name they know (and `pi-bolt` for the operator). MCP needs nothing more: pi has its own
# MCP support, which genie's guard extension wires up per agent.
COPY --from=pi /opt/genie/pi/pi-bolt-linux-x64 /opt/genie/pi-bolt
RUN ln -s /opt/genie/pi-bolt/pi /usr/local/bin/pi \
 && ln -s /opt/genie/pi-bolt/pi /usr/local/bin/pi-bolt \
 && pi --version

COPY docker/gitconfig /etc/gitconfig
RUN printf 'Host *\n    StrictHostKeyChecking accept-new\n    BatchMode yes\n' > /etc/ssh/ssh_config.d/genie.conf

COPY docker/configure.mjs docker/load-secrets.sh /usr/local/lib/genie/
COPY docker/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY docker/genie-cli.sh /usr/local/bin/genie
COPY --from=server /out/genie /opt/genie/bin/genie
COPY --from=web --chown=genie:genie /src/web/dist /opt/genie/web
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh /usr/local/bin/genie

RUN mkdir -p /data /workspace \
 && chown genie:genie /data /workspace

ENV GENIE_DATA=/data \
    GENIE_PORT=7420 \
    HOME=/data/home \
    LANG=C.UTF-8 \
    GIT_TERMINAL_PROMPT=0 \
    PI_SKIP_VERSION_CHECK=1 \
    PI_TELEMETRY=0

# /data: server.db, project trackers, the knowledge vault, agent sessions, pi credentials and settings (HOME).
# /workspace: git repositories of projects and their team worktrees (`<repo>.worktrees/`).
VOLUME ["/data", "/workspace"]
WORKDIR /workspace
EXPOSE 7420

# The server shuts down gracefully on SIGINT only (Ctrl-C); SIGTERM would kill it outright.
STOPSIGNAL SIGINT

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${GENIE_PORT}/api/health" >/dev/null || exit 1

LABEL org.opencontainers.image.title="genie" \
      org.opencontainers.image.description="Genie server: tasks, knowledge and agent teams" \
      org.opencontainers.image.source="https://github.com/grigoryshulga/genie" \
      org.opencontainers.image.licenses="MIT"

# Starts as root (volume ownership, uid mapping), drops to the service user, then runs under tini.
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["serve", "--web", "/opt/genie/web"]
