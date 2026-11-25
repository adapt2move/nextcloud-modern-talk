#!/usr/bin/env python3
"""Exercise native macOS capture against an official Talk receiver in a disposable HPB lab."""
import argparse
import asyncio
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import time
from urllib.parse import urlsplit

from ice_settings import fetch_settings

FLAGS = {"receive-only": 1, "missing-mic": 1, "mic-live": 3,
         "mic-stalled": 1, "mic-retried": 3, "camera-live": 5}
PHASES = {"microphone": ("receive-only", "mic-live", "mic-stalled", "mic-retried"),
          "camera": ("receive-only", "missing-mic", "camera-live")}


@dataclass(frozen=True)
class Lab:
    origin: str
    credentials: Path
    encryption: str

    @property
    def encrypted(self):
        return self.encryption == "required"


class ProbeError(Exception):
    """Only fixed diagnostic labels may enter the report."""


class Subscription:
    """Match a subscriber offer and answer for the same publisher and negotiation."""
    def __init__(self):
        self.members = {}
        self.requests = set()
        self.offers = set()
        self.answered = set()

    def observe(self, payload, outgoing):
        if not isinstance(payload, str) or len(payload) > 131072:
            return
        try:
            frame = json.loads(payload)
        except ValueError:
            return
        if not isinstance(frame, dict):
            return
        if frame.get("type") == "event" and not outgoing:
            event = frame.get("event")
            if not isinstance(event, dict) or not isinstance(event.get("join"), list):
                return
            if event.get("target") == "room" and event.get("type") == "join":
                for member in event.get("join", [])[:8]:
                    if not isinstance(member, dict):
                        continue
                    sfu, talk = member.get("sessionid"), member.get("roomsessionid")
                    if isinstance(sfu, str) and isinstance(talk, str) and len(self.members) < 8:
                        self.members[talk] = sfu
        elif frame.get("type") == "message":
            message = frame.get("message")
            if not isinstance(message, dict):
                return
            data = message.get("data")
            if not isinstance(data, dict) or data.get("roomType") != "video":
                return
            party = message.get("recipient" if outgoing else "sender")
            if not isinstance(party, dict) or party.get("type") != "session":
                return
            peer = party.get("sessionid")
            if not isinstance(peer, str) or not peer:
                return
            if outgoing and data.get("type") == "requestoffer" and len(self.requests) < 8:
                self.requests.add(peer)
            sid = data.get("sid")
            if not isinstance(sid, str) or not 0 < len(sid) <= 64:
                return
            if not outgoing and data.get("type") == "offer" and len(self.offers) < 16:
                self.offers.add((peer, sid))
            if outgoing and data.get("type") == "answer" and (peer, sid) in self.offers:
                self.answered.add(peer)

    def subscribed(self, session):
        return self.members.get(session) in self.answered

    def requested(self, session):
        return self.members.get(session) in self.requests


class Evidence:
    """A fresh window for each phase and remote track, including after Retry."""
    def __init__(self, phase, encrypted):
        self.phase = phase
        self.encrypted = encrypted
        self.reset()

    def reset(self):
        self.track = self.since = self.first = self.shade = None
        self.changes = 0

    def sample(self, flags, rows, subscribed, now):
        if flags != FLAGS[self.phase]:
            self.reset()
            return False
        if self.phase == "camera-live":
            visible = [r for r in rows if r.get("luma") is not None]
            if not subscribed or len(visible) != 1:
                self.reset()
                return False
            row = visible[0]
            count = row.get("frames", 0)
            shade = 0 if row["luma"] < 80 else 1 if row["luma"] > 175 else None
        elif self.phase in ("mic-live", "mic-retried"):
            audible = [r for r in rows if r.get("tone")]
            if len(audible) != 1:
                self.reset()
                return False
            row = audible[0]
            count = row.get("packets", 0)
            shade = None
        else:
            if any(r.get("tone") or r.get("luma") is not None for r in rows):
                self.reset()
                return False
            if self.since is None:
                self.since = now
            return now - self.since >= 1.5

        # Require the decoder observation on the same remote track that supplied
        # the tone or visible pixels. Unknown instrumentation cannot pass either mode.
        if row.get("decoder") is not self.encrypted:
            raise ProbeError("live remote track does not confirm the requested E2EE mode")
        if self.track != row["id"] or count < self.first:
            self.reset()
            self.track, self.first, self.since = row["id"], count, now
        if shade is not None:
            if self.shade is not None and self.shade != shade:
                self.changes += 1
            self.shade = shade
        return (now - self.since >= 1.5 and count - self.first >= 5
                and (self.phase != "camera-live" or self.changes >= 2))


