# syntax=docker/dockerfile:1.7
#
# Genie server: one image with the Rust server (the web UI built in) and the pi harness that runs the agents.
# Guide: docs/platform/docker.md
#
#   docker build -t genie .
#   docker compose up -d           (see docker-compose.yml and .env.example)
#
# Build arguments (all optional):
#   PI_VERSION                 pi release; keep it at the version genie's tests run against (package-lock.json)
#   PI_MCP_ADAPTER_VERSION     pi-mcp-adapter release, shipped for roles with MCP connections
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
RUN --mount=type=cache,target=/root/.npm \
    npm ci --ignore-scripts --no-audit --no-fund
COPY web ./web
RUN npm run build:web

# ---- server -------------------------------------------------------------------------------------
# Debian-based on purpose: rusqlite (bundled SQLite) and aws-lc need a C toolchain, and glibc here
# matches the runtime image.
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# config/, agents/, crates/genie/pi/ and the web UI are embedded into the binary (include_str!, build.rs).
COPY config ./config
COPY agents ./agents
COPY crates ./crates
COPY --from=web /src/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    GENIE_WEB_DIST=web/dist cargo build --release --locked -p genie \
 && install -D -m 0755 target/release/genie /out/genie

# ---- runtime ------------------------------------------------------------------------------------
FROM node:${NODE_VERSION}-${DEBIAN_RELEASE}-slim AS runtime

ARG PI_VERSION=0.87.1
ARG PI_MCP_ADAPTER_VERSION=3.2.0
ARG EXTRA_APT_PACKAGES=""
ARG RUST_TOOLCHAIN=""

# What agents (pi and the shell commands it runs) and the server itself need:
#   tini            PID 1 (after the privilege drop): reaps processes agents leave behind, forwards signals
#   git, ssh        worktrees, commits, pushes; the vault is a git repository
#   ripgrep         pi's search tool
#   bubblewrap      the agent sandbox (needs docker-compose.sandbox.yml, see docs/platform/docker.md)
#   procps          `kill`, used by the server to stop agent processes
#   curl, jq        health check; everyday shell tooling for agents
#   ca-certificates TLS to model providers, Telegram, SMTP, MCP servers
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash bubblewrap ca-certificates curl git jq less openssh-client procps ripgrep tini tzdata ${EXTRA_APT_PACKAGES} \
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
# The base image's `node` user owns uid 1000; replace it.
RUN userdel -r node \
 && groupadd --gid 1000 genie \
 && useradd --uid 1000 --gid genie --home-dir /data/home --no-create-home --shell /bin/bash genie

# pi, exactly as its own containerization guide installs it, plus the MCP adapter as a local package
# (the entrypoint registers it in pi's settings; pi itself never has to install anything at run time).
RUN npm install -g --ignore-scripts --no-audit --no-fund "@earendil-works/pi-coding-agent@${PI_VERSION}" \
 && PI_CODING_AGENT_DIR=/opt/genie/pi-seed HOME=/tmp/pi-home \
      pi install "npm:pi-mcp-adapter@${PI_MCP_ADAPTER_VERSION}" \
 && rm -rf /tmp/pi-home /root/.npm \
 && pi --version

COPY docker/gitconfig /etc/gitconfig
RUN printf 'Host *\n    StrictHostKeyChecking accept-new\n    BatchMode yes\n' > /etc/ssh/ssh_config.d/genie.conf

COPY docker/configure.mjs docker/load-secrets.sh /usr/local/lib/genie/
COPY docker/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY docker/genie-cli.sh /usr/local/bin/genie
COPY --from=server /out/genie /opt/genie/bin/genie
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
CMD ["serve"]
