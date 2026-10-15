#!/usr/bin/env bash
# Container entrypoint.
#
#   as root:      map the service user to GENIE_UID/GENIE_GID, fix ownership of the volumes, drop privileges
#   as the user:  become tini's child (PID 1), apply the environment to config.json, start `genie`
#
# Usage:
#   docker run … genie                         → genie serve (the default command)
#   docker run … genie serve --no-agents       → serve with extra flags
#   docker run … genie user add anna …         → any genie subcommand
#   docker run … genie bash                    → any other command, as the service user
set -euo pipefail

GENIE_BIN=/opt/genie/bin/genie
GENIE_USER=genie

log() { printf 'genie-entrypoint: %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

export GENIE_DATA="${GENIE_DATA:-/data}"
export GENIE_PORT="${GENIE_PORT:-7420}"
export GENIE_WORKSPACE="${GENIE_WORKSPACE:-/workspace}"
case "$GENIE_PORT" in '' | *[!0-9]*) die "GENIE_PORT must be a number, got '$GENIE_PORT'" ;; esac

# --- secrets: NAME_FILE=/run/secrets/x  →  NAME=<contents of the file> ---------------------------------
# Done first, as root, so files readable only by root work too.
# shellcheck source=docker/load-secrets.sh
. /usr/local/lib/genie/load-secrets.sh
load_secrets strict || exit 1

# --- as root: users, ownership, privilege drop ----------------------------------------------------

if [ "$(id -u)" = 0 ] && [ -z "${GENIE_ENTRYPOINT_DROPPED:-}" ]; then
  uid="${GENIE_UID:-$(id -u "$GENIE_USER")}"
  gid="${GENIE_GID:-$uid}"
  for v in "$uid" "$gid"; do
    case "$v" in '' | *[!0-9]*) die "GENIE_UID/GENIE_GID must be numbers, got '$v'" ;; esac
  done
  [ "$uid" != 0 ] && [ "$gid" != 0 ] ||
    die "GENIE_UID/GENIE_GID = 0: agents run arbitrary shell commands and must not run as root; use the default (1000) or your host user's ids"

  # Same ids as the owner of the mounted repositories: git refuses foreign owners and agents need to write.
  if [ "$uid" != "$(id -u "$GENIE_USER")" ] || [ "$gid" != "$(id -g "$GENIE_USER")" ]; then
    groupmod -o -g "$gid" "$GENIE_USER"
    usermod -o -u "$uid" -g "$gid" "$GENIE_USER"
  fi

  home="$(getent passwd "$GENIE_USER" | cut -d: -f6)"
  mkdir -p "$GENIE_DATA" "$home" "$GENIE_WORKSPACE"
  # Recursive only when the top directory is foreign (fresh volume, changed ids): a long-lived data
  # directory is not re-walked on every start, and mounted repositories are never touched recursively.
  if [ "$(stat -c '%u:%g' "$GENIE_DATA")" != "$uid:$gid" ]; then
    log "taking ownership of $GENIE_DATA for $uid:$gid"
    chown -R "$uid:$gid" "$GENIE_DATA"
  fi
  [ "$(stat -c '%u:%g' "$home")" = "$uid:$gid" ] || chown -R "$uid:$gid" "$home"
  [ "$(stat -c '%u:%g' "$GENIE_WORKSPACE")" = "$uid:$gid" ] || chown "$uid:$gid" "$GENIE_WORKSPACE"

  export GENIE_ENTRYPOINT_DROPPED=1 HOME="$home" USER="$GENIE_USER" LOGNAME="$GENIE_USER"
  exec setpriv --reuid="$uid" --regid="$gid" --init-groups "$0" "$@"
fi

# --- as the service user ---------------------------------------------------------------------------

# PID 1 must reap the many short-lived processes agents leave behind and forward signals to genie.
# tini takes over only now, after the privilege drop: it runs as the service user, so it can signal
# genie without CAP_KILL and nothing in the container keeps running as root.
if [ "$$" = 1 ] && command -v tini >/dev/null 2>&1; then
  exec tini -- "$0" "$@"
fi

# Docker sets HOME=/ or nothing for arbitrary `--user` ids.
if [ -z "${HOME:-}" ] || [ "$HOME" = / ] || [ ! -w "$HOME" ]; then
  export HOME="$GENIE_DATA/home"
fi
mkdir -p "$HOME" 2>/dev/null || true

[ -d "$GENIE_DATA" ] && [ -w "$GENIE_DATA" ] ||
  die "$GENIE_DATA is not writable by uid $(id -u). Fix the volume's owner (chown $(id -u):$(id -g) …) or set GENIE_UID/GENIE_GID and start the container as root (the default)."

node /usr/local/lib/genie/configure.mjs

# ssh refuses keys with loose permissions; the directory itself is ours to fix.
[ -d "$HOME/.ssh" ] && chmod 700 "$HOME/.ssh" 2>/dev/null || true

# --- dispatch ------------------------------------------------------------------------------------------

cmd="${1:-serve}"
[ "$#" -gt 0 ] && shift

case "$cmd" in
  serve)
    if ! command -v pi >/dev/null 2>&1; then
      log "warning: pi is not installed: agents cannot start"
    fi
    agent_dir="${PI_CODING_AGENT_DIR:-$HOME/.pi/agent}"
    if [ ! -s "$agent_dir/auth.json" ] && [ ! -s "$agent_dir/models.json" ] &&
      ! env | grep -Eq '^(ANTHROPIC|OPENAI|GEMINI|DEEPSEEK|MISTRAL|GROQ|XAI|OPENROUTER|AZURE_OPENAI|AWS_BEARER_TOKEN_BEDROCK|GOOGLE_CLOUD|CEREBRAS|TOGETHER|FIREWORKS|HF|COPILOT_GITHUB)[A-Z_]*=.+'; then
      log "note: no model credentials found (provider API key variables, auth.json or models.json in $agent_dir): the web UI works, agents cannot call models"
    fi
    # The UI can be embedded in genie or supplied by a runtime image with --web.
    args=(serve)
    has_port=0
    for a in "$@"; do
      case "$a" in --port | --port=*) has_port=1 ;; esac
    done
    [ "$has_port" = 1 ] || args+=(--port "$GENIE_PORT")
    log "starting: genie ${args[*]} $* (data $GENIE_DATA, uid $(id -u))"
    exec "$GENIE_BIN" "${args[@]}" "$@"
    ;;
  task | team | mail | docs | job | automation | agents | project | user | me | server | orchestrate | member | invite | backup | stats | doctor | vault | agent | init | help | -h | --help | -V | --version)
    exec "$GENIE_BIN" "$cmd" "$@"
    ;;
  *)
    exec "$cmd" "$@"
    ;;
esac
