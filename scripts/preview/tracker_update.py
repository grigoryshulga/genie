#!/usr/bin/env python3
"""Update only the local tracker from a verified dev commit, retaining its data volumes."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request

import config
import diskgc
from cleanup import ancestor
from preview import clean, git, lock, read, run, write

CONTAINER = "genie-tracker"
PROJECT = "genie-tracker"
SERVICE = "genie"
REVISION = "org.opencontainers.image.revision"


def ci_passed(checks):
    # A merge's push checks must be complete; the first registered check is insufficient.
    jobs = [job for job in checks if job.get("app", {}).get("slug") == "github-actions"]
    return (any(job["name"] == "Quality gate" and job.get("conclusion") == "success" for job in jobs)
            and all(job["status"] == "completed" and job.get("conclusion") in {"success", "skipped"} for job in jobs))


class Updater:
    def __init__(self, root, preview_home=None):
        self.root = Path(root).resolve()
        self.preview_home = Path(preview_home) if preview_home else config.home_for_repo(root)
        self.home = self.preview_home.parent / "genie-tracker"
        self.home.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.source = self.home / "source"
        self.override = self.home / "compose-image.json"

    def public_url(self):
        discovered = config.service_config(self.preview_home)
        if discovered.get("trackerService") and discovered.get("tailnetDomain"):
            return "https://genie-self." + discovered["tailnetDomain"]
        return None

    def runtime_matches(self, info, sha):
        desired = self.public_url()
        environment = info["Config"].get("Env", [])
        return (info["Config"].get("Labels", {}).get(REVISION) == sha
                and info["State"]["Status"] == "running"
                and (not desired or "GENIE_PUBLIC_URL=" + desired in environment))

    def container(self):
        info = json.loads(run(["docker", "inspect", CONTAINER]))[0]
        labels = info["Config"].get("Labels") or {}
        if labels.get("com.docker.compose.project") != PROJECT or labels.get("com.docker.compose.service") != SERVICE:
            raise RuntimeError("Preserving a container that is not this Compose tracker")
        return info

    def compose(self, *args):
        return ["docker", "compose", "-p", PROJECT, "-f", self.source / "docker/docker-compose.tracker.yml",
                "-f", self.override, *args]

    def checks(self, sha):
        # --paginate covers repositories with more than 100 checks; flatten every page.
        pages = run(["gh", "api", "--paginate", "--slurp", f"repos/{{owner}}/{{repo}}/commits/{sha}/check-runs?per_page=100"], cwd=self.root)
        return [job for page in json.loads(pages) for job in page["check_runs"]]

    def select_revision(self, tip, before):
        previous = before["Config"].get("Labels", {}).get(REVISION)
        if previous and not ancestor(self.root, previous, tip):
            raise RuntimeError("Refusing to downgrade after dev was rewound; inspect the database migration state")
        if ci_passed(self.checks(tip)):
            return tip
        recent = json.loads(run(["gh", "run", "list", "--workflow", "ci.yml", "--branch", "dev",
                                 "--event", "push", "--status", "success", "--limit", "20", "--json", "headSha"], cwd=self.root))
        for record in recent:
            sha = record["headSha"]
            if not ancestor(self.root, sha, tip):
                continue
            # Never downgrade a migrated database to an older deployed revision.
            if previous and not ancestor(self.root, previous, sha):
                continue
            if ci_passed(self.checks(sha)):
                return sha
        return None

    def checkout(self, sha):
        if self.source.exists():
            if (git(self.source, "rev-parse", "--path-format=absolute", "--git-common-dir") !=
                    git(self.root, "rev-parse", "--path-format=absolute", "--git-common-dir") or not clean(self.source)):
                raise RuntimeError("Preserving modified or unrelated tracker deployment worktree")
            git(self.source, "switch", "--detach", sha)
        else:
            git(self.root, "worktree", "add", "--detach", str(self.source), sha)
        # Private state uses umask 077, but Docker COPY preserves source modes.
        # Restore only tracked source permissions so the image's unprivileged user can read them.
        for entry in git(self.source, "ls-files", "--stage", "-z").split("\0"):
            if not entry:
                continue
            metadata, name = entry.split("\t", 1)
            mode = metadata.split(" ", 1)[0]
            if mode not in {"100644", "100755"}:
                continue
            path = self.source / name
            path.chmod(0o755 if mode == "100755" else 0o644)
            for parent in path.parents:
                if parent == self.source.parent:
                    break
                parent.chmod(0o755)

    def image_preflight(self, image):
        # Exercise the real entrypoint/configuration as the image's service UID, without live data.
        run(["docker", "run", "--rm", "--network", "none", "--user", "1000:1000",
             "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
             "--tmpfs", "/data:uid=1000,gid=1000,mode=0700",
             "--tmpfs", "/workspace:uid=1000,gid=1000,mode=0700",
             "-e", "HOME=/data/home", image, "--help"])

    def preflight(self, before):
        effective = json.loads(run(self.compose("config", "--format", "json")))
        service = effective["services"][SERVICE]
        if service.get("container_name") != CONTAINER:
            raise RuntimeError("Refusing a changed tracker container identity")
        current = {mount["Destination"]: mount for mount in before["Mounts"]}
        for target in ("/data", "/workspace"):
            proposed = [mount for mount in service.get("volumes", []) if mount["target"] == target]
            if len(proposed) != 1 or proposed[0]["type"] != "volume" or current.get(target, {}).get("Type") != "volume":
                raise RuntimeError("Tracker update must retain named volumes: " + target)
            name = effective["volumes"][proposed[0]["source"]]["name"]
            if name != current[target]["Name"]:
                raise RuntimeError("Refusing a changed tracker data volume: " + target)

    def backup(self, directory):
        directory.mkdir(parents=True, mode=0o700)
        archive = directory / "data.tar"
        # The service is stopped: copy the entire named data volume, including WAL and keys.
        # Keeping a tar avoids changing file ownership or extracting credentials onto public paths.
        with archive.open("xb") as output:
            result = subprocess.run(["docker", "cp", CONTAINER + ":/data/.", "-"],
                                    stdout=output, stderr=subprocess.PIPE)
        archive.chmod(0o600)
        if result.returncode or archive.stat().st_size == 0:
            raise RuntimeError("Tracker backup failed; " + result.stderr.decode(errors="replace")[-1000:])
        return archive

    def health(self, image):
        until = time.monotonic() + 90
        while time.monotonic() < until:
            info = self.container()
            if info["Image"] != image:
                raise RuntimeError("Another deployment replaced the tracker during update")
            if info["State"]["Status"] == "running":
                try:
                    with urllib.request.urlopen("http://127.0.0.1:7422/api/health", timeout=3) as response:
                        if response.status == 200:
                            return
                except OSError:
                    pass
            time.sleep(1)
        raise RuntimeError("Updated tracker did not become healthy; inspect the paused update and backup")

    def update(self):
        with lock(self.home / "update.lock"):
            if (self.home / "paused.json").exists():
                raise RuntimeError("Tracker updates paused after deployment failure; inspect paused.json before resuming")
            git(self.root, "fetch", "origin", "dev")
            tip = git(self.root, "rev-parse", "origin/dev")
            before = self.container()
            if self.runtime_matches(before, tip):
                self.health(before["Image"])
                return {"status": "current", "sha": tip}
            sha = self.select_revision(tip, before)
            if not sha:
                state = {"status": "waiting_for_ci", "devTip": tip, "checkedAt": time.time()}
                write(self.home / "status.json", state)
                return state
            if self.runtime_matches(before, sha):
                self.health(before["Image"])
                state = {"status": "current_waiting_for_ci", "sha": sha, "devTip": tip, "checkedAt": time.time()}
                write(self.home / "status.json", state)
                return state
            self.checkout(sha)
            image = "genie-tracker:dev-" + sha
            service = {"image": image, "labels": {REVISION: sha}, "build": {"labels": {REVISION: sha}}}
            if self.public_url():
                service["environment"] = {"GENIE_PUBLIC_URL": self.public_url()}
            write(self.override, {"services": {SERVICE: service}})
            self.preflight(before)
            write(self.home / "status.json", {"status": "building", "sha": sha, "devTip": tip, "startedAt": time.time()})
            run(self.compose("build", SERVICE))
            image_id = run(["docker", "image", "inspect", image, "--format", "{{.Id}}"])
            self.image_preflight(image)
            # Builds may be slow. Recheck ownership and CI before touching the running service.
            current = self.container()
            if current["Id"] != before["Id"] or current["Image"] != before["Image"]:
                raise RuntimeError("Tracker changed during build; retry against its new state")
            if not ci_passed(self.checks(sha)):
                return {"status": "waiting_for_ci", "sha": sha}
            stamp = str(time.time_ns())
            directory = self.home / "backups" / stamp
            state = {"sha": sha, "devTip": tip, "image": image, "imageId": image_id, "previousImageId": before["Image"],
                     "backup": str(directory / "data.tar"), "startedAt": time.time()}
            run(["docker", "image", "tag", before["Image"], "genie-tracker:before-" + stamp])
            write(self.home / "status.json", dict(state, status="backing_up"))
            run(["docker", "stop", "--time", "20", CONTAINER])
            try:
                self.backup(directory)
                write(directory / "deployment.json", state)
            except Exception:
                # No new binary has touched the data yet: restarting the original is safe.
                run(["docker", "start", CONTAINER])
                raise
            try:
                write(self.home / "status.json", dict(state, status="starting"))
                run(self.compose("up", "-d", "--no-build", "--no-deps", SERVICE))
                self.health(image_id)
            except Exception as exc:
                # A new binary may already have migrated the databases. Never downgrade it blindly.
                state.update(status="paused", error=str(exc), failedAt=time.time())
                write(self.home / "paused.json", state)
                write(self.home / "status.json", state)
                raise
            state.update(status="deployed", completedAt=time.time())
            write(self.home / "current.json", state)
            write(self.home / "status.json", state)
            self.prune_images()
            return state

    def prune_images(self, keep=2):
        # Old tags only cost disk. The running image, the newest `keep` images and any image a container uses stay.
        try:
            for item in diskgc.image_plan(run, keep):
                run(["docker", "rmi", item["tag"]])
        except RuntimeError:
            pass  # A failed cleanup never undoes a verified deployment.


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--home", type=Path, help="Shared preview installation directory")
    args = parser.parse_args()
    updater = Updater(args.root, args.home)
    try:
        print(json.dumps(updater.update(), indent=2))
    except Exception as exc:
        failure = {"status": "failed", "error": str(exc), "at": time.time()}
        write(updater.home / "last-error.json", failure)
        if not (updater.home / "paused.json").exists():
            write(updater.home / "status.json", failure)
        sys.exit("genie tracker update: " + str(exc))


if __name__ == "__main__":
    main()
