#!/usr/bin/env python3
"""Submit a confirmed campus announcement lookup to the MilkBird tray."""

import argparse
import json
import os
import socket
from pathlib import Path

MAX_FRAME = 8192


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campus", required=True)
    parser.add_argument("--topic", required=True)
    parser.add_argument("--confirmed", action="store_true", required=True)
    args = parser.parse_args()
    query = {"campus": args.campus.strip(), "topic": args.topic.strip()}
    if any(not value or len(value) > 80 for value in query.values()):
        parser.error("use the confirmed campus and topic, each between 1 and 80 characters")

    directory = Path(os.environ.get("MILKBIRD_TRAY_DIR") or Path.home() / ".milkbird")
    payload = {"version": 1, "confirmed": args.confirmed, "query": query}
    frame = json.dumps(payload, ensure_ascii=False).encode("utf-8") + b"\n"
    try:
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(8)
            client.connect(str(directory / "tray.sock"))
            client.sendall(frame)
            with client.makefile("rb") as stream:
                reply = stream.readline(MAX_FRAME + 1)
        if len(reply) > MAX_FRAME or not reply.endswith(b"\n"):
            raise ValueError("invalid response frame")
        result = json.loads(reply)
        if not isinstance(result, dict) or result.get("status") not in {
            "matched", "no_match", "needs_clarification", "error"
        }:
            raise ValueError("unexpected response")
    except (ConnectionRefusedError, FileNotFoundError):
        result = {"status": "error", "code": "tray_unavailable"}
    except OSError:
        result = {"status": "error", "code": "tray_connection_failed"}
    except ValueError:
        result = {"status": "error", "code": "invalid_response"}
    print(json.dumps(result, ensure_ascii=False))
    return int(result["status"] == "error")


if __name__ == "__main__":
    raise SystemExit(main())
