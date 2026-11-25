"""In-memory Hub for the tray developer preview."""
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
import secrets
from threading import Condition
from time import monotonic
from urllib.parse import parse_qs, urlsplit

DOMAINS = ("hkust-gz.edu.cn", "connect.hkust-gz.edu.cn")
CODE = "123456"
CODE_TTL = 600


def timestamp(**offset):
    return (datetime.now(timezone.utc) + timedelta(**offset)).isoformat(timespec="seconds")


class MockError(Exception):
    def __init__(self, status, code):
        self.status = status
        self.code = code


class Demo:
    def __init__(self, member):
        self.member = member
        self.demands = {
            "food": {
                "id": "food", "title": "今晚南门约火锅", "status": "open", "version": 2,
                "category": "dining", "campus": "东校区", "location": "南门",
                "summary": "下课后一起吃火锅，人均 80 元以内。",
                "partySize": {"min": 2, "max": 4}, "preferences": {"预算": "¥80 / 人"},
                "expiresAt": timestamp(hours=6), "createdAt": timestamp(hours=-1),
            },
            "sports": {
                "id": "sports", "title": "周末双打缺一位搭档", "status": "open",
                "category": "sports", "campus": "东校区", "location": "体育馆",
                "expiresAt": timestamp(hours=28), "createdAt": timestamp(hours=-2),
            },
            "study": {
                "id": "study", "title": "找同学一起复习概率论", "status": "matched",
                "category": "study", "campus": "东校区", "location": "图书馆",
                "expiresAt": timestamp(hours=18), "createdAt": timestamp(hours=-3),
            },
            "coffee": {
                "id": "coffee", "title": "找一家课间能坐的咖啡店", "status": "completed",
                "category": "dining", "campus": "东校区", "createdAt": timestamp(days=-1),
            },
        }
        self.responses = {
            "food-link": {
                "id": "food-link", "kind": "link", "title": "南门餐饮指南", "source": "示例链接",
                "why": "从南门步行约 6 分钟，有双人套餐。", "url": "https://example.com/",
            },
            "food-note": {
                "id": "food-note", "kind": "richtext", "title": "来自同学的点单建议",
                "source": "校园互助", "content": "19:00 后通常要排队。\n想赶上晚自习的话，可以在 18:30 前到。",
            },
            "food-card": {
                "id": "food-card", "kind": "html_card", "title": "集合安排", "source": "示例卡片",
                "html": "<strong>18:30 · 东校区南门</strong><p>在门口长椅附近等，齐人后一起出发。</p>",
            },
        }
        self.matches = {}
        self.rooms = {}
        self.messages = {}
        self.events = []
        sports = self.demands["sports"]
        self.matches["sports-match"] = {
            "id": "sports-match", "myDemandId": sports["id"], "myDecision": "pending",
            "status": "pending", "reason": "同校区，周末都有空", "roomId": "",
            "participants": [
                {**self.participant(member), "demandId": sports["id"], "decision": "pending"},
                {"memberId": "peer", "displayName": "小陈", "decision": "accept"},
            ],
        }
        room = self.add_room("study", "概率论自习小组")
        self.add_message(room, "图书馆三楼靠窗有空位，带上第三章练习题～", "seed", peer=True)
        for response in self.responses.values():
            response["createdAt"] = timestamp(minutes=-20)
            self.emit("demand.response", demandId="food", response=response)
        self.emit("match.proposed", match=self.matches["sports-match"], demandId="sports")

    @staticmethod
    def participant(member):
        return {"memberId": member["id"], "displayName": member["displayName"]}

    def emit(self, kind, **payload):
        self.events.append({
            "seq": len(self.events) + 1, "type": kind,
            "createdAt": timestamp(), "payload": deepcopy(payload),
        })

    def add_room(self, demand_id, title, match_id=""):
        room_id = demand_id + "-room"
        room = self.rooms[room_id] = {
            "id": room_id, "demandId": demand_id, "matchId": match_id, "title": title,
            "status": "active", "members": [
                self.participant(self.member), {"memberId": "peer", "displayName": "小陈"},
            ],
            "createdAt": timestamp(), "expiresAt": timestamp(hours=24),
            "retentionStatus": "temporary",
            "retention": {"keepCount": 1, "requiredCount": 2, "totalMembers": 2, "myVote": ""},
        }
        self.messages[room_id] = []
        return room

    def add_message(self, room, body, client_id, peer=False):
        room_id = room["id"]
        sender = room["members"][1 if peer else 0]
        messages = self.messages[room_id]
        for message in messages:
            if (message["clientMsgId"], message["senderId"]) == (client_id, sender["memberId"]):
                return message
        seq = len(messages) + 1
        message = {
            "id": f"{room_id}-{seq}", "seq": seq, "roomId": room_id, "clientMsgId": client_id,
            "senderId": sender["memberId"], "senderName": sender["displayName"],
            "body": body, "createdAt": timestamp(),
        }
        messages.append(message)
        room["lastMessage"] = message
        self.emit("room.message", roomId=room_id, demandId=room["demandId"], message=message)
        return message

    def dispatch(self, method, path, query, body):
        match method, path:
            case ("GET", ["demands"]):
                status = query.get("status", [""])[0]
                return {
                    "demands": [d for d in self.demands.values() if not status or d["status"] == status],
                    "activeCount": sum(d["status"] == "open" for d in self.demands.values()),
                    "maxActive": 3,
                }
            case ("GET", ["demands", demand_id, "responses"]) if demand_id in self.demands:
                return {"responses": list(self.responses.values()) if demand_id == "food" else []}
            case ("GET", ["demands", demand_id, "revisions"]) if demand_id in self.demands:
                demand = self.demands[demand_id]
                if demand_id != "food":
                    return {"revisions": [{**demand, "version": 1, "changeNote": "创建需求"}]}
                return {"revisions": [
                    {**demand, "changeNote": "把集合地点改到南门"},
                    {**demand, "version": 1, "location": "东门", "changeNote": "创建需求"},
                ]}
            case ("POST", ["demands", demand_id, ("complete" | "cancel") as action]) if demand_id in self.demands:
                demand = self.demands[demand_id]
                demand["status"] = "completed" if action == "complete" else "cancelled"
                for match_item in self.matches.values():
                    if match_item["myDemandId"] == demand_id and match_item["status"] == "pending":
                        match_item["status"] = "cancelled"
                self.emit("demand.updated", demand=demand)
                return {"demand": demand}
            case ("POST", ["responses", response_id, "feedback"]) if response_id in self.responses:
                feedback = body.get("feedback")
                if feedback not in ("useful", "dismissed", ""):
                    raise MockError(400, "invalid_request")
                response = self.responses[response_id]
                response["feedback"] = feedback
                return {"response": response}
            case ("GET", ["matches"]):
                return {"matches": list(self.matches.values())}
            case ("POST", ["matches", match_id, "respond"]) if match_id in self.matches:
                return self.respond(match_id, body.get("decision"))
            case ("GET", ["rooms"]):
                return {"rooms": list(self.rooms.values())}
            case ("GET", ["rooms", room_id, "messages"]) if room_id in self.rooms:
                messages = self.messages[room_id]
                after = max(0, int(query.get("after", ["0"])[0]))
                return {"messages": messages[after:], "lastSeq": len(messages)}
            case ("POST", ["rooms", room_id, "messages"]) if room_id in self.rooms:
                text = body.get("body")
                client_id = body.get("clientMsgId")
                if not isinstance(text, str) or not 1 <= len(text.strip()) <= 2000:
                    raise MockError(400, "invalid_request")
                if not isinstance(client_id, str) or not client_id:
                    raise MockError(400, "invalid_request")
                return {"message": self.add_message(self.rooms[room_id], text.strip(), client_id)}
            case ("POST", ["rooms", room_id, "retention-vote"]) if room_id in self.rooms:
                room = self.rooms[room_id]
                if room["retentionStatus"] != "retained":
                    if not isinstance(body.get("keep"), bool):
                        raise MockError(400, "invalid_request")
                    keep = body["keep"]
                    room["retention"].update(myVote="keep" if keep else "drop", keepCount=2 if keep else 1)
                    room["retentionStatus"] = "retained" if keep else "temporary"
                    self.emit("room.updated", room=room)
                return {"room": room}
            case ("POST", ["rooms", room_id, "leave"]) if room_id in self.rooms:
                del self.rooms[room_id]
                del self.messages[room_id]
                self.emit("room.updated", roomId=room_id)
                return {"ok": True}
            case _:
                raise MockError(404, "not_found")

    def respond(self, match_id, decision):
        if decision not in ("accept", "decline"):
            raise MockError(400, "invalid_request")
        item = self.matches[match_id]
        demand = self.demands[item["myDemandId"]]
        room = self.rooms.get(item["roomId"])
        if item["status"] == "pending":
            item["myDecision"] = item["participants"][0]["decision"] = decision
            if decision == "accept":
                room = self.add_room(demand["id"], demand["title"], match_id)
                item.update(status="room_ready", roomId=room["id"])
                demand["status"] = "matched"
                self.emit("room.ready", room=room, match=item, demandId=demand["id"])
            else:
                item["status"] = "declined"
                self.emit("match.updated", match=item)
        return {"match": item, "room": room}


