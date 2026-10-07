#!/bin/sh
# The load drill (G-83): four active teams (maxActiveTeams), mail, an automation
# and the web API at the same time, on a scripted stand-in model — and the
# numbers: turn and API latency percentiles, letters and turns, SQLite lock
# errors, the server's memory, the queue in front of the slots.
#
# One throwaway directory, its own data and port: nothing here touches a running
# server, and no real model is called (the «model» is a script that sleeps and
# answers).
#
#   deploy/load-drill.sh [--bin <genie>] [--work <dir>] [--port <n>] [--teams <n>]
#                        [--rounds <n>] [--turn-seconds <n>] [--keep]
#
#   --teams          teams working at once (default 4, the maxActiveTeams default)
#   --rounds         mail ping-pong rounds per team (default 12)
#   --turn-seconds   how long every scripted model turn sleeps (default 2)
#
# Exit status: 0 when every step behaved as expected, 1 on the first surprise.

set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=
WORK=
PORT=7441
TEAMS=4
ROUNDS=12
TURN_SECONDS=2
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --bin) BIN=$2; shift 2 ;;
    --work) WORK=$2; shift 2 ;;
    --port) PORT=$2; shift 2 ;;
    --teams) TEAMS=$2; shift 2 ;;
    --rounds) ROUNDS=$2; shift 2 ;;
    --turn-seconds) TURN_SECONDS=$2; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
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
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/genie-load-drill.XXXXXX")
fi
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)

