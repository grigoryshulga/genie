import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import config
import install
import install_network
import preview


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="preview alternate $ % ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repo"
        self.root.mkdir()
        subprocess.run(["git", "init", "-q", self.root], check=True)
        self.home = config.home_for_repo(self.root)

    def test_fresh_install_runs_from_another_path_and_preserves_private_state(self):
        installer = Path(install.__file__)
        result = subprocess.run([sys.executable, installer, "--root", self.root], check=True, capture_output=True, text=True)
        self.assertEqual(json.loads(result.stdout)["home"], str(self.home))
        self.assertEqual(config.load(self.home)["indexPort"], 7480)
        private = self.home / "services/state/tailscaled.state"
        private.parent.mkdir(parents=True)
        private.write_text("private node state")
        private.chmod(0o600)
        before = (self.home / "code-current").resolve()
        subprocess.run([sys.executable, installer, "--root", self.root], check=True, capture_output=True)
        self.assertEqual(private.read_text(), "private node state")
        self.assertEqual(private.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.home / "code-current").resolve(), before)
        # The installed launcher works with arbitrary paths, without exporting environment variables.
        result = subprocess.run([sys.executable, self.home / "preview.py", "--root", self.root, "status"],
                                check=True, capture_output=True, text=True)
        self.assertEqual(json.loads(result.stdout)["candidates"], [])
        self.assertEqual(self.home.stat().st_mode & 0o777, 0o700)

    def test_empty_config_needs_no_machine_parameters_and_overrides_are_validated(self):
        self.assertEqual(config.load(self.home)["lan"], {"interface": "auto"})
        preview.write(self.home / "local.json", {"indexPort": 7500, "lan": {"interface": "eth-test"}})
        self.assertEqual(config.load(self.home)["indexPort"], 7500)
        preview.write(self.home / "local.json", {"candidatePortStart": 8500, "candidatePortEnd": 8600})
        with self.assertRaisesRegex(ValueError, "disjoint"):
            config.load(self.home)
        preview.write(self.home / "local.json", {"tailscaleHost": "copied.example.ts.net"})
        with self.assertRaisesRegex(ValueError, "Unknown"):
            config.load(self.home)

    def test_legacy_network_fields_survive_code_updates_for_running_gateways(self):
        network = {"indexPort": 7501, "httpsPort": 8443, "baseRef": "origin/dev", "tailscaleHost": "host.example.ts.net"}
        preview.write(self.home / "config.json", network)
        install.deploy(self.home)
        self.assertEqual(config.read_json(self.home / "config.json", {}), network)
        self.assertEqual(config.load(self.home)["indexPort"], 7501)
        p = preview.Preview(self.root, self.home)
        p.config["tailscale"] = True
        p.save_network()
        self.assertEqual(config.read_json(self.home / "config.json", {})["indexPort"], 7501)

    def test_auto_interface_uses_only_unambiguous_non_tunnel_default_route(self):
        address = [{"addr_info": [{"family": "inet", "scope": "global", "local": "192.168.8.9"}]}]
        with patch.object(config, "json_command", side_effect=[[{"dev": "eth-test"}], [{}], address]):
            self.assertEqual(config.lan_interface(config.DEFAULTS), "eth-test")
        with patch.object(config, "json_command", return_value=[{"dev": "one"}, {"dev": "two"}]):
            with self.assertRaisesRegex(RuntimeError, "set lan.interface"):
                config.lan_interface(config.DEFAULTS)
        with patch.object(config, "json_command", side_effect=[[{"dev": "wg0"}], [{"linkinfo": {"info_kind": "wireguard"}}]]):
            with self.assertRaisesRegex(RuntimeError, "tunnel"):
                config.lan_interface(config.DEFAULTS)
        with patch.object(config, "json_command", return_value=[{"addr_info": [
                {"family": "inet", "scope": "global", "local": "203.0.113.20"},
                {"family": "inet", "scope": "global", "local": "192.168.8.9"}]}]):
            with self.assertRaisesRegex(RuntimeError, "exactly one"):
                config.lan_address("eth-test")

    def test_runtime_socket_follows_the_current_user_not_copied_state(self):
        preview.write(self.home / "services/config.json", {"socket": "/run/user/9999/old.sock"})
        with patch.dict(os.environ, {"XDG_RUNTIME_DIR": "/run/user/4321"}):
            self.assertEqual(config.service_config(self.home)["socket"], "/run/user/4321/genie-services/tailscaled.sock")

    def test_generated_units_derive_identity_paths_and_addresses(self):
        with patch.object(config, "lan_interface", return_value="eth-test"), \
             patch.object(config, "lan_address", return_value="192.168.8.9"), \
             patch.object(install.shutil, "which", return_value="/usr/bin/tailscale"), \
             patch.object(config, "json_command", return_value={"BackendState": "Running", "Self": {"TailscaleIPs": ["100.80.90.10", "fd7a:115c:a1e0::1"]}}):
            units = install.system_units(self.home, config.DEFAULTS)
        self.assertIn("ListenStream=192.168.8.9:80", units["genie-local-proxy.socket"])
        self.assertIn("ListenStream=100.80.90.10:80", units["genie-local-proxy.socket"])
        self.assertNotIn("fd7a", units["genie-local-proxy.socket"])
        self.assertIn("$$", units["genie-local-proxy.service"])
        self.assertIn("%%", units["genie-local-proxy.service"])
        self.assertIn("WorkingDirectory=" + str(self.home).replace("%", "%%") + "\n", units["genie-local-proxy.service"])
        self.assertNotIn('WorkingDirectory="', units["genie-local-proxy.service"])
        self.assertIn(str(os.getuid()), config.runtime_socket())

    def test_network_installer_preflights_unrelated_units_and_preserves_resolver(self):
        source = Path(self.temp.name) / "units"
        system = Path(self.temp.name) / "etc/systemd/system"
        source.mkdir()
        system.mkdir(parents=True)
        for name in install_network.NAMES:
            (source / name).write_text(install_network.MARKER + "[Unit]\nDescription=Genie test\n")
        unrelated = system / "genie-local-proxy.socket"
        unrelated.write_text("[Unit]\nDescription=Another application\n")
        etc = system.parent.parent
        resolver = etc / "nsswitch.conf"
        original = "hosts: mymachines mdns_minimal [NOTFOUND=return] resolve files dns # retained\n"
        resolver.write_text(original)
        with self.assertRaisesRegex(RuntimeError, "unrelated"):
            install_network.install(source, etc, mdns=True)
        self.assertEqual(resolver.read_text(), original)
        unrelated.unlink()
        install_network.install(source, etc, mdns=True)
        changed = resolver.read_text()
        self.assertIn("hosts: files mymachines mdns4 mdns_minimal [NOTFOUND=return] resolve dns # retained", changed)
        self.assertEqual((etc / "nsswitch.conf.genie-preview.bak").read_text(), original)
        install_network.install(source, etc, mdns=True)
        self.assertEqual(resolver.read_text(), changed)
        self.assertEqual((etc / "mdns.allow").read_text().count("# Genie multi-label"), 1)

    def test_user_installer_preserves_unrelated_same_name_unit(self):
        destination = Path(self.temp.name) / "config/systemd/user"
        destination.mkdir(parents=True)
        path = destination / "genie-mdns.service"
        path.write_text("other application")
        with patch.dict(os.environ, {"XDG_CONFIG_HOME": str(destination.parent.parent)}):
            with self.assertRaisesRegex(RuntimeError, "unrelated"):
                install.install_user_units(self.home, {path.name: "replacement"})
        self.assertEqual(path.read_text(), "other application")


if __name__ == "__main__":
    unittest.main()
