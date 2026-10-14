#!/bin/sh
# The kill -9 drill (G-133, the scenario of G-79): a real pi agent on a real model works a
# task, and both the agent and the server are killed with -9 in the middle of the work.
# After the restarts the drill asserts: every marker comment arrived exactly once (nothing
# lost, nothing duplicated), no letter was delivered twice, no orphan processes or worktrees
# are left, the work finished and the task is not stuck in progress.
#
# One throwaway directory, its own data and port: nothing here touches a running server.
# The LiteLLM key comes from bws-run (like docker/local-up.sh) and stays in the
# environment of the drill's server and its agents only.
#
#   deploy/crash-drill.sh [--bin <genie>] [--work <dir>] [--port <n>] [--model <litellm/…>]
#                         [--task-seconds <n>] [--keep]
#
#   --bin            the binary under test (default: this checkout's target/release/genie,
#                    else target/debug/genie)
#   --work           where the stand goes (default: a fresh mktemp -d; --keep leaves it)
#   --port           the stand's port (default: 7431)
#   --model          the executor's model (default: litellm/deepseek-v4-flash-vision-exp)
#   --task-seconds   how long the agent's work pauses between files (default: 8)
#
# Exit status: 0 when every step behaved as expected, 1 on the first surprise.
# Runtime: a few minutes — the agent really works, with real model latency.

set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=
WORK=
PORT=7431
MODEL=litellm/deepseek-v4-flash-vision-exp
TASK_SECONDS=8
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN=$2; shift 2 ;;
    --work) WORK=$2; shift 2 ;;
    --port) PORT=$2; shift 2 ;;
    --model) MODEL=$2; shift 2 ;;
    --task-seconds) TASK_SECONDS=$2; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$BIN" ]; then
  for c in "$ROOT/target/release/genie" "$ROOT/target/debug/genie"; do
    if [ -x "$c" ]; then BIN=$c; break; fi
  done
fi
[ -n "$BIN" ] && [ -x "$BIN" ] || { echo "no genie binary: build it (cargo build -p genie) or pass --bin" >&2; exit 1; }