SERVER_PID=
cleanup() {
  if [ -n "$SERVER_PID" ]; then kill "$SERVER_PID" 2>/dev/null || true; fi
  pkill -9 -f "genie serve --data $WORK/" 2>/dev/null || true
  [ "${KEEP:-0}" = 1 ] || rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() { printf '\nFAILED: %s\n' "$*" >&2; exit 1; }
step() { printf '\n--- %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
rss_mb() { ps -o rss= -p "$1" 2>/dev/null | tr -d ' ' | grep . || echo 0; }

DATA=$WORK/data
REPO=$WORK/repo
TRACKER=$DATA/projects/load/genie.db

step "the stand: $TEAMS teams × ping-pong of $ROUNDS rounds, a ${TURN_SECONDS}s turn, port $PORT"
mkdir -p "$DATA" "$REPO"
git -C "$REPO" init -q -b main
git -C "$REPO" -c user.name=drill -c user.email=drill@drill commit -q --allow-empty -m init

# The scripted model: a letter «ROUND n FROM <name>» sleeps, and answers with
# ROUND n+1 until the rounds run out. Anything else it reads and drops.
cat > "$WORK/agent.sh" <<'EOF'
#!/usr/bin/env bash
# No `set -e`: grep with no match exits 1, and a letter without a ROUND is fine.
set -u
msg="${!#}"
round=$(grep -o 'ROUND [0-9]*' <<<"$msg" | head -1 | awk '{print $2}')
[ -n "$round" ] || exit 0
sleep "${LOAD_TURN_SECONDS:-2}"
next=$((round + 1))
[ "$next" -gt "${LOAD_ROUNDS:-12}" ] && exit 0
from=$(grep -o 'FROM [A-Za-z0-9_-]*' <<<"$msg" | head -1 | awk '{print $2}')
[ -n "$from" ] && "$GENIE_BIN" mail send "$from" "ROUND $next FROM $GENIE_AGENT_NAME"
exit 0
EOF
chmod +x "$WORK/agent.sh"

cat > "$DATA/config.json" <<EOF
{
  "bind": "127.0.0.1",
  "port": $PORT,
  "limits": { "maxActiveTeams": $TEAMS },
  "runtime": {
    "sandbox": { "mode": "off" },
    "maxConcurrent": 4,
    "turnTimeoutSecs": 120,
    "command": [["bash", "$WORK/agent.sh"], ["{message}"]],
    "env": { "LOAD_ROUNDS": "$ROUNDS", "LOAD_TURN_SECONDS": "$TURN_SECONDS", "GENIE_BIN": "$BIN" }
  }
}
EOF

G() { env GENIE_DATA=$DATA GENIE_PROJECT=load "$BIN" "$@"; }

if curl -fsS -m 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
  fail "something already answers on port $PORT: an orphan drill server? (pgrep -af 'genie serve')"
fi
"$BIN" serve --data "$DATA" --port "$PORT" >"$WORK/server.log" 2>&1 &
SERVER_PID=$!
i=0
while [ $i -lt 150 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then cat "$WORK/server.log"; fail "the server died at start"; fi
  if curl -fsS -m 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then break; fi
  i=$((i + 1)); sleep 0.2
done
[ $i -lt 150 ] || { cat "$WORK/server.log"; fail "the server did not come up"; }

RSS_START=$(rss_mb "$SERVER_PID")
G project add load --name "load drill" --repo "$REPO" >/dev/null
G project update load --autonomy manual >/dev/null
G task create --draft "load drill" >/dev/null
G task status ready --task G-1 -m drill >/dev/null

step "the teams: spawn and start the ping-pong"
n=1
while [ $n -le $TEAMS ]; do
  G task create --draft "load $n" >/dev/null
  G task status ready --task "G-$((n + 1))" -m drill >/dev/null
  G team spawn "G-$((n + 1))" --member executor --member reviewer >/dev/null
  n=$((n + 1))
done
# Who to poke with churn letters: the first team's executor.
EXEC0=$(G team show "G-2" | awk '/\(executor\)/{print $2; exit}')
[ -n "$EXEC0" ] || fail "no executor in team 1"

step "the background load: the web API, the live stream, mail churn, a cron automation"
cat > "$WORK/automation.json" <<EOF
{ "name": "load-drill", "on": { "schedule": "*/10 * * * * *" },
  "steps": [ { "id": "note", "task.comment": { "text": "cron tick", "task": "G-1" } } ] }
EOF
G automation create --file "$WORK/automation.json" >/dev/null

mkdir -p "$WORK/api"
: > "$WORK/api/latency"
( end=$(( $(date +%s) + 75 )); path=tasks
  while [ "$(date +%s)" -lt "$end" ]; do
    t=$(curl -fsS -o /dev/null -w '%{time_total}' -m 10 "http://127.0.0.1:$PORT/api/$path" 2>/dev/null || echo 10)
    echo "$t" >> "$WORK/api/latency"
    case "$path" in tasks) path="stats?days=1" ;; *) path=tasks ;; esac
    sleep 0.2
  done ) &
API_LOADER=$!
( curl -fsS -N -m 75 "http://127.0.0.1:$PORT/api/events" >"$WORK/api/events" 2>/dev/null || true ) &
SSE_LOADER=$!
( end=$(( $(date +%s) + 70 )); n=0
  while [ "$(date +%s)" -lt "$end" ]; do
    n=$((n + 1)); G mail send "$EXEC0" "churn $n" --team "G-2" >/dev/null 2>&1 || true
    sleep 1
  done ) &
MAIL_LOADER=$!
QUEUE_MAX=0
( end=$(( $(date +%s) + 70 ))
  while [ "$(date +%s)" -lt "$end" ]; do
    w=$(curl -fsS -m 5 -H "x-genie: 1" "http://127.0.0.1:$PORT/api/runtime" 2>/dev/null || echo '{}')
    m=$(printf '%s' "$w" | python3 -c 'import json,sys; v=json.load(sys.stdin); print(v["membersWaiting"])' 2>/dev/null || echo 0)
    [ "$m" -gt "$QUEUE_MAX" ] && QUEUE_MAX=$m && echo "$QUEUE_MAX" > "$WORK/queue-max"
    sleep 1
  done ) &
QUEUE_LOADER=$!

step "start the ping-pong: the first letter of every chain"
n=1
while [ $n -le $TEAMS ]; do
  reviewer=$(G team show "G-$((n + 1))" | awk '/\(reviewer\)/{print $2; exit}')
  executor=$(G team show "G-$((n + 1))" | awk '/\(executor\)/{print $2; exit}')
  [ -n "$reviewer" ] && [ -n "$executor" ] || fail "no reviewer/executor in team $n"
  G mail send "$executor" "ROUND 1 FROM $reviewer" --team "G-$((n + 1))" >/dev/null
  n=$((n + 1))
done

step "waiting for the ping-pong to run out of rounds"
expected=$(( TEAMS * ROUNDS ))
deadline=$(( $(date +%s) + 600 ))
while :; do
  done_now=$(python3 - "$TRACKER" "$ROUNDS" <<'PY'
import sqlite3, sys
rounds = int(sys.argv[2])
conn = sqlite3.connect(sys.argv[1])
n = conn.execute("SELECT COUNT(*) FROM mail WHERE text LIKE 'ROUND %'").fetchone()[0]
conn.close()
print(n)
PY
)
  [ "$done_now" -ge "$expected" ] && break
  [ "$(date +%s)" -lt "$deadline" ] || fail "the ping-pong stalled at $done_now of $expected letters"
  sleep 3
done
note "$done_now letters"

wait $API_LOADER $MAIL_LOADER $QUEUE_LOADER 2>/dev/null || true
kill $SSE_LOADER 2>/dev/null || true

# The churn letters of the last seconds drain through their turns.
deadline=$(( $(date +%s) + 30 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  left=$(python3 - "$TRACKER" <<'PY'
import sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
print(conn.execute("SELECT COUNT(*) FROM mail WHERE delivered_at IS NULL AND recipient <> 'orchestrator'").fetchone()[0])
conn.close()
PY
)
  [ "$left" = 0 ] && break
  sleep 2
done
RSS_END=$(rss_mb "$SERVER_PID")

step "the numbers"
python3 - "$TRACKER" "$DATA/server.db" "$WORK" "$RSS_START" "$RSS_END" "$TURN_SECONDS" <<'PY'
import json, pathlib, sqlite3, statistics, sys
tracker, server, work, rss0, rss1, turn_s = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), float(sys.argv[6])
conn = sqlite3.connect(tracker)
turns = sqlite3.connect(server).execute("SELECT status, started, finished FROM turns").fetchall()
lat = []
for (status, started, finished) in turns:
    if status == "succeeded" and finished:
        from datetime import datetime
        a, b = datetime.fromisoformat(started), datetime.fromisoformat(finished)
        lat.append((b - a).total_seconds())
lat.sort()
pct = lambda p: lat[min(len(lat) - 1, int(len(lat) * p))] if lat else float("nan")
failed = [t for t in turns if t[0] not in ("succeeded", "skipped")]
undelivered = conn.execute("SELECT COUNT(*) FROM mail WHERE delivered_at IS NULL AND recipient <> 'orchestrator'").fetchone()[0]
letters = conn.execute("SELECT COUNT(*) FROM mail").fetchone()[0]
api = sorted(float(x) for x in (work / "api" / "latency").read_text().split())
apct = lambda p: api[min(len(api) - 1, int(len(api) * p))] if api else float("nan")
queue = (work / "queue-max")
queue_max = queue.read_text().strip() if queue.exists() else "0"
events = (work / "api" / "events").read_text().count("\n")
log = (work / "server.log").read_text()
locks = sum(log.count(w) for w in ("database is locked", "database table is locked", "SQLITE_BUSY"))
print(f"    turns: {len(turns)} ({len(lat)} measured, {len(failed)} failed)")
print(f"    turn latency: p50 {pct(0.5):.2f}s  p95 {pct(0.95):.2f}s  max {lat[-1] if lat else float('nan'):.2f}s  (a scripted turn sleeps {turn_s}s)")
print(f"    letters: {letters}, undelivered to agents: {undelivered}")
print(f"    api latency ({len(api)} calls): p50 {apct(0.5)*1000:.0f}ms  p95 {apct(0.95)*1000:.0f}ms  max {api[-1]*1000:.0f}ms")
print(f"    live stream: {events} event lines")
print(f"    queue in front of the slots: up to {queue_max} member(s) waiting")
print(f"    server rss: {rss0/1024:.0f} -> {rss1/1024:.0f} MB")
print(f"    sqlite lock errors in the log: {locks}")
if failed:
    print("    FAILED turns:", failed[:5])
    raise SystemExit(1)
if undelivered:
    print("    FAILED: letters lost")
    raise SystemExit(1)
PY
[ $? -eq 0 ] || fail "the load drill found failures (see above)"

step "PASSED: no failed turns, no lost letters — the numbers above go to G-83"
