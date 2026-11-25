#!/usr/bin/env python3
"""Run against a locally built .app with a loopback Hub and synthetic data.

The test edits a private-memory fixture and captures native tray HTTP traffic.
Replay the corresponding host-memory actions separately in the Agent host.

From the repository root:
    python3 skills/a2w/tests/test_profile_isolation.py "/path/to/MilkBird Tray.app"
"""

import argparse
import json
import os
import plistlib
import queue
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


MEMBER = {"id": "profile-test", "email": "profile@example.test", "displayName": "测试成员"}
TOKEN = "synthetic-profile-test-token"
PROFILE = "/api/v1/profile"
SYNC = PROFILE + "/local"
PRIVATE_MEMORY = "synthetic private reason for preferring the east gate"
CLI = Path(__file__).resolve().parents[1] / "scripts" / "a2w_tray.py"


def check(condition, message):
    if not condition:
        raise AssertionError(message)


class HubHandler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self, status, value):
        data = json.dumps(value, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_request(self):
        route = urlsplit(self.path).path
        if self.headers.get("Authorization") != f"Bearer {TOKEN}":
            self.respond(401, {"error": "unauthorized"})
            return

        size = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(size)) if size else None
        if (self.command, route, body) == ("GET", "/api/v1/me", None):
            self.respond(200, {"member": MEMBER})
            return
        if (self.command, route, body) == ("GET", "/api/v1/events", None):
            time.sleep(0.1)
            self.respond(200, {"events": []})
            return

        self.server.calls.put((self.command, route, body))
        if (self.command, route) == ("PUT", SYNC):
            self.respond(200, {"localProfile": body})
        elif (self.command, route) == ("GET", PROFILE):
            try:
                self.respond(200, self.server.profiles.get_nowait())
            except queue.Empty:
                self.respond(500, {"error": "unexpected profile read"})
        else:
            self.respond(404, {"error": "unexpected route"})

    do_GET = do_PUT = do_POST = do_PATCH = do_DELETE = handle_request


def tray_cli(env, *args):
    result = subprocess.run([sys.executable, str(CLI), *args], env=env, capture_output=True, text=True, timeout=12)
    check(result.returncode == 0, f"{args[0]} failed: {result.stdout or result.stderr}")
    response = json.loads(result.stdout)
    check(response.get("ok") is True, f"{args[0]} returned an error")
    return response["data"]


def invalid_profile_write(sock, payload):
    request = {"version": 1, "action": "sync_local_profile", "payload": payload}
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(5)
        client.connect(str(sock))
        client.sendall(json.dumps(request).encode() + b"\n")
        with client.makefile("rb") as output:
            reply = json.loads(output.readline(32 * 1024))
    check(reply.get("ok") is False and reply.get("code") == "invalid_request", "The tray accepted a foreign profile field")


def verify_isolation(hub, env, sock, private_memory):
    status = tray_cli(env, "status")
    check(status.get("loggedIn") is True, "Synthetic tray login is unavailable")
    check(status.get("apiOrigin") == env["MILKBIRD_API_ORIGIN"], "The tray must use the loopback Hub")

    private_memory.write_text(PRIVATE_MEMORY, encoding="utf-8")
    tray_cli(env, "status")
    check(hub.calls.empty(), "A private-memory addition triggered an outbound profile operation")
    private_memory.unlink()
    tray_cli(env, "status")
    check(hub.calls.empty(), "A private-memory deletion triggered an outbound profile operation")
    private_memory.write_text(PRIVATE_MEMORY, encoding="utf-8")

    approved = {"campus": "东校区"}
    invalid_profile_write(sock, {**approved, "privateMemory": PRIVATE_MEMORY})
    invalid_profile_write(sock, {**approved, "cloudProfile": {"summary": "后台推断"}})
    check(hub.calls.empty(), "The tray forwarded a foreign profile field to Hub")

    saved = tray_cli(env, "sync-profile", "--campus", approved["campus"])
    check(saved == {"localProfile": approved}, "Club profile sync returned unexpected data")
    check(hub.calls.get(timeout=2) == ("PUT", SYNC, approved), "Profile sync sent fields beyond the approved campus")
    check(private_memory.read_text(encoding="utf-8") == PRIVATE_MEMORY, "Club profile sync changed private memory")

    for summary in ("倾向东校区", "已纠正为西校区", ""):
        expected = {"localProfile": approved, "cloudProfile": {"summary": summary}}
        hub.profiles.put(expected)
        check(tray_cli(env, "get-profile") == expected, "The tray returned unexpected club profile data")
        check(hub.calls.get(timeout=2) == ("GET", PROFILE, None), "Profile reads must have an empty request body")
        check(hub.calls.empty(), "Reading a changed inference caused a profile write")
        check(private_memory.read_text(encoding="utf-8") == PRIVATE_MEMORY, "Club inference changed private memory")


def run(app):
    with (app / "Contents/Info.plist").open("rb") as info:
        plist = plistlib.load(info)
    executable = plist.get("CFBundleExecutable", "")
    check(plist.get("CFBundleIdentifier") == "cn.fordece.milkbird.tray", "Expected a MilkBird Tray application")
    check(executable and Path(executable).name == executable, "Invalid app executable name")
    binary = app / "Contents/MacOS" / executable

    with tempfile.TemporaryDirectory(prefix="mb-profile-", dir="/tmp") as scratch, ThreadingHTTPServer(
        ("127.0.0.1", 0), HubHandler
    ) as hub:
        hub.calls = queue.Queue()
        hub.profiles = queue.Queue()
        root = Path(scratch)
        tray = root / "tray"
        tray.mkdir(mode=0o700)
        session_fd = os.open(tray / "session.json", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(session_fd, "w", encoding="utf-8") as session:
            json.dump({"token": TOKEN, "member": MEMBER}, session)
        env = dict(
            os.environ,
            HOME=scratch,
            MILKBIRD_TRAY_DIR=str(tray),
            MILKBIRD_API_ORIGIN=f"http://127.0.0.1:{hub.server_port}",
            MILKBIRD_TRAY_TITLE="MilkBird Profile QA",
        )
        thread = threading.Thread(target=hub.serve_forever, daemon=True)
        thread.start()
        try:
            with (root / "tray.log").open("w+") as log:
                with subprocess.Popen([str(binary), "--background"], env=env, stdout=log, stderr=subprocess.STDOUT) as process:
                    try:
                        sock = tray / "tray.sock"
                        deadline = time.monotonic() + 20
                        while process.poll() is None and not sock.is_socket() and time.monotonic() < deadline:
                            time.sleep(0.1)
                        check(sock.is_socket() and process.poll() is None, "The native tray failed to start")
                        verify_isolation(hub, env, sock, root / "agent-private-memory.txt")
                    except Exception:
                        log.seek(0)
                        print(log.read()[-2000:].replace(TOKEN, "[test token]"), file=sys.stderr)
                        raise
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            try:
                                process.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                process.kill()
        finally:
            hub.shutdown()
            thread.join(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path, help="Path to a locally built MilkBird Tray.app")
    run(parser.parse_args().app.resolve(strict=True))
    print("PASS: profile reads, approved sync payload, and synthetic private-memory isolation")
