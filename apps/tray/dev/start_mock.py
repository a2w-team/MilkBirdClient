#!/usr/bin/env python3
"""Start the local Mock and real Tauri tray with an isolated session."""
import os
from pathlib import Path
import signal
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread

from mock_hub import CODE, DOMAINS, MockHub


def run_tray(command, env):
    with subprocess.Popen(command, env=env, start_new_session=True) as tray:
        try:
            return tray.wait()
        finally:
            if tray.poll() is None:
                os.killpg(tray.pid, signal.SIGTERM)
                try:
                    tray.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(tray.pid, signal.SIGKILL)


def terminate(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    signal.signal(signal.SIGTERM, terminate)
    manifest = Path(__file__).resolve().parents[1] / "src-tauri" / "Cargo.toml"
    command = ["cargo", "run", "--locked", "--manifest-path", str(manifest)]
    try:
        with TemporaryDirectory(prefix="milkbird-mock-", dir="/tmp") as session_dir, MockHub() as hub:
            origin = f"http://127.0.0.1:{hub.server_port}"
            env = {
                **os.environ,
                "MILKBIRD_API_ORIGIN": origin,
                "MILKBIRD_ALLOWED_EMAIL_DOMAINS": ",".join(DOMAINS),
                "MILKBIRD_TRAY_DIR": session_dir,
                "MILKBIRD_TRAY_TITLE": "a2w Mock",
            }
            for key in ("NO_PROXY", "no_proxy"):
                env[key] = ",".join(filter(None, (env.get(key), "127.0.0.1")))
            server = Thread(target=hub.serve_forever, daemon=True)
            server.start()
            try:
                print(f"MilkBird Mock · {origin}", flush=True)
                print(f"使用任意校园邮箱名，获取验证码后输入 {CODE}。按 Ctrl+C 退出。", flush=True)
                if env.get("MILKBIRD_CAMPUS_DEMO") == "1":
                    print(f"启动 Agent 前在其终端设置环境变量\nexport MILKBIRD_TRAY_DIR={session_dir}", flush=True)
                return run_tray(command, env)
            finally:
                hub.shutdown()
                server.join()
    except KeyboardInterrupt:
        return 130
    except OSError as error:
        raise SystemExit(f"无法启动 MilkBird Mock：{error}") from error


if __name__ == "__main__":
    raise SystemExit(main())
