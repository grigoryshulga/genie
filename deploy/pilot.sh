#!/bin/sh
# Run the pilot instance of genie next to the working server.
#
#   ./deploy/pilot.sh              # foreground; Ctrl-C stops it
#   ./deploy/pilot.sh --no-agents  # UI and automations only, no agent processes
#
# The instance gets its own data directory, port, worktree root and task prefix,
# while serving the same checkout. The LiteLLM key comes from Bitwarden Secrets
# Manager through `bws-run` and is never written to a file. Knobs:
# GENIE_PILOT_DATA, GENIE_PILOT_PORT, GENIE_PILOT_REPO, GENIE_PILOT_PREFIX,
# GENIE_PILOT_PROJECT, GENIE_BIN. Arguments are passed to `genie serve`.
#
# The working server on 7420 is never touched: not its config, not its data.
# Documented in docs/platform/pilot.md.
set -eu

self=$0
if command -v readlink >/dev/null 2>&1; then
  resolved=$(readlink -f -- "$self" 2>/dev/null) && [ -n "$resolved" ] && self=$resolved
fi
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$self")" && pwd)
SELF=$SCRIPT_DIR/$(basename -- "$self")
TEMPLATE=$SCRIPT_DIR/pilot.config.json

DATA=${GENIE_PILOT_DATA:-$HOME/.local/share/genie-pilot}
PORT=${GENIE_PILOT_PORT:-7421}
PROJECT=${GENIE_PILOT_PROJECT:-genie}
PREFIX=${GENIE_PILOT_PREFIX:-PIL}
GENIE_BIN=${GENIE_BIN:-genie}

# The checkout the pilot serves: the main one, also when this script is run from
# a worktree of it (a worktree would make the pilot's repository that worktree).
if [ -n "${GENIE_PILOT_REPO:-}" ]; then
  REPO=$GENIE_PILOT_REPO
else
  REPO=$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)
  common=$(git -C "$REPO" rev-parse --git-common-dir 2>/dev/null) || common=
  case "$common" in
    /*) REPO=$(CDPATH='' cd -- "$(dirname -- "$common")" && pwd) ;;
    ?*) REPO=$(CDPATH='' cd -- "$REPO/$common/.." && pwd) ;;
  esac
fi

# 1. The key. bws-run prepares the environment from the credential cache and
#    re-runs this script; the path must be absolute, a bare name is a harness.
if [ -z "${BWS_WRAPPED:-}" ]; then
  if [ -n "${GENIE_PILOT_BWS_RUN:-}" ]; then
    bws=$GENIE_PILOT_BWS_RUN
  elif [ -x "$HOME/.config/bws/bws-agent" ]; then
    bws=$HOME/.config/bws/bws-agent
  elif command -v bws-run >/dev/null 2>&1; then
    bws=$(command -v bws-run)
  else
    echo "pilot: bws-run not found: install the BWS launchers or set GENIE_PILOT_BWS_RUN" >&2
    exit 1
  fi
  echo "pilot: taking the LiteLLM key from Bitwarden through $bws" >&2
  exec "$bws" "$SELF" "$@"
fi

# 2. bws-run continues without credentials when it has none, and an instance
#    without a key cannot start a single litellm/* agent: stop instead.
if [ -z "${LITELLM_API_KEY:-}" ]; then
  echo "pilot: no LITELLM_API_KEY in the environment bws-run prepared." >&2
  echo "pilot: agents on litellm/* would not start; run bws-status to diagnose, then start again." >&2
  exit 1
fi

# 3. Operations follow GENIE_URL/GENIE_TOKEN, not --data: without this, the
#    bootstrap below would talk to the working server on 7420.
unset GENIE_URL GENIE_TOKEN GENIE_TASK GENIE_TEAM GENIE_PROJECT

# 4. One instance per port, and 7420 belongs to the working server.
if command -v ss >/dev/null 2>&1; then
  if ss -ltn 2>/dev/null | awk '{print $4}' | grep -q ":$PORT\$"; then
    echo "pilot: port $PORT is already in use (the working server is 7420): free it or set GENIE_PILOT_PORT" >&2
    exit 1
  fi
else
  echo "pilot: no ss: cannot check whether port $PORT is free" >&2
fi

# 4b. And one instance per data directory. The port check misses a second launch
#     of a first run that is still bootstrapping, and two servers on one data
#     directory deadlock in SQLite (`database is locked`) instead of failing.
#     fd 9 stays open across the exec below, so the lock lives as long as the
#     server does.
if command -v flock >/dev/null 2>&1; then
  mkdir -p "$DATA"
  exec 9>"$DATA/pilot.lock"
  if ! flock -n 9; then
    echo "pilot: $DATA is already used by another instance: stop it, or set GENIE_PILOT_DATA" >&2
    exit 1
  fi
else
  mkdir -p "$DATA"
fi

if ! command -v "$GENIE_BIN" >/dev/null 2>&1; then
  echo "pilot: $GENIE_BIN not found on PATH: build genie (deploy/install.sh) or set GENIE_BIN" >&2
  exit 1
fi

# 5. First run: config.json from the template (never overwritten later), then
#    the project. `--prefix PIL` keeps the pilot's task ids and branch names
#    (genie/PIL-3) apart from the working server's (genie/G-3).
if [ ! -f "$DATA/server.db" ]; then
  echo "pilot: first run, preparing $DATA" >&2
  if [ ! -f "$DATA/config.json" ]; then
    sed "s/^  \"port\":[[:space:]]*[0-9][0-9]*/  \"port\": $PORT/" "$TEMPLATE" > "$DATA/config.json"
  fi
  if ! grep -q "\"port\": $PORT" "$DATA/config.json"; then
    echo "pilot: $DATA/config.json does not name port $PORT: fix it (the template is $TEMPLATE)" >&2
    exit 1
  fi
  "$GENIE_BIN" --data "$DATA" project add "$PROJECT" --name "$PROJECT" --repo "$REPO" --prefix "$PREFIX"
fi

# 6. Hand over to the server. --port wins over config.json, but the file names
#    the port too: agents get GENIE_URL from it (`genie --data … serve` alone).
echo "pilot: data $DATA · repo $REPO · prefix $PREFIX" >&2
exec "$GENIE_BIN" --data "$DATA" serve --port "$PORT" "$@"
