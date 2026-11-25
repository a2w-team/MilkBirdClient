#!/usr/bin/env python3
"""Run the campus assistant tools through the local MilkBird Tray."""

import argparse
import json
import os
import socket
from datetime import date
from pathlib import Path

BRIDGE_ACTION = "campus_assistant"
MAX_FRAME = 32 * 1024
BOOKING_SERVICES = ("swimming", "meeting-room")
SERVICES = ("mail", "timetable", *BOOKING_SERVICES)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError("Invalid command arguments. Run the command with --help for usage.")


def required_text(value):
    value = value.strip()
    if not value:
        raise argparse.ArgumentTypeError("value must not be blank")
    return value


def calendar_date(value):
    try:
        if date.fromisoformat(value).isoformat() == value:
            return value
    except ValueError:
        pass
    raise argparse.ArgumentTypeError("date must use YYYY-MM-DD")


def parse_arguments():
    parser = Parser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)

    def command(name, help_text):
        return commands.add_parser(name, help=help_text, argument_default=argparse.SUPPRESS)

    command("status", "Check campus service connections.")
    connect = command("connect", "Connect a campus service.")
    connect.add_argument("--service", choices=SERVICES, required=True)
    connect.add_argument("--account", choices=("student", "staff"), help="Demo account role.")
    command("disconnect", "Disconnect a campus service.").add_argument(
        "--service", choices=SERVICES, required=True
    )
    command("mail", "Read campus mailbox results.").add_argument(
        "--unread-only", dest="unreadOnly", action="store_true"
    )
    command("timetable", "Read the personal timetable.").add_argument(
        "--date", type=calendar_date, required=True
    )
    availability = command("availability", "List available booking slots.")
    availability.add_argument("--service", choices=BOOKING_SERVICES, required=True)
    availability.add_argument("--date", type=calendar_date, required=True)
    command("bookings", "List the account's bookings.").add_argument(
        "--service", choices=BOOKING_SERVICES, required=True
    )
    book = command("prepare-booking", "Preview a particular booking.")
    book.add_argument("--service", choices=BOOKING_SERVICES, required=True)
    book.add_argument("--date", type=calendar_date, required=True)
    book.add_argument("--slot-id", dest="slotId", type=required_text, required=True)
    cancel = command("prepare-cancellation", "Preview cancellation of an existing booking.")
    cancel.add_argument("--service", choices=BOOKING_SERVICES, required=True)
    cancel.add_argument("--booking-id", dest="bookingId", type=required_text, required=True)
    confirm = command("confirm", "Apply a booking or cancellation after the user confirms.")
    confirm.add_argument("--intent-id", dest="intentId", type=required_text, required=True)
    confirm.add_argument("--confirmed", action="store_true", required=True)

    payload = vars(parser.parse_args())
    action = payload.pop("action")
    return action, payload


def failure(code, message):
    return {"ok": False, "code": code, "message": message}


def invalid_constant(value):
    raise ValueError("Invalid JSON constant.")


def call(action, payload):
    directory = Path(os.environ.get("MILKBIRD_TRAY_DIR") or Path.home() / ".milkbird")
    request = {
        "version": 1,
        "action": BRIDGE_ACTION,
        "payload": {"action": action, "payload": payload},
    }
    frame = json.dumps(
        request, ensure_ascii=False, allow_nan=False, separators=(",", ":")
    ).encode("utf-8") + b"\n"
    if len(frame) > MAX_FRAME:
        return failure("request_too_large", "The tray request exceeds 32 KiB.")

    try:
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(8)
            client.connect(str(directory / "tray.sock"))
            client.sendall(frame)
            with client.makefile("rb") as stream:
                reply = stream.readline(MAX_FRAME + 1)
    except (ConnectionRefusedError, FileNotFoundError):
        return failure("tray_unavailable", "Open MilkBird Tray and try again.")
    except OSError:
        return failure("tray_connection_failed", "Could not complete the request to MilkBird Tray.")

    if len(reply) > MAX_FRAME or not reply.endswith(b"\n"):
        return failure("invalid_response", "MilkBird Tray returned an unreadable result.")
    try:
        result = json.loads(reply, parse_constant=invalid_constant)
        if isinstance(result, dict):
            if (
                result.get("ok") is True
                and result.get("action") == BRIDGE_ACTION
                and isinstance(result.get("data"), dict)
            ):
                return result
            if result.get("ok") is False and all(
                isinstance(result.get(key), str) for key in ("code", "message")
            ):
                return result
    except ValueError:
        pass
    return failure("invalid_response", "MilkBird Tray returned an unreadable result.")


def main():
    try:
        action, payload = parse_arguments()
        result = call(action, payload)
    except ValueError as error:
        result = failure("invalid_request", str(error))
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
