"""Local settings and reliable discovery for Linux preview hosts."""
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess

DEFAULTS = {
    "baseRef": "origin/dev", "indexPort": 7480, "httpsPort": 8443,
    "candidatePortStart": 8500, "candidatePortEnd": 8599, "backendPortOffset": 100,
    "buildJobs": 4, "lan": {"interface": "auto"},
}
NETWORK_KEYS = {"tailscale", "tailscaleHost", "serviceUrls"}


def json_command(args):
    result = subprocess.run(args, capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def home_for_repo(root=None):
    result = subprocess.run(["git", "-C", str(root or Path.cwd()), "rev-parse",
                             "--path-format=absolute", "--git-common-dir"],
                            capture_output=True, text=True, check=True)
    return Path(result.stdout.strip()) / "genie-preview"


def read_json(path, default):
    return json.loads(path.read_text()) if path.exists() else default


def load(home):
    settings = read_json(Path(home) / "local.json", {})
    if not isinstance(settings, dict) or settings.keys() - DEFAULTS.keys():
        raise ValueError("Unknown preview settings; see scripts/preview/config.example.json and the guide")
    if not isinstance(settings.get("lan", {}), dict):
        raise ValueError("lan must be an object")
    config = dict(DEFAULTS, **settings)
    config["lan"] = dict(DEFAULTS["lan"], **settings.get("lan", {}))
    if config["lan"].keys() != {"interface"}:
        raise ValueError("Only lan.interface is configurable")
    interface = config["lan"]["interface"]
    if not isinstance(interface, str) or not re.fullmatch(r"[A-Za-z0-9_.:-]+", interface):
        raise ValueError("lan.interface must be auto or a network interface name")
    if not isinstance(config["baseRef"], str) or not config["baseRef"] or config["baseRef"].startswith("-"):
        raise ValueError("baseRef must be a Git ref")
    for key in ("indexPort", "httpsPort", "candidatePortStart", "candidatePortEnd", "backendPortOffset", "buildJobs"):
        if type(config[key]) is not int or config[key] <= 0:
            raise ValueError(key + " must be a positive integer")
    start, end, offset = (config[k] for k in ("candidatePortStart", "candidatePortEnd", "backendPortOffset"))
    if start > end or end + offset > 65535 or end >= start + offset:
        raise ValueError("Candidate and backend port ranges must be valid and disjoint")
    if max(config["indexPort"], config["httpsPort"]) > 65535:
        raise ValueError("Index ports must be valid TCP ports")
    if any(start <= config[k] <= end + offset for k in ("indexPort", "httpsPort")):
        raise ValueError("Index ports must be outside the candidate and backend port ranges")
    network = read_json(Path(home) / "config.json", {})
    if not isinstance(network, dict):
        raise ValueError("Invalid discovered network state in config.json")
    config.update(tailscale=False, tailscaleHost="")
    config.update({k: v for k, v in network.items() if k in NETWORK_KEYS})
    return config


def runtime_socket():
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    return str(runtime / "genie-services/tailscaled.sock")


def service_config(home):
    discovered = read_json(Path(home) / "services/config.json", {})
    # Socket location follows this user's runtime directory, never a copied UID.
    return dict(discovered, socket=runtime_socket(), nodeName="genie-service-host", tag="tag:genie-services")


def lan_interface(settings):
    explicit = settings["lan"]["interface"]
    if explicit != "auto":
        return explicit
    routes = json_command(["ip", "-json", "-4", "route", "show", "default"])
    devices = {route["dev"] for route in routes if route.get("dev")}
    if len(devices) != 1:
        raise RuntimeError("Cannot choose one LAN interface reliably; set lan.interface in local.json")
    interface = devices.pop()
    links = json_command(["ip", "-json", "-details", "link", "show", "dev", interface])
    kind = links[0].get("linkinfo", {}).get("info_kind", "") if links else ""
    if kind in {"wireguard", "tun", "gre", "ipip", "sit", "veth", "bridge"}:
        raise RuntimeError("Default route uses a virtual/tunnel interface; set lan.interface in local.json")
    lan_address(interface)
    return interface


def lan_address(interface):
    interfaces = json_command(["ip", "-json", "-4", "address", "show", "dev", interface])
    addresses = {addr["local"] for item in interfaces for addr in item.get("addr_info", [])
                 if addr.get("family") == "inet" and addr.get("scope") == "global"}
    if len(addresses) != 1:
        raise RuntimeError("Expected exactly one global IPv4 address on " + interface)
    address = addresses.pop()
    if not ipaddress.ip_address(address).is_private:
        raise RuntimeError("Refusing automatic LAN publication on a public IPv4 address")
    return address
