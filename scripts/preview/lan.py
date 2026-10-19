#!/usr/bin/env python3
"""Local mDNS names and host routing; no application state is shared between channels."""
import argparse
import html
import http.client
import http.server
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import threading
import time

import config as settings
from preview import alive, read


def candidates(home=None):
    home = Path(home) if home is not None else settings.home_for_repo()
    return [read(p) for p in (home / "candidates").glob("*/manifest.json")]


def resolve(host, home=None):
    home = Path(home) if home is not None else settings.home_for_repo()
    host = host.partition(":")[0].lower()
    if host in ("genie.local", "dev.genie.local"):
        cid = read(home / "channels.json", {}).get("main" if host == "genie.local" else "dev")
    elif host == "preview.genie.local":
        cid = read(home / "current.json", {}).get("id")
    else:
        match = re.fullmatch(r"([a-f0-9]{20})\.preview\.genie\.local", host)
        if not match:
            raise ValueError("Unexpected Host")
        cid = match[1]
    if not cid:
        raise RuntimeError("This environment has not been built yet")
    m = read(home / "candidates" / cid / "manifest.json")
    if not m or m["status"] != "ready" or not alive(m.get("gateway")) or not alive(m.get("backend")):
        raise RuntimeError("Environment is not running; restart its fixed candidate")
    return m


def dashboard(home=None):
    rows = []
    for m in sorted(candidates(home), key=lambda m: m["createdAt"], reverse=True):
        cid = m["id"]
        tasks = ", ".join(t["task"] + " @ " + t["sha"][:10] for t in m["tasks"])
        link = f'<a href="http://{cid}.preview.genie.local/">Open fixed candidate</a>' if m["status"] == "ready" else ""
        rows.append(f'<section><h2>{cid}</h2><p>{html.escape(m.get("channel", "preview"))} · '
                    f'{html.escape(m["status"])} · base {m["base"][:12]}</p><p>{html.escape(tasks)}</p>{link}</section>')
    return ('<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
            '<title>Genie environments</title><style>body{font:16px system-ui;max-width:960px;margin:40px auto;'
            'padding:0 20px;background:#10131b;color:#e8eaf0}a{color:#b3c9ff}section{border-top:1px solid #434b60;'
            'padding:16px 0}p{overflow-wrap:anywhere}</style><h1>Genie environments</h1><p>'
            '<a href="http://genie.local">Main</a> · <a href="http://dev.genie.local">Dev</a> · '
            '<a href="http://preview.genie.local">Latest preview</a></p>' + ''.join(rows))


class Handler(http.server.BaseHTTPRequestHandler):
    preview_home = None
    def log_message(self, *_):
        pass

    def error(self, text, status):
        body = text.encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        host = self.headers.get("Host", "").partition(":")[0].lower()
        if host == "preview.genie.local" and self.path.rstrip("/") == "/__previews":
            body = dashboard(self.preview_home).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        try:
            manifest = resolve(host, self.preview_home)
        except ValueError as exc:
            return self.error(str(exc), 421)
        except RuntimeError as exc:
            return self.error(str(exc), 503)
        # Selected destination comes exclusively from a known manifest, never request headers/URLs.
        port = manifest["port"]
        headers = {k: v for k, v in self.headers.items()
                   if k.lower() not in ("host", "connection", "transfer-encoding")}
        headers["Host"] = f"127.0.0.1:{port}"
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=120)
        try:
            conn.request(self.command, self.path, body=body or None, headers=headers)
            response = conn.getresponse()
            self.send_response(response.status)
            for key, value in response.getheaders():
                if key.lower() not in ("connection", "transfer-encoding"):
                    self.send_header(key, value)
            self.end_headers()
            while chunk := response.read1(65536):
                self.wfile.write(chunk)
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            conn.close()

    do_POST = do_GET
    do_PUT = do_GET
    do_PATCH = do_GET
    do_DELETE = do_GET
    do_OPTIONS = do_GET


def serve(home):
    Handler.preview_home = home
    count = int(os.environ.get("LISTEN_FDS", "0"))
    if int(os.environ.get("LISTEN_PID", "0")) != os.getpid() or not count:
        raise RuntimeError("Start this process through genie-local-proxy.socket")
    servers = []
    for fd in range(3, 3 + count):
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler, bind_and_activate=False)
        server.socket.close()
        server.socket = socket.socket(fileno=fd)
        server.server_address = server.socket.getsockname()
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        servers.append(server)
    signal.pause()


def mdns(home, interface):
    publishers = {}
    previous_address = None
    def terminate(*_):
        for child in publishers.values():
            child.terminate()
        for child in publishers.values():
            child.wait(timeout=5)
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    while True:
        try:
            address = settings.lan_address(interface)
        except RuntimeError:
            address = None
        names = {"genie.local", "dev.genie.local", "preview.genie.local"}
        names.update(m["id"] + ".preview.genie.local" for m in candidates(home) if m["status"] == "ready")
        for name, child in list(publishers.items()):
            if name not in names or address != previous_address or child.poll() is not None:
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=5)
                del publishers[name]
        if address:
            for name in sorted(names - publishers.keys()):
                publishers[name] = subprocess.Popen(["avahi-publish", "-a", "-R", name, address])
        previous_address = address
        time.sleep(5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("serve", "mdns"))
    parser.add_argument("--home", type=Path)
    parser.add_argument("--interface", help="Override lan.interface for this invocation")
    args = parser.parse_args()
    home = args.home or settings.home_for_repo()
    if args.command == "serve":
        serve(home)
    else:
        mdns(home, args.interface or settings.lan_interface(settings.load(home)))
