#!/usr/bin/env python3
"""Named talk-browser-synthetic workflow; see native/media/README.md.

Runs only in the owning Linux development VM. No credentials, SDP, candidates,
room token, HTTP bodies, browser console or storage are written to evidence.
"""
import argparse
import asyncio
import base64
import hashlib
import importlib.metadata
import ipaddress
import itertools
import json
import os
from pathlib import Path
import secrets
import tempfile
import time
from urllib.parse import urlparse, parse_qs, urlencode
import urllib.request

from playwright.async_api import async_playwright, TimeoutError as BrowserTimeout


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def private_json(path):
    p = Path(path)
    st = p.lstat()
    parent = p.parent.lstat()
    if p.is_symlink() or not p.is_file() or st.st_uid != os.getuid() or st.st_mode & 0o777 != 0o600 or parent.st_mode & 0o777 != 0o700 or st.st_size > 65536:
        raise RuntimeError("private fixture permissions")
    return json.loads(p.read_text())


class LoopbackForward:
    """At most 32 connections, 64 KiB reads, 45 s idle / 180 s lifetime."""
    def __init__(self, target, port):
        address = ipaddress.ip_address(target)
        if not address.is_private or address.is_loopback or not 1024 <= port <= 65535:
            raise RuntimeError("private lab target required")
        self.target, self.port = str(address), port
        self.tasks = set()

    async def __aenter__(self):
        self.server = await asyncio.start_server(self.accept, "127.0.0.1", self.port, limit=65536, backlog=16)
        return self

    async def pipe(self, reader, writer):
        while True:
            data = await asyncio.wait_for(reader.read(65536), 45)
            if not data:
                return
            writer.write(data)
            await asyncio.wait_for(writer.drain(), 10)

    async def accept(self, reader, writer):
        if len(self.tasks) >= 32:
            writer.close()
            return
        task = asyncio.current_task()
        self.tasks.add(task)
        remote = None
        pipes = []
        try:
            upstream, remote = await asyncio.wait_for(asyncio.open_connection(self.target, 80, limit=65536), 3)
            pipes = [asyncio.create_task(self.pipe(reader, remote)), asyncio.create_task(self.pipe(upstream, writer))]
            done, _ = await asyncio.wait(pipes, timeout=180, return_when=asyncio.FIRST_COMPLETED)
            for item in done:
                item.result()
        except (OSError, TimeoutError, ConnectionError):
            pass
        finally:
            for item in pipes:
                item.cancel()
            await asyncio.gather(*pipes, return_exceptions=True)
            for stream in (writer, remote):
                if stream:
                    stream.close()
            self.tasks.discard(task)

    async def __aexit__(self, *_):
        self.server.close()
        await self.server.wait_closed()
        tasks = list(self.tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


class FreshRoom:
    """Disposable synthetic setup only; real signaling remains in the Rust adapter.

    Each one-shot run gets an empty room, so repeated workflow runs cannot feed
    historic room broadcasts into a later negotiation. No failed poll is replayed.
    """
    def __init__(self, origin, password, journal_root):
        self.origin, self.password, self.token = origin, password, None
        journal_root = Path(journal_root)
        journal_root.mkdir(mode=0o700, exist_ok=True)
        st = journal_root.lstat()
        if journal_root.is_symlink() or not journal_root.is_dir() or st.st_uid != os.getuid() or st.st_mode & 0o777 != 0o700:
            raise RuntimeError("private room journal directory")
        if len(list(itertools.islice(journal_root.iterdir(), 16))) >= 16:
            raise RuntimeError("unresolved room journal capacity")
        identity = secrets.token_hex(12)
        self.name = "nmt-media-" + identity
        self.journal = journal_root / (identity + ".json")
        fd = os.open(self.journal, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(fd)
        self.state = "not_sent"
        self.requests = []
        self.cleanup_outcome = None
        self.persist()

    def persist(self):
        # No credentials; an uncertain POST must survive temporary fixture cleanup.
        data = json.dumps({"schema":1,"origin":self.origin,"name":self.name,
                           "token":self.token,"state":self.state}).encode()
        if len(data) > 1024:
            raise RuntimeError("room journal bound")
        with self.journal.open("wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        directory = os.open(self.journal.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)

    @staticmethod
    def valid_token(token):
        return isinstance(token, str) and 1 <= len(token) <= 64 and token.isascii() and token.isalnum()

    def request(self, method, suffix, values=None):
        if len(self.requests) >= 12:
            raise RuntimeError("room setup request budget")
        kind = "list" if method == "GET" else "delete" if method == "DELETE" else "participant" if suffix else "create"
        record = {"operation":kind,"started_unix":time.time(),"timeout_seconds":3}
        self.requests.append(record)
        start = time.monotonic()
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *_):
                return None
        authorization = base64.b64encode(("lab_alice:" + self.password).encode()).decode()
        request = urllib.request.Request(self.origin + "/ocs/v2.php/apps/spreed/api/v4/room" + suffix,
            data=urlencode(values).encode() if values is not None else None, method=method,
            headers={"Authorization":"Basic " + authorization, "OCS-APIRequest":"true", "Accept":"application/json"})
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        try:
            with opener.open(request, timeout=3) as response:
                record["status"] = response.status
                record["headers_ms"] = round((time.monotonic() - start) * 1000, 3)
                raw = response.read(65537)
                if len(raw)>65536:
                    raise RuntimeError("lab setup response bound")
                record["bytes"] = len(raw)
                body = json.loads(raw)["ocs"]
                if body["meta"]["status"] != "ok" or body["meta"]["statuscode"] not in (100, 200, 201):
                    raise RuntimeError("lab setup response status")
                return body["data"]
        except Exception as error:
            record["failure_type"] = type(error).__name__
            raise
        finally:
            record["elapsed_ms"] = round((time.monotonic() - start) * 1000, 3)

    async def owned_matches(self):
        rooms = await asyncio.to_thread(self.request, "GET", "?noStatusUpdate=1&includeLastMessage=false")
        if not isinstance(rooms, list) or len(rooms) > 32:
            raise RuntimeError("room reconciliation list bound")
        matches = []
        tokens = set()
        for room in rooms:
            if not isinstance(room, dict) or not self.valid_token(room.get("token")) or room["token"] in tokens:
                raise RuntimeError("room reconciliation shape")
            tokens.add(room["token"])
            if room.get("name") == self.name:
                if room.get("type") != 2 or room.get("participantType") != 1:
                    raise RuntimeError("room reconciliation ownership")
                matches.append(room["token"])
        if len(matches) > 1:
            raise RuntimeError("ambiguous room reconciliation")
        return matches

    async def open(self):
        if self.state != "not_sent" or await self.owned_matches():
            raise RuntimeError("room create is single use")
        self.state = "create_sent_once"
        self.persist()
        room = await asyncio.to_thread(self.request, "POST", "", {"roomType":2,"roomName":self.name})
        token = room.get("token")
        if not self.valid_token(token) or room.get("name") != self.name or room.get("type") != 2 or room.get("participantType") != 1:
            raise RuntimeError("lab setup token shape")
        self.token=token
        self.state = "create_confirmed"
        self.persist()
        await asyncio.to_thread(self.request,"POST","/"+token+"/participants",{"newParticipant":"lab_bob","source":"users"})
        return token

    async def close(self):
        if self.cleanup_outcome is not None:
            return self.cleanup_outcome
        if self.state == "not_sent":
            self.journal.unlink()
            self.cleanup_outcome = "not_created"
            return "not_created"
        # Bounded nondestructive reconciliation, never replay create or delete.
        # Absence after an uncertain create is not proof that it cannot finish late.
        matches = await self.owned_matches()
        if not matches and self.token is None:
            self.state = "uncertain_create"
            self.persist()
            return "uncertain_create"
        if not matches:
            self.journal.unlink()
            self.cleanup_outcome = "absence_confirmed"
            return "absence_confirmed"
        if self.token is not None and matches != [self.token]:
            raise RuntimeError("room reconciliation token mismatch")
        if self.state not in ("delete_sent_once", "uncertain_delete"):
            self.token = matches[0]
            self.state = "delete_sent_once"
            self.persist()
            try:
                await asyncio.to_thread(self.request, "DELETE", "/" + self.token)
            except Exception:
                # DELETE may have committed; only authenticated absence closes it.
                pass
        if await self.owned_matches():
            self.state = "uncertain_delete"
            self.persist()
            return "uncertain_delete"
        self.journal.unlink()
        self.cleanup_outcome = "absence_confirmed"
        return "absence_confirmed"


INSTRUMENT = r"""() => {
  const NativePC = window.RTCPeerConnection;
  const peers = [];
  const audio = new Map();
  const candidates = new Map();
  const states = new Map();
  const channels = new Map();
  const closeCauses = new Map();
  window.RTCPeerConnection = new Proxy(NativePC, {
    construct(target, args) {
      if (peers.length >= 8) throw new Error('diagnostic peer bound');
      const peer = new target(...args); peers.push(peer);
      const timeline=[]; states.set(peer,timeline);
      const dc=[]; channels.set(peer,dc);
      const observeChannel=channel=>{
        if(dc.length>=4)return;
        const row={label:['status','simplewebrtc'].includes(channel.label)?channel.label:'other',ordered:channel.ordered,
          reliable:channel.maxRetransmits===null&&channel.maxPacketLifeTime===null,states:[]};dc.push(row);
        const markChannel=()=>{if(row.states.length<8)row.states.push({state:channel.readyState,ms:Math.round(performance.now())});};
        markChannel();for(const event of ['open','close','error'])channel.addEventListener(event,markChannel);
      };
      peer.addEventListener('datachannel',event=>observeChannel(event.channel));
      const createChannel=peer.createDataChannel.bind(peer);
      peer.createDataChannel=(...args)=>{const channel=createChannel(...args);observeChannel(channel);return channel;};
      const mark=event=>{if(timeline.length<32)timeline.push({event,ms:Math.round(performance.now()),connection:peer.connectionState,ice:peer.iceConnectionState,signaling:peer.signalingState});};
      for(const event of ['connectionstatechange','iceconnectionstatechange','signalingstatechange','negotiationneeded']) peer.addEventListener(event,()=>mark(event));
      const close=peer.close.bind(peer); peer.close=()=>{
        // Never export stack text, script URLs or identifiers. Production bundles
        // may minify these names, so false means unobserved, not disproved.
        const stack=String(new Error().stack??'').slice(0,4096);
        closeCauses.set(peer,{force_reconnect_name:stack.includes('forceReconnect'),negotiation_handler_name:stack.includes('setHandlerForNegotiationNeeded')});
        mark('application-close');return close();
      };
      const counts = {total:0,udp:0,tcp:0,other:0,by_mline:new Uint16Array(24)};
      candidates.set(peer,counts);
      peer.addEventListener('icecandidate',event=>{
        if(!event.candidate || !event.candidate.candidate) return;
        counts.total=Math.min(255,counts.total+1);
        const protocol=event.candidate.protocol;
        const key=protocol==='udp'?'udp':protocol==='tcp'?'tcp':'other';
        counts[key]=Math.min(255,counts[key]+1);
        const index=event.candidate.sdpMLineIndex;
        if(Number.isInteger(index)&&index>=0&&index<24) counts.by_mline[index]=Math.min(255,counts.by_mline[index]+1);
      });
      peer.addEventListener('track', event => {
        if (event.track.kind !== 'audio' || audio.has(peer)) return;
        // Real decoded remote track, observed by a bounded test sink. This does
        // not generate/replace audio and exports only one RMS scalar.
        const context = new AudioContext({sampleRate:48000});
        const source = context.createMediaStreamSource(new MediaStream([event.track]));
        const analyser = context.createAnalyser(); analyser.fftSize = 2048;
        const gain = context.createGain(); gain.gain.value = 0;
        source.connect(analyser); analyser.connect(gain); gain.connect(context.destination);
        audio.set(peer,{context,source,analyser,gain,samples:new Float32Array(2048),peak:0});
        context.resume();
        event.track.addEventListener('ended',()=>{source.disconnect();analyser.disconnect();gain.disconnect();context.close();});
      });
      return peer;
    }
  });
  window.__nmt_stats = async () => {
    const output = [];
    for (const peer of peers) {
      const stats = await peer.getStats(); const rows = [];
      for (const row of stats.values()) {
        if (!['inbound-rtp','outbound-rtp'].includes(row.type)) continue;
        const codec = stats.get(row.codecId);
        const safe = {type:row.type,kind:row.kind,codec:codec?.mimeType,clockRate:codec?.clockRate};
        for (const key of ['bytesReceived','bytesSent','packetsReceived','packetsSent','framesDecoded','framesEncoded','framesReceived','frameWidth','frameHeight','totalAudioEnergy','totalSamplesDuration','totalSamplesReceived','concealedSamples','nackCount','pliCount']) {
          if (Number.isFinite(row[key])) safe[key] = row[key];
        }
        rows.push(safe);
      }
      const sink = audio.get(peer);
      if(sink) {
        sink.analyser.getFloatTimeDomainData(sink.samples);
        let energy=0; for(const sample of sink.samples) energy += sample*sample;
        sink.peak = Math.max(sink.peak,Math.sqrt(energy/sink.samples.length));
      }
      const application = (peer.remoteDescription?.sdp ?? '').split('\n').filter(line=>line.startsWith('m=application '));
      const media=[];
      for(const line of (peer.remoteDescription?.sdp ?? '').split('\n')) {
        if(line.startsWith('m=')) {
          const fields=line.trim().split(' ');
          if(media.length<3)media.push({kind:['m=audio','m=video','m=application'].includes(fields[0])?fields[0].slice(2):'other',rejected:fields[1]==='0',direction:'unspecified',opus:false,vp8:false});
        } else if(media.length) {
          const current=media[media.length-1];
          if(/^a=(sendrecv|sendonly|recvonly|inactive)\r?$/.test(line)) current.direction=line.trim().slice(2);
          if(/^a=rtpmap:\d+ opus\//i.test(line))current.opus=true;
          if(/^a=rtpmap:\d+ VP8\//i.test(line))current.vp8=true;
        }
      }
      const count=candidates.get(peer);
      output.push({connection:peer.connectionState,ice:peer.iceConnectionState,state_timeline:states.get(peer),data_channels:channels.get(peer),close_stack_classification:closeCauses.get(peer)??null,remote_media_profile:media,decoded_audio_peak_rms:sink?.peak ?? 0,audio_sink_state:sink?.context.state ?? 'absent',local_description_type:peer.localDescription?.type ?? 'absent',local_candidate_counts:{total:count.total,udp:count.udp,tcp:count.tcp,other:count.other,by_mline:[...count.by_mline]},remote_application_sections:application.length,remote_application_rejected:application.length>0 && application.every(line=>line.startsWith('m=application 0 ')),rows});
    }
    return output;
  };
}"""


async def press(page, names):
    for name in names:
        choices = page.get_by_role("button", name=name, exact=True)
        button = choices.last  # prejoin's foreground Start call, not the covered header
        if await choices.count() and await button.is_visible():
            for attempt in range(4):
                try:
                    await button.click(timeout=3000)
                    break
                except BrowserTimeout:
                    # First-login Nextcloud welcome and then Talk onboarding can
                    # cover a control after it was found. Use their real controls.
                    dismissed=False
                    if name not in ("Close","Skip"):
                        for overlay in ("Close","Skip"):
                            close=page.get_by_role("button",name=overlay,exact=True)
                            if await close.count() and await close.first.is_visible():
                                await close.first.click(timeout=3000)
                                dismissed=True
                                break
                    if not dismissed or attempt==3:
                        raise
            return name
    return None


def media_checks(samples):
    """An unresolved getStats codec is no observation, never a positive result."""
    rows = [row for sample in samples for peer in sample for row in peer["rows"]]
    def codec(row, expected):
        value = row.get("codec")
        return isinstance(value, str) and value.lower() == expected
    decoded_audio = any(peer.get("decoded_audio_peak_rms", 0) > .005 for sample in samples for peer in sample)
    return {
        "received_native_vp8_frames": any(r["type"] == "inbound-rtp" and codec(r, "video/vp8") and r.get("framesDecoded", 0) >= 20 for r in rows),
        "sent_browser_vp8_frames": any(r["type"] == "outbound-rtp" and codec(r, "video/vp8") and r.get("framesEncoded", 0) >= 20 for r in rows),
        "received_native_opus_audio": decoded_audio and any(r["type"] == "inbound-rtp" and codec(r, "audio/opus") and r.get("packetsReceived", 0) >= 25 for r in rows),
        "sent_browser_opus_packets": any(r["type"] == "outbound-rtp" and codec(r, "audio/opus") and r.get("packetsSent", 0) >= 25 for r in rows),
    }


async def run(args, report):
    if importlib.metadata.version("playwright") != "1.64.0":
        raise RuntimeError("browser package pin mismatch")
    manifest = private_json(args.manifest)
    credentials = private_json(manifest["credentials_file"])
    fixtures = private_json(manifest["fixtures_file"])
    origin, target = urlparse(manifest["client_url"]), urlparse(manifest["vm_url"])
    if origin.scheme != "http" or origin.hostname != "127.0.0.1" or target.scheme != "http" or not manifest["ready"]:
        raise RuntimeError("loopback diagnostic lab required")
    report.update(profile=manifest["profile"], nextcloud=manifest["nextcloud"], talk=manifest["talk"], lab_sources=manifest["source_sha256"])
    report["bridge_sha256"], report["helper_sha256"] = digest(args.bridge), digest(args.helper)
    report["browser_package"] = importlib.metadata.version("playwright")
    with tempfile.TemporaryDirectory(prefix="nmt-media-talk-", dir="/run") as private:
        private = Path(private)
        fixture = private / "native.json"
        fixture.write_text(json.dumps({"origin":manifest["client_url"], "user":"lab_alice", "password":credentials["users"]["lab_alice"], "room":{"token":fixtures["group_token"]}}))
        fixture.chmod(0o600)
        bridge_report = private / "bridge.json"
        async with LoopbackForward(target.hostname, origin.port) as forward, async_playwright() as playwright:
            browser_args = ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream", "--disable-features=WebRtcHideLocalIpsWithMdns", "--autoplay-policy=no-user-gesture-required"]
            if args.browser_network == "default-interface":
                # Explicit narrow network profile, not candidate filtering or a
                # production parser workaround. Chromium itself gathers on its
                # default interface. Keep the unrestricted profile available.
                policy = "default_public_and_private_interfaces"
                browser_args += ["--webrtc-ip-handling-policy=" + policy,
                                 "--force-webrtc-ip-handling-policy=" + policy]
            report["browser_network_profile"] = args.browser_network
            report["browser_arguments"] = browser_args
            browser = await playwright.chromium.launch(headless=True, channel="chromium", args=browser_args)
            report["browser_version"] = browser.version
            context = await browser.new_context(viewport={"width":1280,"height":900}, locale="en-US", permissions=["camera","microphone"])
            await context.add_init_script("(" + INSTRUMENT + ")()")
            page = await context.new_page()
            report["browser_signaling_shapes"] = []
            def shape(request):
                # Only source-bound field names/types/counts escape this parser.
                # Never keep the original request or any signaling string.
                if request.method!="POST" or "/api/v3/signaling/" not in urlparse(request.url).path or len(report["browser_signaling_shapes"])>=64:
                    return
                raw=request.post_data
                if not raw or len(raw)>131072:
                    return
                try:
                    if raw.startswith("{"):
                        data=json.loads(raw)
                        messages=data.get("messages",[])
                    else:
                        messages=parse_qs(raw,max_num_fields=8).get("messages",["[]"])[0]
                    if isinstance(messages,str):messages=json.loads(messages)
                    if not isinstance(messages,list) or len(messages)>32:return
                    for entry in messages:
                        inner=entry.get("fn")
                        if isinstance(inner,str):inner=json.loads(inner)
                        if not isinstance(inner,dict):continue
                        kind=inner.get("type")
                        payload=inner.get("payload")
                        safe={"type":kind if kind in ("offer","answer","candidate","endOfCandidates","mute","unmute","speaking","stoppedSpeaking","nick","control","status","hangup") else "other", "sid_type":type(inner.get("sid")).__name__,"payload_type":type(payload).__name__,"room_type_present":"roomType" in inner,"to_present":"to" in inner}
                        if isinstance(payload,dict):
                            safe["payload_fields"]={key:type(payload[key]).__name__ for key in ("type","sdp","candidate","sdpMid","sdpMLineIndex") if key in payload}
                            if isinstance(payload.get("sdp"),str):
                                sdp=payload["sdp"];safe.update(sdp_bytes=len(sdp),media_sections=sum(line.startswith("m=") for line in sdp.splitlines()),application_section="\nm=application " in sdp,simulcast="\na=simulcast" in sdp)
                            candidate=payload.get("candidate")
                            if isinstance(candidate,dict):
                                safe["candidate_fields"]={key:type(candidate[key]).__name__ for key in ("candidate","sdpMid","sdpMLineIndex") if key in candidate}
                                safe["candidate_empty"]=candidate.get("candidate")==""
                                safe["candidate_mid_bytes"]=len(candidate["sdpMid"]) if isinstance(candidate.get("sdpMid"),str) else None
                                index=candidate.get("sdpMLineIndex")
                                safe["candidate_index"]=index if isinstance(index,int) and 0<=index<24 else None
                        if len(report["browser_signaling_shapes"])<64:report["browser_signaling_shapes"].append(safe)
                except (ValueError,TypeError,AttributeError):
                    report["browser_signaling_shape_error"]=True
            page.on("request",shape)
            bridge = None
            room = FreshRoom(manifest["client_url"],credentials["users"]["lab_alice"], args.manifest.parent / "media-owned-rooms")
            try:
                report["stage"] = "create isolated synthetic room"
                fixtures["group_token"] = await room.open()
                fixture.write_text(json.dumps({"origin":manifest["client_url"], "user":"lab_alice", "password":credentials["users"]["lab_alice"], "room":{"token":fixtures["group_token"]}}))
                report["fresh_room_created"] = True
                report["stage"] = "open login"
                await page.goto(manifest["client_url"] + "/login", wait_until="domcontentloaded", timeout=30000)
                report["stage"] = "fill login user"
                await page.locator("#user").fill("lab_bob")
                report["stage"] = "fill login password"
                await page.locator("#password").fill(credentials["users"]["lab_bob"])
                report["stage"] = "submit login"
                await page.get_by_role("button", name="Log in", exact=True).click()
                report["stage"] = "wait authenticated redirect"
                await page.wait_for_url(lambda url:"/login" not in urlparse(url).path, timeout=30000)
                report["stage"] = "open conversation"
                await page.goto(manifest["client_url"] + "/index.php/call/" + fixtures["group_token"], wait_until="domcontentloaded", timeout=30000)
                await page.wait_for_timeout(2500)
                await press(page,["Close","Dismiss","Skip"])
                # Only fixed synthetic UI text is retained for a failed selector;
                # never DOM HTML, console, network bodies or current URL.
                report["ui_actions"] = []
                report["stage"] = "join actual Talk call controls"
                for _ in range(5):
                    if await page.get_by_role("button", name="Leave call", exact=True).count():
                        break
                    clicked = await press(page,["Start call","Join call","Join the call","Join"])
                    if clicked:
                        report["ui_actions"].append(clicked)
                    await page.wait_for_timeout(1200)
                if args.inspect:
                    report["synthetic_ui_text"] = (await page.locator("body").inner_text())[:6000]
                    report["buttons"] = (await page.get_by_role("button").all_text_contents())[:60]
                    report["result"] = "inspection-only"
                    return
                report["stage"] = "native browser exchange"
                bridge = await asyncio.create_subprocess_exec(str(args.bridge), str(fixture), str(args.helper), str(bridge_report), *(["remote-leave"] if args.scenario=="remote-leave" else []), stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.DEVNULL)
                sampled = []
                departed = False
                report["scenario"] = args.scenario
                start = time.monotonic()
                while bridge.returncode is None and time.monotonic() - start < 40:
                    sample = await page.evaluate("window.__nmt_stats()")
                    if args.scenario=="remote-leave" and not departed:
                        rows_now=[row for peer in sample for row in peer["rows"]]
                        if any(row.get("framesDecoded",0)>=60 for row in rows_now) and any(row.get("packetsSent",0)>=150 and row.get("kind")=="audio" for row in rows_now):
                            departed = await press(page,["Leave call"])
                            report["browser_deliberate_leave"] = bool(departed)
                    if len(sampled) < 64:
                        sampled.append(sample)
                    try:
                        await asyncio.wait_for(bridge.wait(), .5)
                    except TimeoutError:
                        pass
                if bridge.returncode is None:
                    bridge.kill()
                    await bridge.wait()
                    report["bridge_deadline_exceeded"] = True
                report["bridge_exit"] = bridge.returncode
                if bridge_report.exists():
                    report["native"] = json.loads(bridge_report.read_text())
                report["browser_samples"] = sampled
                checks = media_checks(sampled)
                report["browser_checks"] = checks
                report["browser_audio_measurement"] = "RMS of actual decoded remote audio track via bounded WebAudio sink; totalAudioEnergy retained verbatim, no physical playout claim"
                report["result"] = "pass" if all(checks.values()) and report.get("native",{}).get("result")=="pass" else "fail"
            finally:
                if report["result"] == "fail":
                    try:
                        text = (await page.locator("body").inner_text(timeout=2000))[:6000]
                        for secret in [fixtures["group_token"], room.name, *credentials["users"].values()]:
                            text = text.replace(secret, "[redacted]")
                        report["synthetic_ui_text"] = text
                        report["buttons"] = (await page.get_by_role("button").all_text_contents())[:60]
                        report["button_labels"] = await page.get_by_role("button").evaluate_all("elements => elements.slice(0,60).map(e=>({label:e.getAttribute('aria-label'),title:e.getAttribute('title')}))")
                    except Exception:
                        pass
                if bridge and bridge.returncode is None:
                    bridge.terminate()
                    try:
                        await asyncio.wait_for(bridge.wait(), 2)
                    except TimeoutError:
                        bridge.kill()
                        await bridge.wait()
                try:
                    report["browser_leave_control"] = await press(page,["Leave call","End call"])
                except Exception:
                    report["browser_leave_control"] = "unavailable"
                await context.close()
                await browser.close()
                report["disposable_browser_closed"] = True
                try:
                    outcome = await room.close()
                    report["room_cleanup"] = outcome
                    report["fresh_room_deleted"] = outcome == "absence_confirmed"
                    if outcome.startswith("uncertain_"):
                        report["result"] = "fail"
                except Exception as error:
                    report["room_cleanup"] = "unconfirmed"
                    report["room_cleanup_failure_type"] = type(error).__name__
                    report["fresh_room_deleted"] = False
                    report["result"] = "fail"
                report["room_setup_requests"] = room.requests
        report["proxy_closed"] = True
    report["private_fixture_removed"] = True


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest",type=Path,required=True)
    p.add_argument("--bridge",type=Path,required=True)
    p.add_argument("--helper",type=Path,required=True)
    p.add_argument("--output",type=Path,required=True)
    p.add_argument("--inspect",action="store_true")
    p.add_argument("--browser-network",choices=("default","default-interface"),default="default",
                   help="explicit Chromium ICE gathering profile; default preserves all interfaces")
    p.add_argument("--scenario", choices=("exchange","remote-leave"), default="exchange")
    args=p.parse_args()
    report={"schema":1,"workflow":"talk-browser-synthetic","result":"fail","harness_sha256":digest(__file__),"physical_devices":False,"app_webview":False}
    repo = Path(__file__).resolve().parents[2]
    # The final run pairs these hashes with verify.py's compiled bridge hash and
    # frozen candidate archive. A source hash alone is not build provenance.
    report["network_source_sha256"] = {name:digest(repo/name) for name in (
        "crates/talk-adapters/src/nextcloud.rs",
        "crates/talk-adapters/src/nextcloud/diagnostics.rs",
        "crates/talk-adapters/src/nextcloud/transport.rs", "Cargo.lock")}
    try:
        asyncio.run(run(args,report))
    except Exception as error:
        report["failure_type"]=type(error).__name__
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({"result":report["result"],"output":str(args.output)}))
    raise SystemExit(0 if report["result"] in ("pass","inspection-only") else 1)


if __name__=="__main__":
    main()
