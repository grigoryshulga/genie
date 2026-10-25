---
title: Automatic tracker updates from dev
summary: Run a host timer that deploys verified dev revisions to the local Compose tracker while retaining its data.
type: guide
status: current
---

# Automatic tracker updates from dev

The local `genie-tracker` container can be updated on demand with `preview update-tracker`, or follow the newest successful CI revision already merged into `dev`. An optional host-side systemd timer checks every two minutes, with up to ten seconds of jitter. Git, GitHub credentials and the Docker socket stay on the host; the application container runs the server. This is independent of the disposable [preview environments](visual-preview.md).

## Install and enable

Use the ordinary preview owner with access to Docker, the repository remote and authenticated `gh`. Run from a trusted checkout:

```bash
python3 scripts/preview/install.py --tracker-updater --install-user-units
systemctl --user enable --now genie-tracker-update.timer
```

No additional config parameters or copied tokens are needed. The installer discovers the stable primary checkout and installs code outside task worktrees. The updater uses its own detached source worktree under the shared Git directory's `genie-tracker/source`; it never switches or resets an agent's checkout. Source, status, deployment records and private backups remain under that same local `genie-tracker` directory. Keep them outside Git and retain them when cleaning task previews.

```bash
systemctl --user start genie-tracker-update.service
systemctl --user status genie-tracker-update.timer genie-tracker-update.service
journalctl --user -u genie-tracker-update.service
```

The user service runs while the user's systemd manager is available. If updates must run after logout, the host administrator can enable lingering for that user; the installer does not change the machine's login policy. Inspect the timer's next run with `systemctl --user list-timers genie-tracker-update.timer`.

## Update transaction

The updater serializes runs, fetches `origin/dev`, and requires the commit's GitHub Actions checks to be complete and successful, including `Quality gate`. Deliberately skipped jobs are acceptable. It never treats the first registered required check as the whole CI result. If the tip is pending or failing, it looks for the newest successful push-CI revision in `dev`'s ancestry and rechecks all its jobs. It preserves the currently deployed revision when no eligible forward update exists; automatic downgrades are prohibited.

It then:

1. Checks out that exact revision into its deployment worktree and builds a commit-tagged image through the repository's tracker Compose file. Docker reuses existing layers; web-only changes reuse the backend layer. Tracked source permissions follow Git file modes, independently of private runtime permissions; an unprivileged entrypoint smoke test runs without live data before the tracker is stopped.
2. Verifies that the existing container belongs to Compose project `genie-tracker`, service `genie`, and that `/data` and `/workspace` will retain their exact named volumes. A changed volume identity blocks deployment.
3. Rechecks CI and container identity after the build. If another deployment replaced the service, it stops without overwriting it.
4. Retains the previous image, briefly stops the tracker and copies the complete `/data` volume into a private tar archive. With the server stopped, SQLite databases/WAL, configuration and stored-token keys form a consistent recovery point. A backup failure restarts the original container before any new binary runs.
5. Runs Compose `up -d --no-build --no-deps genie`, preserving volumes, then verifies the expected image and the HTTP health endpoint. The image/container carry the deployed Git SHA in the revision label. A successful run records the SHA, desired dev tip, image IDs and backup path.

Build time does not interrupt the tracker. Backup and recreation cause a short outage; startup can take longer when a revision performs migrations. The timer never calls `down -v`, prunes unrelated images or deploys preview task branches. After a verified deployment the updater removes older tracker image tags, keeping the newest two and any image a container uses; backups are retained, and `preview gc` handles other disk cleanup.

## Failures and recovery

`status.json` reports the last outcome; `last-error.json` records command failures. A tip with failed/pending CI leaves the last verified version running. The timer retries fetch/build failures on a later tick.

After a new image may have touched the databases, a startup/health failure writes `paused.json` and blocks later deployments. It preserves the data volume and pre-update backup. It does not launch the old binary against potentially migrated databases or automatically discard writes by restoring an old snapshot.

Stop the timer before investigating:

```bash
systemctl --user stop genie-tracker-update.timer
```

Inspect the private deployment record, container logs and archive. Prefer a forward fix compatible with the migrated data. If restoration is necessary, restore the archive into a separate empty volume and start the retained compatible image with a reviewed Compose override; retain the failed volume until recovery is verified. Follow the [Compose lifecycle](../agents/compose-service-lifecycle.md). Archive files contain real tracker data and keys: never attach them to PRs or task comments.

After validating recovery, remove the local pause marker deliberately and re-enable the timer. Removing it alone does not reverse a migration. The server's separate [native backup/restore commands](../platform/operations.md) can also be used for operator-managed recovery.

## Private tracker address

To expose the real tracker in the tailnet, define Service `genie-self` with endpoint `tcp:443` in the Tailscale console, using the existing tagged Services host. Tailscale Service names are a single DNS label, so `self.genie.<tailnet>.ts.net` is not a supported Service name. The supported address is `https://genie-self.<tailnet>.ts.net`.

```bash
preview enable-tracker-service
```

Here `preview` is the shared helper function from the [preview guide](visual-preview.md#local-installation). The command discovers the dedicated node's tailnet, preserves other Service handlers and advertises `svc:genie-self` directly to loopback port 7422. Approve its host advertisement in the console. Then run the tracker updater: it sets the discovered HTTPS URL through Compose's `GENIE_PUBLIC_URL`, which configures links and the allowed Host header. Configuration drift is applied even when the eligible Git revision has not changed.

This address opens the persistent working tracker with its real accounts and projects. The demo `preview` account and password belong only to disposable preview databases. The existing native host CLI continues to use loopback. The three demo Services, primary T3 endpoint and Serve listeners are preserved; no public Funnel is enabled.
