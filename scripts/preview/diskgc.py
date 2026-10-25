"""Plan and apply disk cleanup: merged worktrees, rebuildable artifacts, stale tracker images, build cache, unused volumes.

Everything here is a dry run until `execute` is set. Git history is never rewritten and no branch is deleted or
pushed; an unmerged worktree only loses directories that a build recreates.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from cleanup import ancestor, candidate_source, retire
from preview import clean, git, read, run

REBUILDABLE = ("target", "node_modules")
TRACKER_IMAGES = "genie-tracker"
# Compose projects whose volumes hold data nobody should lose by accident.
PROTECTED_VOLUME = re.compile(r"^genie-tracker[_-]")
# Anonymous volumes and the volumes of retired development stacks.
DISPOSABLE_VOLUME = re.compile(r"^([0-9a-f]{64}|genie[-_].*|genie-design-preview.*)$")


def tree_size(path):
    total = 0
    stack = [path]
    while stack:
        try:
            with os.scandir(stack.pop()) as entries:
                for entry in entries:
                    if entry.is_dir(follow_symlinks=False):
                        stack.append(entry.path)
                    elif entry.is_file(follow_symlinks=False):
                        total += entry.stat(follow_symlinks=False).st_size
        except OSError:
            continue
    return total


def busy(path, idle_seconds):
    """A directory touched recently may belong to a running build."""
    newest = path.stat().st_mtime
    for child in path.iterdir():
        newest = max(newest, child.lstat().st_mtime)
    return time.time() - newest < idle_seconds


def worktrees(root):
    entries, current = [], {}
    for field in git(root, "worktree", "list", "--porcelain", "-z").split("\0"):
        if field.startswith("worktree "):
            current = {"path": Path(field.removeprefix("worktree "))}
            entries.append(current)
        elif field.startswith("branch "):
            current["branch"] = field.removeprefix("branch refs/heads/")
        elif field == "detached":
            current["detached"] = True
        elif field.startswith("HEAD "):
            current["head"] = field.removeprefix("HEAD ")
    return entries


def merged_pr_tip(root, branch, tip):
    """A squash-merged branch looks unmerged to Git; a merged PR whose head is exactly the branch tip proves it."""
    try:
        found = json.loads(run(["gh", "pr", "list", "--head", branch, "--state", "merged", "--base", "dev",
                                "--json", "number,headRefOid"], cwd=root))
    except (RuntimeError, OSError, ValueError):
        return None
    return next((pr["number"] for pr in found if pr["headRefOid"] == tip), None)


def worktree_plan(root, base="origin/dev", protect=(), idle_seconds=24 * 3600, use_gh=True):
    root = Path(root).resolve()
    common = Path(git(root, "rev-parse", "--path-format=absolute", "--git-common-dir")).resolve()
    tip = git(root, "rev-parse", base)
    protected = {Path(p).resolve() for p in protect}
    plan = []
    for entry in worktrees(root):
        path = entry["path"]
        item = {"path": str(path), "branch": entry.get("branch"), "action": "keep"}
        plan.append(item)
        if path.resolve() == root or path.resolve() in protected or path.is_relative_to(common):
            item["reason"] = "primary, current or runtime worktree"
            continue
        if not path.exists():
            item.update(action="prune", reason="directory is gone")
            continue
        head = git(path, "rev-parse", "HEAD")
        if not clean(path):
            item["reason"] = "uncommitted changes"
            continue
        if head == tip:
            item["reason"] = "at the base tip: nothing of its own yet"
            continue
        branch = entry.get("branch")
        if ancestor(root, head, base):
            item.update(action="remove", reason="merged into " + base)
        elif branch and not [c for c in git(root, "cherry", base, head).splitlines() if c.startswith("+")]:
            item.update(action="remove", reason="every commit has an equivalent in " + base)
        elif branch and use_gh and (number := merged_pr_tip(root, branch, head)):
            item.update(action="remove", reason=f"squash-merged by PR #{number}")
        else:
            unique = int(git(root, "rev-list", "--count", f"{base}..{head}"))
            pushed = bool(branch) and subprocess.run(["git", "-C", str(root), "rev-parse", "--verify", "-q",
                                                      "refs/remotes/origin/" + branch], capture_output=True).returncode == 0
            item["reason"] = f"{unique} commit(s) not in {base}" + ("" if pushed else ", never pushed")
            item["unpublished"] = not pushed
            trim = [name for name in REBUILDABLE if (path / name).is_dir()
                    and git_ignored(path, name) and not busy(path / name, idle_seconds)]
            if trim:
                item.update(action="trim", trim=trim)
        item["bytes"] = sum(tree_size(path / name) for name in (item.get("trim") or [""]))
        if item["action"] == "keep":
            item["bytes"] = 0
    return plan


def git_ignored(path, name):
    return subprocess.run(["git", "-C", str(path), "check-ignore", "-q", name], capture_output=True).returncode == 0


def apply_worktrees(root, plan):
    for item in plan:
        path = Path(item["path"])
        if item["action"] == "remove":
            git(root, "worktree", "remove", str(path))  # no --force: Git refuses local changes
        elif item["action"] == "trim":
            for name in item["trim"]:
                shutil.rmtree(path / name)
    git(root, "worktree", "prune")


def docker_json(runner, *args):
    return [json.loads(line) for line in runner(["docker", *args, "--format", "{{json .}}"]).splitlines() if line]


def image_plan(runner, keep=2):
    """Remove surplus tags of the tracker image; keep the newest `keep` images and any image a container uses."""
    images = docker_json(runner, "image", "ls", TRACKER_IMAGES)
    if not images:
        return []
    created = {i["Id"]: i["Created"] for i in json.loads(runner(["docker", "image", "inspect", *{i["ID"] for i in images}]))}
    full = {i["ID"]: next(k for k in created if k.removeprefix("sha256:").startswith(i["ID"])) for i in images}
    used = {runner(["docker", "inspect", "--format", "{{.Image}}", c])
            for c in runner(["docker", "ps", "-aq"]).split()}
    keepers = set(sorted(created, key=created.get, reverse=True)[:keep]) | used
    return [{"tag": f"{i['Repository']}:{i['Tag']}", "id": i["ID"], "size": i["Size"], "action": "remove"}
            for i in images if full[i["ID"]] not in keepers]


def volume_plan(runner):
    plan = []
    for volume in runner(["docker", "volume", "ls", "-q", "--filter", "dangling=true"]).split():
        if PROTECTED_VOLUME.match(volume) or not DISPOSABLE_VOLUME.match(volume):
            continue
        plan.append({"volume": volume, "action": "remove"})
    return plan


def candidate_plan(preview, keep=3, base="origin/dev"):
    channels = set(read(preview.home / "channels.json", {}).values())
    current = read(preview.home / "current.json", {}).get("id")
    manifests = sorted(preview.manifests(), key=lambda m: m["createdAt"], reverse=True)
    plan = []
    for manifest in manifests[keep:]:
        if manifest["id"] in channels or manifest["id"] == current:
            continue
        merged = all(ancestor(preview.root, t["sha"], base) for t in manifest["tasks"])
        if manifest.get("status") == "failed" or merged:
            try:
                candidate_source(preview, manifest)
            except RuntimeError as exc:
                plan.append({"id": manifest["id"], "action": "keep", "reason": str(exc)})
                continue
            plan.append({"id": manifest["id"], "action": "retire",
                         "reason": "failed build" if manifest.get("status") == "failed" else "its tasks are merged"})
    return plan


def collect(preview, execute=False, keep_images=2, cache_gb=8, keep_candidates=3, idle_hours=24,
            parts=("worktrees", "candidates", "images", "volumes", "cache"), runner=run, use_gh=True):
    report = {"executed": execute}
    root = preview.root
    if "worktrees" in parts:
        report["worktrees"] = worktree_plan(root, protect=[Path.cwd()], idle_seconds=idle_hours * 3600, use_gh=use_gh)
    if "candidates" in parts:
        report["candidates"] = candidate_plan(preview, keep_candidates)
    if "images" in parts:
        report["images"] = image_plan(runner, keep_images)
    if "volumes" in parts:
        report["volumes"] = volume_plan(runner)
    if "cache" in parts:
        report["cache"] = {"keepStorage": f"{cache_gb}GB", "action": "prune"}
    if not execute:
        return report
    if "worktrees" in parts:
        apply_worktrees(root, report["worktrees"])
    for item in report.get("candidates", []):
        if item["action"] == "retire":
            manifest = next(m for m in preview.manifests() if m["id"] == item["id"])
            retire(preview, manifest, "gc")
    for item in report.get("images", []):
        runner(["docker", "rmi", item["tag"]])  # never forced: an image in use is refused
    for item in report.get("volumes", []):
        runner(["docker", "volume", "rm", item["volume"]])
    if "cache" in parts:
        runner(["docker", "builder", "prune", "-f", "--keep-storage", f"{cache_gb}GB"])
    return report


def summary(report):
    lines = []
    def gib(n):
        return f"{n / 2**30:.1f} GiB"

    for item in report.get("worktrees", []):
        if item["action"] in ("remove", "trim", "prune"):
            what = "remove worktree" if item["action"] == "remove" else (
                "drop " + ", ".join(item["trim"]) + " in" if item["action"] == "trim" else "prune")
            lines.append(f"  {what} {item['path']}  [{item['reason']}; {gib(item.get('bytes', 0))}]")
    for item in report.get("worktrees", []):
        if item.get("unpublished"):
            lines.append(f"  NOTE unpublished work: {item['branch'] or Path(item['path']).name} ({item['reason']})")
    for item in report.get("candidates", []):
        lines.append(f"  {item['action']} preview candidate {item['id']}  [{item['reason']}]")
    for item in report.get("images", []):
        lines.append(f"  remove image {item['tag']}  [{item['size']}]")
    for item in report.get("volumes", []):
        lines.append(f"  remove unused volume {item['volume'][:20]}")
    if "cache" in report:
        lines.append(f"  trim Docker build cache to {report['cache']['keepStorage']}")
    head = "Cleaned:" if report["executed"] else "Dry run, nothing removed (add --execute):"
    return "\n".join([head, *lines]) if lines else head + " nothing to do"
