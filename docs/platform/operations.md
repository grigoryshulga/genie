---
title: Operations
summary: Back up, update, restore, and monitor a Genie server.
type: runbook
status: current
---

# Operations

Run operator commands as the service user, using the same `GENIE_DATA` and `PATH` as the server. Commands without a token access the local data directory as its administrator.

## Data directory

The directory comes from `--data`, `GENIE_DATA`, or `~/.local/share/genie`.

```text
server.db             Users, projects, rules, runs, and notifications
projects/<slug>/      Project tracker, unless registered in place
vault/                Knowledge files and Git history
vault-index.db        Rebuildable knowledge search index
agents/, teams/       Role and team overrides
skills/, mcp.json     Skills and MCP connections
config.json, git.json Server and Git host settings
secrets.key           Encryption key for stored credentials
sessions/, runtime/   Agent conversations and logs
workspaces/, repos/   Workspaces and hosted repository mirrors
```

A local project can keep its tracker in `<repository>/.genie/`.

## Back up and restore

```bash
genie backup /backups --keep 14
genie backup /backups --with-secrets
```

Backups snapshot SQLite databases with `VACUUM INTO` and include project trackers, committed vault history as a Git bundle, and configuration. They do **not** include source repositories, worktrees, agent session files, or uncommitted vault edits. Back up those files separately when needed.

By default, `secrets.key` and Git host SSH private keys are omitted. Use `--with-secrets` for a trusted private backup, or preserve the key separately. Restoring without the encryption key requires re-entering stored provider keys and repository tokens. Configuration can contain other secrets, so protect the backup directory even without that option.

Stop the server before restoring:

```bash
sudo systemctl stop genie
genie restore /backups/genie-TIMESTAMP --force
sudo systemctl start genie
genie doctor
```

Run the restore command under the service account with the correct data directory. Rebuild the knowledge index when needed with `genie vault reindex`; this changes the cache, not the pages.

## Update and roll back

1. Create a backup before replacing the binary.
2. Run the new binary with `genie migrate --check`. It tests migrations on temporary database copies without changing live data.
3. Stop the server, install the new binary, run `genie migrate`, and restart. Startup also applies required migrations.
4. Run `genie doctor` and check the service log.

To roll back, stop the server, restore the **previous binary**, and use that binary to restore the pre-update backup. Migrations have no reverse operation. Releases with schema-version protection refuse to open data written by a newer release.

## systemd

From a built checkout:

```bash
sudo deploy/install.sh
sudo -iu genie npm install -g --prefix ~/.local @earendil-works/pi-coding-agent
sudo -iu genie pi  # /login to your model providers
sudo cp deploy/systemd/genie.service deploy/systemd/genie-backup.service deploy/systemd/genie-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now genie genie-backup.timer
sudo -iu genie env GENIE_DATA=/var/lib/genie /opt/genie/bin/genie doctor
```

The installer uses `/opt/genie/bin/genie`, the `genie` user, and `/var/lib/genie`. The backup timer runs daily at 03:30 and keeps 14 backups in `/var/backups/genie`. Copy backups to another machine. Put MCP environment secrets in `/etc/genie/secrets.env`.

## Health and logs

```bash
genie doctor
genie team board
genie team turns
genie server runtime
genie stats --days 7 --project shop
journalctl -u genie -f
```

`doctor` checks storage, schema versions, accounts, projects, role configuration, available pi models, sandbox support, Git identity, channels, and network settings. A failing check gives a nonzero exit code. The same report is in **Settings → Management → Health**.

Agent logs live under `<data>/runtime/<project>/<agent>/`: `stderr.log` for live sessions and `turn-<id>.log` for individual turns. Automation run details show each step's input, output, wait, and error.

## Recovery behavior

Unacknowledged mail returns to the queue after a crash. Live sessions restart with their conversations; interrupted agents receive a continuation message. Failed agents retry with backoff up to `runtime.maxAttempts`, then remain in `error` until restarted. Their pending mail is retained.

Automation runs resume their persisted step state. External effects still need idempotent handling; a durable run is not a guarantee that every external side effect happens exactly once.

Use [runtime supervision](agent-bus.md) to diagnose stalled or repeatedly failing teams. The [pilot guide](pilot.md) lists crash, load, and rollback drills.