if [ -z "$WORK" ]; then
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/genie-crash-drill.XXXXXX")
fi
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)
SERVER_PID=
cleanup() {
  if [ -n "$SERVER_PID" ]; then kill -9 "$SERVER_PID" 2>/dev/null || true; fi
  pkill -9 -f "genie serve --data $WORK/" 2>/dev/null || true
  [ "${KEEP:-0}" = 1 ] || rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() { printf '\nFAILED: %s\n' "$*" >&2; exit 1; }
step() { printf '\n--- %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

# The LiteLLM key, the way docker/local-up.sh takes it: from bws-run into the environment.
LITELLM_API_KEY=${LITELLM_API_KEY:-$(bws-run bash -c 'printf %s "$LITELLM_API_KEY"' 2>/dev/null || true)}
[ -n "$LITELLM_API_KEY" ] || fail "no LITELLM_API_KEY (bws-run printed none): the drill runs a real model"
export LITELLM_API_KEY

DATA=$WORK/data
REPO=$WORK/repo
TRACKER=$DATA/projects/t/genie.db

step "the stand: data $DATA, repo $REPO, port $PORT, binary $BIN"
mkdir -p "$DATA" "$REPO"
git -C "$REPO" init -q -b main
git -C "$REPO" -c user.name=drill -c user.email=drill@drill commit -q --allow-empty -m init

cat > "$DATA/config.json" <<EOF
{
  "bind": "127.0.0.1",
  "port": $PORT,
  "roleModels": { "executor": { "model": "$MODEL" } },
  "runtime": { "turnTimeoutSecs": 600, "maxAttempts": 8 }
}
EOF

# The command line works on the stand's data directly; without a token it also pokes the
# running loopback server awake (POST /api/wake), so what it creates is picked up at once.
G() { env GENIE_DATA=$DATA GENIE_PROJECT=t "$BIN" "$@"; }

SERVER_PID=
start_server() {
  # Another drill's orphan on the port would answer /health as if it were ours.
  if curl -fsS -m 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    fail "something already answers on port $PORT: an orphan drill server? (pgrep -af 'genie serve')"
  fi
  "$BIN" serve --data "$DATA" --port "$PORT" >"$WORK/server.log" 2>&1 &
  SERVER_PID=$!
  i=0
  while [ $i -lt 150 ]; do
    # Our process first: a dying server (a held port) must not pass by someone else's answer.
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then cat "$WORK/server.log"; fail "the server died at start"; fi
    if curl -fsS -m 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then return 0; fi
    i=$((i + 1)); sleep 0.2
  done
  cat "$WORK/server.log"; fail "the server did not come up"
}

stop_server_hard() {
  if [ -n "$SERVER_PID" ]; then kill -9 "$SERVER_PID" 2>/dev/null || true; fi
  if [ -n "$SERVER_PID" ]; then wait "$SERVER_PID" 2>/dev/null || true; fi
  SERVER_PID=
}

# How many times the agent left a task comment containing the marker.
markers() {
  python3 - "$TRACKER" "$1" <<'PY'
import sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
print(conn.execute("SELECT COUNT(*) FROM comments WHERE text LIKE ?", ("%" + sys.argv[2] + "%",)).fetchone()[0])
conn.close()
PY
}

# wait_markers <seconds> <marker> <at-least> <what>
wait_markers() {
  deadline=$(( $(date +%s) + $1 )); marker=$2; want=$3; what=$4
  while :; do
    n=$(markers "$marker")
    if [ "$n" -ge "$want" ]; then note "$what: «$marker» seen $n time(s)"; return 0; fi
    if [ "$(date +%s)" -ge "$deadline" ]; then fail "timed out waiting for $what («$marker» seen $n, want ≥ $want)"; fi
    sleep 2
  done
}

# The agent's process: the pid file of the stand's project under <data>/runtime.
agent_pid() {
  cat "$DATA"/runtime/t/*/session.pid 2>/dev/null | head -1 | cut -d' ' -f1
}

start_server
G project add t --name "crash drill" --repo "$REPO" >/dev/null
# No server orchestrator on the stand: the drill drives the task itself, the way
# the manual run of G-79 did (otherwise the orchestrator takes the task in for
# shaping and the scripted waits measure the wrong thing).
G project update t --autonomy manual >/dev/null

step "the task: six slow files, a comment after each, one commit at the end"
TASK=$(G task create --draft --json "crash drill: slow files" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
note "task $TASK"
G task status ready --task "$TASK" -m "drill" >/dev/null
TEAM=$(G team spawn "$TASK" --member executor --json --note "Work in the task's worktree. Create the files a1..a6 one at a time (no per-file commits). After creating each file a<N>, immediately leave a task comment with exactly the text: DRILL-FILE a<N>. Pause about ${TASK_SECONDS} seconds after each comment before the next file (sleep in your shell). After a6, make ONE git commit of all six files, then leave a task comment with exactly the text: DRILL-DONE. Do not move the task anywhere else." | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
[ -n "$TEAM" ] || fail "no team id in the spawn answer"
note "team $TEAM"

step "waiting for the agent to reach the middle of its work (DRILL-FILE a2)"
wait_markers 360 "DRILL-FILE a2" 1 "the second file"

step "kill -9 the AGENT mid-turn, and wait for the server to restart and continue it"
APID=$(agent_pid)
[ -n "$APID" ] || fail "no agent session pid file under $DATA/runtime/t"
kill -9 "$APID"
note "killed agent pid $APID"
wait_markers 360 "DRILL-FILE a4" 1 "the work continued after the agent kill"

step "kill -9 the SERVER mid-turn, and start it again at once"
stop_server_hard
note "server killed; restarting immediately (the port may still be held by the kernel)"
start_server
wait_markers 420 "DRILL-DONE" 1 "the work finished after the server kill"

step "nothing lost, nothing duplicated"
for n in 1 2 3 4 5 6; do
  c=$(markers "DRILL-FILE a$n")
  [ "$c" = 1 ] || fail "comment «DRILL-FILE a$n» found $c time(s), expected exactly 1"
done
c=$(markers "DRILL-DONE")
[ "$c" = 1 ] || fail "comment «DRILL-DONE» found $c time(s), expected exactly 1"
note "every DRILL-FILE and DRILL-DONE marker arrived exactly once"
# A letter delivered to a person twice would appear in two channel deliveries.
python3 - "$TRACKER" <<'PY' || fail "a letter was delivered twice (see above)"
import collections, json, sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
seen = collections.Counter()
for (ids,) in conn.execute("SELECT mail FROM deliveries"):
    for i in json.loads(ids or "[]"):
        seen[i] += 1
dupes = [i for i, n in seen.items() if n > 1]
if dupes:
    print("delivered more than once:", dupes)
    raise SystemExit(1)
print(f"    channel deliveries: {sum(seen.values())} letter(s), none duplicated")
conn.close()
PY

step "the work itself finished"
WT=$(G task show "$TASK" | sed -n 's/^worktree: \(.*\) @ .*/\1/p')
[ -n "$WT" ] || fail "the task shows no worktree"
for n in 1 2 3 4 5 6; do
  [ -f "$WT/a$n" ] || fail "file a$n is missing in the worktree"
done
git -C "$WT" log --oneline -1 | grep -q . || fail "no commit in the worktree"
note "a1..a6 and a commit are in $WT"

step "no orphans: stop the team with its worktree, stop the server, nothing is left"
G team stop "$TEAM" --remove-worktree >/dev/null
deadline=$(( $(date +%s) + 60 ))
while [ -e "$WT" ] && [ "$(date +%s)" -lt "$deadline" ]; do sleep 1; done
[ ! -e "$WT" ] || fail "the worktree stayed: $WT"
LEFT=$(git -C "$REPO" worktree list --porcelain | grep '^worktree ' | grep -v " $REPO$" || true)
[ -z "$LEFT" ] || { git -C "$REPO" worktree list; fail "a worktree stayed"; }
stop_server_hard
sleep 3
ORPHANS=$(pgrep -f "$WORK" || true)
[ -z "$ORPHANS" ] || { ps -fp $ORPHANS; fail "processes of the drill are still running"; }
note "no drill processes, no worktrees"

STATUS=$(G task show "$TASK" | sed -n 's/^type task · status \([a-z_]*\).*/\1/p')
case "$STATUS" in in_progress) fail "the task is stuck in in_progress" ;; esac
note "task status: $STATUS"

step "PASSED: kill -9 of the agent and the server lost nothing, duplicated nothing, left no orphans"
