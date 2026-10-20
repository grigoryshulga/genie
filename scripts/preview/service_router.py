#!/usr/bin/env python3
"""Loopback channel gateways for named Tailscale Services."""
import argparse
import html
from pathlib import Path
import http.server
import json
import signal
import threading

import lan
import config as settings
from preview import alive, read

CHANNELS = {
    "main": {"name": "genie", "host": "genie.local", "port": 7491},
    "dev": {"name": "genie-dev", "host": "dev.genie.local", "port": 7492},
    "preview": {"name": "genie-preview", "host": "preview.genie.local", "port": 7493},
}


def urls(config):
    domain = config.get("tailnetDomain", "")
    if not domain:
        return {}
    return {channel: f"https://{entry['name']}.{domain}" for channel, entry in CHANNELS.items()}


def make_handler(channel, home=None):
    home = Path(home) if home is not None else settings.home_for_repo()
    entry = CHANNELS[channel]

    class Handler(lan.Handler):
        preview_home = home
        def do_GET(self):
            config = settings.service_config(home)
            named = urls(config)
            host = self.headers.get("Host", "").lower().rstrip(".")
            service_host = entry["name"] + "." + config.get("tailnetDomain", "")
            allowed = {f"127.0.0.1:{entry['port']}", f"localhost:{entry['port']}",
                       service_host, service_host + ":443"}
            if host not in allowed:
                return self.error("Unexpected service Host", 421)
            if channel == "preview" and self.path.rstrip("/") == "/__previews":
                rows = []
                for manifest in sorted(lan.candidates(home), key=lambda m: m["createdAt"], reverse=True):
                    url = manifest.get("urls", {}).get("remote")
                    ready = manifest["status"] == "ready" and alive(manifest.get("gateway")) and alive(manifest.get("backend"))
                    state = "offline" if manifest["status"] == "ready" and not ready else manifest["status"]
                    link = f'<a href="{html.escape(url)}">Open fixed candidate</a>' if ready and url else ""
                    tasks = ", ".join(t["task"] + " @ " + t["sha"][:10] for t in manifest["tasks"])
                    rows.append(f'<section><h2>{manifest["id"]}</h2><p>{html.escape(manifest.get("channel", "preview"))}'
                                f' · {html.escape(state)} · base {manifest["base"][:12]}</p>'
                                f'<p>{html.escape(tasks)}</p>{link}</section>')
                nav = " · ".join(f'<a href="{url}">{html.escape(name)}</a>' for name, url in named.items())
                page = ('<!doctype html><html lang="en"><meta charset="utf-8">'
                        '<meta name="viewport" content="width=device-width,initial-scale=1">'
                        '<meta http-equiv="refresh" content="5"><title>Genie previews</title><style>body{font:16px system-ui;max-width:960px;margin:40px auto;'
                        'padding:0 20px;background:#10131b;color:#e8eaf0}a{color:#b3c9ff}section{border-top:1px solid #434b60;'
                        'padding:16px 0}p{overflow-wrap:anywhere}</style><h1>Genie previews</h1><nav>' + nav + '</nav>'
                        '<p>Channel names advance. Use a fixed candidate link for approval.</p>' + ''.join(rows) + '</html>')
                body = page.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if not self.headers.get("Host"):
                return self.error("Missing Host", 421)
            self.headers.replace_header("Host", entry["host"])
            super().do_GET()

        do_POST = do_GET
        do_PUT = do_GET
        do_PATCH = do_GET
        do_DELETE = do_GET
        do_OPTIONS = do_GET

    return Handler


def serve(home):
    servers = []
    for channel, entry in CHANNELS.items():
        server = http.server.ThreadingHTTPServer(("127.0.0.1", entry["port"]), make_handler(channel, home))
        server.daemon_threads = True
        servers.append(server)
    for server in servers:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    signal.pause()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", type=Path)
    args = parser.parse_args()
    serve(args.home or settings.home_for_repo())
