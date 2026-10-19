import concurrent.futures
import contextlib
import importlib.util
import io
import json
import os
import http.server
import http.client
import socket
import subprocess
import sys
import threading
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

import preview


class FakePreview(preview.Preview):
    def artifacts(self, source, directory, manifest):
        with (self.home / "build-count").open("a") as out:
            out.write("build\n")
        time.sleep(.1)

    def start_candidate(self, directory, manifest):
        proc = {"pid": os.getpid(), "start": preview.process_id(os.getpid())}
        manifest.update(backend=proc, gateway=proc)

    @staticmethod
    def stop_process(proc):
        pass


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        preview.run(["git", "init", "-b", "main", self.root])
        preview.git(self.root, "config", "user.name", "Test")
        preview.git(self.root, "config", "user.email", "test@localhost")
        (self.root / "shared").write_text("base\n")
        preview.git(self.root, "add", ".")
        preview.git(self.root, "commit", "-m", "base")
        preview.git(self.root, "update-ref", "refs/remotes/origin/dev", "HEAD")
        self.base = preview.git(self.root, "rev-parse", "HEAD")
        self.home = self.root / ".git/genie-preview"
        self.p = FakePreview(self.root, self.home)

    def task(self, name, filename, content):
        root = Path(self.temp.name) / name
        preview.git(self.root, "worktree", "add", "-b", name, str(root), self.base)
        (root / filename).write_text(content)
        preview.git(root, "add", ".")
        preview.git(root, "commit", "-m", name)
        return root

    def mark(self, root, task):
        with contextlib.redirect_stdout(io.StringIO()):
            self.p.ready(root, task)

    def test_combination_is_exact_and_duplicate_requests_reuse_artifacts(self):
        a = self.task("a", "a", "A")
        b = self.task("b", "b", "B")
        self.mark(a, "GNE-101")
        self.mark(b, "GNE-102")
        result = self.p.build()
        source = self.home / "candidates" / result["id"] / "source"
        self.assertEqual((source / "a").read_text(), "A")
        self.assertEqual((source / "b").read_text(), "B")
        self.assertEqual(self.p.build()["id"], result["id"])
        self.assertEqual((self.home / "build-count").read_text().splitlines(), ["build"])
        self.assertEqual(preview.git(a, "rev-parse", "HEAD"), result["tasks"][0]["sha"])
        self.assertEqual(preview.git(self.root, "rev-parse", "HEAD"), self.base)

    def test_simultaneous_requests_serialize_and_deduplicate(self):
        a = self.task("a", "a", "A")
        self.mark(a, "GNE-101")
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            results = list(pool.map(lambda _: self.p.build(), range(2)))
        self.assertEqual(results[0]["id"], results[1]["id"])
        self.assertEqual((self.home / "build-count").read_text().splitlines(), ["build"])

    def test_dirty_or_changed_head_is_never_published(self):
        a = self.task("a", "a", "A")
        self.mark(a, "GNE-101")
        (a / "untracked").write_text("unfinished")
        with self.assertRaisesRegex(RuntimeError, "Stale readiness"):
            self.p.build()
        with self.assertRaisesRegex(RuntimeError, "Commit all"):
            self.p.ready(a, "GNE-101")
        preview.git(a, "add", ".")
        preview.git(a, "commit", "-m", "changed head")
        with self.assertRaisesRegex(RuntimeError, "Stale readiness"):
            self.p.build()

    def test_conflict_preserves_current_candidate(self):
        a = self.task("a", "shared", "A\n")
        b = self.task("b", "shared", "B\n")
        self.mark(a, "GNE-101")
        first = self.p.build()
        self.mark(b, "GNE-102")
        with self.assertRaisesRegex(RuntimeError, "Integration conflict.*GNE-102"):
            self.p.build()
        self.assertEqual(preview.read(self.home / "current.json")["id"], first["id"])
        self.assertEqual((a / "shared").read_text(), "A\n")
        self.assertEqual((b / "shared").read_text(), "B\n")

    def test_mid_build_edit_is_detected_before_start(self):
        a = self.task("a", "a", "A")
        self.mark(a, "GNE-101")
        with patch.object(self.p, "artifacts", side_effect=lambda *_: (a / "a").write_text("changed")), \
             patch.object(self.p, "start_candidate") as start:
            with self.assertRaisesRegex(RuntimeError, "Stale readiness"):
                self.p.build()
            start.assert_not_called()
        self.assertFalse((self.home / "current.json").exists())

    def test_settle_without_current_readiness_is_noop(self):
        a = self.task("a", "a", "A")
        with patch.object(self.p, "ensure_index") as start, patch.object(preview, "spawn") as spawn:
            self.assertFalse(self.p.request(a))
            start.assert_not_called()
            spawn.assert_not_called()

    def test_withdraw_removes_only_current_task(self):
        a = self.task("a", "a", "A")
        b = self.task("b", "b", "B")
        self.mark(a, "GNE-101")
        self.mark(b, "GNE-102")
        self.p.withdraw(a)
        self.assertEqual([e["task"] for e in self.p.selection()], ["GNE-102"])

    def test_runtime_does_not_inherit_tracker_or_provider_secrets(self):
        with patch.dict(os.environ, {"GENIE_DATA": "/live/tracker", "GENIE_TOKEN": "private",
                                     "OPENAI_API_KEY": "private", "LITELLM_API_KEY": "private"}):
            env = self.p.runtime_env(self.home)
        self.assertFalse(any(k.startswith("GENIE_") or k.endswith("API_KEY") for k in env))
        self.assertEqual(env["HOME"], str(self.home / "home"))

    def test_unrelated_tailscale_listener_is_preserved(self):
        self.p.config["tailscaleHost"] = "machine.example.ts.net"
        status = {"TCP": {"8443": {"HTTPS": True}}, "Web": {
            "machine.example.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:9999"}}}}}
        with patch.object(preview, "run", return_value=json.dumps(status)) as run:
            with self.assertRaisesRegex(RuntimeError, "Preserving unrelated"):
                self.p.publish_tailscale(8443, 7480)
            self.assertEqual(run.call_count, 1)

    def test_tailnet_name_is_discovered_at_enable_time(self):
        responses = [json.dumps({"BackendState": "Running", "Self": {"DNSName": "machine.new-tailnet.ts.net."}})]
        with patch.object(preview, "run", side_effect=responses), patch.object(self.p, "publish_tailscale") as publish:
            self.p.enable_tailscale()
        self.assertEqual(self.p.config["tailscaleHost"], "machine.new-tailnet.ts.net")
        publish.assert_called_once_with(8443, 7480)

    def test_channel_build_needs_no_task_and_does_not_replace_preview(self):
        a = self.task("a", "a", "A")
        self.mark(a, "GNE-101")
        candidate = self.p.build()
        dev = self.p.build("dev")
        self.assertEqual(dev["tasks"], [])
        self.assertEqual(dev["base"], self.base)
        self.assertEqual(preview.read(self.home / "channels.json")["dev"], dev["id"])
        self.assertEqual(preview.read(self.home / "current.json")["id"], candidate["id"])

    def test_lan_routing_uses_only_known_manifests(self):
        import lan
        cid = "1" * 20
        proc = {"pid": os.getpid(), "start": preview.process_id(os.getpid())}
        manifest = {"id": cid, "port": 8500, "status": "ready", "gateway": proc, "backend": proc}
        preview.write(self.home / "candidates" / cid / "manifest.json", manifest)
        preview.write(self.home / "current.json", {"id": cid})
        preview.write(self.home / "channels.json", {"dev": cid, "main": cid})
        for host in ("genie.local", "dev.genie.local", "preview.genie.local", cid + ".preview.genie.local"):
            self.assertEqual(lan.resolve(host, self.home), manifest)
        with self.assertRaisesRegex(ValueError, "Unexpected Host"):
            lan.resolve("evil.example", self.home)
        with self.assertRaisesRegex(RuntimeError, "not running"):
            lan.resolve("2" * 20 + ".preview.genie.local", self.home)

    def test_prepare_excludes_the_symlink_and_reuses_dependency_installation(self):
        for name in ("package.json", "package-lock.json", "web/package.json"):
            path = self.root / name
            path.parent.mkdir(exist_ok=True)
            path.write_text("{}")
        def install(args, **kwargs):
            (self.root / "node_modules").mkdir(exist_ok=True)
            return ""
        original = preview.run
        def command(args, **kwargs):
            return install(args, **kwargs) if args[0] == "npm" else original(args, **kwargs)
        with patch.object(preview, "run", side_effect=command) as run, patch.object(self.p, "ensure_index"):
            self.p.prepare(self.root)
            self.p.prepare(self.root)
        self.assertEqual(sum(call.args[0][0] == "npm" for call in run.call_args_list), 1)
        self.assertEqual(preview.git(self.root, "check-ignore", ".genie-preview"), ".genie-preview")
        self.assertTrue((self.root / ".genie-preview").is_symlink())

    def test_gateway_isolates_cookies_and_streams_responses(self):
        cid = "1" * 20
        # Reserve an adjacent pair for the gateway and its private upstream.
        for port in range(35000, 45000):
            try:
                with socket.socket() as sock:
                    sock.bind(("127.0.0.1", port))
                server = http.server.ThreadingHTTPServer(("127.0.0.1", port + 100), self.upstream())
                break
            except OSError:
                continue
        else:
            self.fail("No test port pair available")
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        preview.write(self.home / "candidates" / cid / "manifest.json", {"id": cid, "port": port})
        proc = subprocess.Popen([sys.executable, Path(preview.__file__), "--root", self.root,
                                 "--home", self.home, "gateway", cid],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: (proc.terminate(), proc.wait(timeout=3)))
        self.p.wait_health(port, cid)
        conn = http.client.HTTPConnection("127.0.0.1", port)
        conn.request("GET", "/echo", headers={"Cookie": f"pv_{cid}_session=mine; pv_{'2'*20}_session=other; unrelated=hidden"})
        response = conn.getresponse()
        self.assertEqual(response.getheader("Set-Cookie"), f"pv_{cid}_session=issued; Path=/; HttpOnly")
        self.assertEqual(response.read().decode(), "session=mine")
        conn.close()

    @staticmethod
    def upstream():
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                body = self.headers.get("Cookie", "").encode()
                self.send_response(200)
                self.send_header("Set-Cookie", "session=issued; Path=/; HttpOnly")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
        return Upstream


if __name__ == "__main__":
    unittest.main()
