import json
from pathlib import Path
import tempfile
import unittest

import diskgc
import preview
from test_preview import FakePreview


class FakeDocker:
    """Answers the few docker queries the planner makes and records mutating calls."""

    def __init__(self, images, used=(), volumes=()):
        self.images, self.used, self.volumes, self.calls = images, used, volumes, []

    def __call__(self, args, **_):
        args = [str(a) for a in args]
        if args[:3] == ["docker", "image", "ls"]:
            return "\n".join(json.dumps({"Repository": "genie-tracker", "Tag": t, "ID": i, "Size": "878MB"})
                             for t, i, _ in self.images)
        if args[:3] == ["docker", "image", "inspect"]:
            return json.dumps([{"Id": "sha256:" + i.ljust(64, "0"), "Created": c} for _, i, c in self.images])
        if args[:3] == ["docker", "ps", "-aq"]:
            return "\n".join(f"c{n}" for n, _ in enumerate(self.used))
        if args[:2] == ["docker", "inspect"]:
            return "sha256:" + self.used[int(args[-1][1:])].ljust(64, "0")
        if args[:3] == ["docker", "volume", "ls"]:
            return "\n".join(self.volumes)
        self.calls.append(args)
        return ""


class DiskGcTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        preview.run(["git", "init", "-b", "dev", self.root])
        preview.git(self.root, "config", "user.name", "Test")
        preview.git(self.root, "config", "user.email", "test@localhost")
        (self.root / ".gitignore").write_text("target/\nnode_modules/\n")
        (self.root / "base").write_text("base")
        preview.git(self.root, "add", ".")
        preview.git(self.root, "commit", "-m", "base")
        self.p = FakePreview(self.root, self.root / ".git/genie-preview")

    def branch(self, name, files=1, merge=False, squash=False):
        path = Path(self.temp.name) / name
        preview.git(self.root, "worktree", "add", "-b", name, path)
        for n in range(files):
            (path / f"{name}{n}").write_text(name)
            preview.git(path, "add", ".")
            preview.git(path, "commit", "-m", name)
        if merge:
            preview.git(self.root, "merge", "--no-ff", "-m", "merge " + name, name)
        if squash:
            # Another change on dev first, so the cherry-pick is a new commit like a squash merge.
            (self.root / "other").write_text("other")
            preview.git(self.root, "add", ".")
            preview.git(self.root, "commit", "-m", "other")
            preview.git(self.root, "cherry-pick", name)
        preview.git(self.root, "update-ref", "refs/remotes/origin/dev", "HEAD")
        return path

    def plan(self, **kwargs):
        plan = diskgc.worktree_plan(self.root, idle_seconds=0, use_gh=False, **kwargs)
        return {Path(i["path"]).name: i for i in plan}

    def test_merged_and_patch_equivalent_worktrees_are_removed(self):
        self.branch("done", merge=True)
        self.branch("squashed", squash=True)
        plan = self.plan()
        self.assertEqual(plan["done"]["action"], "remove")
        self.assertEqual(plan["squashed"]["action"], "remove")
        self.assertIn("equivalent", plan["squashed"]["reason"])

    def test_unmerged_work_keeps_commits_and_only_drops_rebuildable_output(self):
        work = self.branch("wip")
        (work / "target").mkdir()
        (work / "target/artifact").write_text("x" * 1000)
        (work / "docs").mkdir()
        plan = self.plan()["wip"]
        self.assertEqual((plan["action"], plan["trim"], plan["unpublished"]), ("trim", ["target"], True))
        diskgc.apply_worktrees(self.root, [plan])
        self.assertTrue(work.exists())
        self.assertFalse((work / "target").exists())
        self.assertEqual(preview.git(self.root, "rev-list", "--count", "dev..wip"), "1")

    def test_dirty_fresh_current_and_busy_worktrees_are_preserved(self):
        dirty = self.branch("dirty", merge=True)
        (dirty / "scratch").write_text("not committed")
        fresh = Path(self.temp.name) / "fresh"
        preview.git(self.root, "worktree", "add", "-b", "fresh", fresh)
        busy = self.branch("busy")
        (busy / "target").mkdir()
        plan = diskgc.worktree_plan(self.root, idle_seconds=3600, use_gh=False)
        by_name = {Path(i["path"]).name: i for i in plan}
        self.assertEqual(by_name["dirty"]["action"], "keep")
        self.assertEqual(by_name["fresh"]["action"], "keep")
        self.assertEqual(by_name["busy"]["action"], "keep")
        self.assertEqual(by_name["repo"]["action"], "keep")
        protected = diskgc.worktree_plan(self.root, protect=[dirty], idle_seconds=0, use_gh=False)
        self.assertTrue(all(i["action"] == "keep" for i in protected if i["path"] == str(dirty)))

    def test_removal_refuses_changes_that_appear_after_planning(self):
        done = self.branch("done", merge=True)
        plan = [self.plan()["done"]]
        (done / "late").write_text("appeared after the plan")
        with self.assertRaises(RuntimeError):
            diskgc.apply_worktrees(self.root, plan)
        self.assertTrue(done.exists())

    def test_images_keep_newest_and_those_in_use(self):
        images = [("dev-a", "aaa", "2026-01-01"), ("before-1", "bbb", "2026-01-02"), ("dev-b", "ccc", "2026-01-03"),
                  ("dev-c", "ddd", "2026-01-04")]
        plan = diskgc.image_plan(FakeDocker(images, used=["aaa"]), keep=2)
        self.assertEqual(sorted(i["tag"] for i in plan), ["genie-tracker:before-1"])

    def test_only_unused_disposable_volumes_are_removed(self):
        anonymous = "a" * 64
        docker = FakeDocker([], volumes=[anonymous, "genie-tracker_data", "genie_genie-data",
                                         "genie-design-preview_genie-workspace", "searxng_cache"])
        self.assertEqual([v["volume"] for v in diskgc.volume_plan(docker)],
                         [anonymous, "genie_genie-data", "genie-design-preview_genie-workspace"])

    def test_dry_run_changes_nothing_and_execute_uses_unforced_docker_calls(self):
        docker = FakeDocker([("dev-a", "aaa", "1"), ("dev-b", "bbb", "2")], volumes=["a" * 64])
        report = diskgc.collect(self.p, keep_images=1, parts=("images", "volumes", "cache"), runner=docker)
        self.assertEqual(docker.calls, [])
        self.assertIn("Dry run", diskgc.summary(report))
        diskgc.collect(self.p, execute=True, keep_images=1, parts=("images", "volumes", "cache"), runner=docker)
        self.assertEqual(docker.calls, [["docker", "rmi", "genie-tracker:dev-a"],
                                        ["docker", "volume", "rm", "a" * 64],
                                        ["docker", "builder", "prune", "-f", "--keep-storage", "8GB"]])


if __name__ == "__main__":
    unittest.main()
