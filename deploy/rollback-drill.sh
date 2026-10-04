#!/bin/sh
# The «updated — rolled back» drill (G-120, gap 3 of docs/platform/proposals.md): migrate a
# copy of a server's data with the new binary, check the result, and roll back to the previous
# binary with a pre-update backup.
#
# Two binaries and one throwaway directory: nothing here touches a running server or its data,
# and no port is opened (the working server's 7420 is never used — every command works on
# `--data` directly). Every command runs as the local operator: with GENIE_TOKEN in the
# environment the CLI acts as an agent and refuses `user add`/`project add` («agents cannot do
# this»), so the GENIE_* variables are cleared first.
#
#   deploy/rollback-drill.sh [--old <bin>] [--new <bin>] [--work <dir>] [--backup <genie-…>] [--keep]
#
#   --old     the binary of the previous release (default: the main checkout's release build)
#   --new     the binary under test (default: this checkout's target/release/genie)
#   --work    where the copies go (default: a fresh mktemp -d; --keep leaves it behind)
#   --backup  a `genie backup` of real data instead of the seeded fixture (one human command
#             makes it: agents cannot read the working server's data directory)
#
# The drill asserts that `genie doctor` is green at three points; a red check there is a
# problem of the machine, not of the data — fix it before reading the drill as evidence.
#
# Exit status: 0 when every step behaved as expected, 1 on the first surprise.

set -eu

OLD=/home/gshulga/projects/personal/genie/target/release/genie
NEW=$(cd "$(dirname "$0")/.." && pwd)/target/release/genie
WORK=
PRE=
KEEP=0

while [ $# -gt 0 ]; do
  case "$1" in
    --old) OLD=$2; shift 2 ;;
    --new) NEW=$2; shift 2 ;;
    --work) WORK=$2; shift 2 ;;
    --backup) PRE=$2; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# The data directory the working server uses: the drill must never copy itself into it.
LIVE_DATA=${GENIE_DATA:-$HOME/.local/share/genie}
unset GENIE_TOKEN GENIE_URL GENIE_PROJECT GENIE_TASK GENIE_TEAM GENIE_DATA

fail() { printf '\nFAILED: %s\n' "$*" >&2; exit 1; }
step() { printf '\n--- %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
run() { printf '$ %s\n' "$*"; "$@"; }

# A command that must be refused (non-zero), with its output kept for inspection.
must_fail() {
  printf '$ %s   (must be refused)\n' "$*"
  if "$@" > "$WORK/.refusal" 2>&1; then
    cat "$WORK/.refusal"
    fail "expected a refusal from: $*"
  fi
  cat "$WORK/.refusal"
}

grep_out() { # grep_out <pattern> <file>
  grep -q "$1" "$2" || { cat "$2"; fail "expected \"$1\" in the output above"; }
}

# One hash over every file name and content of a data directory: what «byte-identical» means.
hash_dir() {
  ( cd "$1" && find . -type f | LC_ALL=C sort | xargs -r sha256sum | sha256sum ) | cut -d' ' -f1
}

# "database table rows", one line per table: the row counts that must survive the migration.
counts() {
  python3 - "$1" <<'PY'
import pathlib, sqlite3, sys
data = pathlib.Path(sys.argv[1])
for db in sorted(data.rglob("*.db")):
    if db.name == "vault-index.db":
        continue
    conn = sqlite3.connect(db)
    try:
        tables = [t for (t,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        for table in tables:
            n = conn.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0]
            print(f"{db.relative_to(data)} {table} {n}")
    finally:
        conn.close()
PY
}

# Raise the schema a database records: «the data was written by a newer genie».
raise_schema() { # raise_schema <dir> <server version> <tracker version>
  python3 - "$1" "$2" "$3" <<'PY'
import pathlib, sqlite3, sys
data, server, tracker = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
def set_version(db, version):
    conn = sqlite3.connect(db)
    conn.execute("UPDATE meta SET value = ? WHERE key = 'schema'", (version,))
    conn.commit()
    conn.close()
set_version(data / "server.db", server)
for db in sorted((data / "projects").rglob("*.db")):
    set_version(db, tracker)
PY
}

# Point a copied server.db at its copied trackers: a project is registered by absolute path.
relocate() { # relocate <copied server.db> <copied from> <copied to>
  python3 - "$1" "$2" "$3" <<'PY'
import sqlite3, sys
db, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
conn = sqlite3.connect(db)
conn.execute("UPDATE projects SET tracker_dir = replace(tracker_dir, ?, ?)", (old, new))
conn.commit()
conn.close()
PY
}

[ -x "$OLD" ] || fail "no binary at $OLD (--old); the previous release's build"
[ -x "$NEW" ] || fail "no binary at $NEW: build it (cargo build --release -p genie) or pass --new"

if [ -n "$PRE" ]; then
  [ -d "$PRE" ] || fail "no backup directory at $PRE (--backup)"
fi

if [ -z "$WORK" ]; then
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/genie-rollback-drill.XXXXXX")
fi
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)
case "$WORK/" in
  "$LIVE_DATA/"*) fail "the work directory is inside a data directory: $WORK" ;;
  "$HOME/.local/share/genie/"*) fail "the work directory is inside the working server's data directory: $WORK" ;;
