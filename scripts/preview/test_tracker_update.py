import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import preview
import tracker_update as tracker


def checks(conclusion="success", status="completed", name="Quality gate"):
    return [{"name": name, "conclusion": conclusion, "status": status, "app": {"slug": "github-actions"}}]


class TrackerUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        self.u = tracker.Updater(self.root, self.root / ".git/genie-preview")
        self.before = {"Id": "old-container", "Image": "old-image", "State": {"Status": "running"},
                       "Config": {"Labels": {"com.docker.compose.project": "genie-tracker",
                                              "com.docker.compose.service": "genie"}},
                       "Mounts": [{"Destination": "/data", "Type": "volume", "Name": "genie-tracker_data"},
                                  {"Destination": "/workspace", "Type": "volume", "Name": "genie-tracker_workspace"}]}

    def commands(self, args, **kwargs):
        if args[:3] == ["docker", "image", "inspect"]:
            return "new-image"
        return ""

    def transaction(self, **overrides):
        patches = [patch.object(tracker, "git", side_effect=["", "a" * 40]),
                   patch.object(self.u, "container", return_value=self.before),
                   patch.object(self.u, "checks", return_value=checks()),
                   patch.object(self.u, "checkout"), patch.object(self.u, "preflight"),
                   patch.object(self.u, "backup"), patch.object(self.u, "health"),
                   patch.object(tracker, "run", side_effect=self.commands)]
        return patches

    def test_ci_waits_for_every_job_and_requires_quality_gate(self):
        self.assertFalse(tracker.ci_passed(checks(name="Changes")))
        self.assertFalse(tracker.ci_passed(checks() + checks(None, "in_progress", "Rust tests")))
        self.assertFalse(tracker.ci_passed(checks() + checks("failure", name="Rust tests")))
        self.assertTrue(tracker.ci_passed(checks() + checks("skipped", name="Main release gate")))

    def test_incomplete_ci_never_builds_stops_or_deploys(self):
        with patch.object(tracker, "git", side_effect=["", "a" * 40]), \
             patch.object(self.u, "container", return_value=self.before), \
             patch.object(self.u, "select_revision", return_value=None), \
             patch.object(tracker, "run") as run:
            result = self.u.update()
        self.assertEqual(result["status"], "waiting_for_ci")
        run.assert_not_called()

    def test_failing_tip_selects_a_verified_dev_ancestor_without_downgrading(self):
        candidate = "b" * 40
        with patch.object(self.u, "checks", side_effect=[checks(name="Changes"), checks()]), \
             patch.object(tracker, "run", return_value=json.dumps([{"headSha": candidate}])), \
             patch.object(tracker, "ancestor", return_value=True):
            self.assertEqual(self.u.select_revision("a" * 40, self.before), candidate)
        self.before["Config"]["Labels"][tracker.REVISION] = "c" * 40
        with patch.object(self.u, "checks", return_value=checks(name="Changes")), \
             patch.object(tracker, "run", return_value=json.dumps([{"headSha": candidate}])), \
             patch.object(tracker, "ancestor", side_effect=[True, False]):
            self.assertIsNone(self.u.select_revision("a" * 40, self.before))

    def test_volume_identity_change_is_rejected(self):
        effective = {"services": {"genie": {"container_name": "genie-tracker", "volumes": [
            {"type": "volume", "target": "/data", "source": "data"},
            {"type": "volume", "target": "/workspace", "source": "workspace"}]}},
            "volumes": {"data": {"name": "wrong_data"}, "workspace": {"name": "genie-tracker_workspace"}}}
        with patch.object(tracker, "run", return_value=json.dumps(effective)):
            with self.assertRaisesRegex(RuntimeError, "changed tracker data volume"):
                self.u.preflight(self.before)

    def test_rewound_dev_never_downgrades_even_when_old_ci_is_green(self):
        self.before["Config"]["Labels"][tracker.REVISION] = "c" * 40
        with patch.object(tracker, "ancestor", return_value=False), patch.object(self.u, "checks") as ci:
            with self.assertRaisesRegex(RuntimeError, "downgrade"):
                self.u.select_revision("a" * 40, self.before)
        ci.assert_not_called()

    def test_success_builds_before_stopping_and_records_recovery_metadata(self):
        from contextlib import ExitStack
        with ExitStack() as stack:
            entered = [stack.enter_context(p) for p in self.transaction()]
            result = self.u.update()
            commands = [call.args[0] for call in entered[-1].call_args_list]
        self.assertEqual(result["status"], "deployed")
        build = next(i for i, command in enumerate(commands) if "build" in command)
        stop = next(i for i, command in enumerate(commands) if command[:2] == ["docker", "stop"])
        up = next(i for i, command in enumerate(commands) if "up" in command)
        self.assertLess(build, stop)
        smoke = next(i for i, command in enumerate(commands) if command[:2] == ["docker", "run"])
        self.assertLess(smoke, stop)
        self.assertLess(stop, up)
        self.assertIn("--no-deps", commands[up])
        self.assertEqual(preview.read(self.u.home / "current.json")["previousImageId"], "old-image")
        self.assertFalse((self.u.home / "paused.json").exists())

    def test_image_startup_failure_keeps_original_container_running(self):
        from contextlib import ExitStack
        def command(args, **kwargs):
            if args[:2] == ["docker", "run"]:
                raise RuntimeError("entrypoint unreadable")
            return self.commands(args, **kwargs)
        with ExitStack() as stack:
            entered = [stack.enter_context(p) for p in self.transaction()]
            entered[-1].side_effect = command
            with self.assertRaisesRegex(RuntimeError, "entrypoint unreadable"):
                self.u.update()
            commands = [call.args[0] for call in entered[-1].call_args_list]
        self.assertFalse(any(command[:2] == ["docker", "stop"] for command in commands))

    def test_private_source_modes_are_normalized_only_for_tracked_build_inputs(self):
        self.u.source.mkdir(parents=True)
        helper = self.u.source / "docker/helper.sh"
        helper.parent.mkdir(mode=0o700)
        helper.write_text("sourceable helper")
        helper.chmod(0o600)
        executable = self.u.source / "run"
        executable.write_text("script")
        executable.chmod(0o700)
        index = "100644 abc 0\tdocker/helper.sh\0" + "100755 def 0\trun\0"
        with patch.object(tracker, "git", side_effect=["common", "common", "", index]), \
             patch.object(tracker, "clean", return_value=True):
            self.u.checkout("a" * 40)
        self.assertEqual(helper.stat().st_mode & 0o777, 0o644)
        self.assertEqual(helper.parent.stat().st_mode & 0o777, 0o755)
        self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
        self.assertEqual(self.u.home.stat().st_mode & 0o777, 0o700)

    def test_backup_failure_restarts_original_before_new_binary_touches_data(self):
        from contextlib import ExitStack
        with ExitStack() as stack:
            entered = [stack.enter_context(p) for p in self.transaction()]
            entered[5].side_effect = RuntimeError("disk full")
            with self.assertRaisesRegex(RuntimeError, "disk full"):
                self.u.update()
            commands = [call.args[0] for call in entered[-1].call_args_list]
        self.assertIn(["docker", "start", "genie-tracker"], commands)
        self.assertFalse(any("up" in command for command in commands))
        self.assertFalse((self.u.home / "paused.json").exists())

    def test_failed_new_runtime_pauses_and_never_downgrades_migrated_data(self):
        from contextlib import ExitStack
        with ExitStack() as stack:
            entered = [stack.enter_context(p) for p in self.transaction()]
            entered[6].side_effect = RuntimeError("health failed")
            with self.assertRaisesRegex(RuntimeError, "health failed"):
                self.u.update()
            commands = [call.args[0] for call in entered[-1].call_args_list]
        self.assertFalse(any(command[:2] == ["docker", "start"] for command in commands))
        self.assertEqual(preview.read(self.u.home / "paused.json")["previousImageId"], "old-image")
        with self.assertRaisesRegex(RuntimeError, "paused"):
            self.u.update()

    def test_successful_update_prunes_surplus_images_but_never_fails_the_deployment(self):
        surplus = [{"tag": "genie-tracker:dev-old", "id": "abc", "size": "1MB", "action": "remove"}]
        with patch.object(tracker.diskgc, "image_plan", return_value=surplus) as plan, \
             patch.object(tracker, "run") as run:
            self.u.prune_images()
            plan.assert_called_once_with(tracker.run, 2)
            run.assert_called_once_with(["docker", "rmi", "genie-tracker:dev-old"])
            run.side_effect = RuntimeError("image is in use")
            self.u.prune_images()

    def test_unchanged_running_revision_is_idempotent(self):
        self.before["Config"]["Labels"][tracker.REVISION] = "a" * 40
        with patch.object(tracker, "git", side_effect=["", "a" * 40]), \
             patch.object(self.u, "container", return_value=self.before), \
             patch.object(self.u, "health"), patch.object(tracker, "run") as run:
            self.assertEqual(self.u.update()["status"], "current")
        run.assert_not_called()

    def test_tracker_public_url_is_discovered_and_configuration_drift_requires_update(self):
        preview.write(self.u.preview_home / "services/config.json", {"trackerService": True, "tailnetDomain": "example.ts.net"})
        self.before["Config"]["Labels"][tracker.REVISION] = "a" * 40
        self.assertEqual(self.u.public_url(), "https://genie-self.example.ts.net")
        self.assertFalse(self.u.runtime_matches(self.before, "a" * 40))
        self.before["Config"]["Env"] = ["GENIE_PUBLIC_URL=https://genie-self.example.ts.net"]
        self.assertTrue(self.u.runtime_matches(self.before, "a" * 40))


if __name__ == "__main__":
    unittest.main()
