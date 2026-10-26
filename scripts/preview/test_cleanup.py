import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import cleanup
import preview
from test_preview import FakePreview


class CleanupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        preview.run(["git", "init", "-b", "dev", self.root])
        preview.git(self.root, "config", "user.name", "Test")
        preview.git(self.root, "config", "user.email", "test@localhost")
        (self.root / "base").write_text("base")
        (self.root / ".gitignore").write_text("node_modules/\n")
        preview.git(self.root, "add", ".")
        preview.git(self.root, "commit", "-m", "base")
        self.home = self.root / ".git/genie-preview"
        self.p = FakePreview(self.root, self.home)
        self.task = Path(self.temp.name) / "task"
        preview.git(self.root, "worktree", "add", "-b", "feature", self.task)
        (self.task / "feature").write_text("change")
        preview.git(self.task, "add", ".")
        preview.git(self.task, "commit", "-m", "task")
        self.sha = preview.git(self.task, "rev-parse", "HEAD")
        preview.git(self.root, "update-ref", "refs/remotes/origin/dev", self.sha)
        self.info = {"url": "https://example.test/pr/1", "state": "MERGED", "baseRefName": "dev",
                     "headRefName": "feature", "headRefOid": self.sha, "mergeCommit": {"oid": self.sha}}
        self.p.ready(self.task, "GNE-101")

    def merged(self, **kwargs):
        with patch.object(cleanup, "merged_pr", return_value=self.info):
            return cleanup.cleanup_merged(self.p, "1", "GNE-101", self.task, **kwargs)

    def test_retire_stops_processes_removes_source_and_workspace_and_reserves_ports(self):
        m = self.p.build()
        directory = self.home / "candidates" / m["id"]
        for root in (self.task, directory / "source"):
            (root / "node_modules").mkdir()
            (root / "node_modules/cache").write_text("disposable ignored dependency")
        m.update(port=35000, backendPort=35100)
        preview.write(directory / "manifest.json", m)
        plan = self.merged()
        self.assertTrue(self.task.exists())
        self.assertEqual(plan["retire"], [m["id"]])
        with patch.object(self.p, "stop_process") as stop:
            self.merged(execute=True)
        self.assertEqual(stop.call_count, 2)
        self.assertFalse(self.task.exists())
        self.assertFalse(directory.exists())
        self.assertFalse((self.home / "current.json").exists())
        self.assertEqual(preview.read(self.p.ready_path), {})
        tombstone = preview.read(self.home / "retired" / (m["id"] + ".json"))
        self.assertEqual(tombstone["port"], 35000)
        self.p.config.update(candidatePortStart=35000, candidatePortEnd=35001)
        self.assertEqual(self.p.next_port(), 35001)
        self.assertEqual(self.merged(execute=True)["retire"], [])

    def test_dirty_task_or_candidate_preserves_everything(self):
        m = self.p.build()
        (self.task / "untracked").write_text("keep")
        with self.assertRaisesRegex(RuntimeError, "dirty"):
            self.merged(execute=True)
        (self.task / "untracked").unlink()
        source = self.home / "candidates" / m["id"] / "source"
        (source / "base").write_text("edited")
        with self.assertRaisesRegex(RuntimeError, "candidate worktree"):
            self.merged(execute=True)
        self.assertTrue(self.task.exists())
        self.assertEqual(preview.read(self.home / "current.json")["id"], m["id"])

    def test_mixed_unmerged_candidate_is_retained(self):
        m = self.p.build()
        other = Path(self.temp.name) / "other"
        preview.git(self.root, "worktree", "add", "-b", "other", other)
        (other / "other").write_text("not merged")
        preview.git(other, "add", ".")
        preview.git(other, "commit", "-m", "other")
        m["tasks"].append({"task": "GNE-102", "sha": preview.git(other, "rev-parse", "HEAD")})
        preview.write(self.home / "candidates" / m["id"] / "manifest.json", m)
        result = self.merged(execute=True)
        self.assertEqual(result["retire"], [])
        self.assertEqual(len(result["retain"]), 1)
        self.assertTrue(other.exists())

    def test_live_worktree_must_be_retained_explicitly(self):
        self.p.root = self.task
        with self.assertRaisesRegex(RuntimeError, "Move outside"):
            self.merged(execute=True)
        self.merged(execute=True, keep_worktree=True)
        self.assertTrue(self.task.exists())

    def test_missing_candidate_registration_is_removed_without_pruning_other_worktrees(self):
        import shutil
        source = self.home / "candidates" / ("a" * 20) / "source"
        preview.git(self.root, "worktree", "add", "--detach", source)
        unrelated = Path(self.temp.name) / "unrelated"
        preview.git(self.root, "worktree", "add", "--detach", unrelated)
        shutil.rmtree(source)
        shutil.rmtree(unrelated)
        result = self.merged(execute=True)
        self.assertEqual(result["missingCandidateWorktrees"], [str(source)])
        registry = preview.git(self.root, "worktree", "list", "--porcelain")
        self.assertNotIn(str(source), registry)
        self.assertIn(str(unrelated), registry)

    def test_open_pr_or_other_target_is_not_cleanup_authority(self):
        for state, base in (("OPEN", "dev"), ("MERGED", "main")):
            info = dict(self.info, state=state, baseRefName=base)
            with patch.object(cleanup, "run", return_value=json.dumps(info)):
                with self.assertRaisesRegex(RuntimeError, "confirmed PR merge"):
                    cleanup.merged_pr(self.root, "1")

    def test_unrelated_tailscale_handler_is_preserved(self):
        self.p.config["tailscale"] = True
        status = {"TCP": {"8500": {}}, "Web": {"host.example:8500": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:9999"}}}}}
        with patch.object(cleanup, "run", return_value=json.dumps(status)) as run:
            with self.assertRaisesRegex(RuntimeError, "unrelated"):
                cleanup.unpublish(self.p, 8500)
        self.assertEqual(run.call_count, 1)

    def test_demo_credentials_migrate_existing_data_only_once(self):
        directory = self.home / "demo"
        directory.mkdir()
        with patch.object(preview, "run") as run:
            self.p.demo_credentials(directory, directory / "data", {})
            self.p.demo_credentials(directory, directory / "data", {})
        self.assertEqual(run.call_count, 1)
        self.assertEqual(run.call_args.kwargs["data"], b"preview123\n")
        self.assertEqual((directory / "credentials.txt").read_text(), "Login: preview\nPassword: preview123\n")

    def test_stop_waits_until_the_owned_preview_process_has_exited(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: child.kill() if child.poll() is None else None)
        proc = {"pid": child.pid, "start": preview.process_id(child.pid)}
        preview.Preview.stop_process(proc)
        child.wait(timeout=2)
        self.assertFalse(preview.alive(proc))

    def test_shared_processes_launch_from_primary_checkout_instead_of_task_worktree(self):
        p = preview.Preview(self.task, self.home)
        with patch.object(preview, "spawn") as spawn:
            p.launch(["serve"], self.home / "index.log")
        self.assertEqual(spawn.call_args.kwargs["cwd"], self.root)
        self.assertNotIn(self.task, spawn.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