class MockHub(ThreadingHTTPServer):
    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.changed = Condition()
        self.codes = {}
        self.sessions = {}
        self.demos = {}

    def dispatch(self, method, path, query, body, token):
        if method == "POST" and path in (["auth", "email", "code"], ["auth", "email", "verify"]):
            email = str(body.get("email", "")).strip().lower()
            local, _, domain = email.partition("@")
            if not re.fullmatch(r"[a-z0-9.!#$%&'*+/=?^_`{|}~-]+", local) or domain not in DOMAINS:
                raise MockError(400, "invalid_campus_email")
            if path[-1] == "code":
                self.codes[email] = monotonic() + CODE_TTL
                return {"sent": True, "expiresInSeconds": CODE_TTL, "devCode": CODE}
            if body.get("code") != CODE or self.codes.get(email, 0) <= monotonic():
                raise MockError(401, "invalid_or_expired_code")
            del self.codes[email]
            if email not in self.demos:
                name = str(body.get("displayName") or "").strip() or local
                self.demos[email] = Demo({"id": "mock-" + email, "email": email, "displayName": name})
            token = "mock_" + secrets.token_urlsafe(24)
            self.sessions[token] = self.demos[email]
            return {"token": token, "member": self.demos[email].member}

        demo = self.sessions.get(token)
        if demo is None:
            raise MockError(401, "unauthorized")
        if method == "GET" and path == ["me"]:
            return {"member": demo.member}
        if method == "POST" and path == ["auth", "logout"]:
            del self.sessions[token]
            return {"ok": True}
        if method == "GET" and path == ["events"]:
            after = max(0, int(query.get("after", ["0"])[0]))
            wait = max(0, min(25, int(query.get("wait", ["0"])[0])))
            self.changed.wait_for(lambda: len(demo.events) > after or token not in self.sessions, wait)
            if token not in self.sessions:
                raise MockError(401, "unauthorized")
            return {"events": demo.events[after:], "lastSeq": len(demo.events)}
        return demo.dispatch(method, path, query, body)


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.handle_api()

    def do_POST(self):
        self.handle_api()

    def handle_api(self):
        url = urlsplit(self.path)
        status = 200
        try:
            if not url.path.startswith("/api/v1/"):
                raise MockError(404, "not_found")
            size = int(self.headers.get("Content-Length", 0))
            if not 0 <= size <= 32 * 1024:
                raise MockError(413, "invalid_request")
            body = json.loads(self.rfile.read(size)) if size else {}
            if not isinstance(body, dict):
                raise MockError(400, "invalid_request")
            token = self.headers.get("Authorization", "").removeprefix("Bearer ")
            path = url.path.removeprefix("/api/v1/").split("/")
            with self.server.changed:
                result = self.server.dispatch(self.command, path, parse_qs(url.query), body, token)
                encoded = json.dumps(result, ensure_ascii=False).encode()
                if self.command == "POST":
                    self.server.changed.notify_all()
        except MockError as error:
            status = error.status
            encoded = json.dumps({"error": error.code}).encode()
        except ValueError:
            status = 400
            encoded = b'{"error":"invalid_request"}'
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        try:
            self.wfile.write(encoded)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def log_message(self, fmt, *args):
        # Long-poll requests otherwise fill the launcher terminal.
        if urlsplit(self.path).path != "/api/v1/events":
            super().log_message(fmt, *args)