# The pinned HPB lab advertises the feature in both modes. Talk enables it
# when the setting is omitted. Missing or malformed capability data is unknown.
SERVER_E2EE = """async () => {
    const response = await fetch('/ocs/v2.php/cloud/capabilities', {cache:'no-store',
        redirect:'error', signal:AbortSignal.timeout(4000),
        headers:{'OCS-APIRequest':'true', Accept:'application/json', requesttoken:window.OC.requestToken}});
    if (!response.ok) return null;
    const {ocs} = await response.json();
    const talk = ocs?.data?.capabilities?.spreed;
    if (ocs?.meta?.status !== 'ok' || !Array.isArray(talk?.features)
        || !talk.features.includes('call-end-to-end-encryption') || !talk.config?.call) return null;
    if (typeof talk.config.call !== 'object' || Array.isArray(talk.config.call)) return null;
    const value = talk.config.call['end-to-end-encryption'];
    return typeof value === 'boolean' ? value : value === undefined ? true : null;
}"""


# Read exactly the native lab user's aggregate flags, attributable only when the
# account has one room session. Keep the opaque session inside the test process.
PARTICIPANT = """async token => {
    const url = '/ocs/v2.php/apps/spreed/api/v4/room/' + token + '/participants';
    const response = await fetch(url, {headers:{'OCS-APIRequest':'true', Accept:'application/json',
        requesttoken:window.OC.requestToken},
        cache:'no-store', redirect:'error', signal:AbortSignal.timeout(4000)});
    if (!response.ok) throw Error('participant response');
    const {ocs} = await response.json();
    if (ocs.meta.status !== 'ok') throw Error('participant status');
    const users = ocs.data.filter(u => u.actorType === 'users' && u.actorId === 'lab_alice');
    if (users.length !== 1 || users[0].sessionIds?.length !== 1) return null;
    const [session] = users[0].sessionIds;
    if (typeof session !== 'string' || !session || !Number.isInteger(users[0].inCall)) return null;
    return {flags:users[0].inCall, session};
}"""


async def phases(page, token, sockets, scenario, encrypted, directory, process, report):
    session = None
    for phase in PHASES[scenario]:
        report["stage"] = phase
        evidence = Evidence(phase, encrypted)
        ready = directory / (phase + ".ready")
        async with asyncio.timeout(50):
            while True:
                if process.returncode is not None:
                    raise ProbeError("sender exited before receiver confirmation")
                if ready.is_file():
                    participant = await page.evaluate(PARTICIPANT, token)
                    if participant and participant["flags"] & 1:
                        if session is None:
                            session = participant["session"]
                        if session != participant["session"]:
                            raise ProbeError("sender room session replaced")
                    flags = participant["flags"] if participant else None
                    rows = await page.evaluate("window.deviceCallSample()")
                    subscribed = any(s.subscribed(session) for s in sockets)
                    report["last_sample"] = {
                        "sender_encrypted": encrypted,  # the native owner checked this before .ready
                        "flags": flags, "tone": any(r.get("tone") for r in rows),
                        "requestoffer": any(s.requested(session) for s in sockets),
                        "subscriber_offer_answered": subscribed,
                        "visible_video": any(r.get("luma") is not None for r in rows),
                        "decoded_frames": max((r.get("frames", 0) for r in rows), default=0),
                    }
                    if evidence.sample(flags, rows, subscribed, time.monotonic()):
                        summary = report.pop("last_sample")
                        if evidence.track is not None:
                            row = next(r for r in rows if r["id"] == evidence.track)
                            summary["receiver_decode_transform"] = row["decoder"]
                            if phase == "camera-live":
                                summary["decoded_frame_delta"] = row["frames"] - evidence.first
                                summary["dark_light_transitions"] = evidence.changes
                            else:
                                summary["opus_packet_delta"] = row["packets"] - evidence.first
                        report["phases"][phase] = summary
                        (directory / (phase + ".ok")).touch(exist_ok=False)
                        break
                await asyncio.sleep(0.25)


