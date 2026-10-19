import http.client
import http.server
import json
import io
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import preview
import service_router


class ServiceRouterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        preview.write(self.home / "services/config.json", {"tailnetDomain": "example.ts.net"})
        self.process = {"pid": os.getpid(), "start": preview.process_id(os.getpid())}
        self.first = self.upstream("1" * 20)
        self.second = self.upstream("2" * 20)
        preview.write(self.home / "current.json", {"id": self.first["id"]})
        preview.write(self.home / "channels.json", {"main": self.first["id"], "dev": self.second["id"]})

    def server(self, handler):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        return server.server_port

    def upstream(self, cid):
        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                body = json.dumps({"id": cid, "method": self.command, "cookie": self.headers.get("Cookie"),
                                   "body": self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(body)

            do_POST = do_GET

        port = self.server(Upstream)
        manifest = {"id": cid, "port": port, "base": "a" * 40, "tasks": [], "createdAt": 1,
                    "status": "ready", "backend": self.process, "gateway": self.process,
                    "urls": {"remote": f"https://fixed.example.ts.net:{port}"}}
        preview.write(self.home / "candidates" / cid / "manifest.json", manifest)
        return manifest

    def call(self, port, host, path="/api/test", method="GET", body=None, headers=None):
        client = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        client.request(method, path, body=body, headers={"Host": host, **(headers or {})})
        response = client.getresponse()
        status, data = response.status, response.read().decode()
        client.close()
        return status, data

    def test_named_channels_route_independently_and_preview_follows_current(self):
        main = self.server(service_router.make_handler("main", self.home))
        dev = self.server(service_router.make_handler("dev", self.home))
        candidate = self.server(service_router.make_handler("preview", self.home))
        self.assertEqual(json.loads(self.call(main, "genie.example.ts.net")[1])["id"], self.first["id"])
        self.assertEqual(json.loads(self.call(dev, "genie-dev.example.ts.net")[1])["id"], self.second["id"])
        self.assertEqual(json.loads(self.call(candidate, "genie-preview.example.ts.net")[1])["id"], self.first["id"])
        preview.write(self.home / "current.json", {"id": self.second["id"]})
        self.assertEqual(json.loads(self.call(candidate, "genie-preview.example.ts.net")[1])["id"], self.second["id"])
        self.assertEqual(json.loads(self.call(main, "genie.example.ts.net")[1])["id"], self.first["id"])

    def test_api_writes_and_cookies_are_forwarded_to_selected_candidate(self):
        port = self.server(service_router.make_handler("preview", self.home))
        status, data = self.call(port, "genie-preview.example.ts.net:443", method="POST", body="example",
                                 headers={"Cookie": "candidate-session=value", "X-Genie": "1"})
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(data), {"id": self.first["id"], "method": "POST",
                                          "cookie": "candidate-session=value", "body": "example"})

    def test_unrecognized_hosts_are_rejected_and_dashboard_uses_fixed_https_urls(self):
        port = self.server(service_router.make_handler("preview", self.home))
        self.assertEqual(self.call(port, "unrelated.example.com")[0], 421)
        status, page = self.call(port, "genie-preview.example.ts.net", "/__previews")
        self.assertEqual(status, 200)
        self.assertIn("https://genie.example.ts.net", page)
        self.assertIn(self.first["urls"]["remote"], page)
        self.assertNotIn("127.0.0.1", page)

    def test_offline_candidate_returns_unavailable_instead_of_another_environment(self):
        self.first["backend"] = {"pid": 999999999, "start": "missing"}
        preview.write(self.home / "candidates" / self.first["id"] / "manifest.json", self.first)
        port = self.server(service_router.make_handler("main", self.home))
        self.assertEqual(self.call(port, "genie.example.ts.net")[0], 503)

    def setup_enable(self):
        config = {"tailnetDomain": "example.ts.net", "socket": "/isolated/socket", "tag": "tag:genie-services"}
        preview.write(self.home / "services/config.json", config)
        status = {"BackendState": "Running", "Self": {"DNSName": "service-host.example.ts.net.", "Tags": [config["tag"]]}}
        return preview.Preview(self.home, self.home), status

    def test_named_index_becomes_default_only_after_all_candidates_match(self):
        p, status = self.setup_enable()
        responses = [io.BytesIO(json.dumps({"preview": m["id"]}).encode()) for m in (self.first, self.second, self.first)]
        with patch.object(preview, "run", return_value=json.dumps(status)), \
             patch.object(preview.urllib.request, "urlopen", side_effect=responses):
            p.enable_services()
        self.assertEqual(p.urls()["remote"], "https://genie-preview.example.ts.net/__previews")
        self.assertEqual(preview.read(p.config_path)["serviceUrls"]["main"], "https://genie.example.ts.net")

    def test_wrong_candidate_does_not_select_unverified_service_urls(self):
        p, status = self.setup_enable()
        with patch.object(preview, "run", return_value=json.dumps(status)), \
             patch.object(preview.urllib.request, "urlopen", return_value=io.BytesIO(b'{"preview":"wrong"}')):
            with self.assertRaisesRegex(RuntimeError, "expected candidate"):
                p.enable_services()
        self.assertNotIn("serviceUrls", p.config)
        self.assertFalse(p.config_path.exists())

    def test_service_domain_is_discovered_without_manual_tailnet_configuration(self):
        p, status = self.setup_enable()
        (self.home / "services/config.json").unlink()
        responses = [io.BytesIO(json.dumps({"preview": m["id"]}).encode()) for m in (self.first, self.second, self.first)]
        with patch.object(preview, "run", return_value=json.dumps(status)), \
             patch.object(preview.urllib.request, "urlopen", side_effect=responses):
            p.enable_services()
        saved = preview.read(self.home / "services/config.json")
        self.assertEqual(saved["tailnetDomain"], "example.ts.net")
        self.assertEqual(p.urls()["remote"], "https://genie-preview.example.ts.net/__previews")

    def test_untagged_node_cannot_be_selected_as_service_host(self):
        p, status = self.setup_enable()
        status["Self"]["Tags"] = None
        with patch.object(preview, "run", return_value=json.dumps(status)):
            with self.assertRaisesRegex(RuntimeError, "expected tag"):
                p.enable_services()

    def test_real_tracker_service_is_direct_and_does_not_change_demo_channels(self):
        p, status = self.setup_enable()
        with patch.object(preview, "run", side_effect=[json.dumps(status), "{}", "ok"]) as run:
            p.enable_tracker_service()
        self.assertEqual(run.call_args.args[0][-3:], ["--service=svc:genie-self", "--https=443", "http://127.0.0.1:7422"])
        saved = preview.read(self.home / "services/config.json")
        self.assertTrue(saved["trackerService"])
        self.assertEqual(saved["tailnetDomain"], "example.ts.net")
        self.assertNotIn("self", service_router.CHANNELS)

    def test_tracker_service_preserves_an_existing_other_destination(self):
        p, status = self.setup_enable()
        existing = {"Services": {"svc:genie-self": {"TCP": {"443": {"HTTPS": True}},
                    "Web": {"genie-self.example.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:9999"}}}}}}}
        with patch.object(preview, "run", side_effect=[json.dumps(status), json.dumps(existing)]) as run:
            with self.assertRaisesRegex(RuntimeError, "Preserving unrelated"):
                p.enable_tracker_service()
        self.assertEqual(run.call_count, 2)


if __name__ == "__main__":
    unittest.main()
