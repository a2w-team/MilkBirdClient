#!/usr/bin/env python3
"""Use the MilkBird Tray tools on this Mac."""

import argparse
import json
import os
import socket
import uuid
from pathlib import Path

MAX_FRAME = 32 * 1024


class UsageError(ValueError):
    pass


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise UsageError(message)


def required_text(value):
    if not value.strip():
        raise argparse.ArgumentTypeError("value must not be blank")
    return value.strip()


def positive_int(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("value must be at least 1")
    return number


def versioned_field(value):
    field, separator, version = value.rpartition("=")
    if not separator:
        raise argparse.ArgumentTypeError("expected FIELD=VERSION")
    return required_text(field), positive_int(version)


def json_object(value):
    try:
        result = json.loads(value, parse_constant=invalid_constant)
    except ValueError as error:
        raise argparse.ArgumentTypeError("value must be a JSON object") from error
    if not isinstance(result, dict):
        raise argparse.ArgumentTypeError("value must be a JSON object")
    return result


def invalid_constant(value):
    raise ValueError(f"invalid JSON constant: {value}")


def parse_arguments():
    parser = Parser(description=__doc__)
    commands = parser.add_subparsers(required=True, metavar="COMMAND")

    def command(name, description, action=None):
        sub = commands.add_parser(
            name, help=description, description=description,
            argument_default=argparse.SUPPRESS,
        )
        sub.set_defaults(action=action or name.replace("-", "_"))
        return sub

    command("status", "Check login and unread notifications.")
    submit = command("submit-demand", "Submit a confirmed demand.")
    update = command("update-demand", "Apply confirmed changes to an existing demand.")
    submit.add_argument("--category", type=required_text, required=True)
    update.add_argument("--id", dest="demandId", type=required_text, required=True)
    for demand in (submit, update):
        demand.add_argument("--confirmed", action="store_true", required=True)
        for field in ("title", "campus"):
            demand.add_argument(f"--{field}", type=required_text, required=demand is submit)
        for field in ("summary", "location", "start", "end"):
            demand.add_argument(f"--{field}", type=required_text)
        demand.add_argument("--party-min", dest="partyMin", type=positive_int)
        demand.add_argument("--party-max", dest="partyMax", type=positive_int)
        demand.add_argument("--preferences-json", dest="preferences", type=json_object)
        demand.add_argument("--expires-at", dest="expiresAt", type=required_text)
    submit.add_argument("--idempotency-key", dest="idempotencyKey", type=required_text)
    submit.add_argument("--single-match", dest="singleMatch", action="store_true")
    submit.add_argument("--profile-field", dest="profileFields", type=versioned_field, action="append",
                        metavar="FIELD=VERSION")
    submit.add_argument("--inferred-tag", dest="inferredTags", type=required_text, action="append")
    submit.add_argument("--cloud-version", dest="cloudVersion", type=required_text)
    update.add_argument("--change-note", dest="changeNote", type=required_text)

    command("list-demands", "List your demands.").add_argument("--status", type=required_text)
    command("list-submissions", "List this device's unsynced demands.")
    command("withdraw-submission", "Withdraw an unsynced demand.").add_argument(
        "--idempotency-key", dest="idempotencyKey", type=required_text, required=True)
    command("get-demand", "Get a demand and its match authorization status.").add_argument(
        "--id", dest="demandId", type=required_text, required=True)
    command("cancel-demand", "Cancel an open demand.").add_argument(
        "--id", dest="demandId", type=required_text, required=True)
    command("complete-demand", "Complete a demand and free an active slot.").add_argument(
        "--id", dest="demandId", type=required_text, required=True)
    command("list-revisions", "List the revision history of a demand.", "list_demand_revisions").add_argument(
        "--demand-id", dest="demandId", type=required_text, required=True)
    command("list-responses", "List the responses to a demand.", "list_demand_responses").add_argument(
        "--demand-id", dest="demandId", type=required_text, required=True)
    command("list-matches", "List your matches.")
    respond = command("respond-match", "Accept or decline a match.")
    respond.add_argument("--id", dest="matchId", type=required_text, required=True)
    respond.add_argument("--decision", choices=("accept", "decline"), required=True)
    command("list-rooms", "List your chat rooms.")
    room = command("open-room", "Open a chat room in the tray.")
    room.add_argument("--id", dest="roomId", type=required_text, required=True)
    room.add_argument("--demand-id", dest="demandId", type=required_text)
    vote = command("vote-retention", "Vote on keeping a temporary chat room.", "vote_room_retention")
    vote.add_argument("--room-id", dest="roomId", type=required_text, required=True)
    choice = vote.add_mutually_exclusive_group(required=True)
    choice.add_argument("--keep", dest="keep", action="store_true")
    choice.add_argument("--no-keep", dest="keep", action="store_false")

    notify = command("notify", "Show a short notification in the tray.")
    notify.add_argument("--title", type=required_text, required=True)
    notify.add_argument("--body", type=required_text, required=True)
    notify.add_argument("--demand-id", dest="demandId", type=required_text)
    notify.add_argument("--match-id", dest="matchId", type=required_text)
    notify.add_argument("--room-id", dest="roomId", type=required_text)

    profile = command("sync-profile", "Sync approved profile details.", "sync_local_profile")
    profile.add_argument("--campus", type=required_text)
    profile.add_argument("--bio", type=required_text)
    profile.add_argument("--preferences-json", dest="preferences", type=json_object)
    profile_fields = command("sync-profile-fields", "Save confirmed profile fields with their versions.", "edit_profile_fields")
    profile_fields.add_argument("--confirmed", action="store_true", required=True)
    profile_fields.add_argument("--mutation-id", dest="mutationId", required=True, type=required_text)
    profile_fields.add_argument("--changes-json", dest="changes", required=True, type=json_object)
    command("get-profile", "Read your profile.")

    payload = vars(parser.parse_args())
    action = payload.pop("action")
    if action in ("submit_demand", "update_demand"):
        start = payload.pop("start", None)
        end = payload.pop("end", None)
        if bool(start) != bool(end):
            raise UsageError("--start and --end must be provided together")
        if start and end:
            payload["timeWindow"] = {"start": start, "end": end}
        party_min = payload.pop("partyMin", None)
        party_max = payload.pop("partyMax", None)
        if party_min is not None or party_max is not None:
            resolved_min = party_min if party_min is not None else 1
            resolved_max = party_max if party_max is not None else resolved_min
            if resolved_min > resolved_max:
                raise UsageError("--party-min must be at most --party-max")
            payload["partySize"] = {"min": resolved_min, "max": resolved_max}
    if action == "submit_demand":
        single_match = payload.pop("singleMatch", False)
        scope = {
            field: payload.pop(field)
            for field in ("profileFields", "inferredTags", "cloudVersion")
            if field in payload
        }
        if scope and not single_match:
            raise UsageError("profile selections require --single-match")
        if single_match:
            if "expiresAt" not in payload or "idempotencyKey" not in payload:
                raise UsageError("--single-match requires --expires-at and --idempotency-key")
            if ("inferredTags" in scope) != ("cloudVersion" in scope):
                raise UsageError("--inferred-tag and --cloud-version must be provided together")
            if "profileFields" in scope:
                selected = scope["profileFields"]
                scope["profileFields"] = dict(selected)
                if len(scope["profileFields"]) != len(selected):
                    raise UsageError("each --profile-field must name a different field")
            payload["matchAuthorization"] = {"purpose": "single_match", **scope}
        elif "idempotencyKey" not in payload:
            payload["idempotencyKey"] = str(uuid.uuid4())
    return action, payload


def failure(code, message):
    return {"ok": False, "code": code, "message": message}


def call(action, payload):
    directory = Path(os.environ.get("MILKBIRD_TRAY_DIR") or Path.home() / ".milkbird")
    frame = json.dumps(
        {"version": 1, "action": action, "payload": payload},
        ensure_ascii=False, allow_nan=False, separators=(",", ":"),
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

    try:
        if len(reply) > MAX_FRAME or not reply.endswith(b"\n"):
            raise ValueError("invalid frame")
        result = json.loads(reply, parse_constant=invalid_constant)
        if isinstance(result, dict):
            if result.get("ok") is True and result.get("action") == action and isinstance(result.get("data"), dict):
                return result
            if result.get("ok") is False and all(isinstance(result.get(key), str) for key in ("code", "message")):
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