async def scenario_run(executable, lab, browser, scenario, report):
    from talk_browser import FreshRoom, join_call, private_json

    users = private_json(lab.credentials)["users"]

    context = await browser.new_context(viewport={"width": 1280, "height": 900}, locale="en-US",
                                        permissions=["microphone", "camera"])
    await context.add_init_script(path=Path(__file__).with_name("device_receiver.js"))
    page = await context.new_page()
    sockets = []

    def attach(socket):
        if len(sockets) < 8:
            observer = Subscription()
            sockets.append(observer)
            socket.on("framesent", lambda frame: observer.observe(frame, True))
            socket.on("framereceived", lambda frame: observer.observe(frame, False))
    page.on("websocket", attach)

    room = FreshRoom(lab.origin, users["lab_alice"], lab.credentials.parent / "device-call-rooms")
    try:
        report["stage"] = "room"
        token = await room.open()
        settings = await asyncio.to_thread(fetch_settings, lab.origin, users["lab_bob"], token, "lab_bob")
        if settings.get("signalingMode") != "external":
            raise ProbeError("external signaling required")
        report["stage"] = "receiver login"
        await page.goto(lab.origin + "/login", wait_until="domcontentloaded")
        if urlsplit(page.url)[:2] != urlsplit(lab.origin)[:2]:
            raise ProbeError("unexpected login origin")
        await page.locator("#user").fill("lab_bob")
        await page.locator("#password").fill(users["lab_bob"])
        await page.get_by_role("button", name="Log in", exact=True).click()
        await page.wait_for_url(lambda url: "/login" not in urlsplit(url).path)
        report["stage"] = "receiver E2EE policy"
        report["server_e2ee"] = await page.evaluate(SERVER_E2EE)
        if report["server_e2ee"] is not lab.encrypted:
            raise ProbeError("Talk capability does not match the requested E2EE mode")
        report["stage"] = "receiver joins"
        if not await join_call(page, lab.origin + "/index.php/call/" + token):
            raise ProbeError("official Talk receiver did not join")

        with tempfile.TemporaryDirectory(prefix="talk-device-call-") as tmp:
            directory = Path(tmp).resolve()
            fixture = directory / "credentials.json"
            fd = os.open(fixture, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as stream:
                json.dump({"origin": lab.origin, "user": "lab_alice", "password": users["lab_alice"],
                           "room": {"token": token}}, stream)
            process = await asyncio.create_subprocess_exec(
                executable, "--dev-device-call", scenario, lab.encryption, fixture,
                stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL)
            try:
                await phases(page, token, sockets, scenario, lab.encrypted, directory, process, report)
                report["stage"] = "sender shutdown"
                if await asyncio.wait_for(process.wait(), 15) != 0 or not (directory / "finished").is_file():
                    raise ProbeError("sender did not confirm cleanup")
                if scenario == "microphone":
                    report["stage"] = "stall diagnosis"
                    # Read only the bounded, fresh probe log. Report the matched cause.
                    with (directory / "logs/diagnostics.jsonl").open() as stream:
                        diagnosed = any(event.get("operation") == "call_microphone"
                                        and event.get("outcome") == "failed"
                                        and event.get("detail") == "callback_stalled"
                                        for event in map(json.loads, stream))
                    if not diagnosed:
                        raise ProbeError("callback stall diagnosis absent")
                    report["microphone_failure"] = "callback_stalled"
            finally:
                if process.returncode is None:
                    (directory / "abort").touch()
                    try:
                        await asyncio.wait_for(process.wait(), 12)
                    except TimeoutError:
                        process.kill()
                        await process.wait()
    finally:
        try:
            await context.close()
        finally:
            report["room_cleanup"] = await room.close()
    if report["room_cleanup"] != "absence_confirmed":
        raise ProbeError("room cleanup unconfirmed")
    report["stage"] = "complete"


async def run_cases(args, labs, browser, report):
    for lab in labs:
        for scenario in (PHASES if args.scenario == "both" else [args.scenario]):
            result = {"encryption": lab.encryption, "phases": {}, "stage": "startup"}
            report["scenarios"][f"{scenario}-{lab.encryption}"] = result
            try:
                await scenario_run(args.executable, lab, browser, scenario, result)
            except Exception as error:
                result["failure"] = str(error) if isinstance(error, ProbeError) else type(error).__name__


async def run(args, labs, report):
    from playwright.async_api import async_playwright

    async with async_playwright() as pw:
        browser = await pw.chromium.launch(channel="chromium", args=[
            "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
            "--autoplay-policy=no-user-gesture-required", "--mute-audio",
            "--disable-features=WebRtcHideLocalIpsWithMdns"])
        try:
            await run_cases(args, labs, browser, report)
        finally:
            await browser.close()


def loopback_origin(value):
    origin = re.fullmatch(r"https?://127\.0\.0\.1(?::([1-9][0-9]{0,4}))?", value)
    if not origin or (origin[1] and int(origin[1]) > 65535):
        raise argparse.ArgumentTypeError("expected a literal loopback origin with a valid port")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", type=loopback_origin, required=True, help="literal loopback origin of the HPB lab")
    parser.add_argument("--credentials", required=True, type=Path)
    parser.add_argument("--encryption", choices=["both", "disabled", "required"], default="both")
    parser.add_argument("--e2ee-origin", type=loopback_origin, help="E2EE-enabled HPB lab for the matrix")
    parser.add_argument("--e2ee-credentials", type=Path, help="private credentials for that lab")
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--scenario", choices=["both", *PHASES], default="both")
    parser.add_argument("--inputs", choices=["physical", "virtual", "mixed"], required=True,
                        help="type of the configured Mac inputs, recorded as declared by the operator")
    parser.add_argument("--output", required=True, type=Path, help="new metadata report")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("the sender must run in a logged-in macOS session")
    if args.encryption == "both":
        if args.e2ee_origin is None or args.e2ee_credentials is None:
            parser.error("both modes require --e2ee-origin and --e2ee-credentials")
        labs = [Lab(args.origin, args.credentials, "disabled"),
                Lab(args.e2ee_origin, args.e2ee_credentials, "required")]
    else:
        if args.e2ee_origin is not None or args.e2ee_credentials is not None:
            parser.error("use --origin and --credentials for a single encryption mode")
        labs = [Lab(args.origin, args.credentials, args.encryption)]
    if not os.access(args.executable, os.X_OK) or not args.executable.is_file():
        parser.error("missing debug desktop executable")
    os.umask(0o077)
    report = {"result": "fail", "operator_declared_inputs": args.inputs, "scenarios": {}}
    with args.output.open("x") as stream:
        try:
            asyncio.run(run(args, labs, report))
            if all(s["stage"] == "complete" and "failure" not in s for s in report["scenarios"].values()):
                report["result"] = "pass"
        except Exception as error:
            report["failure"] = str(error) if isinstance(error, ProbeError) else type(error).__name__
        finally:
            json.dump(report, stream, indent=2)
            stream.write("\n")
    return int(report["result"] != "pass")


if __name__ == "__main__":
    sys.exit(main())