esac
trap '[ "$KEEP" = 1 ] || rm -rf "$WORK"' EXIT

DATA="$WORK/data"
SEEDED=1
printf 'previous binary: %s\n' "$OLD"
printf 'binary under test: %s\n' "$NEW"
printf 'work directory: %s\n' "$WORK"

step "1. seed a throwaway server with the previous binary"
if [ -n "$PRE" ]; then
  SEEDED=0
  note "using $PRE instead of the seeded fixture"
  mkdir -p "$WORK/backups"
  run cp -r "$PRE" "$WORK/backups/"
  PRE="$WORK/backups/$(basename "$PRE")"
  run "$OLD" --data "$DATA" restore "$PRE"
else
  run mkdir -p "$DATA"
  printf 'drill-password\n' | "$OLD" --data "$DATA" user add admin --admin --name Drill --password-stdin
  run "$OLD" --data "$DATA" project add shop --name "Drill shop" --prefix D
  run "$OLD" --data "$DATA" --project shop task create "Export the price list" -d "The shop needs a CSV export." -a "a CSV file with one row per product" -a "the export is tested" --label export
  run "$OLD" --data "$DATA" --project shop task create "Fix the discount rounding" -d "The total is off by a cent." -a "a test covers the rounding" --label bug
  run "$OLD" --data "$DATA" --project shop task create "Ship the nightly report" -d "A nightly report of the day's orders." -a "the report is attached to the task" --label report
  run "$OLD" --data "$DATA" --project shop task comment --task D-1 "Seeded by deploy/rollback-drill.sh (G-120)."
fi

step "2. the previous binary is happy with this data (baseline)"
run "$OLD" --data "$DATA" doctor

step "3. the pre-update backup (the rollback path)"
if [ "$SEEDED" = 1 ]; then
  run "$OLD" --data "$DATA" backup "$WORK/backups"
  PRE="$WORK/backups/$(cd "$WORK/backups" && ls -d genie-* | head -1)"
fi
note "pre-update backup: $PRE"

step "4. the new binary checks the pending migrations on a copy (criterion 1)"
COUNTS_BEFORE=$(counts "$DATA")
HASH_BEFORE=$(hash_dir "$DATA")
run "$NEW" --data "$DATA" migrate --check > "$WORK/check.txt" 2>&1
cat "$WORK/check.txt"
grep_out "database(s) to migrate" "$WORK/check.txt"
grep_out "server.db: schema 1 → 2" "$WORK/check.txt"
HASH_AFTER=$(hash_dir "$DATA")
[ "$HASH_BEFORE" = "$HASH_AFTER" ] || fail "migrate --check changed the data directory ($HASH_BEFORE → $HASH_AFTER)"
note "the data directory is byte-identical after the check: $HASH_AFTER"

