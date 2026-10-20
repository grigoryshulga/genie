#!/usr/bin/env python3
"""Build fixed integration previews with private, repository-local runtime state."""
import argparse
import contextlib
import fcntl
import hashlib
import html
import http.client
import http.server
import io
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import time
import urllib.request

import config as settings

HERE = Path(__file__).resolve().parent


def run(args, cwd=None, env=None, data=None):
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, input=data,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode:
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): " +
                           result.stderr.decode(errors="replace")[-6000:])
    return result.stdout.decode().strip()


def git(root, *args):
    return run(["git", "-C", root, *args])


def read(path, default=None):
    return json.loads(path.read_text()) if path.exists() else default


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + "." + secrets.token_hex(6))
    temp.write_text(json.dumps(value, indent=2) + "\n")
    os.chmod(temp, 0o600)
    os.replace(temp, path)


@contextlib.contextmanager
def lock(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def identity(root):
    root = Path(git(root, "rev-parse", "--show-toplevel"))
    return {"worktree": str(root), "sha": git(root, "rev-parse", "HEAD"),
            "branch": git(root, "symbolic-ref", "--short", "HEAD")}


def clean(root):
    return not git(root, "status", "--porcelain", "--untracked-files=all")


def valid(entry):
    try:
        return clean(entry["worktree"]) and identity(entry["worktree"]) == {
            k: entry[k] for k in ("worktree", "sha", "branch")}
    except (RuntimeError, FileNotFoundError):
        return False


def process_id(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()[19]
    except (FileNotFoundError, ProcessLookupError):
        return None


def alive(proc):
    return bool(proc and process_id(proc["pid"]) == proc["start"])


def spawn(args, log, cwd=None, env=None):
    with log.open("ab") as output:
        proc = subprocess.Popen([str(a) for a in args], cwd=cwd, env=env,
                                stdin=subprocess.DEVNULL, stdout=output, stderr=output,
                                start_new_session=True, close_fds=True)
    return {"pid": proc.pid, "start": process_id(proc.pid)}


class Preview:
    def __init__(self, root, home=None):
        self.root = Path(root).resolve()
        self.home = Path(home) if home is not None else settings.home_for_repo(root)
        self.home.mkdir(parents=True, exist_ok=True)
        self.config_path = self.home / "config.json"
        self.config = settings.load(self.home)
        self.ready_path = self.home / "ready.json"
        self.state_lock = self.home / "state.lock"

    def save_network(self):
        # Preserve legacy fields while previously installed gateways still read this file.
        network = read(self.config_path, {})
        network.update({k: v for k, v in self.config.items() if k in settings.NETWORK_KEYS})
        write(self.config_path, network)

    def base(self):
        return git(self.root, "rev-parse", "--verify", self.config["baseRef"] + "^{commit}")

    def urls(self, port=None):
        remote = (f"https://{self.config['tailscaleHost']}:{port or self.config['httpsPort']}"
                  if self.config["tailscale"] else None)
        if not port and self.config.get("serviceUrls", {}).get("preview"):
            remote = self.config["serviceUrls"]["preview"] + "/__previews"
        return {"local": f"http://genie.localhost:{port or self.config['indexPort']}", "remote": remote}

    def prepare(self, root):
        common = Path(git(root, "rev-parse", "--path-format=absolute", "--git-common-dir"))
        if common != Path(git(self.root, "rev-parse", "--path-format=absolute", "--git-common-dir")):
            raise RuntimeError("Worktree belongs to another repository")
        with lock(self.state_lock):
            exclude = common / "info/exclude"
            existing = exclude.read_text() if exclude.exists() else ""
            if "/.genie-preview" not in existing.splitlines():
                exclude.parent.mkdir(exist_ok=True)
                with exclude.open("a") as f:
                    f.write("\n# Machine-local preview tools shared by worktrees\n/.genie-preview\n")
            link = Path(root) / ".genie-preview"
            if not link.exists() and not link.is_symlink():
                link.symlink_to(self.home, target_is_directory=True)
            elif link.resolve() != self.home.resolve():
                raise RuntimeError(f"Preserving unrelated path: {link}")
        # npm's download cache is shared; installed packages stay worktree-local.
        dep_key = digest([(name, (Path(root) / name).read_text()) for name in
                          ("package.json", "package-lock.json", "web/package.json")])
        stamp = Path(root) / "node_modules/.genie-preview-deps"
        with lock(self.home / ("prepare-" + digest(str(root))[:16] + ".lock")):
            if not stamp.exists() or stamp.read_text() != dep_key:
                run(["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=root)
                stamp.write_text(dep_key)
        self.ensure_index()
        print(json.dumps(self.urls(), indent=2))

    def ready(self, root, task):
        if not re.fullmatch(r"GNE-[1-9][0-9]*", task):
            raise RuntimeError("Expected an explicit GNE task ID")
        with lock(self.state_lock):
            if not clean(root):
                raise RuntimeError("Commit all task changes before marking ready")
            entry = identity(root)
            if entry["branch"] in ("main", "dev"):
                raise RuntimeError("Mark readiness from an isolated task branch")
            entry.update(task=task, markedAt=time.time())
            entries = read(self.ready_path, {})
            entries[str(Path(root).resolve())] = entry
            if not valid(entry):
                raise RuntimeError("Worktree changed while marking readiness")
            write(self.ready_path, entries)
        print(json.dumps(entry, indent=2))

    def withdraw(self, root):
        with lock(self.state_lock):
            entries = read(self.ready_path, {})
            entries.pop(str(Path(root).resolve()), None)
            write(self.ready_path, entries)

    def selection(self):
        entries = list(read(self.ready_path, {}).values())
        stale = [e["task"] for e in entries if not valid(e)]
        # Fail the entire declared composition instead of silently dropping stale tasks.
        if stale:
            raise RuntimeError("Stale readiness; re-mark or withdraw: " + ", ".join(stale))
        tasks = [e["task"] for e in entries]
        if len(set(tasks)) != len(tasks):
            raise RuntimeError("Multiple worktrees declare the same task")
        return sorted(entries, key=lambda e: e["task"])

    def request(self, root):
        with lock(self.state_lock):
            entry = read(self.ready_path, {}).get(str(Path(root).resolve()))
            if not entry or not valid(entry):
                print("No current explicit readiness; no preview requested")
                return False
            self.selection()
        self.ensure_index()
        spawn([sys.executable, HERE / "preview.py", "--home", self.home, "--root", self.root, "worker"],
              self.home / "worker.log", cwd=self.root)
        print("Preview queued: " + (self.urls()["remote"] or self.urls()["local"]))
        return True

    def ensure_index(self):
        with lock(self.home / "index.lock"):
            pid_path = self.home / "index-process.json"
            if not alive(read(pid_path)):
                proc = spawn([sys.executable, HERE / "preview.py", "--home", self.home, "--root", self.root, "serve"],
                             self.home / "index.log", cwd=self.root)
                write(pid_path, proc)
            self.wait_health(self.config["indexPort"], "index", pid_path=pid_path)

    @staticmethod
    def wait_health(port, expected=None, pid_path=None):
        end = time.monotonic() + 30
        while time.monotonic() < end:
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/health", timeout=2) as r:
                    data = r.read().decode()
                if expected and expected not in data:
                    raise RuntimeError("Port is occupied by an unrelated service")
                if pid_path and not alive(read(pid_path)):
                    raise RuntimeError("Preview index exited; inspect index.log")
                return
            except (OSError, TimeoutError):
                time.sleep(.2)
        raise RuntimeError(f"Health check timed out on port {port}")

    def publish_tailscale(self, port, target):
        status = json.loads(run(["tailscale", "serve", "status", "--json"]))
        endpoint = f"{self.config['tailscaleHost']}:{port}"
        existing = status.get("Web", {}).get(endpoint, {}).get("Handlers", {})
        wanted = f"http://127.0.0.1:{target}"
        if existing:
            if existing != {"/": {"Proxy": wanted}}:
                raise RuntimeError(f"Preserving unrelated Tailscale handler on {endpoint}")
            return
        if str(port) in status.get("TCP", {}):
            raise RuntimeError(f"Preserving unrelated Tailscale TCP listener on {port}")
        run(["tailscale", "serve", "--bg", f"--https={port}", wanted])

    def enable_tailscale(self):
        status = json.loads(run(["tailscale", "status", "--json"]))
        host = status.get("Self", {}).get("DNSName", "").rstrip(".")
        if status.get("BackendState") != "Running" or not host:
            raise RuntimeError("Tailscale must be running with a MagicDNS name")
        self.config.update(tailscaleHost=host)
        self.publish_tailscale(self.config["httpsPort"], self.config["indexPort"])
        self.config["tailscale"] = True
        self.save_network()
        print(json.dumps(self.urls(), indent=2))

    def enable_services(self):
        from service_router import urls
        config = settings.service_config(self.home)
        if not config.get("socket"):
            raise RuntimeError("Install the separate Genie Services host first")
        status = json.loads(run(["tailscale", "--socket=" + config["socket"], "status", "--json"]))
        if status.get("BackendState") != "Running" or config["tag"] not in (status.get("Self", {}).get("Tags") or []):
            raise RuntimeError("The dedicated service host must be logged in with its expected tag")
        domain = status["Self"]["DNSName"].rstrip(".").split(".", 1)[1]
        config["tailnetDomain"] = domain
        # The router needs the discovered domain before probing the new named endpoints.
        # The index adopts those endpoints only after every health check succeeds.
        write(self.home / "services/config.json", config)
        endpoints = urls(config)
        for channel, url in endpoints.items():
            manifest = self.candidate_path(read(self.home / "current.json")["id"]) if channel == "preview" else \
                       self.candidate_path(read(self.home / "channels.json")[channel])
            expected = read(manifest / "manifest.json")["id"]
            with urllib.request.urlopen(url + "/api/health", timeout=15) as response:
                health = json.load(response)
            if health.get("preview") != expected:
                raise RuntimeError("Service does not select the expected candidate: " + channel)
        self.config["serviceUrls"] = endpoints
        self.save_network()
        print(json.dumps({"services": endpoints, "index": self.urls()["remote"]}, indent=2))

    def manifests(self):
        return [read(p) for p in sorted((self.home / "candidates").glob("*/manifest.json"))]

    def build(self, channel=None):
        # Every request gets a worker; blocking flock means a request arriving during exit is retained.
        with lock(self.home / "build.lock"):
            with lock(self.state_lock):
                entries = [] if channel else self.selection()
                if not entries and not channel:
                    return None
                base = git(self.root, "rev-parse", "--verify", f"origin/{channel}^{{commit}}") if channel else self.base()
            recipe = {"version": 1, "base": base,
                      "tasks": [{k: e[k] for k in ("task", "sha")} for e in entries]}
            if channel:
                recipe["channel"] = channel
            cid = digest(recipe)[:20]
            directory = self.home / "candidates" / cid
            manifest_path = directory / "manifest.json"
            previous = read(manifest_path)
            if previous and previous.get("status") == "ready":
                if alive(previous.get("backend")) and alive(previous.get("gateway")):
                    if channel:
                        self.set_channel(channel, cid)
                    print("Reusing candidate " + cid)
                    return previous
                raise RuntimeError("Candidate stopped; use restart to restore the same artifacts")
            directory.mkdir(parents=True, exist_ok=True)
            manifest = dict(recipe, id=cid, status="building", createdAt=time.time())
            write(manifest_path, manifest)
            started = time.monotonic()
            try:
                source = directory / "source"
                if source.exists():
                    raise RuntimeError("Failed candidate retained for diagnosis; withdraw/fix task or remove its failed worktree explicitly")
                git(self.root, "worktree", "add", "--detach", str(source), base)
                for e in entries:
                    try:
                        git(source, "-c", "user.name=Genie Preview", "-c", "user.email=preview@localhost",
                            "merge", "--no-ff", "--no-edit", e["sha"])
                    except RuntimeError:
                        git(source, "merge", "--abort")
                        raise RuntimeError("Integration conflict while merging " + e["task"])
                manifest["integratedSha"] = git(source, "rev-parse", "HEAD")
                manifest["tree"] = git(source, "rev-parse", "HEAD^{tree}")
                self.artifacts(source, directory, manifest)
                with lock(self.state_lock):
                    if not channel and (self.selection() != entries or self.base() != base):
                        raise RuntimeError("Composition changed during build; request a fresh candidate")
                manifest["status"] = "starting"
                manifest["seconds"] = round(time.monotonic() - started, 2)
                write(manifest_path, manifest)
                self.start_candidate(directory, manifest)
                manifest["status"] = "ready"
                write(manifest_path, manifest)
                if channel:
                    self.set_channel(channel, cid)
                else:
                    write(self.home / "current.json", {"id": cid})
                print(json.dumps(manifest, indent=2))
                return manifest
            except Exception as exc:
                for name in ("gateway", "backend"):
                    self.stop_process(manifest.get(name))
                manifest.update(status="failed", error=str(exc))
                write(manifest_path, manifest)
                raise

    def set_channel(self, channel, cid):
        channels = read(self.home / "channels.json", {})
        channels[channel] = cid
        write(self.home / "channels.json", channels)

    def artifacts(self, source, directory, manifest):
        backend_paths = ["Cargo.toml", "Cargo.lock", ".cargo", "crates", "agents", "config"]
        key = digest([git(source, "ls-tree", "-r", "HEAD", "--", *backend_paths), run(["rustc", "-vV"])])
        cache = self.home / "cache"
        binary = cache / "binaries" / key / "genie"
        if not binary.exists():
            build_source = cache / "backend-source"
            if build_source.exists():
                shutil.rmtree(build_source)
            build_source.mkdir(parents=True)
            archive = subprocess.check_output(["git", "-C", str(source), "archive", "HEAD", *backend_paths])
            with tarfile.open(fileobj=io.BytesIO(archive)) as tf:
                tf.extractall(build_source, filter="data")
            env = dict(os.environ, CARGO_TARGET_DIR=str(cache / "cargo-target"), CARGO_BUILD_JOBS=str(self.config["buildJobs"]))
            env.pop("GENIE_WEB_DIST", None)
            run(["cargo", "build", "--locked", "-p", "genie"], cwd=build_source, env=env)
            binary.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(cache / "cargo-target/debug/genie", binary)
        shutil.copy2(binary, directory / "genie")
        manifest["backendKey"] = key
        # No shared writable node_modules: each candidate has its own exact lockfile installation.
        run(["npm", "ci", "--workspace", "genie-web", "--include-workspace-root=false",
             "--ignore-scripts", "--no-audit", "--no-fund"], cwd=source)
        run(["npm", "run", "build:web"], cwd=source)
        shutil.copytree(source / "web/dist", directory / "web")

    def next_port(self):
        used = {p for m in self.manifests() for p in (m.get("port"), m.get("backendPort", m.get("port", 0) + 100))}
        ts = json.loads(run(["tailscale", "serve", "status", "--json"])) if self.config["tailscale"] else {}
        for port in range(self.config["candidatePortStart"], self.config["candidatePortEnd"] + 1):
            if port in used or port + self.config["backendPortOffset"] in used or str(port) in ts.get("TCP", {}):
                continue
            try:
                with socket.socket() as a, socket.socket() as b:
                    a.bind(("127.0.0.1", port))
                    b.bind(("127.0.0.1", port + self.config["backendPortOffset"]))
                return port
            except OSError:
                continue
        raise RuntimeError("No free candidate ports; stop and retire old previews")

    @staticmethod
    def runtime_env(directory):
        # A preview never inherits tracker credentials, model keys or integration configuration.
        env = {k: os.environ[k] for k in ("PATH", "LANG", "TZ") if k in os.environ}
        home = directory / "home"
        home.mkdir(exist_ok=True)
        env["HOME"] = str(home)
        return env

    def start_candidate(self, directory, manifest):
        port = manifest["port"] if "port" in manifest else self.next_port()
        manifest["port"] = port
        # Retain old candidates' original backend ports when local defaults change.
        manifest.setdefault("backendPort", port + (100 if "urls" in manifest else self.config["backendPortOffset"]))
        manifest["urls"] = self.urls(port)
        data = directory / "data"
        data.mkdir(exist_ok=True)
        config = {"bind": "127.0.0.1", "port": manifest["backendPort"], "runtime": {"enabled": False},
                  "publicUrl": manifest["urls"]["remote"] or manifest["urls"]["local"]}
        write(data / "config.json", config)
        env = self.runtime_env(directory)
        binary = directory / "genie"
        if not (data / "server.db").exists():
            password = secrets.token_urlsafe(24)
            credential = directory / "credentials.txt"
            credential.write_text("Login: preview\nPassword: " + password + "\n")
            os.chmod(credential, 0o600)
            run([binary, "--data", data, "user", "add", "preview", "--admin", "--password-stdin"],
                cwd=directory, env=env, data=(password + "\n").encode())
            run([binary, "--data", data, "project", "add", "preview", "--name", "Integration preview", "--prefix", "PV"],
                cwd=directory, env=env)
            run([binary, "--data", data, "--project", "preview", "project", "update", "preview", "--autonomy", "manual"],
                cwd=directory, env=env)
            for task in manifest["tasks"]:
                run([binary, "--data", data, "--project", "preview", "task", "create", "Review " + task["task"],
                     "--description", "Included source commit: " + task["sha"]], cwd=directory, env=env)
        manifest["backend"] = spawn([binary, "--data", data, "serve", "--no-agents", "--web", directory / "web"],
                                    directory / "server.log", cwd=directory, env=env)
        self.wait_health(manifest["backendPort"])
        write(directory / "manifest.json", manifest)
        manifest["gateway"] = spawn([sys.executable, HERE / "preview.py", "--home", self.home, "--root", self.root,
                                     "gateway", manifest["id"]], directory / "gateway.log", cwd=self.root)
        self.wait_health(port, manifest["id"])
        if self.config["tailscale"]:
            self.publish_tailscale(port, port)

    @staticmethod
    def stop_process(proc):
        if alive(proc):
            os.kill(proc["pid"], signal.SIGINT)

    def stop(self, cid):
        with lock(self.home / "build.lock"):
            path = self.candidate_path(cid)
            manifest = read(path / "manifest.json")
            for name in ("gateway", "backend"):
                self.stop_process(manifest.get(name))
            manifest["status"] = "stopped"
            write(path / "manifest.json", manifest)
            # Keep the port and Tailscale mapping reserved; never repoint old candidate links.

    def restart(self, cid):
        with lock(self.home / "build.lock"):
            path = self.candidate_path(cid)
            manifest = read(path / "manifest.json")
            if manifest["status"] not in ("ready", "stopped"):
                raise RuntimeError("Only previously ready artifacts can be restarted")
            for name in ("gateway", "backend"):
                self.stop_process(manifest.get(name))
            time.sleep(.5)
            self.start_candidate(path, manifest)
            manifest["status"] = "ready"
            write(path / "manifest.json", manifest)

    def candidate_path(self, cid):
        if not re.fullmatch(r"[0-9a-f]{20}", cid):
            raise RuntimeError("Invalid candidate ID")
        path = self.home / "candidates" / cid
        if not (path / "manifest.json").exists():
            raise RuntimeError("Unknown candidate")
        return path

    def serve(self, candidate=None):
        preview = self
        manifest = read(self.candidate_path(candidate) / "manifest.json") if candidate else None
        port = manifest["port"] if manifest else self.config["indexPort"]

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, body, content_type="text/html; charset=utf-8", status=200):
                raw = body.encode()
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                preview.config = settings.load(preview.home)
                if not self.valid_host():
                    return self.reply("Unexpected Host", status=421)
                if self.path == "/api/health":
                    return self.reply(json.dumps({"preview": candidate or "index"}), "application/json")
                if manifest:
                    return self.proxy()
                if self.path == "/api/status":
                    return self.reply(json.dumps({"current": read(preview.home / "current.json"),
                                                  "candidates": preview.manifests(),
                                                  "readiness": read(preview.ready_path, {})}), "application/json")
                if self.path != "/":
                    return self.reply("Not found", status=404)
                rows = []
                remote = self.headers.get("Host", "").startswith(preview.config["tailscaleHost"] + ":") if preview.config["tailscaleHost"] else False
                current = read(preview.home / "current.json", {}).get("id")
                for m in sorted(preview.manifests(), key=lambda m: m["createdAt"], reverse=True):
                    state = m["status"]
                    if state == "ready" and not (alive(m.get("backend")) and alive(m.get("gateway"))):
                        state = "offline"
                    tasks = ", ".join(f"{t['task']} @ {t['sha'][:10]}" for t in m["tasks"])
                    url = m.get("urls", {}).get("remote" if remote else "local")
                    anchor = f'<a href="{html.escape(url)}">Open application</a>' if state == "ready" and url else ""
                    error = '<pre>' + html.escape(m.get("error", "")) + '</pre>' if m.get("error") else ""
                    rows.append(f'<section><h2>{m["id"]}{" · current" if m["id"] == current else ""}</h2>'
                                f'<p>{html.escape(state)} · base {m["base"][:12]} · {m.get("seconds", "—")} s</p>'
                                f'<p>{html.escape(tasks)}</p>{anchor}{error}</section>')
                self.reply('<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
                           '<meta http-equiv="refresh" content="5"><title>Genie previews</title><style>'
                           'body{font:16px system-ui;max-width:960px;margin:40px auto;padding:0 20px;background:#10131b;color:#e8eaf0}'
                           'a{color:#b3c9ff}section{border-top:1px solid #434b60;padding:16px 0}h2{font-size:18px}'
                           'pre,p{overflow-wrap:anywhere;white-space:pre-wrap}</style><h1>Genie previews</h1>'
                           '<p>Fixed source versions for owner review. Approval is recorded in the task conversation.</p>'
                           + (''.join(rows) or '<p>No candidates yet. An agent must explicitly mark a committed task ready.</p>') + '</html>')

            def valid_host(self):
                preview.config = settings.load(preview.home)
                host = self.headers.get("Host", "")
                allowed = {f"127.0.0.1:{port}", f"localhost:{port}", f"genie.localhost:{port}",
                           f"{preview.config['tailscaleHost']}:{port}",
                           f"{preview.config['tailscaleHost']}:{preview.config['httpsPort']}"}
                return host in allowed

            def do_POST(self):
                if not self.valid_host():
                    return self.reply("Unexpected Host", status=421)
                if not manifest:
                    return self.reply("Read-only preview index", status=405)
                return self.proxy()

            do_PUT = do_POST
            do_PATCH = do_POST
            do_DELETE = do_POST
            do_OPTIONS = do_POST

            def proxy(self):
                # Candidate-specific cookie names keep sessions separate on the shared tailnet hostname.
                prefix = "pv_" + candidate + "_"
                headers = {k: v for k, v in self.headers.items()
                           if k.lower() not in ("host", "connection", "cookie", "transfer-encoding")}
                cookies = []
                for part in self.headers.get("Cookie", "").split(";"):
                    key, sep, value = part.strip().partition("=")
                    if sep and key.startswith(prefix):
                        cookies.append(key[len(prefix):] + "=" + value)
                if cookies:
                    headers["Cookie"] = "; ".join(cookies)
                length = int(self.headers.get("Content-Length", "0"))
                body = self.rfile.read(length) if length else None
                conn = http.client.HTTPConnection("127.0.0.1", manifest.get("backendPort", port + 100), timeout=120)
                try:
                    conn.request(self.command, self.path, body=body, headers=headers)
                    response = conn.getresponse()
                    self.send_response(response.status)
                    for key, value in response.getheaders():
                        if key.lower() in ("transfer-encoding", "connection"):
                            continue
                        if key.lower() == "set-cookie":
                            value = prefix + value
                        self.send_header(key, value)
                    self.end_headers()
                    while chunk := response.read1(65536):
                        self.wfile.write(chunk)
                        self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
                finally:
                    conn.close()

        server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
        server.daemon_threads = True
        server.serve_forever()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=os.getcwd())
    parser.add_argument("--home", type=Path, help="Private runtime directory; defaults to the shared Git directory")
    subs = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "request", "serve", "withdraw", "status", "enable-tailscale", "enable-services"):
        subs.add_parser(name)
    worker = subs.add_parser("worker")
    worker.add_argument("--channel", choices=("main", "dev"))
    channel = subs.add_parser("channel")
    channel.add_argument("name", choices=("main", "dev"))
    ready = subs.add_parser("ready")
    ready.add_argument("--task", required=True)
    for name in ("gateway", "stop", "restart"):
        sub = subs.add_parser(name)
        sub.add_argument("candidate")
    args = parser.parse_args()
    common = Path(git(args.root, "rev-parse", "--path-format=absolute", "--git-common-dir"))
    root = Path(git(args.root, "rev-parse", "--show-toplevel"))
    preview = Preview(root, args.home or common / "genie-preview")
    if args.command in ("prepare", "request", "withdraw"):
        getattr(preview, args.command)(root)
    elif args.command == "ready":
        preview.ready(root, args.task)
    elif args.command == "worker":
        preview.build(args.channel)
    elif args.command == "channel":
        preview.ensure_index()
        spawn([sys.executable, HERE / "preview.py", "--home", self.home, "--root", root, "worker", "--channel", args.name],
              preview.home / "worker.log", cwd=root)
        print("Queued isolated baseline: origin/" + args.name)
    elif args.command == "enable-tailscale":
        preview.enable_tailscale()
    elif args.command == "enable-services":
        preview.enable_services()
    elif args.command == "serve":
        preview.serve()
    elif args.command in ("gateway", "stop", "restart"):
        getattr(preview, "serve" if args.command == "gateway" else args.command)(args.candidate)
    elif args.command == "status":
        print(json.dumps({"urls": preview.urls(), "readiness": read(preview.ready_path, {}),
                          "candidates": preview.manifests()}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as exc:
        print("genie-preview: " + str(exc), file=sys.stderr)
        sys.exit(1)
