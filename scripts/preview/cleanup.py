"""Retire task previews and remove a clean workspace after a verified merge into dev."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

from preview import clean, git, identity, lock, read, run, write


def ancestor(root, older, newer):
    result = subprocess.run(["git", "-C", str(root), "merge-base", "--is-ancestor", older, newer], capture_output=True)
    if result.returncode not in (0, 1):
        raise RuntimeError("Cannot verify commit ancestry: " + older)
    return result.returncode == 0


def merged_pr(root, number):
    info = json.loads(run(["gh", "pr", "view", number, "--json",
                           "url,state,baseRefName,headRefName,headRefOid,mergeCommit"], cwd=root))
    if info["state"] != "MERGED" or info["baseRefName"] != "dev" or not info.get("mergeCommit"):
        raise RuntimeError("Cleanup requires a confirmed PR merge into dev")
    git(root, "fetch", "origin", "dev")
    if not ancestor(root, info["mergeCommit"]["oid"], "origin/dev"):
        raise RuntimeError("The PR merge is not present in fetched origin/dev")
    return info


def unpublish(preview, port):
    if not port or not preview.config["tailscale"]:
        return
    status = json.loads(run(["tailscale", "serve", "status", "--json"]))
    handlers = [entry.get("Handlers", {}) for endpoint, entry in status.get("Web", {}).items()
                if endpoint.endswith(":" + str(port))]
    wanted = {"/": {"Proxy": f"http://127.0.0.1:{port}"}}
    if any(handler != wanted for handler in handlers) or (not handlers and str(port) in status.get("TCP", {})):
        raise RuntimeError("Preserving unrelated Serve configuration on port " + str(port))
    if handlers:
        run(["tailscale", "serve", f"--https={port}", "off"])


def candidate_source(preview, manifest):
    cid = manifest["id"]
    if not re.fullmatch(r"[a-f0-9]{20}", cid):
        raise RuntimeError("Invalid candidate ID")
    directory = preview.home / "candidates" / cid
    if directory.is_symlink():
        raise RuntimeError("Preserving unexpected candidate symlink")
    source = directory / "source"
    if source.exists():
        common = git(source, "rev-parse", "--path-format=absolute", "--git-common-dir")
        if common != git(preview.root, "rev-parse", "--path-format=absolute", "--git-common-dir") or not clean(source):
            raise RuntimeError("Preserving changed or unrelated candidate worktree: " + str(source))
    return directory, source


def retire(preview, manifest, pr_url):
    directory, source = candidate_source(preview, manifest)
    # A compact tombstone keeps old fixed URLs' ports reserved permanently.
    tombstone = {k: manifest[k] for k in ("id", "base", "tasks", "port", "backendPort") if k in manifest}
    tombstone.update(status="retired", retiredAt=time.time(), mergedPR=pr_url)
    write(preview.home / "retired" / (manifest["id"] + ".json"), tombstone)
    manifest["status"] = "retiring"
    write(directory / "manifest.json", manifest)
    for name in ("gateway", "backend"):
        preview.stop_process(manifest.get(name))
    unpublish(preview, manifest.get("port"))
    if source.exists():
        git(preview.root, "worktree", "remove", str(source))
    current = preview.home / "current.json"
    if read(current, {}).get("id") == manifest["id"]:
        current.unlink()
    shutil.rmtree(directory)


def missing_sources(preview):
    paths = []
    for field in git(preview.root, "worktree", "list", "--porcelain", "-z").split("\0"):
        if not field.startswith("worktree "):
            continue
        path = Path(field.removeprefix("worktree "))
        if (path.name == "source" and path.parent.parent == preview.home / "candidates"
                and re.fullmatch(r"[a-f0-9]{20}", path.parent.name) and not path.exists()):
            paths.append(str(path))
    return paths


def cleanup_merged(preview, number, task, worktree, execute=False, keep_worktree=False):
    if not re.fullmatch(r"GNE-[1-9][0-9]*", task):
        raise RuntimeError("Expected an explicit GNE task ID")
    worktree = Path(worktree).resolve()
    with lock(preview.home / "build.lock"), lock(preview.state_lock):
        info = merged_pr(preview.root, number)
        if worktree.exists():
            entry = identity(worktree)
            same_repo = (git(worktree, "rev-parse", "--path-format=absolute", "--git-common-dir") ==
                         git(preview.root, "rev-parse", "--path-format=absolute", "--git-common-dir"))
            if not same_repo or entry["sha"] != info["headRefOid"] or entry["branch"] != info["headRefName"] or not clean(worktree):
                raise RuntimeError("Preserving changed, dirty or unrelated task worktree")
            if not keep_worktree and any(worktree == path or worktree in path.parents
                                         for path in (preview.root.resolve(), Path.cwd().resolve())):
                raise RuntimeError("Move outside the task worktree before removal; use --keep-worktree for a live T3 binding")
        entries = read(preview.ready_path, {})
        previous = entries.get(str(worktree))
        if previous and previous["task"] != task:
            raise RuntimeError("Preserving another task's readiness")
        selected, retained = [], []
        for manifest in preview.manifests():
            own = [t for t in manifest["tasks"] if t["task"] == task]
            if not own:
                continue
            if not all(ancestor(preview.root, t["sha"], info["headRefOid"]) for t in own):
                retained.append({"id": manifest["id"], "reason": "task commit is outside the merged PR"})
                continue
            others = [t for t in manifest["tasks"] if t["task"] != task]
            if not all(ancestor(preview.root, t["sha"], "origin/dev") for t in others):
                retained.append({"id": manifest["id"], "reason": "contains other unmerged task commits"})
                continue
            candidate_source(preview, manifest)
            selected.append(manifest)
        plan = {"pr": info["url"], "task": task, "retire": [m["id"] for m in selected], "retain": retained,
                "missingCandidateWorktrees": missing_sources(preview),
                "worktree": str(worktree), "removeWorktree": not keep_worktree, "executed": execute}
        if not execute:
            return plan
        for manifest in selected:
            retire(preview, manifest, info["url"])
        for source in plan["missingCandidateWorktrees"]:
            git(preview.root, "worktree", "remove", source)
        entries.pop(str(worktree), None)
        write(preview.ready_path, entries)
        if worktree.exists() and not keep_worktree:
            # No --force: Git refuses local changes or newly added files.
            git(preview.root, "worktree", "remove", str(worktree))
        write(preview.home / "cleanup" / (task + ".json"), plan)
        return plan
