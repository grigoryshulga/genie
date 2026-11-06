#!/usr/bin/env bash
# One incoming request, exercised through the same scoped CLI as a real intake agent.
set -euo pipefail
[[ "$GENIE_AGENT_NAME" == request:* ]] || exit 0
response="${GENIE_AGENT_NAME#request:}"
message="${!#}"
if [ "${REQUEST_INPUT:-}" = stdin ]; then
  IFS= read -r -d '' message || :
elif [ "${1:-}" = --message-file ]; then
  IFS= read -r -d '' message < "$2" || :
fi
capture="${GENIE_MARK%/*}/requests"
mkdir -p "$capture"
printf '%s' "$PWD" > "$capture/$response.cwd"
printf '%s' "${!#}" > "$capture/$response.argument"
printf '%s' "$message" > "$capture/$response.message"
case "${REQUEST_MODE:-reply}" in
  no_reply) exit 0 ;;
  crash) exit 7 ;;
  wait)
    until [ -f "$capture/release" ]; do sleep 0.05; done
    ;;
esac
"$GENIE_BIN" conversation show "$response" >/dev/null
printf '%s\n' "$message"
printf '%s' "$message" | "$GENIE_BIN" conversation reply "$response" --text - >/dev/null
