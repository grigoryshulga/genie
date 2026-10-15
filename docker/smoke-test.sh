#!/usr/bin/env bash
# Smoke test of a built image: starts it, checks the health endpoint and the web UI, creates a user
# through the CLI wrapper, checks file ownership, and stops it with the signal `docker stop` sends.
#
#   docker/smoke-test.sh genie:ci
set -euo pipefail

image="${1:?usage: smoke-test.sh <image>}"
name="genie-smoke-$$"
port="${SMOKE_PORT:-17420}"
fail() { echo "smoke test FAILED: $*" >&2; docker logs "$name" 2>&1 | tail -30 >&2 || true; exit 1; }
cleanup() { docker rm -f -v "$name" >/dev/null 2>&1 || true; }
trap cleanup EXIT

# The server compares the Host header with its own port, so the published port must equal it.
docker run -d --name "$name" -p "127.0.0.1:$port:$port" -e "GENIE_PORT=$port" \
  --cap-drop ALL --cap-add CHOWN --cap-add DAC_OVERRIDE --cap-add FOWNER --cap-add SETUID --cap-add SETGID \
  --security-opt no-new-privileges:true "$image" >/dev/null

status=starting
for _ in $(seq 1 45); do
  status="$(docker inspect -f '{{.State.Health.Status}}' "$name")"
  [ "$status" = healthy ] && break
  [ "$(docker inspect -f '{{.State.Running}}' "$name")" = true ] || fail "the container exited"
  sleep 2
done
[ "$status" = healthy ] || fail "not healthy after 90 s (status: $status)"

curl -fsS "http://127.0.0.1:$port/api/health" | grep -q '"ok":true' || fail "/api/health"
curl -fsS "http://127.0.0.1:$port/" | grep -qi '<div id="root"' || fail "the web UI is not served"

echo 'smoke-test-pw-1' | docker exec -i "$name" genie user add smoke --admin --password-stdin | grep -q 'user smoke' ||
  fail "genie user add"
[ -z "$(docker exec "$name" find /data ! -user genie -print -quit)" ] || fail "files in /data not owned by genie"
[ "$(docker exec "$name" ps -o user= -p 1 | tr -d ' ')" = genie ] || fail "PID 1 does not run as the service user"
docker exec -u genie "$name" pi --version >/dev/null || fail "pi is not runnable"
docker exec -u genie "$name" git --version >/dev/null || fail "git is not runnable"

docker kill --signal SIGINT "$name" >/dev/null
for _ in $(seq 1 20); do
  [ "$(docker inspect -f '{{.State.Running}}' "$name")" = false ] && break
  sleep 1
done
[ "$(docker inspect -f '{{.State.Running}}' "$name")" = false ] || fail "did not stop within 20 s"
[ "$(docker inspect -f '{{.State.ExitCode}}' "$name")" = 0 ] || fail "exit code $(docker inspect -f '{{.State.ExitCode}}' "$name") on SIGINT"

# The agent sandbox (bubblewrap) needs seccomp and /proc unconfined (docker/docker-compose.sandbox.yml). Whether
# the host's kernel allows it varies, so this only fails the test when SMOKE_REQUIRE_SANDBOX=1.
sandbox_name="$name-sandbox"
trap 'docker rm -f -v "$name" "$sandbox_name" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$sandbox_name" \
  --cap-drop ALL --cap-add CHOWN --cap-add DAC_OVERRIDE --cap-add FOWNER --cap-add SETUID --cap-add SETGID \
  --security-opt no-new-privileges:true --security-opt seccomp=unconfined --security-opt systempaths=unconfined \
  --security-opt apparmor=unconfined "$image" >/dev/null
for _ in $(seq 1 30); do
  [ "$(docker inspect -f '{{.State.Health.Status}}' "$sandbox_name")" = healthy ] && break
  sleep 2
done
if docker exec -u genie "$sandbox_name" genie agents check 2>&1 | grep -q 'agents run in a bubblewrap sandbox'; then
  echo "sandbox: works with the docker/docker-compose.sandbox.yml options"
elif [ "${SMOKE_REQUIRE_SANDBOX:-0}" = 1 ]; then
  fail "the agent sandbox does not work with the docker/docker-compose.sandbox.yml options: $(docker exec -u genie "$sandbox_name" genie agents check 2>&1 | grep -i sandbox)"
else
  echo "sandbox: NOT working on this host (not required): $(docker exec -u genie "$sandbox_name" genie agents check 2>&1 | grep -i sandbox)"
fi

echo "smoke test passed: $image"