step "5. the new binary applies the migrations"
run "$NEW" --data "$DATA" migrate > "$WORK/migrate.txt" 2>&1
cat "$WORK/migrate.txt"
grep_out "migrated" "$WORK/migrate.txt"
grep_out "database(s)" "$WORK/migrate.txt"
run "$NEW" --data "$DATA" migrate --check > "$WORK/check2.txt" 2>&1
cat "$WORK/check2.txt"
grep_out "nothing to migrate" "$WORK/check2.txt"
run "$NEW" --data "$DATA" doctor
if [ "$SEEDED" = 1 ]; then
  run "$NEW" --data "$DATA" --project shop task list > "$WORK/tasks.txt" 2>&1
  cat "$WORK/tasks.txt"
  grep_out "Export the price list" "$WORK/tasks.txt"
fi
COUNTS_AFTER=$(counts "$DATA")
if [ "$COUNTS_BEFORE" != "$COUNTS_AFTER" ]; then
  printf '%s\n' "$COUNTS_BEFORE" > "$WORK/counts-before.txt"
  printf '%s\n' "$COUNTS_AFTER" > "$WORK/counts-after.txt"
  diff -u "$WORK/counts-before.txt" "$WORK/counts-after.txt" || true
  fail "the migration changed row counts"
fi
note "every row count survived the migration"

step "6. roll back: the pre-update backup with the previous binary"
run "$NEW" --data "$WORK/rollback" restore "$PRE"
run "$OLD" --data "$WORK/rollback" doctor
if [ "$SEEDED" = 1 ]; then
  run "$OLD" --data "$WORK/rollback" --project shop task list
fi

step "7. the guard: data written by a newer genie is refused"
run cp -r "$DATA" "$WORK/newer"
run relocate "$WORK/newer/server.db" "$DATA" "$WORK/newer"
run raise_schema "$WORK/newer" 3 6
must_fail "$NEW" --data "$WORK/newer" migrate --check
grep_out "records schema 3" "$WORK/.refusal"
grep_out "restore a backup made before the update" "$WORK/.refusal"

run cp -r "$PRE" "$WORK/sneaky"
run raise_schema "$WORK/sneaky" 3 6
must_fail "$NEW" --data "$WORK/restored" restore "$WORK/sneaky"
grep_out "newer genie" "$WORK/.refusal"
[ ! -e "$WORK/restored/server.db" ] || fail "restore wrote into the target although it refused the backup"
note "nothing was written into $WORK/restored"

step "8. drift: a tracker whose version lies about a missing column"
run cp -r "$DATA" "$WORK/drift"
run relocate "$WORK/drift/server.db" "$DATA" "$WORK/drift"
python3 - "$WORK/drift" <<'PY'
import pathlib, sqlite3, sys
data = pathlib.Path(sys.argv[1])
for db in sorted((data / "projects").rglob("*.db")):
    conn = sqlite3.connect(db)
    conn.execute("ALTER TABLE tasks DROP COLUMN assignee")
    conn.execute("UPDATE meta SET value = '4' WHERE key = 'schema'")
    conn.commit()
    conn.close()
PY
run "$NEW" --data "$WORK/drift" migrate --check > "$WORK/drift-check.txt" 2>&1
cat "$WORK/drift-check.txt"
grep_out "genie.db: schema 4 → 5 (tasks.assignee)" "$WORK/drift-check.txt"
DRIFT_BEFORE=$(counts "$WORK/drift")
run "$NEW" --data "$WORK/drift" migrate
DRIFT_AFTER=$(counts "$WORK/drift")
[ "$DRIFT_BEFORE" = "$DRIFT_AFTER" ] || fail "the tracker migration changed row counts"
run "$NEW" --data "$WORK/drift" migrate --check > "$WORK/drift-check2.txt" 2>&1
cat "$WORK/drift-check2.txt"
grep_out "nothing to migrate" "$WORK/drift-check2.txt"
note "the dropped column is back and the tracker records schema 5"

step "done"
note "checked on a copy → applied → rolled back from the backup → newer data refused → drift repaired"
if [ "$KEEP" = 1 ]; then note "the work directory is kept: $WORK"; fi
