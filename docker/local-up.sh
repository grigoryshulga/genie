#!/usr/bin/env bash
# Start (or update) the local genie container. The LiteLLM key goes from Bitwarden into a 0600 file on
# tmpfs (/run/user/<uid>), which compose mounts as a secret; it is never written to the disk.
#
#   docker/local-up.sh [build|up|down|logs …]     (default: up -d --build)
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
key=/run/user/$(id -u)/genie-litellm-key
if [ ! -s "$key" ]; then
  ( umask 077; bws-run bash -c 'printf %s "$LITELLM_API_KEY"' > "$key" )
  [ -s "$key" ] || { echo "local-up: no LITELLM_API_KEY from bws-run" >&2; rm -f "$key"; exit 1; }
fi
files=(-f docker-compose.yml -f docker-compose.sandbox.yml -f docker-compose.local.yml)
case "${1:-up}" in
  up) shift || true; exec pkexec docker compose "${files[@]}" up -d --build "$@" ;;
  *) exec pkexec docker compose "${files[@]}" "$@" ;;
esac
