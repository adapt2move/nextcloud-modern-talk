#!/usr/bin/env python3
"""Check call-control and internal signaling on one owned disposable lab.

No media or browser is involved. Synthetic session IDs, credentials, and message
bodies remain in the private lab process and are never written to the result.
"""
import argparse
import json
import sys
import urllib.request

sys.dont_write_bytecode = True
from dev import owned_state, restore
from lab import API, LabError


def send(lab, token, actor, source, target, messages):
    rows = []
    for kind, payload in messages:
        inner = {"to": target, "roomType": "video", "type": kind, "payload": payload}
        rows.append({"ev": "message", "sessionId": source, "fn": json.dumps(inner)})
    lab.request(actor, "POST", API + "v3/signaling/" + token,
                body={"messages": json.dumps(rows)}, name="call-activity-send")


def receive(lab, token, actor, source, target):
    events = lab.request(actor, "GET", API + "v3/signaling/" + token,
                         name="call-activity-pull")
    if not isinstance(events, list) or len(events) > 128:
        raise LabError("Unexpected bounded signaling response")
    result = []
    for event in events:
        if not isinstance(event, dict) or event.get("type") != "message":
            continue
        raw = event.get("data")
        if not isinstance(raw, str) or len(raw) > 4096:
            raise LabError("Unexpected signaling message")
        message = json.loads(raw)
        if message.get("from") == source and message.get("to") == target:
            result.append((message.get("type"), message.get("payload")))
    return result


def check(state):
    with owned_state(state) as (_, record):
        lab = restore(record)
        room = lab.request("lab_alice", "POST", API + "v4/room",
                           body={"roomType": 2, "roomName": "Call activity fixture"},
                           expected=(201,), name="call-activity-create")
        token = room["token"]
        lab.sensitive.append(token)
        room_path = API + "v4/room/" + token
        call_path = API + "v4/call/" + token
        try:
            lab.request("lab_alice", "POST", room_path + "/participants",
                        body={"newParticipant": "lab_bob", "source": "users"},
                        name="call-activity-invite")
            sessions = {}
            result = lab.request("lab_alice", "POST", room_path + "/participants/active",
                                 body={}, name="call-activity-join-owner-room")
            sessions["lab_alice"] = result["sessionId"]
            lab.sensitive.append(sessions["lab_alice"])
            lab.request("lab_alice", "POST", call_path, body={"flags": 1, "silent": True},
                        name="call-activity-start")
            members = lab.request("lab_alice", "GET", room_path + "/participants",
                                  name="call-activity-members")
            bob = [p for p in members if p.get("actorType") == "users"
                   and p.get("actorId") == "lab_bob"]
            lab.require("call-activity-single-target", len(bob) == 1
                        and isinstance(bob[0].get("attendeeId"), int))
            attendee = bob[0]["attendeeId"]
            lab.request("lab_alice", "POST", call_path + "/ring/" + str(attendee),
                        body={}, name="call-activity-ring-absent")
            # Establish the authenticated browser session before binding it
            # to a room, as the ordinary account owner does during inspection.
            lab.request("lab_bob", "GET", "/ocs/v2.php/cloud/user", name="call-activity-inspect-member")
            result = lab.request("lab_bob", "POST", room_path + "/participants/active",
                                 body={}, name="call-activity-join-member-room")
            sessions["lab_bob"] = result["sessionId"]
            lab.sensitive.append(sessions["lab_bob"])
            members = lab.request("lab_alice", "GET", room_path + "/participants",
                                  name="call-activity-member-session")
            observed = [p for p in members if p.get("actorId") == "lab_bob"]
            cookie_counts = {actor: sum(len(handler.cookiejar) for handler in lab.clients[actor].handlers
                                        if isinstance(handler, urllib.request.HTTPCookieProcessor))
                             for actor in ("lab_alice", "lab_bob")}
            print(json.dumps({"member_session_observed": len(observed) == 1
                              and sessions["lab_bob"] in (observed[0].get("sessionIds") or []),
                              "cookie_counts": cookie_counts}), flush=True)
            lab.request("lab_bob", "POST", call_path, body={"flags": 1, "silent": True},
                        name="call-activity-join-call")
            # The released server may acknowledge a no-op rather than returning
            # the documented 400 when the attendee is already in the call.
            lab.request("lab_alice", "POST", call_path + "/ring/" + str(attendee),
                        body={}, expected=(200, 400), name="call-activity-ring-present")
            alice, bob_session = sessions["lab_alice"], sessions["lab_bob"]
            hand = {"state": True, "timestamp": 42}
            reaction = {"reaction": "👍"}
            mute = {"action": "forceMute", "peerId": bob_session}
            send(lab, token, "lab_alice", alice, bob_session,
                 [("raiseHand", hand), ("reaction", reaction), ("control", mute)])
            received = receive(lab, token, "lab_bob", alice, bob_session)
            for label, pair in (("hand", ("raiseHand", hand)),
                                ("reaction", ("reaction", reaction)),
                                ("moderator-mute", ("control", mute))):
                lab.require("call-activity-" + label + "-received", pair in received)
            # The ordinary member sends an allowed marker alongside a control;
            # receipt of the marker proves the destructive poll was meaningful.
            forbidden = {"action": "forceMute", "peerId": alice}
            send(lab, token, "lab_bob", bob_session, alice,
                 [("control", forbidden), ("reaction", reaction)])
            received = receive(lab, token, "lab_alice", bob_session, alice)
            lab.require("call-activity-member-marker", ("reaction", reaction) in received)
            lab.require("call-activity-member-mute-denied", ("control", forbidden) not in received)
            lab.request("lab_alice", "DELETE", call_path, body={"all": True},
                        name="call-activity-end-all")
            remaining = lab.request("lab_alice", "GET", call_path,
                                    name="call-activity-ended-members")
            lab.require("call-activity-end-all-empty", remaining == [])
            print(json.dumps({"profile": record["development"]["profile"], "result": "pass",
                              "scope": "internal signaling and call control; no media"}))
        except LabError:
            failed = [row for row in lab.current["assertions"] if row["result"] == "fail"]
            print(json.dumps({"failed_assertions": failed}), flush=True)
            raise
        finally:
            lab.request("lab_alice", "DELETE", room_path, expected=(200, 404),
                        name="call-activity-cleanup")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True)
    args = parser.parse_args()
    try:
        check(args.state)
    except (LabError, KeyError, ValueError, TypeError) as error:
        print(json.dumps({"result": "fail", "failure": type(error).__name__,
                          "detail": str(error) if isinstance(error, LabError) else "fixture shape"}))
        raise SystemExit(1)
